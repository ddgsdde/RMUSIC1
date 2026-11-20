use super::*;
async fn prepare_v2(f:&Fixture)->Value {
    let z=zone(f).await;let endpoint=z["endpoint_id"].as_str().unwrap();f.s.endpoints.write().await.get_mut(endpoint).unwrap()["capabilities"]["measured_listens_v2"]=json!(true);
    let(status,p)=f.call("POST",&format!("{ZONE}/prepare"),Some(json!({"endpoint_id":endpoint,"track_id":z["track"]["id"],"playback_id":z["playback_id"],"measured_version":2}))).await;assert_eq!(status,StatusCode::OK,"{p}");assert_eq!(p["measured_version"],2);assert_eq!(p["continuous_recovery"],false);p
}
fn snapshot(z:&Value,p:&Value,seconds:f64)->Value {
    let mut s=measured_snapshot(&z["playback_id"],&p["listen_session_id"],&z["track"]["id"],0.,seconds*1.0005,"active");s["measured"]["version"]=json!(2);s["measured"]["segments"][0]["output"]=json!({"start":0,"end":(seconds*48000.).round() as u64,"sample_rate":48000});for d in s["days"].as_object_mut().unwrap().values_mut(){d["seconds"]=json!(seconds);}s
}
async fn upload(f:&Fixture,batch:Value)->Value {let(status,value)=f.call("POST","/api/v1/stats/offline",Some(batch)).await;assert_eq!(status,StatusCode::OK,"{value}");value}

#[tokio::test]
async fn dual_listen_output_union_counts_real_time_across_retries_sources_and_restart() {
    let f=fixture(&["local-shared"]).await;let z=zone(&f).await;let p=prepare_v2(&f).await;let retry=prepare_v2(&f).await;assert_eq!(retry["plan_id"],p["plan_id"]);
    let(status,_)=f.call("POST",&format!("{ZONE}/prepare"),Some(json!({"endpoint_id":z["endpoint_id"],"track_id":z["track"]["id"],"playback_id":z["playback_id"]}))).await;assert_eq!(status,StatusCode::BAD_REQUEST);
    device_progress(&f,&z,1.,1.).await;let zero=f.s.db.run(|c|Ok(c.query_row("SELECT COALESCE(SUM(play_seconds),0) FROM track_stats",[],|r|r.get::<_,f64>(0))?)).await.unwrap();assert_eq!(zero,0.,"old position reports must not credit v2 time");
    let first=snapshot(&z,&p,10.);let mut batch=measured_batch(&f,vec![first]).await;assert_eq!(upload(&f,batch.clone()).await["rejected"],json!([]));assert_eq!(totals(&f).await["tracks"][0]["seconds"],10.);assert_eq!(upload(&f,batch.clone()).await["changed"],false);
    let mut extended=snapshot(&z,&p,20.);extended["id"]=batch["sessions"][0]["id"].clone();extended["version"]=json!(2);extended["reason"]=json!("ended");batch["sessions"][0]=extended;
    assert_eq!(upload(&f,batch.clone()).await["rejected"],json!([]));assert_eq!(totals(&f).await["tracks"][0],json!({"id":"local-shared","seconds":20.,"plays":1,"skips":0}));
    let mut duplicate=batch.clone();duplicate["client_id"]=json!(uuid::Uuid::new_v4().to_string());duplicate["sessions"][0]["id"]=json!(uuid::Uuid::new_v4().to_string());assert_eq!(upload(&f,duplicate).await["rejected"],json!([]));assert_eq!(totals(&f).await["tracks"][0]["seconds"],20.);
    let db=crate::db::Db::open(&f.s.config.data.join("db.sqlite")).unwrap();let restarted=app(AppState::new(db,f.s.config.clone(),String::new()).await.unwrap());let(status,value)=call(&restarted,"POST","/api/v1/stats/offline",Some(&f.token),Some(batch)).await;assert_eq!(status,StatusCode::OK);assert_eq!(value["changed"],false);assert_eq!(call(&restarted,"GET",ZONE,Some(&f.token),None).await.1["status"],"stopped");
    let stored=f.s.db.run(|c|Ok(c.query_row("SELECT start,end,sample_rate FROM listen_output_intervals",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,u32>(2)?)))?)).await.unwrap();assert_eq!(stored,(0,960000,48000));
}

