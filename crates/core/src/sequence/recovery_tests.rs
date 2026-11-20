use super::*;
#[path="recovery_v2_tests.rs"]
mod output_timing;

async fn register(router: &Router, token: &str) {
    let(status,value)=call(router,"POST","/api/v1/endpoints/browser",Some(token),Some(json!({
        "id":"browser-continuous","name":"durable","playback_instance":true,"continuous_pcm":true,"continuous_recovery":true,"recovery_controls":true
    }))).await;
    assert_eq!(status, StatusCode::OK, "{value}");
}
async fn durable(queue: &[&str]) -> Fixture {
    let f = fixture(queue).await;
    register(&f.app, &f.token).await;
    f
}
async fn restart(f: &Fixture) -> (AppState, Router) {
    let db = crate::db::Db::open(&f.s.config.data.join("db.sqlite")).unwrap();
    let state = AppState::new(db, f.s.config.clone(), String::new())
        .await
        .unwrap();
    crate::stats::recover(&state).await.unwrap();
    let router = app(state.clone());
    (state, router)
}
fn request(z: &Value, p: &Value, heard: Vec<Value>, terminal: bool) -> Value {
    json!({"endpoint_id":z["endpoint_id"],"request_id":uuid::Uuid::new_v4().to_string(),
        "plan_id":p["plan_id"],"sample_rate":48000,"heard":heard,"terminal":terminal})
}
async fn recover(router: &Router, token: &str, body: Value) -> Value {
    let (status, value) = call(
        router,
        "POST",
        &format!("{ZONE}/recover"),
        Some(token),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}
async fn current(router: &Router, token: &str) -> Value {
    call(router, "GET", ZONE, Some(token), None).await.1
}

#[tokio::test]
async fn device_interruption_waits_for_explicit_play_across_restart_without_consuming_its_cursor() {
    let f=durable(&["local-shared","local-unknown"]).await;
    let z=zone(&f).await;let p=prepare(&f).await;
    let report=json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"recoverable":true,"device_failed":true});
    let mut stale=report.clone();stale["playback_id"]=json!(uuid::Uuid::new_v4().to_string());
    assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(stale)).await.0,StatusCode::NO_CONTENT);
    assert_eq!(current(&f.app,&f.token).await["playback_id"],z["playback_id"]);
    assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(report)).await.0,StatusCode::NO_CONTENT);
    let body=request(&z,&p,vec![boundary(&z["playback_id"],&z["track"]["id"],0.25)],false);
    let waiting=recover(&f.app,&f.token,body.clone()).await;
    assert_eq!(waiting["waiting_for_play"],true);assert_eq!(waiting["accepted"],false);
    assert_eq!(waiting["zone"]["status"],"paused");assert!(waiting["zone"]["playback_id"].is_null());
    let (_,router)=restart(&f).await;
    register(&router,&f.token).await;
    call(&router,"PATCH",ZONE,Some(&f.token),Some(json!({"volume":0.17,"muted":true}))).await;
    assert_eq!(recover(&router,&f.token,body.clone()).await["waiting_for_play"],true);
    call(&router,"POST",&format!("{ZONE}/play"),Some(&f.token),Some(json!({}))).await;
    let restored=recover(&router,&f.token,body.clone()).await;
    assert_eq!(restored["accepted"],true);assert_eq!(restored["zone"]["status"],"playing");
    assert_eq!(restored["zone"]["position"],0.25);assert_eq!(restored["zone"]["volume"],0.17);assert_eq!(restored["zone"]["muted"],true);
    assert_eq!(recover(&router,&f.token,body).await["applied"],false);
}

#[tokio::test]
async fn device_interruption_must_be_recoverable_and_stop_revokes_waiting_play() {
    let f=durable(&["local-shared"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    let mut report=json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"device_failed":true});
    assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(report.clone())).await.0,StatusCode::BAD_REQUEST);
    report["recoverable"]=json!(true);
    f.call("POST",&format!("{ZONE}/progress"),Some(report)).await;
    f.call("POST",&format!("{ZONE}/stop"),Some(json!({}))).await;
    let result=recover(&f.app,&f.token,request(&z,&p,vec![boundary(&z["playback_id"],&z["track"]["id"],0.25)],false)).await;
    assert_eq!(result["accepted"],false);assert_ne!(result["waiting_for_play"],true);assert_eq!(result["zone"]["status"],"stopped");
}

#[tokio::test]
async fn transient_bridge_fault_resumes_without_play_and_respects_pause() {
    use futures_util::{SinkExt,StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest,Message};
    for pause in [false,true] {
        let f=fixture(&["local-shared"]).await;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr=listener.local_addr().unwrap();
        let router=f.app.clone();
        let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        let mut handshake=format!("ws://{addr}/api/v1/bridge/events").into_client_request().unwrap();
        handshake.headers_mut().insert("Authorization",format!("Bearer {}",f.token).parse().unwrap());
        let (mut socket,_)=tokio_tungstenite::connect_async(handshake).await.unwrap();
        socket.send(Message::Text(json!({"type":"register","endpoint":{"id":"resume-test","kind":"alsa","capabilities":{"continuous_pcm":true,"continuous_recovery":true,"recovery_controls":true,"playback_instance":true,"media_url_api_relative":true}}}).to_string().into())).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2),async {
            while let Some(Ok(message))=socket.next().await {
                if message.to_text().is_ok_and(|text|text.contains("\"registered\"")) {return;}
            }
            panic!("bridge closed before registration");
        }).await.unwrap();
        assert_eq!(f.call("PATCH",ZONE,Some(json!({"endpoint_id":"resume-test"}))).await.0,StatusCode::OK);
        f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;
        let z=zone(&f).await;
        let plan=prepare(&f).await;
        socket.send(Message::Text(json!({"type":"error","endpoint_id":"resume-test","zone_id":z["id"],"track_id":z["track"]["id"],"core_playback_id":z["playback_id"],"recoverable":true,"source_failed":true,"retryable":true,"automatic_recovery":true}).to_string().into())).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2),async {
            while !current(&f.app,&f.token).await["playback_id"].is_null() {tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
        }).await.unwrap();
        if pause {assert_eq!(f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await.0,StatusCode::OK);}
        let result=recover(&f.app,&f.token,request(&z,&plan,vec![boundary(&z["playback_id"],&z["track"]["id"],0.25)],false)).await;
        assert_eq!(result["accepted"],true,"{result}");
        assert_eq!(result["zone"]["status"],if pause{"paused"}else{"playing"});
        assert_eq!(result["zone"]["position"],0.25);
        server.abort();
    }
}

#[tokio::test]
async fn rejected_handoff_recovers_the_heard_successor_without_replaying_or_double_credit() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let plan = prepare(&f).await;
    let body = handoff(&z, &plan, 1, 30., 0., 0.25);
    let before = totals(&f).await;
    let (status, rejection) = f.call("POST", &format!("{ZONE}/handoff"), Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(rejection["error"]["code"], "handoff_timing_conflict");
    assert_eq!(zone(&f).await, z);
    assert_eq!(totals(&f).await, before);

    // The bridge freezes its real cutover cursor and asks Core to reconcile the same plan.
    elapsed(&f, 35).await;
    let report = json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"recoverable":true});
    assert_eq!(f.call("POST", &format!("{ZONE}/progress"), Some(report)).await.0, StatusCode::NO_CONTENT);
    let next = &plan["items"][0];
    let body = request(&z,&plan,vec![
        boundary(&z["playback_id"],&z["track"]["id"],30.),
        boundary(&next["playback_id"],&next["track"]["id"],0.25),
    ],false);
    let resolved = recover(&f.app, &f.token, body.clone()).await;
    assert_eq!(resolved["accepted"], true, "{resolved}");
    assert_eq!(resolved["zone"]["track"]["id"], next["track"]["id"]);
    assert_eq!(resolved["zone"]["position"], 0.25);
    assert_eq!(resolved["zone"]["status"], "playing");
    let after = totals(&f).await;
    assert_eq!(recover(&f.app, &f.token, body).await["applied"], false);
    assert_eq!(totals(&f).await, after);
}

