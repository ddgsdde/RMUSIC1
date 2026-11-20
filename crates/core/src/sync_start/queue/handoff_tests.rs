use super::*;
#[path = "control_tests.rs"]
mod controls;
#[path = "output_tests.rs"]
mod outputs;
#[path = "wire_v2_tests.rs"]
mod wire_v2;

async fn begin(f:&Fixture,p:&mut[Peer],req:&Value,rate:u32)->(Vec<Value>,Vec<Value>,Vec<Value>) {
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
    ready_segment(f,p,rate).await
}
async fn ready_segment(f:&Fixture,p:&mut[Peer],rate:u32)->(Vec<Value>,Vec<Value>,Vec<Value>) {
    let mut commands=Vec::new();let mut plans=Vec::new();let mut commits=Vec::new();
    for peer in p.iter_mut(){let command=receive(peer).await;let plan=prepare(f,peer,&command).await;commands.push(command);plans.push(plan);}
    for(i,command)in commands.iter().enumerate(){let mut report_value=ready(command);report_value["sample_rate"]=json!(rate);report(f,&p[i],report_value).await;}
    for peer in p.iter_mut(){let commit=receive(peer).await;assert_eq!(commit["type"],"commit_start");commits.push(commit);}
    let path=format!("/api/v1/sync/starts/{}",commands[0]["start"]["plan_id"].as_str().unwrap());let value=f.call("GET",&path,None).await.1;assert_eq!(value["phase"],"committed");
    let deadline=value["deadline_core_ns"].as_u64().unwrap();tokio::time::sleep(std::time::Duration::from_nanos(deadline.saturating_sub(clock::now_ns())+1_000_000)).await;
    (commands,plans,commits)
}
fn boundary(pb:&Value,track:&Value,frames:u64,rate:u32,base:f64)->Value {json!({"playback_id":pb,"track_id":track,"position":base+frames as f64/rate as f64,"frames":frames})}
fn handoff(command:&Value,plan:&Value,from:usize,ended_frames:u64,current_frames:u64)->Value {
    let (pb,track,base)=if from==0{(&command["playback_id"],&command["track"]["id"],command["position"].as_f64().unwrap())}else{(&plan["items"][from-1]["playback_id"],&plan["items"][from-1]["track"]["id"],0.)};
    json!({"endpoint_id":command["endpoint_id"],"request_id":uuid::Uuid::new_v4().to_string(),"plan_id":plan["plan_id"],"from_playback_id":pb,"sample_rate":48000,
        "ended":[boundary(pb,track,ended_frames,48000,base)],"current":boundary(&plan["items"][from]["playback_id"],&plan["items"][from]["track"]["id"],current_frames,48000,0.)})
}
async fn total(f:&Fixture)->f64 {f.s.db.run(|c|Ok(c.query_row("SELECT COALESCE(SUM(play_seconds),0) FROM track_stats",[],|r|r.get(0))?)).await.unwrap()}
async fn eof(f:&Fixture,p:&Peer,pb:&Value,track:&Value,frames:u64,rate:u32)->Value {
    let mut body=boundary(pb,track,frames,rate,0.);body["sample_rate"]=json!(rate);let(status,result)=f.call("POST",&format!("/api/v1/zones/{}/ended",p.zone),Some(body)).await;assert_eq!(status,StatusCode::OK,"{result}");result
}
fn final_report_value(command:&Value,commit:&Value,frames:u64,sources:Value)->Value {
    json!({"type":"start_state","version":1,"start":command["start"],"phase":"cancelled","final":true,"zone_id":command["zone_id"],"core_playback_id":command["playback_id"],"sample_rate":48000,"programme_frames_played":frames,"sequence_final":sources,
        "gate":{"phase":"released","expires_host_ns":command["start"]["expires_host_ns"],"deadline_host_ns":commit["deadline_host_ns"],"released_host_ns":commit["deadline_host_ns"],"ready_host_ns":ready(command)["gate"]["ready_host_ns"]}})
}
fn final_source(pb:&Value,track:&Value,frames:u64,ended:bool)->Value {let mut value=boundary(pb,track,frames,48000,0.);value["ended"]=json!(ended);value}

