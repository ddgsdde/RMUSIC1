use crate::{auth::Auth,error::{ApiResult,ApiError},state::AppState,models::Track};
use axum::{extract::{State,Path,Query},Json};
use serde_json::{json,Value};
use rusqlite::{params,OptionalExtension};

pub async fn home(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>>{
 a.require("read")?;
 let predicate=crate::accounts::scope::predicate("t");let user=if a.role=="external"{Some(a.id)}else{None};let preferred=if user.is_some(){"1"}else{crate::library::PREFERRED_COPY};
 let result=s.db.read(move|c|crate::accounts::listening::with_stats(c,user.as_deref(),||{
  let release=crate::library::RELEASE_ID;
  let mut albums=c.prepare(&format!("SELECT {release},album,album_artist,MAX(cover_url),COUNT(*),MAX(created_at),MIN(t.id) FROM tracks t WHERE available=1 AND {predicate} AND {} AND {} AND {} GROUP BY {release} ORDER BY MAX(created_at) DESC,1 LIMIT 12",crate::library::COLLECTION_VISIBILITY,crate::library_query::KNOWN_ALBUM,preferred))?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"artist":r.get::<_,String>(2)?,"cover_url":r.get::<_,Option<String>>(3)?,"track_count":r.get::<_,i64>(4)?,"track_id":r.get::<_,String>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
  for album in &mut albums {
   let id=album["track_id"].as_str().unwrap_or("");
   let t=c.query_row("SELECT * FROM tracks WHERE id=?1",[id],Track::from_row)?;
   album["track_id"]=json!(t.id);album["source"]=json!(t.source);
   let mut navigation=json!({});
   for key in ["artist_credits","artist_mids","album_mid","release_id"] {if let Some(value)=t.provider_data.get(key){navigation[key]=value.clone();}}
   if let Some(source)=t.provider_data["cached_from"]["source"].as_str(){navigation["cached_from"]=json!({"source":source});}
   album["provider_data"]=navigation;
   album["artist"]=json!(t.album_artist);album["album_artist"]=json!(t.album_artist);
  }
  let artists=c.prepare(&format!("SELECT credit.artist_id,credit.name,SUM(s.play_seconds),MAX(t.cover_url) FROM track_stats s JOIN tracks t ON t.id=s.track_id JOIN track_artists credit ON credit.track_id=t.id WHERE t.available=1 AND {predicate} AND s.play_seconds>0 GROUP BY credit.artist_id ORDER BY SUM(s.play_seconds) DESC,credit.artist_id LIMIT 12"))?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"listened_seconds":r.get::<_,f64>(2)?,"cover_url":r.get::<_,Option<String>>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
  let ranked=c.prepare(&format!("SELECT {} FROM track_stats s JOIN tracks t ON t.id=s.track_id WHERE t.available=1 AND {predicate} AND s.play_seconds>0 GROUP BY {} ORDER BY SUM(s.play_seconds) DESC,SUM(s.play_count) DESC,t.id LIMIT 12",crate::cache_identity::ORIGINAL,crate::cache_identity::ORIGINAL))?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
  let mut tracks=Vec::new();for id in ranked{let id=if user.is_some(){id}else{crate::cache_identity::representative(c,&id)?};tracks.push(c.query_row("SELECT * FROM tracks WHERE id=?1",[id],Track::from_row)?);}

  let mut recent_albums=c.prepare(&format!("WITH members AS (SELECT t.id,{release} release_key,{} logical_id FROM tracks t WHERE t.available=1 AND {predicate} AND {}), recent AS (SELECT t.release_key,MAX(s.last_played_at) last_played_at,MIN(t.id) track_id FROM members t JOIN track_stats s ON t.id=s.track_id WHERE s.play_seconds>0 GROUP BY t.release_key ORDER BY last_played_at DESC,t.release_key LIMIT 12) SELECT r.release_key,t.album,t.album_artist,t.cover_url,r.last_played_at,(SELECT COUNT(DISTINCT logical_id) FROM members m WHERE m.release_key=r.release_key),t.id FROM recent r JOIN tracks t ON t.id=r.track_id ORDER BY r.last_played_at DESC,r.release_key",crate::cache_identity::ORIGINAL,crate::library_query::KNOWN_ALBUM))?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"artist":r.get::<_,String>(2)?,"cover_url":r.get::<_,Option<String>>(3)?,"last_played_at":r.get::<_,Option<i64>>(4)?,"track_count":r.get::<_,i64>(5)?,"track_id":r.get::<_,String>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
  for album in &mut recent_albums {
   let t=c.query_row("SELECT * FROM tracks WHERE id=?1",[album["track_id"].as_str().unwrap_or("")],Track::from_row)?;
   album["source"]=json!(t.source);
   let mut navigation=json!({});
   for key in ["artist_credits","artist_mids","album_mid","release_id"] {if let Some(value)=t.provider_data.get(key){navigation[key]=value.clone();}}
   if let Some(source)=t.provider_data["cached_from"]["source"].as_str(){navigation["cached_from"]=json!({"source":source});}
   album["provider_data"]=navigation;
  }
  Ok(json!({"albums":albums,"artists":artists,"tracks":tracks.iter().map(crate::zones::snapshots::track).collect::<Vec<_>>(),"recent_albums":recent_albums}))
 })).await?;Ok(Json(result))
}
#[derive(serde::Deserialize)]pub struct FolderQuery{#[serde(default)]folder:String,#[serde(default="default_storage")]storage:String,limit:Option<usize>,offset:Option<usize>}
fn default_storage()->String{"local".into()}
pub async fn folders(State(s):State<AppState>,a:Auth,Query(query):Query<FolderQuery>)->ApiResult<Json<Value>>{
 a.require("read")?;if !["local","cache"].contains(&query.storage.as_str()){return Err(ApiError::bad("库类型无效"));}
 let value=s.db.run(move|c|{
  let sql=if query.storage=="local"{"SELECT path FROM tracks WHERE source='local' AND available=1 AND path IS NOT NULL AND json_type(provider_data,'$.cached_from.cache_id') IS NULL"}else{"SELECT path FROM cache_entries WHERE complete=1"};
  if query.storage=="local"&&query.folder.is_empty(){
   let roots=c.prepare("SELECT r.path,COUNT(t.id) FROM roots r JOIN tracks t ON t.root_id=r.id AND t.available=1 GROUP BY r.id ORDER BY r.path")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
   if !roots.is_empty(){let total=roots.len();let items=roots.into_iter().skip(query.offset.unwrap_or(0)).take(query.limit.unwrap_or(50).clamp(1,100)).map(|(path,count)|{let name=std::path::Path::new(&path).file_name().and_then(|v|v.to_str()).unwrap_or(&path).to_owned();json!({"name":name,"path":format!("{}{}",path.trim_end_matches(['/', '\\']),std::path::MAIN_SEPARATOR),"track_count":count})}).collect::<Vec<_>>();return Ok(json!({"items":items,"total":total}));}
  }
  let paths=c.prepare(sql)?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
  let mut folders=std::collections::BTreeMap::<String,(String,usize)>::new();let prefix=&query.folder;
  for path in paths{let path=std::path::PathBuf::from(path);let Some(parent)=path.parent() else{continue;};
   let (key,name)=if prefix.is_empty(){let parent=parent.to_string_lossy().into_owned();(format!("{parent}{}",std::path::MAIN_SEPARATOR),parent)}else{
    let base=std::path::Path::new(prefix);let Ok(relative)=path.strip_prefix(base)else{continue;};let mut parts=relative.components();let Some(first)=parts.next()else{continue;};if parts.next().is_none(){continue;}let name=first.as_os_str().to_string_lossy().into_owned();(format!("{}{}",base.join(&name).to_string_lossy(),std::path::MAIN_SEPARATOR),name)
   };let entry=folders.entry(key).or_insert((name,0));entry.1+=1;
  }
  let total=folders.len();Ok(json!({"total":total,"items":folders.into_iter().skip(query.offset.unwrap_or(0)).take(query.limit.unwrap_or(50).clamp(1,100)).map(|(path,(name,count))|json!({"path":path,"name":name,"track_count":count})).collect::<Vec<_>>()}))
 }).await?;Ok(Json(value))
}
async fn status(s:&AppState,id:String)->ApiResult<Value>{
 let track=crate::library::get_track(s,id.clone()).await?;
 if track.source=="local"{return Ok(json!({"status":"ready","track_id":id,"storage":if track.provider_data["cached_from"]["cache_id"].is_string(){"cache"}else{"local"},"cache_id":track.provider_data["cached_from"]["cache_id"]}));}
 Ok(s.db.run(move|c|{
  if let Some((cache,export))=c.query_row("SELECT e.id,x.status FROM cache_entries e LEFT JOIN cache_exports x ON x.cache_id=e.id WHERE e.track_id=?1 AND e.complete=1 ORDER BY e.last_access DESC LIMIT 1",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).optional()?{return Ok(json!({"status":if export.as_deref()==Some("ready"){"ready"}else{"tagging"},"cache_id":cache,"track_id":id}));}
  let job=c.query_row("SELECT status,error,phase,processed,total FROM jobs WHERE kind='cache_fill' AND json_extract(payload,'$.track_id')=?1 ORDER BY created_at DESC LIMIT 1",[&id],|r|Ok(json!({"status":r.get::<_,String>(0)?,"error":r.get::<_,Option<String>>(1)?,"phase":r.get::<_,String>(2)?,"processed":r.get::<_,i64>(3)?,"total":r.get::<_,i64>(4)?}))).optional()?;
  Ok(job.unwrap_or(json!({"status":"missing"})))
 }).await?)
}
pub async fn cache_status(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>>{a.require("read")?;Ok(Json(status(&s,id).await?))}
pub async fn cache(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>>{
 a.require("control")?;let track=crate::library::get_track(&s,id.clone()).await?;let existing=status(&s,id.clone()).await?;
 if track.source=="local"||["ready","tagging"].contains(&existing["status"].as_str().unwrap_or("")){return Ok(Json(existing));}
 let quality=crate::catalog::quality(&s,&track.source).await;let job=crate::cache_policy::schedule(&s,&id,&quality,"manual",None,50).await?;
 let key=job.clone();s.db.run(move|c|{c.execute("UPDATE jobs SET status='pending',error=NULL,next_try=0,updated_at=unixepoch() WHERE id=?1 AND status IN ('failed','cancelled','unable','completed')",params![key])?;Ok(())}).await?;
 Ok(Json(json!({"status":"pending","job_id":job,"track_id":id})))
}
