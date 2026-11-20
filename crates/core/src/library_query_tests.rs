use super::*;
#[tokio::test]
async fn library_query_filters_before_paging_and_keeps_unknown_release_last() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let today = chrono::Utc::now().date_naive().to_string();
    f.s.db.run(move|c|{
  c.execute("UPDATE tracks SET provider_data=json_object('release_date',?1),duration=150 WHERE id='qq-only'",[today])?;
  c.execute("UPDATE tracks SET provider_data=json_object('release_date','2000-01-01'),duration=90 WHERE id='qq-live'",[])?;
  c.execute("INSERT INTO track_stats(track_id,play_count)VALUES('qq-only',3)",[])?;Ok(())
 }).await.unwrap();
    let (status, v) = f
        .call(
            "GET",
            "/api/v1/library/tracks?sort=release_new&limit=1&min_rate=48000&min_depth=24",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["items"][0]["id"], "qq-only");
    assert_eq!(
        v["items"][0]["provider_data"]["catalog_display"]["is_new"],
        true
    );
    let (_, v) = f
        .call(
            "GET",
            "/api/v1/library/tracks?sort=release_old&limit=1",
            None,
        )
        .await;
    assert_eq!(v["items"][0]["id"], "qq-live");
    let (_, v) = f
        .call(
            "GET",
            "/api/v1/library/tracks?listening=played&max_seconds=200&released_after=2020-01-01",
            None,
        )
        .await;
    assert_eq!(v["total"], 1);
    assert_eq!(
        v["items"][0]["provider_data"]["catalog_display"]["play_count"],
        3
    );
    let (_, v) = f
        .call(
            "GET",
            "/api/v1/library/albums?sort=release_new&listening=played",
            None,
        )
        .await;
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["play_count"], 3);
    for q in [
        "sort=drop%20table",
        "min_rate=oops",
        "min_seconds=200&max_seconds=100",
        "released_after=2026-02-30",
    ] {
        assert_eq!(
            f.call("GET", &format!("/api/v1/library/tracks?{q}"), None)
                .await
                .0,
            StatusCode::BAD_REQUEST,
            "{q}"
        );
    }
}
#[tokio::test]
async fn library_query_playlist_membership_is_user_scoped() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let user = auth::authenticate(&f.s, &f.token).await.unwrap().id;
    f.s.db.run(move|c|{c.execute("INSERT INTO playlists(id,name,user_id,tracks)VALUES('mine','mine',?1,'[\"qq-only\"]')",[user])?;c.execute("INSERT INTO playlists(id,name,user_id,tracks)VALUES('foreign','foreign','other-user','[\"qq-live\"]')",[])?;Ok(())}).await.unwrap();
    let (status, v) = f
        .call("GET", "/api/v1/library/tracks?playlist=in", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["id"], "qq-only");
    let (_, v) = f
        .call(
            "GET",
            "/api/v1/library/tracks?playlist=out&listening=unplayed",
            None,
        )
        .await;
    assert!(v["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["id"] == "qq-live"));
}
#[tokio::test]
async fn source_cover_recovers_from_exact_cached_metadata_and_respects_manual_clear() {
    let f=Fixture::new().await;f.collection_tracks().await;
    f.s.db.run(|c|{
        let mut copy=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;
        copy.id="owned-artwork-copy".into();copy.source="local".into();copy.cover_url=None;
        copy.provider_data=json!({"cached_from":{"track_id":"qq-only","source":"qqmusic"},"provider_metadata":{"matched":true,"track":{"cover_url":"https://images.example/song.jpg"}}});
        library::upsert(c,&copy,None,None,0,0,None,None)?;
        c.execute("UPDATE tracks SET cover_url=NULL WHERE id='qq-only'",[])?;Ok(())
    }).await.unwrap();
    let recovered=library::get_track(&f.s,"qq-only".into()).await.unwrap();
    assert_eq!(recovered.cover_url,Some(format!("/api/v1/artwork/{}",auth::hash("https://images.example/song.jpg"))));
    f.s.db.run(|c|{c.execute("UPDATE tracks SET cover_url=NULL WHERE id='qq-only'",[])?;c.execute("INSERT INTO catalog_profiles(kind,entity_id,manual)VALUES('track','qq-only','{\"image_url\":\"\"}')",[])?;Ok(())}).await.unwrap();
    assert!(library::get_track(&f.s,"qq-only".into()).await.unwrap().cover_url.is_none());
    assert!(library::get_track(&f.s,"qq-live".into()).await.unwrap().cover_url.is_none());
}
#[tokio::test]
async fn metadata_cover_repair_and_upsert_preserve_confirmed_images_and_manual_clear() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    f.s.db.run(|c|{
  let mut t=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;
  t.provider_data=json!({"provider_metadata":{"matched":true,"track":{"cover_url":"https://images.example/verified.jpg"}}});
  library::upsert(c,&t,None,None,0,0,None,None)?;
  let cover:String=c.query_row("SELECT cover_url FROM tracks WHERE id='qq-only'",[],|r|r.get(0))?;assert!(cover.starts_with("/api/v1/artwork/"));
  c.execute("UPDATE tracks SET cover_url=NULL WHERE id='qq-only'",[])?;
  c.execute("DELETE FROM settings WHERE key='metadata_cover_repair_v2'",[])?;
  crate::media_assets::repair_metadata_covers(c)?;
  assert!(c.query_row("SELECT cover_url IS NOT NULL FROM tracks WHERE id='qq-only'",[],|r|r.get::<_,bool>(0))?);
  c.execute("INSERT INTO catalog_profiles(kind,entity_id,manual)VALUES('track','qq-only','{\"image_url\":\"\"}')",[])?;
  library::upsert(c,&t,None,None,0,0,None,None)?;
  assert!(c.query_row("SELECT cover_url IS NULL FROM tracks WHERE id='qq-only'",[],|r|r.get::<_,bool>(0))?);Ok(())
 }).await.unwrap();
}
#[tokio::test]
async fn album_sort_reads_all_pages_before_sorting_without_audio() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let mut anchor = library::get_track(&f.s, "qq-only".into()).await.unwrap();
    anchor.source = "example".into();
    anchor.id = "example:Anchor001".into();
    anchor.recording_id = anchor.id.clone();
    anchor.provider_data = json!({"album_mid":"Album001","artist_mids":["Artist001"]});
    f.s.db
        .run({
            let a = anchor.clone();
            move |c| {
                library::upsert(c, &a, None, None, 0, 0, None, None)?;
                Ok(())
            }
        })
        .await
        .unwrap();
    let dir = f.s.config.plugin_dir.join("example");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        json!({"id":"example","capabilities":["metadata","related_catalog_v1"]}).to_string(),
    )
    .unwrap();
    let mut last = anchor.clone();
    last.id = "example:Last001".into();
    last.title = "AAAA".into();
    last.recording_id = last.id.clone();
    let page1 = json!({"source":"example","matched":true,"kind":"album","anchor":anchor,"collection":{"id":"Album001"},"artists":[{"id":"Artist001"}],"items":[anchor],"page":1,"total":2,"next_page":2});
    let page2 = json!({"page":2,"items":[last],"next_page":null});
    std::fs::write(dir.join("response.json"), json!([page1, page2]).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);pages=json.loads(pathlib.Path(__file__).with_name('response.json').read_text());v=pages[0]|pages[r['params'].get('page',1)-1];print(json.dumps({'id':r['id'],'result':v}),flush=True)\n").unwrap();
    let (status, v) = f
        .call(
            "GET",
            "/api/v1/library/tracks/example:Anchor001/related?kind=album&sort=title",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["items"][0]["title"], "AAAA");
    assert_eq!(v["total"], 2);
    assert_eq!(
        f.s.db
            .read(
                |c| Ok(c.query_row("SELECT COUNT(*) FROM cache_entries", [], |r| r
                    .get::<_, i64>(0))?)
            )
            .await
            .unwrap(),
        0
    );
    f.s.plugin.stop("example").await;
}

