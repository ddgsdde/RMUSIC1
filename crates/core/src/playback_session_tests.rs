use super::*;

async fn prepared(guarded:bool)->Fixture {
    let f=Fixture::new().await;f.collection_tracks().await;
    f.s.endpoints.write().await.get_mut("browser-local").unwrap()["capabilities"]["playback_instance"]=json!(guarded);
    assert_eq!(f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"autoplay":{
        "enabled":false,"source":"local","fallback_local":true
    }}))).await.0,StatusCode::OK);
    f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":["local-shared","local-unknown"]}))).await;
    f
}
async fn command(f:&Fixture,action:&str,body:Value)->Value {
    let(status,value)=f.call("POST",&format!("/api/v1/zones/zone-local/{action}"),Some(body)).await;
    assert_eq!(status,if action=="progress"{StatusCode::NO_CONTENT}else{StatusCode::OK});value
}
async fn zone(f:&Fixture)->Value {f.call("GET","/api/v1/zones/zone-local",None).await.1}
fn id(zone:&Value)->String {zone["playback_id"].as_str().expect("core must issue a playback ID").into()}
async fn sessions(f:&Fixture)->i64 {f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM play_sessions",[],|r|r.get(0))?)).await.unwrap()}

#[tokio::test]
async fn guarded_system_controls_resume_same_instance_and_reject_replaced_playback() {
    let f=prepared(true).await;let first=command(&f,"play",json!({})).await;
    assert_eq!(f.call("GET","/api/v1/status",None).await.1["capabilities"]["guarded_transport_v1"],true);
    let guard=json!({"endpoint_id":"browser-local","track_id":"local-shared","playback_id":id(&first)});
    let paused=command(&f,"pause",json!({"control":guard,"position":3.})).await;
    assert_eq!(paused["position"],3.);assert_eq!(paused["playback_id"],first["playback_id"]);
    let resumed=command(&f,"play",json!({"control":guard,"position":3.})).await;
    assert_eq!(resumed["status"],"playing");assert_eq!(resumed["playback_id"],first["playback_id"]);
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":id(&resumed),"position":4.,"status":"paused"})).await;
    assert_eq!(zone(&f).await,resumed);
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":id(&resumed),"position":5.,"status":"playing"})).await;
    let advanced=zone(&f).await;
    assert_eq!(advanced["status"],"playing");assert_eq!(advanced["position"],5.);
    let replacement=command(&f,"play",json!({"track_id":"local-shared"})).await;
    for action in ["pause","play","stop"] {
        assert_eq!(f.call("POST",&format!("/api/v1/zones/zone-local/{action}"),Some(json!({"control":guard,"position":4.}))).await.0,StatusCode::CONFLICT);
        assert_eq!(zone(&f).await,replacement);
    }
    let current=json!({"endpoint_id":"browser-local","track_id":"local-shared","playback_id":id(&replacement)});
    for key in ["endpoint_id","track_id"] {
        let mut wrong=current.clone();wrong[key]=json!("stale");
        assert_eq!(f.call("POST","/api/v1/zones/zone-local/pause",Some(json!({"control":wrong}))).await.0,StatusCode::CONFLICT);
        assert_eq!(zone(&f).await,replacement);
    }
    let stopped=command(&f,"stop",json!({"control":current,"position":1.})).await;
    assert_eq!(stopped["status"],"stopped");assert!(stopped["playback_id"].is_null());
}

#[tokio::test]
async fn same_track_replay_rejects_old_progress_and_eof_without_stats_changes(){
    let f=prepared(true).await;let first=command(&f,"play",json!({"track_id":"local-shared"})).await;let old=id(&first);
    let second=command(&f,"play",json!({"track_id":"local-shared"})).await;let current=id(&second);assert_ne!(old,current);let count=sessions(&f).await;
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":old,"position":29,"status":"stopped"})).await;
    let stale=command(&f,"ended",json!({"track_id":"local-shared","playback_id":old,"position":30})).await;
    assert_eq!(stale,second);assert_eq!(zone(&f).await,second);assert_eq!(sessions(&f).await,count);
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":current,"position":5})).await;assert_eq!(zone(&f).await["position"],5.);
}

#[tokio::test]
async fn final_eof_is_saved_before_bridge_stop_can_cancel_its_response(){
    let f=prepared(true).await;
    let current=command(&f,"play",json!({"track_id":"local-unknown"})).await;
    let(tx,mut rx)=tokio::sync::mpsc::channel(4);
    f.s.bridges.write().await.insert("browser-local".into(),tx);
    let auth=crate::auth::Auth{id:"test".into(),username:"test".into(),role:"admin".into(),totp_enabled:false,interactive:true,scopes:vec!["control".into()],session_hash:crate::auth::hash(&f.token)};
    let payload=serde_json::from_value(json!({"track_id":"local-unknown","playback_id":id(&current),"position":30})).unwrap();
    // Hold persistence so an early stop is observable before it can cancel the caller's EOF.
    let held=f.s.db.0.lock().unwrap();
    let request=crate::zones::command(axum::extract::State(f.s.clone()),auth,axum::extract::Path(("zone-local".into(),"ended".into())),Some(axum::Json(payload)));
    futures_util::pin_mut!(request);
    assert!(futures_util::poll!(request.as_mut()).is_pending());
    let early=rx.try_recv();drop(held);
    assert!(early.is_err(),"bridge received stop before the EOF state could be saved");
    let response=request.await.unwrap();assert_eq!(response.status(),StatusCode::OK);
    assert_eq!(rx.recv().await.unwrap()["type"],"stop");
    let saved=zone(&f).await;assert_eq!(saved["status"],"stopped");assert!(saved["playback_id"].is_null());
    let durable:Value=f.s.db.run(|c|{let raw:String=c.query_row("SELECT data FROM zones WHERE id='zone-local'",[],|r|r.get(0))?;Ok(serde_json::from_str(&raw)?)}).await.unwrap();
    assert_eq!(durable,saved);
}

#[tokio::test]
async fn accepted_eof_finishes_next_output_dispatch_after_the_request_is_dropped(){
    for last in [false,true] {
        let f=prepared(true).await;
        let track=if last{"local-unknown"}else{"local-shared"};
        let current=command(&f,"play",json!({"track_id":track})).await;
        let(tx,mut rx)=tokio::sync::mpsc::channel(4);
        f.s.bridges.write().await.insert("browser-local".into(),tx);
        let auth=crate::auth::Auth{id:"test".into(),username:"test".into(),role:"admin".into(),totp_enabled:false,interactive:true,scopes:vec!["control".into()],session_hash:crate::auth::hash(&f.token)};
        let payload=serde_json::from_value(json!({"track_id":track,"playback_id":id(&current),"position":30})).unwrap();
        let held=f.s.db.0.lock().unwrap();
        let mut request=Box::pin(crate::zones::command(axum::extract::State(f.s.clone()),auth,axum::extract::Path(("zone-local".into(),"ended".into())),Some(axum::Json(payload))));
        assert!(futures_util::poll!(request.as_mut()).is_pending());
        drop(request);drop(held);
        let message=tokio::time::timeout(std::time::Duration::from_secs(3),rx.recv()).await.expect("accepted EOF must dispatch without its original caller").unwrap();
        assert_eq!(message["type"],if last{"stop"}else{"play"});
        let saved=zone(&f).await;
        assert_eq!(saved["status"],if last{"stopped"}else{"playing"});
        if !last {assert_eq!(saved["playback_id"],message["playback_id"]);assert_ne!(id(&saved),id(&current));assert_eq!(saved["track"]["id"],"local-unknown");}
    }
}

#[tokio::test]
async fn committed_eof_dispatch_survives_the_client_retiring_its_previous_output(){
    let f=prepared(true).await;let current=command(&f,"play",json!({"track_id":"local-shared"})).await;
    let(tx,mut rx)=tokio::sync::mpsc::channel(4);f.s.bridges.write().await.insert("browser-local".into(),tx);
    let auth=crate::auth::Auth{id:"test".into(),username:"test".into(),role:"admin".into(),totp_enabled:false,interactive:true,scopes:vec!["control".into()],session_hash:crate::auth::hash(&f.token)};
    let payload=serde_json::from_value(json!({"track_id":"local-shared","playback_id":id(&current),"position":30})).unwrap();
    // Media-ticket ownership is read after the next zone state is committed.
    let held=f.s.bridge_owners.write().await;
    let mut request=Box::pin(crate::zones::command(axum::extract::State(f.s.clone()),auth,axum::extract::Path(("zone-local".into(),"ended".into())),Some(axum::Json(payload))));
    let committed=async {
        loop {
            if f.s.zones.read().await.get("zone-local").unwrap().playback_id.as_deref()!=Some(&id(&current)){break;}
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        tokio::select!{_ = request.as_mut()=>panic!("dispatch must wait for ticket ownership"),_ = committed=>{}}
    }).await.unwrap();
    assert!(rx.try_recv().is_err());drop(request);drop(held);
    let play=tokio::time::timeout(std::time::Duration::from_secs(3),rx.recv()).await.expect("committed next track must receive its play command").unwrap();
    let saved=zone(&f).await;assert_eq!(play["type"],"play");assert_eq!(play["playback_id"],saved["playback_id"]);assert_eq!(saved["position"],0.);assert_eq!(saved["status"],"playing");
}

#[tokio::test]
async fn repeat_one_consumes_each_eof_once_even_when_track_id_is_unchanged(){
    let f=prepared(true).await;f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"repeat":"one"}))).await;
    let first=command(&f,"play",json!({})).await;let old=id(&first);
    let second=command(&f,"ended",json!({"track_id":"local-shared","playback_id":old,"position":30})).await;
    assert_ne!(id(&second),old);assert_eq!(second["track"],first["track"]);assert_eq!(sessions(&f).await,2);
    let repeated=command(&f,"ended",json!({"track_id":"local-shared","playback_id":old,"position":30})).await;assert_eq!(repeated,second);assert_eq!(sessions(&f).await,2);
}

