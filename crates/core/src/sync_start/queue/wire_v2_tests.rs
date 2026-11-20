//! Versioned group API/channel receipts, synthetic counters, private SQLite, no audio device.
use super::*;
#[path = "final_receipt_tests.rs"]
mod receipts;
#[path = "rate_adjustment_tests.rs"]
mod rate_adjustment;

#[tokio::test]
async fn group_progress_refreshes_position_timestamp_only_after_valid_frame_commit() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;let mut req=request(&track,&p);req["measured_version"]=json!(2);
    let(commands,_,_)=begin_v2(&f,&mut p,&req,768000).await;
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let before=chrono::Utc::now().timestamp_millis();
    let value=json!({"type":"state","zone_id":p[0].zone,"core_playback_id":commands[0]["playback_id"],"track_id":track,"status":"playing","measured_version":2,"position":0.1,"frames":76800,"sample_rate":768000});
    report(&f,&p[0],value.clone()).await;
    let zone=f.s.zones.read().await[&p[0].zone].clone();assert_eq!(zone.position,0.1);
    assert!((before..=chrono::Utc::now().timestamp_millis()).contains(&zone.updated_at),"fresh actual position must not retain the start timestamp: {}",zone.updated_at);
    let zid=p[0].zone.clone();let stored=f.s.db.run(move|c|Ok(c.query_row("SELECT json_extract(data,'$.updated_at') FROM zones WHERE id=?1",[zid],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(stored,zone.updated_at);
    let mut invalid=value;invalid["position"]=json!(0.2);report(&f,&p[0],invalid).await;
    let unchanged=f.s.zones.read().await[&p[0].zone].clone();assert_eq!(unchanged.position,zone.position);assert_eq!(unchanged.updated_at,zone.updated_at);
}

#[tokio::test]
async fn group_eof_joined_final_from_confirmed_member_does_not_cancel_remaining_output() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;let mut req=request(&track,&p);req["measured_version"]=json!(2);
    let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());
    for(i,command)in commands.iter().enumerate(){let mut value=final_report_value(command,&commits[i],0,json!([]));value["phase"]=json!("released");value["final"]=json!(false);value["measured_version"]=json!(2);value["sample_rate"]=json!(768000);report(&f,&p[i],value).await;}
    for i in 0..2 {
        let frames=if i==0{64}else{65};let position=frames as f64/768000.;let body=json!({"playback_id":commands[i]["playback_id"],"track_id":track,"measured_version":2,"position":position,"frames":frames,"sample_rate":768000});
        let(status,value)=f.call("POST",&format!("/api/v1/zones/{}/ended",p[i].zone),Some(body)).await;assert_eq!(status,StatusCode::OK,"{value}");
        let mut final_value=final_report_value(&commands[i],&commits[i],frames,json!([{"playback_id":commands[i]["playback_id"],"track_id":track,"frames":frames,"position":position,"ended":true}]));final_value["measured_version"]=json!(2);final_value["sample_rate"]=json!(768000);report(&f,&p[i],final_value.clone()).await;report(&f,&p[i],final_value).await;
        let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],if i==0{"playing"}else{"ended"},"{state}");assert!(state["error"].is_null());assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    }
    assert!((total(&f).await-65./768000.).abs()<1e-12);assert_eq!(sessions(&f).await,1);
}

