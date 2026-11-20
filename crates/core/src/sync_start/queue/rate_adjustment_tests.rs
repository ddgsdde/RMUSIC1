//! Synthetic registered channels and SQLite only; no output device.
use super::*;
use std::time::Duration;

#[tokio::test]
async fn bridge_clock_underrun_rebuffers_once_through_final_barrier_without_rate_changes(){
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,32).await;enable_v2(&f,&p).await;
    let mut req=request(&track,&p);req["measured_version"]=json!(2);
    let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;release(&f,&p,&commands,&commits).await;
    report(&f,&p[1],json!({"type":"state","zone_id":p[1].zone,"core_playback_id":commands[1]["playback_id"],"measured_version":2,"frames":9600,"sample_rate":768000,"position":9600./768000.,"status":"playing"})).await;
    let value=json!({"type":"state","zone_id":p[0].zone,"core_playback_id":commands[0]["playback_id"],"track_id":track,"measured_version":2,"frames":4800,"sample_rate":768000,"position":4800./768000.,"status":"playing","continuity_lost":true});
    report(&f,&p[0],value.clone()).await;for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    report(&f,&p[0],value).await;assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()),"repeated underrun report cannot start another recovery");
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let state=f.call("GET",&path,None).await.1;
    assert_eq!(state["phase"],"seeking");assert_eq!(state["buffer_recovery"]["count"],1);assert_eq!(state["buffer_recovery"]["changes_samples"],false);
    for i in 0..2{let frames=if i==0{4800}else{9600};let mut source=boundary(&commands[i]["playback_id"],&commands[i]["track"]["id"],frames,768000,0.);source["ended"]=json!(false);let mut value=final_report_value(&commands[i],&commits[i],frames,json!([source]));value["measured_version"]=json!(2);value["sample_rate"]=json!(768000);report(&f,&p[i],value).await;if i==0{assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()),"one receipt cannot release the group");}}
    for peer in &mut p{let next=receive(peer).await;assert_eq!(next["type"],"prepare_start");assert_ne!(next["pcm_rate_adjustment"],true);assert_ne!(next["automatic_rate_adjustment"],true);assert_eq!(next["position"],4800./768000.,"recovery must not skip the lagging member's unheard samples");}
    assert_eq!(f.call("GET",&path,None).await.1["buffer_recovery"]["count"],1);assert!((total(&f).await-9600./768000.).abs()<1e-9);
}

