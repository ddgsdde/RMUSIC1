//! Real Core/SQLite coordinator; registered channels, no sound device.
use super::*;

async fn extra(f:&Fixture)->Peer {
    let endpoint="sync-peer-2".to_owned();let zone="sync-zone-2".to_owned();let offset=9_000_000_000i64;let(tx,rx)=mpsc::channel(32);
    f.s.save_zone(models::Zone::new(zone.clone(),zone.clone(),endpoint.clone())).await.unwrap();
    f.s.bridges.write().await.insert(endpoint.clone(),tx.clone());f.s.bridge_owners.write().await.insert(endpoint.clone(),auth::hash(&f.token));
    let mut filter=clock::Filter::default();let now=clock::now_ns();let mut reply=filter.probe(&endpoint,now);reply["host_clock_id"]=json!(uuid::Uuid::new_v4().to_string());reply["t2_ns"]=json!(now+offset as u64+1000);reply["t3_ns"]=json!(now+offset as u64+2000);assert!(filter.accept(&reply,now+3000));
    f.s.endpoints.write().await.insert(endpoint.clone(),json!({"id":endpoint,"online":true,"capabilities":{"scheduled_start_v1":true,"scheduled_queue_v1":true,"continuous_pcm":true,"host_clock_v1":true,"media_url_api_relative":true,"playback_instance":true},"clock_calibration":filter.estimate(now+3000)}));
    f.s.clock_filters.write().await.insert(endpoint.clone(),(tx.clone(),filter));Peer{endpoint,zone,offset,tx,rx}
}
fn members(from:&[&Peer],to:&[&Peer])->Value {json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":"members","from_zone_ids":from.iter().map(|p|&p.zone).collect::<Vec<_>>(),"zone_ids":to.iter().map(|p|&p.zone).collect::<Vec<_>>()})}
async fn close(f:&Fixture,p:&[Peer],commands:&[Value],commits:&[Value],frames:&[u64]){for(i,peer)in p.iter().enumerate(){report(f,peer,final_value(&commands[i],&commits[i],frames[i],48000,false,true)).await;}}

#[tokio::test]
async fn group_members_live_join_remove_and_paused_join_keep_one_listen(){
    for queued in [false,true] {
        let(f,_,mut p,_,path)=setup(queued).await;let(first,commits)=epoch(&f,&mut p,48000).await;let mut third=extra(&f).await;
        let request=members(&[&p[0],&p[1]],&[&third,&p[1],&p[0]]);assert_eq!(control(&f,&path,request.clone()).await["phase"],"seeking");stops(&mut p).await;
        assert!(sync_start::active_zone(&f.s,&third.zone).await);assert!(third.rx.try_recv().is_err());
        report(&f,&p[0],final_value(&first[0],&commits[0],4800,48000,false,true)).await;assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));assert!(third.rx.try_recv().is_err());
        report(&f,&p[1],final_value(&first[1],&commits[1],7200,48000,false,true)).await;p.push(third);
        let(joined,commits)=epoch(&f,&mut p,48000).await;assert!(joined.iter().all(|c|c["position"]==0.15));
        assert_eq!(f.call("POST",&format!("{path}/control"),Some(request)).await.0,StatusCode::OK);assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));
        let stale=members(&[&p[0],&p[1]],&[&p[0]]);assert_eq!(f.call("POST",&format!("{path}/control"),Some(stale)).await.0,StatusCode::CONFLICT);
        control(&f,&path,members(&[&p[0],&p[1],&p[2]],&[&p[2]])).await;stops(&mut p).await;close(&f,&p,&joined,&commits,&[4800,9600,7200]).await;
        for (peer,position) in [(&p[0],0.25),(&p[1],0.35)]{let zone=&f.s.zones.read().await[&peer.zone];assert_eq!(zone.status,"stopped");assert!(zone.playback_id.is_none());assert!((zone.position-position).abs()<1e-10);assert!(!sync_start::active_zone(&f.s,&peer.zone).await);}
        let mut current=vec![p.pop().unwrap()];let(single,commits)=epoch(&f,&mut current,48000).await;assert_eq!(single[0]["position"],0.35);
        control(&f,&path,operation("pause")).await;stops(&mut current).await;close(&f,&current,&single,&commits,&[4800]).await;
        let rejoin=members(&[&current[0]],&[&current[0],&p[0]]);assert_eq!(control(&f,&path,rejoin).await["phase"],"paused");assert!(current[0].rx.try_recv().is_err());assert!(p[0].rx.try_recv().is_err());
        current.push(p.remove(0));control(&f,&path,operation("resume")).await;let(resumed,commits)=epoch(&f,&mut current,48000).await;assert!(resumed.iter().all(|c|(c["position"].as_f64().unwrap()-0.45).abs()<1e-10));
        f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut current).await;close(&f,&current,&resumed,&commits,&[4800,7200]).await;
        assert!((total(&f).await-0.6).abs()<1e-10);assert_eq!(sessions(&f).await,1);assert_eq!(open_sessions(&f).await,0);
        let state=f.call("GET",&path,None).await.1;assert!((state["listened_seconds"].as_f64().unwrap()-0.6).abs()<1e-10);
        let skips=f.s.db.run(|c|Ok(c.query_row("SELECT SUM(skip_count) FROM track_stats",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(skips,1);
    }
}

#[tokio::test]
async fn group_members_admission_rejects_without_stopping_and_pending_outputs_are_reserved(){
    let(f,track,mut p,_,path)=setup(true).await;let(_first,_commits)=epoch(&f,&mut p,48000).await;let mut third=extra(&f).await;let body=members(&[&p[0],&p[1]],&[&p[0],&third]);
    for field in ["scheduled_queue_v1","continuous_pcm"] {
        f.s.endpoints.write().await.get_mut(&third.endpoint).unwrap()["capabilities"][field]=json!(false);
        assert_eq!(f.call("POST",&format!("{path}/control"),Some(body.clone())).await.0,StatusCode::BAD_REQUEST);
        f.s.endpoints.write().await.get_mut(&third.endpoint).unwrap()["capabilities"][field]=json!(true);
    }
    let mut bad=body.clone();bad["position"]=json!(0);assert_eq!(f.call("POST",&format!("{path}/control"),Some(bad)).await.0,StatusCode::BAD_REQUEST);
    let mut bad=body.clone();bad["zone_ids"]=json!([p[0].zone,p[0].zone]);assert_eq!(f.call("POST",&format!("{path}/control"),Some(bad)).await.0,StatusCode::BAD_REQUEST);
    assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));assert!(third.rx.try_recv().is_err());assert!(!sync_start::active_zone(&f.s,&third.zone).await);
    control(&f,&path,body).await;stops(&mut p).await;assert!(sync_start::active_zone(&f.s,&third.zone).await);
    let mut other=request(&track,&p);other["zone_ids"]=json!([third.zone,"zz-idle"]);let(status,error)=f.call("POST","/api/v1/sync/starts",Some(other)).await;assert_eq!(status,StatusCode::BAD_REQUEST);assert!(error.to_string().contains("音区或端点正在另一组播放中"));
    {let _lock=f.s.zone_lock.lock().await;sync_start::peer_changed(&f.s,&third.endpoint).await.unwrap();}
    assert_eq!(f.call("GET",&path,None).await.1["phase"],"failed");assert!(!sync_start::active_zone(&f.s,&third.zone).await);assert!(third.rx.try_recv().is_err());
}

