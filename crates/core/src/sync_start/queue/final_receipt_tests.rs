//! Private Core/SQLite contracts with synthetic final frames; no device output.
use super::*;
#[path="recovery_tests.rs"]
mod recovery;

async fn reopened(f:&Fixture)->(AppState,axum::Router) {
    let db=crate::db::Db::open(&f.s.config.data.join("db.sqlite")).unwrap();let s=AppState::new(db,f.s.config.clone(),String::new()).await.unwrap();crate::stats::recover(&s).await.unwrap();let router=app(s.clone());(s,router)
}
fn receipt(command:&Value,commit:&Value,peer:&Peer,frames:u64,sources:Value)->Value {
    let mut report=final_report_value(command,commit,frames,sources);report["measured_version"]=json!(2);report["sample_rate"]=json!(768000);
    json!({"request_id":uuid::Uuid::new_v4().to_string(),"endpoint_id":peer.endpoint,"captured_at":chrono::Utc::now().timestamp_millis(),"captured_host_ns":commit["deadline_host_ns"].as_u64().unwrap()+frames*1_000_000_000/768000+1,"report":report})
}
fn source(command:&Value,frames:u64,ended:bool)->Value {json!({"playback_id":command["playback_id"],"track_id":command["track"]["id"],"position":frames as f64/768000.,"frames":frames,"ended":ended})}
async fn seconds(s:&AppState)->f64{s.db.run(|c|Ok(c.query_row("SELECT COALESCE(SUM(play_seconds),0) FROM track_stats",[],|r|r.get(0))?)).await.unwrap()}

#[tokio::test]
async fn group_final_v2_restart_merge_retry_budget_and_frozen_identity() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;let mut req=request(&track,&p);req["measured_version"]=json!(2);let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;
    let path=format!("/api/v1/sync/starts/{}/final-receipts",req["request_id"].as_str().unwrap());let a=receipt(&commands[0],&commits[0],&p[0],4608000,json!([source(&commands[0],4608000,false)]));
    assert_eq!(f.call("POST",&path,Some(a.clone())).await.0,StatusCode::CONFLICT,"active epoch must use live reports");
    let(s,router)=reopened(&f).await;
    // Model a historical epoch without waiting six seconds or opening an output device.
    s.db.run(|c|{c.execute("UPDATE sync_output_epochs SET data=json_set(data,'$.deadline_at_ms',?1)",[chrono::Utc::now().timestamp_millis()-10000])?;Ok(())}).await.unwrap();
    let mut invalid=a.clone();invalid["captured_host_ns"]=commits[0]["deadline_host_ns"].clone();invalid["report"]["programme_frames_played"]=json!(1536000);invalid["report"]["sequence_final"][0]=source(&commands[0],1536000,false);assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(invalid)).await.0,StatusCode::BAD_REQUEST);assert_eq!(seconds(&s).await,0.);
    let(status,result)=call(&router,"POST",&path,Some(&f.token),Some(a.clone())).await;assert_eq!(status,StatusCode::OK,"{result}");assert_eq!(result["applied"],true);assert_eq!(seconds(&s).await,6.);
    assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(a.clone())).await.1["applied"],false);
    let mut changed=a.clone();changed["request_id"]=json!(uuid::Uuid::new_v4().to_string());assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(changed)).await.0,StatusCode::CONFLICT);
    let mut changed=a.clone();changed["captured_at"]=json!(a["captured_at"].as_i64().unwrap()+1);assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(changed)).await.0,StatusCode::CONFLICT);
    let b=receipt(&commands[1],&commits[1],&p[1],4761600,json!([source(&commands[1],4761600,false)]));assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(b.clone())).await.0,StatusCode::OK);assert_eq!(seconds(&s).await,6.2,"cohort maximum, not sum");
    assert_eq!(s.db.run(|c|Ok(c.query_row("SELECT SUM(play_count) FROM track_stats",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),1);
    let(s2,router2)=reopened(&f).await;assert_eq!(call(&router2,"POST",&path,Some(&f.token),Some(b)).await.1["applied"],false);assert_eq!(seconds(&s2).await,6.2);
    let public=call(&router2,"GET",&path.replace("/final-receipts",""),Some(&f.token),None).await.1;assert_eq!(public["phase"],"interrupted");assert_eq!(public["listened_seconds"],6.2);
}

#[tokio::test]
async fn group_final_v2_rolls_back_preloaded_sources_and_requires_original_account() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;let mut req=request(&track,&p);req["measured_version"]=json!(2);req["queue"]=json!([track,track]);let(commands,plans,commits)=begin_v2(&f,&mut p,&req,768000).await;
    let path=format!("/api/v1/sync/starts/{}/final-receipts",req["request_id"].as_str().unwrap());let next=json!({"playback_id":plans[0]["items"][0]["playback_id"],"track":{"id":track}});let value=receipt(&commands[0],&commits[0],&p[0],230400,json!([source(&commands[0],153600,true),source(&next,76800,false)]));let(s,router)=reopened(&f).await;
    s.db.run(|c|{c.execute_batch("CREATE TRIGGER qa_final_fail BEFORE INSERT ON sync_output_final_receipts BEGIN SELECT RAISE(FAIL,'final rollback');END;")?;Ok(())}).await.unwrap();assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(value.clone())).await.0,StatusCode::INTERNAL_SERVER_ERROR);assert_eq!(seconds(&s).await,0.);assert_eq!(s.db.run(|c|Ok(c.query_row("SELECT SUM(frames) FROM sync_output_sources",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),0);
    s.db.run(|c|{c.execute_batch("DROP TRIGGER qa_final_fail")?;Ok(())}).await.unwrap();
    let(status,_)=call(&router,"POST","/api/v1/auth/users",Some(&f.token),Some(json!({"username":"final-other","password":"correct-final-password"}))).await;assert_eq!(status,StatusCode::OK);let(_,login)=call(&router,"POST","/api/v1/auth/login",None,Some(json!({"username":"final-other","password":"correct-final-password"}))).await;assert_eq!(call(&router,"POST",&path,login["token"].as_str(),Some(value.clone())).await.0,StatusCode::FORBIDDEN);
    assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(value)).await.0,StatusCode::OK);assert!((seconds(&s).await-0.3).abs()<1e-12);assert_eq!(s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM play_sessions",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),2);
}

