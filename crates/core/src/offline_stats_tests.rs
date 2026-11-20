use super::*;
#[tokio::test]
async fn offline_stats_auth_identity_scopes_and_restart_deduplication() {
    let f = Fixture::new().await;
    let (_, user) = f.call("GET", "/api/v1/auth/me", None).await;
    let now = (chrono::Utc::now().timestamp() / 86400 * 86400 - 43200) * 1000;
    let day = chrono::DateTime::from_timestamp_millis(now)
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
    let mut body = json!({"core_id":*f.s.core_id,"user_id":user["id"],"client_id":uuid::Uuid::new_v4().to_string(),"sessions":[{
        "id":uuid::Uuid::new_v4().to_string(),"version":1,"track_id":"cached-deleted-track","title":"离线验收曲","artist":"QA","duration":120.,"started_at":now-40000,"updated_at":now,"position":40.,"reason":"ended","days":{(day):{"seconds":40.,"last_at":now}}
    }]});
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/api/v1/stats/offline",
            None,
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, token) = f
        .call(
            "POST",
            "/api/v1/auth/tokens",
            Some(json!({"name":"read-stats","scopes":["read"]})),
        )
        .await;
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/api/v1/stats/offline",
            token["token"].as_str(),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let core = body["core_id"].clone();
    body["core_id"] = json!("wrong-core");
    assert_eq!(
        f.call("POST", "/api/v1/stats/offline", Some(body.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    body["core_id"] = core;
    let owner = body["user_id"].clone();
    body["user_id"] = json!("another-user");
    assert_eq!(
        f.call("POST", "/api/v1/stats/offline", Some(body.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    body["user_id"] = owner;
    assert_eq!(
        f.call("POST", "/api/v1/stats/offline", Some(body.clone()))
            .await
            .0,
        StatusCode::OK
    );
    let restored = app(
        AppState::new(f.s.db.clone(), f.s.config.clone(), String::new())
            .await
            .unwrap(),
    );
    assert_eq!(
        call(
            &restored,
            "POST",
            "/api/v1/stats/offline",
            Some(&f.token),
            Some(body)
        )
        .await
        .1["changed"],
        false
    );
    let stats = f
        .call(
            "GET",
            "/api/v1/library/tracks/cached-deleted-track/stats",
            None,
        )
        .await
        .1;
    assert_eq!(stats["listened_seconds"], 40.);
    assert_eq!(stats["play_count"], 1);
    assert_eq!(stats["skip_count"], 0);
}