#[tokio::test]
async fn seek_invalidates_old_timeline_but_pause_and_resume_keep_the_session(){
    let f=prepared(true).await;let first=command(&f,"play",json!({})).await;let old=id(&first);
    let seek=command(&f,"seek",json!({"position":20})).await;let current=id(&seek);assert_ne!(old,current);
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":old,"position":3})).await;assert_eq!(zone(&f).await,seek);
    assert_eq!(command(&f,"pause",json!({})).await["playback_id"],current);
    let paused=zone(&f).await;
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":current,"position":21,"status":"playing"})).await;assert_eq!(zone(&f).await,paused);
    assert_eq!(command(&f,"play",json!({})).await["playback_id"],current);
    assert_eq!(f.call("POST","/api/v1/zones/zone-local/seek",Some(json!({"position":31}))).await.0,StatusCode::BAD_REQUEST);assert_eq!(id(&zone(&f).await),current);
}

#[tokio::test]
async fn stop_queue_replacement_and_restart_do_not_reuse_a_playback_id(){
    let f=prepared(true).await;let first=command(&f,"play",json!({})).await;let old=id(&first);
    let stopped=command(&f,"stop",json!({})).await;assert!(stopped["playback_id"].is_null());
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":old,"position":29,"status":"playing"})).await;assert_eq!(zone(&f).await,stopped);
    let second=command(&f,"play",json!({})).await;assert_ne!(old,id(&second));
    let(_,replaced)=f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":["local-shared"],"queue_index":0}))).await;assert_ne!(id(&second),id(&replaced));
    let mut legacy=replaced;legacy.as_object_mut().unwrap().remove("playback_id");
    f.s.db.run(move|c|{c.execute("UPDATE zones SET data=?1 WHERE id='zone-local'",[legacy.to_string()])?;Ok(())}).await.unwrap();
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();assert!(serde_json::to_value(&restored.zones.read().await["zone-local"]).unwrap()["playback_id"].is_null());assert_eq!(restored.zones.read().await["zone-local"].queue.len(),1);
}

