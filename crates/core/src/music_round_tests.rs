use super::*;
use axum::Json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test]
async fn versions_require_unique_recording_identity_and_keep_live_performances_separate() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    for id in ["local-shared", "qq-shared"] {
        let(status,_) = f.call("PUT",&format!("/api/v1/library/tracks/{id}/identity"),Some(json!({"expected_revision":0,"assertions":[{"kind":"recording","namespace":"manual","external_id":"confirmed-recording","title":"已确认录音","role":"recording"}]}))).await;
        assert_eq!(status, StatusCode::OK);
    }
    let path = "/api/v1/library/tracks/local-shared/versions";
    let (status, versions) = f.call("GET", path, None).await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    assert_eq!(versions["total"], 2);
    assert_eq!(versions["automatic_merge"], false);
    assert!(versions["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["track"]["id"] != "qq-live"));
    assert_eq!(
        f.call(
            "PUT",
            path,
            Some(json!({"expected_revision":0,"track_id":"qq-live"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.call(
            "PUT",
            path,
            Some(json!({"expected_revision":0,"track_id":"qq-shared"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call(
            "PUT",
            path,
            Some(json!({"expected_revision":0,"track_id":"local-shared"}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let local =
        f.s.db
            .read(|c| {
                Ok(c.query_row(
                    "SELECT * FROM tracks WHERE id='local-shared'",
                    [],
                    models::Track::from_row,
                )?)
            })
            .await
            .unwrap();
    let ordered = crate::music_versions::order(&f.s, "1".into(), vec![local.clone()])
        .await
        .unwrap();
    // The actual account id, not an invented global account, owns the preference.
    let user =
        f.s.db
            .read(|c| {
                Ok(
                    c.query_row("SELECT id FROM users WHERE username='admin'", [], |r| {
                        r.get::<_, String>(0)
                    })?,
                )
            })
            .await
            .unwrap();
    assert_eq!(ordered[0].id, "local-shared");
    assert_eq!(
        crate::music_versions::order(&f.s, user, vec![local])
            .await
            .unwrap()[0]
            .id,
        "qq-shared"
    );
    let(status,edition)=f.call("PUT","/api/v1/library/tracks/local-shared/edition",Some(json!({"expected_revision":0,"kind":"remaster","label":"24-bit 发行","notes":"人工标注，未验证母带"}))).await;
    assert_eq!(status, StatusCode::OK, "{edition}");
    assert_eq!(edition["revision"], 1);
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/library/tracks/local-shared/edition",
            Some(json!({"expected_revision":0,"kind":"original","label":"conflict","notes":""}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, list) = f.call("GET", path, None).await;
    assert_eq!(list["edition"]["label"], "24-bit 发行");
    let (_, playlist) = f
        .call(
            "POST",
            "/api/v1/playlists",
            Some(json!({"name":"固定歌单","track_ids":["local-shared"]})),
        )
        .await;
    assert_eq!(
        f.call(
            "GET",
            &format!("/api/v1/playlists/{}", playlist["id"].as_str().unwrap()),
            None
        )
        .await
        .1["track_ids"],
        json!(["local-shared"])
    );
}

#[tokio::test]
async fn conflicting_manual_recordings_are_not_merged_by_name_or_work() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let (_, unproven) = f
        .call("GET", "/api/v1/library/tracks/local-shared/versions", None)
        .await;
    assert_eq!(unproven["identity_resolved"], false);
    assert_eq!(unproven["total"], 1);
    let assertion = |kind: &str, id: &str| json!({"kind":kind,"namespace":"manual","external_id":id,"title":"同名作品","role":kind});
    let(_,saved)=f.call("PUT","/api/v1/library/tracks/local-shared/identity",Some(json!({"expected_revision":0,"assertions":[assertion("recording","first"),assertion("recording","second"),assertion("work","work")]}))).await;
    assert_eq!(saved["revision"], 1);
    f.call(
        "PUT",
        "/api/v1/library/tracks/qq-live/identity",
        Some(json!({"expected_revision":0,"assertions":[assertion("work","work")]})),
    )
    .await;
    let (_, v) = f
        .call("GET", "/api/v1/library/tracks/local-shared/versions", None)
        .await;
    assert_eq!(v["identity_resolved"], false);
    assert_eq!(v["total"], 1);
    assert_eq!(v["other_recordings"][0]["id"], "qq-live");
    assert_eq!(
        f.call(
            "PUT",
            "/api/v1/library/tracks/local-shared/versions",
            Some(json!({"expected_revision":0,"track_id":"qq-shared"}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn smart_rules_preview_resolve_export_and_isolate_personal_favorites() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    f.s.db.run(|c|{c.execute("INSERT INTO track_stats(track_id,play_count,last_played_at) VALUES('local-shared',8,unixepoch()-86400*40)",[])?;Ok(())}).await.unwrap();
    f.call(
        "PUT",
        "/api/v1/favorites/local-shared",
        Some(json!({"liked":true})),
    )
    .await;
    let rule = json!({"favorites_only":true,"min_plays":5,"not_played_days":30,"min_sample_rate":88200,"min_bit_depth":24,"sort":"most_played","max_results":10});
    let (status, preview) = f
        .call(
            "POST",
            "/api/v1/smart-playlists/preview",
            Some(json!({"rule":rule})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["total"], 1);
    assert_eq!(preview["items"][0]["id"], "local-shared");
    let (_, created) = f
        .call(
            "POST",
            "/api/v1/playlists",
            Some(json!({"name":"动态收藏","smart_query":rule})),
        )
        .await;
    let path = format!("/api/v1/playlists/{}", created["id"].as_str().unwrap());
    assert_eq!(
        f.call("GET", &path, None).await.1["track_ids"],
        json!(["local-shared"])
    );
    assert_eq!(
        f.call("GET", &format!("{path}/export?format=json"), None)
            .await
            .1["tracks"][0]["id"],
        "local-shared"
    );
    f.call(
        "POST",
        "/api/v1/auth/users",
        Some(json!({"username":"other","password":"other-test-password"})),
    )
    .await;
    let (_, other) = call(
        &f.app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"username":"other","password":"other-test-password"})),
    )
    .await;
    let response = call(
        &f.app,
        "POST",
        "/api/v1/smart-playlists/preview",
        other["token"].as_str(),
        Some(json!({"rule":{"favorites_only":true}})),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    assert_eq!(response.1["total"], 0);
    let (_, excluded) = f
        .call(
            "POST",
            "/api/v1/smart-playlists/preview",
            Some(json!({"rule":{"favorites_only":true,"exclude_saved":true}})),
        )
        .await;
    assert_eq!(excluded["total"], 0);
    for rule in [
        json!({"min_plays":5,"max_plays":1}),
        json!({"sort":"title; DROP TABLE tracks"}),
        json!({"max_results":0}),
        json!({"min_bit_depth":65}),
        json!({"unknown":true}),
    ] {
        let expected = if rule.get("unknown").is_some() {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::BAD_REQUEST
        };
        assert_eq!(
            f.call(
                "POST",
                "/api/v1/smart-playlists/preview",
                Some(json!({"rule":rule}))
            )
            .await
            .0,
            expected
        );
    }
}

async fn llm_server() -> (
    String,
    Arc<AtomicUsize>,
    Arc<tokio::sync::Mutex<Vec<Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let mode = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(tokio::sync::Mutex::new(vec![]));
    let app=Router::new().route("/v1/models",axum::routing::get(||async{Json(json!({"data":[{"id":"qa-model"},{"id":"qa-model"}]}))}))
        .route("/v1/chat/completions",axum::routing::post({let mode=mode.clone();let requests=requests.clone();move|Json(body):Json<Value>|{let mode=mode.clone();let requests=requests.clone();async move{requests.lock().await.push(body);let mode=mode.load(Ordering::SeqCst);if mode==1{return (StatusCode::TOO_MANY_REQUESTS,Json(json!({"error":"SECRET-SHOULD-NOT-LEAK"})));}if mode==2{return (StatusCode::OK,Json(json!({"content":"x".repeat(130*1024)})));}if mode==3{tokio::time::sleep(std::time::Duration::from_secs(10)).await;}let content=json!({"items":[{"id":"qq-only","reason":"平静的配器"},{"id":"invented-from-model"},{"id":"local-shared","reason":"同一录音的已选版本"},{"id":"qq-only"}]}).to_string();(StatusCode::OK,Json(json!({"choices":[{"message":{"content":content}}]})))}}}));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), mode, requests, task)
}

#[tokio::test]
async fn llm_configuration_is_optional_redacted_revisioned_and_models_are_real() {
    let f = Fixture::new().await;
    let (base, _mode, _requests, server) = llm_server().await;
    let (_, initial) = f.call("GET", "/api/v1/llm/settings", None).await;
    assert_eq!(initial["enabled"], false);
    assert!(initial.get("key").is_none());
    let body = json!({"expected_revision":"0","enabled":true,"base_url":base,"model":"qa-model","key":"dummy-secret-key"});
    let (status, saved) = f
        .call("PUT", "/api/v1/llm/settings", Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["key_configured"], true);
    assert!(!saved.to_string().contains("dummy-secret-key"));
    assert_eq!(
        f.call("PUT", "/api/v1/llm/settings", Some(body)).await.0,
        StatusCode::CONFLICT
    );
    let (status, models) = f.call("POST", "/api/v1/llm/models", Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["items"], json!(["qa-model"]));
    let restored = AppState::new(
        db::Db::open(&f.s.config.data.join("db.sqlite")).unwrap(),
        f.s.config.clone(),
        "unused".into(),
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &app(restored),
            "GET",
            "/api/v1/llm/settings",
            Some(&f.token),
            None
        )
        .await
        .1["key_configured"],
        true
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(f.s.config.data.join("providers/.core-llm.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let (_, token) = f
        .call(
            "POST",
            "/api/v1/auth/tokens",
            Some(json!({"name":"read","scopes":["read"]})),
        )
        .await;
    assert_eq!(
        call(
            &f.app,
            "GET",
            "/api/v1/llm/settings",
            token["token"].as_str(),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let(status,moved)=f.call("PUT","/api/v1/llm/settings",Some(json!({"expected_revision":saved["revision"],"enabled":false,"base_url":"https://different.example/v1","model":"qa-model"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(moved["key_configured"], false);
    server.abort();
}

#[tokio::test]
async fn llm_only_ranks_allowlisted_tracks_and_releases_cancelled_requests() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let (base, mode, requests, server) = llm_server().await;
    let input = json!({"track_ids":["local-shared","qq-only","missing"],"preference":"安静器乐"});
    assert_eq!(
        f.call("POST", "/api/v1/llm/recommend", Some(input.clone()))
            .await
            .0,
        StatusCode::CONFLICT
    );
    f.call("PUT","/api/v1/llm/settings",Some(json!({"expected_revision":"0","enabled":true,"base_url":base,"model":"qa-model","key":"dummy"}))).await;
    let (status, result) = f
        .call("POST", "/api/v1/llm/recommend", Some(input.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["items"].as_array().unwrap().len(), 2);
    assert_eq!(result["ignored"], 2);
    assert_eq!(result["host_actions_applied"], false);
    let request = requests.lock().await[0].clone();
    let sent: Value =
        serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(sent["candidates"].as_array().unwrap().len(), 2);
    assert!(sent["candidates"][0].get("path").is_none());
    assert!(sent["candidates"][0].get("provider_data").is_none());
    mode.store(1, Ordering::SeqCst);
    let (status, error) = f
        .call("POST", "/api/v1/llm/recommend", Some(input.clone()))
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(!error.to_string().contains("SECRET"));
    mode.store(2, Ordering::SeqCst);
    assert_eq!(
        f.call("POST", "/api/v1/llm/recommend", Some(input.clone()))
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    mode.store(3, Ordering::SeqCst);
    let app = f.app.clone();
    let token = f.token.clone();
    let pending = tokio::spawn(async move {
        call(
            &app,
            "POST",
            "/api/v1/llm/recommend",
            Some(&token),
            Some(input),
        )
        .await
    });
    for _ in 0..100 {
        if f.s.llm.gate.available_permits() == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(f.s.llm.gate.available_permits(), 1);
    pending.abort();
    let _ = pending.await;
    for _ in 0..100 {
        if f.s.llm.gate.available_permits() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(f.s.llm.gate.available_permits(), 2);
    assert!(f
        .s
        .zones
        .read()
        .await
        .values()
        .all(|z| z.status == "stopped" && z.queue.is_empty()));
    server.abort();
}

#[tokio::test]
async fn plugin_development_templates_install_probe_and_enforce_admin_access() {
    use std::io::Read;
    let f = Fixture::new().await;
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/plugins/development/template?kind=extension&id=qa-sdk")
                .header("Authorization", format!("Bearer {}", f.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
    assert_eq!(zip.len(), 5);
    let mut sdk = String::new();
    zip.by_name("rmusic_plugin.py")
        .unwrap()
        .read_to_string(&mut sdk)
        .unwrap();
    assert!(sdk.contains("MAX_REPLY"));
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/plugins/install")
                .header("Authorization", format!("Bearer {}", f.token))
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
    );
    let (status, checked) = f
        .call(
            "POST",
            "/api/v1/plugins/qa-sdk/contract-check",
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{checked}");
    assert_eq!(checked["passed"], true);
    assert_eq!(checked["uses_installed_credentials"], false);
    let (status, output) = f
        .call(
            "POST",
            "/api/v1/extensions/qa-sdk/invoke/summarize",
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{output}");
    assert_eq!(output["output"]["tracks"], 0);
    assert!(!std::fs::read_dir(&f.s.config.plugin_dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().starts_with(".health-")));
    assert_eq!(
        call(&f.app, "GET", "/api/v1/plugins/development", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    f.s.plugin.stop_all().await;
}

#[tokio::test]
async fn llm_candidates_deduplicate_only_confirmed_recordings_and_respect_version_preference() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    for id in ["local-shared", "qq-shared"] {
        f.call("PUT",&format!("/api/v1/library/tracks/{id}/identity"),Some(json!({"expected_revision":0,"assertions":[{"kind":"recording","namespace":"manual","external_id":"same","title":"已确认录音","role":"recording"}]}))).await;
    }
    let user =
        f.s.db
            .read(|c| {
                Ok(c.query_row("SELECT id FROM users LIMIT 1", [], |r| {
                    r.get::<_, String>(0)
                })?)
            })
            .await
            .unwrap();
    let tracks=f.s.db.read(|c|Ok(c.prepare("SELECT * FROM tracks WHERE id IN('local-shared','qq-shared','qq-live') ORDER BY id")?.query_map([],models::Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?)).await.unwrap();
    let chosen = crate::music_versions::order(&f.s, user, tracks)
        .await
        .unwrap();
    assert_eq!(chosen.len(), 2);
    assert!(chosen.iter().any(|t| t.id == "qq-live"));
}
