use super::*;
async fn register_v2(router:&Router,token:&str) {
 let(status,v)=call(router,"POST","/api/v1/endpoints/browser",Some(token),Some(json!({"id":"browser-continuous","name":"v2 durable","playback_instance":true,"continuous_pcm":true,"continuous_recovery":true,"recovery_controls":true,"measured_listens_v2":true,"measured_listens_recovery_v2":true}))).await;assert_eq!(status,StatusCode::OK,"{v}");
}
async fn prepare_output(router:&Router,token:&str,z:&Value)->Value {
 let(status,p)=call(router,"POST",&format!("{ZONE}/prepare"),Some(token),Some(json!({"endpoint_id":z["endpoint_id"],"track_id":z["track"]["id"],"playback_id":z["playback_id"],"measured_version":2}))).await;assert_eq!(status,StatusCode::OK,"{p}");assert_eq!(p["continuous_recovery"],true);p
}
#[tokio::test]
async fn dual_recovery_v2_restart_keeps_source_position_version_and_idempotent_queue() {
 let f=fixture(&["local-shared","local-unknown"]).await;register_v2(&f.app,&f.token).await;let z=zone(&f).await;let p=prepare_output(&f.app,&f.token,&z).await;
 let slot=&p["items"][0];let mut first=boundary(&z["playback_id"],&z["track"]["id"],10.005);first["frames"]=json!(480000);let mut second=boundary(&slot["playback_id"],&slot["track"]["id"],0.50025);second["frames"]=json!(24000);
 let mut body=request(&z,&p,vec![first,second],false);body["measured_version"]=json!(2);let(_,router)=restart(&f).await;register_v2(&router,&f.token).await;
 let mut wrong=body.clone();wrong.as_object_mut().unwrap().remove("measured_version");assert_eq!(call(&router,"POST",&format!("{ZONE}/recover"),Some(&f.token),Some(wrong)).await.0,StatusCode::BAD_REQUEST);
 let mut invalid=body.clone();invalid["heard"][0]["position"]=json!(10.1);assert_eq!(call(&router,"POST",&format!("{ZONE}/recover"),Some(&f.token),Some(invalid)).await.0,StatusCode::BAD_REQUEST);
 let restored=recover(&router,&f.token,body.clone()).await;assert_eq!(restored["accepted"],true);assert_eq!(restored["measured_version"],2);assert_eq!(restored["zone"]["queue_index"],1);assert_eq!(restored["zone"]["position"],0.50025);assert_ne!(restored["zone"]["playback_id"],slot["playback_id"]);
 let duplicate=recover(&router,&f.token,body).await;assert_eq!(duplicate["applied"],false);assert_eq!(duplicate["measured_version"],2);let prepared=prepare_output(&router,&f.token,&restored["zone"]).await;assert_eq!(prepared["plan_id"],restored["plan_id"]);assert_eq!(prepared["listen_session_id"],slot["listen_session_id"]);assert_eq!(totals(&f).await["tracks"],json!([]),"restore must not credit source displacement as heard output time");
 let mut snap=measured_snapshot(&z["playback_id"],&p["listen_session_id"],&z["track"]["id"],0.,10.005,"ended");snap["measured"]["version"]=json!(2);snap["measured"]["segments"][0]["output"]=json!({"start":0,"end":480000,"sample_rate":48000});for day in snap["days"].as_object_mut().unwrap().values_mut(){day["seconds"]=json!(10.);}
 let batch=measured_batch(&f,vec![snap]).await;let(status,uploaded)=call(&router,"POST","/api/v1/stats/offline",Some(&f.token),Some(batch.clone())).await;assert_eq!(status,StatusCode::OK,"{uploaded}");assert_eq!(uploaded["rejected"],json!([]));assert_eq!(totals(&f).await["tracks"][0]["seconds"],10.);assert_eq!(call(&router,"POST","/api/v1/stats/offline",Some(&f.token),Some(batch)).await.1["changed"],false);
}
#[tokio::test]
async fn dual_recovery_v2_capability_loss_and_explicit_stop_do_not_resume_old_cursor() {
 let f=fixture(&["local-shared"]).await;register_v2(&f.app,&f.token).await;let z=zone(&f).await;let p=prepare_output(&f.app,&f.token,&z).await;let mut heard=boundary(&z["playback_id"],&z["track"]["id"],0.250125);heard["frames"]=json!(12000);let mut body=request(&z,&p,vec![heard],false);body["measured_version"]=json!(2);
 register(&f.app,&f.token).await;assert_eq!(f.call("POST",&format!("{ZONE}/recover"),Some(body.clone())).await.0,StatusCode::BAD_REQUEST);register_v2(&f.app,&f.token).await;f.call("POST",&format!("{ZONE}/stop"),Some(json!({}))).await;let result=recover(&f.app,&f.token,body).await;assert_eq!(result["accepted"],false);assert_eq!(result["measured_version"],2);assert_eq!(result["zone"]["status"],"stopped");assert_eq!(totals(&f).await["tracks"],json!([]));
}