#[tokio::test]
async fn missing_album_refresh_requires_exact_provider_identity_and_preserves_local_tags(){
 let f=Fixture::new().await;f.collection_tracks().await;
 f.s.db.run(|c|{c.execute("UPDATE tracks SET album='' WHERE id='qq-only'",[])?;Ok(())}).await.unwrap();
 crate::metadata::save_track(&f.s,"qq-only".into(),json!({"source":"qqmusic","matched":true,"track":{"id":"wrong-id","album":"Wrong"}})).await.unwrap();
 assert_eq!(library::get_track(&f.s,"qq-only".into()).await.unwrap().album,"");
 crate::metadata::save_track(&f.s,"qq-only".into(),json!({"source":"qqmusic","matched":true,"track":{"id":"qq-only","album":"Verified Album"}})).await.unwrap();
 assert_eq!(library::get_track(&f.s,"qq-only".into()).await.unwrap().album,"Verified Album");
 crate::metadata::save_track(&f.s,"local-unknown".into(),json!({"source":"qqmusic","matched":true,"track":{"id":"local-unknown","album":"Cloud Album"}})).await.unwrap();
 assert_eq!(library::get_track(&f.s,"local-unknown".into()).await.unwrap().album,"   ");
}

#[tokio::test]
async fn a_real_release_named_none_is_not_misclassified_as_missing(){
 let f=Fixture::new().await;f.collection_tracks().await;
 f.s.db.run(|c|{let mut t=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;t.album="None".into();t.provider_data=json!({"album_mid":"VerifiedRelease01"});library::upsert(c,&t,None,None,0,0,None,None)?;Ok(())}).await.unwrap();
 let(status,v)=f.call("GET","/api/v1/library/albums?source=qqmusic",None).await;assert_eq!(status,StatusCode::OK,"{v}");assert!(v["items"].as_array().unwrap().iter().any(|v|v["title"]=="None"));
 let(_,v)=f.call("GET","/api/v1/library/tracks?album_status=unassigned",None).await;assert!(!v["items"].as_array().unwrap().iter().any(|v|v["id"]=="qq-only"));
}