#[tokio::test]
async fn guarded_endpoints_require_identity_and_legacy_reports_remain_compatible(){
    let f=prepared(true).await;let first=command(&f,"play",json!({})).await;
    for payload in [json!({"position":29,"status":"stopped"}),json!({"track_id":"local-shared","position":29}),json!({"playback_id":id(&first),"position":29})]{command(&f,"progress",payload).await;assert_eq!(zone(&f).await,first);}
    assert_eq!(command(&f,"ended",json!({"track_id":"local-shared","position":30})).await,first);
    f.s.endpoints.write().await.get_mut("browser-local").unwrap()["capabilities"]["playback_instance"]=json!(false);
    command(&f,"progress",json!({"position":4})).await;assert_eq!(zone(&f).await["position"],4.);
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":"wrong","position":29})).await;assert_eq!(zone(&f).await["position"],4.);
    assert_eq!(command(&f,"ended",json!({"track_id":"local-shared","position":30})).await["track"]["id"],"local-unknown");
}

#[tokio::test]
async fn browser_registration_negotiates_identity_guard_and_progress_events_include_it(){
    let f=prepared(false).await;
    let(_,endpoint)=f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-guarded","name":"guarded","playback_instance":true}))).await;assert_eq!(endpoint["capabilities"]["playback_instance"],true);
    let playing=command(&f,"play",json!({})).await;let mut events=f.s.events.subscribe();
    command(&f,"progress",json!({"track_id":"local-shared","playback_id":id(&playing),"position":5})).await;
    let event=events.recv().await.unwrap();assert_eq!(event["type"],"zone_progress");assert_eq!(event["data"]["playback_id"],playing["playback_id"]);
}

