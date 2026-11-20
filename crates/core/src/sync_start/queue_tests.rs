use super::*;
#[path = "queue/handoff_tests.rs"]
mod handoffs;

async fn enable(f:&Fixture,p:&[Peer]) {
    for peer in p {let mut endpoints=f.s.endpoints.write().await;let caps=&mut endpoints.get_mut(&peer.endpoint).unwrap()["capabilities"];caps["scheduled_queue_v1"]=json!(true);caps["playback_instance"]=json!(true);}
}
async fn prepare(f:&Fixture,p:&Peer,command:&Value)->Value {
    let(status,value)=f.call("POST",&format!("/api/v1/zones/{}/prepare",p.zone),Some(json!({"endpoint_id":p.endpoint,"track_id":command["track"]["id"],"playback_id":command["playback_id"]}))).await;
    assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["accepted"],true,"{value}");value
}
#[tokio::test]
async fn sync_queue_reserves_before_ready_without_independent_listen_sessions() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;enable(&f,&p).await;
    let mut req=request(&track,&p);req["queue"]=json!([track,track,track]);
    let(status,result)=f.call("POST","/api/v1/sync/starts",Some(req.clone())).await;assert_eq!(status,StatusCode::ACCEPTED,"{result}");
    let commands=[receive(&mut p[0]).await,receive(&mut p[1]).await];let mut ids=Vec::new();
    for(i,command)in commands.iter().enumerate(){assert_eq!(command["group_queue"],true);let plan=prepare(&f,&p[i],command).await;assert_eq!(plan["group_start_id"],req["request_id"]);assert_eq!(plan["items"].as_array().unwrap().len(),2);assert_eq!(plan["has_more"],false);assert_eq!(plan["continuous_recovery"],false);assert!(plan["listen_session_id"].is_null());assert!(plan["items"][0]["listen_session_id"].is_null());assert_ne!(plan["items"][0]["playback_id"],plan["items"][1]["playback_id"]);let retry=prepare(&f,&p[i],command).await;assert_eq!(retry["plan_id"],plan["plan_id"]);for index in 0..2{assert_eq!(retry["items"][index]["playback_id"],plan["items"][index]["playback_id"]);}ids.push(plan["items"][0]["playback_id"].clone());assert_eq!(f.s.zones.read().await[&p[i].zone].status,"stopped");}
    assert_ne!(ids[0],ids[1]);assert_eq!(sessions(&f).await,0);
    let claims=f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM listen_claims",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(claims,0);
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::OK);
    f.call("POST",&format!("/api/v1/sync/starts/{}/cancel",req["request_id"].as_str().unwrap()),None).await;
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    let result=f.call("GET",&format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap()),None).await.1;
    assert_eq!(result["phase"],"cancelled");assert_eq!(result["may_have_started"],false);assert_eq!(sessions(&f).await,0);assert!(f.s.sequences.lock().await.is_empty());
    assert_eq!(f.call("GET","/api/v1/status",None).await.1["capabilities"]["group_queue_v1"],true);
    assert_eq!(f.call("GET","/api/v1/status",None).await.1["capabilities"]["group_transport_v1"],true);
}

