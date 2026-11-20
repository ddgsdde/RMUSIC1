use super::*;

#[tokio::test]
async fn relocated_music_keeps_track_identity_playlists_and_history_after_rescan(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;
    let identity=f.call("GET",&format!("/api/v1/library/tracks/{id}/identity"),None).await.1;
    let target=f.s.config.data.join("relocated-music");
    std::fs::rename(f.s.config.data.join("测试专辑"),&target).unwrap();
    let folder=std::fs::canonicalize(&target).unwrap().to_string_lossy().to_string();
    let path=std::fs::canonicalize(target.join("夜航.wav")).unwrap().to_string_lossy().to_string();
    let saved=id.clone();
    f.s.db.run(move |c|{
        c.execute("UPDATE roots SET path=?1",[folder])?;
        c.execute("UPDATE tracks SET path=?1,mtime=0 WHERE id=?2",rusqlite::params![path,saved])?;
        c.execute("INSERT INTO track_stats(track_id,play_seconds) VALUES(?1,42)",[saved])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(f.call("POST","/api/v1/playlists",Some(json!({"name":"Migration","track_ids":[id]}))).await.0,StatusCode::OK);
    let(status,job)=f.call("POST","/api/v1/library/scan",Some(json!({}))).await;assert_eq!(status,StatusCode::OK);
    let completed=tokio::time::timeout(std::time::Duration::from_secs(5),async{loop{
        let(_,jobs)=f.call("GET","/api/v1/jobs",None).await;
        if let Some(current)=jobs["items"].as_array().unwrap().iter().find(|j|j["id"]==job["id"]) {if matches!(current["status"].as_str(),Some("completed"|"failed"|"cancelled")) {break current.clone();}}
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }}).await.unwrap();assert_eq!(completed["status"],"completed");
    let(_,tracks)=f.call("GET","/api/v1/library/tracks",None).await;assert_eq!(tracks["total"],1);assert_eq!(tracks["items"][0]["id"],id);
    let after_identity=f.call("GET",&format!("/api/v1/library/tracks/{id}/identity"),None).await.1;let ids=|value:&Value|value["entities"].as_array().unwrap().iter().map(|e|e["id"].as_str().unwrap().to_owned()).collect::<std::collections::BTreeSet<_>>();assert_eq!(ids(&after_identity),ids(&identity),"moving a root must preserve identity IDs, while inferred labels may reflect the new folder");
    let(_,playlists)=f.call("GET","/api/v1/playlists",None).await;assert_eq!(playlists["items"][0]["track_ids"][0],id);
    let seconds:f64=f.s.db.run(move |c|Ok(c.query_row("SELECT play_seconds FROM track_stats WHERE track_id=?1",[id],|r|r.get(0))?)).await.unwrap();assert_eq!(seconds,42.);
}