#[tokio::test]
async fn sync_queue_ordered_handoffs_merge_each_repeated_occurrence_and_natural_eof_once() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable(&f,&p).await;
    f.s.db.run(|c|{c.execute("UPDATE tracks SET duration=8.0",[])?;Ok(())}).await.unwrap();
    let mut req=request(&track,&p);req["queue"]=json!([track,track,track]);req["position"]=json!(0.05);
    let(commands,plans,_)=begin(&f,&mut p,&req,48000).await;
    // Exercise the real five-second counting threshold and delayed multi-track receipts.
    tokio::time::sleep(std::time::Duration::from_secs(17)).await;
    let first=handoff(&commands[0],&plans[0],0,288000,288000);let path=format!("/api/v1/zones/{}/handoff",p[0].zone);
    let mut invalid=first.clone();invalid["current"]["playback_id"]=plans[0]["items"][1]["playback_id"].clone();assert_eq!(f.call("POST",&path,Some(invalid)).await.0,StatusCode::BAD_REQUEST);assert_eq!(total(&f).await,0.);
    assert_eq!(f.call("POST",&path,Some(first.clone())).await.1["applied"],true);
    let b=handoff(&commands[1],&plans[1],0,264000,278400);assert_eq!(f.call("POST",&format!("/api/v1/zones/{}/handoff",p[1].zone),Some(b)).await.1["applied"],true);
    for(i,ended,current)in [(0,288000,288000),(1,278400,283200)]{let body=handoff(&commands[i],&plans[i],1,ended,current);assert_eq!(f.call("POST",&format!("/api/v1/zones/{}/handoff",p[i].zone),Some(body)).await.1["applied"],true);}
    for(i,frames)in [(0,288000),(1,283200)]{eof(&f,&p[i],&plans[i]["items"][1]["playback_id"],&json!(track),frames,48000).await;}
    assert!((total(&f).await-18.).abs()<1e-12);assert_eq!(sessions(&f).await,3);
    let count=f.s.db.run(|c|Ok(c.query_row("SELECT SUM(play_count) FROM track_stats",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(count,3);
    let result=f.call("GET",&format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap()),None).await.1;assert_eq!(result["phase"],"ended");assert_eq!(result["settled"],true);assert_eq!(result["queue"]["length"],3);
    assert_eq!(f.call("POST",&path,Some(first.clone())).await.1["applied"],false);let mut conflict=first;conflict["current"]["frames"]=json!(288001);conflict["current"]["position"]=json!(288001./48000.);assert_eq!(f.call("POST",&path,Some(conflict)).await.0,StatusCode::CONFLICT);
    eof(&f,&p[0],&plans[0]["items"][1]["playback_id"],&json!(track),288000,48000).await;assert!((total(&f).await-18.).abs()<1e-12);
}

#[tokio::test]
async fn sync_queue_atomic_handoff_and_final_corrections_survive_db_failure_and_new_group() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable(&f,&p).await;let mut req=request(&track,&p);req["queue"]=json!([track,track,track]);
    let(commands,plans,commits)=begin(&f,&mut p,&req,48000).await;let body=handoff(&commands[0],&plans[0],0,9600,4800);let path=format!("/api/v1/zones/{}/handoff",p[0].zone);
    f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER fail_queue_receipt BEFORE INSERT ON sequence_receipts BEGIN SELECT RAISE(ABORT,'queue test'); END;")?;Ok(())}).await.unwrap();
    assert_eq!(f.call("POST",&path,Some(body.clone())).await.0,StatusCode::INTERNAL_SERVER_ERROR);assert_eq!(f.s.zones.read().await[&p[0].zone].playback_id.as_deref(),commands[0]["playback_id"].as_str());assert_eq!(total(&f).await,0.);assert_eq!(sessions(&f).await,1);
    f.s.db.run(|c|{c.execute_batch("DROP TRIGGER fail_queue_receipt")?;Ok(())}).await.unwrap();assert_eq!(f.call("POST",&path,Some(body)).await.1["applied"],true);
    let old_path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());f.call("POST",&format!("{old_path}/cancel"),None).await;for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    let mut newer=request(&track,&p);newer["queue"]=json!([track,track,track]);let(new_commands,_,_)=begin(&f,&mut p,&newer,48000).await;
    let final_a=final_report_value(&commands[0],&commits[0],19200,json!([final_source(&plans[0]["items"][0]["playback_id"],&json!(track),9600,false)]));
    let mut wrong_epoch=final_a.clone();wrong_epoch["core_playback_id"]=plans[0]["items"][0]["playback_id"].clone();report(&f,&p[0],wrong_epoch).await;assert_eq!(f.call("GET",&old_path,None).await.1["members"][0]["final_report_received"],false);assert!((total(&f).await-0.3).abs()<1e-12);
    let old_id=req["request_id"].as_str().unwrap().to_owned();f.s.db.run(move|c|{c.execute_batch(&format!("CREATE TRIGGER fail_final BEFORE UPDATE ON sync_starts WHEN NEW.id='{old_id}' BEGIN SELECT RAISE(ABORT,'final test'); END;"))?;Ok(())}).await.unwrap();
    {let _lock=f.s.zone_lock.lock().await;assert!(sync_start::report(&f.s,&p[0].endpoint,&p[0].tx,&final_a).await.is_err());}
    assert!((total(&f).await-0.3).abs()<1e-12);assert_eq!(f.call("GET",&old_path,None).await.1["members"][0]["final_report_received"],false);
    f.s.db.run(|c|{c.execute_batch("DROP TRIGGER fail_final")?;Ok(())}).await.unwrap();report(&f,&p[0],final_a.clone()).await;
    let final_b=final_report_value(&commands[1],&commits[1],24000,json!([final_source(&commands[1]["playback_id"],&json!(track),12000,true),final_source(&plans[1]["items"][0]["playback_id"],&json!(track),7200,true),final_source(&plans[1]["items"][1]["playback_id"],&json!(track),4800,false)]));report(&f,&p[1],final_b.clone()).await;
    report(&f,&p[0],final_a).await;report(&f,&p[1],final_b).await;assert!((total(&f).await-0.55).abs()<1e-12);assert_eq!(sessions(&f).await,4);
    let old=f.call("GET",&old_path,None).await.1;assert_eq!(old["settled"],true);assert_eq!(old["phase"],"cancelled");
    for(i,peer)in p.iter().enumerate(){assert_eq!(f.s.zones.read().await[&peer.zone].playback_id.as_deref(),new_commands[i]["playback_id"].as_str());}
    let open=f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(open,1);
    let(replacement,_)=mpsc::channel(16);f.s.bridges.write().await.insert(p[0].endpoint.clone(),replacement);eof(&f,&p[0],&new_commands[0]["playback_id"],&json!(track),4800,48000).await;assert!((total(&f).await-0.55).abs()<1e-12);assert_eq!(f.s.zones.read().await[&p[0].zone].playback_id.as_deref(),new_commands[0]["playback_id"].as_str());
}