async fn capabilities(f:&Fixture,p:&[Peer]) {
    enable_v2(f,p).await;let mut endpoints=f.s.endpoints.write().await;
    for peer in p{for key in ["pcm_rate_adjustment_v1","programme_clock_v2"]{endpoints.get_mut(&peer.endpoint).unwrap()["capabilities"][key]=json!(true);}}
}
async fn release(f:&Fixture,p:&[Peer],commands:&[Value],commits:&[Value]) {
    for(i,command)in commands.iter().enumerate(){let mut value=final_report_value(command,&commits[i],0,json!([]));value["measured_version"]=json!(2);value["sample_rate"]=json!(768000);value["phase"]=json!("released");value["final"]=json!(false);report(f,&p[i],value).await;}
}
async fn controller_clock(f:&Fixture,p:&Peer,command:&Value,ppm:f64,time:u64) {
    let frames=4800;let counter=(1u64<<60)+frames;
    let value=json!({"version":1,"core_clock_id":command["start"]["core_clock_id"],"host_clock_id":command["start"]["host_clock_id"],"device_epoch":1,"source":"wasapi_ticks","timestamp_domain":"qpc","counter":counter,"nominal_frequency_hz":10_000_000,"core_sample_ns":time,"core_read_before_ns":time,"core_read_after_ns":time,"host_network_uncertainty_ns":1000,"driver_read_span_ns":100,"valid_until_core_ns":time+500_000_000,"running":true,"device_clock_mapped":true,"driver_accuracy_verified":false,"rate_observation":{"observed_frequency_hz":10_000_000.*(1.+ppm/1e6),"sample_count":4,"span_ns":1_000_000_000u64,"query_time_approximation":false,"rate_sane":true},"programme_clock":{"version":2,"device_epoch":1,"device_counter":counter,"core_sample_ns":time,"core_playback_id":command["playback_id"],"track_id":command["track"]["id"],"sample_rate":768000,"output_frame":frames,"source_frame":frames,"source_fraction":0.,"source_per_output":1.,"track_output_frames":frames,"position":frames as f64/768000.}});
    let mut value=value;value["programme_clock"]["zone_id"]=json!(p.zone);f.s.endpoints.write().await.get_mut(&p.endpoint).unwrap()["device_clock_calibration"]=value;sync_start::observe_device_clock(&f.s,&p.endpoint,&p.tx).await;
}
#[tokio::test]
async fn drift_large_confirmed_phase_realigns_group_without_replaying_rate_targets(){
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,32).await;capabilities(&f,&p).await;
    for peer in &p{f.s.endpoints.write().await.get_mut(&peer.endpoint).unwrap()["capabilities"]["group_rate_lease_v1"]=json!(true);}
    let mut req=request(&track,&p);req["measured_version"]=json!(2);req["pcm_rate_adjustment"]=json!(true);req["automatic_rate_adjustment"]=json!(true);
    let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;release(&f,&p,&commands,&commits).await;
    tokio::time::sleep(Duration::from_millis(70)).await;
    for round in 0..3 {
        let time=clock::now_ns()-100_000;controller_clock(&f,&p[0],&commands[0],0.,time).await;controller_clock(&f,&p[1],&commands[1],100.,time-50_000_000).await;
        sync_start::tick(&f.s).await.unwrap();
        if round<2{assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));tokio::time::sleep(Duration::from_millis(30)).await;}
    }
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let value=f.call("GET",&path,None).await.1;
    assert_eq!(value["phase"],"seeking");assert_eq!(value["transport"]["action"],"resync");assert_eq!(value["rate_adjustment"]["controller"]["realignments"],1);assert_eq!(total(&f).await,0.);
    for i in 0..2{let mut value=final_report_value(&commands[i],&commits[i],0,json!([]));value["measured_version"]=json!(2);value["sample_rate"]=json!(768000);report(&f,&p[i],value).await;}
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"prepare_start");}
    let rearmed=f.call("GET",&path,None).await.1;
    assert_eq!(rearmed["phase"],"preparing");assert_eq!(rearmed["rate_adjustment"]["controller"]["realignments"],1,"rearming must preserve the automatic realignment limit and cooldown");
}
#[tokio::test]
async fn drift_controller_requires_explicit_leases_warms_bounds_targets_and_neutralizes_lost_feedback() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;capabilities(&f,&p).await;
    let mut req=request(&track,&p);req["measured_version"]=json!(2);req["automatic_rate_adjustment"]=json!(true);
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);req["pcm_rate_adjustment"]=json!(true);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);
    for peer in &p{f.s.endpoints.write().await.get_mut(&peer.endpoint).unwrap()["capabilities"]["group_rate_lease_v1"]=json!(true);}
    let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;assert!(commands.iter().all(|c|c["automatic_rate_adjustment"]==true));release(&f,&p,&commands,&commits).await;
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let manual=json!({"request_id":uuid::Uuid::new_v4().to_string(),"zone_id":p[0].zone,"playback_id":commands[0]["playback_id"],"ppm":100.});assert_eq!(f.call("POST",&format!("{path}/rate"),Some(manual)).await.0,StatusCode::CONFLICT);
    for round in 0..3 {
        let time=clock::now_ns()-100_000;for i in 0..2{controller_clock(&f,&p[i],&commands[i],i as f64*100.,time).await;}
        sync_start::tick(&f.s).await.unwrap();
        if round<2{assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));tokio::time::sleep(Duration::from_millis(30)).await;}
    }
    let ready=f.call("GET",&path,None).await.1;assert_eq!(ready["rate_adjustment"]["controller"]["state"],"tracking","{ready}");
    let targets=[receive(&mut p[0]).await,receive(&mut p[1]).await];assert_eq!(targets[0]["ppm"],0.);assert!(targets[1]["ppm"].as_f64().unwrap()<0.&&targets[1]["ppm"].as_f64().unwrap()>=-10.);
    for i in 0..2{assert_eq!(targets[i]["type"],"group_rate");assert_eq!(targets[i]["start"],commands[i]["start"]);let expiry=targets[i]["rate_expires_host_ns"].as_u64().unwrap();assert!(expiry>clock::now_ns()+p[i].offset as u64&&expiry<=clock::now_ns()+p[i].offset as u64+1_500_000_000);}
    let state=f.call("GET",&path,None).await.1;assert_eq!(state["rate_adjustment"]["controller"]["state"],"tracking");assert_eq!(state["clock_diagnostics"]["correction"],"experimental");assert_eq!(state["strict_sync"],false);assert_eq!(total(&f).await,0.);
    for peer in &p{f.s.endpoints.write().await.get_mut(&peer.endpoint).unwrap()["device_clock_calibration"]=Value::Null;}
    tokio::time::sleep(Duration::from_millis(510)).await;sync_start::tick(&f.s).await.unwrap();for peer in &mut p{assert_eq!(receive(peer).await["ppm"],0.);}
    let held=f.call("GET",&path,None).await.1;assert_eq!(held["rate_adjustment"]["controller"]["state"],"held");assert!(held["members"].as_array().unwrap().iter().all(|m|m["requested_rate_ppm"]==0.));
    assert_eq!(f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM sync_controls",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),0,"ephemeral clock targets must not become replayable transport intent");
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":"pause"}))).await.0,StatusCode::ACCEPTED);for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    for i in 0..2{let mut value=final_report_value(&commands[i],&commits[i],0,json!([]));value["measured_version"]=json!(2);value["sample_rate"]=json!(768000);report(&f,&p[i],value).await;}
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":"resume"}))).await.0,StatusCode::ACCEPTED);for peer in &mut p{assert_eq!(receive(peer).await["automatic_rate_adjustment"],true);}
    let reset=f.call("GET",&path,None).await.1;assert_eq!(reset["rate_adjustment"]["controller"]["state"],"inactive");assert!(reset["members"].as_array().unwrap().iter().all(|m|m["requested_rate_ppm"]==0.));
}
#[tokio::test]
async fn group_signal_path_is_published_only_after_current_valid_progress_commits() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;capabilities(&f,&p).await;
    let mut req=request(&track,&p);req["pcm_rate_adjustment"]=json!(true);req["measured_version"]=json!(2);
    let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;release(&f,&p,&commands,&commits).await;
    let state=json!({"type":"state","status":"playing","zone_id":p[0].zone,"track_id":track,"core_playback_id":commands[0]["playback_id"],"measured_version":2,"position":0.,"frames":0,"sample_rate":768000,"signal_path":{"bit_perfect_eligible":false,"processing":["Adaptive PCM sinc f64; output/source clocks separated"]}});
    for bad in [{let mut v=state.clone();v["measured_version"]=json!(1);v},{let mut v=state.clone();v["core_playback_id"]=json!(uuid::Uuid::new_v4().to_string());v},{let mut v=state.clone();v["track_id"]=json!("stale-track");v},{let mut v=state.clone();v["frames"]=json!(7680000);v}] {
        let _lock=f.s.zone_lock.lock().await;sync_start::report(&f.s,&p[0].endpoint,&p[0].tx,&bad).await.unwrap();assert!(f.s.endpoints.read().await[&p[0].endpoint].get("playback").is_none());
    }
    f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER qa_signal_fail BEFORE UPDATE ON sync_starts BEGIN SELECT RAISE(FAIL,'signal rollback'); END;")?;Ok(())}).await.unwrap();
    {let _lock=f.s.zone_lock.lock().await;assert!(sync_start::report(&f.s,&p[0].endpoint,&p[0].tx,&state).await.is_err());}assert!(f.s.endpoints.read().await[&p[0].endpoint].get("playback").is_none());
    f.s.db.run(|c|{c.execute_batch("DROP TRIGGER qa_signal_fail")?;Ok(())}).await.unwrap();report(&f,&p[0],state.clone()).await;
    assert_eq!(f.s.endpoints.read().await[&p[0].endpoint]["playback"]["signal_path"],state["signal_path"]);
}
#[tokio::test]
async fn group_rate_admission_is_explicit_all_member_v2_and_legacy_stays_unmodified() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;capabilities(&f,&p).await;
    let mut req=request(&track,&p);req["pcm_rate_adjustment"]=json!(true);
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);
    req["measured_version"]=json!(2);
    for key in ["pcm_rate_adjustment_v1","programme_clock_v2"] {
        f.s.endpoints.write().await.get_mut(&p[1].endpoint).unwrap()["capabilities"][key]=json!(false);
        assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::BAD_REQUEST);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
        f.s.endpoints.write().await.get_mut(&p[1].endpoint).unwrap()["capabilities"][key]=json!(true);
    }
    req.as_object_mut().unwrap().remove("pcm_rate_adjustment");
    let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;release(&f,&p,&commands,&commits).await;
    assert!(commands.iter().all(|c|c.get("pcm_rate_adjustment").is_none()));let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());
    let state=f.call("GET",&path,None).await.1;assert!(state.get("rate_adjustment").is_none());assert!(state["members"].as_array().unwrap().iter().all(|m|m.get("requested_rate_ppm").is_none()));
    let rate=json!({"request_id":uuid::Uuid::new_v4().to_string(),"zone_id":p[0].zone,"playback_id":commands[0]["playback_id"],"ppm":500.});
    assert_eq!(f.call("POST",&format!("{path}/rate"),Some(rate)).await.0,StatusCode::CONFLICT);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
}
#[tokio::test]
async fn group_rate_target_is_atomic_idempotent_epoch_bound_and_reset_by_rearm() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;capabilities(&f,&p).await;
    let mut req=request(&track,&p);req["pcm_rate_adjustment"]=json!(true);req["measured_version"]=json!(2);
    let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;assert!(commands.iter().all(|c|c["pcm_rate_adjustment"]==true));
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let route=format!("{path}/rate");
    let rate=json!({"request_id":uuid::Uuid::new_v4().to_string(),"zone_id":p[0].zone,"playback_id":commands[0]["playback_id"],"ppm":500.});
    assert_eq!(f.call("POST",&route,Some(rate.clone())).await.0,StatusCode::CONFLICT);release(&f,&p,&commands,&commits).await;
    for bad in [{let mut v=rate.clone();v["ppm"]=json!(500.01);v},{let mut v=rate.clone();v["playback_id"]=json!(uuid::Uuid::new_v4().to_string());v}] {assert!(matches!(f.call("POST",&route,Some(bad)).await.0,StatusCode::BAD_REQUEST|StatusCode::CONFLICT));}
    let before=f.call("GET",&path,None).await.1;let initial_sessions=sessions(&f).await;
    f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER qa_rate_fail BEFORE UPDATE ON sync_starts BEGIN SELECT RAISE(FAIL,'rate rollback'); END;")?;Ok(())}).await.unwrap();
    assert_eq!(f.call("POST",&route,Some(rate.clone())).await.0,StatusCode::INTERNAL_SERVER_ERROR);let after=f.call("GET",&path,None).await.1;for field in ["members","rate_adjustment","transport","phase","listened_seconds"]{assert_eq!(after[field],before[field]);}assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    f.s.db.run(|c|{assert_eq!(c.query_row("SELECT COUNT(*) FROM sync_controls",[],|r|r.get::<_,i64>(0))?,0);c.execute_batch("DROP TRIGGER qa_rate_fail")?;Ok(())}).await.unwrap();
    let(status,value)=f.call("POST",&route,Some(rate.clone())).await;assert_eq!(status,StatusCode::ACCEPTED,"{value}");assert_eq!(value["members"][0]["requested_rate_ppm"],500.);assert_eq!(value["members"][1]["requested_rate_ppm"],0.);assert_eq!(value["transport"],before["transport"]);
    let command=receive(&mut p[0]).await;assert_eq!(command["type"],"group_rate");assert_eq!(command["start"],commands[0]["start"]);assert_eq!(command["playback_id"],commands[0]["playback_id"]);assert_eq!(command["ppm"],500.);
    assert_eq!(f.call("POST",&route,Some(rate.clone())).await.0,StatusCode::OK);let mut conflict=rate.clone();conflict["ppm"]=json!(-500.);assert_eq!(f.call("POST",&route,Some(conflict)).await.0,StatusCode::CONFLICT);
    assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));assert_eq!(total(&f).await,0.);assert_eq!(sessions(&f).await,initial_sessions);
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":"pause"}))).await.0,StatusCode::ACCEPTED);
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    for(i,command)in commands.iter().enumerate(){let mut value=final_report_value(command,&commits[i],0,json!([]));value["measured_version"]=json!(2);value["sample_rate"]=json!(768000);report(&f,&p[i],value).await;}
    let paused=f.call("GET",&path,None).await.1;assert_eq!(paused["phase"],"paused","{paused}");assert_eq!(f.call("POST",&route,Some(rate.clone())).await.0,StatusCode::OK);
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":"resume"}))).await.0,StatusCode::ACCEPTED);
    let next=[receive(&mut p[0]).await,receive(&mut p[1]).await];assert!(next.iter().all(|c|c["type"]=="prepare_start"&&c["pcm_rate_adjustment"]==true));assert_ne!(next[0]["playback_id"],commands[0]["playback_id"]);
    let reset=f.call("POST",&route,Some(rate.clone())).await.1;assert!(reset["rate_adjustment"]["last_request_id"].is_null());assert!(reset["members"].as_array().unwrap().iter().all(|m|m["requested_rate_ppm"]==0.));assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();let router=app(restored);let(status,saved)=call(&router,"POST",&route,Some(&f.token),Some(rate)).await;assert_eq!(status,StatusCode::OK);assert_eq!(saved["phase"],"interrupted");assert!(saved["rate_adjustment"]["last_request_id"].is_null());
}
