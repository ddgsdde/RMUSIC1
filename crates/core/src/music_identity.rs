//! Provenanced identity assertions, separate from immutable playback and playlist IDs.
use crate::{auth::Auth,error::{ApiError,ApiResult},models::Track,state::AppState};
use axum::{extract::{Path,Query,State},Json};
use rusqlite::{params,OptionalExtension};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};

#[derive(Clone,Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assertion {pub kind:String,pub namespace:String,pub external_id:String,pub title:String,pub role:String,#[serde(default)]pub position:u32}
impl Assertion {
 pub(crate) fn validate(&self)->ApiResult<()> {
  let role=match self.kind.as_str(){"work"=>self.role=="work","recording"=>self.role=="recording","release"=>self.role=="release","artist"=>["performer","composer","conductor","lyricist","producer","engineer","album_artist"].contains(&self.role.as_str()),_=>false};
  if !role||self.namespace.is_empty()||self.namespace.len()>64||!self.namespace.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-".contains(&b))||self.external_id.is_empty()||self.external_id.len()>256||self.external_id.chars().any(char::is_control)||self.title.trim().is_empty()||self.title.len()>500||self.position>10000{return Err(ApiError::bad("音乐实体、身份空间、参与角色或标题无效"));}
  if self.namespace=="musicbrainz"&&uuid::Uuid::parse_str(&self.external_id).is_err(){return Err(ApiError::bad("MusicBrainz 身份须为 UUID"));}Ok(())
 }
 fn id(&self)->String {crate::auth::hash(&format!("{}\0{}\0{}",self.kind,self.namespace,self.external_id))}
}
pub(crate) fn insert(c:&rusqlite::Connection,track:&str,a:&Assertion,basis:&str)->anyhow::Result<()> {
 let id=a.id();c.execute("INSERT INTO music_entities(id,kind,namespace,external_id,title) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO NOTHING",params![id,a.kind,a.namespace,a.external_id,a.title])?;
 c.execute("INSERT INTO music_assertions(track_id,entity_id,role,basis,position,metadata) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(track_id,entity_id,role,basis) DO UPDATE SET position=excluded.position,metadata=excluded.metadata",params![track,id,a.role,basis,a.position,json!({"title":a.title,"assertion_only":true}).to_string()])?;Ok(())
}
/// Runs with the existing upsert transaction. Names alone never establish a cross-source recording.
pub(crate) fn index(c:&rusqlite::Connection,t:&Track,path:Option<&str>)->anyhow::Result<()> {
 c.execute_batch("SAVEPOINT music_identity_index")?;
 let result=index_inner(c,t,path);
 if result.is_ok(){c.execute_batch("RELEASE music_identity_index")?;}else{c.execute_batch("ROLLBACK TO music_identity_index; RELEASE music_identity_index")?;}
 result
}
fn index_inner(c:&rusqlite::Connection,t:&Track,path:Option<&str>)->anyhow::Result<()> {
 let prior=c.query_row("SELECT e.external_id FROM music_assertions a JOIN music_entities e ON e.id=a.entity_id WHERE a.track_id=?1 AND a.role='release' AND a.basis='source_identity' AND e.namespace=?2 AND (?5=0 OR (e.title=?3 AND EXISTS(SELECT 1 FROM music_assertions credit JOIN music_entities artist ON artist.id=credit.entity_id WHERE credit.track_id=?1 AND credit.role='album_artist' AND artist.title=?4))) LIMIT 1",params![t.id,t.source,t.album,t.album_artist,t.provider_data["identity"]["album_tagged"]==true],|r|r.get::<_,String>(0)).optional()?;
 c.execute("DELETE FROM music_assertions WHERE track_id=?1 AND basis!='manual'",[&t.id])?;
 let add=|kind:&str,namespace:&str,key:&str,title:&str,role:&str,basis:&str|->anyhow::Result<()>{let a=Assertion{kind:kind.into(),namespace:namespace.into(),external_id:key.into(),title:if title.trim().is_empty(){"未知".into()}else{title.into()},role:role.into(),position:t.track_number};if a.validate().is_ok(){insert(c,&t.id,&a,basis)?;}Ok(())};
 add("recording","rmusic",&t.recording_id,&t.title,"recording","source_identity")?;
 let release=t.provider_data["release_id"].as_str().or_else(||t.provider_data["album_mid"].as_str()).map(String::from).or(prior).unwrap_or_else(||crate::auth::hash(&format!("{}\0{}\0{}\0{}",t.source,path.and_then(|p|std::path::Path::new(p).parent()).map(|p|p.to_string_lossy()).unwrap_or_default(),t.album_artist,t.album)));
 if !t.album.trim().is_empty(){add("release",&t.source,&release,&t.album,"release","source_identity")?;}
 if let Some(credits)=t.provider_data["artist_credits"].as_array(){for credit in credits.iter().take(32){if let (Some(id),Some(name))=(credit["id"].as_str(),credit["name"].as_str()){add("artist",&t.source,id,name,"performer","provider_id")?;}}}
 if !t.artist.trim().is_empty(){add("artist",&t.source,&format!("{}:performer",t.id),&t.artist,"performer","unresolved_credit")?;}
 if !t.album_artist.trim().is_empty(){add("artist",&t.source,&format!("{}:album_artist",t.id),&t.album_artist,"album_artist","unresolved_credit")?;}
 for (field,kind,role,title) in [("work","work","work",t.title.as_str()),("recording","recording","recording",t.title.as_str()),("release","release","release",t.album.as_str()),("artist","artist","performer",t.artist.as_str()),("release_artist","artist","album_artist",t.album_artist.as_str())]{
  if let Some(ids)=t.provider_data["identifiers"]["musicbrainz"][field].as_array(){for id in ids.iter().take(32).filter_map(Value::as_str){if let Ok(id)=uuid::Uuid::parse_str(id){add(kind,"musicbrainz",&id.to_string(),title,role,"file_tags")?;}}}
 }
 for role in ["composer","conductor","lyricist","producer","engineer"] {if let Some(names)=t.provider_data["credits"][role].as_array(){for(name,i)in names.iter().filter_map(Value::as_str).zip(0..32){add("artist",&t.source,&format!("{}:{role}:{i}",t.id),name,role,"file_tags")?;}}}
 Ok(())
}
pub(crate) fn graph(c:&rusqlite::Connection,id:&str)->anyhow::Result<Value> {
 let revision=c.query_row("SELECT revision FROM music_identity_revisions WHERE track_id=?1",[id],|r|r.get::<_,i64>(0)).optional()?.unwrap_or(0);
 let entities=c.prepare("SELECT e.*,a.role,a.basis,a.position,a.metadata FROM music_assertions a JOIN music_entities e ON e.id=a.entity_id WHERE a.track_id=?1 ORDER BY a.role,a.position,e.id,a.basis")?.query_map([id],|r|{let metadata:Value=serde_json::from_str(&r.get::<_,String>("metadata")?).unwrap_or_default();Ok(json!({"id":r.get::<_,String>("id")?,"kind":r.get::<_,String>("kind")?,"namespace":r.get::<_,String>("namespace")?,"external_id":r.get::<_,String>("external_id")?,"title":metadata["title"].as_str().map(String::from).unwrap_or(r.get("title")?),"role":r.get::<_,String>("role")?,"basis":r.get::<_,String>("basis")?,"position":r.get::<_,u32>("position")?,"assertion_only":true}))})?.collect::<rusqlite::Result<Vec<_>>>()?;
 let recordings=entities.iter().filter(|e|e["kind"]=="recording").collect::<Vec<_>>();let mut relations=vec![];
 for recording in recordings {for entity in &entities {let relation=match entity["kind"].as_str(){Some("work")=>"recording_of",Some("release")=>"released_on",Some("artist")=>entity["role"].as_str().unwrap_or("performer"),_=>continue};relations.push(json!({"from":recording["id"],"to":entity["id"],"relation":relation,"basis":entity["basis"],"track_id":id}));}}
 Ok(json!({"track_id":id,"revision":revision,"entities":entities,"relations":relations,"automatic_merge":false}))
}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
 a.require("read")?;
 let track=s.db.run({let id=id.clone();move|c|Ok(c.query_row("SELECT * FROM tracks WHERE id=?1",[id],Track::from_row).optional()?)}).await?.ok_or_else(ApiError::missing)?;
 let value=s.db.run(move|c|{let tx=c.transaction()?;if !tx.query_row("SELECT EXISTS(SELECT 1 FROM music_assertions WHERE track_id=?1)",[&id],|r|r.get::<_,bool>(0))?{let path=tx.query_row("SELECT path FROM tracks WHERE id=?1",[&id],|r|r.get::<_,Option<String>>(0))?;index(&tx,&track,path.as_deref())?;}let value=graph(&tx,&id)?;tx.commit()?;Ok(value)}).await?;Ok(Json(value))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]
