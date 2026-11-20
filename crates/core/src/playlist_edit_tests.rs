use super::*;

#[tokio::test]
async fn playlist_edit_rejects_a_stale_snapshot_and_preserves_duplicate_positions() {
    let f=Fixture::new().await;
    let (_,created)=f.call("POST","/api/v1/playlists",Some(json!({"name":"Original","track_ids":["a","a","b"]}))).await;
    let path=format!("/api/v1/playlists/{}",created["id"].as_str().unwrap());
    let expected=json!({"name":"Original","track_ids":["a","a","b"]});
    assert_eq!(f.call("PUT",&path,Some(json!({"name":"Changed elsewhere"}))).await.0,StatusCode::OK);
    let (status,value)=f.call("PUT",&path,Some(json!({"name":"Overwrite","track_ids":["b","a","a"],"expected":expected}))).await;
    assert_eq!(status,StatusCode::CONFLICT,"{value}");
    let (_,latest)=f.call("GET",&path,None).await;
    assert_eq!(latest["name"],"Changed elsewhere");assert_eq!(latest["track_ids"],json!(["a","a","b"]));
    let expected=json!({"name":"Changed elsewhere","track_ids":["a","a","b"]});
    let (status,value)=f.call("PUT",&path,Some(json!({"name":"Sorted","track_ids":["b","a","a"],"expected":expected}))).await;
    assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["track_ids"],json!(["b","a","a"]));
    assert_eq!(f.call("PUT",&path,Some(json!({"name":"Invalid","expected":{"name":12}}))).await.0,StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn simultaneous_playlist_edit_has_one_winner_and_group_seek_rejects_jump_fields() {
    let f=Fixture::new().await;
    let (_,created)=f.call("POST","/api/v1/playlists",Some(json!({"name":"Race","track_ids":["a","a"]}))).await;
    let path=format!("/api/v1/playlists/{}",created["id"].as_str().unwrap());
    let a=json!({"name":"First","track_ids":["a"],"expected":{"name":"Race","track_ids":["a","a"]}});
    let b=json!({"name":"Second","track_ids":["a","a","a"],"expected":{"name":"Race","track_ids":["a","a"]}});
    let (one,two)=tokio::join!(f.call("PUT",&path,Some(a)),f.call("PUT",&path,Some(b)));
    let mut statuses=vec![one.0,two.0];statuses.sort();assert_eq!(statuses,vec![StatusCode::OK,StatusCode::CONFLICT]);
    let path=format!("/api/v1/sync/starts/{}/control",uuid::Uuid::new_v4());
    let (_,error)=f.call("POST",&path,Some(json!({"request_id":uuid::Uuid::new_v4(),"action":"seek","queue_index":0,"position":1,"from_queue_index":0}))).await;
    assert!(error["error"]["message"].as_str().unwrap_or("").contains("不接受原曲序"),"{error}");
}

#[tokio::test]
async fn cache_listing_pages_have_stable_ties_and_reject_unbounded_requests() {
    let f=Fixture::new().await;
    f.s.db.run(|c|{let tx=c.transaction()?;for index in 0..123 {tx.execute("INSERT INTO cache_entries(id,track_id,format,path,last_access,complete) VALUES(?1,?1,'flac','synthetic-unused',1,1)",[format!("cache-{index:03}")])?;}tx.commit()?;Ok(())}).await.unwrap();
    let (_,one)=f.call("GET","/api/v1/cache?limit=50",None).await;
    let (_,two)=f.call("GET","/api/v1/cache?limit=50&offset=50",None).await;
    let (_,three)=f.call("GET","/api/v1/cache?limit=50&offset=100",None).await;
    assert_eq!(one["total"],123);assert_eq!(one["entries"].as_array().unwrap().len(),50);assert_eq!(two["entries"][0]["id"],"cache-050");assert_eq!(three["entries"].as_array().unwrap().len(),23);
    assert_eq!(f.call("GET","/api/v1/cache?limit=1001",None).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(f.call("GET","/api/v1/cache?offset=1000001",None).await.0,StatusCode::BAD_REQUEST);
}