#[tokio::test]
async fn dual_listen_authorization_version_prefix_and_rate_rejections_are_atomic() {
    let f=fixture(&["local-shared"]).await;let z=zone(&f).await;
    let req=json!({"endpoint_id":z["endpoint_id"],"track_id":z["track"]["id"],"playback_id":z["playback_id"],"measured_version":2});assert_eq!(f.call("POST",&format!("{ZONE}/prepare"),Some(req)).await.0,StatusCode::BAD_REQUEST);
    let p=prepare_v2(&f).await;let original=snapshot(&z,&p,10.);let batch=measured_batch(&f,vec![original.clone()]).await;assert_eq!(upload(&f,batch.clone()).await["rejected"],json!([]));let before=totals(&f).await;
    for field in ["missing","rate","overlap","mode","claim","prefix","fractional"] {
        let mut b=batch.clone();let s=&mut b["sessions"][0];s["id"]=json!(uuid::Uuid::new_v4().to_string());
        match field {
            "missing"=>{s["measured"]["segments"][0].as_object_mut().unwrap().remove("output");},
            "rate"=>{s["measured"]["segments"][0]["output"]["sample_rate"]=json!(96000);s["measured"]["segments"][0]["output"]["end"]=json!(960000);},
            "overlap"=>{let seg=s["measured"]["segments"][0].clone();s["measured"]["segments"].as_array_mut().unwrap().push(seg);},
            "mode"=>{s["measured"]["version"]=json!(1);s["measured"]["segments"][0].as_object_mut().unwrap().remove("output");for d in s["days"].as_object_mut().unwrap().values_mut(){d["seconds"]=json!(10.005);}},
            "claim"=>s["measured"]["playback_id"]=json!(uuid::Uuid::new_v4().to_string()),
            "prefix"=>{s["id"]=original["id"].clone();s["version"]=json!(2);s["measured"]["segments"][0]["output"]["start"]=json!(1);s["measured"]["segments"][0]["output"]["end"]=json!(480001);},
            _=>s["measured"]["segments"][0]["output"]["end"]=json!(480000.5),
        }
        let(status,value)=f.call("POST","/api/v1/stats/offline",Some(b)).await;
        if field=="fractional" {assert_eq!(status,StatusCode::UNPROCESSABLE_ENTITY);}else{assert_eq!(status,StatusCode::OK,"{field}: {value}");assert_eq!(value["accepted"],json!([]),"{field}");if field=="prefix" {assert_eq!(value["rejected"][0]["code"],"conflicting_snapshot");}}
        assert_eq!(totals(&f).await,before,"invalid {field} changed issued time");
    }
}

#[tokio::test]
async fn dual_listen_shared_client_wire_fixture_imports_actual_time_without_starting_audio() {
    let f=fixture(&["local-shared"]).await;let z=zone(&f).await;let p=prepare_v2(&f).await;
    let mut golden:Value=serde_json::from_str(include_str!("../../../bridge/testdata/dual-listen-v2.json")).unwrap();let time=measured_snapshot(&z["playback_id"],&p["listen_session_id"],&z["track"]["id"],10.,11.0005,"ended");let at=time["updated_at"].as_i64().unwrap();let day=time["days"].as_object().unwrap().keys().next().unwrap();
    golden["id"]=json!(uuid::Uuid::new_v4().to_string());golden["track_id"]=z["track"]["id"].clone();golden["measured"]["playback_id"]=z["playback_id"].clone();golden["measured"]["listen_session_id"]=p["listen_session_id"].clone();golden["started_at"]=json!(at-1000);golden["updated_at"]=json!(at);golden["measured"]["segments"][0]["last_at"]=json!(at);golden["days"]=json!({(day):{"seconds":1.,"last_at":at}});
    let b=measured_batch(&f,vec![golden]).await;assert_eq!(upload(&f,b).await["rejected"],json!([]));assert_eq!(totals(&f).await["tracks"][0]["seconds"],1.);assert_eq!(zone(&f).await["playback_id"],z["playback_id"]);assert_eq!(zone(&f).await["position"],z["position"]);
}