pub struct Edit {pub(crate) expected_revision:u64,pub(crate) assertions:Vec<Assertion>}
impl Edit {pub(crate) fn validate(&self)->ApiResult<()>{if self.expected_revision>=i64::MAX as u64||self.assertions.len()>32{return Err(ApiError::bad("一次最多32项人工身份关系"));}for item in &self.assertions{item.validate()?;}Ok(())}}
pub async fn put(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(edit):Json<Edit>)->ApiResult<Json<Value>> {
 a.require("library")?;edit.validate()?;
 let result=s.db.run(move|c|{let tx=c.transaction()?;
  if !tx.query_row("SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",[&id],|r|r.get::<_,bool>(0))?{return Ok(None);}
  let before=graph(&tx,&id)?;if before["revision"].as_u64()!=Some(edit.expected_revision){return Ok(Some(Err(())));}
  tx.execute("DELETE FROM music_assertions WHERE track_id=?1 AND basis='manual'",[&id])?;
  for assertion in &edit.assertions{insert(&tx,&id,assertion,"manual")?;}
  tx.execute("INSERT INTO music_identity_revisions(track_id,revision) VALUES(?1,1) ON CONFLICT(track_id) DO UPDATE SET revision=revision+1",[&id])?;
  let after=graph(&tx,&id)?;tx.execute("INSERT INTO music_identity_audit(track_id,revision,user_id,before_data,after_data,created_at) VALUES(?1,?2,?3,?4,?5,unixepoch())",params![id,(edit.expected_revision+1) as i64,a.id,before.to_string(),after.to_string()])?;tx.commit()?;Ok(Some(Ok(after)))
 }).await?.ok_or_else(ApiError::missing)?.map_err(|_|ApiError(axum::http::StatusCode::CONFLICT,"identity_revision_conflict".into(),"音乐身份已被修改，请重新读取后确认".into()))?;
 s.emit("music_identity",result.clone());Ok(Json(result))
}
#[derive(Deserialize)]pub struct Page {#[serde(default)]offset:u32,#[serde(default="limit")]limit:u32}
fn limit()->u32{30}
#[derive(Deserialize)]pub struct Search {q:String,kind:String,#[serde(default)]offset:u32}
pub async fn search(State(s):State<AppState>,a:Auth,Query(q):Query<Search>)->ApiResult<Json<Value>>{
 a.require("library")?;if q.q.len()>200||!["work","recording","release","artist"].contains(&q.kind.as_str())||q.offset>1000000{return Err(ApiError::bad("身份筛选无效"));}
 let value=s.db.read(move|c|{let pattern=format!("%{}%",q.q.replace('\\',"\\\\").replace('%',"\\%").replace('_',"\\_"));let total=c.query_row("SELECT COUNT(*) FROM music_entities WHERE kind=?1 AND title LIKE ?2 ESCAPE '\\'",params![q.kind,pattern],|r|r.get::<_,i64>(0))?;let items=c.prepare("SELECT kind,namespace,external_id,title,id FROM music_entities WHERE kind=?1 AND title LIKE ?2 ESCAPE '\\' ORDER BY title,id LIMIT 20 OFFSET ?3")?.query_map(params![q.kind,pattern,q.offset],|r|Ok(json!({"kind":r.get::<_,String>(0)?,"namespace":r.get::<_,String>(1)?,"external_id":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"id":r.get::<_,String>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(json!({"items":items,"total":total,"offset":q.offset}))}).await?;Ok(Json(value))
}
pub async fn entity(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(p):Query<Page>)->ApiResult<Json<Value>> {
 a.require("read")?;if !(1..=100).contains(&p.limit)||p.offset>1000000{return Err(ApiError::bad("实体分页无效"));}
 let value=s.db.run(move|c|{let entity=c.query_row("SELECT kind,namespace,external_id,title FROM music_entities WHERE id=?1",[&id],|r|Ok(json!({"id":id,"kind":r.get::<_,String>(0)?,"namespace":r.get::<_,String>(1)?,"external_id":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?}))).optional()?;
 let Some(entity)=entity else{return Ok(None);};let total=c.query_row("SELECT COUNT(DISTINCT track_id) FROM music_assertions WHERE entity_id=?1",[&id],|r|r.get::<_,i64>(0))?;
 let items=c.prepare("SELECT t.* FROM tracks t WHERE EXISTS(SELECT 1 FROM music_assertions a WHERE a.track_id=t.id AND a.entity_id=?1) ORDER BY t.disc_number,t.track_number,t.id LIMIT ?2 OFFSET ?3")?.query_map(params![id,p.limit,p.offset],Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(Some(json!({"entity":entity,"items":items,"total":total,"limit":p.limit,"offset":p.offset})))
 }).await?.ok_or_else(ApiError::missing)?;Ok(Json(value))
}

pub async fn enqueue_index(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>>{a.require("library")?;let id=crate::background::enqueue(&s,"identity_index","all",json!({}),0).await?;Ok(Json(json!({"id":id,"status":"pending"})))}
pub(crate) async fn run_index(s:&AppState,job:&str)->anyhow::Result<bool>{
 let key=job.to_owned();s.db.run(move|c|{let tx=c.transaction()?;
 let after=tx.query_row("SELECT COALESCE(json_extract(payload,'$.after'),'') FROM jobs WHERE id=?1",[&key],|r|r.get::<_,String>(0))?;
 let rows=tx.prepare("SELECT * FROM tracks WHERE id>?1 ORDER BY id LIMIT 100")?.query_map([&after],|r|Ok((Track::from_row(r)?,r.get::<_,Option<String>>("path")?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
 for(t,path)in &rows{index(&tx,t,path.as_deref())?;}
 let done=rows.len()<100;let last=rows.last().map(|r|r.0.id.as_str()).unwrap_or(&after);
 tx.execute("UPDATE jobs SET processed=processed+?2,payload=json_set(payload,'$.after',?3),phase='索引音乐身份',updated_at=unixepoch() WHERE id=?1",params![key,rows.len() as i64,last])?;tx.commit()?;Ok(done)
 }).await
}

pub async fn history(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(p):Query<Page>)->ApiResult<Json<Value>>{
 a.require("library")?;if !(1..=100).contains(&p.limit)||p.offset>1000000{return Err(ApiError::bad("历史分页无效"));}
 let value=s.db.run(move|c|{let total=c.query_row("SELECT COUNT(*) FROM music_identity_audit WHERE track_id=?1",[&id],|r|r.get::<_,i64>(0))?;
 let items=c.prepare("SELECT revision,before_data,after_data,created_at FROM music_identity_audit WHERE track_id=?1 ORDER BY revision DESC LIMIT ?2 OFFSET ?3")?.query_map(params![id,p.limit,p.offset],|r|{let before:Value=serde_json::from_str(&r.get::<_,String>(1)?).unwrap_or_default();let after:Value=serde_json::from_str(&r.get::<_,String>(2)?).unwrap_or_default();Ok(json!({"revision":r.get::<_,i64>(0)?,"before":before,"after":after,"created_at":r.get::<_,i64>(3)?}))})?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(json!({"items":items,"total":total}))}).await?;Ok(Json(value))
}