#[tokio::test]
async fn real_bridge_socket_rejects_old_signal_progress_error_and_duplicate_eof(){
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest,Message};
    let f=prepared(false).await;
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let app=f.app.clone();
    let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
    let mut request=format!("ws://{address}/api/v1/bridge/events").into_client_request().unwrap();request.headers_mut().insert("Authorization",format!("Bearer {}",f.token).parse().unwrap());
    let(mut socket,_)=tokio_tungstenite::connect_async(request).await.unwrap();
    socket.send(Message::Text(json!({"type":"register","endpoint":{"id":"guarded-bridge","kind":"wasapi","capabilities":{"playback_instance":true}}}).to_string().into())).await.unwrap();
    async fn barrier(socket:&mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>)->Vec<Value>{
        use futures_util::{SinkExt,StreamExt};use tokio_tungstenite::tungstenite::Message;
        socket.send(Message::Ping(vec![7,8,9].into())).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3),async{
            let mut messages=vec![];while let Some(message)=socket.next().await {match message.unwrap(){
                Message::Pong(bytes) if bytes[..]==[7,8,9]=>return messages,
                Message::Ping(bytes)=>socket.send(Message::Pong(bytes)).await.unwrap(),
                Message::Text(text)=>messages.push(serde_json::from_str(&text).unwrap()),_=>{}
            }}panic!("bridge closed before barrier");
        }).await.unwrap()
    }
    assert!(barrier(&mut socket).await.iter().any(|v|v["type"]=="registered"));
    f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"endpoint_id":"guarded-bridge"}))).await;
    let first=command(&f,"play",json!({"track_id":"local-shared"})).await;let old=id(&first);
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        use futures_util::StreamExt;
        while let Some(message)=socket.next().await {
            match message.unwrap() {
                Message::Text(text)=>{
                    let value:Value=serde_json::from_str(&text).unwrap();
                    if value["type"]=="play"&&value["playback_id"]==old{return;}
                },
                Message::Ping(bytes)=>socket.send(Message::Pong(bytes)).await.unwrap(),
                _=>{}
            }
        }
        panic!("bridge closed before receiving play");
    }).await.unwrap();
    let second=command(&f,"play",json!({"track_id":"local-shared"})).await;let current=id(&second);barrier(&mut socket).await;
    for report in [json!({"type":"state","status":"playing","position":29,"signal_path":{"output":"stale"}}),json!({"type":"error","error":"stale"}),json!({"type":"state","status":"ended","position":30})]{
        let mut report=report;report["zone_id"]=json!("zone-local");report["track_id"]=json!("local-shared");report["core_playback_id"]=json!(old);
        socket.send(Message::Text(report.to_string().into())).await.unwrap();
    }
    barrier(&mut socket).await;assert_eq!(zone(&f).await,second);assert!(f.s.endpoints.read().await["guarded-bridge"]["playback"].is_null());
    socket.send(Message::Text(json!({"type":"state","zone_id":"zone-local","track_id":"local-shared","core_playback_id":current,"status":"playing","position":5,"signal_path":{"output":"current"}}).to_string().into())).await.unwrap();
    barrier(&mut socket).await;assert_eq!(zone(&f).await["position"],5.);assert_eq!(f.s.endpoints.read().await["guarded-bridge"]["playback"]["core_playback_id"],current);
    command(&f,"pause",json!({})).await;
    let resumed=command(&f,"play",json!({})).await;
    assert_eq!(id(&resumed),current);
    barrier(&mut socket).await;
    socket.send(Message::Text(json!({"type":"state","zone_id":"zone-local","track_id":"local-shared","core_playback_id":current,"status":"paused","position":5,"signal_path":{"output":"stale pause"}}).to_string().into())).await.unwrap();
    barrier(&mut socket).await;
    assert_eq!(zone(&f).await,resumed);
    assert_eq!(f.s.endpoints.read().await["guarded-bridge"]["playback"]["signal_path"]["output"],"current");
    socket.send(Message::Text(json!({"type":"state","zone_id":"zone-local","track_id":"local-shared","core_playback_id":current,"status":"playing","position":6}).to_string().into())).await.unwrap();
    barrier(&mut socket).await;
    assert_eq!(zone(&f).await["position"],6.);
    f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"repeat":"one"}))).await;let count=sessions(&f).await;
    for _ in 0..2 {socket.send(Message::Text(json!({"type":"state","zone_id":"zone-local","track_id":"local-shared","core_playback_id":current,"status":"ended","position":30}).to_string().into())).await.unwrap();}
    barrier(&mut socket).await;assert_ne!(id(&zone(&f).await),current);assert_eq!(sessions(&f).await,count+1);
    socket.close(None).await.unwrap();server.abort();
}

#[tokio::test]
async fn delayed_end_from_an_old_timing_version_does_not_fail_the_new_song() {
 let f=prepared(true).await;let first=command(&f,"play",json!({})).await;
 let current=command(&f,"next",json!({})).await;
 let (status,returned)=f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":"local-shared","playback_id":id(&first),"position":30,"frames":1323000,"sample_rate":44100,"measured_version":2}))).await;
 assert_eq!(status,StatusCode::OK);assert_eq!(returned["playback_id"],current["playback_id"]);
 assert_eq!(zone(&f).await,current);
}
