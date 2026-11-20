//! Real Axum/SQLite and registered channels; no device output is opened.
use super::*;
#[path = "member_tests.rs"]
mod members;
#[path = "clock_tests.rs"]
mod clocks;

fn operation(action:&str)->Value{json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":action})}
fn jump(from:usize,to:usize,position:f64)->Value{json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":"jump","from_queue_index":from,"queue_index":to,"position":position})}
async fn open_sessions(f:&Fixture)->i64{f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get(0))?)).await.unwrap()}
async fn refresh_clocks(f:&Fixture){
    let endpoints=f.s.endpoints.read().await.clone();let mut filters=f.s.clock_filters.write().await;
    for (id,(_,filter)) in filters.iter_mut(){let now=clock::now_ns();let estimate=&endpoints[id]["clock_calibration"];let mut reply=filter.probe(id,now);reply["host_clock_id"]=estimate["host_clock_id"].clone();let offset=estimate["host_minus_core_ns"].as_i64().unwrap();reply["t2_ns"]=json!((now as i128+offset as i128+1000) as u64);reply["t3_ns"]=json!((now as i128+offset as i128+2000) as u64);assert!(filter.accept(&reply,now+3000));}
}
async fn control(f:&Fixture,path:&str,body:Value)->Value{
    refresh_clocks(f).await;
    let(status,value)=f.call("POST",&format!("{path}/control"),Some(body)).await;assert_eq!(status,StatusCode::ACCEPTED,"{value}");value
}
async fn epoch(f:&Fixture,p:&mut[Peer],rate:u32)->(Vec<Value>,Vec<Value>){
    let mut commands=Vec::new();let mut commits=Vec::new();
    for peer in p.iter_mut(){let command=receive(peer).await;assert_eq!(command["type"],"prepare_start");if command["group_queue"]==true{prepare(f,peer,&command).await;}commands.push(command);}
    for(i,command)in commands.iter().enumerate(){let mut value=ready(command);value["sample_rate"]=json!(rate);report(f,&p[i],value).await;}
    for peer in p.iter_mut(){let commit=receive(peer).await;assert_eq!(commit["type"],"commit_start");commits.push(commit);}
    let path=format!("/api/v1/sync/starts/{}",commands[0]["start"]["plan_id"].as_str().unwrap());let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],"committed","{state}");
    let deadline=state["deadline_core_ns"].as_u64().unwrap();tokio::time::sleep(std::time::Duration::from_nanos(deadline.saturating_sub(clock::now_ns())+1_000_000)).await;(commands,commits)
}
async fn stops(p:&mut[Peer]){for peer in p{assert_eq!(receive(peer).await["type"],"cancel_start");}}
fn final_value(command:&Value,commit:&Value,frames:u64,rate:u32,ended:bool,released:bool)->Value{
    let base=command["position"].as_f64().unwrap();let mut source=boundary(&command["playback_id"],&command["track"]["id"],frames,rate,base);source["ended"]=json!(ended);
    let mut value=final_report_value(command,commit,frames,json!([source]));value["sample_rate"]=json!(rate);
    if !released{value["gate"]["phase"]=json!("cancelled");value["gate"]["deadline_host_ns"]=json!(0);value["gate"]["released_host_ns"]=json!(0);}
    value
}
async fn finals(f:&Fixture,p:&[Peer],commands:&[Value],commits:&[Value],frames:[u64;2],rate:u32,ended:bool){
    for(i,peer)in p.iter().enumerate(){report(f,peer,final_value(&commands[i],&commits[i],frames[i],rate,ended,true)).await;}
}
async fn progress(f:&Fixture,p:&Peer,command:&Value,frames:u64,rate:u32){
    report(f,p,json!({"type":"state","zone_id":p.zone,"core_playback_id":command["playback_id"],"status":"playing","frames":frames,"sample_rate":rate,"position":command["position"].as_f64().unwrap()+frames as f64/rate as f64})).await;
}
async fn setup(queued:bool)->(Fixture,String,Vec<Peer>,Value,String){
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;f.s.db.run(|c|{c.execute("UPDATE tracks SET duration=8.0",[])?;Ok(())}).await.unwrap();let p=peers(&f,32).await;
    if queued{enable(&f,&p).await;}let mut req=request(&track,&p);if queued{req["queue"]=json!([track,track]);}
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);(f,track,p,req,path)
}
#[tokio::test]
async fn group_resync_waits_for_both_final_cursors_rearms_once_and_preserves_listening_identity(){
    let(f,_,mut p,_,path)=setup(true).await;let(first,commits)=epoch(&f,&mut p,48000).await;
    let request=operation("resync");let state=control(&f,&path,request.clone()).await;assert_eq!(state["phase"],"seeking");stops(&mut p).await;
    report(&f,&p[0],final_value(&first[0],&commits[0],4800,48000,false,true)).await;
    assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()),"one final cursor cannot restart either room");
    report(&f,&p[1],final_value(&first[1],&commits[1],7200,48000,false,true)).await;
    let(next,next_commits)=epoch(&f,&mut p,48000).await;
    assert!(next.iter().all(|c|c["position"]==0.15));assert_ne!(next[0]["playback_id"],first[0]["playback_id"]);
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(request)).await.0,StatusCode::OK);
    assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()),"retry cannot submit a second resync");
    assert!((total(&f).await-0.15).abs()<1e-10);assert_eq!(sessions(&f).await,1);
    f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;finals(&f,&p,&next,&next_commits,[4800,4800],48000,false).await;
    assert!((total(&f).await-0.25).abs()<1e-10);assert_eq!(sessions(&f).await,1);assert_eq!(open_sessions(&f).await,0);
}
#[tokio::test]
async fn group_paused_seek_then_jump_keeps_rebased_source_position_without_extra_listening(){
    let(f,_,mut p,_,path)=setup(true).await;let(first,commits)=epoch(&f,&mut p,48000).await;control(&f,&path,operation("pause")).await;stops(&mut p).await;finals(&f,&p,&first,&commits,[4800,7200],48000,false).await;
    for position in [1.2,0.05]{
        control(&f,&path,json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":"seek","queue_index":0,"position":position})).await;
        let request=jump(0,1,0.);let next=control(&f,&path,request.clone()).await;assert_eq!(next["playhead"]["queue_index"],1);assert_eq!(next["phase"],"paused");
        assert_eq!(f.call("POST",&format!("{path}/control"),Some(request)).await.0,StatusCode::OK);
        control(&f,&path,jump(1,0,0.)).await;assert_eq!(sessions(&f).await,1);assert_eq!(open_sessions(&f).await,0);assert!((total(&f).await-0.15).abs()<1e-10);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    }
}
#[tokio::test]
async fn group_jump_paused_replay_preserves_actual_totals_without_future_phantom_sessions(){
    let(f,_,mut p,_,path)=setup(true).await;let(first,commits)=epoch(&f,&mut p,48000).await;control(&f,&path,operation("pause")).await;stops(&mut p).await;finals(&f,&p,&first,&commits,[4800,7200],48000,false).await;
    let request=jump(0,1,0.);assert_eq!(control(&f,&path,request.clone()).await["phase"],"paused");assert_eq!(open_sessions(&f).await,0);assert_eq!(sessions(&f).await,1);assert!((total(&f).await-0.15).abs()<1e-10);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(request)).await.0,StatusCode::OK);assert_eq!(f.call("POST",&format!("{path}/control"),Some(jump(0,1,0.))).await.0,StatusCode::CONFLICT);
    control(&f,&path,operation("resume")).await;let(second,commits)=epoch(&f,&mut p,48000).await;assert!(second.iter().all(|command|command["track"]["id"]==first[0]["track"]["id"]));control(&f,&path,operation("pause")).await;stops(&mut p).await;finals(&f,&p,&second,&commits,[9600,14400],48000,false).await;
    control(&f,&path,jump(1,0,0.05)).await;assert_eq!(open_sessions(&f).await,0);assert_eq!(sessions(&f).await,2);control(&f,&path,operation("resume")).await;let(replayed,commits)=epoch(&f,&mut p,48000).await;
    assert!(replayed.iter().all(|command|command["position"]==0.05));{let _lock=f.s.zone_lock.lock().await;assert!(!sync_start::report(&f.s,&p[0].endpoint,&p[0].tx,&final_value(&first[0],&Value::Null,9600,48000,false,false)).await.unwrap());}
    f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;finals(&f,&p,&replayed,&commits,[4800,7200],48000,false).await;
    assert_eq!(sessions(&f).await,3,"replaying index 0 must not create a future index-1 session");assert_eq!(open_sessions(&f).await,0);assert!((total(&f).await-0.6).abs()<1e-10);assert!((f.call("GET",&path,None).await.1["listened_seconds"].as_f64().unwrap()-0.6).abs()<1e-10);
    let skips=f.s.db.run(|c|Ok(c.query_row("SELECT SUM(skip_count) FROM track_stats",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(skips,3,"later controls must not turn a previously skipped occurrence into a natural end");
}
#[tokio::test]
async fn group_jump_waits_for_all_final_frames_changes_format_and_can_return_from_last_eof(){
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,32).await;enable(&f,&p).await;
    let mut high=library::get_track(&f.s,track.clone()).await.unwrap();high.id="jump-768k".into();high.sample_rate=768000;high.bit_depth=32;let saved=high.clone();f.s.db.run(move|c|library::upsert(c,&saved,None,None,0,0,None,None)).await.unwrap();
    let mut req=request(&track,&p);req["queue"]=json!([track,high.id,track]);let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let(first,_,commits)=begin(&f,&mut p,&req,48000).await;
    control(&f,&path,jump(0,1,0.1)).await;stops(&mut p).await;report(&f,&p[0],final_value(&first[0],&commits[0],4800,48000,false,true)).await;assert_eq!(f.call("GET",&path,None).await.1["phase"],"seeking");assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));report(&f,&p[1],final_value(&first[1],&commits[1],7200,48000,false,true)).await;
    let(high_commands,high_commits)=epoch(&f,&mut p,768000).await;assert!(high_commands.iter().all(|command|command["position"]==0.1&&command["track"]["sample_rate"]==768000));
    control(&f,&path,jump(1,2,0.)).await;stops(&mut p).await;finals(&f,&p,&high_commands,&high_commits,[7680,11520],768000,false).await;let(last,last_commits)=epoch(&f,&mut p,48000).await;
    control(&f,&path,jump(2,0,0.)).await;stops(&mut p).await;finals(&f,&p,&last,&last_commits,[9600,12000],48000,true).await;let(back,back_commits)=epoch(&f,&mut p,48000).await;assert!(back.iter().all(|command|command["track"]["id"]==json!(track)));
    f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;finals(&f,&p,&back,&back_commits,[4800,7200],48000,false).await;assert_eq!(sessions(&f).await,4);assert_eq!(open_sessions(&f).await,0);assert!((total(&f).await-0.565).abs()<1e-10);
}
#[tokio::test]
async fn group_jump_rollback_keeps_the_old_session_and_rejects_invalid_targets(){
    let(f,_,mut p,_,path)=setup(true).await;let(first,commits)=epoch(&f,&mut p,48000).await;control(&f,&path,operation("pause")).await;stops(&mut p).await;finals(&f,&p,&first,&commits,[4800,4800],48000,false).await;
    for body in [jump(0,0,0.),jump(0,99,0.)]{assert_eq!(f.call("POST",&format!("{path}/control"),Some(body)).await.0,StatusCode::BAD_REQUEST);}
    let request=jump(0,1,0.);f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER reject_group_jump BEFORE UPDATE ON sync_starts BEGIN SELECT RAISE(ABORT,'jump rollback'); END;")?;Ok(())}).await.unwrap();
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(request.clone())).await.0,StatusCode::INTERNAL_SERVER_ERROR);assert_eq!(open_sessions(&f).await,1);assert_eq!(f.call("GET",&path,None).await.1["playhead"]["queue_index"],0);assert!((total(&f).await-0.1).abs()<1e-10);
    f.s.db.run(|c|{c.execute_batch("DROP TRIGGER reject_group_jump")?;Ok(())}).await.unwrap();control(&f,&path,request).await;assert_eq!(open_sessions(&f).await,0);assert_eq!(sessions(&f).await,1);
}
#[tokio::test]
async fn sync_group_power_manual_controls_refresh_but_automatic_format_rearm_keeps_the_original_grant() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,32).await;enable(&f,&p).await;
    let mut high=library::get_track(&f.s,track.clone()).await.unwrap();high.id="power-768k".into();high.sample_rate=768000;high.bit_depth=32;let saved=high.clone();f.s.db.run(move|c|library::upsert(c,&saved,None,None,0,0,None,None)).await.unwrap();
    let host=uuid::Uuid::new_v4().to_string();
    for peer in &p{let mut endpoints=f.s.endpoints.write().await;let endpoint=endpoints.get_mut(&peer.endpoint).unwrap();endpoint["capabilities"]["group_power_v1"]=json!(true);endpoint["group_power"]=json!({"host_id":host,"generation":1,"wake_generation":1});endpoint["group_power_clock_id"]=endpoint["clock_calibration"]["host_clock_id"].clone();}
    let mut req=request(&track,&p);req["queue"]=json!([track,high.id]);let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let(commands,_,_)=begin(&f,&mut p,&req,48000).await;
    for peer in &p{f.s.endpoints.write().await.get_mut(&peer.endpoint).unwrap()["group_power"]=json!({"host_id":host,"generation":2,"wake_generation":2});}
    for(i,peer)in p.iter().enumerate(){eof(&f,peer,&commands[i]["playback_id"],&json!(track),9600,48000).await;}
    let(auto,_,auto_commits)=ready_segment(&f,&mut p,768000).await;for(i,command)in auto.iter().enumerate(){assert_eq!(command["start"]["group_power"],commands[i]["start"]["group_power"],"automatic format rearm must never acquire a new wake grant");}
    control(&f,&path,operation("pause")).await;stops(&mut p).await;finals(&f,&p,&auto,&auto_commits,[0,0],768000,false).await;assert_eq!(f.call("GET",&path,None).await.1["phase"],"paused");
    let resume=operation("resume");control(&f,&path,resume.clone()).await;let(resumed,resume_commits)=epoch(&f,&mut p,768000).await;assert!(resumed.iter().all(|command|command["start"]["group_power"]["generation"]==2));
    for peer in &p{f.s.endpoints.write().await.get_mut(&peer.endpoint).unwrap()["group_power"]=json!({"host_id":host,"generation":3,"wake_generation":3});}
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(resume)).await.0,StatusCode::OK);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    let mut seek=operation("seek");seek["queue_index"]=json!(1);seek["position"]=json!(0.1);control(&f,&path,seek).await;stops(&mut p).await;finals(&f,&p,&resumed,&resume_commits,[0,0],768000,false).await;
    let(seeked,commits)=epoch(&f,&mut p,768000).await;assert!(seeked.iter().all(|command|command["start"]["group_power"]["generation"]==3));f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;finals(&f,&p,&seeked,&commits,[0,0],768000,false).await;
}

