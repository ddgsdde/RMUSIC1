//! Isolated Core API and registered-channel coordinator tests; no output device is opened.
use super::*;
#[path = "sync_start/queue_tests.rs"]
mod queues;
use tokio::sync::mpsc;
struct Peer {endpoint:String,zone:String,offset:i64,tx:mpsc::Sender<Value>,rx:mpsc::Receiver<Value>}
#[tokio::test]
async fn group_rejects_unknown_zero_rate_before_dispatch() {
    let f=Fixture::new().await;let mut peers=peers(&f,8).await;
    let (track,_)=f.add_audio().await;
    let request=request(&track,&peers);
    f.s.db.run(move|c|{c.execute("UPDATE tracks SET sample_rate=0 WHERE id=?1",[track])?;Ok(())}).await.unwrap();
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(request)).await.0,StatusCode::BAD_REQUEST);
    assert!(peers.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
}
async fn peers(f:&Fixture,capacity:usize)->Vec<Peer> {
    let mut peers=Vec::new();
    for (i,offset) in [5_000_000_000i64,8_000_000_000].into_iter().enumerate(){
        let endpoint=format!("sync-peer-{i}");let zone=format!("sync-zone-{i}");let(tx,rx)=mpsc::channel(capacity);
        f.s.save_zone(models::Zone::new(zone.clone(),zone.clone(),endpoint.clone())).await.unwrap();
        f.s.bridges.write().await.insert(endpoint.clone(),tx.clone());f.s.bridge_owners.write().await.insert(endpoint.clone(),auth::hash(&f.token));
        let mut filter=clock::Filter::default();let now=clock::now_ns();let mut reply=filter.probe(&endpoint,now);reply["host_clock_id"]=json!(uuid::Uuid::new_v4().to_string());reply["t2_ns"]=json!(now+offset as u64+1000);reply["t3_ns"]=json!(now+offset as u64+2000);assert!(filter.accept(&reply,now+3000));
        f.s.endpoints.write().await.insert(endpoint.clone(),json!({"id":endpoint,"name":endpoint,"kind":"alsa","online":true,"verified":false,"capabilities":{"scheduled_start_v1":true,"continuous_pcm":true,"host_clock_v1":true,"media_url_api_relative":true},"clock_calibration":filter.estimate(now+3000)}));
        f.s.clock_filters.write().await.insert(endpoint.clone(),(tx.clone(),filter));
        peers.push(Peer{endpoint,zone,offset,tx,rx});
    }peers
}
fn request(track:&str,peers:&[Peer])->Value {json!({"request_id":uuid::Uuid::new_v4().to_string(),"zone_ids":peers.iter().map(|p|p.zone.clone()).collect::<Vec<_>>(),"track_id":track,"position":0.})}
async fn receive(peer:&mut Peer)->Value {tokio::time::timeout(std::time::Duration::from_secs(2),peer.rx.recv()).await.unwrap().unwrap()}
fn ready(command:&Value)->Value {json!({"type":"start_state","version":1,"zone_id":command["zone_id"],"core_playback_id":command["playback_id"],"start":command["start"],"phase":"ready","programme_frames_played":0,"sample_rate":48000,"gate":{"phase":"ready","released_host_ns":0,"deadline_host_ns":0,"expires_host_ns":command["start"]["expires_host_ns"],"ready_host_ns":command["start"]["expires_host_ns"].as_u64().unwrap()-5_000_000_000u64+1}})}
async fn report(f:&Fixture,peer:&Peer,v:Value){let _lock=f.s.zone_lock.lock().await;assert!(sync_start::report(&f.s,&peer.endpoint,&peer.tx,&v).await.unwrap());}
async fn sessions(f:&Fixture)->i64 {f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM play_sessions",[],|r|r.get(0))?)).await.unwrap()}
#[tokio::test]
async fn sync_group_power_requires_a_current_clock_snapshot_and_preserves_legacy_tokens() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;let req=request(&track,&p);
    {let mut endpoints=f.s.endpoints.write().await;endpoints.get_mut(&p[0].endpoint).unwrap()["capabilities"]["group_power_v1"]=json!(true);}
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    let stamp=json!({"host_id":uuid::Uuid::new_v4().to_string(),"generation":3,"wake_generation":1});
    {let mut endpoints=f.s.endpoints.write().await;let endpoint=endpoints.get_mut(&p[0].endpoint).unwrap();endpoint["group_power"]=stamp.clone();endpoint["group_power_clock_id"]=json!("retired-clock");}
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);
    {let mut endpoints=f.s.endpoints.write().await;let endpoint=endpoints.get_mut(&p[0].endpoint).unwrap();endpoint["group_power_clock_id"]=endpoint["clock_calibration"]["host_clock_id"].clone();}
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
    let commands=[receive(&mut p[0]).await,receive(&mut p[1]).await];assert_eq!(commands[0]["start"]["group_power"],stamp);assert!(commands[1]["start"].get("group_power").is_none());
    let reason="分组请求早于当前唤醒状态，请重新手动起播";report(&f,&p[0],json!({"type":"start_error","version":1,"zone_id":p[0].zone,"core_playback_id":commands[0]["playback_id"],"start":commands[0]["start"],"error":reason})).await;
    assert_eq!(f.call("GET",&format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap()),None).await.1["error"],reason);
}
#[tokio::test]
async fn sync_group_list_filters_pages_checks_permissions_and_reads_restart_receipts() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;assert_eq!(f.call("GET","/api/v1/sync/starts",None).await.1["total"],0);
    let old=request(&track,&p);let old_path=format!("/api/v1/sync/starts/{}",old["request_id"].as_str().unwrap());f.call("POST","/api/v1/sync/starts",Some(old.clone())).await;for peer in &mut p{receive(peer).await;}
    f.call("POST",&format!("{old_path}/cancel"),None).await;for peer in &mut p{receive(peer).await;}
    let next=request(&track,&p);let next_path=format!("/api/v1/sync/starts/{}",next["request_id"].as_str().unwrap());assert_eq!(f.call("POST","/api/v1/sync/starts",Some(next.clone())).await.0,StatusCode::ACCEPTED);
    let(_,active)=f.call("GET","/api/v1/sync/starts?active=true&limit=32",None).await;assert_eq!(active["total"],1);assert_eq!(active["items"][0]["id"],next["request_id"]);assert!(active["items"][0]["title"].is_string());
    let(_,first)=f.call("GET","/api/v1/sync/starts?limit=1",None).await;let(_,second)=f.call("GET","/api/v1/sync/starts?limit=1&offset=1",None).await;assert_eq!(first["total"],2);assert_eq!(second["total"],2);assert_ne!(first["items"][0]["id"],second["items"][0]["id"]);
    for query in ["limit=0","limit=101","offset=100001","active=unknown"]{assert_eq!(f.call("GET",&format!("/api/v1/sync/starts?{query}"),None).await.0,StatusCode::BAD_REQUEST);}
    assert_eq!(call(&f.app,"GET","/api/v1/sync/starts",None,None).await.0,StatusCode::UNAUTHORIZED);
    let read=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"group read","scopes":["read"]}))).await.1["token"].as_str().unwrap().to_owned();let control=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"group control","scopes":["control"]}))).await.1["token"].as_str().unwrap().to_owned();
    assert_eq!(call(&f.app,"GET","/api/v1/sync/starts",Some(&read),None).await.0,StatusCode::OK);assert_eq!(call(&f.app,"POST",&format!("{next_path}/cancel"),Some(&read),None).await.0,StatusCode::FORBIDDEN);assert_eq!(call(&f.app,"GET","/api/v1/sync/starts",Some(&control),None).await.0,StatusCode::FORBIDDEN);
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();let app=app(restored);let(_,list)=call(&app,"GET","/api/v1/sync/starts?active=true",Some(&read),None).await;assert_eq!(list["total"],0);
    let(_,all)=call(&app,"GET","/api/v1/sync/starts",Some(&read),None).await;assert_eq!(all["total"],2);assert!(all["items"].as_array().unwrap().iter().all(|j|j["settled"]==true));assert!(all["items"].as_array().unwrap().iter().any(|j|j["id"]==next["request_id"]&&j["phase"]=="interrupted"));
    assert_eq!(call(&app,"POST",&format!("{next_path}/cancel"),Some(&control),None).await.0,StatusCode::OK);
}
#[tokio::test]
async fn sync_group_new_start_preserves_old_final_receipts_until_output_confirmation() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;let old=request(&track,&p);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(old.clone())).await.0,StatusCode::ACCEPTED);
    let commands=[receive(&mut p[0]).await,receive(&mut p[1]).await];for(i,command)in commands.iter().enumerate(){report(&f,&p[i],ready(command)).await;}
    let commits=[receive(&mut p[0]).await,receive(&mut p[1]).await];let path=format!("/api/v1/sync/starts/{}",old["request_id"].as_str().unwrap());let(_,committed)=f.call("GET",&path,None).await;let deadline=committed["deadline_core_ns"].as_u64().unwrap();tokio::time::sleep(std::time::Duration::from_nanos(deadline.saturating_sub(clock::now_ns())+1_000_000)).await;
    f.call("POST",&format!("{path}/cancel"),None).await;for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    let next=request(&track,&p);let next_path=format!("/api/v1/sync/starts/{}",next["request_id"].as_str().unwrap());assert_eq!(f.call("POST","/api/v1/sync/starts",Some(next)).await.0,StatusCode::ACCEPTED);let newer=[receive(&mut p[0]).await,receive(&mut p[1]).await];
    for(i,frames)in [(0,7200),(1,9600)] {report(&f,&p[i],json!({"type":"start_state","version":1,"start":commands[i]["start"],"phase":"cancelled","final":true,"zone_id":commands[i]["zone_id"],"core_playback_id":commands[i]["playback_id"],"sample_rate":48000,"programme_frames_played":frames,"gate":{"phase":"released","expires_host_ns":commands[i]["start"]["expires_host_ns"],"deadline_host_ns":commits[i]["deadline_host_ns"],"released_host_ns":commits[i]["deadline_host_ns"],"ready_host_ns":ready(&commands[i])["gate"]["ready_host_ns"]}})).await;}
    let(_,finished)=f.call("GET",&path,None).await;assert_eq!(finished["settled"],true);assert_eq!(finished["members"][0]["frames"],7200);assert_eq!(finished["members"][1]["frames"],9600);assert_eq!(finished["phase"],"cancelled");
    let(_,preparing)=f.call("GET",&next_path,None).await;assert_eq!(preparing["phase"],"preparing");assert!(preparing["members"].as_array().unwrap().iter().all(|m|m["frames"]==0&&m["final_report_received"]==false));assert_eq!(sessions(&f).await,1);
    for(i,command)in newer.iter().enumerate(){report(&f,&p[i],ready(command)).await;}for peer in &mut p{assert_eq!(receive(peer).await["type"],"commit_start");}
    for(i,peer)in p.iter().enumerate(){assert_eq!(f.s.zones.read().await[&peer.zone].playback_id.as_deref(),newer[i]["playback_id"].as_str());}
    let seconds=f.s.db.run(|c|Ok(c.query_row("SELECT SUM(play_seconds) FROM track_stats",[],|r|r.get::<_,f64>(0))?)).await.unwrap();assert!((seconds-0.2).abs()<1e-12);assert_eq!(sessions(&f).await,2);
}
#[tokio::test]
async fn sync_group_cancel_final_frames_correct_one_closed_session_without_touching_new_playback() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;let req=request(&track,&p);
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
    let commands=[receive(&mut p[0]).await,receive(&mut p[1]).await];for(i,command)in commands.iter().enumerate(){report(&f,&p[i],ready(command)).await;}
    let commits=[receive(&mut p[0]).await,receive(&mut p[1]).await];let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let(_,committed)=f.call("GET",&path,None).await;
    let deadline=committed["deadline_core_ns"].as_u64().unwrap();tokio::time::sleep(std::time::Duration::from_nanos(deadline.saturating_sub(clock::now_ns())+1_000_000)).await;
    report(&f,&p[0],json!({"type":"state","zone_id":commands[0]["zone_id"],"core_playback_id":commands[0]["playback_id"],"status":"playing","frames":4800,"sample_rate":48000,"position":0.1})).await;
    assert_eq!(f.call("POST",&format!("{path}/cancel"),None).await.1["phase"],"cancelled");for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    let mut new_zone=f.s.zones.read().await[&p[0].zone].clone();new_zone.renew_playback();new_zone.status="playing".into();let new_id=new_zone.playback_id.clone();f.s.save_zone(new_zone).await.unwrap();
    let final_report=|i:usize,frames:u64|json!({"type":"start_state","version":1,"start":commands[i]["start"],"phase":"cancelled","final":true,"zone_id":commands[i]["zone_id"],"core_playback_id":commands[i]["playback_id"],"sample_rate":48000,"programme_frames_played":frames,"gate":{"phase":"released","expires_host_ns":commands[i]["start"]["expires_host_ns"],"deadline_host_ns":commits[i]["deadline_host_ns"],"released_host_ns":commits[i]["deadline_host_ns"],"ready_host_ns":ready(&commands[i])["gate"]["ready_host_ns"]}});
    let mut invalid=final_report(0,7200);invalid["final"]=json!(false);report(&f,&p[0],invalid).await;
    let mut foreign=final_report(0,7200);foreign["start"]["generation"]=json!(999);report(&f,&p[0],foreign).await;
    let mut bad_rate=final_report(0,7200);bad_rate["sample_rate"]=json!(96000);report(&f,&p[0],bad_rate).await;
    let mut early=final_report(0,7200);early["gate"]["released_host_ns"]=json!(0);report(&f,&p[0],early).await;
    let(other,_)=mpsc::channel(1);{let _lock=f.s.zone_lock.lock().await;assert!(!sync_start::report(&f.s,&p[0].endpoint,&other,&final_report(0,7200)).await.unwrap());}
    report(&f,&p[0],final_report(0,7200)).await;let(_,first)=f.call("GET",&path,None).await;assert_eq!(first["members"][0]["frames"],7200);assert_eq!(first["settled"],false);
    report(&f,&p[0],final_report(0,12000)).await;report(&f,&p[1],final_report(1,9600)).await;
    let(_,finished)=f.call("GET",&path,None).await;assert_eq!(finished["members"][0]["frames"],7200);assert_eq!(finished["members"][1]["frames"],9600);assert_eq!(finished["settled"],true);assert_eq!(finished["phase"],"cancelled");
    assert_eq!(f.s.zones.read().await[&p[0].zone].playback_id,new_id);assert_eq!(sessions(&f).await,1);
    let(seconds,open)=f.s.db.run(|c|Ok((c.query_row("SELECT SUM(play_seconds) FROM track_stats",[],|r|r.get::<_,f64>(0))?,c.query_row("SELECT COUNT(*) FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get::<_,i64>(0))?))).await.unwrap();assert!((seconds-0.2).abs()<1e-12);assert_eq!(open,0);
    println!("SYNC_GROUP_FINAL_QA={finished}");
}
#[tokio::test]
async fn sync_group_final_wait_expires_and_restart_closes_pending_receipts_without_inventing_frames() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;let req=request(&track,&p);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
    let command=receive(&mut p[0]).await;receive(&mut p[1]).await;let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let(_,cancelled)=f.call("POST",&format!("{path}/cancel"),None).await;assert_eq!(cancelled["settled"],false);
    let id=req["request_id"].as_str().unwrap().to_owned();let saved=f.s.db.run(move|c|{sync_start::restart(c)?;Ok(c.query_row("SELECT result FROM sync_starts WHERE id=?1",[id],|r|r.get::<_,String>(0))?)}).await.unwrap();assert_eq!(serde_json::from_str::<Value>(&saved).unwrap()["settled"],true);
    tokio::time::sleep(std::time::Duration::from_millis(6050)).await;sync_start::tick(&f.s).await.unwrap();
    report(&f,&p[0],json!({"type":"start_state","version":1,"start":command["start"],"phase":"cancelled","final":true,"zone_id":command["zone_id"],"core_playback_id":command["playback_id"],"sample_rate":0,"programme_frames_played":0,"gate":{"phase":"cancelled","expires_host_ns":command["start"]["expires_host_ns"],"released_host_ns":0,"deadline_host_ns":0,"ready_host_ns":0}})).await;
    let(_,finished)=f.call("GET",&path,None).await;assert_eq!(finished["settled"],true);assert!(finished["members"].as_array().unwrap().iter().all(|m|m["final_report_received"]==false&&m["frames"]==0));assert_eq!(sessions(&f).await,0);
}
#[tokio::test]
async fn sync_group_expired_published_clock_uses_current_accepted_samples() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;
    for peer in &p{f.s.endpoints.write().await.get_mut(&peer.endpoint).unwrap()["clock_calibration"]["valid_until_core_ns"]=json!(0);}
    let(_,endpoints)=f.call("GET","/api/v1/endpoints",None).await;assert!(endpoints["items"].as_array().unwrap().iter().filter(|e|e["kind"]=="alsa").all(|e|e["clock_calibration"]["valid_until_core_ns"].as_u64().unwrap()>clock::now_ns()));
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(request(&track,&p))).await.0,StatusCode::ACCEPTED);
    let a=receive(&mut p[0]).await;let b=receive(&mut p[1]).await;report(&f,&p[0],ready(&a)).await;report(&f,&p[1],ready(&b)).await;
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"commit_start");}
}
#[tokio::test]
async fn sync_group_clock_reselects_fresh_sample_without_extending_expiry_or_connection_owner() {
    let f=Fixture::new().await;let p=peers(&f,8).await;let host=uuid::Uuid::new_v4().to_string();let mut filter=clock::Filter::default();
    for (t1,t2,t3,received) in [(1_000_000_000u64,1_000_000_100u64,1_000_000_200u64,1_000_000_300),(3_000_000_000,3_000_000_400,3_000_000_500,3_000_001_000)]{let mut reply=filter.probe(&p[0].endpoint,t1);reply["host_clock_id"]=json!(host);reply["t2_ns"]=json!(t2);reply["t3_ns"]=json!(t3);assert!(filter.accept(&reply,received));}
    let published=filter.estimate(6_900_000_000);assert_eq!(published["observed_at_core_ns"],1_000_000_300u64);assert!(published["valid_until_core_ns"].as_u64().unwrap()<7_100_000_000);
    f.s.clock_filters.write().await.insert(p[0].endpoint.clone(),(p[0].tx.clone(),filter));
    let fresh=f.s.host_clock_estimate(&p[0].endpoint,&p[0].tx,7_100_000_000).await;assert_eq!(fresh["observed_at_core_ns"],3_000_001_000u64);assert_eq!(fresh["valid_until_core_ns"],9_000_001_000u64);
    let(other,_)=mpsc::channel(1);assert!(f.s.host_clock_estimate(&p[0].endpoint,&other,7_100_000_000).await.is_null());assert!(f.s.host_clock_estimate(&p[0].endpoint,&p[0].tx,9_000_001_001).await.is_null());
}
#[tokio::test]
async fn sync_group_all_ready_uses_one_core_deadline_measured_frames_and_durable_idempotency() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;let req=request(&track,&p);let(status,created)=f.call("POST","/api/v1/sync/starts",Some(req.clone())).await;assert_eq!(status,StatusCode::ACCEPTED,"{created}");
    let first=receive(&mut p[0]).await;let second=receive(&mut p[1]).await;assert_eq!(first["type"],"prepare_start");assert_eq!(second["type"],"prepare_start");assert_eq!(sessions(&f).await,0);
    report(&f,&p[0],ready(&first)).await;assert!(p[0].rx.try_recv().is_err());assert!(p[1].rx.try_recv().is_err());assert_eq!(sessions(&f).await,0);
    report(&f,&p[1],ready(&second)).await;let commits=[receive(&mut p[0]).await,receive(&mut p[1]).await];
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let(_,committed)=f.call("GET",&path,None).await;assert_eq!(committed["phase"],"committed");let deadline=committed["deadline_core_ns"].as_u64().unwrap();
    for(i,commit)in commits.iter().enumerate(){assert_eq!(commit["type"],"commit_start");assert_eq!(commit["deadline_host_ns"].as_u64().unwrap() as i128-p[i].offset as i128,deadline as i128);}
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::OK);assert!(p[0].rx.try_recv().is_err());assert_eq!(sessions(&f).await,1);
    tokio::time::sleep(std::time::Duration::from_nanos(deadline.saturating_sub(clock::now_ns())+1_000_000)).await;
    for(i,frames)in [(0,12000),(1,24000),(0,36000)]{let prepare=if i==0{&first}else{&second};report(&f,&p[i],json!({"type":"state","zone_id":prepare["zone_id"],"core_playback_id":prepare["playback_id"],"status":"playing","frames":frames,"sample_rate":48000,"position":frames as f64/48000.})).await;}
    for(i,prepare)in [first.clone(),second.clone()].into_iter().enumerate(){report(&f,&p[i],json!({"type":"state","zone_id":prepare["zone_id"],"core_playback_id":prepare["playback_id"],"track_id":track,"status":"ended","frames":48000,"sample_rate":48000,"position":1.})).await;}
    let(_,ended)=f.call("GET",&path,None).await;assert_eq!(ended["phase"],"ended");assert!(ended["members"].as_array().unwrap().iter().all(|m|m["frames"]==48000));
    let seconds=f.s.db.run(|c|Ok(c.query_row("SELECT COALESCE(SUM(play_seconds),0) FROM track_stats",[],|r|r.get::<_,f64>(0))?)).await.unwrap();assert!((seconds-1.).abs()<1e-9,"the cohort must be counted once: {seconds}");
    assert_eq!(sessions(&f).await,1);assert_eq!(ended["statistics_scope"],"group");
    let mut conflict=req.clone();conflict["position"]=json!(0.1);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(conflict)).await.0,StatusCode::BAD_REQUEST);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req)).await.1["phase"],"ended");
    println!("SYNC_GROUP_QA={ended}");
}
#[tokio::test]
async fn sync_group_failure_and_user_zone_edit_cancel_every_preparation_without_listening_credit() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;let req=request(&track,&p);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
    let commands=[receive(&mut p[0]).await,receive(&mut p[1]).await];report(&f,&p[0],ready(&commands[0])).await;
    let mut failure=ready(&commands[1]);failure["type"]=json!("start_error");report(&f,&p[1],failure).await;
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}assert_eq!(sessions(&f).await,0);
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let(_,failed)=f.call("GET",&path,None).await;assert_eq!(failed["phase"],"failed");assert_eq!(failed["may_have_started"],false);
    let next=request(&track,&p);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(next)).await.0,StatusCode::ACCEPTED);receive(&mut p[0]).await;receive(&mut p[1]).await;
    assert_eq!(f.call("POST",&format!("/api/v1/zones/{}/stop",p[0].zone),Some(json!({}))).await.0,StatusCode::OK);
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}assert_eq!(sessions(&f).await,0);
}
#[tokio::test]
async fn sync_group_full_control_queue_never_partially_commits_and_restart_never_replays_a_request() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,1).await;let req=request(&track,&p);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
    let commands=[receive(&mut p[0]).await,receive(&mut p[1]).await];report(&f,&p[0],ready(&commands[0])).await;p[1].tx.try_send(json!({"type":"queued"})).unwrap();report(&f,&p[1],ready(&commands[1])).await;
    assert_eq!(receive(&mut p[0]).await["type"],"cancel_start");assert_eq!(receive(&mut p[1]).await["type"],"queued");assert!(p[1].rx.try_recv().is_err());assert_eq!(sessions(&f).await,0);
    let next=request(&track,&p);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(next.clone())).await.0,StatusCode::ACCEPTED);let a=receive(&mut p[0]).await;let b=receive(&mut p[1]).await;report(&f,&p[0],ready(&a)).await;report(&f,&p[1],ready(&b)).await;assert_eq!(receive(&mut p[0]).await["type"],"commit_start");assert_eq!(receive(&mut p[1]).await["type"],"commit_start");assert_eq!(sessions(&f).await,1);
    let restarted=AppState::new(f.s.db.clone(),f.s.config.clone(),"bootstrap-test".into()).await.unwrap();let restarted=app(restarted);
    let(status,result)=call(&restarted,"POST","/api/v1/sync/starts",Some(&f.token),Some(next)).await;assert_eq!(status,StatusCode::OK);assert_eq!(result["phase"],"interrupted");assert!(p[0].rx.try_recv().is_err());assert!(p[1].rx.try_recv().is_err());
    assert_eq!(f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),0);
}

