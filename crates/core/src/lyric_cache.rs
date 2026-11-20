//! Shared lyric documents survive restarts. Cancellation of one controller must
//! not kill the shared plugin RPC or force the next controller to start again.
use crate::{error::{ApiError,ApiResult},models::Track,state::AppState};
use rusqlite::{params,OptionalExtension};
use serde_json::Value;
use std::sync::{Arc,Mutex,OnceLock,Weak};
const MAX_DOCUMENT:usize=1024*1024;
pub(crate) fn is_qrc_container(text: &str) -> bool {
    let text = text.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let root = if text.starts_with("<?xml") {
        text.split_once("?>").map_or(text, |(_, rest)| rest.trim_start())
    } else {
        text
    };
    root.starts_with("<QrcInfos")
}
fn gate(key:String)->Arc<tokio::sync::Mutex<()>> {
 static GATES:OnceLock<Mutex<std::collections::HashMap<String,Weak<tokio::sync::Mutex<()>>>>>=OnceLock::new();
 let mut gates=GATES.get_or_init(Default::default).lock().unwrap();gates.retain(|_,v|v.strong_count()>0);
 if let Some(g)=gates.get(&key).and_then(Weak::upgrade){return g;}let g=Arc::new(tokio::sync::Mutex::new(()));gates.insert(key,Arc::downgrade(&g));g
}
async fn cached(s:&AppState,key:String)->ApiResult<Option<(Value,i64,bool)>> {
 Ok(s.db.read(move|c|Ok(c.query_row("SELECT document,expires,nonempty FROM lyric_documents WHERE id=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,bool>(2)?))).optional()?.map(|(raw,at,nonempty)|Ok::<_,serde_json::Error>((serde_json::from_str::<Value>(&raw)?,at,nonempty))).transpose()?.filter(|(doc,_,_)|!doc["text"].as_str().is_some_and(is_qrc_container)))).await?)
}
pub async fn get(s:AppState,t:Track)->ApiResult<Value> {
 let source=t.provider_data["cached_from"]["source"].as_str().unwrap_or(&t.source).to_owned();
 let original=t.provider_data["cached_from"]["track_id"].as_str().unwrap_or(&t.id);
 let generation=if source!="local"{crate::plugin::check_enabled(&s,&source).await?;crate::provider_session::start(&s,&source).await?}else{0};
 let local=if t.source=="local" {let id=t.id.clone();s.db.read(move|c|Ok(c.query_row("SELECT COALESCE(lyrics,'') FROM tracks WHERE id=?1",[id],|r|r.get::<_,String>(0))?)).await?}else{String::new()};
 // Native sidecar/embedded lyrics are immediately available and need no provider.
 if source=="local"&&!local.is_empty(){return Ok(serde_json::json!({"format":if local.contains('['){"lrc"}else{"plain"},"text":local,"offset_ms":0,"source":"local"}));}
 let identity=if source=="local"{format!("{}:{}:{}:{}",t.title,t.artist,t.album,t.duration)}else{String::new()};
 let key=crate::auth::hash(&format!("lyrics-v2:{source}:{generation}:{original}:{}:{identity}",crate::auth::hash(&local)));
 let saved=cached(&s,key.clone()).await?;
 if let Some((value,expiry,_))=&saved{if *expiry>chrono::Utc::now().timestamp(){return Ok(value.clone());}}
 let lock=gate(format!("{}:{key}",s.core_id));
 if let Some((value,_,true))=saved {
  if let Ok(lease)=lock.try_lock_owned(){tokio::spawn(async move{let _lease=lease;let _=refresh(&s,&key,t,&source,generation).await;});}
  return Ok(value);
 }
 // Detaching the worker on a cancelled HTTP request preserves useful work.
 tokio::spawn(async move{let _lease=lock.lock_owned().await;refresh(&s,&key,t,&source,generation).await}).await.map_err(|_|ApiError::bad("歌词读取任务中断，请重试"))?
}
async fn refresh(s:&AppState,key:&str,t:Track,source:&str,generation:i64)->ApiResult<Value> {
 if let Some((value,expiry,_))=cached(s,key.into()).await?{if expiry>chrono::Utc::now().timestamp(){return Ok(value);}}
 let value=crate::library::fetch_lyrics(s.clone(),t).await?;
 let document=serde_json::to_string(&value).map_err(anyhow::Error::from)?;
 if document.len()>MAX_DOCUMENT{return Err(ApiError::bad("歌词文档超过 1 MiB"));}
 let nonempty=value["text"].as_str().is_some_and(|v|!v.trim().is_empty())||value["word_timing"].as_array().is_some_and(|v|!v.is_empty());
 let expires=chrono::Utc::now().timestamp()+if nonempty{86400*7}else{60};let key=key.to_owned();let source=source.to_owned();
 s.db.run(move|c|{let tx=c.transaction()?;if source!="local"{crate::provider_session::check(&tx,&source,generation)?;}
  tx.execute("INSERT INTO lyric_documents(id,document,expires,nonempty,size) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET document=excluded.document,expires=excluded.expires,nonempty=excluded.nonempty,size=excluded.size",params![key,document,expires,nonempty,document.len() as i64])?;
  // Only automatic lyric copies are bounded; embedded/sidecar originals stay.
  let mut used:i64=tx.query_row("SELECT COALESCE(SUM(size),0) FROM lyric_documents",[],|r|r.get(0))?;
  let mut count:i64=tx.query_row("SELECT COUNT(*) FROM lyric_documents",[],|r|r.get(0))?;
  if used>64*1024*1024||count>10000 {
   let rows=tx.prepare("SELECT id,size FROM lyric_documents WHERE id<>?1 ORDER BY expires,id")?.query_map([&key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
   for(id,size)in rows{if used<=64*1024*1024&&count<=10000{break;}tx.execute("DELETE FROM lyric_documents WHERE id=?1",[id])?;used-=size;count-=1;}
  }tx.commit()?;Ok(())}).await?;Ok(value)
}
#[cfg(test)]mod tests {
 use super::*;
 #[tokio::test]async fn lyric_singleflight_locks_share_only_live_work(){let a=gate("test-key".into());let b=gate("test-key".into());assert!(Arc::ptr_eq(&a,&b));let lease=a.lock().await;assert!(b.try_lock().is_err());drop(lease);assert!(b.try_lock().is_ok());}
}
