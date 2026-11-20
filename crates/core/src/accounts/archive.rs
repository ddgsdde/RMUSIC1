//! NDJSON archives exclude authentication material and use bounded keyset batches.
use super::*;
use axum::{body::{Body,Bytes},extract::{Path,Query,State},http::HeaderMap,response::Response,Json};
use crate::auth::account::Reauthenticate;
use futures_util::StreamExt;
use rusqlite::params;
 pub(crate) mod imports;
pub(crate) mod records;
pub(crate) mod preferences;
pub(crate) use records::{History,HistoryEntry,Stat,StatEntry,Origin,Playlist,Record};
const BATCH:usize=128;
pub const MAX_BYTES:usize=2*1024*1024*1024;
const MAX_LINE:usize=4*1024*1024;

#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub struct Credentials{#[serde(flatten)]credentials:Reauthenticate}
pub async fn export_ticket(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Credentials>)->ApiResult<Json<Value>> {
 admin::gate(&s,&a).await?;let uid=id.clone();let auth=a.clone();
 s.db.run(move|c|{let tx=c.transaction()?;admin::verify(&tx,&auth,&v.credentials)?;if public_user(&tx,&uid)?.is_none(){return Err(invalid("外部账号不存在"));}tx.commit()?;Ok(())}).await?;
 crate::auth::account::security_success(&s,&a).await;
 let ticket=s.ticket_for(format!("account-export:{id}"),60,Some(a.session_hash)).await;
 Ok(Json(json!({"api_path":format!("api/v1/auth/external-accounts/{id}/export?ticket={ticket}"),"filename":"rmusic-account.jsonl"})))
}
#[derive(Deserialize)]pub struct ExportQuery{ticket:String}
pub async fn export(State(s):State<AppState>,Path(id):Path<String>,Query(q):Query<ExportQuery>)->ApiResult<Response> {
 require_enabled(&s).await?;
 if !s.check_ticket(&q.ticket,&format!("account-export:{id}")).await{return Err(ApiError::auth());}
 let owner=s.tickets.lock().await.get(&crate::auth::hash(&q.ticket)).and_then(|t|t.owner.clone()).ok_or_else(ApiError::auth)?;
 let a=crate::auth::authenticate_hash(&s,owner.clone()).await?;a.require("admin")?;if !a.interactive{return Err(ApiError::forbidden());}
 let user=s.db.read({let id=id.clone();move|c|public_user(c,&id)}).await?.ok_or_else(ApiError::missing)?;
 let manifest=Record::Manifest{schema:1,origin_core:s.core_id.to_string(),origin_user:id.clone(),username:user["username"].as_str().unwrap_or("").into()};
 let stream=futures_util::stream::try_unfold((s,id,owner,0u8,String::new(),vec![manifest],0usize),|(s,id,owner,mut phase,mut cursor,mut pending,index)|async move{
  if !s.session_active(owner.clone()).await{return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied,"导出登录已失效"));}
  let mut index=index;
  if index>=pending.len(){
   if phase>=5{return Ok(None);}
   pending=s.db.read({let id=id.clone();let cursor=cursor.clone();let core=s.core_id.to_string();move|c|export_batch(c,&id,&core,phase,&cursor)}).await.map_err(|_|std::io::Error::other("账号导出读取失败"))?;
   if let Some(last)=pending.last(){cursor=last.cursor();}else{phase+=1;cursor.clear();}
   index=0;
   // A phase transition may contain no rows; do not buffer a whole archive.
   if pending.is_empty(){return Ok(Some((Bytes::new(),(s,id,owner,phase,cursor,pending,index))));}
  }
  let mut bytes=serde_json::to_vec(&pending[index]).map_err(std::io::Error::other)?;bytes.push(b'\n');index+=1;
  Ok::<_,std::io::Error>(Some((Bytes::from(bytes),(s,id,owner,phase,cursor,pending,index))))
 });
 Ok(Response::builder().header("content-type","application/x-ndjson; charset=utf-8").header("content-disposition","attachment; filename=rmusic-account.jsonl").header("cache-control","no-store").body(Body::from_stream(stream)).unwrap())
}
fn export_batch(c:&rusqlite::Connection,user:&str,core:&str,phase:u8,cursor:&str)->anyhow::Result<Vec<Record>> {
 match phase {
  0=>{
   let mut q=c.prepare("SELECT event_id,origin_core,origin_user,origin_id,data FROM account_history WHERE user_id=?1 AND event_id>?2 ORDER BY event_id LIMIT ?3")?;
   let result=q.query_map(params![user,cursor,BATCH as i64],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?.map(|r|{let(key,core,user,id,data)=r?;let mut record:History=serde_json::from_str(&data)?;record.session_id=key;Ok(Record::History{data:HistoryEntry{origin:Origin{core,user,id},record}})}).collect();result
  },
  1=>{
   let keys=cursor.split('\x1f').collect::<Vec<_>>();let a=keys.first().copied().unwrap_or("");let b=keys.get(1).copied().unwrap_or("");let d=keys.get(2).copied().unwrap_or("");
   let mut q=c.prepare("SELECT origin_core,origin_user,origin_id,data FROM account_stat_origins WHERE user_id=?1 AND (origin_core,origin_user,origin_id)>(?2,?3,?4) ORDER BY origin_core,origin_user,origin_id LIMIT ?5")?;
   let result=q.query_map(params![user,a,b,d,BATCH as i64],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))?.map(|r|{let(core,user,id,data)=r?;Ok(Record::Stat{data:StatEntry{origin:Origin{core,user,id},record:serde_json::from_str(&data)?}})}).collect();result
  },
  2=>{
   let mut q=c.prepare("SELECT id,name,tracks,smart_query FROM playlists WHERE user_id=?1 AND id>?2 ORDER BY id LIMIT ?3")?;
   let result=q.query_map(params![user,cursor,BATCH as i64],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?)))?.map(|r|{let(id,name,tracks,smart)=r?;Ok(Record::Playlist{data:Playlist{id,name,track_ids:serde_json::from_str(&tracks)?,smart_query:smart.map(|v|serde_json::from_str(&v)).transpose()?,favorites:false}})}).map(|r|r.map(|mut r|{if let Record::Playlist{data}=&mut r{data.favorites=data.id==format!("favorites-{}",crate::auth::hash(user));}r})).collect();result
  },
  4=>preferences::export(c,user,cursor),
  3 if cursor.is_empty()=>{let value=c.query_row("SELECT value FROM preferences WHERE user_id=?1",[user],|r|r.get::<_,String>(0)).optional()?.and_then(|v|serde_json::from_str::<Value>(&v).ok()).unwrap_or(json!({}));let _=core;Ok(vec![Record::Preferences{data:records::safe_preferences(value)}])},
  _=>Ok(vec![]),
 }
}
