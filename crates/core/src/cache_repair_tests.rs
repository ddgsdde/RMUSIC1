use super::*;
use axum::body::Bytes;
use futures_util::{stream, StreamExt};
use rusqlite::params;

async fn missing_entry(f: &Fixture, cloud: &models::Track, id: &str, present: bool) {
    let path = f.s.cache_dir().await.join(id);
    if present {
        std::fs::write(&path, vec![9; 16384]).unwrap();
    }
    let mut local = cloud.clone();
    local.id = format!("local-{id}");
    local.source = "local".into();
    local.available = true;
    local.provider_data["cached_from"] =
        json!({"track_id":cloud.id,"source":cloud.source,"cache_id":id});
    let id = id.to_owned();
    let track = cloud.id.clone();
    let path = path.to_string_lossy().into_owned();
    f.s.db.run(move|c|{
        library::upsert(c,&local,Some(&path),None,0,16384,None,None)?;
        c.execute("INSERT INTO cache_entries(id,track_id,path,format,total_size,size,complete,single_file,last_access) VALUES(?1,?2,?3,'flac',16384,16384,1,1,1)",params![id,track,path])?;
        c.execute("INSERT INTO cache_exports(cache_id,status,updated_at)VALUES(?1,'ready',1)",[id])?;
        Ok(())
    }).await.unwrap();
}
async fn claim(s: &AppState, id: &str) -> Value {
    let id = id.to_owned();
    s.db.run(move |c| {
        c.execute("UPDATE jobs SET status='running' WHERE id=?1", [&id])?;
        Ok(serde_json::from_str(&c.query_row(
            "SELECT payload FROM jobs WHERE id=?1",
            [id],
            |r| r.get::<_, String>(0),
        )?)?)
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn cache_disabled_passes_bytes_without_buffers_files_or_repair_requests() {
    let f = Fixture::new().await;
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"cache_enabled":false}))).await.0,StatusCode::BAD_REQUEST);
    // Internal emergency bypass remains byte preserving; it is no longer a public preference.
    f.s.cache_enabled.store(false,std::sync::atomic::Ordering::Release);
    let input = stream::iter([Ok(Bytes::from_static(b"original audio"))]).boxed();
    let mut read = media::caching_stream(
        f.s.clone(),
        "cloud".into(),
        "flac".into(),
        "entity".into(),
        14,
        0,
        input,
    )
    .await
    .unwrap();
    assert_eq!(read.next().await.unwrap().unwrap(), "original audio");
    assert!(read.next().await.is_none());
    assert!(f.s.cache_buffers.lock().await.is_empty());
    assert_eq!(f.s.resources.status(&f.s).await["used_bytes"], 0);
    let n =
        f.s.db
            .run(|c| {
                Ok(c.query_row(
                    "SELECT (SELECT COUNT(*) FROM cache_entries)+(SELECT COUNT(*) FROM jobs)",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await
            .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/library/cache-repair",
            Some(json!({"concurrency":2}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let p = json!({"track_id":"cloud","quality":"best"});
    let id = crate::background::enqueue(&f.s, "cache_fill", "legacy", p.clone(), 0)
        .await
        .unwrap();
    claim(&f.s, &id).await;
    assert!(!media::fill_cache(&f.s, &id, &p).await.unwrap());
    assert_eq!(
        f.s.db
            .run(move |c| Ok(c
                .query_row("SELECT status FROM jobs WHERE id=?1", [id], |r| r
                    .get::<_, String>(0))?))
            .await
            .unwrap(),
        "paused"
    );
    assert_eq!(std::fs::read_dir(f.s.cache_dir().await).unwrap().count(), 0);
}
#[tokio::test]
async fn missing_cache_reads_and_legacy_jobs_do_not_fetch_but_playback_fetches_exact_bytes() {
    let f = Fixture::new().await;
    let remote = super::playlist_sync::provider(&f).await;
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    let expected = Bytes::from(vec![9; 16384]);
    let data = expected.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = axum::Router::new().route(
        "/audio",
        axum::routing::get(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let data = data.clone();
            async move {
                axum::http::Response::builder()
                    .header("ETag", "\"lazy-entity\"")
                    .body(Body::from(data))
                    .unwrap()
            }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut v: Value = serde_json::from_slice(&std::fs::read(&remote).unwrap()).unwrap();
    v["source"] = json!({"url":format!("http://{address}/audio"),"format":"flac","cache_allowed":true,"expected_size":16384});
    std::fs::write(&remote, v.to_string()).unwrap();
    let cloud = crate::recommendations::select(&f.s, "example", None, vec![], 1, false)
        .await
        .unwrap()
        .items
        .remove(0);
    let mut stale_jobs = Vec::new();
    for origin in ["playback", "manual", "repair"] {
        let p = json!({"track_id":cloud.id,"quality":"best","origin":origin});
        let job = crate::background::enqueue(
            &f.s,
            "cache_fill",
            &format!("stale-{origin}"),
            p.clone(),
            0,
        )
        .await
        .unwrap();
        stale_jobs.push((job, p));
    }
    missing_entry(&f, &cloud, "gone", false).await;
    for _ in 0..2 {
        let t = library::get_track(&f.s, "local-gone".into()).await.unwrap();
        assert_eq!(t.id, "local-gone");
        assert_eq!(t.source, "example");
    }
    for (id, p) in stale_jobs {
        claim(&f.s, &id).await;
        assert!(
            !media::fill_cache(&f.s, &id, &p).await.unwrap(),
            "a request from before the loss must not authorize a new download"
        );
    }
    let payload = json!({"track_id":cloud.id,"quality":"best"});
    let job = crate::background::enqueue(&f.s, "cache_fill", "legacy", payload.clone(), 0)
        .await
        .unwrap();
    claim(&f.s, &job).await;
    assert!(!media::fill_cache(&f.s, &job, &payload).await.unwrap());
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    let fresh = crate::cache_policy::schedule(&f.s, &cloud.id, "best", "manual", None, 50)
        .await
        .unwrap();
    let p = claim(&f.s, &fresh).await;
    assert!(
        crate::cache_policy::guard_fill(&f.s, &fresh, &p)
            .await
            .unwrap(),
        "a new explicit request may authorize repair after validation"
    );
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/media/local-gone")
                .header("Authorization", format!("Bearer {}", f.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 32768).await.unwrap(),
        expected
    );
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    f.s.plugin.stop("example").await;
    server.abort();
}
#[tokio::test]
async fn cache_repair_scan_is_bounded_resumable_and_controls_child_concurrency() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let cloud = library::get_track(&f.s, "qq-only".into()).await.unwrap();
    for index in 0..101 {
        missing_entry(&f, &cloud, &format!("entry-{index:03}"), index == 100).await;
    }
    let (status, result) = f
        .call(
            "POST",
            "/api/v1/library/cache-repair",
            Some(json!({"concurrency":1})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let id = result["job_id"].as_str().unwrap().to_owned();
    let payload = claim(&f.s, &id).await;
    assert!(!crate::cache_repair::run(&f.s, &id, &payload).await.unwrap());
    let payload = claim(&f.s, &id).await;
    assert_eq!(payload["checked"], 100);
    assert_eq!(payload["scan_complete"], false);
    assert!(!crate::cache_repair::run(&f.s, &id, &payload).await.unwrap());
    let payload = claim(&f.s, &id).await;
    assert_eq!(payload["checked"], 101);
    assert_eq!(payload["scan_complete"], true);
    let parent = id.clone();
    let children=f.s.db.run(move|c|Ok(c.prepare("SELECT id,payload FROM jobs WHERE kind='cache_fill' AND json_extract(payload,'$.repair_job')=?1")?.query_map([parent],|r|Ok((r.get::<_,String>(0)?,serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap())))?.collect::<rusqlite::Result<Vec<_>>>()?)).await.unwrap();
    assert_eq!(
        children.len(),
        1,
        "same origin shares one bounded repair transfer"
    );
    assert!(crate::cache_repair::admitted(&f.s, &children[0].1)
        .await
        .unwrap());
    claim(&f.s, &children[0].0).await;
    assert!(!crate::cache_repair::admitted(&f.s, &children[0].1)
        .await
        .unwrap());
    assert_eq!(
        f.call(
            "PATCH",
            &format!("/api/v1/library/cache-repair/{id}"),
            Some(json!({"concurrency":3}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(crate::cache_repair::admitted(&f.s, &children[0].1)
        .await
        .unwrap());
    assert_eq!(
        f.call(
            "PATCH",
            &format!("/api/v1/jobs/{id}"),
            Some(json!({"status":"paused"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(!crate::cache_repair::admitted(&f.s, &children[0].1)
        .await
        .unwrap());
    let child = children[0].0.clone();
    assert_eq!(
        f.s.db
            .run(move |c| Ok(
                c.query_row("SELECT status FROM jobs WHERE id=?1", [child], |r| r
                    .get::<_, String>(0))?
            ))
            .await
            .unwrap(),
        "paused"
    );
    assert_eq!(
        f.call(
            "PATCH",
            &format!("/api/v1/jobs/{id}"),
            Some(json!({"status":"running"}))
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let restored = AppState::new(f.s.db.clone(), f.s.config.clone(), "bootstrap-test".into())
        .await
        .unwrap();
    let retained = claim(&restored, &id).await;
    assert_eq!(retained["checked"], 101);
    assert_eq!(retained["concurrency"], 3);
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/library/cache-repair",
            Some(json!({"concurrency":0}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn previously_marked_missing_exports_remain_repairable_without_a_local_alias() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let cloud = library::get_track(&f.s, "qq-only".into()).await.unwrap();
    missing_entry(&f, &cloud, "export-only", false).await;
    f.s.db
        .run(|c| {
            c.execute("DELETE FROM tracks WHERE id='local-export-only'", [])?;
            c.execute(
                "UPDATE cache_entries SET complete=0,size=0 WHERE id='export-only'",
                [],
            )?;
            c.execute(
                "UPDATE cache_exports SET status='missing' WHERE cache_id='export-only'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (_, result) = f
        .call(
            "POST",
            "/api/v1/library/cache-repair",
            Some(json!({"concurrency":1})),
        )
        .await;
    let id = result["job_id"].as_str().unwrap();
    let payload = claim(&f.s, id).await;
    assert!(!crate::cache_repair::run(&f.s, id, &payload).await.unwrap());
    let progress = claim(&f.s, id).await;
    assert_eq!(progress["checked"], 1);
    let parent = id.to_owned();
    let count = f.s.db.run(move |c| Ok(c.query_row("SELECT COUNT(*) FROM jobs WHERE kind='cache_fill' AND json_extract(payload,'$.repair_job')=?1", [parent], |r| r.get::<_,i64>(0))?)).await.unwrap();
    assert_eq!(count, 1);
}