#[tokio::test]
async fn sync_queue_format_barrier_rearms_all_members_at_768k_and_restart_closes_each_occurrence() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable(&f,&p).await;
    let mut high=library::get_track(&f.s,track.clone()).await.unwrap();high.id="queue-768k".into();high.sample_rate=768000;high.bit_depth=32;high.channels=2;let saved=high.clone();f.s.db.run(move|c|library::upsert(c,&saved,None,None,0,0,None,None)).await.unwrap();
    let mut req=request(&track,&p);req["queue"]=json!([track,high.id,high.id]);let(commands,plans,_)=begin(&f,&mut p,&req,48000).await;assert!(plans.iter().all(|plan|plan["items"].as_array().unwrap().is_empty()));
    eof(&f,&p[0],&commands[0]["playback_id"],&json!(track),9600,48000).await;assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    eof(&f,&p[1],&commands[1]["playback_id"],&json!(track),9600,48000).await;
    let(high_commands,high_plans,_)=ready_segment(&f,&mut p,768000).await;
    for(i,command)in high_commands.iter().enumerate(){assert_eq!(command["track"]["sample_rate"],768000);assert_eq!(command["position"],0.);assert_ne!(command["playback_id"],commands[i]["playback_id"]);assert!(command["start"]["generation"].as_u64().unwrap()>commands[i]["start"]["generation"].as_u64().unwrap());}
    eof(&f,&p[0],&commands[0]["playback_id"],&json!(track),9600,48000).await;assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
    report(&f,&p[0],json!({"type":"state","zone_id":p[0].zone,"core_playback_id":high_commands[0]["playback_id"],"status":"playing","frames":76800,"sample_rate":768000,"position":0.1})).await;
    let mut body=handoff(&high_commands[0],&high_plans[0],0,76800,38400);body["sample_rate"]=json!(768000);body["ended"]=json!([boundary(&high_commands[0]["playback_id"],&json!(high.id),76800,768000,0.)]);body["current"]=boundary(&high_plans[0]["items"][0]["playback_id"],&json!(high.id),38400,768000,0.);
    assert_eq!(f.call("POST",&format!("/api/v1/zones/{}/handoff",p[0].zone),Some(body)).await.1["applied"],true);assert_ne!(f.s.zones.read().await[&p[0].zone].playback_id.as_deref(),high_commands[0]["playback_id"].as_str());assert!((total(&f).await-0.35).abs()<1e-12);
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();let restored_app=app(restored.clone());let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());let result=call(&restored_app,"GET",&path,Some(&f.token),None).await.1;assert_eq!(result["phase"],"interrupted");assert_eq!(result["settled"],true);
    for peer in &p{assert!(restored.zones.read().await[&peer.zone].playback_id.is_none());assert_eq!(restored.zones.read().await[&peer.zone].status,"stopped");}assert_eq!(restored.zones.read().await[&p[0].zone].queue_index,2);assert_eq!(restored.zones.read().await[&p[0].zone].position,0.05);
    let open=f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM play_sessions WHERE finished_at IS NULL",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(open,0);assert_eq!(sessions(&f).await,3);assert!((total(&f).await-0.35).abs()<1e-12);
}