#[tokio::test]
async fn restart_recovers_uncommitted_boundary_with_new_identity_and_latest_controls() {
    let f = durable(&["local-shared", "local-unknown", "local-shared"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    f.call("PATCH", ZONE, Some(json!({"volume":0.17,"muted":true})))
        .await;
    let old = f.s.zones.read().await["zone-local"].clone();
    let legacy = crate::sequence::signature_with_loudness(&old,true);
    f.s.db.run(move |c| {
        c.execute("UPDATE sequence_recovery SET data=json_set(data,'$.anchor',?2,'$.plan.signature',?2) WHERE zone_id=?1",
            rusqlite::params![old.id,legacy])?;
        Ok(())
    }).await.unwrap();
    let (_, router) = restart(&f).await;
    assert_eq!(current(&router, &f.token).await["status"], "stopped");
    // A pause while the adapter is offline must survive its later reconnection.
    assert_eq!(
        call(
            &router,
            "POST",
            &format!("{ZONE}/pause"),
            Some(&f.token),
            Some(json!({}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, login) = call(
        &router,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"username":"admin","password":"correct-test-password"})),
    )
    .await;
    let token = login["token"].as_str().unwrap();
    register(&router, token).await;
    let slot = &p["items"][0];
    let body = request(
        &z,
        &p,
        vec![
            boundary(&z["playback_id"], &z["track"]["id"], 2.),
            boundary(&slot["playback_id"], &slot["track"]["id"], 1.25),
        ],
        false,
    );
    let result = recover(&router, token, body.clone()).await;
    let restored = &result["zone"];
    assert_eq!(result["accepted"], true);
    assert_eq!(result["applied"], true);
    assert_eq!(restored["queue_index"], 1);
    assert_eq!(restored["position"], 1.25);
    assert_eq!(restored["status"], "paused");
    assert_eq!(restored["volume"], 0.17);
    assert_eq!(restored["muted"], true);
    assert_ne!(restored["playback_id"], slot["playback_id"]);
    assert_ne!(result["plan_id"], p["plan_id"]);
    let duplicate = recover(&router, token, body).await;
    assert_eq!(duplicate["applied"], false);
    assert_eq!(duplicate["zone"], *restored);
    let(status,prepared)=call(&router,"POST",&format!("{ZONE}/prepare"),Some(token),Some(json!({"endpoint_id":restored["endpoint_id"],"track_id":restored["track"]["id"],"playback_id":restored["playback_id"]}))).await;
    assert_eq!(status, StatusCode::OK, "{prepared}");
    assert_eq!(prepared["plan_id"], result["plan_id"]);
    assert_eq!(prepared["listen_session_id"], slot["listen_session_id"]);
    assert_eq!(
        totals(&f).await["tracks"],
        json!([]),
        "restoring a cursor must not add wall-clock listening"
    );
}

#[tokio::test]
async fn paused_restart_accepts_already_confirmed_position_after_device_counter_reset() {
    let f=durable(&["local-shared","local-unknown"]).await;
    let z=zone(&f).await;let p=prepare(&f).await;
    device_progress(&f,&z,1.25,1.25).await;
    f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await;
    let (_,router)=restart(&f).await;register(&router,&f.token).await;
    // Reopening a paused HAL output starts its device counter at zero. Its
    // source offset is already backed by the Core's durable listening ledger.
    let source=json!({"playback_id":z["playback_id"],"track_id":z["track"]["id"],"position":1.25,"frames":0});
    let mut unconfirmed=source.clone();unconfirmed["position"]=json!(1.5);
    let (status,_)=call(&router,"POST",&format!("{ZONE}/recover"),Some(&f.token),Some(request(&z,&p,vec![unconfirmed],false))).await;
    assert_eq!(status,StatusCode::BAD_REQUEST,"zero device frames must not advance past confirmed output");
    let before=totals(&f).await;
    let restored=recover(&router,&f.token,request(&z,&p,vec![source],false)).await;
    assert_eq!(restored["accepted"],true);assert_eq!(restored["zone"]["status"],"paused");
    assert_eq!(restored["zone"]["position"],1.25);
    assert_eq!(totals(&f).await,before,"restoring a confirmed position must not credit listening twice");
}

#[tokio::test]
async fn restart_uses_durable_actual_position_and_preserves_counted_group_without_old_reports() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    device_progress(&f, &z, 20., 20.).await;
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let result = recover(
        &router,
        &f.token,
        request(
            &z,
            &p,
            vec![boundary(&z["playback_id"], &z["track"]["id"], 1.)],
            false,
        ),
    )
    .await;
    let next = &result["zone"];
    assert_eq!(next["status"], "playing");
    assert_eq!(next["position"], 20.);
    assert_ne!(next["playback_id"], z["playback_id"]);
    for action in ["progress", "ended"] {
        call(&router,"POST",&format!("{ZONE}/{action}"),Some(&f.token),Some(json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"position":30.,"frames":1440000,"sample_rate":48000}))).await;
        assert_eq!(current(&router, &f.token).await, *next);
    }
    call(&router,"POST",&format!("{ZONE}/progress"),Some(&f.token),Some(json!({"track_id":next["track"]["id"],"playback_id":next["playback_id"],"position":20.5,"frames":24000,"sample_rate":48000}))).await;
    assert_eq!(
        totals(&f).await["tracks"][0],
        json!({"id":"local-shared","seconds":20.5,"plays":1,"skips":0})
    );
    let batch = measured_batch(
        &f,
        vec![measured_snapshot(
            &z["playback_id"],
            &p["listen_session_id"],
            &z["track"]["id"],
            0.,
            20.,
            "interrupted",
        )],
    )
    .await;
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/stats/offline",
            Some(&f.token),
            Some(batch)
        )
        .await
        .1["rejected"],
        json!([])
    );
    assert_eq!(totals(&f).await["tracks"][0]["seconds"], 20.5);
}

#[tokio::test]
async fn committed_handoff_outruns_an_older_terminal_cursor_without_rollback() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    elapsed(&f, 40).await;
    let (_, committed) = f
        .call(
            "POST",
            &format!("{ZONE}/handoff"),
            Some(handoff(&z, &p, 1, 2., 0., 0.25)),
        )
        .await;
    assert_eq!(committed["applied"], true);
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let result = recover(
        &router,
        &f.token,
        request(
            &z,
            &p,
            vec![boundary(&z["playback_id"], &z["track"]["id"], 2.)],
            true,
        ),
    )
    .await;
    assert_eq!(result["accepted"], true);
    assert_eq!(result["zone"]["queue_index"], 1);
    assert_eq!(result["zone"]["position"], 0.25);
    assert_eq!(result["zone"]["status"], "playing");
    assert_eq!(
        totals(&f).await["tracks"],
        json!([{"id":"local-shared","seconds":2.,"plays":0,"skips":0},{"id":"local-unknown","seconds":0.25,"plays":0,"skips":0}])
    );
}

