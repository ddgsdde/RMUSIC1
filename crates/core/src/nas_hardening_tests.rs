use super::*;
#[tokio::test]
async fn playlist_exchange_is_owned_bounded_explicit_and_never_fetches_urls() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let content=json!({"format":"rmusic-playlist","version":1,"tracks":[{"id":"local-shared","source":"local","title":"夜航"},{"title":"夜航 · 验收曲","artist":"林间","album":"夜航","duration":30},{"location":"http://127.0.0.1:1/must-not-fetch.flac","title":"Unknown"}]}).to_string();
    let (status, preview) = f
        .call(
            "POST",
            "/api/v1/playlists/import/preview",
            Some(json!({"format":"json","content":content,"offset":0,"limit":2})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preview["total"], 3);
    assert_eq!(preview["items"].as_array().unwrap().len(), 2);
    assert_eq!(preview["items"][0]["status"], "matched");
    assert_eq!(preview["items"][1]["status"], "review");
    assert!(preview["items"][1]["matched"].is_null());
    assert_eq!(
        preview["items"][1]["candidates"].as_array().unwrap().len(),
        2
    );
    let (_, page) = f
        .call(
            "POST",
            "/api/v1/playlists/import/preview",
            Some(json!({"format":"json","content":content,"offset":2})),
        )
        .await;
    assert_eq!(page["items"][0]["status"], "missing");
    let (status, created) = f
        .call(
            "POST",
            "/api/v1/playlists/import",
            Some(json!({"name":"导入列表","track_ids":["local-shared","qq-only","local-shared"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let id = created["id"].as_str().unwrap();
    let (_, export) = f
        .call(
            "GET",
            &format!("/api/v1/playlists/{id}/export?format=json"),
            None,
        )
        .await;
    assert_eq!(export["tracks"].as_array().unwrap().len(), 3);
    assert_eq!(export["tracks"][0]["id"], "local-shared");
    assert!(export["tracks"][0].get("provider_data").is_none());
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/api/v1/playlists/import",
            None,
            Some(json!({"name":"Not authorized","track_ids":["qq-only"]}))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn empty_or_missing_mount_never_marks_the_existing_library_missing() {
    let f = Fixture::new().await;
    let root = f.s.config.data.join("offline-root");
    std::fs::create_dir(&root).unwrap();
    let path = root.to_string_lossy().into_owned();
    f.s.db.run(move|c|{c.execute("INSERT INTO roots VALUES('offline',?1)",[path])?;c.execute("INSERT INTO tracks(id,root_id,title,artist,album,album_artist,source,format,recording_id,created_at) VALUES('retained','offline','T','A','B','A','local','flac','r',0)",[])?;c.execute("INSERT INTO jobs(id,kind,status,created_at) VALUES('scan-empty','scan','running',0)",[])?;Ok(())}).await.unwrap();
    assert!(library::run_scan_job(&f.s, "scan-empty")
        .await
        .unwrap_err()
        .to_string()
        .contains("目录为空"));
    std::fs::remove_dir(&root).unwrap();
    assert!(library::run_scan_job(&f.s, "scan-empty")
        .await
        .unwrap_err()
        .to_string()
        .contains("离线"));
    assert!(f
        .s
        .db
        .read(|c| Ok(c.query_row(
            "SELECT available FROM tracks WHERE id='retained'",
            [],
            |r| r.get::<_, bool>(0)
        )?))
        .await
        .unwrap());
}
#[tokio::test]
async fn health_and_identity_search_are_authorized_and_honest() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let (status, value) = f.call("GET", "/api/v1/maintenance/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["storage"]["durability"], "FULL");
    assert_eq!(value["database"]["read_connections"], 2);
    let (status, result) = f
        .call(
            "GET",
            "/api/v1/music/entities?q=%E5%A4%9C%E8%88%AA&kind=release",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(result["total"].as_i64().unwrap() > 0);
    assert!(result["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["kind"] == "release"));
    assert_eq!(
        f.call("GET", "/api/v1/stats?timezone=Invalid%2FZone", None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn import_retries_are_idempotent_across_restart_and_reject_changed_intent() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let body = json!({"request_id":uuid::Uuid::new_v4().to_string(),"name":"重试验收","track_ids":["local-shared","qq-only"]});
    let (first, second) = tokio::join!(
        f.call("POST", "/api/v1/playlists/import", Some(body.clone())),
        f.call("POST", "/api/v1/playlists/import", Some(body.clone()))
    );
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(second.0, StatusCode::OK);
    assert_eq!(first.1["id"], second.1["id"]);
    let restarted = AppState::new(
        crate::db::Db::open(&f.s.config.data.join("db.sqlite")).unwrap(),
        f.s.config.clone(),
        "test".into(),
    )
    .await
    .unwrap();
    let (status, replayed) = call(
        &app(restarted),
        "POST",
        "/api/v1/playlists/import",
        Some(&f.token),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replayed["id"], first.1["id"]);
    assert_eq!(replayed["replayed"], true);
    let mut conflict = body.clone();
    conflict["name"] = json!("不同意图");
    assert_eq!(
        f.call("POST", "/api/v1/playlists/import", Some(conflict))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let path = format!("/api/v1/playlists/{}", first.1["id"].as_str().unwrap());
    assert_eq!(
        f.call("DELETE", &path, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        f.call("POST", "/api/v1/playlists/import", Some(body))
            .await
            .0,
        StatusCode::GONE
    );
}

#[tokio::test]
async fn metadata_only_local_track_reports_recovery_action_instead_of_internal_error() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    f.s.db
        .run(|c| {
            c.execute("UPDATE tracks SET path=NULL WHERE id='local-shared'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, result) = f
        .call("GET", "/api/v1/library/tracks/local-shared", None)
        .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(result["error"]["code"], "media_missing");
    assert!(result["error"]["message"]
        .as_str()
        .unwrap()
        .contains("重新定位"));
}
#[tokio::test]
async fn two_mib_playlist_content_fits_the_json_envelope_and_larger_content_is_rejected() {
    let f = Fixture::new().await;
    let tail = "\n/missing.wav\n";
    let content = format!("#{}{}", "x".repeat(2 * 1024 * 1024 - 1 - tail.len()), tail);
    assert_eq!(content.len(), 2 * 1024 * 1024);
    let (status, value) = f
        .call(
            "POST",
            "/api/v1/playlists/import/preview",
            Some(json!({"format":"m3u8","content":content})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["total"], 1);
    let (status, _) = f
        .call(
            "POST",
            "/api/v1/playlists/import/preview",
            Some(json!({"format":"m3u8","content":"#".repeat(2*1024*1024+1)})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn statistics_timezone_is_core_owned_defaults_to_shanghai_and_rejects_invalid_updates() {
    let f=Fixture::new().await;
    assert_eq!(f.call("GET","/api/v1/settings",None).await.1["statistics_timezone"],"Asia/Shanghai");
    let(_,stats)=f.call("GET","/api/v1/stats?period_days=1",None).await;
    assert_eq!(stats["timezone"],"Asia/Shanghai");
    assert_eq!(stats["end_day"],chrono::Utc::now().with_timezone(&chrono_tz::Asia::Shanghai).format("%Y-%m-%d").to_string());
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"statistics_timezone":"America/New_York"}))).await.0,StatusCode::OK);
    assert_eq!(f.call("GET","/api/v1/stats?period_days=1",None).await.1["timezone"],"America/New_York");
    assert_eq!(f.call("GET","/api/v1/stats?period_days=1&timezone=UTC",None).await.1["timezone"],"UTC");
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"statistics_timezone":"Invalid/Timezone","name":"should-not-save"}))).await.0,StatusCode::BAD_REQUEST);
    let(_,settings)=f.call("GET","/api/v1/settings",None).await;
    assert_eq!(settings["statistics_timezone"],"America/New_York");assert_ne!(settings["name"],"should-not-save");
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();
    assert_eq!(restored.setting("statistics_timezone",json!("missing")).await,"America/New_York");
}