#[tokio::test]
async fn group_authority_v2_sources_and_confirmed_frames_survive_core_restart() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;let mut req=request(&track,&p);req["measured_version"]=json!(2);req["queue"]=json!([track,track]);
    let(commands,plans,_)=begin_v2(&f,&mut p,&req,768000).await;
    report(&f,&p[0],json!({"type":"state","zone_id":p[0].zone,"core_playback_id":commands[0]["playback_id"],"track_id":track,"status":"playing","measured_version":2,"position":0.10005,"frames":76800,"sample_rate":768000})).await;
    let snapshot=f.s.db.run(|c|{let header:String=c.query_row("SELECT data FROM sync_output_epochs",[],|r|r.get(0))?;let mut q=c.prepare("SELECT grant,frames,sample_rate,position FROM sync_output_sources ORDER BY endpoint_id,queue_index")?;let sources=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,u32>(2)?,r.get::<_,f64>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok((header,sources))}).await.unwrap();
    let header:Value=serde_json::from_str(&snapshot.0).unwrap();assert_eq!(header["measured_version"],2);assert_eq!(header["dispatched"],true);assert_eq!(snapshot.1.len(),4);assert_eq!((snapshot.1[0].1,snapshot.1[0].2,snapshot.1[0].3),(76800,768000,0.10005));
    let grant:Value=serde_json::from_str(&snapshot.1[0].0).unwrap();assert_eq!(grant["start"],commands[0]["start"]);assert_eq!(grant["track"]["id"],track);assert!(grant["endpoint_account_id"].as_str().is_some_and(|s|!s.is_empty()));assert!(!plans[0].to_string().contains(grant["logical_playback_id"].as_str().unwrap()));assert!(plans[0]["listen_session_id"].is_null());
    prepare_v2(&f,&p[0],&commands[0]).await;
    let before=total(&f).await;f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER qa_epoch_fail BEFORE UPDATE ON sync_output_epochs BEGIN SELECT RAISE(FAIL,'epoch rollback'); END;")?;Ok(())}).await.unwrap();
    {let _lock=f.s.zone_lock.lock().await;let next=json!({"type":"state","zone_id":p[0].zone,"core_playback_id":commands[0]["playback_id"],"track_id":track,"status":"playing","measured_version":2,"position":0.2001,"frames":153600,"sample_rate":768000});assert!(sync_start::report(&f.s,&p[0].endpoint,&p[0].tx,&next).await.is_err());}
    assert_eq!(total(&f).await,before);f.s.db.run(|c|{c.execute_batch("DROP TRIGGER qa_epoch_fail")?;Ok(())}).await.unwrap();
    let db=crate::db::Db::open(&f.s.config.data.join("db.sqlite")).unwrap();let state=AppState::new(db,f.s.config.clone(),String::new()).await.unwrap();crate::stats::recover(&state).await.unwrap();
    let sources=state.db.run(|c|{let mut q=c.prepare("SELECT grant,frames,sample_rate,position FROM sync_output_sources ORDER BY endpoint_id,queue_index")?;let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,u32>(2)?,r.get::<_,f64>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(rows)}).await.unwrap();assert_eq!(sources,snapshot.1);
    let router=app(state);let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());assert_eq!(call(&router,"GET",&path,Some(&f.token),None).await.1["phase"],"interrupted");
}

#[tokio::test]
async fn group_authority_v2_grant_failure_rolls_back_and_legacy_preparation_stays_untracked() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;let mut req=request(&track,&p);req["measured_version"]=json!(2);req["queue"]=json!([track,track]);
    f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER qa_initial_grant_fail BEFORE INSERT ON sync_output_sources BEGIN SELECT RAISE(FAIL,'initial grant rollback'); END;")?;Ok(())}).await.unwrap();assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::INTERNAL_SERVER_ERROR);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    f.s.db.run(|c|{for table in ["sync_starts","sync_output_epochs","sync_output_sources"]{assert_eq!(c.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get::<_,i64>(0))?,0);}c.execute_batch("DROP TRIGGER qa_initial_grant_fail")?;Ok(())}).await.unwrap();
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req)).await.0,StatusCode::ACCEPTED);let command=receive(&mut p[0]).await;
    f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER qa_grant_fail BEFORE INSERT ON sync_output_sources BEGIN SELECT RAISE(FAIL,'grant rollback'); END;")?;Ok(())}).await.unwrap();
    let path=format!("/api/v1/zones/{}/prepare",p[0].zone);let body=json!({"endpoint_id":p[0].endpoint,"track_id":track,"playback_id":command["playback_id"],"measured_version":2});assert_eq!(f.call("POST",&path,Some(body)).await.0,StatusCode::INTERNAL_SERVER_ERROR);assert!(!f.s.sequences.lock().await.contains_key(&p[0].zone));
    let count=f.s.db.run(|c|{let count=c.query_row("SELECT COUNT(*) FROM sync_output_sources",[],|r|r.get::<_,i64>(0))?;c.execute_batch("DROP TRIGGER qa_grant_fail")?;Ok(count)}).await.unwrap();assert_eq!(count,2);let plan=prepare_v2(&f,&p[0],&command).await;let retry=prepare_v2(&f,&p[0],&command).await;assert_eq!(plan["items"][0]["playback_id"],retry["items"][0]["playback_id"]);assert_eq!(f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM sync_output_sources",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),3);
    let legacy=Fixture::new().await;let(track,_)=legacy.add_audio().await;let mut peers=peers(&legacy,16).await;enable_v2(&legacy,&peers).await;assert_eq!(legacy.call("POST","/api/v1/sync/starts",Some(request(&track,&peers))).await.0,StatusCode::ACCEPTED);let cmd=receive(&mut peers[0]).await;let(status,_)=legacy.call("POST",&format!("/api/v1/zones/{}/prepare",peers[0].zone),Some(json!({"endpoint_id":peers[0].endpoint,"track_id":track,"playback_id":cmd["playback_id"]}))).await;assert_eq!(status,StatusCode::OK);assert_eq!(legacy.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM sync_output_epochs",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),0);
}

#[tokio::test]
async fn group_authority_v2_binds_endpoint_account_separately_from_group_listener() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;
    let(status,user)=f.call("POST","/api/v1/auth/users",Some(json!({"username":"epoch-owner","password":"correct-epoch-password"}))).await;assert_eq!(status,StatusCode::OK);let(status,login)=call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"epoch-owner","password":"correct-epoch-password"}))).await;assert_eq!(status,StatusCode::OK);let token=login["token"].as_str().unwrap();f.s.bridge_owners.write().await.insert(p[0].endpoint.clone(),auth::hash(token));
    let mut req=request(&track,&p);req["measured_version"]=json!(2);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);let cmd=receive(&mut p[0]).await;let path=format!("/api/v1/zones/{}/prepare",p[0].zone);let body=json!({"endpoint_id":p[0].endpoint,"track_id":track,"playback_id":cmd["playback_id"],"measured_version":2});assert_eq!(f.call("POST",&path,Some(body.clone())).await.0,StatusCode::FORBIDDEN);assert_eq!(call(&f.app,"POST",&path,Some(token),Some(body)).await.0,StatusCode::OK);
    let grant=f.s.db.run(|c|Ok(c.query_row("SELECT grant FROM sync_output_sources",[],|r|r.get::<_,String>(0))?)).await.unwrap();let grant:Value=serde_json::from_str(&grant).unwrap();assert_eq!(grant["endpoint_account_id"],user["id"]);assert_ne!(grant["endpoint_account_id"],grant["listener_id"]);let public=f.call("GET",&format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap()),None).await.1;assert!(!public.to_string().contains(user["id"].as_str().unwrap()));
}