#[test]
fn dual_listen_schema_upgrade_keeps_legacy_claims_and_integer_output_constraints() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("old.sqlite");let old=include_str!("../schema.sql").replace(",timing_version INTEGER NOT NULL DEFAULT 1","");let c=rusqlite::Connection::open(&path).unwrap();c.execute_batch(&old).unwrap();
    c.execute("INSERT INTO listen_groups(id,track_id,title,artist,duration,started_at,updated_at) VALUES('g','t','t','',30,0,0)",[]).unwrap();c.execute("INSERT INTO listen_claims(playback_id,group_id,baseline) VALUES('p','g',0)",[]).unwrap();drop(c);
    for _ in 0..2 {let db=crate::db::Db::open(&path).unwrap();let c=db.0.lock().unwrap();assert_eq!(c.query_row("SELECT timing_version FROM listen_claims WHERE playback_id='p'",[],|r|r.get::<_,u32>(0)).unwrap(),1);assert!(c.execute("INSERT INTO listen_output_intervals(playback_id,start,end,sample_rate) VALUES('p',-1,1,48000)",[]).is_err());assert_eq!(c.query_row("SELECT COUNT(*) FROM listen_output_intervals",[],|r|r.get::<_,i64>(0)).unwrap(),0);}
}

async fn online_elapsed(f:&Fixture) {elapsed(f,60).await;f.s.db.run(|c|{c.execute("UPDATE listen_output_cursors SET origin_at=origin_at-60000",[])?;Ok(())}).await.unwrap();}
fn online_report(z:&Value,seconds:f64)->Value {json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"position":seconds*1.0005,"frames":(seconds*48000.).round() as u64,"sample_rate":48000,"measured_version":2})}
#[tokio::test]
async fn output_online_progress_and_eof_share_integer_coverage_with_durable_history() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare_v2(&f).await;online_elapsed(&f).await;
    let earlier=measured_batch(&f,vec![snapshot(&z,&p,5.)]).await;assert_eq!(upload(&f,earlier).await["rejected"],json!([]));assert_eq!(totals(&f).await["tracks"][0]["seconds"],5.);
    let body=online_report(&z,10.);assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(body.clone())).await.0,StatusCode::NO_CONTENT);assert_eq!(zone(&f).await["position"],10.005);assert_eq!(totals(&f).await["tracks"][0]["seconds"],10.);
    let batch=measured_batch(&f,vec![snapshot(&z,&p,10.)]).await;assert_eq!(upload(&f,batch.clone()).await["rejected"],json!([]));assert_eq!(totals(&f).await["tracks"][0]["seconds"],10.);
    for field in ["backwards","position","rate"] {let mut bad=body.clone();match field {"backwards"=>bad["frames"]=json!(479999),"position"=>bad["position"]=json!(10.2),_=>{bad["sample_rate"]=json!(96000);bad["frames"]=json!(960000);}}assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(bad)).await.0,StatusCode::BAD_REQUEST,"{field}");assert_eq!(zone(&f).await["position"],10.005);}
    let mut legacy=online_report(&z,11.);legacy.as_object_mut().unwrap().remove("measured_version");assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(legacy.clone())).await.0,StatusCode::NO_CONTENT);assert_eq!(zone(&f).await["position"],10.005);assert_eq!(f.call("POST",&format!("{ZONE}/ended"),Some(legacy)).await.0,StatusCode::BAD_REQUEST);
    online_elapsed(&f).await;let end=online_report(&z,20.);let(status,next)=f.call("POST",&format!("{ZONE}/ended"),Some(end.clone())).await;assert_eq!(status,StatusCode::OK,"{next}");assert_ne!(next["playback_id"],p["items"][0]["playback_id"]);assert_eq!(next["track"]["id"],p["items"][0]["track"]["id"]);assert_eq!(totals(&f).await["tracks"][0],json!({"id":"local-shared","seconds":20.,"plays":1,"skips":0}));
    let before=totals(&f).await;f.call("POST",&format!("{ZONE}/ended"),Some(end)).await;assert_eq!(totals(&f).await,before);
    let mut tail=batch;tail["client_id"]=json!(uuid::Uuid::new_v4().to_string());tail["sessions"][0]=snapshot(&z,&p,20.);assert_eq!(upload(&f,tail).await["rejected"],json!([]));assert_eq!(totals(&f).await,before);
}

