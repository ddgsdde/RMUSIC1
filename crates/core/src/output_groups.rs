//! Shared saved room selections. Editing definitions never dispatches playback.
use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{extract::{Path,Query,State},http::StatusCode,Json};
use rusqlite::{params,OptionalExtension};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::collections::HashSet;

#[derive(Clone,Deserialize,Serialize)]
pub struct Group {id:String,name:String,zone_ids:Vec<String>,revision:u64,updated_at:i64}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {name:String,zone_ids:Vec<String>,expected_revision:u64}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {expected_revision:u64}
enum Change {Saved(Group,bool),Deleted,Conflict,InvalidMembers}
fn id(value:&str)->ApiResult<String>{uuid::Uuid::parse_str(value).map(|id|id.to_string()).map_err(|_|ApiError::bad("组合标识须为 UUID"))}
fn conflict()->ApiError {ApiError(StatusCode::CONFLICT,"group_conflict".into(),"组合已被其他控制端修改或删除，请刷新后重新选择；当前输入可保留".into())}
fn load(c:&rusqlite::Connection,id:&str)->anyhow::Result<Option<(Group,bool)>> {
    let row=c.query_row("SELECT data,deleted FROM output_groups WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?))).optional()?;
    row.map(|(data,deleted)|Ok((serde_json::from_str(&data)?,deleted))).transpose()
}
pub async fn list(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>> {
    a.require("read")?;
    let items=s.db.run(|c|{let data=c.prepare("SELECT data FROM output_groups WHERE deleted=0 ORDER BY json_extract(data,'$.name'),id")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;data.into_iter().map(|data|Ok(serde_json::from_str::<Group>(&data)?)).collect::<anyhow::Result<Vec<_>>>()}).await?;
    Ok(Json(json!({"items":items})))
}
pub async fn save(State(s):State<AppState>,a:Auth,Path(value):Path<String>,Json(mut input):Json<Save>)->ApiResult<(StatusCode,Json<Group>)> {
    a.require("control")?;let id=id(&value)?;input.name=input.name.trim().into();
    if input.name.is_empty()||input.name.chars().count()>80||input.name.chars().any(char::is_control)||!(2..=4).contains(&input.zone_ids.len())||input.zone_ids.iter().any(|id|id.is_empty()||id.len()>200){return Err(ApiError::bad("请输入至多 80 字的组合名，并选择二至四个不同音区"));}
    input.zone_ids.sort();if input.zone_ids.windows(2).any(|ids|ids[0]==ids[1]){return Err(ApiError::bad("组合音区不能重复"));}
    let _guard=s.zone_lock.lock().await;let zones=s.zones.read().await;
    let mut endpoints=HashSet::new();let valid=input.zone_ids.iter().all(|id|zones.get(id).is_some_and(|zone|endpoints.insert(zone.endpoint_id.clone())));drop(zones);
    let result=s.db.run(move|c|{let tx=c.transaction()?;let previous=load(&tx,&id)?;let created=previous.is_none();
        let revision=if let Some((group,deleted))=previous {
            if deleted{return Ok(Change::Conflict);}
            let same=group.name==input.name&&group.zone_ids==input.zone_ids;
            if same&&(input.expected_revision==group.revision||input.expected_revision.checked_add(1)==Some(group.revision)){return Ok(Change::Saved(group,false));}
            if group.revision!=input.expected_revision{return Ok(Change::Conflict);}group.revision+1
        }else{if input.expected_revision!=0{return Ok(Change::Conflict);}1};
        if !valid{return Ok(Change::InvalidMembers);}
        let group=Group{id,name:input.name,zone_ids:input.zone_ids,revision,updated_at:chrono::Utc::now().timestamp_millis()};
        tx.execute("INSERT INTO output_groups(id,data,deleted) VALUES(?1,?2,0) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![group.id,serde_json::to_string(&group)?])?;tx.commit()?;Ok(Change::Saved(group,created))
    }).await?;
    match result {Change::Saved(group,created)=>{s.emit("output_groups",json!({}));Ok((if created{StatusCode::CREATED}else{StatusCode::OK},Json(group)))},Change::InvalidMembers=>Err(ApiError::bad("音区已移除或重复使用同一输出，请刷新选择")),_=>Err(conflict())}
}
pub async fn delete(State(s):State<AppState>,a:Auth,Path(value):Path<String>,Query(input):Query<Revision>)->ApiResult<StatusCode> {
    a.require("control")?;let id=id(&value)?;if input.expected_revision==0{return Err(ApiError::bad("删除须指定已保存的组合版本"));}
    let result=s.db.run(move|c|{let tx=c.transaction()?;let Some((mut group,deleted))=load(&tx,&id)? else{return Ok(Change::Deleted)};
        if deleted{return Ok(Change::Deleted);}if group.revision!=input.expected_revision{return Ok(Change::Conflict);}
        group.revision+=1;group.updated_at=chrono::Utc::now().timestamp_millis();tx.execute("UPDATE output_groups SET data=?2,deleted=1 WHERE id=?1",params![id,serde_json::to_string(&group)?])?;tx.commit()?;Ok(Change::Deleted)
    }).await?;
    if matches!(result,Change::Conflict){return Err(conflict());}s.emit("output_groups",json!({}));Ok(StatusCode::NO_CONTENT)
}