async fn enable_v2(f:&Fixture,p:&[Peer]) {
    enable(f,p).await;let mut endpoints=f.s.endpoints.write().await;
    for peer in p {let caps=&mut endpoints.get_mut(&peer.endpoint).unwrap()["capabilities"];caps["measured_listens_v2"]=json!(true);caps["group_output_v2"]=json!(true);}
}
async fn prepare_v2(f:&Fixture,p:&Peer,command:&Value)->Value {
    let(status,value)=f.call("POST",&format!("/api/v1/zones/{}/prepare",p.zone),Some(json!({"endpoint_id":p.endpoint,"track_id":command["track"]["id"],"playback_id":command["playback_id"],"measured_version":2}))).await;
    assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["measured_version"],2);assert_eq!(value["continuous_recovery"],false);assert!(value["listen_session_id"].is_null());value
}
#[tokio::test]
async fn group_wire_v2_admission_canonical_retry_and_unreleased_final_are_version_bound() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;
    let mut req=request(&track,&p);req["measured_version"]=json!(2);
    f.s.endpoints.write().await.get_mut(&p[1].endpoint).unwrap()["capabilities"]["group_output_v2"]=json!(false);
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    enable_v2(&f,&p).await;let(status,result)=f.call("POST","/api/v1/sync/starts",Some(req.clone())).await;
    assert_eq!(status,StatusCode::ACCEPTED,"{result}");assert_eq!(result["measured_version"],2);assert_eq!(result["queue"]["length"],1);
    let commands=[receive(&mut p[0]).await,receive(&mut p[1]).await];
    for(i,command)in commands.iter().enumerate(){assert_eq!(command["measured_version"],2);assert_eq!(command["group_queue"],true);let old=json!({"endpoint_id":p[i].endpoint,"track_id":track,"playback_id":command["playback_id"]});assert_eq!(f.call("POST",&format!("/api/v1/zones/{}/prepare",p[i].zone),Some(old.clone())).await.0,StatusCode::BAD_REQUEST);prepare_v2(&f,&p[i],command).await;}
    let mut canonical=req.clone();canonical["queue"]=json!([track]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(canonical)).await.0,StatusCode::OK);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());f.call("POST",&format!("{path}/cancel"),None).await;for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    for(i,command)in commands.iter().enumerate(){let mut value=final_report_value(command,&json!({"deadline_host_ns":0}),0,json!([]));value["sample_rate"]=json!(0);value["gate"]["phase"]=json!("cancelled");value["gate"]["released_host_ns"]=json!(0);report(&f,&p[i],value.clone()).await;assert_eq!(f.call("GET",&path,None).await.1["settled"],false);value["measured_version"]=json!(2);report(&f,&p[i],value.clone()).await;report(&f,&p[i],value).await;}
    assert_eq!(f.call("GET",&path,None).await.1["settled"],true);assert_eq!(sessions(&f).await,0);assert_eq!(total(&f).await,0.);
}