#[tokio::test]
async fn sync_queue_ready_without_owned_preload_reservations_never_commits_or_credits() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,8).await;enable(&f,&p).await;let mut req=request(&track,&p);req["queue"]=json!([track,track]);assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
    for peer in p.iter_mut(){let command=receive(peer).await;report(&f,peer,ready(&command)).await;}
    for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    assert_eq!(sessions(&f).await,0);let result=f.call("GET",&format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap()),None).await.1;assert_eq!(result["phase"],"failed");assert_eq!(result["may_have_started"],false);
}

#[tokio::test]
async fn sync_queue_next_group_generation_exceeds_retired_format_segment() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable(&f,&p).await;
    let mut high=library::get_track(&f.s,track.clone()).await.unwrap();high.id="generation-768k".into();high.sample_rate=768000;high.bit_depth=32;high.channels=2;
    let saved=high.clone();f.s.db.run(move|c|library::upsert(c,&saved,None,None,0,0,None,None)).await.unwrap();
    let mut req=request(&track,&p);req["queue"]=json!([track,high.id]);let(commands,_,_)=begin(&f,&mut p,&req,48000).await;
    for i in 0..p.len(){eof(&f,&p[i],&commands[i]["playback_id"],&json!(track),4800,48000).await;}
    let(high_commands,_,_)=ready_segment(&f,&mut p,768000).await;
    for i in 0..p.len(){eof(&f,&p[i],&high_commands[i]["playback_id"],&json!(high.id),76800,768000).await;}
    let mut next=request(&track,&p);next["queue"]=json!([track]);
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(next)).await.0,StatusCode::ACCEPTED);
    for(i,peer)in p.iter_mut().enumerate(){let command=receive(peer).await;
        assert!(command["start"]["generation"].as_u64().unwrap()>high_commands[i]["start"]["generation"].as_u64().unwrap(),"new group reused a retired native generation");
    }
}

#[tokio::test]
async fn sync_queue_prepared_zero_frame_final_ledger_settles_without_listening_or_commit() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable(&f,&p).await;let mut req=request(&track,&p);req["queue"]=json!([track,track]);
    assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req.clone())).await.0,StatusCode::ACCEPTED);
    let mut commands=Vec::new();for peer in &mut p{let command=receive(peer).await;prepare(&f,peer,&command).await;commands.push(command);}
    report(&f,&p[0],ready(&commands[0])).await;
    let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());assert_eq!(f.call("GET",&path,None).await.1["phase"],"preparing");
    f.call("POST",&format!("{path}/cancel"),None).await;for peer in &mut p{assert_eq!(receive(peer).await["type"],"cancel_start");}
    for(i,peer)in p.iter().enumerate(){let command=&commands[i];let mut value=final_report_value(command,&Value::Null,0,json!([{"playback_id":command["playback_id"],"track_id":track,"position":req["position"],"frames":0,"ended":false}]));
        value["gate"]["phase"]=json!("cancelled");value["gate"]["deadline_host_ns"]=json!(0);value["gate"]["released_host_ns"]=json!(0);report(&f,peer,value).await;}
    let result=f.call("GET",&path,None).await.1;assert!(result["members"].as_array().unwrap().iter().all(|member|member["final_report_received"]==true),"prepared zero-frame ledger was rejected: {result}");
    assert_eq!(result["settled"],true);assert_eq!(result["may_have_started"],false);assert_eq!(sessions(&f).await,0);assert_eq!(total(&f).await,0.);
}
