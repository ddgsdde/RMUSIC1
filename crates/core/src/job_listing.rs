//! Bounded task pages, including portable-file tasks, with cheap status summaries.
use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{extract::{State,Query},Json};
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;
#[derive(Default,Deserialize)] pub struct JobsQuery {limit:Option<usize>,offset:Option<usize>,status:Option<String>,id:Option<String>}
const CTE:&str="WITH listed AS (SELECT j.id,j.kind,j.status,j.processed,j.total,j.error,j.phase,j.updated_at,j.priority,COALESCE(t.title,p.name,CASE WHEN j.kind='playlist_directory' THEN json_extract(j.payload,'$.provider') ELSE NULL END,j.cursor,'') AS target FROM jobs j LEFT JOIN tracks t ON COALESCE(json_extract(j.payload,'$.track_id'),j.cursor)=t.id LEFT JOIN playlists p ON COALESCE(json_extract(j.payload,'$.local_id'),j.cursor)=p.id UNION ALL SELECT 'cache-export:'||e.cache_id,'cache_metadata',CASE WHEN e.status='ready' THEN 'completed' WHEN e.status='writing' THEN 'running' ELSE e.status END,0,0,e.error,'完善缓存标签、封面与歌词',e.updated_at,0,COALESCE(t.title,'') FROM cache_exports e JOIN cache_entries c ON c.id=e.cache_id LEFT JOIN tracks t ON t.id=c.track_id) ";
const ACTIVE:&str="status IN ('pending','running','waiting','interrupted','paused','waiting_provider')";
pub async fn list(State(s):State<AppState>,a:Auth,Query(q):Query<JobsQuery>)->ApiResult<Json<serde_json::Value>> {
 a.require("read")?;let status=q.status.unwrap_or_else(||"all".into());
 let filter=match status.as_str(){"all"=>"1=1".to_owned(),"active"=>ACTIVE.into(),"history"=>format!("NOT ({ACTIVE})"),_=>return Err(ApiError::bad("任务分类无效"))};
 let limit=q.limit.unwrap_or(10).clamp(1,100) as i64;let offset=q.offset.unwrap_or(0).min(1_000_000) as i64;let id=q.id.unwrap_or_default();
 let value=s.db.read(move|c|{
  let where_sql=format!("WHERE ({filter}) AND (?1='' OR id=?1)");
  let total=c.query_row(&format!("{CTE} SELECT COUNT(*) FROM listed {where_sql}"),[&id],|r|r.get::<_,i64>(0))?;
  let items=c.prepare(&format!("{CTE} SELECT id,kind,status,processed,total,error,phase,updated_at,priority,target FROM listed {where_sql} ORDER BY priority DESC,updated_at DESC,id DESC LIMIT ?2 OFFSET ?3"))?.query_map(params![id,limit,offset],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"processed":r.get::<_,i64>(3)?,"total":r.get::<_,i64>(4)?,"error":r.get::<_,Option<String>>(5)?,"phase":r.get::<_,String>(6)?,"updated_at":r.get::<_,i64>(7)?,"priority":r.get::<_,i64>(8)?,"target":r.get::<_,String>(9)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
  let mut counts=serde_json::Map::new();for row in c.prepare(&format!("{CTE} SELECT status,COUNT(*) FROM listed GROUP BY status"))?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))? {let(k,v)=row?;counts.insert(k,json!(v));}
  Ok(json!({"items":items,"total":total,"limit":limit,"offset":offset,"counts":counts}))
 }).await?;let mut value=value;value["resources"]=s.resources.status(&s).await;Ok(Json(value))
}
