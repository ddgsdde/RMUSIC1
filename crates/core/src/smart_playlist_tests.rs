use super::*;

#[tokio::test]
async fn smart_playlist_resolves_current_filtered_library_and_preserves_manual_lists() {
    let f=Fixture::new().await;f.collection_tracks().await;
    let (status,created)=f.call("POST","/api/v1/playlists",Some(json!({"name":"夜航规则","smart_query":{"q":"夜航"}}))).await;
    assert_eq!(status,StatusCode::OK);let id=created["id"].as_str().unwrap();let path=format!("/api/v1/playlists/{id}");
    let(_,listed)=f.call("GET","/api/v1/playlists",None).await;
    assert_eq!(listed["items"][0]["smart_query"],json!({"q":"夜航"}));assert_eq!(listed["items"][0]["track_ids"],json!([]));
    let(_,resolved)=f.call("GET",&path,None).await;
    let(_,tracks)=f.call("GET","/api/v1/library/tracks?q=%E5%A4%9C%E8%88%AA",None).await;
    assert_eq!(resolved["track_ids"],Value::Array(tracks["items"].as_array().unwrap().iter().map(|t|t["id"].clone()).collect()));
    assert!(!resolved["track_ids"].as_array().unwrap().contains(&json!("qq-shared")));assert_eq!(resolved["truncated"],false);
    f.s.db.run(|c|{c.execute("UPDATE tracks SET available=0 WHERE id='local-shared'",[])?;Ok(())}).await.unwrap();
    let(_,updated)=f.call("GET",&path,None).await;assert!(updated["track_ids"].as_array().unwrap().contains(&json!("qq-shared")));
    assert_eq!(f.call("PUT",&path,Some(json!({"name":"云端夜航","smart_query":{"q":"夜航","source":"qqmusic"}}))).await.0,StatusCode::OK);
    let(_,cloud)=f.call("GET",&path,None).await;assert_eq!(cloud["track_ids"].as_array().unwrap().len(),2);
    let(_,manual)=f.call("POST","/api/v1/playlists",Some(json!({"name":"手动","track_ids":["qq-only"]}))).await;
    let manual_path=format!("/api/v1/playlists/{}",manual["id"].as_str().unwrap());
    assert_eq!(f.call("GET",&manual_path,None).await.1["track_ids"],json!(["qq-only"]));
    assert_eq!(f.call("PUT",&manual_path,Some(json!({"name":"改名","track_ids":[]}))).await.0,StatusCode::OK);
    let db=db::Db::open(&f.s.config.data.join("db.sqlite")).unwrap();
    let restored=AppState::new(db,f.s.config.clone(),"unused".into()).await.unwrap();
    assert_eq!(call(&app(restored),"GET",&path,Some(&f.token),None).await.1["track_ids"],cloud["track_ids"]);
    let (album,artist)=f.s.db.run(|c|Ok(c.query_row("SELECT album_id,artist_id FROM tracks WHERE id='qq-live'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?)).await.unwrap();
    assert_eq!(f.call("PUT",&path,Some(json!({"smart_query":{"q":"夜航","source":"qqmusic","album_id":album,"artist_id":artist}}))).await.0,StatusCode::OK);
    assert_eq!(f.call("GET",&path,None).await.1["track_ids"],json!(["qq-live"]));
    assert_eq!(f.call("DELETE",&path,None).await.0,StatusCode::NO_CONTENT);assert_eq!(f.call("GET",&path,None).await.0,StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn smart_playlist_owner_isolation_and_explicit_result_bound() {
    let f=Fixture::new().await;
    let(_,created)=f.call("POST","/api/v1/playlists",Some(json!({"name":"owner","smart_query":{"source":"local"}}))).await;
    let path=format!("/api/v1/playlists/{}",created["id"].as_str().unwrap());
    assert_eq!(f.call("POST","/api/v1/auth/users",Some(json!({"username":"other","password":"other-test-password"}))).await.0,StatusCode::OK);
    let(_,login)=call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"other","password":"other-test-password"}))).await;
    let other=login["token"].as_str().unwrap();
    assert_eq!(call(&f.app,"GET","/api/v1/playlists",Some(other),None).await.1["items"],json!([]));
    for method in ["GET","PUT"] {assert_eq!(call(&f.app,method,&path,Some(other),if method=="PUT"{Some(json!({"name":"no"}))}else{None}).await.0,StatusCode::NOT_FOUND);}
    assert_eq!(call(&f.app,"DELETE",&path,Some(other),None).await.0,StatusCode::NO_CONTENT);
    assert_eq!(f.call("GET",&path,None).await.0,StatusCode::OK);
    f.s.db.run(|c|{c.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10001) INSERT INTO tracks(id,title,artist,album,album_artist,source,format,recording_id,created_at) SELECT printf('bound-%05d',x),'Bound','','','','local','wav',printf('recording-%05d',x),0 FROM n;")?;Ok(())}).await.unwrap();
    let(_,resolved)=f.call("GET",&path,None).await;
    assert_eq!(resolved["track_ids"].as_array().unwrap().len(),10000);assert_eq!(resolved["truncated"],true);
    assert_eq!(resolved["track_ids"][0],"bound-00001");assert_eq!(resolved["track_ids"][9999],"bound-10000");
    assert_eq!(f.call("GET","/api/v1/status",None).await.1["capabilities"]["smart_playlists"],true);
    let(_,doc)=f.call("GET","/api/v1/openapi.json",None).await;
    assert!(doc["paths"]["/api/v1/playlists/{id}"]["get"].is_object());
    assert_eq!(doc["components"]["schemas"]["SmartPlaylistQuery"]["additionalProperties"],false);
}

#[tokio::test]
async fn smart_playlist_rejects_invalid_rules_and_read_only_writes() {
    let f=Fixture::new().await;
    for body in [json!({"name":"invalid","smart_query":{"limit":2}}),json!({"name":"invalid","smart_query":{"q":"x".repeat(501)}}),json!({"name":"invalid","smart_query":[],"track_ids":[]}),json!({"name":"invalid","smart_query":{},"track_ids":[]})] {
        assert_eq!(f.call("POST","/api/v1/playlists",Some(body)).await.0,StatusCode::BAD_REQUEST);
    }
    let(_,created)=f.call("POST","/api/v1/playlists",Some(json!({"name":"empty","smart_query":{}}))).await;
    let path=format!("/api/v1/playlists/{}",created["id"].as_str().unwrap());
    assert_eq!(f.call("GET",&path,None).await.1["track_ids"],json!([]));
    assert_eq!(f.call("PUT",&path,Some(json!({"track_ids":[]}))).await.0,StatusCode::BAD_REQUEST);
    let(_,token)=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"read only","scopes":["read"]}))).await;
    let token=token["token"].as_str().unwrap();
    assert_eq!(call(&f.app,"GET",&path,Some(token),None).await.0,StatusCode::OK);
    for method in ["POST","PUT","DELETE"] {assert_eq!(call(&f.app,method,if method=="POST"{"/api/v1/playlists"}else{&path},Some(token),Some(json!({"name":"no","smart_query":{}}))).await.0,StatusCode::FORBIDDEN);}
}

#[test]
fn smart_playlist_migrates_existing_manual_playlist_database() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("old.db");
    let c=rusqlite::Connection::open(&path).unwrap();c.execute_batch("CREATE TABLE playlists(id TEXT PRIMARY KEY,name TEXT NOT NULL,user_id TEXT NOT NULL,tracks TEXT NOT NULL DEFAULT '[]'); INSERT INTO playlists VALUES('old','保留','owner','[\"track\"]');").unwrap();drop(c);
    let db=db::Db::open(&path).unwrap();let c=db.0.lock().unwrap();
    let row=c.query_row("SELECT tracks,smart_query FROM playlists WHERE id='old'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).unwrap();
    assert_eq!(row,("[\"track\"]".into(),None));
}