#[tokio::test]
async fn fully_heard_queue_finishes_without_replaying_and_history_remains_idempotent() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let slot = &p["items"][0];
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let body = request(
        &z,
        &p,
        vec![
            boundary(&z["playback_id"], &z["track"]["id"], 2.),
            boundary(&slot["playback_id"], &slot["track"]["id"], 3.),
        ],
        true,
    );
    let result = recover(&router, &f.token, body.clone()).await;
    assert_eq!(result["accepted"], true);
    assert_eq!(result["finished"], true);
    assert!(result["zone"]["playback_id"].is_null());
    assert_eq!(result["zone"]["status"], "stopped");
    assert_eq!(result["zone"]["queue_index"], 1);
    assert_eq!(recover(&router, &f.token, body).await["applied"], false);
    let batch = measured_batch(
        &f,
        vec![
            measured_snapshot(
                &z["playback_id"],
                &p["listen_session_id"],
                &z["track"]["id"],
                0.,
                2.,
                "ended",
            ),
            measured_snapshot(
                &slot["playback_id"],
                &slot["listen_session_id"],
                &slot["track"]["id"],
                0.,
                3.,
                "ended",
            ),
        ],
    )
    .await;
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/stats/offline",
            Some(&f.token),
            Some(batch.clone())
        )
        .await
        .1["rejected"],
        json!([])
    );
    let before = totals(&f).await;
    assert_eq!(before["tracks"][0]["seconds"], 2.);
    assert_eq!(before["tracks"][1]["seconds"], 3.);
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/v1/stats/offline",
            Some(&f.token),
            Some(batch)
        )
        .await
        .1["changed"],
        false
    );
    assert_eq!(totals(&f).await, before);
}

#[tokio::test]
async fn durable_source_eof_overrides_a_lagging_nonterminal_cursor() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let slot = &p["items"][0];
    let batch = measured_batch(
        &f,
        vec![
            measured_snapshot(
                &z["playback_id"],
                &p["listen_session_id"],
                &z["track"]["id"],
                0.,
                2.,
                "ended",
            ),
            measured_snapshot(
                &slot["playback_id"],
                &slot["listen_session_id"],
                &slot["track"]["id"],
                0.,
                3.,
                "ended",
            ),
        ],
    )
    .await;
    assert_eq!(
        f.call("POST", "/api/v1/stats/offline", Some(batch)).await.1["rejected"],
        json!([])
    );
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let result = recover(
        &router,
        &f.token,
        request(
            &z,
            &p,
            vec![
                boundary(&z["playback_id"], &z["track"]["id"], 2.),
                boundary(&slot["playback_id"], &slot["track"]["id"], 1.),
            ],
            false,
        ),
    )
    .await;
    assert_eq!(result["finished"], true);
    assert_eq!(result["zone"]["status"], "stopped");
}

#[tokio::test]
async fn newer_navigation_including_stopped_seek_and_identical_queue_revokes_recovery() {
    for action in ["stop", "seek", "queue", "next", "repeat", "dsp"] {
        let f = durable(&["local-shared", "local-unknown"]).await;
        let z = zone(&f).await;
        let p = prepare(&f).await;
        let (_, router) = restart(&f).await;
        register(&router, &f.token).await;
        let (method, path, body) = match action {
            "queue" => (
                "PUT",
                format!("{ZONE}/queue"),
                json!({"track_ids":["local-shared","local-unknown"]}),
            ),
            "repeat" => ("PATCH", ZONE.into(), json!({"repeat":"one"})),
            "dsp" => (
                "PUT",
                format!("{ZONE}/dsp"),
                json!({"enabled":true,"preamp_db":-2.}),
            ),
            _ => (
                "POST",
                format!("{ZONE}/{action}"),
                if action == "seek" {
                    json!({"position":4.})
                } else {
                    json!({})
                },
            ),
        };
        assert_eq!(
            call(&router, method, &path, Some(&f.token), Some(body))
                .await
                .0,
            StatusCode::OK,
            "{action}"
        );
        let before = current(&router, &f.token).await;
        assert_eq!(
            recover(
                &router,
                &f.token,
                request(
                    &z,
                    &p,
                    vec![boundary(&z["playback_id"], &z["track"]["id"], 1.)],
                    false
                )
            )
            .await["accepted"],
            false,
            "{action}"
        );
        assert_eq!(current(&router, &f.token).await, before, "{action}");
        // Revocation remains effective through another reconstruction of Core state.
        let (_, again) = restart(&f).await;
        register(&again, &f.token).await;
        assert_eq!(
            recover(
                &again,
                &f.token,
                request(
                    &z,
                    &p,
                    vec![boundary(&z["playback_id"], &z["track"]["id"], 1.)],
                    false
                )
            )
            .await["accepted"],
            false,
            "restart {action}"
        );
    }
}