#[tokio::test]
async fn compact_library_pages_keep_navigation_without_large_provider_payloads() {
 let f=Fixture::new().await;f.collection_tracks().await;
 f.s.db.run(|c|{c.execute("UPDATE tracks SET provider_data=json_object('large_detail',?1,'artist_mids',json('[\"artist\"]'),'album_mid','album') WHERE id='qq-only'",["x".repeat(100000)])?;Ok(())}).await.unwrap();
 let (status,full)=f.call("GET","/api/v1/library/tracks?source=qqmusic",None).await;assert_eq!(status,StatusCode::OK);
 let (status,small)=f.call("GET","/api/v1/library/tracks?source=qqmusic&view=compact",None).await;assert_eq!(status,StatusCode::OK);
 assert_eq!(full["total"],small["total"]);
 let track=small["items"].as_array().unwrap().iter().find(|t|t["id"]=="qq-only").unwrap();
 assert_eq!(track["provider_data"]["album_mid"],"album");assert!(track["provider_data"]["large_detail"].is_null());
 assert!(small.to_string().len()<full.to_string().len()/5);
}

#[tokio::test]
async fn favorites_sort_the_whole_collection_before_paging_and_keep_manual_order() {
 let f=Fixture::new().await;f.collection_tracks().await;
 f.s.db.run(|c|{c.execute("UPDATE tracks SET title='Zulu' WHERE id='local-shared'",[])?;c.execute("UPDATE tracks SET title='Alpha' WHERE id='local-unknown'",[])?;Ok(())}).await.unwrap();
 for id in ["local-shared","local-unknown"]{assert_eq!(f.call("PUT",&format!("/api/v1/favorites/{id}"),Some(json!({"liked":true}))).await.0,StatusCode::OK);}
 let favorite=f.call("GET","/api/v1/favorites",None).await.1;let id=favorite["id"].as_str().unwrap();
 let (status,sorted)=f.call("GET",&format!("/api/v1/playlists/{id}/tracks?sort=title&limit=1&offset=1&view=compact"),None).await;
 assert_eq!(status,StatusCode::OK);assert_eq!(sorted["items"][0]["title"],"Zulu");assert_eq!(sorted["total"],2);
 assert_eq!(f.call("GET",&format!("/api/v1/playlists/{id}/tracks?limit=1"),None).await.1["items"][0]["title"],"Zulu");
 assert_eq!(f.call("GET","/api/v1/favorites",None).await.1["track_ids"],favorite["track_ids"]);
 let (status,state)=f.call("PUT","/api/v1/zones/zone-local/queue?view=state",Some(json!({"track_ids":["local-shared","local-unknown"]}))).await;
 assert_eq!(status,StatusCode::OK);assert_eq!(state["queue_length"],2);assert_eq!(state["queue"],json!([]));
 let snapshot=f.call("GET","/api/v1/zones?view=state",None).await.1;
 let actual=snapshot["items"].as_array().unwrap().iter().find(|z|z["id"]=="zone-local").unwrap();assert_eq!(state["queue_revision"],actual["queue_revision"]);
}
