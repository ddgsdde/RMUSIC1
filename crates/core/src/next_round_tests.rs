use super::*;

#[tokio::test]
async fn artwork_budget_check_and_refresh_are_protected_and_restart_safe() {
    let f = Fixture::new().await;
    let bytes=data_encoding::BASE64.decode(b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aXioAAAAASUVORK5CYII=").unwrap();
    let cover = crate::artwork_files::save(&f.s, &bytes).await.unwrap();
    let file = f.s.config.data.join("covers").join(&cover);
    let key =
        f.s.db
            .run(move |c| {
                let url = crate::media_assets::register(c, "https://example.test/picture.png")?;
                let id = url.rsplit('/').next().unwrap().to_owned();
                c.execute(
                    "UPDATE artwork_sources SET cover_id=?1 WHERE id=?2",
                    rusqlite::params![cover, id],
                )?;
                Ok(id)
            })
            .await
            .unwrap();
    assert_eq!(
        f.call(
            "PATCH",
            "/api/v1/settings",
            Some(json!({"artwork_limit_bytes":1}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(file.exists());
    let budget = bytes.len() as u64;
    assert_eq!(
        f.call(
            "PATCH",
            "/api/v1/settings",
            Some(json!({"artwork_limit_bytes":budget}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let other = b"\xff\xd8\xff\xe0\xff\xd9";
    assert_eq!(
        crate::artwork_files::save(&f.s, other)
            .await
            .err()
            .unwrap()
            .1,
        "artwork_budget_exceeded"
    );
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    let (_, status) = f.call("GET", "/api/v1/artwork-cache?limit=1", None).await;
    assert_eq!(status["file_count"], 1);
    assert_eq!(status["limit_bytes"], budget);
    assert_eq!(status["items"][0]["id"], key);
    std::fs::write(&file, b"damaged-fixture").unwrap();
    let (_, job) = f
        .call("POST", "/api/v1/artwork-cache/check", Some(json!({})))
        .await;
    let id = job["id"].as_str().unwrap().to_owned();
    let j = id.clone();
    f.s.db
        .run(move |c| {
            c.execute("UPDATE jobs SET status='running' WHERE id=?1", [j])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        crate::artwork_maintenance::run(&f.s, &id, "artwork_check", &json!({}))
            .await
            .unwrap()
    );
    let (_, status) = f
        .call("GET", "/api/v1/artwork-cache?failed=true", None)
        .await;
    assert_eq!(status["total"], 1);
    assert_eq!(status["items"][0]["state"], "corrupt");
    assert_eq!(
        std::fs::read(file).unwrap(),
        b"damaged-fixture",
        "inspection must not fetch or delete"
    );
    let restarted = AppState::new(f.s.db.clone(), f.s.config.clone(), String::new())
        .await
        .unwrap();
    assert_eq!(
        restarted.setting("artwork_limit_bytes", json!(0)).await,
        budget
    );
}
#[tokio::test]
async fn identity_preview_restore_and_quality_do_not_rewrite_files_or_playlists() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let path = "/api/v1/library/tracks/local-shared/identity";
    let assertion = json!({"kind":"recording","namespace":"manual","external_id":"confirmed","title":"录音","role":"recording"});
    let edit = json!({"expected_revision":0,"assertions":[assertion]});
    let (status, preview) = f
        .call("POST", &(path.to_owned() + "/preview"), Some(edit.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["preview_only"], true);
    assert_eq!(preview["resolution"]["recording"]["resolved"], true);
    let (_, old) = f.call("GET", path, None).await;
    assert_eq!(old["revision"], 0);
    assert_eq!(f.call("PUT", path, Some(edit)).await.0, StatusCode::OK);
    assert_eq!(
        f.call(
            "PUT",
            path,
            Some(json!({"expected_revision":1,"assertions":[]}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, value) = f
        .call(
            "POST",
            &(path.to_owned() + "/restore"),
            Some(json!({"expected_revision":2,"revision":1})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["revision"], 3);
    assert!(value["entities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["basis"] == "manual"));
    assert_eq!(
        f.call(
            "POST",
            &(path.to_owned() + "/restore"),
            Some(json!({"expected_revision":2,"revision":1}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, versions) = f
        .call("GET", "/api/v1/library/tracks/local-shared/versions", None)
        .await;
    assert_eq!(
        versions["items"][0]["quality_evidence"]["output_requires_live_report"],
        true
    );
}
#[tokio::test]
async fn catalog_scrape_defaults_off_and_unchanged_profiles_keep_their_update_time() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let (_, settings) = f.call("GET", "/api/v1/settings", None).await;
    assert_eq!(settings["catalog_scrape_enabled"], false);
    assert_eq!(settings["catalog_scrape_concurrency"], 1);
    crate::catalog_scrape::schedule(&f.s).await.unwrap();
    assert_eq!(
        f.s.db
            .read(|c| Ok(c.query_row(
                "SELECT COUNT(*) FROM jobs WHERE kind='catalog_scrape'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        0
    );
    crate::catalog_cache::save(
        &f.s,
        "artist",
        "sample",
        json!({"name":"Sample","description":"Original"}),
    )
    .await
    .unwrap();
    f.s.db
        .run(|c| {
            c.execute(
                "UPDATE catalog_profiles SET updated_at=123 WHERE entity_id='sample'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    crate::catalog_cache::save(
        &f.s,
        "artist",
        "sample",
        json!({"name":"Sample","description":"Original"}),
    )
    .await
    .unwrap();
    assert_eq!(
        crate::catalog_cache::profile(&f.s, "artist", "sample")
            .await
            .unwrap()["updated_at"],
        123
    );
    assert_eq!(
        f.call(
            "PATCH",
            "/api/v1/settings",
            Some(json!({"catalog_scrape_enabled":true,"catalog_scrape_concurrency":2}))
        )
        .await
        .0,
        StatusCode::OK
    );
    crate::catalog_scrape::schedule(&f.s).await.unwrap();
    let count =
        f.s.db
            .read(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM jobs WHERE kind='catalog_scrape'",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn catalog_scrape_does_not_recursively_crawl_browsed_platform_entries() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let before = crate::catalog_scrape::due(&f.s, 1000).await.unwrap();
    f.s.db.run(|c|{let mut track=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;track.id="qqmusic:HiddenDirectory".into();track.title="Hidden".into();track.artist="Uncollected Artist".into();track.album_artist=track.artist.clone();track.album="Uncollected Album".into();track.provider_data=json!({"catalog_browse_only":true,"artist_credits":[{"id":"hidden-artist","name":"Uncollected Artist"}],"album_mid":"hidden-album"});crate::library::upsert(c,&track,None,None,0,0,None,None)?;Ok(())}).await.unwrap();
    let after = crate::catalog_scrape::due(&f.s, 1000).await.unwrap();
    assert_eq!(
        before, after,
        "background scraping must stay within the collected library"
    );
}

#[tokio::test]
async fn persistent_catalog_pages_are_checked_by_age_without_touching_timestamps() {
    let f = Fixture::new().await;
    let key = "catalog:example:album:Confirmed".to_string();
    crate::catalog_cache::save_page(&f.s, key.clone(), 1, &json!({"items":[]}), false)
        .await
        .unwrap();
    assert!(crate::catalog_cache::page_fresh(&f.s, key.clone(), 1)
        .await
        .unwrap());
    f.s.db
        .run(|c| {
            c.execute("UPDATE catalog_pages SET updated_at=123", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(!crate::catalog_cache::page_fresh(&f.s, key.clone(), 1)
        .await
        .unwrap());
    assert!(
        crate::catalog_cache::load_page(&f.s, key.clone(), 1)
            .await
            .unwrap()
            .is_some(),
        "stale data remains durable for offline fallback"
    );
    assert_eq!(
        f.s.db
            .read(|c| Ok(
                c.query_row("SELECT updated_at FROM catalog_pages", [], |r| r
                    .get::<_, i64>(0))?
            ))
            .await
            .unwrap(),
        123
    );
}
