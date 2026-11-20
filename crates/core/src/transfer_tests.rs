use super::*;
#[tokio::test]
async fn queue_reorder_preserves_duplicate_occurrence_and_playback_and_rejects_stale_edits() {
    let(f,a,_,_)=prepared_transfer().await;
    let mut z=f.s.zones.read().await[&a].clone();
    z.queue=vec![z.queue[1].clone(),z.queue[0].clone(),z.queue[1].clone()];
    z.queue_index=2;z.position=3.;z.shuffle=true;z.status="paused".into();
    f.s.save_zone(z.clone()).await.unwrap();
    let request=json!({"expected_queue":z.queue.iter().map(|t|&t.id).collect::<Vec<_>>(),"expected_playback_id":z.playback_id,"order":[2,0,1]});
    let path=format!("/api/v1/zones/{a}/queue/reorder");
    let(status,result)=f.call("POST",&path,Some(request.clone())).await;
    assert_eq!(status,StatusCode::OK,"{result}");
    assert_eq!(result["queue_index"],0);assert_eq!(result["position"],3.);assert_eq!(result["playback_id"],json!(z.playback_id));assert_eq!(result["status"],"paused");assert_eq!(result["shuffle"],true);
    assert_eq!(f.call("POST",&path,Some(request)).await.0,StatusCode::CONFLICT);
    let mut invalid=json!({"expected_queue":result["queue"].as_array().unwrap().iter().map(|t|t["id"].clone()).collect::<Vec<_>>(),"expected_playback_id":result["playback_id"],"order":[0,0,2]});
    assert_eq!(f.call("POST",&path,Some(invalid.clone())).await.0,StatusCode::BAD_REQUEST);
    invalid["order"]=json!([0,1,100000]);assert_eq!(f.call("POST",&path,Some(invalid)).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(json!(f.s.zones.read().await[&a]),result);
}
#[tokio::test]
async fn queue_play_next_is_immediate_even_with_shuffle_and_never_restarts_the_current_song() {
    let(f,a,_,_)=prepared_transfer().await;
    let mut z=f.s.zones.read().await[&a].clone();z.shuffle=true;z.position=3.;
    f.s.save_zone(z.clone()).await.unwrap();
    let path=format!("/api/v1/zones/{a}/queue");
    for _ in 0..8 {
        let(status,next)=f.call("POST",&path,Some(json!({"track_ids":["local-shared"],"play_next":true}))).await;
        assert_eq!(status,StatusCode::OK,"{next}");
        assert_eq!(next["queue"][z.queue_index+1]["id"],"local-shared");assert_eq!(next["position"],3.);assert_eq!(next["playback_id"],json!(z.playback_id));assert_eq!(next["track"],json!(z.track));assert_eq!(next["shuffle"],true);
    }
    let before=f.s.zones.read().await[&a].clone();
    let(_,appended)=f.call("POST",&path,Some(json!({"track_ids":["local-unknown"]}))).await;
    assert_eq!(appended["queue"].as_array().unwrap()[..before.queue.len()],json!(before.queue).as_array().unwrap()[..]);
    assert_eq!(f.call("POST",&path,Some(json!({"track_ids":[],"play_next":true,"insertion_index":0}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(f.call("PUT",&path,Some(json!({"track_ids":[],"play_next":true}))).await.0,StatusCode::BAD_REQUEST);
}
async fn prepared_transfer()->(Fixture,String,String,Value) {
    let f=Fixture::new().await;f.collection_tracks().await;
    for id in ["browser-from","browser-to"] {assert_eq!(f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":id,"name":id,"playback_instance":true}))).await.0,StatusCode::OK);}
    let zones=f.s.zones.read().await;let a=zones.values().find(|z|z.endpoint_id=="browser-from").unwrap().id.clone();let b=zones.values().find(|z|z.endpoint_id=="browser-to").unwrap().id.clone();drop(zones);
    f.call("PUT",&format!("/api/v1/zones/{a}/queue"),Some(json!({"track_ids":["local-shared","local-unknown"],"queue_index":1}))).await;
    let(_,playing)=f.call("POST",&format!("/api/v1/zones/{a}/play"),Some(json!({"queue_index":1,"position":5.}))).await;
    f.call("PATCH",&format!("/api/v1/zones/{a}"),Some(json!({"repeat":"all"}))).await;
    (f,a,b,playing)
}
#[tokio::test]
async fn transfer_prepares_silently_then_moves_the_exact_queue_and_stops_only_the_source() {
    let(f,a,b,playing)=prepared_transfer().await;
    let router=f.app.clone();let token=f.token.clone();let source=a.clone();let target=b.clone();
    let job=tokio::spawn(async move{call(&router,"POST",&format!("/api/v1/zones/{source}/transfer"),Some(&token),Some(json!({"target_zone_id":target,"expected_playback_id":playing["playback_id"]}))).await});
    let mut prepared=None;
    for _ in 0..100 {let z=f.s.zones.read().await[&b].clone();if z.track.is_some(){prepared=Some(z);break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
    let target=prepared.unwrap();assert_eq!(target.status,"paused");assert_eq!(f.s.zones.read().await[&a].status,"playing");
    assert_eq!(f.call("POST",&format!("/api/v1/zones/{b}/ready"),Some(json!({"track_id":target.track.unwrap().id,"playback_id":target.playback_id}))).await.0,StatusCode::NO_CONTENT);
    let(status,result)=job.await.unwrap();assert_eq!(status,StatusCode::OK,"{result}");
    assert_eq!(result["source"]["status"],"stopped");assert_eq!(result["target"]["status"],"playing");assert_eq!(result["target"]["repeat"],"all");
    assert_eq!(result["target"]["queue"],result["source"]["queue"]);assert_eq!(result["target"]["queue_index"],1);assert_ne!(result["source"]["playback_id"],result["target"]["playback_id"]);
}
#[tokio::test]
async fn transfer_offline_or_stale_target_preserves_source_and_rolls_back_destination() {
    let(f,a,b,playing)=prepared_transfer().await;
    assert_eq!(f.call("POST",&format!("/api/v1/zones/{a}/transfer"),Some(json!({"target_zone_id":b,"expected_playback_id":"stale"}))).await.0,StatusCode::CONFLICT);
    let original=f.s.zones.read().await[&a].clone();let old=f.s.zones.read().await[&b].clone();
    let router=f.app.clone();let token=f.token.clone();let source=a.clone();let target=b.clone();
    let job=tokio::spawn(async move{call(&router,"POST",&format!("/api/v1/zones/{source}/transfer"),Some(&token),Some(json!({"target_zone_id":target,"expected_playback_id":playing["playback_id"]}))).await});
    for _ in 0..100 {if f.s.zones.read().await[&b].track.is_some(){break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
    f.s.endpoints.write().await.get_mut("browser-to").unwrap()["online"]=json!(false);
    assert_eq!(job.await.unwrap().0,StatusCode::BAD_REQUEST);
    assert_eq!(serde_json::to_value(&f.s.zones.read().await[&a]).unwrap(),json!(original));
    assert_eq!(f.s.zones.read().await[&b].queue.len(),old.queue.len());
}
#[tokio::test]
async fn browser_registration_ignores_late_unload_and_cannot_renew_a_closed_lease() {
    let f=Fixture::new().await;
    for registration in ["first","second"]{f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-lease","name":"QA","registration_id":registration}))).await;}
    f.call("POST","/api/v1/endpoints/browser/browser-lease/disconnect?registration_id=first",None).await;
    assert_eq!(f.s.endpoints.read().await["browser-lease"]["online"],true);
    f.call("POST","/api/v1/endpoints/browser/browser-lease/disconnect?registration_id=second",None).await;
    assert_eq!(f.s.endpoints.read().await["browser-lease"]["online"],false);
    f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-lease","name":"QA","registration_id":"second"}))).await;
    assert_eq!(f.s.endpoints.read().await["browser-lease"]["online"],false);
    f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-lease","name":"QA","registration_id":"third"}))).await;
    assert_eq!(f.s.endpoints.read().await["browser-lease"]["online"],true);
}

#[tokio::test]
async fn browser_heartbeat_preserves_readiness_and_notifies_viewers_only_when_inventory_changes() {
    let f=Fixture::new().await;let mut events=f.s.events.subscribe();
    let registration=json!({"id":"browser-heartbeat","name":"QA","registration_id":"first","playback_instance":true});
    f.call("POST","/api/v1/endpoints/browser",Some(registration.clone())).await;
    while events.try_recv().is_ok() {}
    f.s.endpoints.write().await.get_mut("browser-heartbeat").unwrap()["ready_playback_id"]=json!("ready");
    f.call("POST","/api/v1/endpoints/browser",Some(registration.clone())).await;
    assert_eq!(f.s.endpoints.read().await["browser-heartbeat"]["ready_playback_id"],"ready");
    assert!(events.try_recv().is_err());
    f.s.endpoints.write().await.get_mut("browser-heartbeat").unwrap()["online"]=json!(false);
    f.call("POST","/api/v1/endpoints/browser",Some(registration.clone())).await;
    assert_eq!(events.try_recv().unwrap()["type"],"endpoints");
    let mut renewed=registration;renewed["registration_id"]=json!("new");
    f.call("POST","/api/v1/endpoints/browser",Some(renewed)).await;
    assert!(f.s.endpoints.read().await["browser-heartbeat"]["ready_playback_id"].is_null());
}

#[tokio::test]
async fn browser_visualization_requires_a_viewer_lease_and_the_current_output_session() {
    let(f,a,_,playing)=prepared_transfer().await;
    f.s.endpoints.write().await.get_mut("browser-from").unwrap()["capabilities"]["visualization_v1"]=json!(true);
    let report=json!({"version":1,"core_playback_id":playing["playback_id"],"track_id":playing["track"]["id"],"position":5.,"frames":[{"playback_id":playing["playback_id"],"position":5.,"bands":[1,2,3,4,5,6,7,8],"energy":20,"peak":30}]});
    let path=format!("/api/v1/zones/{a}/visualization");
    assert_eq!(f.call("POST",&format!("{path}/report"),Some(report.clone())).await.0,StatusCode::NO_CONTENT);
    assert_eq!(f.call("POST",&path,Some(json!({}))).await.1["frames"],json!([]));
    assert_eq!(f.call("POST",&format!("{path}/report"),Some(report.clone())).await.0,StatusCode::NO_CONTENT);
    assert_eq!(f.call("POST",&path,Some(json!({}))).await.1["frames"].as_array().unwrap().len(),1);
    let(_,login)=call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"admin","password":"correct-test-password"}))).await;
    assert_eq!(call(&f.app,"POST",&format!("{path}/report"),login["token"].as_str(),Some(report.clone())).await.0,StatusCode::FORBIDDEN);
    let mut zone=f.s.zones.read().await[&a].clone();zone.renew_playback();f.s.save_zone(zone).await.unwrap();
    assert_eq!(f.call("POST",&path,Some(json!({}))).await.1["frames"],json!([]));
    f.call("POST",&format!("{path}/report"),Some(report)).await;
    assert_eq!(f.call("POST",&path,Some(json!({}))).await.1["frames"],json!([]));
}

#[tokio::test]
async fn paused_transfer_keeps_order_modes_and_the_destinations_own_volume_and_dsp() {
    let(f,a,b,_)=prepared_transfer().await;
    f.call("POST",&format!("/api/v1/zones/{a}/pause"),Some(json!({}))).await;
    f.call("PATCH",&format!("/api/v1/zones/{a}"),Some(json!({"shuffle":true,"autoplay":{"enabled":false,"source":"local","fallback_local":true}}))).await;
    f.call("PATCH",&format!("/api/v1/zones/{b}"),Some(json!({"volume":0.25,"dsp":{"enabled":true,"preamp_db":-3.}}))).await;
    f.call("PUT",&format!("/api/v1/zones/{b}/queue"),Some(json!({"track_ids":["local-unknown","local-shared"]}))).await;
    let source=f.s.zones.read().await[&a].clone();let old=f.s.zones.read().await[&b].clone();
    let router=f.app.clone();let token=f.token.clone();let target=b.clone();let from=a.clone();
    let job=tokio::spawn(async move{call(&router,"POST",&format!("/api/v1/zones/{from}/transfer"),Some(&token),Some(json!({"target_zone_id":target,"expected_playback_id":source.playback_id}))).await});
    for _ in 0..100 {let z=f.s.zones.read().await[&b].clone();if z.track.is_some(){f.call("POST",&format!("/api/v1/zones/{b}/ready"),Some(json!({"track_id":z.track.unwrap().id,"playback_id":z.playback_id}))).await;break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
    let(status,result)=job.await.unwrap();assert_eq!(status,StatusCode::OK,"{result}");
    let z=f.s.zones.read().await[&b].clone();assert_eq!(z.status,"paused");assert!(z.shuffle);assert_eq!(z.repeat,"all");assert!(!z.autoplay.enabled);
    assert!(z.queue_revision>old.queue_revision);
    assert_eq!(z.volume,old.volume);assert_eq!(json!(z.dsp),json!(old.dsp));assert_eq!(z.queue_index,1);
}
