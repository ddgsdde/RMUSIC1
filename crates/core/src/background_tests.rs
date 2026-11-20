use super::*;
use futures_util::{stream,StreamExt};
use axum::body::Bytes;

#[tokio::test]
async fn cache_buffers_share_capacity_and_dropped_readers_flush_committed_ranges(){
    let f=Fixture::new().await;
    let input=||stream::iter([Ok(Bytes::from(vec![7;65536])),Ok(Bytes::from(vec![8;65536]))]).boxed();
    let mut a=media::caching_stream(f.s.clone(),"cloud".into(),"flac".into(),"\"entity-qbest\"".into(),131072,0,input()).await.unwrap();
    let mut b=media::caching_stream(f.s.clone(),"cloud".into(),"flac".into(),"\"entity-qbest\"".into(),131072,0,input()).await.unwrap();
    assert_eq!(a.next().await.unwrap().unwrap().len(),65536);b.next().await.unwrap().unwrap();
    assert_eq!(f.s.cache_buffers.lock().await.len(),1);
    assert_eq!(f.s.resources.status(&f.s).await["used_bytes"],131072);
    let (size,jobs)=f.s.db.run(|c|Ok((c.query_row("SELECT SUM(size) FROM cache_entries",[],|r|r.get::<_,i64>(0))?,c.query_row("SELECT COUNT(*) FROM jobs WHERE kind='cache_fill'",[],|r|r.get::<_,i64>(0))?))).await.unwrap();assert_eq!((size,jobs),(0,0));
    drop(a);drop(b);
    tokio::time::timeout(std::time::Duration::from_secs(3),async{loop{if f.s.cache_active.lock().await.is_empty(){break;}tokio::task::yield_now().await;}}).await.unwrap();
    assert_eq!(f.s.resources.status(&f.s).await["used_bytes"],0);
    assert_eq!(f.s.db.run(|c|Ok(c.query_row("SELECT SUM(size) FROM cache_entries",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),65536);
    assert_eq!(std::fs::read_dir(f.s.cache_dir().await).unwrap().count(),1,"部分缓存落盘供续传，但不作为完整歌曲发布");
    let mut tail=media::caching_stream(f.s.clone(),"cloud".into(),"flac".into(),"\"entity-qbest\"".into(),131072,65536,stream::iter([Ok(Bytes::from(vec![8;65536]))]).boxed()).await.unwrap();while let Some(chunk)=tail.next().await{chunk.unwrap();}
    assert_eq!(f.s.resources.status(&f.s).await["used_bytes"],0);assert_eq!(std::fs::read_dir(f.s.cache_dir().await).unwrap().count(),1);

    let id=crate::cache_policy::schedule(&f.s,"cloud","best","manual",None,50).await.unwrap();
    assert_eq!(f.call("PATCH",&format!("/api/v1/jobs/{id}"),Some(json!({"status":"paused"}))).await.0,StatusCode::NO_CONTENT);
    let restarted=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();
    let id=id.to_owned();assert_eq!(restarted.db.run(move|c|Ok(c.query_row("SELECT status FROM jobs WHERE id=?1",[id],|r|r.get::<_,String>(0))?)).await.unwrap(),"paused");
}
#[tokio::test]
async fn completion_budget_keeps_playback_reserve_and_partial_mode_does_not_fill(){
    let f=Fixture::new().await;
    f.s.db.run(|c|{c.execute("INSERT INTO settings(key,value) VALUES('managed_memory_bytes','33554432'),('playback_cache_mode','\"partial\"')",[])?;Ok(())}).await.unwrap();
    let background=f.s.resources.background(&f.s,24*1024*1024,false).await.unwrap();
    assert!(f.s.resources.background(&f.s,1,false).await.is_err());
    let playback=f.s.resources.memory(8*1024*1024,32*1024*1024).unwrap();drop(playback);drop(background);
    let mut input=media::caching_stream(f.s.clone(),"large".into(),"flac".into(),"\"large-qbest\"".into(),1024_u64.pow(3)*10,0,stream::iter([Ok(Bytes::from(vec![3;1024*1024+3]))]).boxed()).await.unwrap();
    while let Some(chunk)=input.next().await{chunk.unwrap();}
    assert_eq!(f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM jobs WHERE kind='cache_fill'",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),0);
    assert!(f.s.resources.status(&f.s).await["used_bytes"].as_u64().unwrap()<=1024*1024);
    assert_eq!(f.s.db.run(|c|Ok(c.query_row("SELECT complete FROM cache_entries WHERE track_id='large'",[],|r|r.get::<_,bool>(0))?)).await.unwrap(),false);
}

#[tokio::test]
async fn cache_publication_streams_only_a_complete_file_and_recovers_own_staging(){
    let f=Fixture::new().await;let remote=super::playlist_sync::provider(&f).await;
    let data=Bytes::from(vec![9;16384]);let expected=data.clone();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let app=axum::Router::new().route("/audio",axum::routing::get(move||{let data=data.clone();async move{axum::http::Response::builder().header("ETag","\"cache-source\"").body(axum::body::Body::from(data)).unwrap()}}));
    let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
    let mut value:Value=serde_json::from_slice(&std::fs::read(&remote).unwrap()).unwrap();value["source"]=json!({"url":format!("http://{address}/audio"),"format":"flac","cache_allowed":true,"expected_size":16384});std::fs::write(&remote,value.to_string()).unwrap();
    let track=crate::recommendations::select(&f.s,"example",None,vec![],1,false).await.unwrap().items.remove(0);
    let payload=json!({"track_id":track.id,"quality":"clean"});let job=crate::background::enqueue(&f.s,"cache_fill","test",payload.clone(),0).await.unwrap();let id=job.clone();f.s.db.run(move|c|{c.execute("UPDATE jobs SET status='running' WHERE id=?1",[id])?;Ok(())}).await.unwrap();
    assert!(media::fill_cache(&f.s,&job,&payload).await.unwrap());
    let (path,total)=f.s.db.run(|c|Ok(c.query_row("SELECT path,total_size FROM cache_entries WHERE complete=1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?)).await.unwrap();assert_eq!(total,16384);assert_eq!(std::fs::read(path).unwrap(),expected);
    let root=f.s.cache_dir().await;let interrupted=root.join(format!(".rmusic-incoming-{}",uuid::Uuid::new_v4()));std::fs::write(&interrupted,b"not complete").unwrap();let legacy=root.join("legacy-fragment");std::fs::write(&legacy,b"keep").unwrap();media::cleanup_incoming(&f.s).await.unwrap();assert!(!interrupted.exists());assert!(legacy.exists());
    f.s.plugin.stop("example").await;server.abort();
}