#[tokio::test]
async fn sync_queue_bounds_preloads_and_stops_at_native_format_boundaries() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;enable(&f,&p).await;
    let mut base=library::get_track(&f.s,track.clone()).await.unwrap();base.sample_rate=768000;base.bit_depth=32;let saved=base.clone();f.s.db.run(move|c|library::upsert(c,&saved,None,None,0,0,None,None)).await.unwrap();
    for (case,rate,bits,channels) in [("rate",44100,32,1),("depth",768000,24,1),("channels",768000,32,2),("unknown",0,32,1),("same",768000,32,1),("window",768000,32,1)] {
        let mut next=base.clone();next.id=format!("queue-{case}");next.sample_rate=rate;next.bit_depth=bits;next.channels=channels;let saved=next.clone();f.s.db.run(move|c|library::upsert(c,&saved,None,None,0,0,None,None)).await.unwrap();
        let mut req=request(&track,&p);req["queue"]=if case=="window"{json!(vec![track.clone();12])}else{json!([track,track,next.id])};
        let(status,value)=f.call("POST","/api/v1/sync/starts",Some(req.clone())).await;
        if case=="unknown"{assert_eq!(status,StatusCode::BAD_REQUEST,"{value}");assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));continue;}
        assert_eq!(status,StatusCode::ACCEPTED,"{case}: {value}");
        for peer in &mut p{let command=receive(peer).await;let plan=prepare(&f,peer,&command).await;let expected=if case=="window"{8}else if case=="same"{2}else{1};assert_eq!(plan["items"].as_array().unwrap().len(),expected,"{case}");assert_eq!(plan["has_more"],case=="window","{case}");assert_eq!(plan["items"][0]["track"]["sample_rate"],768000);}
        f.call("POST",&format!("/api/v1/sync/starts/{}/cancel",req["request_id"].as_str().unwrap()),None).await;for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    }
    assert_eq!(sessions(&f).await,0);
}

#[tokio::test]
async fn sync_queue_validates_capability_order_sources_and_legacy_retry_identity() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;
    let mut req=request(&track,&p);req["queue"]=json!([track,track]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));enable(&f,&p).await;
    let mut bad=req.clone();bad["queue"]=json!(["different",track]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(bad)).await.0,StatusCode::BAD_REQUEST);
    let mut bad=req.clone();bad["queue"]=json!(vec![track.clone();10001]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(bad)).await.0,StatusCode::BAD_REQUEST);
    let mut bad=req.clone();bad["queue"]=json!([track,"missing"]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(bad)).await.0,StatusCode::NOT_FOUND);
    for(rate,bits,format)in [(1536000,32,"wav"),(2822400,1,"dsf")] {let mut next=library::get_track(&f.s,track.clone()).await.unwrap();next.id="unsupported-queue".into();next.sample_rate=rate;next.bit_depth=bits;next.format=format.into();f.s.db.run(move|c|library::upsert(c,&next,None,None,0,0,None,None)).await.unwrap();let mut bad=req.clone();bad["queue"]=json!([track,"unsupported-queue"]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(bad)).await.0,StatusCode::BAD_REQUEST);}
    req["queue"]=json!([]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);for peer in &mut p{receive(peer).await;}
    req.as_object_mut().unwrap().remove("queue");assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req)).await.0,StatusCode::OK);assert_eq!(sessions(&f).await,0);
}

#[tokio::test]
async fn sync_queue_preparation_rejects_foreign_credentials_and_stale_instances() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;enable(&f,&p).await;
    let mut req=request(&track,&p);req["queue"]=json!([track,track]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);let command=receive(&mut p[0]).await;receive(&mut p[1]).await;
    let path=format!("/api/v1/zones/{}/prepare",p[0].zone);let body=json!({"endpoint_id":p[0].endpoint,"track_id":track,"playback_id":command["playback_id"]});
    let token=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"foreign queue credential","scopes":["control"]}))).await.1["token"].as_str().unwrap().to_owned();assert_eq!(call(&f.app,"POST",&path,Some(&token),Some(body.clone())).await.0,StatusCode::FORBIDDEN);
    let mut stale=body.clone();stale["playback_id"]=json!(uuid::Uuid::new_v4().to_string());assert_eq!(f.call("POST",&path,Some(stale)).await.1["accepted"],false);
    prepare(&f,&p[0],&command).await;let(tx,_)=mpsc::channel(8);f.s.bridges.write().await.insert(p[0].endpoint.clone(),tx);assert_eq!(f.call("POST",&path,Some(body)).await.1["accepted"],false);
    assert_eq!(sessions(&f).await,0);
}