#[tokio::test]
async fn output_online_handoff_checks_whole_cohort_before_atomic_commit_and_replay() {
    let f=fixture(&["local-shared","local-unknown","local-shared"]).await;let z=zone(&f).await;let p=prepare_v2(&f).await;online_elapsed(&f).await;let mut body=handoff(&z,&p,2,5.,3.,0.5);body["measured_version"]=json!(2);
    for report in body["ended"].as_array_mut().unwrap(){report["position"]=json!(report["position"].as_f64().unwrap()*1.0005);}body["current"]["position"]=json!(0.50025);
    let before=totals(&f).await;let mut fast=body.clone();fast["request_id"]=json!(uuid::Uuid::new_v4().to_string());fast["ended"][0]["position"]=json!(62.*1.0005);fast["ended"][0]["frames"]=json!(62*48000);let(status,rejected)=f.call("POST",&format!("{ZONE}/handoff"),Some(fast)).await;assert_eq!(status,StatusCode::BAD_REQUEST,"{rejected}");assert_eq!(rejected["error"]["code"],"invalid_request");assert_eq!(totals(&f).await,before);assert_eq!(zone(&f).await["playback_id"],z["playback_id"]);
    let mut foreign=body.clone();foreign["current"]["playback_id"]=json!(uuid::Uuid::new_v4().to_string());assert_eq!(f.call("POST",&format!("{ZONE}/handoff"),Some(foreign)).await.0,StatusCode::BAD_REQUEST);assert_eq!(totals(&f).await,before);
    let(status,result)=f.call("POST",&format!("{ZONE}/handoff"),Some(body.clone())).await;assert_eq!(status,StatusCode::OK,"{result}");assert_eq!(result["accepted"],true,"{result}");assert_eq!(result["zone"]["position"],0.50025);assert_eq!(result["zone"]["playback_id"],p["items"][1]["playback_id"]);
    let after=totals(&f).await;let total=after["tracks"].as_array().unwrap().iter().map(|track|track["seconds"].as_f64().unwrap()).sum::<f64>();assert_eq!(total,8.5);assert_eq!(after["receipts"],1);
    let current=result["zone"]["playback_id"].as_str().unwrap().to_string();let check=current.clone();let cursor=f.s.db.run(move|c|Ok(c.query_row("SELECT base_frames,frames FROM listen_output_cursors WHERE playback_id=?1",[check],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?)).await.unwrap();assert_eq!(cursor,(384000,24000));prepare_v2(&f).await;let cursor_after=f.s.db.run(move|c|Ok(c.query_row("SELECT base_frames,frames FROM listen_output_cursors WHERE playback_id=?1",[current],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?)).await.unwrap();assert_eq!(cursor_after,cursor,"prepare retry must not reset the admitted output clock");
    let retry=f.call("POST",&format!("{ZONE}/handoff"),Some(body.clone())).await.1;assert_eq!(retry["accepted"],true);assert_eq!(retry["applied"],false);assert_eq!(totals(&f).await,after);
    body["ended"][0]["frames"]=json!(240001);assert_eq!(f.call("POST",&format!("{ZONE}/handoff"),Some(body)).await.0,StatusCode::CONFLICT);assert_eq!(totals(&f).await,after);
}


#[tokio::test]
async fn native_output_v2_socket_progress_and_eof_use_the_same_coverage_as_http_and_journal() {
    use futures_util::{SinkExt,StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest,Message};
    type Socket=tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
    async fn barrier(ws:&mut Socket) {
        ws.send(Message::Ping(vec![31,41].into())).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3),async{loop{match ws.next().await.unwrap().unwrap(){Message::Pong(data) if data[..]==[31,41]=>break,Message::Ping(data)=>ws.send(Message::Pong(data)).await.unwrap(),_=>{}}}}).await.unwrap();
    }
    let f=fixture(&["local-shared","local-unknown"]).await;
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let app=f.app.clone();let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
    let mut request=format!("ws://{address}/api/v1/bridge/events").into_client_request().unwrap();request.headers_mut().insert("Authorization",format!("Bearer {}",f.token).parse().unwrap());
    let(mut ws,_)=tokio_tungstenite::connect_async(request).await.unwrap();let z=zone(&f).await;
    ws.send(Message::Text(json!({"type":"register","endpoint":{"id":z["endpoint_id"],"kind":"alsa","capabilities":{"continuous_pcm":true,"playback_instance":true,"measured_listens_v2":true}}}).to_string().into())).await.unwrap();barrier(&mut ws).await;
    let p=prepare_v2(&f).await;online_elapsed(&f).await;
    let mut report=online_report(&z,10.);report["type"]=json!("state");report["status"]=json!("playing");report["zone_id"]=z["id"].clone();report["core_playback_id"]=report["playback_id"].clone();report.as_object_mut().unwrap().remove("playback_id");
    ws.send(Message::Text(report.to_string().into())).await.unwrap();barrier(&mut ws).await;
    assert_eq!(zone(&f).await["position"],10.005,"v2 socket progress must use actual output frames independently of source position");assert_eq!(totals(&f).await["tracks"][0]["seconds"],10.);
    let batch=measured_batch(&f,vec![snapshot(&z,&p,10.)]).await;assert_eq!(upload(&f,batch).await["rejected"],json!([]));assert_eq!(totals(&f).await["tracks"][0]["seconds"],10.);
    for invalid in [json!({"measured_version":3,"position":11.,"frames":528000}),json!({"measured_version":2,"position":11.0005,"frames":1}),json!({"measured_version":2,"position":12.,"status":"ended","frames":null}),json!({"measured_version":1,"position":12.,"frames":null})] {
        let mut bad=report.clone();for (key,value) in invalid.as_object().unwrap(){bad[key]=value.clone();}ws.send(Message::Text(bad.to_string().into())).await.unwrap();barrier(&mut ws).await;
        assert_eq!(zone(&f).await["playback_id"],z["playback_id"]);assert_eq!(zone(&f).await["position"],10.005);
    }
    let(status,_)=f.call("POST",&format!("{ZONE}/progress"),Some(json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"position":12.}))).await;assert_eq!(status,StatusCode::NO_CONTENT);assert_eq!(zone(&f).await["position"],10.005,"bare legacy HTTP position must not replace the v2 cursor");
    let mut next=online_report(&z,15.);assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(next.clone())).await.0,StatusCode::NO_CONTENT);
    next["frames"]=json!(960000);next["position"]=json!(20.01);next["type"]=json!("state");next["status"]=json!("ended");next["zone_id"]=z["id"].clone();next["core_playback_id"]=z["playback_id"].clone();next.as_object_mut().unwrap().remove("playback_id");
    ws.send(Message::Text(next.to_string().into())).await.unwrap();barrier(&mut ws).await;assert_eq!(zone(&f).await["track"]["id"],"local-unknown");assert_eq!(totals(&f).await["tracks"][0]["seconds"],20.);
    ws.send(Message::Text(next.to_string().into())).await.unwrap();barrier(&mut ws).await;let batch=measured_batch(&f,vec![snapshot(&z,&p,20.)]).await;upload(&f,batch).await;assert_eq!(totals(&f).await["tracks"][0]["seconds"],20.);
    ws.close(None).await.unwrap();server.abort();
}