#[tokio::test]
async fn group_members_transaction_rollback_retries_after_final_frames_without_replaying_old_output(){
    let(f,_,mut p,_,path)=setup(true).await;let(first,commits)=epoch(&f,&mut p,48000).await;let mut third=extra(&f).await;
    let request=members(&[&p[0],&p[1]],&[&p[0],&p[1],&third]);control(&f,&path,request.clone()).await;stops(&mut p).await;
    report(&f,&p[0],final_value(&first[0],&commits[0],4800,48000,false,true)).await;
    f.s.db.run(|c|{c.execute_batch("CREATE TRIGGER reject_members BEFORE UPDATE ON zones BEGIN SELECT RAISE(ABORT,'membership rollback'); END;")?;Ok(())}).await.unwrap();
    {let _lock=f.s.zone_lock.lock().await;assert!(sync_start::report(&f.s,&p[1].endpoint,&p[1].tx,&final_value(&first[1],&commits[1],7200,48000,false,true)).await.is_err());}
    let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],"seeking");assert_eq!(state["members"].as_array().unwrap().len(),2);assert!(sync_start::active_zone(&f.s,&third.zone).await);
    assert!(third.rx.try_recv().is_err());assert!(p.iter_mut().all(|peer|peer.rx.try_recv().is_err()));assert!((total(&f).await-0.15).abs()<1e-10);
    assert_eq!(f.call("POST",&format!("{path}/control"),Some(request)).await.0,StatusCode::OK);
    f.s.db.run(|c|{c.execute_batch("DROP TRIGGER reject_members")?;Ok(())}).await.unwrap();sync_start::tick(&f.s).await.unwrap();p.push(third);
    let(next,commits)=epoch(&f,&mut p,48000).await;assert!(next.iter().all(|c|c["position"]==0.15));
    {let _lock=f.s.zone_lock.lock().await;assert!(!sync_start::report(&f.s,&p[0].endpoint,&p[0].tx,&final_value(&first[0],&Value::Null,9600,48000,false,false)).await.unwrap());}
    f.call("POST",&format!("{path}/cancel"),None).await;stops(&mut p).await;close(&f,&p,&next,&commits,&[4800,7200,9600]).await;
    assert_eq!(sessions(&f).await,1);assert_eq!(open_sessions(&f).await,0);assert!((total(&f).await-0.35).abs()<1e-10);
}

#[tokio::test]
async fn group_members_last_eof_keeps_the_finished_group_and_schema_describes_every_action(){
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,32).await;enable(&f,&p).await;
    let mut req=request(&track,&p);req["queue"]=json!([track]);let path=format!("/api/v1/sync/starts/{}",req["request_id"].as_str().unwrap());assert_eq!(f.call("POST","/api/v1/sync/starts",Some(req)).await.0,StatusCode::ACCEPTED);
    let(first,commits)=epoch(&f,&mut p,48000).await;let mut third=extra(&f).await;control(&f,&path,members(&[&p[0],&p[1]],&[&p[0],&p[1],&third])).await;stops(&mut p).await;
    finals(&f,&p,&first,&commits,[4800,4800],48000,true).await;let state=f.call("GET",&path,None).await.1;assert_eq!(state["phase"],"ended");assert_eq!(state["transport"]["state"],"failed");assert_eq!(state["members"].as_array().unwrap().len(),2);assert!(!sync_start::active_zone(&f.s,&third.zone).await);assert!(third.rx.try_recv().is_err());assert_eq!(open_sessions(&f).await,0);
    let schema=f.call("GET","/api/v1/openapi.json",None).await.1;let branches=schema["components"]["schemas"]["SyncControlRequest"]["oneOf"].as_array().unwrap();
    for action in ["seek","jump","members"]{assert!(branches.iter().any(|b|b["properties"]["action"]["const"]==action));}
    assert_eq!(schema["components"]["schemas"]["SyncControlRequest"]["properties"]["zone_ids"]["minItems"],1);
}