#[tokio::test]
async fn invalid_navigation_and_malformed_cursor_do_not_destroy_the_valid_lease() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    assert_eq!(
        call(
            &router,
            "POST",
            &format!("{ZONE}/seek"),
            Some(&f.token),
            Some(json!({"position":-1.}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let original = request(
        &z,
        &p,
        vec![boundary(&z["playback_id"], &z["track"]["id"], 1.)],
        false,
    );
    for field in ["frames", "order", "rate"] {
        let mut body = original.clone();
        match field {
            "frames" => {
                body["heard"][0]["position"] = json!(1000.);
            }
            "order" => {
                body["heard"][0]["track_id"] = json!("local-unknown");
            }
            _ => {
                body["sample_rate"] = json!(1);
            }
        }
        assert_eq!(
            call(
                &router,
                "POST",
                &format!("{ZONE}/recover"),
                Some(&f.token),
                Some(body)
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "{field}"
        );
    }
    assert_eq!(recover(&router, &f.token, original).await["accepted"], true);
}

#[tokio::test]
async fn recovery_ack_loss_is_idempotent_but_never_revives_a_later_stop() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let body = request(
        &z,
        &p,
        vec![boundary(&z["playback_id"], &z["track"]["id"], 1.)],
        false,
    );
    let first = recover(&router, &f.token, body.clone()).await;
    let again = recover(&router, &f.token, body.clone()).await;
    assert_eq!(again["applied"], false);
    assert_eq!(again["plan_id"], first["plan_id"]);
    assert_eq!(again["zone"], first["zone"]);
    let mut conflict = body.clone();
    conflict["terminal"] = json!(true);
    assert_eq!(
        call(
            &router,
            "POST",
            &format!("{ZONE}/recover"),
            Some(&f.token),
            Some(conflict)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    call(
        &router,
        "POST",
        &format!("{ZONE}/stop"),
        Some(&f.token),
        Some(json!({})),
    )
    .await;
    let stopped = current(&router, &f.token).await;
    assert_eq!(recover(&router, &f.token, body).await["accepted"], false);
    assert_eq!(current(&router, &f.token).await, stopped);
}

#[tokio::test]
async fn recovery_transaction_failure_rolls_back_zone_claims_and_receipt() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let (state, router) = restart(&f).await;
    register(&router, &f.token).await;
    let before = current(&router, &f.token).await;
    let counts = state
        .db
        .run(|c| {
            Ok(c.query_row("SELECT COUNT(*) FROM listen_claims", [], |r| {
                r.get::<_, i64>(0)
            })?)
        })
        .await
        .unwrap();
    state.db.run(|c|{c.execute_batch("CREATE TRIGGER fail_recover BEFORE INSERT ON sequence_recovery_receipts BEGIN SELECT RAISE(FAIL,'QA restore rollback'); END;")?;Ok(())}).await.unwrap();
    let body = request(
        &z,
        &p,
        vec![boundary(&z["playback_id"], &z["track"]["id"], 1.)],
        false,
    );
    assert_eq!(
        call(
            &router,
            "POST",
            &format!("{ZONE}/recover"),
            Some(&f.token),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(current(&router, &f.token).await, before);
    state
        .db
        .run(move |c| {
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM listen_claims", [], |r| r
                    .get::<_, i64>(0))?,
                counts
            );
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM sequence_recovery_receipts", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            c.execute_batch("DROP TRIGGER fail_recover")?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(recover(&router, &f.token, body).await["applied"], true);
}

#[tokio::test]
async fn browser_disconnect_is_session_scoped_and_keeps_latest_pause_for_recovery() {
    let f=durable(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    let (_,login)=f.call("POST","/api/v1/auth/login",Some(json!({"username":"admin","password":"correct-test-password"}))).await;
    let new_token=login["token"].as_str().unwrap();register(&f.app,new_token).await;
    assert!(current(&f.app,new_token).await["playback_id"].is_null());
    let body=request(&z,&p,vec![boundary(&z["playback_id"],&z["track"]["id"],0.25)],false);
    let bound=recover(&f.app,new_token,body.clone()).await;
    let path="/api/v1/endpoints/browser/browser-continuous/disconnect";
    assert_eq!(f.call("POST",path,None).await.0,StatusCode::NO_CONTENT);
    assert_eq!(current(&f.app,new_token).await,bound["zone"]);
    assert_eq!(f.s.endpoints.read().await["browser-continuous"]["online"],true);
    assert_eq!(call(&f.app,"POST",path,Some(new_token),None).await.0,StatusCode::NO_CONTENT);
    assert_eq!(f.s.endpoints.read().await["browser-continuous"]["online"],false);
    assert!(current(&f.app,new_token).await["playback_id"].is_null());
    assert_eq!(f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await.0,StatusCode::OK);
    register(&f.app,&f.token).await;
    let resumed=recover(&f.app,&f.token,body).await;assert_eq!(resumed["accepted"],true);assert_eq!(resumed["zone"]["status"],"paused");
}
#[tokio::test]
async fn another_listener_registration_suspends_live_audio_without_taking_its_saved_plan() {
    let f=durable(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    let token=crate::auth::random_token();let hash=crate::auth::hash(&token);
    f.s.db.run(move |c|{c.execute("INSERT INTO users(id,username,password,role) VALUES('other','other','unused','member')",[])?;c.execute("INSERT INTO sessions(hash,user_id,expires,scopes,created_at) VALUES(?1,'other',?2,'[\"control\",\"read\"]',0)",rusqlite::params![hash,chrono::Utc::now().timestamp()+60])?;Ok(())}).await.unwrap();
    register(&f.app,&token).await;
    let suspended=current(&f.app,&token).await;assert_eq!(suspended["status"],"paused");assert!(suspended["playback_id"].is_null());
    let missing=json!({"endpoint_id":z["endpoint_id"]});
    assert_eq!(f.call("POST",&format!("{ZONE}/recover/unavailable"),Some(missing.clone())).await.0,StatusCode::FORBIDDEN,"old registration cannot discard the new listener's pending recovery");
    assert_eq!(call(&f.app,"POST",&format!("{ZONE}/recover/unavailable"),Some(&token),Some(missing)).await.1["applied"],false,"another account cannot discard the original owner's cursor");
    assert_eq!(current(&f.app,&token).await,suspended);
    let (status,prepared)=call(&f.app,"POST",&format!("{ZONE}/prepare"),Some(&token),Some(json!({"endpoint_id":z["endpoint_id"],"track_id":z["track"]["id"],"playback_id":z["playback_id"]}))).await;
    assert_eq!(status,StatusCode::OK);assert_eq!(prepared["accepted"],false);
    register(&f.app,&f.token).await;
    let restored=recover(&f.app,&f.token,request(&z,&p,vec![boundary(&z["playback_id"],&z["track"]["id"],0.25)],false)).await;
    assert_eq!(restored["accepted"],true);assert_eq!(restored["zone"]["status"],"playing");
}
#[tokio::test]
async fn another_account_cannot_recover_a_saved_plan_even_after_owning_the_endpoint() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let (_, router) = restart(&f).await;
    let token = crate::auth::random_token();
    let hash = crate::auth::hash(&token);
    f.s.db.run(move |c|{c.execute("INSERT INTO users(id,username,password,role) VALUES('other','other','unused','member')",[])?;c.execute("INSERT INTO sessions(hash,user_id,expires,scopes,created_at) VALUES(?1,'other',?2,'[\"control\",\"read\"]',0)",rusqlite::params![hash,chrono::Utc::now().timestamp()+60])?;Ok(())}).await.unwrap();
    register(&router, &token).await;
    let body = request(
        &z,
        &p,
        vec![boundary(&z["playback_id"], &z["track"]["id"], 1.)],
        false,
    );
    assert_eq!(
        call(
            &router,
            "POST",
            &format!("{ZONE}/recover"),
            Some(&token),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    register(&router, &f.token).await;
    assert_eq!(recover(&router, &f.token, body).await["accepted"], true);
}

#[tokio::test]
async fn lost_restore_ack_followed_by_another_core_restart_uses_the_latest_receipt_cursor() {
    let f = durable(&["local-shared", "local-unknown", "local-shared"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let slot = &p["items"][0];
    let body = request(
        &z,
        &p,
        vec![
            boundary(&z["playback_id"], &z["track"]["id"], 2.),
            boundary(&slot["playback_id"], &slot["track"]["id"], 1.),
        ],
        false,
    );
    let first = recover(&router, &f.token, body.clone()).await;
    let current = &first["zone"];
    let (_,next_plan)=call(&router,"POST",&format!("{ZONE}/prepare"),Some(&f.token),Some(json!({"endpoint_id":current["endpoint_id"],"track_id":current["track"]["id"],"playback_id":current["playback_id"]}))).await;
    elapsed(&f, 40).await;
    let (_, ahead) = call(
        &router,
        "POST",
        &format!("{ZONE}/handoff"),
        Some(&f.token),
        Some(handoff(current, &next_plan, 1, 3., 0., 0.25)),
    )
    .await;
    assert_eq!(ahead["applied"], true);
    call(
        &router,
        "POST",
        &format!("{ZONE}/pause"),
        Some(&f.token),
        Some(json!({})),
    )
    .await;
    let before = totals(&f).await;
    let (_, again) = restart(&f).await;
    register(&again, &f.token).await;
    let second = recover(&again, &f.token, body.clone()).await;
    assert_eq!(second["accepted"], true);
    assert_eq!(second["zone"]["queue_index"], 2);
    assert_eq!(second["zone"]["position"], 0.25);
    assert_eq!(second["zone"]["status"], "paused");
    assert_ne!(first["zone"]["playback_id"], second["zone"]["playback_id"]);
    assert_eq!(recover(&again, &f.token, body).await["applied"], false);
    assert_eq!(totals(&f).await, before);
}

#[tokio::test]
async fn recovery_respects_repeat_one_after_all_nine_reserved_instances_were_heard() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    f.call("PATCH", ZONE, Some(json!({"repeat":"one"}))).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let mut heard = vec![boundary(&z["playback_id"], &z["track"]["id"], 0.125)];
    heard.extend(
        p["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|slot| boundary(&slot["playback_id"], &slot["track"]["id"], 0.125)),
    );
    assert_eq!(heard.len(), 9);
    let body = request(&z, &p, heard.clone(), true);
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let result = recover(&router, &f.token, body).await;
    assert_eq!(result["accepted"], true);
    assert_eq!(result["finished"], false);
    assert_eq!(result["zone"]["queue_index"], 0);
    assert_eq!(result["zone"]["position"], 0.);
    assert!(heard
        .iter()
        .all(|r| r["playback_id"] != result["zone"]["playback_id"]));
    assert_eq!(totals(&f).await["tracks"], json!([]));
}

#[tokio::test]
async fn a_durable_cursor_older_than_retained_history_resumes_the_confirmed_queue_only() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    f.call("PATCH", ZONE, Some(json!({"repeat":"one"}))).await;
    let initial = zone(&f).await;
    let original = prepare(&f).await;
    for _ in 0..10 {
        let current = zone(&f).await;
        let plan = prepare(&f).await;
        let (_, result) = f
            .call(
                "POST",
                &format!("{ZONE}/handoff"),
                Some(handoff(&current, &plan, 1, 0.25, 0., 0.125)),
            )
            .await;
        assert_eq!(result["applied"], true, "{result}");
    }
    let ahead = zone(&f).await;
    let before = totals(&f).await;
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let result = recover(
        &router,
        &f.token,
        request(
            &initial,
            &original,
            vec![boundary(
                &initial["playback_id"],
                &initial["track"]["id"],
                0.25,
            )],
            true,
        ),
    )
    .await;
    assert_eq!(result["accepted"], true);
    assert_eq!(result["finished"], false);
    assert_eq!(result["zone"]["queue_index"], ahead["queue_index"]);
    assert_eq!(result["zone"]["position"], ahead["position"]);
    assert_ne!(result["zone"]["playback_id"], ahead["playback_id"]);
    assert_eq!(totals(&f).await, before);
}

#[tokio::test]
async fn recovery_applies_the_reserved_shuffled_wrap_and_retains_display_metadata() {
    let f = durable(&["local-shared", "local-unknown", "qq-only"]).await;
    f.call("PATCH", ZONE, Some(json!({"repeat":"all","shuffle":true})))
        .await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let slots = p["items"].as_array().unwrap();
    let mut heard = vec![boundary(&z["playback_id"], &z["track"]["id"], 0.125)];
    heard.extend(
        slots
            .iter()
            .take(3)
            .map(|slot| boundary(&slot["playback_id"], &slot["track"]["id"], 0.125)),
    );
    f.s.db.run(|c|{c.execute("UPDATE tracks SET title='refreshed display' WHERE id IN ('local-shared','local-unknown','qq-only')",[])?;Ok(())}).await.unwrap();
    let mut model = f.s.zones.read().await["zone-local"].clone();
    for t in &mut model.queue {
        t.title = "refreshed display".into();
    }
    model.track = model.queue.get(model.queue_index).cloned();
    f.s.save_zone(model).await.unwrap();
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let result = recover(&router, &f.token, request(&z, &p, heard, false)).await;
    assert_eq!(result["accepted"], true);
    assert_eq!(result["zone"]["queue_index"], 0);
    assert_eq!(result["zone"]["track"]["id"], slots[2]["track"]["id"]);
    assert_eq!(result["zone"]["track"]["title"], "refreshed display");
    assert_eq!(result["zone"]["position"], 0.125);
}

#[tokio::test]
async fn earlier_seek_source_eof_does_not_finish_the_current_source_in_the_same_listen_group() {
    let f = durable(&["local-shared", "local-unknown"]).await;
    let old = zone(&f).await;
    let old_plan = prepare(&f).await;
    let (_, seek) = f
        .call(
            "POST",
            &format!("{ZONE}/seek"),
            Some(json!({"position":5.})),
        )
        .await;
    let plan = prepare(&f).await;
    let history = measured_batch(
        &f,
        vec![measured_snapshot(
            &old["playback_id"],
            &old_plan["listen_session_id"],
            &old["track"]["id"],
            0.,
            30.,
            "ended",
        )],
    )
    .await;
    assert_eq!(
        f.call("POST", "/api/v1/stats/offline", Some(history))
            .await
            .1["rejected"],
        json!([])
    );
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    let mut b = boundary(&seek["playback_id"], &seek["track"]["id"], 6.);
    b["frames"] = json!(48000);
    let restored = recover(&router, &f.token, request(&seek, &plan, vec![b], false)).await;
    assert_eq!(restored["accepted"], true);
    assert_eq!(restored["finished"], false);
    assert_eq!(restored["zone"]["queue_index"], 0);
    assert_eq!(restored["zone"]["position"], 6.);
}

#[tokio::test]
async fn registered_rest_interruptions_preserve_intent_and_reject_stale_or_mixed_reports() {
    for paused in [false,true] {
        let f=durable(&["local-shared","local-unknown"]).await;
        let z=zone(&f).await;let p=prepare(&f).await;assert_eq!(p["continuous_recovery"],true);
        if paused {f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await;}
        let before=current(&f.app,&f.token).await;
        let report=json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"recoverable":true});
        let mut stale=report.clone();stale["playback_id"]=json!(uuid::Uuid::new_v4().to_string());
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(stale)).await.0,StatusCode::NO_CONTENT);
        assert_eq!(current(&f.app,&f.token).await,before);
        let mut mixed=report.clone();mixed["position"]=json!(1.);
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(mixed)).await.0,StatusCode::BAD_REQUEST);
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(report.clone())).await.0,StatusCode::NO_CONTENT);
        let suspended=current(&f.app,&f.token).await;assert_eq!(suspended["status"],"paused");assert!(suspended["playback_id"].is_null());
        let result=recover(&f.app,&f.token,request(&z,&p,vec![boundary(&z["playback_id"],&z["track"]["id"],0.25)],false)).await;
        assert_eq!(result["accepted"],true);assert_eq!(result["zone"]["status"],if paused {"paused"}else{"playing"});
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(report)).await.0,StatusCode::NO_CONTENT);
        assert_eq!(current(&f.app,&f.token).await,result["zone"]);
    }
    let f=fixture(&["local-shared"]).await;let z=zone(&f).await;
    assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"recoverable":true}))).await.0,StatusCode::BAD_REQUEST);
}
#[tokio::test]
async fn play_during_suspension_keeps_the_saved_queue_and_latest_intent() {
    for paused in [false,true] {
        let f=durable(&["local-shared","local-unknown"]).await;
        let z=zone(&f).await;let p=prepare(&f).await;
        let old=f.s.zones.read().await["zone-local"].clone();
        crate::stats::finish(&f.s,&old,"disconnect").await.unwrap();
        crate::sequence::recovery::suspend(&f.s,&old).await.unwrap();
        let (status,pending)=f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;
        assert_eq!(status,StatusCode::OK);assert!(pending["playback_id"].is_null(),"resume must wait for the actual local checkpoint");
        assert_eq!(pending["status"],"playing");
        if paused {f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await;}
        let slot=&p["items"][0];
        let result=recover(&f.app,&f.token,request(&z,&p,vec![boundary(&z["playback_id"],&z["track"]["id"],30.),boundary(&slot["playback_id"],&slot["track"]["id"],1.25)],false)).await;
        assert_eq!(result["accepted"],true);assert_eq!(result["zone"]["queue_index"],1);assert_eq!(result["zone"]["position"],1.25);
        assert_eq!(result["zone"]["status"],if paused {"paused"}else{"playing"});
        assert_eq!(totals(&f).await["tracks"],json!([]));
    }
}
#[tokio::test]
async fn a_missing_local_checkpoint_only_fulfils_an_explicit_pending_play() {
    for playing in [false,true] {
        let f=durable(&["local-shared","local-unknown"]).await;
        let z=zone(&f).await;let plan=prepare(&f).await;
        let old=f.s.zones.read().await["zone-local"].clone();
        crate::stats::finish(&f.s,&old,"disconnect").await.unwrap();
        crate::sequence::recovery::suspend(&f.s,&old).await.unwrap();
        if playing {f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;}
        let body=json!({"endpoint_id":z["endpoint_id"]});
        let (status,result)=f.call("POST",&format!("{ZONE}/recover/unavailable"),Some(body.clone())).await;
        assert_eq!(status,StatusCode::OK);assert_eq!(result["applied"],true);
        assert_eq!(result["zone"]["status"],if playing {"playing"}else{"paused"});
        assert_eq!(result["zone"]["playback_id"].is_string(),playing);
        assert_eq!(totals(&f).await["tracks"],json!([]));
        let original=request(&z,&plan,vec![boundary(&z["playback_id"],&z["track"]["id"],1.)],false);
        assert_eq!(recover(&f.app,&f.token,original).await["accepted"],false);
        if !playing {f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;}
        let current=zone(&f).await;let fresh=prepare(&f).await;
        assert_eq!(fresh["accepted"],true);
        assert_eq!(f.call("POST",&format!("{ZONE}/recover/unavailable"),Some(body)).await.1["applied"],false);
        assert_eq!(zone(&f).await,current,"late missing-checkpoint notification must leave a new instance alone");
    }
}
#[tokio::test]
async fn legacy_durable_control_and_explicit_replay_keep_their_original_behavior() {
    for legacy in [false,true] {
        let f=durable(&["local-shared","local-unknown"]).await;
        let z=zone(&f).await;let plan=prepare(&f).await;
        let old=f.s.zones.read().await["zone-local"].clone();
        crate::stats::finish(&f.s,&old,"disconnect").await.unwrap();
        crate::sequence::recovery::suspend(&f.s,&old).await.unwrap();
        if legacy {f.s.endpoints.write().await.get_mut("browser-continuous").unwrap()["capabilities"]["recovery_controls"]=json!(false);}
        let body=if legacy {json!({})}else{json!({"track_id":z["track"]["id"]})};
        let (status,next)=f.call("POST",&format!("{ZONE}/play"),Some(body)).await;
        assert_eq!(status,StatusCode::OK);assert!(next["playback_id"].is_string());
        assert_eq!(recover(&f.app,&f.token,request(&z,&plan,vec![boundary(&z["playback_id"],&z["track"]["id"],1.)],false)).await["accepted"],false);
    }
}
#[tokio::test]
async fn legacy_continuous_endpoint_does_not_silently_acquire_persistent_resume() {
    let f = fixture(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    assert_eq!(p["continuous_recovery"],false);
    let (_, router) = restart(&f).await;
    register(&router, &f.token).await;
    assert_eq!(
        recover(
            &router,
            &f.token,
            request(
                &z,
                &p,
                vec![boundary(&z["playback_id"], &z["track"]["id"], 1.)],
                false
            )
        )
        .await["accepted"],
        false
    );
    assert_eq!(current(&router, &f.token).await["status"], "stopped");
}
#[tokio::test]
async fn an_explicit_paused_seek_after_suspension_can_prepare_without_crediting_its_offset() {
    let f=durable(&["local-shared","local-unknown"]).await;let old=f.s.zones.read().await["zone-local"].clone();
    prepare(&f).await;crate::stats::finish(&f.s,&old,"disconnect").await.unwrap();
    crate::sequence::recovery::suspend(&f.s,&old).await.unwrap();
    let (_,seek)=f.call("POST",&format!("{ZONE}/seek"),Some(json!({"position":0.5}))).await;
    assert_eq!(seek["status"],"paused");assert_ne!(seek["playback_id"],old.playback_id.unwrap());
    let prepared=prepare(&f).await;assert_eq!(prepared["accepted"],true);assert_eq!(totals(&f).await["tracks"],json!([]));
    device_progress(&f,&seek,0.75,0.25).await;
    assert_eq!(totals(&f).await["tracks"][0]["seconds"],0.25);assert_eq!(zone(&f).await["status"],"paused");
}
#[tokio::test]
async fn late_eof_corrects_disconnected_history_but_cannot_relabel_a_newer_seek_source() {
    for (seek,first) in [(false,false),(false,true),(true,false),(true,true)] {
        let f=durable(&["local-shared","local-unknown"]).await;let original=zone(&f).await;let plan=prepare(&f).await;
        device_progress(&f,&original,20.,20.).await;
        if seek {f.call("POST",&format!("{ZONE}/seek"),Some(json!({"position":5.}))).await;prepare(&f).await;}
        let batch=measured_batch(&f,vec![measured_snapshot(&original["playback_id"],&plan["listen_session_id"],&original["track"]["id"],0.,30.,"ended")]).await;
        if first {assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch.clone())).await.1["rejected"],json!([]));}
        let old=f.s.zones.read().await["zone-local"].clone();crate::stats::finish(&f.s,&old,"disconnect").await.unwrap();
        crate::sequence::recovery::suspend(&f.s,&old).await.unwrap();
        if !first {assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch)).await.1["rejected"],json!([]));}
        let (reason,position)=f.s.db.run(|c|Ok(c.query_row("SELECT finish_reason,last_position FROM play_sessions WHERE zone_id='zone-local' ORDER BY started_at LIMIT 1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?)))?)).await.unwrap();
        assert_eq!(reason,if seek{"disconnect"}else{"ended"});assert_eq!(position,if seek{5.}else{30.});
    }
}

#[tokio::test]
async fn real_socket_takeover_rejects_old_reports_and_disconnect_preserves_resume_intent() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{
        tungstenite::{client::IntoClientRequest, Message},
        MaybeTlsStream, WebSocketStream,
    };
    type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
    async fn barrier(ws: &mut Socket, marker: &[u8]) {
        ws.send(Message::Ping(marker.to_vec().into()))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while let Some(message) = ws.next().await {
                match message.unwrap() {
                    Message::Pong(bytes) if bytes[..] == *marker => return,
                    Message::Ping(bytes) => ws.send(Message::Pong(bytes)).await.unwrap(),
                    _ => {}
                }
            }
            panic!("socket closed before barrier");
        })
        .await
        .unwrap();
    }
    async fn connect(address: std::net::SocketAddr, token: &str) -> Socket {
        let mut request = format!("ws://{address}/api/v1/bridge/events")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse().unwrap());
        let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        ws.send(Message::Text(json!({"type":"register","endpoint":{"id":"recovery-alsa","kind":"alsa","capabilities":{"continuous_pcm":true,"continuous_recovery":true,"recovery_controls":true,"playback_instance":true,"media_url_api_relative":true}}}).to_string().into())).await.unwrap();
        barrier(&mut ws, b"registered").await;
        ws
    }
    let f = fixture(&["local-shared", "local-unknown"]).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = f.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut first = connect(address, &f.token).await;
    assert_eq!(
        f.call("PATCH", ZONE, Some(json!({"endpoint_id":"recovery-alsa"})))
            .await
            .0,
        StatusCode::OK
    );
    f.call("POST", &format!("{ZONE}/play"), Some(json!({})))
        .await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let mut second = connect(address, &f.token).await;
    first.send(Message::Text(json!({"type":"state","zone_id":"zone-local","track_id":z["track"]["id"],"core_playback_id":z["playback_id"],"status":"ended","position":0.25,"frames":12000,"sample_rate":48000}).to_string().into())).await.unwrap();
    barrier(&mut first, b"stale").await;
    assert_eq!(zone(&f).await, z);
    first.close(None).await.unwrap();
    barrier(&mut second, b"still-owned").await;
    assert_eq!(f.s.endpoints.read().await["recovery-alsa"]["online"], true);
    second.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let suspended = zone(&f).await;
            if f.s.endpoints.read().await["recovery-alsa"]["online"] == false
                && suspended["status"] == "paused"
                && suspended["playback_id"].is_null()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let suspended = zone(&f).await;
    assert_eq!(suspended["status"], "paused");
    assert!(suspended["playback_id"].is_null());
    let mut third = connect(address, &f.token).await;
    let result = recover(
        &f.app,
        &f.token,
        request(
            &z,
            &p,
            vec![boundary(&z["playback_id"], &z["track"]["id"], 0.25)],
            false,
        ),
    )
    .await;
    assert_eq!(result["accepted"], true);
    assert_eq!(result["zone"]["status"], "playing");
    assert_eq!(result["zone"]["position"], 0.25);
    assert_ne!(result["zone"]["playback_id"], z["playback_id"]);
    let active=&result["zone"];
    let (_,plan)=f.call("POST",&format!("{ZONE}/prepare"),Some(json!({"endpoint_id":active["endpoint_id"],"track_id":active["track"]["id"],"playback_id":active["playback_id"]}))).await;
    third.send(Message::Text(json!({"type":"error","zone_id":"zone-local","track_id":active["track"]["id"],"core_playback_id":active["playback_id"],"recoverable":true,"error":"QA control transport"}).to_string().into())).await.unwrap();
    barrier(&mut third,b"suspended-error").await;
    assert_eq!(zone(&f).await["status"],"paused");assert!(zone(&f).await["playback_id"].is_null());
    let mut source=boundary(&active["playback_id"],&active["track"]["id"],0.25);source["frames"]=json!(0);
    let again=recover(&f.app,&f.token,request(active,&plan,vec![source],false)).await;
    assert_eq!(again["accepted"],true);assert_eq!(again["zone"]["status"],"playing");
    let active=&again["zone"];
    let (_,plan)=f.call("POST",&format!("{ZONE}/prepare"),Some(json!({"endpoint_id":active["endpoint_id"],"track_id":active["track"]["id"],"playback_id":active["playback_id"]}))).await;
    third.send(Message::Text(json!({"type":"error","zone_id":"zone-local","track_id":active["track"]["id"],"core_playback_id":active["playback_id"],"recoverable":true,"source_failed":true,"error":"QA truncated source tail"}).to_string().into())).await.unwrap();
    barrier(&mut third,b"source-wait").await;
    let mut source=boundary(&active["playback_id"],&active["track"]["id"],0.25);source["frames"]=json!(0);
    let body=request(active,&plan,vec![source],false);
    let waiting=recover(&f.app,&f.token,body.clone()).await;
    assert_eq!(waiting["waiting_for_play"],true,"a failed source must not enter an automatic reconnect/redecode loop");
    assert_eq!(zone(&f).await["queue_index"],active["queue_index"],"source failure is not proven EOF");
    f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;
    barrier(&mut third,b"explicit-source-retry").await;
    let resumed=recover(&f.app,&f.token,body).await;
    assert_eq!(resumed["accepted"],true);assert_eq!(resumed["zone"]["position"],0.25);
    let active=&resumed["zone"];
    let (_,plan)=f.call("POST",&format!("{ZONE}/prepare"),Some(json!({"endpoint_id":active["endpoint_id"],"track_id":active["track"]["id"],"playback_id":active["playback_id"]}))).await;
    third.send(Message::Text(json!({"type":"error","zone_id":"zone-local","track_id":active["track"]["id"],"core_playback_id":active["playback_id"],"recoverable":true,"device_failed":true,"error":"QA device resources invalidated"}).to_string().into())).await.unwrap();
    barrier(&mut third,b"device-wait").await;
    let mut source=boundary(&active["playback_id"],&active["track"]["id"],0.25);source["frames"]=json!(0);
    let body=request(active,&plan,vec![source],false);
    assert_eq!(recover(&f.app,&f.token,body.clone()).await["waiting_for_play"],true);
    f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        while let Some(message)=third.next().await {match message.unwrap(){
            Message::Text(text)=>{let command:Value=serde_json::from_str(&text).unwrap();if command["type"]=="resume" {assert!(command["playback_id"].is_null());return;}},
            Message::Ping(bytes)=>third.send(Message::Pong(bytes)).await.unwrap(),_=>{}
        }}panic!("waiting device did not receive an explicit resume wake");
    }).await.unwrap();
    assert_eq!(recover(&f.app,&f.token,body).await["accepted"],true);
    f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await;
    let (_,seek)=f.call("POST",&format!("{ZONE}/seek"),Some(json!({"position":0.5}))).await;
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        while let Some(message)=third.next().await {match message.unwrap(){
            Message::Text(text)=>{let command:Value=serde_json::from_str(&text).unwrap();if command["type"]=="seek" {
                assert_eq!(command["playback_id"],seek["playback_id"]);assert_eq!(command["paused"],true);
                assert!(command["url"].as_str().unwrap().starts_with("api/v1/media/"));return;
            }},Message::Ping(bytes)=>third.send(Message::Pong(bytes)).await.unwrap(),_=>{}
        }}panic!("missing seek command for the registered durable endpoint");
    }).await.unwrap();
    third.close(None).await.unwrap();
    server.abort();
}
#[tokio::test]
async fn system_suspend_recovery_without_a_delivered_error_waits_for_manual_play() {
    let f=durable(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let plan=prepare(&f).await;
    let mut body=request(&z,&plan,vec![json!({"playback_id":z["playback_id"],"track_id":z["track"]["id"],"position":0.25,"frames":12000})],false);
    body["system_suspend"]=json!({"host_id":"e70465ba-68d8-444d-b8ed-7bfb90243872","generation":1});
    let waiting=recover(&f.app,&f.token,body.clone()).await;
    assert_eq!(waiting["waiting_for_play"],true,"重连不能恢复挂起前的 playing 意图");assert_eq!(waiting["accepted"],false);
    let (_,router)=restart(&f).await;register(&router,&f.token).await;
    assert_eq!(recover(&router,&f.token,body.clone()).await["waiting_for_play"],true);
    call(&router,"POST",&format!("{ZONE}/play"),Some(&f.token),Some(json!({}))).await;
    let accepted=recover(&router,&f.token,body.clone()).await;assert_eq!(accepted["accepted"],true);assert_eq!(accepted["system_suspend_ack"],body["system_suspend"]);
    assert_eq!(recover(&router,&f.token,body.clone()).await["applied"],false,"确认丢失重传不能再次计听音");
    body["system_suspend"]["generation"]=json!(2);
    assert_eq!(recover(&router,&f.token,body.clone()).await["waiting_for_play"],true,"第二次挂起须重新等待手动播放");
    call(&router,"POST",&format!("{ZONE}/play"),Some(&f.token),Some(json!({}))).await;
    assert_eq!(recover(&router,&f.token,body.clone()).await["accepted"],true);
    body["system_suspend"]["generation"]=json!(1);
    assert_ne!(recover(&router,&f.token,body).await["waiting_for_play"],true,"迟到的旧挂起不能撤销较新的手动确认");
}

#[tokio::test]
async fn system_suspend_reports_validate_before_mutation_and_match_paused_playback() {
    for paused in [false,true] {
        let f=durable(&["local-shared"]).await;
        if paused {f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await;}
        let z=zone(&f).await;let plan=prepare(&f).await;
        let stamp=json!({"host_id":"e70465ba-68d8-444d-b8ed-7bfb90243872","generation":3});
        let report=json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"recoverable":true,"device_failed":true,"system_suspend":stamp});
        for invalid in [json!({"host_id":"invalid","generation":3}),json!({"host_id":stamp["host_id"],"generation":0}),json!({"host_id":stamp["host_id"],"generation":3,"unknown":true})] {
            let mut bad=report.clone();bad["system_suspend"]=invalid.clone();
            assert!(f.call("POST",&format!("{ZONE}/progress"),Some(bad)).await.0.is_client_error());
            let mut bad=request(&z,&plan,vec![],false);bad["system_suspend"]=invalid;
            assert!(f.call("POST",&format!("{ZONE}/recover"),Some(bad)).await.0.is_client_error());
            assert_eq!(zone(&f).await,z);
        }
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(report)).await.0,StatusCode::NO_CONTENT);
        let mut body=request(&z,&plan,vec![json!({"playback_id":z["playback_id"],"track_id":z["track"]["id"],"position":0.25,"frames":12000})],false);body["system_suspend"]=stamp.clone();
        assert_eq!(recover(&f.app,&f.token,body.clone()).await["waiting_for_play"],true);
        f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;
        let restored=recover(&f.app,&f.token,body).await;assert_eq!(restored["accepted"],true);
        let active=zone(&f).await;
        prepare(&f).await;
        let stale=json!({"track_id":active["track"]["id"],"playback_id":active["playback_id"],"recoverable":true,"device_failed":true,"system_suspend":stamp});
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(stale)).await.0,StatusCode::NO_CONTENT);
        assert_eq!(zone(&f).await,active,"迟到的挂起报告不能结束新的播放实例");
        assert_eq!(f.call("GET","/api/v1/status",None).await.1["capabilities"]["system_suspend_recovery"],true);
    }
}

