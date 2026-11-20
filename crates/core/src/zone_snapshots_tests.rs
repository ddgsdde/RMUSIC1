use super::*;

#[tokio::test]
async fn queue_snapshots_page_duplicates_preserve_quality_and_persist_revisions() {
    let f = Fixture::new().await;
    let (id, _) = f.add_audio().await;
    let mut track = crate::library::get_track(&f.s, id).await.unwrap();
    track.provider_data = json!({"provider_metadata":{"biography":"x".repeat(8192)},
        "source_audio":{"verified":true,"sample_rate":96000,"bit_depth":24},"quality_hint":{"label":"Hi-Res"}});
    let stored = track.clone();
    f.s.db
        .run(move |c| {
            c.execute(
                "UPDATE tracks SET provider_data=?1 WHERE id=?2",
                rusqlite::params![stored.provider_data.to_string(), stored.id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let mut zone =
        crate::models::Zone::new("queue-snapshot".into(), "Queue".into(), "offline".into());
    zone.queue = vec![track.clone(); 501];
    zone.track = Some(track.clone());
    zone.queue_index = 500;
    f.s.save_zone(zone).await.unwrap();
    let (_, state) = f.call("GET", "/api/v1/zones?view=state", None).await;
    let row = state["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|z| z["id"] == "queue-snapshot")
        .unwrap();
    assert_eq!(state["queue_view"], "versioned");
    assert_eq!(row["queue"], json!([]));
    assert_eq!(row["queue_length"], 501);
    assert_eq!(
        row["track"]["provider_data"]["source_audio"]["sample_rate"],
        96000
    );
    assert!(row["track"]["provider_data"]
        .get("provider_metadata")
        .is_none());
    let revision = row["queue_revision"].as_u64().unwrap();
    assert_eq!(revision, 1);
    let (status, first) = f
        .call(
            "GET",
            &format!("/api/v1/zones/queue-snapshot/queue?revision={revision}&limit=500"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"].as_array().unwrap().len(), 500);
    let (_, last) = f
        .call(
            "GET",
            &format!("/api/v1/zones/queue-snapshot/queue?revision={revision}&offset=500&limit=500"),
            None,
        )
        .await;
    assert_eq!(last["items"].as_array().unwrap().len(), 1);
    assert_eq!(first["items"][0]["id"], last["items"][0]["id"]);
    let mut zone = f.s.zones.read().await["queue-snapshot"].clone();
    zone.volume = 0.4;
    f.s.save_zone(zone.clone()).await.unwrap();
    assert_eq!(
        f.s.zones.read().await["queue-snapshot"].queue_revision,
        revision
    );
    let mut other = track.clone();
    other.id = "another-version".into();
    zone.queue.push(other);
    f.s.save_zone(zone).await.unwrap();
    let mut zone = f.s.zones.read().await["queue-snapshot"].clone();
    assert_eq!(zone.queue_revision, revision + 1);
    zone.queue.swap(0, 501);
    f.s.save_zone(zone).await.unwrap();
    assert_eq!(
        f.s.zones.read().await["queue-snapshot"].queue_revision,
        revision + 2
    );
    assert_eq!(
        f.call(
            "GET",
            &format!("/api/v1/zones/queue-snapshot/queue?revision={revision}"),
            None
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.s.zones.read().await["queue-snapshot"].queue[1].provider_data["provider_metadata"]
            ["biography"]
            .as_str()
            .unwrap()
            .len(),
        8192
    );
    let restored = AppState::new(f.s.db.clone(), f.s.config.clone(), String::new())
        .await
        .unwrap();
    assert_eq!(
        restored.zones.read().await["queue-snapshot"].queue_revision,
        revision + 2
    );
    assert_ne!(restored.queue_epoch, f.s.queue_epoch);
    let restored_app = app(restored);
    let stale = format!("/api/v1/zones/queue-snapshot/queue?revision={}&epoch={}", revision+2, f.s.queue_epoch);
    assert_eq!(call(&restored_app,"GET",&stale,Some(&f.token),None).await.0,StatusCode::CONFLICT);

}

#[tokio::test]
async fn queue_snapshot_routes_keep_external_accounts_scoped() {
    let f = Fixture::new().await;
    f.s.db.run(|c|{c.execute("INSERT INTO settings(key,value) VALUES('external_accounts_development','true') ON CONFLICT(key) DO UPDATE SET value='true'",[])?;Ok(())}).await.unwrap();
    assert_eq!(f.call("POST","/api/v1/auth/external-accounts",Some(json!({"username":"snapshot-visitor","password":"visitor-test-password","current_password":"correct-test-password"}))).await.0,StatusCode::OK);
    let (_, login) = call(
        &f.app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"username":"snapshot-visitor","password":"visitor-test-password"})),
    )
    .await;
    let token = login["token"].as_str().unwrap();
    assert_eq!(
        call(
            &f.app,
            "GET",
            "/api/v1/zones/zone-local/queue",
            Some(token),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, state) = call(&f.app, "GET", "/api/v1/zones?view=state", Some(token), None).await;
    assert_eq!(state["items"], json!([]));
}

#[tokio::test]
async fn versioned_zone_events_are_small_and_existing_event_clients_keep_their_snapshot() {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    async fn text(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if let Message::Text(value) = socket.next().await.unwrap().unwrap() {
                    return value.to_string();
                }
            }
        })
        .await
        .unwrap()
    }
    let f = Fixture::new().await;
    let (id, _) = f.add_audio().await;
    let mut track = crate::library::get_track(&f.s, id).await.unwrap();
    track.provider_data = json!({"provider_metadata":"x".repeat(128*1024)});
    let mut zone = f.s.zones.read().await["zone-local"].clone();
    zone.queue = vec![track; 12];
    f.s.save_zone(zone).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = f.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let mut request = format!("ws://{address}/api/v1/events?zone_view=state")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", f.token).parse().unwrap(),
    );
    let (mut compact, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let initial = text(&mut compact).await;
    assert!(initial.len() < 200);
    assert_eq!(
        serde_json::from_str::<Value>(&initial).unwrap()["type"],
        "zones_changed"
    );
    let mut request = format!("ws://{address}/api/v1/events")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", f.token).parse().unwrap(),
    );
    let (mut existing, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let initial = text(&mut existing).await;
    assert!(initial.len() > 1024 * 1024);
    assert_eq!(
        serde_json::from_str::<Value>(&initial).unwrap()["type"],
        "zones"
    );
    let mut zone = f.s.zones.read().await["zone-local"].clone();
    zone.volume = 0.3;
    f.s.save_zone(zone).await.unwrap();
    assert!(text(&mut compact).await.len() < 200);
    drop(compact);
    drop(existing);
    server.abort();
}