async fn begin_v2(f:&Fixture,p:&mut[Peer],req:&Value,rate:u32)->(Vec<Value>,Vec<Value>,Vec<Value>) {
    let(status,result)=f.call("POST","/api/v1/sync/starts",Some(req.clone())).await;assert_eq!(status,StatusCode::ACCEPTED,"{result}");
    let mut commands=Vec::new();let mut plans=Vec::new();let mut commits=Vec::new();
    for peer in p.iter_mut(){let command=receive(peer).await;plans.push(prepare_v2(f,peer,&command).await);commands.push(command);}
    report(f,&p[0],ready(&commands[0])).await;
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let state=f.call("GET",&path,None).await.1;assert!(state["members"].as_array().unwrap().iter().all(|member|member["phase"]=="preparing"));
    for (i,command) in commands.iter().enumerate(){let mut value=ready(command);value["measured_version"]=json!(2);value["sample_rate"]=json!(rate);report(f,&p[i],value).await;}
    for peer in p.iter_mut(){let commit=receive(peer).await;assert_eq!(commit["type"],"commit_start");commits.push(commit);}
    let deadline=f.call("GET",&path,None).await.1["deadline_core_ns"].as_u64().unwrap();tokio::time::sleep(std::time::Duration::from_nanos(deadline.saturating_sub(clock::now_ns())+1_000_000)).await;(commands,plans,commits)
}
fn curve(command:&Value,plan:&Value,frames:u64,current_frames:u64,ratio:f64)->Value {
    let rate=768000;let mut ended=boundary(&command["playback_id"],&command["track"]["id"],frames,rate,0.);ended["position"]=json!(frames as f64/rate as f64*ratio);
    let mut current=boundary(&plan["items"][0]["playback_id"],&plan["items"][0]["track"]["id"],current_frames,rate,0.);current["position"]=json!(current_frames as f64/rate as f64*ratio);
    json!({"endpoint_id":command["endpoint_id"],"plan_id":plan["plan_id"],"request_id":uuid::Uuid::new_v4().to_string(),"from_playback_id":command["playback_id"],"measured_version":2,"sample_rate":rate,"ended":[ended],"current":current})
}
#[tokio::test]
async fn group_wire_v2_768k_handoff_progress_and_natural_eof_count_once_across_distinct_source_rates() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;f.s.db.run(|c|{c.execute("UPDATE tracks SET sample_rate=768000,bit_depth=32,duration=8.0",[])?;Ok(())}).await.unwrap();
    let mut req=request(&track,&p);req["measured_version"]=json!(2);req["queue"]=json!([track,track]);let(commands,plans,_)=begin_v2(&f,&mut p,&req,768000).await;
    tokio::time::sleep(std::time::Duration::from_millis(5300)).await;
    let first=curve(&commands[0],&plans[0],4608000,76800,1.0005);let path=format!("/api/v1/zones/{}/handoff",p[0].zone);
    let mut bad=first.clone();bad["ended"][0]["position"]=json!(6.006);assert_eq!(f.call("POST",&path,Some(bad)).await.0,StatusCode::BAD_REQUEST);assert_eq!(total(&f).await,0.);
    let mut legacy=first.clone();legacy.as_object_mut().unwrap().remove("measured_version");legacy["ended"][0]["position"]=json!(6.);legacy["current"]["position"]=json!(0.1);assert_eq!(f.call("POST",&path,Some(legacy)).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(f.call("POST",&path,Some(first.clone())).await.1["applied"],true);assert_eq!(f.call("POST",&path,Some(first.clone())).await.1["applied"],false);
    let second=curve(&commands[1],&plans[1],4531200,115200,0.9995);assert_eq!(f.call("POST",&format!("/api/v1/zones/{}/handoff",p[1].zone),Some(second)).await.1["applied"],true);assert!((total(&f).await-6.15).abs()<1e-12);
    for (i,frames,ratio) in [(0,153600,1.0005),(1,153599,0.9995)] {
        let position=frames as f64/768000.*ratio;let mut body=boundary(&plans[i]["items"][0]["playback_id"],&json!(track),frames,768000,0.);body["position"]=json!(position);body["sample_rate"]=json!(768000);body["measured_version"]=json!(2);
        let mut old=body.clone();old.as_object_mut().unwrap().remove("measured_version");old["position"]=json!(frames as f64/768000.);assert_eq!(f.call("POST",&format!("/api/v1/zones/{}/ended",p[i].zone),Some(old)).await.0,StatusCode::BAD_REQUEST);
        let state=json!({"type":"state","zone_id":p[i].zone,"core_playback_id":body["playback_id"],"track_id":track,"status":"playing","measured_version":2,"position":position,"frames":frames,"sample_rate":768000});report(&f,&p[i],state).await;
        let(status,value)=f.call("POST",&format!("/api/v1/zones/{}/ended",p[i].zone),Some(body.clone())).await;assert_eq!(status,StatusCode::OK,"peer {i} first EOF {body}: {value}");let(status,value)=f.call("POST",&format!("/api/v1/zones/{}/ended",p[i].zone),Some(body)).await;assert_eq!(status,StatusCode::OK,"peer {i} repeated EOF: {value}");
    }
    assert!((total(&f).await-6.2).abs()<1e-12);assert_eq!(sessions(&f).await,2);let(count,open)=f.s.db.run(|c|Ok((c.query_row("SELECT SUM(play_count) FROM track_stats",[],|r|r.get::<_,i64>(0))?,c.query_row("SELECT COUNT(*) FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get::<_,i64>(0))?))).await.unwrap();assert_eq!((count,open),(1,0));
    assert_eq!(f.call("POST",&path,Some(first)).await.1["applied"],false);let state=f.call("GET",&format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap()),None).await.1;assert_eq!(state["phase"],"ended");assert_eq!(state["measured_version"],2);assert!((state["listened_seconds"].as_f64().unwrap()-6.2).abs()<1e-12);
}
#[tokio::test]
async fn group_wire_v2_pause_final_tail_and_draining_preparation_never_issue_individual_claims() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;let mut req=request(&track,&p);req["measured_version"]=json!(2);let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;
    for i in 0..2 {report(&f,&p[i],json!({"type":"state","zone_id":p[i].zone,"core_playback_id":commands[i]["playback_id"],"track_id":track,"status":"playing","measured_version":2,"position":0.1*(if i==0{1.0005}else{0.9995}),"frames":76800,"sample_rate":768000})).await;}
    let endpoints=f.s.endpoints.read().await.clone();let mut filters=f.s.clock_filters.write().await;for (id,(_,filter)) in filters.iter_mut(){let now=clock::now_ns();let estimate=&endpoints[id]["clock_calibration"];let mut reply=filter.probe(id,now);reply["host_clock_id"]=estimate["host_clock_id"].clone();let offset=estimate["host_minus_core_ns"].as_i64().unwrap();reply["t2_ns"]=json!((now as i128+offset as i128+1000) as u64);reply["t3_ns"]=json!((now as i128+offset as i128+2000) as u64);assert!(filter.accept(&reply,now+3000));}drop(filters);
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let(status,value)=f.call("POST",&format!("{path}/control"),Some(json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":"pause"}))).await;assert_eq!(status,StatusCode::ACCEPTED,"{value}");for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    let(status,value)=f.call("POST",&format!("/api/v1/zones/{}/prepare",p[0].zone),Some(json!({"endpoint_id":p[0].endpoint,"track_id":track,"playback_id":commands[0]["playback_id"],"measured_version":2}))).await;assert_eq!(status,StatusCode::OK);assert_eq!(value["accepted"],false);
    for i in 0..2 {let frames=153600-i as u64;let ratio=if i==0{1.0005}else{0.9995};let source=json!({"playback_id":commands[i]["playback_id"],"track_id":track,"position":frames as f64/768000.*ratio,"frames":frames,"ended":false});let mut value=final_report_value(&commands[i],&commits[i],frames,json!([source]));value["sample_rate"]=json!(768000);report(&f,&p[i],value.clone()).await;value["measured_version"]=json!(2);report(&f,&p[i],value.clone()).await;report(&f,&p[i],value).await;}
    let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],"paused","{state}");assert!((total(&f).await-0.2).abs()<1e-12);assert_eq!(sessions(&f).await,1);let claims=f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM listen_claims",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(claims,1);
}

#[path="device_group_tests.rs"]
mod logical_devices;
