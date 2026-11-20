use super::*;

#[tokio::test]
async fn cached_recordings_have_one_representative_before_counts_and_paging() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let (artist, album) = f.s.db.run(|c| {
        let template = c.query_row("SELECT * FROM tracks WHERE id='qq-only'", [], models::Track::from_row)?;
        for i in 0..60 {
            let mut cloud = template.clone();
            cloud.id = format!("qqmusic:Duplicate{i:03}");
            cloud.recording_id = cloud.id.clone();
            cloud.title = format!("Song {i:03}");
            cloud.artist = "Verified Artist".into();
            cloud.album_artist = cloud.artist.clone();
            cloud.album = "Verified Album".into();
            cloud.provider_data = json!({"artist_mids":["Artist001"],"album_mid":"Release001"});
            library::upsert(c,&cloud,None,None,0,0,None,None)?;
            let mut cached = cloud.clone();
            cached.id = format!("cached-{i:03}");
            cached.source = "local".into();
            cached.provider_data["cached_from"] = json!({"source":"qqmusic","track_id":cloud.id,"cache_id":format!("cache-{i:03}")});
            library::upsert(c,&cached,Some(&format!("/fixture/cache-{i:03}.flac")),None,0,0,None,None)?;
        }
        let artist=crate::artist_credits::credits("qqmusic","Verified Artist",&json!({"artist_mids":["Artist001"]}))[0].id.clone();
        Ok((artist,"release:qqmusic:Release001".to_owned()))
    }).await.unwrap();
    for (kind,id) in [("artist",&artist),("album",&album)] {
        let uri=format!("/api/v1/library/catalog/{kind}/{id}?remote=false");
        let(status,first)=f.call("GET",&uri,None).await;
        assert_eq!(status,StatusCode::OK,"{first}");
        assert_eq!(first["local"]["total"],60,"{kind} must count recordings, not cached source duplicates");
        assert_eq!(first["local"]["items"].as_array().unwrap().len(),if kind=="album"{60}else{50});
        assert_eq!(first["local"]["next_page"],if kind=="album"{Value::Null}else{json!(2)});
        let(_,second)=f.call("GET",&(uri+"&local_page=2"),None).await;
        assert_eq!(second["local"]["items"].as_array().unwrap().len(),if kind=="album"{0}else{10});
        let ids:std::collections::HashSet<_>=first["local"]["items"].as_array().unwrap().iter().chain(second["local"]["items"].as_array().unwrap()).map(|t|t["id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(),60);assert!(ids.iter().all(|id|id.starts_with("cached-")));
    }
    // Availability changes select the surviving source without destroying either row.
    f.s.db.run(|c|{c.execute("UPDATE tracks SET available=0 WHERE id='cached-000'",[])?;Ok(())}).await.unwrap();
    let(_,page)=f.call("GET",&format!("/api/v1/library/catalog/artist/{artist}?remote=false&sort=title"),None).await;
    assert_eq!(page["local"]["total"],60);assert_eq!(page["local"]["items"][0]["id"],"qqmusic:Duplicate000");
    let (_,listed)=f.call("GET","/api/v1/library/tracks?q=Song&limit=200",None).await;
    assert_eq!(listed["total"],60);
    // Source-filtered views still allow explicitly inspecting cloud records.
    let (_,cloud)=f.call("GET","/api/v1/library/tracks?q=Song&source=qqmusic&limit=200",None).await;
    assert_eq!(cloud["total"],60);
    let (_,albums)=f.call("GET",&format!("/api/v1/library/catalog/artist/{artist}/albums"),None).await;
    assert_eq!(albums["local"]["total"],1,"a partially cached album has one identity");
    let (_,artists)=f.call("GET","/api/v1/library/artists?q=Verified",None).await;
    let card=artists["items"].as_array().unwrap().iter().find(|a|a["id"]==artist).unwrap();assert_eq!(card["track_count"],60);
    let count=f.s.db.read(|c|Ok(c.query_row("SELECT COUNT(*) FROM tracks WHERE id LIKE 'cached-%' OR id LIKE 'qqmusic:Duplicate%'",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(count,120);
}

#[tokio::test]
async fn same_titles_distinct_recordings_and_unidentified_records_stay_separate() {
    let f=Fixture::new().await;f.collection_tracks().await;
    let artist=f.s.db.run(|c|{
        let template=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;
        for (id,source,recording) in [("qqmusic:Studio001","qqmusic","qqmusic:Studio001"),("cache-studio","local","qqmusic:Studio001"),("qqmusic:Live001","qqmusic","qqmusic:Live001"),("qqmusic:Other001","qqmusic","qqmusic:Studio001"),("qqmusic:Unknown001","qqmusic",""),("unknown-file","local","")] {
            let mut t=template.clone();t.id=id.into();t.source=source.into();t.recording_id=recording.into();t.title="Identical title".into();t.artist="Verified Artist".into();t.album_artist=t.artist.clone();t.provider_data=json!({"artist_mids":["Artist001"],"album_mid":"Release001"});
            // Another release may share a recording, but this cache belongs only to Studio001.
            if id=="qqmusic:Other001"{t.provider_data["album_mid"]=json!("OtherRelease001");}
            if id=="cache-studio"{t.provider_data["cached_from"]=json!({"source":"qqmusic","track_id":"qqmusic:Studio001","cache_id":"cache-studio"});}
            if id=="unknown-file"{t.provider_data["cached_from"]=json!({"source":"qqmusic","track_id":"qqmusic:Unknown001","cache_id":"unknown-file"});}
            library::upsert(c,&t,if source=="local"{Some(id)}else{None},None,0,0,None,None)?;
        }
        Ok(crate::artist_credits::credits("qqmusic","Verified Artist",&json!({"artist_mids":["Artist001"]}))[0].id.clone())
    }).await.unwrap();
    let(_,page)=f.call("GET",&format!("/api/v1/library/catalog/artist/{artist}?remote=false"),None).await;
    assert_eq!(page["local"]["total"],5);
    let ids=page["local"]["items"].as_array().unwrap().iter().map(|v|v["id"].as_str().unwrap()).collect::<std::collections::HashSet<_>>();
    assert!(!ids.contains("qqmusic:Studio001"));
    for id in ["cache-studio","qqmusic:Live001","qqmusic:Other001","qqmusic:Unknown001","unknown-file"]{assert!(ids.contains(id));}
}

#[tokio::test]
async fn cached_copy_projections_keep_favorites_statistics_home_and_smart_results_consistent(){
 let f=Fixture::new().await;f.collection_tracks().await;
 f.s.db.run(|c|{
  let mut original=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;original.recording_id="cloud-recording".into();
  let mut local=original.clone();local.id="cached-owned".into();local.source="local".into();local.provider_data=json!({"cached_from":{"source":"qqmusic","track_id":"qq-only","cache_id":"cache-owned"}});library::upsert(c,&local,Some("/fixture/cached-owned"),None,0,0,None,None)?;
  for id in ["qq-only","cached-owned"]{c.execute("INSERT INTO track_stats(track_id,play_count,play_seconds,last_played_at)VALUES(?1,1,30,unixepoch())",[id])?;c.execute("INSERT INTO daily_stats(day,track_id,title,artist,play_count,play_seconds,last_played_at)VALUES(date('now'),?1,'Title','Artist',1,30,unixepoch())",[id])?;}Ok(())
 }).await.unwrap();
 let(status,_)=f.call("PUT","/api/v1/favorites/qq-only",Some(json!({"liked":true}))).await;assert_eq!(status,StatusCode::OK);
 let(_,liked)=f.call("PUT","/api/v1/favorites/cached-owned",Some(json!({"liked":true}))).await;assert_eq!(liked["track_ids"],json!(["qq-only"]));assert!(liked["alias_track_ids"].as_array().unwrap().contains(&json!("cached-owned")));
 let(_,cards)=f.call("GET","/api/v1/playlists",None).await;let card=cards["items"].as_array().unwrap().iter().find(|v|v["id"]==liked["id"]).unwrap();assert_eq!(card["track_ids"],json!(["qq-only"]));
 let(_,fav)=f.call("GET",&format!("/api/v1/playlists/{}/tracks",liked["id"].as_str().unwrap()),None).await;assert_eq!(fav["total"],1);assert_eq!(fav["items"][0]["id"],"cached-owned");
 let(_,stats)=f.call("GET","/api/v1/stats?period_days=1",None).await;assert_eq!(stats["summary"]["unique_tracks"],1,"{stats}");assert_eq!(stats["summary"]["play_count"],2);assert_eq!(stats["artists"][0]["unique_tracks"],1);assert_eq!(stats["albums"][0]["unique_tracks"],1);assert_eq!(stats["tracks"].as_array().unwrap().len(),1);assert_eq!(stats["tracks"][0]["listened_seconds"],60.);
 let(_,home)=f.call("GET","/api/v1/library/home",None).await;assert_eq!(home["tracks"].as_array().unwrap().len(),1);assert_eq!(home["tracks"][0]["id"],"cached-owned");
 let(_,smart)=f.call("POST","/api/v1/smart-playlists/preview",Some(json!({"rule":{"favorites_only":true}}))).await;assert_eq!(smart["total"],1,"{smart}");
 let(_,manual)=f.call("POST","/api/v1/playlists",Some(json!({"name":"Deliberate positions","track_ids":["qq-only","qq-only","cached-owned"]}))).await;
 let(_,positions)=f.call("GET",&format!("/api/v1/playlists/{}/tracks",manual["id"].as_str().unwrap()),None).await;assert_eq!(positions["total"],3);
 // Explicit unlike through the cache removes the source favorite, not another playlist's positions.
 let(_,liked)=f.call("PUT","/api/v1/favorites/cached-owned",Some(json!({"liked":false}))).await;assert_eq!(liked["track_ids"],json!([]));
}

#[tokio::test]
async fn smart_favorites_match_an_old_cache_id_but_never_unverified_lineage(){
 let f=Fixture::new().await;f.collection_tracks().await;
 let(_,favorite)=f.call("PUT","/api/v1/favorites/qq-only",Some(json!({"liked":true}))).await;let id=favorite["id"].as_str().unwrap().to_owned();
 f.s.db.run(move|c|{
  let mut original=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;original.recording_id="cloud-recording".into();library::upsert(c,&original,None,None,0,0,None,None)?;
  for (id,depth,recording,source)in[("cache-low",16,"cloud-recording","qqmusic"),("cache-high",24,"cloud-recording","qqmusic"),("cache-unknown",24,"","qqmusic"),("cache-foreign",24,"cloud-recording","another-provider")] {
   let mut t=original.clone();t.id=id.into();t.source="local".into();t.bit_depth=depth;t.recording_id=recording.into();t.provider_data=json!({"cached_from":{"source":source,"track_id":"qq-only","cache_id":id}});library::upsert(c,&t,Some(id),None,0,0,None,None)?;
  }
  c.execute(r#"UPDATE playlists SET tracks='["cache-low"]' WHERE id=?1"#,[id])?;Ok(())
 }).await.unwrap();
 let(_,smart)=f.call("POST","/api/v1/smart-playlists/preview",Some(json!({"rule":{"favorites_only":true}}))).await;assert_eq!(smart["total"],1,"{smart}");assert_eq!(smart["items"][0]["id"],"cache-high");
 let(_,smart)=f.call("POST","/api/v1/smart-playlists/preview",Some(json!({"rule":{"exclude_saved":true}}))).await;let ids=smart["items"].as_array().unwrap().iter().map(|t|t["id"].as_str().unwrap()).collect::<std::collections::HashSet<_>>();assert!(!ids.contains("cache-high"));assert!(ids.contains("cache-unknown")&&ids.contains("cache-foreign"));
}

#[tokio::test]
async fn home_has_cached_album_artwork_before_any_catalog_detail_is_opened() {
    let f=Fixture::new().await;f.collection_tracks().await;
    let expected=f.s.db.run(|c|{
        let mut origin=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;
        origin.provider_data=json!({"album_mid":"home-release"});origin.cover_url=Some("https://art.example/home.jpg".into());
        library::upsert(c,&origin,None,None,0,0,None,None)?;
        let mut cached=origin.clone();cached.id="home-cache".into();cached.source="local".into();cached.cover_url=None;cached.provider_data["cached_from"]=json!({"source":"qqmusic","track_id":origin.id,"cache_id":"home-cache"});
        library::upsert(c,&cached,Some("/fixture/home-cache"),None,0,0,None,None)?;
        c.execute("INSERT INTO track_stats(track_id,play_count,play_seconds,last_played_at)VALUES('home-cache',1,10,unixepoch())",[])?;
        Ok(crate::media_assets::register(c,"https://art.example/home.jpg")?)
    }).await.unwrap();
    let(status,home)=f.call("GET","/api/v1/library/home",None).await;assert_eq!(status,StatusCode::OK);
    assert_eq!(home["albums"].as_array().unwrap().iter().find(|a|a["id"]=="release:qqmusic:home-release").unwrap()["cover_url"],expected);
    assert_eq!(home["tracks"].as_array().unwrap().iter().find(|t|t["id"]=="home-cache").unwrap()["cover_url"],expected);
    assert_eq!(home["recent_albums"].as_array().unwrap().iter().find(|a|a["id"]=="release:qqmusic:home-release").unwrap()["cover_url"],expected);
}

#[tokio::test]
async fn album_shelves_group_provider_editions_across_track_credits() {
    let f=Fixture::new().await;
    f.collection_tracks().await;
    f.s.db.run(|c|{
        let seed=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;
        for (id,artist,release) in [("album-main","Composer","Complete01"),("album-theme","Theme Artist","Complete01"),("album-different","Composer","Other01")] {
            let mut track=seed.clone();
            track.id=id.into();track.recording_id=id.into();
            track.album="Soundtrackfixture".into();
            track.artist=artist.into();track.album_artist=artist.into();
            track.provider_data=json!({"album_mid":release});
            library::upsert(c,&track,None,None,0,0,None,None)?;
        }
        Ok(())
    }).await.unwrap();
    let complete="release:qqmusic:Complete01";
    let (status,albums)=f.call("GET","/api/v1/library/albums?q=Soundtrackfixture",None).await;
    assert_eq!(status,StatusCode::OK,"{albums}");
    assert_eq!(albums["total"],2);
    let cards=albums["items"].as_array().unwrap();
    assert_eq!(cards.iter().find(|a|a["id"]==complete).unwrap()["track_count"],2);
    assert!(cards.iter().any(|a|a["id"]=="release:qqmusic:Other01"));
    let (status,home)=f.call("GET","/api/v1/library/home",None).await;
    assert_eq!(status,StatusCode::OK,"{home}");
    let cards:Vec<_>=home["albums"].as_array().unwrap().iter().filter(|a|a["id"]==complete).collect();
    assert_eq!(cards.len(),1);
    assert_eq!(cards[0]["track_count"],2);
    let (status,tracks)=f.call("GET","/api/v1/library/tracks?album_id=release%3Aqqmusic%3AComplete01",None).await;
    assert_eq!(status,StatusCode::OK,"{tracks}");
    assert_eq!(tracks["total"],2);
}
