use super::*;
#[tokio::test]
async fn purging_removed_device_erases_settings_but_preserves_library_and_history() {
    let f=Fixture::new().await;f.collection_tracks().await;
    f.call("POST","/api/v1/devices/register",Some(json!({"id":"purge-test","name":"Disposable","platform":"windows","outputs":[{"id":"purge-out","name":"Output","kind":"wasapi","default":true,"configurable":true}]}))).await;
    let path="/api/v1/devices/purge-test/purge";let body=Some(json!({"confirmation":"purge-test"}));
    assert_eq!(f.call("POST",path,body.clone()).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(f.call("DELETE","/api/v1/devices/purge-test",None).await.0,StatusCode::OK);
    assert_eq!(f.call("POST",path,Some(json!({"confirmation":"wrong"}))).await.0,StatusCode::BAD_REQUEST);
    let router=app(f.s.clone());assert_eq!(call(&router,"POST",path,None,body.clone()).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(f.call("POST",path,body.clone()).await.0,StatusCode::OK);
    assert_eq!(f.call("POST","/api/v1/devices/purge-test/restore",Some(json!({}))).await.0,StatusCode::NOT_FOUND);
    assert!(f.s.db.run(|c|Ok(!c.query_row("SELECT EXISTS(SELECT 1 FROM devices WHERE id='purge-test')",[],|r|r.get::<_,bool>(0))?&&!c.query_row("SELECT EXISTS(SELECT 1 FROM output_settings WHERE id='purge-out')",[],|r|r.get::<_,bool>(0))?&&c.query_row("SELECT EXISTS(SELECT 1 FROM tracks WHERE id='local-shared')",[],|r|r.get::<_,bool>(0))?)).await.unwrap());
}
#[tokio::test]
async fn removed_device_stays_hidden_across_registration_and_restart_then_restores_its_queue() {
    let f=Fixture::new().await;f.collection_tracks().await;
    f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-remove","name":"临时浏览器"}))).await;
    f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":["local-shared"]}))).await;
    assert_eq!(f.call("DELETE","/api/v1/devices/browser-remove",None).await.0,StatusCode::OK);
    let(_,registration)=f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-remove","name":"重新上线"}))).await;
    assert_eq!(registration["removed"],true);assert_eq!(registration["online"],false);assert!(f.s.zones.read().await.is_empty());
    let state=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();assert!(state.zones.read().await.is_empty());let router=app(state.clone());
    let(_,devices)=call(&router,"GET","/api/v1/devices",Some(&f.token),None).await;assert_eq!(devices["items"],json!([]));assert_eq!(devices["removed"].as_array().unwrap().len(),1);
    assert_eq!(call(&router,"POST","/api/v1/devices/browser-remove/restore",Some(&f.token),Some(json!({}))).await.0,StatusCode::OK);
    let restored=state.zones.read().await["zone-local"].clone();assert_eq!(restored.queue[0].id,"local-shared");assert_eq!(restored.status,"stopped");assert!(restored.playback_id.is_none());
    assert_eq!(call(&router,"POST","/api/v1/endpoints/browser",Some(&f.token),Some(json!({"id":"browser-remove","name":"重新上线"}))).await.1["online"],true);
}
#[tokio::test]
async fn removing_legacy_bridge_sends_stop_then_closes_its_control_connection() {
    let f=Fixture::new().await;let mut zone=f.s.zones.read().await["zone-local"].clone();zone.endpoint_id="bridge-retired".into();f.s.save_zone(zone).await.unwrap();
    let(tx,mut rx)=tokio::sync::mpsc::channel(8);f.s.bridges.write().await.insert("bridge-retired".into(),tx);
    f.s.endpoints.write().await.insert("bridge-retired".into(),json!({"id":"bridge-retired","name":"旧网桥","online":true,"kind":"alsa"}));
    assert_eq!(f.call("DELETE","/api/v1/devices/bridge-retired",None).await.0,StatusCode::OK);
    assert_eq!(rx.recv().await.unwrap()["type"],"stop");assert_eq!(rx.recv().await.unwrap()["type"],"remove_endpoint");
    assert!(!f.s.endpoints.read().await.contains_key("bridge-retired"));assert!(crate::devices::is_removed(&f.s,"bridge-retired").await.unwrap());
}
#[tokio::test]
async fn more_than_four_clients_keep_their_own_zones() {
    let f=Fixture::new().await;
    for n in 0..6 {let id=format!("browser-{n}");let(status,_)=f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":id,"name":id}))).await;assert_eq!(status,StatusCode::OK);}
    let(_,zones)=f.call("GET","/api/v1/zones",None).await;
    for n in 0..6 {assert!(zones["items"].as_array().unwrap().iter().any(|z|z["endpoint_id"]==format!("browser-{n}")));}
    let(_,inventory)=f.call("GET","/api/v1/devices",None).await;
    assert_eq!(inventory["items"].as_array().unwrap().len(),6);
    assert!(inventory["items"].as_array().unwrap().iter().all(|d|d["outputs"][0]["config"]["enabled"]==true));
}
#[tokio::test]
async fn output_inventory_alias_and_settings_survive_registration_and_restart() {
    let f=Fixture::new().await;
    let inventory=json!({"id":"browser-desktop","name":"Computer","platform":"windows","outputs":[{"id":"browser-speaker","name":"Speakers","kind":"wasapi","native_device_id":"physical-speaker","default":false,"configurable":true}]});
    assert_eq!(f.call("POST","/api/v1/devices/register",Some(inventory.clone())).await.0,StatusCode::OK);
    assert_eq!(f.call("PATCH","/api/v1/devices/browser-desktop",Some(json!({"name":"书房电脑"}))).await.0,StatusCode::OK);
    let mut config=json!({"enabled":true,"device_id":"physical-speaker","exclusive":true,"buffer_ms":250,"conversion_location":"device","dsd_output":"disabled","android_mode":null});
    assert_eq!(f.call("PUT","/api/v1/outputs/browser-speaker/config",Some(config.clone())).await.0,StatusCode::OK);
    let(_,registered)=f.call("POST","/api/v1/devices/register",Some(inventory)).await;
    assert_eq!(registered["name"],"书房电脑");assert_eq!(registered["outputs"][0]["config"],config);
    config["buffer_ms"]=json!(0);assert_eq!(f.call("PUT","/api/v1/outputs/browser-speaker/config",Some(config.clone())).await.0,StatusCode::BAD_REQUEST);
    config["buffer_ms"]=json!(250);config["dsd_output"]=json!("native");assert_eq!(f.call("PUT","/api/v1/outputs/browser-speaker/config",Some(config)).await.0,StatusCode::BAD_REQUEST);
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();let app=app(restored);
    let(status,result)=call(&app,"GET","/api/v1/devices",Some(&f.token),None).await;
    assert_eq!(status,StatusCode::OK);assert_eq!(result["items"][0]["name"],"书房电脑");assert_eq!(result["items"][0]["outputs"][0]["config"]["buffer_ms"],250);assert_eq!(result["items"][0]["outputs"][0]["online"],false);
}

#[tokio::test]
async fn closure_android_output_limits_persist_across_registration_and_restart(){
    let f=Fixture::new().await;let registration=json!({"id":"browser-android-limits","name":"Android QA","platform":"android","playback_instance":true,"continuous_pcm":true,"output_config":{"buffer_ms":100,"android_mode":"system"}});
    assert_eq!(f.call("POST","/api/v1/endpoints/browser",Some(registration.clone())).await.0,StatusCode::OK);
    let config=json!({"enabled":true,"device_id":null,"exclusive":false,"buffer_ms":300,"conversion_location":"device","android_mode":"system","dsd_output":"disabled","max_sample_rate":96000,"max_bit_depth":24,"channels":2,"volume_control":"software","volume_max":0.6});
    let path="/api/v1/outputs/browser-android-limits/config";assert_eq!(f.call("PUT",path,Some(config.clone())).await.0,StatusCode::OK);
    let mut invalid=config.clone();invalid["max_sample_rate"]=json!(12345);assert_eq!(f.call("PUT",path,Some(invalid)).await.0,StatusCode::BAD_REQUEST);
    let(_,endpoint)=f.call("POST","/api/v1/endpoints/browser",Some(registration.clone())).await;assert_eq!(endpoint["output_config"],config);
    let restarted=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();let app=app(restarted);let(_,endpoint)=call(&app,"POST","/api/v1/endpoints/browser",Some(&f.token),Some(registration)).await;assert_eq!(endpoint["output_config"]["max_sample_rate"],96000);
}
