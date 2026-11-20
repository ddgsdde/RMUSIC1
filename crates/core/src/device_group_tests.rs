//! Logical-device lifecycle checks with isolated channels; no sound card or production data.
use super::*;
async fn members(f: &Fixture) -> Vec<models::Zone> {
    let (track, _) = f.add_audio().await;
    let t = library::get_track(&f.s, track).await.unwrap();
    let mut result = Vec::new();
    for i in 0..2 {
        let endpoint = format!("logical-endpoint-{i}");
        let mut z = models::Zone::new(
            format!("logical-zone-{i}"),
            format!("Room {i}"),
            endpoint.clone(),
        );
        z.queue = vec![t.clone(), t.clone()];
        z.track = Some(t.clone());
        z.queue_index = i;
        z.position = (i + 1) as f64;
        z.repeat = "all".into();
        z.shuffle = i == 1;
        z.status = "paused".into();
        z.volume = 0.2 + i as f64 * 0.1;
        f.s.save_zone(z.clone()).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        f.s.bridges.write().await.insert(endpoint.clone(), tx);
        f.s.endpoints.write().await.insert(endpoint.clone(),json!({"id":endpoint,"kind":"alsa","online":true,"capabilities":{"platform":"linux","playback_instance":true,"scheduled_start_v1":true,"scheduled_queue_v1":true,"group_output_v2":true,"group_output_final_v2":true,"measured_listens_v2":true,"host_clock_v1":true,"device_clock_v1":true,"programme_clock_v1":true}}));
        result.push(z);
    }
    result
}
fn create_body() -> Value {
    json!({"request_id":uuid::Uuid::new_v4().to_string(),"name":"全屋","zone_ids":["logical-zone-0","logical-zone-1"],"source_zone_id":"logical-zone-1"})
}
async fn third_member(f:&Fixture,original:&models::Zone)->models::Zone {
    let mut zone=original.clone();zone.id="logical-zone-2".into();zone.endpoint_id="logical-endpoint-2".into();zone.name="第三房间".into();zone.position=9.;zone.volume=0.6;
    f.s.save_zone(zone.clone()).await.unwrap();
    let (send,mut receive)=tokio::sync::mpsc::channel(64);tokio::spawn(async move{while receive.recv().await.is_some(){}});
    f.s.bridges.write().await.insert(zone.endpoint_id.clone(),send);
    let mut endpoint=f.s.endpoints.read().await[&original.endpoint_id].clone();endpoint["id"]=json!(zone.endpoint_id);
    f.s.endpoints.write().await.insert(zone.endpoint_id.clone(),endpoint);zone
}
#[tokio::test]
async fn device_group_members_can_change_in_place_and_all_original_queues_restore() {
    let f=Fixture::new().await;let originals=members(&f).await;let third=third_member(&f,&originals[0]).await;
    let (_,created)=f.call("POST","/api/v1/device-groups",Some(create_body())).await;
    let id=created["id"].as_str().unwrap();let path=format!("/api/v1/device-groups/{id}");
    let before=f.s.zones.read().await[created["zone_id"].as_str().unwrap()].clone();
    let body=json!({"name":"卧室组合","zone_ids":["logical-zone-1","logical-zone-2"],"expected_revision":created["revision"]});
    let (status,edited)=f.call("PUT",&path,Some(body.clone())).await;
    assert_eq!(status,StatusCode::OK,"{edited}");assert_eq!(edited["id"],created["id"]);assert_eq!(edited["zone_id"],created["zone_id"]);
    let current=f.s.zones.read().await[created["zone_id"].as_str().unwrap()].clone();
    assert_eq!(serde_json::to_value(current.queue).unwrap(),serde_json::to_value(before.queue).unwrap());assert_eq!(current.queue_index,before.queue_index);
    assert_eq!(f.call("PUT",&path,Some(body)).await.0,StatusCode::OK,"an identical retry is idempotent");
    assert_eq!(f.call("PUT",&path,Some(json!({"name":"stale","zone_ids":["logical-zone-0","logical-zone-1"],"expected_revision":created["revision"]}))).await.0,StatusCode::CONFLICT);
    assert_eq!(f.call("DELETE",&path,None).await.0,StatusCode::NO_CONTENT);
    for old in originals.into_iter().chain([third]) {
        let restored=f.s.zones.read().await[&old.id].clone();
        assert_eq!(serde_json::to_value(restored.queue).unwrap(),serde_json::to_value(old.queue).unwrap());
        assert_eq!(restored.queue_index,old.queue_index);assert_eq!(restored.position,old.position);assert_eq!(restored.volume,old.volume);
    }
}
#[tokio::test]
async fn device_group_and_single_output_transfer_keep_queue_occurrences_and_target_settings() {
    let f=Fixture::new().await;let originals=members(&f).await;let third=third_member(&f,&originals[0]).await;
    let (_,created)=f.call("POST","/api/v1/device-groups",Some(create_body())).await;
    let group=created["zone_id"].as_str().unwrap();
    let source=f.s.zones.read().await[group].clone();
    let (status,moved)=f.call("POST",&format!("/api/v1/zones/{group}/transfer"),Some(json!({"target_zone_id":third.id,"expected_playback_id":source.playback_id}))).await;
    assert_eq!(status,StatusCode::OK,"{moved}");assert_eq!(moved["target"]["volume"],third.volume);assert_eq!(moved["target"]["queue_index"],source.queue_index);
    assert_eq!(moved["target"]["queue"],serde_json::to_value(&source.queue).unwrap());assert_eq!(moved["source"]["status"],"stopped");
    let (status,returned)=f.call("POST","/api/v1/zones/logical-zone-2/transfer",Some(json!({"target_zone_id":group,"expected_playback_id":moved["target"]["playback_id"]}))).await;
    assert_eq!(status,StatusCode::OK,"{returned}");assert_eq!(returned["target"]["queue"],moved["target"]["queue"]);assert_eq!(returned["target"]["queue_index"],source.queue_index);assert_eq!(returned["target"]["status"],"paused");
}
#[tokio::test]
async fn device_group_is_a_persistent_device_and_dissolve_restores_occurrences_and_modes() {
    let f = Fixture::new().await;
    let original = members(&f).await;
    let body = create_body();
    let (status, group) = f
        .call("POST", "/api/v1/device-groups", Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let id = group["id"].as_str().unwrap();
    let zone = group["zone_id"].as_str().unwrap();
    assert_eq!(
        f.call("POST", "/api/v1/device-groups", Some(body)).await.0,
        StatusCode::OK
    );
    let virtual_zone = f.s.zones.read().await[zone].clone();
    assert_eq!(
        virtual_zone.queue.iter().map(|t| &t.id).collect::<Vec<_>>(),
        original[1].queue.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    assert_eq!(virtual_zone.queue_index, 1);
    assert_eq!(virtual_zone.repeat, "all");
    assert!(virtual_zone.shuffle);
    assert_eq!(
        f.call("POST", "/api/v1/zones/logical-zone-0/play", Some(json!({})))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/zones/logical-zone-0/queue",
            Some(json!({"track_ids":[]}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let listing = f.call("GET", "/api/v1/endpoints", None).await.1;
    assert!(listing["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "group" && e["id"] == group["endpoint_id"]));
    assert_eq!(
        f.call("DELETE", &format!("/api/v1/device-groups/{id}"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    for original in original {
        let actual = f.s.zones.read().await[&original.id].clone();
        assert_eq!(actual.queue_index, original.queue_index);
        assert_eq!(actual.position, original.position);
        assert_eq!(actual.repeat, original.repeat);
        assert_eq!(actual.shuffle, original.shuffle);
        assert_eq!(actual.volume, original.volume);
        assert_eq!(actual.status, "paused");
        assert_eq!(
            serde_json::to_value(actual.queue).unwrap(),
            serde_json::to_value(original.queue).unwrap()
        );
    }
    assert!(!f.s.zones.read().await.contains_key(zone));
    assert_eq!(
        f.call("DELETE", &format!("/api/v1/device-groups/{id}"), None)
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert!(
        f.call("GET", "/api/v1/device-groups", None).await.1["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn device_group_rejects_browser_android_and_overlapping_output_leases() {
    let f = Fixture::new().await;
    members(&f).await;
    for kind in ["browser", "android"] {
        f.s.endpoints
            .write()
            .await
            .get_mut("logical-endpoint-0")
            .unwrap()["kind"] = json!(kind);
        let (status, _) = f
            .call("POST", "/api/v1/device-groups", Some(create_body()))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    f.s.endpoints
        .write()
        .await
        .get_mut("logical-endpoint-0")
        .unwrap()["kind"] = json!("alsa");
    assert_eq!(
        f.call("POST", "/api/v1/device-groups", Some(create_body()))
            .await
            .0,
        StatusCode::CREATED
    );
    assert_eq!(
        f.call("POST", "/api/v1/device-groups", Some(create_body()))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let external = auth::Auth {
        id: "external".into(),
        username: "external".into(),
        role: "external".into(),
        totp_enabled: false,
        interactive: true,
        scopes: vec!["read".into(), "control".into()],
        session_hash: String::new(),
    };
    assert_eq!(
        device_groups::list(axum::extract::State(f.s.clone()), external)
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn device_group_survives_restart_as_paused_without_resuming_any_member() {
    let f = Fixture::new().await;
    members(&f).await;
    let group = f
        .call("POST", "/api/v1/device-groups", Some(create_body()))
        .await
        .1;
    let restored = AppState::new(f.s.db.clone(), f.s.config.clone(), String::new())
        .await
        .unwrap();
    let z = restored.zones.read().await[group["zone_id"].as_str().unwrap()].clone();
    assert_eq!(z.status, "paused");
    assert!(z.playback_id.is_none());
    assert!(restored.bridges.read().await.is_empty());
    assert_eq!(
        restored.endpoints.read().await[group["endpoint_id"].as_str().unwrap()]["kind"],
        "group"
    );
    assert_eq!(
        f.call("GET", "/api/v1/sync/starts?active=true", None)
            .await
            .1["total"],
        0
    );
}