#[tokio::test]
async fn group_final_v2_late_old_epoch_corrects_shared_listen_without_moving_new_source() {
    let f=Fixture::new().await;let(track,_)=f.add_audio().await;let mut p=peers(&f,16).await;enable_v2(&f,&p).await;let mut req=request(&track,&p);req["measured_version"]=json!(2);let(commands,_,commits)=begin_v2(&f,&mut p,&req,768000).await;
    report(&f,&p[0],json!({"type":"state","zone_id":p[0].zone,"core_playback_id":commands[0]["playback_id"],"track_id":track,"status":"playing","measured_version":2,"position":0.1,"frames":76800,"sample_rate":768000})).await;
    let(s,router)=reopened(&f).await;
    // Model the existing rebase contract: a newer source aliases the old logical listen.
    s.db.run(|c|{let grant:String=c.query_row("SELECT grant FROM sync_output_sources LIMIT 1",[],|r|r.get(0))?;let grant:Value=serde_json::from_str(&grant)?;let old=grant["logical_playback_id"].as_str().unwrap();let mut z=crate::models::Zone::new(format!("sync:{}:0",grant["group_id"].as_str().unwrap()),"QA".into(),String::new());z.track=Some(serde_json::from_value(grant["track"].clone())?);z.position=5.;z.status="paused".into();z.renew_playback();let tx=c.transaction()?;crate::stats::restore_sequence_tx(&tx,grant["listener_id"].as_str().unwrap(),old,Some(old),&z)?;z.position=5.3;z.status="playing".into();crate::stats::group_output::record_tx(&tx,&z,grant["listener_id"].as_str().unwrap(),5.,0.3,chrono::Utc::now().timestamp_millis(),None)?;tx.commit()?;Ok(())}).await.unwrap();assert!((seconds(&s).await-0.4).abs()<1e-12);
    let path=format!("/api/v1/sync/starts/{}/final-receipts",req["request_id"].as_str().unwrap());let value=receipt(&commands[0],&commits[0],&p[0],153600,json!([source(&commands[0],153600,true)]));let(status,result)=call(&router,"POST",&path,Some(&f.token),Some(value)).await;assert_eq!(status,StatusCode::OK,"{result}");assert!((seconds(&s).await-0.5).abs()<1e-12);
    let state=s.db.run(|c|Ok(c.query_row("SELECT last_position,last_status,finished_at IS NULL FROM play_sessions",[],|r|Ok((r.get::<_,f64>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?)))?)).await.unwrap();assert_eq!(state,(5.3,"playing".into(),true));
}
