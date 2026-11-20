use super::*;
#[tokio::test]
async fn version_pages_have_no_candidate_truncation_and_page_other_recordings() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let(_,root)=f.call("PUT","/api/v1/library/tracks/local-shared/identity",Some(json!({"expected_revision":0,"assertions":[{"kind":"recording","namespace":"manual","external_id":"shared","title":"Shared","role":"recording"},{"kind":"work","namespace":"manual","external_id":"work","title":"Work","role":"work"}]}))).await;
    let rec = root["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "recording" && v["basis"] == "manual")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let work = root["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == "work")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.s.db.run(move|c|{let tx=c.transaction()?;for i in 0..11030{let id=format!("page-{i:05}");tx.execute("INSERT INTO tracks(id,title,artist,album,album_artist,duration,source,format,recording_id,created_at)VALUES(?1,?1,'QA','QA','QA',120,'fixture','flac',?1,0)",[&id])?;tx.execute("INSERT INTO music_assertions(track_id,entity_id,role,basis)VALUES(?1,?2,'recording','manual')",rusqlite::params![id,rec])?;}
 for i in 0..35{let id=format!("other-{i:03}");tx.execute("INSERT INTO tracks(id,title,artist,album,album_artist,duration,source,format,recording_id,created_at)VALUES(?1,?1,'QA','QA','QA',120,'fixture','flac',?1,0)",[&id])?;tx.execute("INSERT INTO music_assertions(track_id,entity_id,role,basis)VALUES(?1,?2,'work','manual')",rusqlite::params![id,work])?;}tx.commit()?;Ok(())}).await.unwrap();
    let (status, v) = f
        .call(
            "GET",
            "/api/v1/library/tracks/local-shared/versions?offset=11000&other_offset=20",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 11031);
    assert_eq!(v["candidate_scan_limited"], false);
    assert_eq!(v["items"].as_array().unwrap().len(), 20);
    assert_eq!(v["other_page"]["total"], 35);
    assert_eq!(v["other_recordings"].as_array().unwrap().len(), 15);
    let (_, v) = f
        .call(
            "GET",
            "/api/v1/library/tracks/local-shared/versions?q=page-11029",
            None,
        )
        .await;
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["track"]["id"], "page-11029");
}
#[tokio::test]
async fn catalog_alias_preview_confirm_and_unlink_preserve_tracks_and_manual_origins() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let base = crate::library::get_track(&f.s, "qq-only".into())
        .await
        .unwrap();
    f.s.db
        .run(move |c| {
            for (source, artist) in [("first", "One Artist"), ("second", "Other Alias")] {
                let mut t = base.clone();
                t.id = format!("{source}:song");
                t.source = source.into();
                t.recording_id = t.id.clone();
                t.artist = artist.into();
                t.album_artist = artist.into();
                t.provider_data = json!({"artist_credits":[{"id":"person","name":artist}]});
                crate::library::upsert(c, &t, None, None, 0, 0, None, None)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let (a, b) =
        f.s.db
            .read(|c| {
                Ok((
                    c.query_row(
                        "SELECT artist_id FROM track_artists WHERE track_id='first:song'",
                        [],
                        |r| r.get::<_, String>(0),
                    )?,
                    c.query_row(
                        "SELECT artist_id FROM track_artists WHERE track_id='second:song'",
                        [],
                        |r| r.get::<_, String>(0),
                    )?,
                ))
            })
            .await
            .unwrap();
    let path = format!("/api/v1/library/catalog/artist/{a}/links");
    let (_, p) = f
        .call(
            "PUT",
            &path,
            Some(json!({"other_id":b,"linked":true,"expected_revision":0,"preview_only":true})),
        )
        .await;
    assert_eq!(p["preview_only"], true);
    let (_, v) = f.call("GET", &path, None).await;
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    let (status, p) = f
        .call(
            "PUT",
            &path,
            Some(json!({"other_id":b,"linked":true,"expected_revision":0})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{p}");
    let (_, v) = f
        .call(
            "GET",
            &format!("/api/v1/library/catalog/artist/{a}?remote=false"),
            None,
        )
        .await;
    assert_eq!(v["local"]["total"], 2, "{v}");
    let (status, v) = f
        .call(
            "GET",
            &format!("/api/v1/library/catalog/artist/{b}?remote=false"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["local"]["total"], 2);
    assert_eq!(
        f.call(
            "PUT",
            &path,
            Some(json!({"other_id":b,"linked":false,"expected_revision":0}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        f.call(
            "PUT",
            &path,
            Some(json!({"other_id":b,"linked":false,"expected_revision":p["revision"]}))
        )
        .await
        .0,
        StatusCode::OK
    );
    crate::catalog_cache::save(&f.s,"artist",&a,json!({"description":"Automatic","_provenance":{"description":"first"},"_alternatives":{"description":[{"source":"first","value":"Automatic"},{"source":"second","value":"Other"}]}})).await.unwrap();
    let (status, _) = f
        .call(
            "PATCH",
            &format!("/api/v1/library/catalog/artist/{a}/profile"),
            Some(json!({"expected_revision":0,"fields":{"description":"Human"}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let profile = crate::catalog_cache::profile(&f.s, "artist", &a)
        .await
        .unwrap();
    assert_eq!(profile["provenance"]["description"], "manual");
    assert_eq!(
        profile["field_sources"]["description"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(profile["profile"].get("_provenance").is_none());
}
#[tokio::test]
async fn playback_policy_is_opt_in_exact_and_persists_without_rewriting_queue() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    for id in ["local-shared", "qq-shared"] {
        assert_eq!(f.call("PUT",&format!("/api/v1/library/tracks/{id}/identity"),Some(json!({"expected_revision":0,"assertions":[{"kind":"recording","namespace":"manual","external_id":"same","title":"Same","role":"recording"}]}))).await.0,StatusCode::OK);
    }
    let path = "/api/v1/library/tracks/local-shared/versions/policy";
    let (_, v) = f.call("GET", path, None).await;
    assert_eq!(v["policy"]["automatic_fallback"], false);
    assert_eq!(v["policy"]["apply_to_new_playback"], false);
    let user = f.call("GET", "/api/v1/auth/me", None).await.1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = crate::library::get_track(&f.s, "local-shared".into())
        .await
        .unwrap();
    assert_eq!(
        crate::music_playback_policy::select(&f.s, &user, original.clone())
            .await
            .unwrap()
            .0
            .id,
        original.id
    );
    let policy = json!({"preferred_track_id":"qq-shared","apply_to_new_playback":true,"automatic_fallback":false,"same_release":true,"lossless_only":true});
    let (status, value) = f
        .call(
            "PUT",
            path,
            Some(json!({"expected_revision":0,"policy":policy})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(
        f.call(
            "PUT",
            path,
            Some(json!({"expected_revision":0,"policy":policy}))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (selected, used) = crate::music_playback_policy::select(&f.s, &user, original.clone())
        .await
        .unwrap();
    assert!(used);
    assert_eq!(selected.id, "qq-shared");
    assert_eq!(
        selected.provider_data["playback_selection"]["reason"],
        "preferred"
    );
    assert_eq!(
        crate::library::get_track(&f.s, original.id)
            .await
            .unwrap()
            .provider_data["playback_selection"],
        Value::Null
    );
    let reopened = AppState::new(f.s.db.clone(), f.s.config.clone(), String::new())
        .await
        .unwrap();
    let selected = crate::music_playback_policy::select(
        &reopened,
        &user,
        crate::library::get_track(&reopened, "local-shared".into())
            .await
            .unwrap(),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(selected.id, "qq-shared");
    let original = crate::library::get_track(&f.s, "local-shared".into())
        .await
        .unwrap();
    f.s.db
        .run(|c| {
            c.execute("UPDATE tracks SET available=0 WHERE id='qq-shared'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        crate::music_playback_policy::select(&f.s, &user, original.clone())
            .await
            .is_err()
    );
    f.s.db.run(|c|{c.execute("UPDATE tracks SET available=1 WHERE id='qq-shared'",[])?;c.execute("DELETE FROM music_assertions WHERE track_id='qq-shared' AND basis='manual' AND role='recording'",[])?;Ok(())}).await.unwrap();
    let error = crate::music_playback_policy::select(&f.s, &user, original)
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(error.1, "version_preference_invalid");
}

#[tokio::test]
async fn fallback_is_same_recording_measured_lossless_and_rejects_missing_or_lower_quality() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    for id in ["local-shared", "qq-shared"] {
        assert_eq!(f.call("PUT",&format!("/api/v1/library/tracks/{id}/identity"),Some(json!({"expected_revision":0,"assertions":[{"kind":"recording","namespace":"manual","external_id":"safe","title":"Safe","role":"recording"}]}))).await.0,StatusCode::OK);
    }
    f.s.db.run(|c| {c.execute("UPDATE tracks SET sample_rate=48000,bit_depth=16,channels=1 WHERE id IN('qq-shared','local-shared')",[])?;Ok(())}).await.unwrap();
    let path = "/api/v1/library/tracks/qq-shared/versions/policy";
    let (_,p)=f.call("PUT",path,Some(json!({"expected_revision":0,"policy":{"automatic_fallback":true,"same_release":false}}))).await;
    assert_eq!(p["revision"], 1);
    let user = f.call("GET", "/api/v1/auth/me", None).await.1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut original = crate::library::get_track(&f.s, "qq-shared".into())
        .await
        .unwrap();
    let selected = crate::music_playback_policy::select(&f.s, &user, original.clone())
        .await
        .unwrap()
        .0;
    assert_eq!(selected.id, "local-shared");
    assert_eq!(
        selected.provider_data["playback_selection"]["reason"],
        "verified_fallback"
    );
    original.sample_rate = 96000;
    original.bit_depth = 24;
    assert!(
        crate::music_playback_policy::select(&f.s, &user, original.clone())
            .await
            .is_err(),
        "do not downgrade high-resolution original"
    );
    original.sample_rate = 48000;
    original.bit_depth = 16;
    f.s.db
        .run(|c| {
            c.execute(
                "UPDATE tracks SET path='fixture-missing-file' WHERE id='local-shared'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        crate::music_playback_policy::select(&f.s, &user, original.clone())
            .await
            .is_err(),
        "do not select a missing cache file"
    );
    // Same title or stale recording_id cannot substitute for a revoked explicit assertion.
    f.s.db
        .run(|c| {
            c.execute(
                "DELETE FROM music_assertions WHERE track_id='local-shared' AND basis='manual'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(crate::music_playback_policy::select(&f.s, &user, original)
        .await
        .is_err());
}
#[tokio::test]
async fn exact_play_bypasses_preference_and_preserves_existing_queue_ids() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    for id in ["local-shared", "qq-shared"] {
        f.call("PUT",&format!("/api/v1/library/tracks/{id}/identity"),Some(json!({"expected_revision":0,"assertions":[{"kind":"recording","namespace":"manual","external_id":"exact","title":"Exact","role":"recording"}]}))).await;
    }
    assert_eq!(
        f.call(
            "POST",
            "/api/v1/endpoints/browser",
            Some(json!({"id":"browser-fixture","name":"Fixture"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, z) = f
        .call(
            "PUT",
            "/api/v1/zones/zone-local/queue",
            Some(json!({"track_ids":["local-shared","qq-only"]})),
        )
        .await;
    let before = z["queue"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].clone())
        .collect::<Vec<_>>();
    f.call("PUT","/api/v1/library/tracks/qq-shared/versions/policy",Some(json!({"expected_revision":0,"policy":{"preferred_track_id":"local-shared","apply_to_new_playback":true}}))).await;
    let (status, z) = f
        .call(
            "POST",
            "/api/v1/zones/zone-local/play",
            Some(json!({"track_id":"qq-shared","exact_version":true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{z}");
    assert_eq!(z["track"]["id"], "qq-shared");
    let (status, z) = f
        .call(
            "POST",
            "/api/v1/zones/zone-local/play",
            Some(json!({"track_id":"qq-shared"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{z}");
    assert_eq!(z["track"]["id"], "local-shared");
    assert_eq!(
        z["track"]["provider_data"]["playback_selection"]["reason"],
        "preferred"
    );
    let after = z["queue"].as_array().unwrap();
    assert_eq!(
        after
            .iter()
            .take(2)
            .map(|t| t["id"].clone())
            .collect::<Vec<_>>(),
        before
    );
}
#[tokio::test]
async fn artwork_history_is_bounded_private_and_persistent() {
    let f = Fixture::new().await;
    let key = "a".repeat(64);
    let err = crate::error::ApiError(
        StatusCode::BAD_GATEWAY,
        "artwork_rate_limited".into(),
        "https://private.invalid?token=do-not-save".into(),
    );
    for _ in 0..515 {
        crate::artwork_diagnostics::record(&f.s, &key, "upstream", &err, std::time::Instant::now())
            .await;
    }
    let (status, v) = f.call("GET", "/api/v1/artwork-cache/events", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["items"].as_array().unwrap().len(), 50);
    assert!(!v.to_string().contains("do-not-save"));
    assert_eq!(v["items"][0]["stage"], "upstream");
    assert_eq!(v["items"][0]["status"], 502);
    assert_eq!(
        f.s.db
            .read(
                |c| Ok(c.query_row("SELECT COUNT(*) FROM artwork_events", [], |r| r
                    .get::<_, i64>(0))?)
            )
            .await
            .unwrap(),
        512
    );
    assert_eq!(
        f.call("GET", "/api/v1/artwork-cache/events?source=invalid", None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let reopened = crate::db::Db::open(&f.s.config.data.join("db.sqlite")).unwrap();
    assert_eq!(
        reopened
            .read(
                |c| Ok(c.query_row("SELECT COUNT(*) FROM artwork_events", [], |r| r
                    .get::<_, i64>(0))?)
            )
            .await
            .unwrap(),
        512
    );
}
#[tokio::test]
async fn unlink_restores_original_metadata_and_rejects_late_aggregate_write() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let base = crate::library::get_track(&f.s, "qq-only".into())
        .await
        .unwrap();
    f.s.db
        .run(move |c| {
            for source in ["one", "two"] {
                let mut t = base.clone();
                t.id = format!("{source}:song");
                t.source = source.into();
                t.provider_data = json!({"artist_credits":[{"id":"person","name":"Exact Person"}]});
                crate::library::upsert(c, &t, None, None, 0, 0, None, None)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let (a, b) =
        f.s.db
            .read(|c| {
                Ok((
                    c.query_row(
                        "SELECT artist_id FROM track_artists WHERE track_id='one:song'",
                        [],
                        |r| r.get::<_, String>(0),
                    )?,
                    c.query_row(
                        "SELECT artist_id FROM track_artists WHERE track_id='two:song'",
                        [],
                        |r| r.get::<_, String>(0),
                    )?,
                ))
            })
            .await
            .unwrap();
    crate::catalog_cache::save(
        &f.s,
        "artist",
        &a,
        json!({"description":"original","image_url":"https://artwork.example/one.jpg","_provenance":{"description":"one"}}),
    )
    .await
    .unwrap();
    let path = format!("/api/v1/library/catalog/artist/{a}/links");
    let (status, p) = f
        .call(
            "PUT",
            &path,
            Some(json!({"other_id":b,"linked":true,"expected_revision":0})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{p}");
    crate::catalog_cache::save(
        &f.s,
        "artist",
        &a,
        json!({"description":"merged","image_url":"https://artwork.example/two.jpg","_alias_members":[a,b],"_provenance":{"description":"two"}}),
    )
    .await
    .unwrap();
    assert_eq!(
        crate::catalog_cache::profile(&f.s, "artist", &a)
            .await
            .unwrap()["profile"]["description"],
        "merged"
    );
    assert_eq!(
        f.call(
            "PUT",
            &path,
            Some(json!({"other_id":b,"linked":false,"expected_revision":p["revision"]}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let saved = crate::catalog_cache::profile(&f.s, "artist", &a)
        .await
        .unwrap();
    assert_eq!(saved["profile"]["description"], "original");
    let expected = crate::auth::hash("https://artwork.example/one.jpg");
    let binding =
        f.s.db
            .read({
                let a = a.clone();
                move |c| {
                    Ok(c.query_row(
                    "SELECT source_id FROM artwork_entities WHERE kind='artist' AND entity_id=?1",
                    [a],
                    |r| r.get::<_, String>(0),
                )?)
                }
            })
            .await
            .unwrap();
    assert_eq!(binding, expected);
    let error = crate::catalog_cache::save(
        &f.s,
        "artist",
        &a,
        json!({"description":"late merged","_alias_members":[a,b]}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::CONFLICT);
    assert_eq!(
        crate::catalog_cache::profile(&f.s, "artist", &a)
            .await
            .unwrap()["profile"]["description"],
        "original"
    );
}
#[tokio::test]
async fn expired_artist_and_release_images_refresh_only_the_bound_provider_identity() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let mut t = crate::library::get_track(&f.s, "qq-only".into())
        .await
        .unwrap();
    t.id = "example:Anchor001".into();
    t.recording_id = t.id.clone();
    t.source = "example".into();
    t.provider_data = json!({"album_mid":"Album001","artist_mids":["Artist001"],"artist_credits":[{"id":"Artist001","name":t.artist}]});
    f.s.db
        .run({
            let t = t.clone();
            move |c| crate::library::upsert(c, &t, None, None, 0, 0, None, None)
        })
        .await
        .unwrap();
    let artist =
        f.s.db
            .read(|c| {
                Ok(c.query_row(
                    "SELECT artist_id FROM track_artists WHERE track_id='example:Anchor001'",
                    [],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .await
            .unwrap();
    let dir = f.s.config.plugin_dir.join("example");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),json!({"id":"example","name":"fixture","version":"1","capabilities":["metadata","related_catalog_v1"]}).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);p=r['params'];v=json.loads(pathlib.Path(__file__).with_name('response.json').read_text());k=p['kind'];i='Artist001' if k=='artist' else 'Album001';v.update(kind=k,collection={'id':i,'name':'Exact'},profile={'id':i,'source':'example','image_url':'https://artwork.example/new-'+k+'.jpg'})\n print(json.dumps({'id':r['id'],'result':v}),flush=True)\n").unwrap();
    std::fs::write(dir.join("response.json"),json!({"matched":true,"source":"example","anchor":t,"artists":[{"mid":"Artist001","name":t.artist}],"items":[t],"page":1,"total":1,"next_page":null}).to_string()).unwrap();
    for (kind, entity) in [
        ("artist", artist),
        ("album", "release:example:Album001".into()),
    ] {
        let k = kind.to_owned();
        let e = entity.clone();
        let id =
            f.s.db
                .run(move |c| {
                    let handle = crate::media_assets::register(
                        c,
                        &format!("https://artwork.example/old-{k}.jpg"),
                    )?;
                    crate::artwork_recovery::bind(c, &k, &e, &handle)?;
                    Ok(handle.rsplit('/').next().unwrap().to_owned())
                })
                .await
                .unwrap();
        let next = crate::catalog_artwork::refresh(&f.s, &id).await.unwrap();
        assert_eq!(
            next,
            Some(format!("https://artwork.example/new-{kind}.jpg"))
        );
        f.s.db.run({let kind=kind.to_owned();let e=entity.clone();move|c|{c.execute("INSERT INTO catalog_profiles(kind,entity_id,manual)VALUES(?1,?2,?3) ON CONFLICT(kind,entity_id)DO UPDATE SET manual=excluded.manual",rusqlite::params![kind,e,json!({"image_url":""}).to_string()])?;Ok(())}}).await.unwrap();
        assert!(
            crate::catalog_artwork::refresh(&f.s, &id)
                .await
                .unwrap()
                .is_none(),
            "a deliberate manual clear must never be replaced"
        );
    }
    f.s.plugin.stop("example").await;
}
#[tokio::test]
async fn slow_version_preflight_does_not_hold_zone_lock_and_cannot_undo_a_stop() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let mut cloud = crate::library::get_track(&f.s, "qq-shared".into())
        .await
        .unwrap();
    cloud.id = "slow:anchor".into();
    cloud.source = "slow".into();
    cloud.recording_id = cloud.id.clone();
    cloud.sample_rate = 48000;
    cloud.bit_depth = 16;
    cloud.channels = 1;
    f.s.db
        .run(move |c| crate::library::upsert(c, &cloud, None, None, 0, 0, None, None))
        .await
        .unwrap();
    for id in ["slow:anchor", "local-shared"] {
        f.call("PUT",&format!("/api/v1/library/tracks/{id}/identity"),Some(json!({"expected_revision":0,"assertions":[{"kind":"recording","namespace":"manual","external_id":"slow","title":"Slow","role":"recording"}]}))).await;
    }
    let dir = f.s.config.plugin_dir.join("slow");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        json!({"id":"slow","name":"fixture","version":"1","capabilities":["stream"]}).to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys,pathlib,time\nfor line in sys.stdin:\n r=json.loads(line);pathlib.Path(__file__).with_name('started').write_text('1');time.sleep(1);print(json.dumps({'id':r['id'],'result':{'url':'http://127.0.0.1:1/unavailable'}}),flush=True)\n").unwrap();
    f.call(
        "POST",
        "/api/v1/endpoints/browser",
        Some(json!({"id":"browser-slow-fixture","name":"Fixture"})),
    )
    .await;
    f.call(
        "PUT",
        "/api/v1/zones/zone-local/queue",
        Some(json!({"track_ids":["local-shared"]})),
    )
    .await;
    f.call(
        "POST",
        "/api/v1/zones/zone-local/play",
        Some(json!({"track_id":"local-shared","exact_version":true})),
    )
    .await;
    f.call("PUT","/api/v1/library/tracks/slow:anchor/versions/policy",Some(json!({"expected_revision":0,"policy":{"automatic_fallback":true,"same_release":false}}))).await;
    let app = f.app.clone();
    let token = f.token.clone();
    let task = tokio::spawn(async move {
        call(
            &app,
            "POST",
            "/api/v1/zones/zone-local/play",
            Some(&token),
            Some(json!({"track_id":"slow:anchor"})),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        while !dir.join("started").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        f.call("POST", "/api/v1/zones/zone-local/stop", Some(json!({}))),
    )
    .await
    .expect("source preflight blocked the shared playback lock");
    assert_eq!(result.0, StatusCode::OK);
    let (status, _) = task.await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, zone) = f.call("GET", "/api/v1/zones/zone-local", None).await;
    assert_ne!(zone["status"], "playing");
    f.s.plugin.stop("slow").await;
}
#[tokio::test]
async fn empty_original_metadata_does_not_retain_an_unlinked_platform_profile() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let ids =
        f.s.db
            .read(|c| {
                Ok((
                    c.query_row(
                        "SELECT artist_id FROM track_artists WHERE track_id='local-shared'",
                        [],
                        |r| r.get::<_, String>(0),
                    )?,
                    c.query_row(
                        "SELECT artist_id FROM track_artists WHERE track_id='qq-only'",
                        [],
                        |r| r.get::<_, String>(0),
                    )?,
                ))
            })
            .await
            .unwrap();
    let path = format!("/api/v1/library/catalog/artist/{}/links", ids.0);
    let (status, revision) = f
        .call(
            "PUT",
            &path,
            Some(json!({"other_id":ids.1,"linked":true,"expected_revision":0})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    crate::catalog_cache::save(&f.s,"artist",&ids.0,json!({"description":"only from the linked platform","image_url":"https://artwork.example/temporary-alias.jpg","_alias_members":[ids.0,ids.1]})).await.unwrap();
    assert_eq!(
        f.call(
            "PUT",
            &path,
            Some(json!({"other_id":ids.1,"linked":false,"expected_revision":revision["revision"]}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let saved = crate::catalog_cache::profile(&f.s, "artist", &ids.0)
        .await
        .unwrap();
    assert!(saved["profile"].get("description").is_none());
    assert_eq!(
        f.s.db
            .read(move |c| Ok(c.query_row(
                "SELECT COUNT(*) FROM artwork_entities WHERE kind='artist' AND entity_id=?1",
                [ids.0],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn failed_playback_preference_can_fall_back_to_the_requested_measured_version() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    f.s.db.run(|c|{c.execute("UPDATE tracks SET format='wav',sample_rate=48000,bit_depth=16,channels=1 WHERE id='local-shared'",[])?;Ok(())}).await.unwrap();
    for id in ["local-shared", "qq-shared"] {
        f.call("PUT",&format!("/api/v1/library/tracks/{id}/identity"),Some(json!({"expected_revision":0,"assertions":[{"kind":"recording","namespace":"manual","external_id":"return","title":"Return","role":"recording"}]}))).await;
    }
    f.call("PUT","/api/v1/library/tracks/local-shared/versions/policy",Some(json!({"expected_revision":0,"policy":{"preferred_track_id":"qq-shared","apply_to_new_playback":true,"automatic_fallback":true,"same_release":false}}))).await;
    let user = f.call("GET", "/api/v1/auth/me", None).await.1["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let original = crate::library::get_track(&f.s, "local-shared".into())
        .await
        .unwrap();
    let selected = crate::music_playback_policy::select(&f.s, &user, original)
        .await
        .unwrap()
        .0;
    assert_eq!(selected.id, "local-shared");
    assert_eq!(
        selected.provider_data["playback_selection"]["reason"],
        "verified_fallback"
    );
}