#[tokio::test]
async fn system_suspend_without_a_cursor_requires_a_new_manual_instance_across_restart() {
    let f=durable(&["local-shared"]).await;let original=zone(&f).await;
    let path=format!("{ZONE}/recover/unavailable");
    let mut body=json!({"endpoint_id":original["endpoint_id"],"system_suspend":{"host_id":"e70465ba-68d8-444d-b8ed-7bfb90243872","generation":4}});
    let (status,held)=f.call("POST",&path,Some(body.clone())).await;assert_eq!(status,StatusCode::OK,"{held}");
    assert_eq!(held["waiting_for_play"],true);assert_eq!(held["accepted"],false);assert!(held["zone"]["playback_id"].is_null());
    assert_eq!(totals(&f).await["tracks"],json!([]),"没有游标不能伪造听音帧");
    let (_,router)=restart(&f).await;register(&router,&f.token).await;
    assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(body.clone())).await.1["waiting_for_play"],true);
    let mut manual=call(&router,"POST",&format!("{ZONE}/play"),Some(&f.token),Some(json!({}))).await.1;
    body["playback_id"]=manual["playback_id"].clone();
    let (_,accepted)=call(&router,"POST",&path,Some(&f.token),Some(body.clone())).await;
    assert_eq!(accepted["accepted"],true);assert_eq!(accepted["system_suspend_ack"],body["system_suspend"]);assert_eq!(accepted["zone"]["playback_id"],manual["playback_id"]);
    assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(body.clone())).await.1["applied"],false);
    let (_,router)=restart(&f).await;register(&router,&f.token).await;
    body.as_object_mut().unwrap().remove("playback_id");
    assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(body.clone())).await.1["waiting_for_play"],true,"重启不能复用旧实例的手动准入");
    manual=call(&router,"POST",&format!("{ZONE}/play"),Some(&f.token),Some(json!({}))).await.1;
    body["playback_id"]=manual["playback_id"].clone();
    assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(body.clone())).await.1["accepted"],true);
    body["playback_id"]=original["playback_id"].clone();assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(body.clone())).await.1["accepted"],false);
    assert_eq!(current(&router,&f.token).await["playback_id"],manual["playback_id"]);
    body["playback_id"]=manual["playback_id"].clone();body["system_suspend"]["generation"]=json!(5);
    assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(body.clone())).await.1["waiting_for_play"],true);
    body["system_suspend"]["generation"]=json!(4);assert_eq!(call(&router,"POST",&path,Some(&f.token),Some(body)).await.1["accepted"],false);
}