#[tokio::test]
async fn sync_group_invalid_clocks_and_revoked_controller_never_commit_or_credit() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;
    let saved=f.s.clock_filters.write().await.remove(&p[0].endpoint).unwrap();
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(request(&track,&p))).await.0,StatusCode::BAD_REQUEST);assert!(p[0].rx.try_recv().is_err());
    f.s.clock_filters.write().await.insert(p[0].endpoint.clone(),saved);
    let req=request(&track,&p);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req)).await.0,StatusCode::ACCEPTED);let first=receive(&mut p[0]).await;let second=receive(&mut p[1]).await;
    let mut foreign=ready(&first);foreign["start"]["host_clock_id"]=json!(uuid::Uuid::new_v4().to_string());report(&f,&p[0],foreign).await;assert!(p[0].rx.try_recv().is_err());
    report(&f,&p[0],ready(&first)).await;let hash=auth::hash(&f.token);f.s.db.run(move|c|{c.execute("DELETE FROM sessions WHERE hash=?1",[hash])?;Ok(())}).await.unwrap();report(&f,&p[1],ready(&second)).await;
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}assert_eq!(sessions(&f).await,0);
}

#[tokio::test]
async fn group_sync_policy_rejects_missing_capability_and_stops_when_feedback_never_arrives(){
 let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;let mut req=request(&track,&p);req["sync_policy"]=json!({"max_known_error_us":1000,"warmup_ms":1000});
 assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);assert!(p.iter_mut().all(|p|p.rx.try_recv().is_err()));
 {let mut endpoints=f.s.endpoints.write().await;for peer in &p {for cap in ["device_clock_v1","programme_clock_v1"] {endpoints.get_mut(&peer.endpoint).unwrap()["capabilities"][cap]=json!(true);}}}
 assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
 let commands=[receive(&mut p[0]).await,receive(&mut p[1]).await];for (i,c)in commands.iter().enumerate(){report(&f,&p[i],ready(c)).await;}
 let commits=[receive(&mut p[0]).await,receive(&mut p[1]).await];let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let deadline=f.call("GET",&path,None).await.1["deadline_core_ns"].as_u64().unwrap();tokio::time::sleep(std::time::Duration::from_nanos(deadline.saturating_sub(clock::now_ns())+1_000_000)).await;
 for (i,c)in commands.iter().enumerate(){let mut released=ready(c);released["phase"]=json!("released");released["gate"]["phase"]=json!("released");released["gate"]["deadline_host_ns"]=commits[i]["deadline_host_ns"].clone();released["gate"]["released_host_ns"]=commits[i]["deadline_host_ns"].clone();report(&f,&p[i],released).await;}
 sync_start::tick(&f.s).await.unwrap();assert_eq!(f.call("GET",&path,None).await.1["sync_monitor"]["state"],"warming");
 tokio::time::sleep(std::time::Duration::from_millis(1010)).await;sync_start::tick(&f.s).await.unwrap();let result=f.call("GET",&path,None).await.1;
 assert_eq!(result["phase"],"failed");assert_eq!(result["sync_monitor"]["state"],"violated");assert_eq!(result["strict_sync"],false);for peer in &mut p {assert_eq!(receive(peer).await["type"],"cancel_start");}
}