#[tokio::test]
async fn group_control_pause_resume_seek_preserves_one_count_and_actual_positions(){
    for queued in [true,false]{
        let(f,_,mut p,_,path)=setup(queued).await;let(commands,commits)=epoch(&f,&mut p,48000).await;
        tokio::time::sleep(std::time::Duration::from_millis(4900)).await;
        progress(&f,&p[0],&commands[0],230400,48000).await;progress(&f,&p[1],&commands[1],235200,48000).await;
        let pause=operation("pause");assert_eq!(control(&f,&path,pause.clone()).await["phase"],"pausing");stops(&mut p).await;
        assert_eq!(f.call("POST",&format!("{path}/control"),Some(operation("resume"))).await.0,StatusCode::CONFLICT);
        finals(&f,&p,&commands,&commits,[235200,240000],48000,false).await;
        let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],"paused","{state}");assert_eq!(state["playhead"]["position"],5.);assert_eq!(state["listened_seconds"],5.);assert_eq!(total(&f).await,5.);assert_eq!(sessions(&f).await,1);
        assert_eq!(f.call("GET","/api/v1/sync/starts?active=true",None).await.1["total"],1);sync_start::tick(&f.s).await.unwrap();assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));assert_eq!(total(&f).await,5.);
        let mut seek=operation("seek");seek["queue_index"]=json!(0);seek["position"]=json!(6.5);assert_eq!(control(&f,&path,seek.clone()).await["phase"],"paused");
        let stored=f.s.db.run(|c|Ok(c.query_row("SELECT last_position FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get::<_,f64>(0))?)).await.unwrap();assert_eq!(stored,6.5);
        let retry=f.call("POST",&format!("{path}/control"),Some(pause)).await;assert_eq!(retry.0,StatusCode::OK);assert_eq!(retry.1["transport"]["request_id"],seek["request_id"]);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
        assert_eq!(control(&f,&path,operation("resume")).await["phase"],"preparing");let(resumed,resume_commits)=epoch(&f,&mut p,48000).await;
        for(i,command)in resumed.iter().enumerate(){assert_eq!(command["position"],6.5);assert_ne!(command["playback_id"],commands[i]["playback_id"]);assert!(command["start"]["generation"].as_u64().unwrap()>commands[i]["start"]["generation"].as_u64().unwrap());}
        {let _lock=f.s.zone_lock.lock().await;assert!(!sync_start::report(&f.s,&p[0].endpoint,&p[0].tx,&final_value(&commands[0],&commits[0],240000,48000,false,true)).await.unwrap());}
        progress(&f,&p[0],&resumed[0],4800,48000).await;
        let mut back=operation("seek");back["queue_index"]=json!(0);back["position"]=json!(0.25);assert_eq!(control(&f,&path,back).await["phase"],"seeking");stops(&mut p).await;
        finals(&f,&p,&resumed,&resume_commits,[9600,7200],48000,false).await;
        let(seeked,seek_commits)=epoch(&f,&mut p,48000).await;assert!(seeked.iter().all(|command|command["position"]==0.25));
        f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;finals(&f,&p,&seeked,&seek_commits,[4800,2400],48000,false).await;
        assert!((total(&f).await-5.3).abs()<1e-10);assert_eq!(sessions(&f).await,1);
        let(count,skip,open)=f.s.db.run(|c|Ok((c.query_row("SELECT SUM(play_count) FROM track_stats",[],|r|r.get::<_,i64>(0))?,c.query_row("SELECT SUM(skip_count) FROM track_stats",[],|r|r.get::<_,i64>(0))?,c.query_row("SELECT COUNT(*) FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get::<_,i64>(0))?))).await.unwrap();assert_eq!((count,skip,open),(1,1,0));
    }
}

#[tokio::test]
async fn group_control_prepared_pause_has_no_listen_and_restart_never_replays(){
    for queued in [true,false]{
        let(f,_,mut p,req,path)=setup(queued).await;let mut commands=Vec::new();for peer in &mut p{let command=receive(peer).await;if queued{prepare(&f,peer,&command).await;}commands.push(command);}
        let pause=operation("pause");control(&f,&path,pause.clone()).await;stops(&mut p).await;
        let mut bad=final_value(&commands[0],&Value::Null,0,48000,false,false);bad["sequence_final"][0]["position"]=json!(3.);
        if queued{report(&f,&p[0],bad).await;assert_eq!(f.call("GET",&path,None).await.1["members"][0]["final_report_received"],false);}
        for(i,peer)in p.iter().enumerate(){report(&f,peer,final_value(&commands[i],&Value::Null,0,48000,false,false)).await;}
        let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],"paused","{state}");assert_eq!(state["may_have_started"],false);assert_eq!(sessions(&f).await,0);assert_eq!(total(&f).await,0.);
        let mut wrong=pause.clone();wrong["action"]=json!("resume");assert_eq!(f.call("POST",&format!("{path}/control"),Some(wrong)).await.0,StatusCode::CONFLICT);
        assert_eq!(f.call("POST","/api/v1/sync/starts",Some(request(req["track_id"].as_str().unwrap(),&p))).await.0,StatusCode::BAD_REQUEST);
        let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();let app=app(restored.clone());
        let value=call(&app,"POST",&format!("{path}/control"),Some(&f.token),Some(pause)).await;assert_eq!(value.0,StatusCode::OK);assert_eq!(value.1["phase"],"interrupted");assert_eq!(value.1["transport"]["state"],"failed");assert_eq!(sessions(&f).await,0);assert!(restored.zones.read().await.values().filter(|z|p.iter().any(|peer|peer.zone==z.id)).all(|z|z.playback_id.is_none()));
    }
}

#[tokio::test]
async fn group_control_pause_transaction_failure_is_retryable_without_dispatch(){
    for queued in [true,false]{
        let(f,_,mut p,_,path)=setup(queued).await;let(_,_) =epoch(&f,&mut p,48000).await;let pause=operation("pause");
        f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER fail_group_control BEFORE INSERT ON sync_controls BEGIN SELECT RAISE(ABORT,'control test'); END;")?;Ok(())}).await.unwrap();
        assert_eq!(f.call("POST",&format!("{path}/control"),Some(pause.clone())).await.0,StatusCode::INTERNAL_SERVER_ERROR);assert!(!matches!(f.call("GET",&path,None).await.1["phase"].as_str(),Some("pausing"|"paused")));assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
        f.s.db.run(|c|{c.execute_batch("DROP TRIGGER fail_group_control")?;Ok(())}).await.unwrap();assert_eq!(control(&f,&path,pause.clone()).await["phase"],"pausing");stops(&mut p).await;
        assert_eq!(f.call("POST",&format!("{path}/control"),Some(pause)).await.0,StatusCode::OK);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    }
}

#[tokio::test]
async fn group_control_missing_final_fails_without_resuming_or_creating_listens(){
    let(f,_,mut p,_,path)=setup(true).await;for peer in &mut p{let command=receive(peer).await;prepare(&f,peer,&command).await;}
    control(&f,&path,operation("pause")).await;stops(&mut p).await;tokio::time::sleep(std::time::Duration::from_millis(6050)).await;sync_start::tick(&f.s).await.unwrap();
    let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],"failed");assert_eq!(state["transport"]["incomplete_final_reports"],true);assert_eq!(f.call("POST",&format!("{path}/control"),Some(operation("resume"))).await.0,StatusCode::CONFLICT);assert_eq!(sessions(&f).await,0);assert_eq!(total(&f).await,0.);
}

#[tokio::test]
async fn group_control_cancel_paused_queue_closes_the_existing_listen_after_optional_seek(){
    for seek in [false,true]{
        let(f,_,mut p,_,path)=setup(true).await;let(commands,commits)=epoch(&f,&mut p,48000).await;
        control(&f,&path,operation("pause")).await;stops(&mut p).await;finals(&f,&p,&commands,&commits,[4800,2400],48000,false).await;
        assert_eq!(f.call("GET",&path,None).await.1["phase"],"paused");
        if seek{let mut body=operation("seek");body["queue_index"]=json!(0);body["position"]=json!(6.5);control(&f,&path,body).await;}
        let value=f.call("POST",&format!("{path}/cancel"),None).await.1;assert_eq!(value["phase"],"cancelled");assert_eq!(value["settled"],true);
        let open=f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(open,0,"paused cancellation leaked a group session");
        assert_eq!(sessions(&f).await,1);assert!((total(&f).await-0.1).abs()<1e-12);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    }
}

#[tokio::test]
async fn group_control_paused_format_boundary_resumes_all_members_at_768k(){
    let(f,track,mut p,req,path)=setup(true).await;f.call("POST",&format!("{path}/cancel"),None).await;for peer in &mut p{receive(peer).await;receive(peer).await;}
    let mut high=library::get_track(&f.s,track.clone()).await.unwrap();high.id="control-768k".into();high.sample_rate=768000;high.bit_depth=32;high.channels=2;let saved=high.clone();f.s.db.run(move|c|library::upsert(c,&saved,None,None,0,0,None,None)).await.unwrap();
    let mut req=req;req["request_id"]=json!(uuid::Uuid::new_v4().to_string());req["queue"]=json!([track,high.id]);let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req)).await.0,StatusCode::ACCEPTED);
    let(commands,commits)=epoch(&f,&mut p,48000).await;control(&f,&path,operation("pause")).await;stops(&mut p).await;finals(&f,&p,&commands,&commits,[4800,4800],48000,true).await;
    let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],"paused");assert_eq!(state["playhead"],json!({"queue_index":1,"position":0.}));assert_eq!(sessions(&f).await,1);
    control(&f,&path,operation("resume")).await;let(commands,commits)=epoch(&f,&mut p,768000).await;assert!(commands.iter().all(|command|command["track"]["sample_rate"]==768000&&command["position"]==0.));
    f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;finals(&f,&p,&commands,&commits,[7680,7680],768000,false).await;assert_eq!(sessions(&f).await,2);assert!((total(&f).await-0.11).abs()<1e-12);
}