#[tokio::test]
async fn system_suspend_fresh_admission_rejects_bad_or_stale_bindings_without_mutation() {
    let f=fixture(&["local-shared"]).await;let z=zone(&f).await;let path=format!("{ZONE}/recover/unavailable");
    let body=json!({"endpoint_id":z["endpoint_id"],"system_suspend":{"host_id":"e70465ba-68d8-444d-b8ed-7bfb90243872","generation":1}});
    assert_eq!(f.call("POST",&path,Some(body.clone())).await.0,StatusCode::BAD_REQUEST);assert_eq!(zone(&f).await,z);
    register(&f.app,&f.token).await;
    let mut invalid=body.clone();invalid["system_suspend"]["generation"]=json!(0);
    assert_eq!(f.call("POST",&path,Some(invalid)).await.0,StatusCode::BAD_REQUEST);assert_eq!(zone(&f).await,z);
    let mut stale=body.clone();stale["playback_id"]=json!(uuid::Uuid::new_v4().to_string());
    assert_eq!(f.call("POST",&path,Some(stale)).await.1["accepted"],false);assert_eq!(zone(&f).await,z);
    let mut foreign=body;foreign["endpoint_id"]=json!("browser-unregistered");
    assert!(f.call("POST",&path,Some(foreign)).await.0.is_client_error());assert_eq!(zone(&f).await,z);
    assert_eq!(f.call("GET","/api/v1/status",None).await.1["capabilities"]["system_suspend_fresh_start"],true);
}


#[tokio::test]
async fn queue_refresh_keeps_restart_recovery_bound_to_the_latest_queue() {
    let f=durable(&["local-shared","local-unknown","local-shared"]).await;refresh_capable(&f).await;
    let z=zone(&f).await;let p=prepare(&f).await;
    assert_eq!(f.call("PUT",&format!("{ZONE}/queue"),Some(json!({"track_ids":["local-shared","local-shared"]}))).await.0,StatusCode::OK);
    let refreshed=refresh_preparation(&f,&p,vec![]).await;assert_eq!(refreshed["plan_id"],p["plan_id"]);
    let(_,router)=restart(&f).await;register(&router,&f.token).await;
    let restored=recover(&router,&f.token,request(&z,&p,vec![boundary(&z["playback_id"],&z["track"]["id"],0.25)],false)).await;
    assert_eq!(restored["accepted"],true);assert_eq!(restored["zone"]["queue"].as_array().unwrap().len(),2);assert_eq!(restored["zone"]["position"],0.25);assert_eq!(restored["zone"]["track"]["id"],"local-shared");
}
