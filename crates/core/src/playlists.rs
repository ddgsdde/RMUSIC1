use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{extract::{Path,State,Query},http::StatusCode,Json};
use rusqlite::{params,OptionalExtension};
use serde::Deserialize;
use serde_json::{json,Value};

use crate::smart_rules::Rule as SmartQuery;
fn validate(mut v:Value)->ApiResult<Value> {
    if !v.is_object(){return Err(ApiError::bad("播放列表参数必须为对象"));}
    if let Some(name)=v.get("name") {if name.as_str().is_none_or(|s|s.trim().is_empty()||s.len()>100){return Err(ApiError::bad("播放列表名称无效"));}}
    if let Some(tracks)=v.get("track_ids") {if tracks.as_array().is_none_or(|a|a.len()>10000||a.iter().any(|id|!id.is_string())){return Err(ApiError::bad("播放列表曲目无效"));}}
    if let Some(expected)=v.get("expected") {if expected.as_object().is_none_or(|o|o.len()!=2)||expected["name"].as_str().is_none()||expected["track_ids"].as_array().is_none_or(|a|a.len()>10000||a.iter().any(|id|!id.is_string())) {return Err(ApiError::bad("歌单编辑快照无效"));}}
    if let Some(query)=v.get("smart_query") {
        if v.get("track_ids").is_some(){return Err(ApiError::bad("曲目列表与智能规则只能指定一种"));}
        let rule:SmartQuery=serde_json::from_value(query.clone()).map_err(|_|ApiError::bad("智能规则包含不支持的字段或类型"))?;
        rule.validate()?;
        v["smart_query"]=serde_json::to_value(rule).unwrap();
    }
    Ok(v)
}
fn row(r:&rusqlite::Row<'_>)->rusqlite::Result<(String,String,String,Option<String>)> {Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))}
fn record((id,name,tracks,query):(String,String,String,Option<String>))->anyhow::Result<Value> {
    let mut item=json!({"id":id,"name":name,"track_ids":serde_json::from_str::<Value>(&tracks).unwrap_or(json!([]))});
    if let Some(query)=query{item["smart_query"]=serde_json::to_value(serde_json::from_str::<SmartQuery>(&query)?)?;}
    Ok(item)
}
pub(crate) fn resolve(c:&rusqlite::Connection,item:&mut Value)->anyhow::Result<()> {
    if let Some(query)=item.get("smart_query") {
        let rule:SmartQuery=serde_json::from_value(query.clone())?;
        let user:String=c.query_row("SELECT user_id FROM playlists WHERE id=?1",[item["id"].as_str().unwrap_or("")],|r|r.get(0))?;
        let(ids,truncated)=rule.ids(c,&user)?;item["track_ids"]=json!(ids);item["truncated"]=json!(truncated);
    }
    if item["id"].as_str().is_some_and(|id|id.starts_with("favorites-")){crate::cache_identity::favorites(c,item)?;}
    Ok(())
}
pub(crate) fn protected_tracks(c:&rusqlite::Connection)->anyhow::Result<std::collections::HashSet<String>> {
    let rows=c.prepare("SELECT id,name,tracks,smart_query FROM playlists")?.query_map([],row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut protected=std::collections::HashSet::new();
    for row in rows {let mut item=record(row)?;resolve(c,&mut item)?;protected.extend(item["track_ids"].as_array().into_iter().flatten().filter_map(|id|id.as_str().map(String::from)));}
    let cached=c.prepare("SELECT id,json_extract(provider_data,'$.cached_from.track_id') FROM tracks WHERE source='local' AND json_type(provider_data,'$.cached_from.track_id')='text'")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (local,original) in cached{if protected.contains(&local){protected.insert(original);}}
    Ok(protected)
}
pub async fn list(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>> {
    a.require("read")?;
    let items=s.db.read(move|c|{let rows=c.prepare("SELECT id,name,tracks,smart_query FROM playlists WHERE user_id=?1 ORDER BY name")?.query_map([a.id],row)?.collect::<rusqlite::Result<Vec<_>>>()?;rows.into_iter().map(|row|{let mut item=record(row)?;if let Some(sync)=crate::playlist_sync::info(c,item["id"].as_str().unwrap())?{item["sync"]=sync;}Ok(item)}).collect::<anyhow::Result<Vec<_>>>()}).await?;
    let items=s.db.read(move|c|{let mut items=items;for item in &mut items {
        if item["id"].as_str().is_some_and(|id|id.starts_with("favorites-")){crate::cache_identity::favorites(c,item)?;}
        let cover:Option<String>=c.query_row("SELECT cover_url FROM tracks WHERE id IN(SELECT value FROM json_each(?1) LIMIT 8) AND cover_url IS NOT NULL AND cover_url<>'' LIMIT 1",[item["track_ids"].to_string()],|r|r.get(0)).optional()?;
        item["cover_url"]=json!(cover);
        if item["cover_url"].is_null(){if let Some(sync)=item.get("sync"){let cover:Option<String>=c.query_row("SELECT json_extract(data,'$.cover_url') FROM provider_playlist_directory WHERE provider=?1 AND account=?2 AND remote_id=?3 LIMIT 1",params![sync["provider"].as_str(),sync["account_context"].as_str(),sync["playlist_id"].as_str()],|r|r.get(0)).optional()?.flatten();item["cover_url"]=json!(cover);}}
    }Ok(items)}).await?;
    Ok(Json(json!({"items":items})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackChange {track_id:String,#[serde(default)]remove:bool}
fn change_track(c:&mut rusqlite::Connection,user:&str,id:&str,mut change:TrackChange)->anyhow::Result<Value>{
    let tx=c.transaction()?;
    let stored=tx.query_row("SELECT id,name,tracks,smart_query FROM playlists WHERE id=?1 AND user_id=?2",params![id,user],row).optional()?.ok_or_else(||anyhow::anyhow!("歌单不存在"))?;
    let mut item=record(stored)?;if item.get("smart_query").is_some(){anyhow::bail!("智能歌单请修改筛选规则");}
    if !tx.query_row("SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",[&change.track_id],|r|r.get::<_,bool>(0))?{anyhow::bail!("歌曲尚未导入资料库");}
    let mut ids:Vec<String>=serde_json::from_value(item["track_ids"].clone())?;
    if id==format!("favorites-{}",crate::auth::hash(user)) {
        ids=crate::cache_identity::unique(&tx,ids,false)?;
        change.track_id=crate::cache_identity::canonical(&tx,&change.track_id)?;
    }
    if change.remove{ids.retain(|value|value!=&change.track_id);}else if !ids.contains(&change.track_id){if ids.len()>=10000{anyhow::bail!("歌单最多一万首歌曲");}ids.push(change.track_id.clone());}
    item["track_ids"]=json!(ids);tx.execute("UPDATE playlists SET tracks=?2 WHERE id=?1",params![id,item["track_ids"].to_string()])?;
    let mut operation=item.clone();operation["track_change"]=json!({"track_id":change.track_id,"remove":change.remove});
    crate::playlist_sync::changed(&tx,id,&operation)?;tx.commit()?;Ok(item)
}
pub async fn append(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(change):Json<TrackChange>)->ApiResult<Json<Value>>{
    a.require("control")?;if change.track_id.len()>256{return Err(ApiError::bad("歌曲编号过长"));}
    let item=s.db.run(move|c|change_track(c,&a.id,&id,change)).await?;s.emit("playlists",json!({}));Ok(Json(item))
}
pub async fn favorites(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>>{
    a.require("read")?;let id=format!("favorites-{}",crate::auth::hash(&a.id));
    let value=s.db.read(move|c|{let mut value=c.query_row("SELECT id,name,tracks,smart_query FROM playlists WHERE id=?1 AND user_id=?2",params![id,a.id],row).optional()?.map(record).transpose()?.unwrap_or(json!({"id":id,"name":"我喜欢","track_ids":[]}));crate::cache_identity::favorites(c,&mut value)?;if let Some(sync)=crate::playlist_sync::info(c,&id)?{value["sync"]=sync;}Ok(value)}).await?;Ok(Json(value))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub struct FavoriteChange {liked:bool}
pub async fn favorite(State(s):State<AppState>,a:Auth,Path(track_id):Path<String>,Json(change):Json<FavoriteChange>)->ApiResult<Json<Value>>{
    a.require("control")?;if track_id.len()>256{return Err(ApiError::bad("歌曲编号过长"));}let id=format!("favorites-{}",crate::auth::hash(&a.id));
    let value=s.db.run(move|c|{c.execute("INSERT OR IGNORE INTO playlists(id,name,user_id,tracks) VALUES(?1,'我喜欢',?2,'[]')",params![id,a.id])?;let mut value=change_track(c,&a.id,&id,TrackChange{track_id,remove:!change.liked})?;crate::cache_identity::favorites(c,&mut value)?;if let Some(sync)=crate::playlist_sync::info(c,&id)?{value["sync"]=sync;}Ok(value)}).await?;s.emit("playlists",json!({}));Ok(Json(value))
}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("read")?;
    let item=s.db.run(move|c|{let Some(row)=c.query_row("SELECT id,name,tracks,smart_query FROM playlists WHERE id=?1 AND user_id=?2",params![id,a.id],row).optional()? else{return Ok(None);};let mut item=record(row)?;resolve(c,&mut item)?;if let Some(sync)=crate::playlist_sync::info(c,item["id"].as_str().unwrap())?{item["sync"]=sync;}Ok(Some(item))}).await?;
    Ok(Json(item.ok_or_else(ApiError::missing)?))
}
#[derive(Deserialize)]pub struct TracksPage {limit:Option<usize>,offset:Option<usize>,sort:Option<String>,view:Option<String>}
pub async fn tracks(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(page):Query<TracksPage>)->ApiResult<Json<Value>> {
    a.require("read")?;let limit=page.limit.unwrap_or(50);let offset=page.offset.unwrap_or(0);if limit==0||limit>100{return Err(ApiError::bad("每页数量须为 1–100"));}
    let predicate=crate::accounts::scope::predicate("t");
    let order=page.sort.as_deref().filter(|sort|*sort!="default").map(|sort|crate::library_query::order(Some(sort),false,"CAST(j.key AS INTEGER)")).transpose()?;
    let compact=page.view.as_deref()==Some("compact");
    let value=s.db.read(move|c|{let row=c.query_row("SELECT id,name,tracks,smart_query FROM playlists WHERE id=?1 AND user_id=?2",params![id,a.id],row).optional()?;let Some(row)=row else{return Ok(None);};let mut item=record(row)?;
        let favorite=id.starts_with("favorites-");
        if !favorite{resolve(c,&mut item)?;}
        let mut ids:Vec<String>=serde_json::from_value(item["track_ids"].clone())?;
        if favorite{ids=crate::cache_identity::unique(c,ids,true)?;ids.reverse();}
        if let Some(order)=order {
            let ids=json!(ids).to_string();
            let total=c.query_row(&format!("SELECT COUNT(*) FROM json_each(?1) j JOIN tracks t ON t.id=j.value WHERE {predicate}"),[&ids],|r|r.get::<_,i64>(0))?;
            let items=c.prepare(&format!("SELECT t.* FROM json_each(?1) j JOIN tracks t ON t.id=j.value WHERE {predicate} ORDER BY {order},CAST(j.key AS INTEGER) LIMIT ?2 OFFSET ?3"))?.query_map(params![ids,limit as i64,offset as i64],crate::models::Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            let items=if compact{json!(items.iter().map(crate::zones::snapshots::track).collect::<Vec<_>>())}else{json!(items)};
            return Ok(Some(json!({"items":items,"total":total,"limit":limit,"offset":offset,"missing_ids":[]})));
        }
        let mut items=vec![];let mut missing=vec![];for id in ids.iter().skip(offset).take(limit){if let Some(track)=c.query_row(&format!("SELECT t.* FROM tracks t WHERE id=?1 AND {predicate}"),[id],crate::models::Track::from_row).optional()?{items.push(track);}else{missing.push(id);}}
        let items=if compact{json!(items.iter().map(crate::zones::snapshots::track).collect::<Vec<_>>())}else{json!(items)};
        Ok(Some(json!({"items":items,"total":ids.len(),"limit":limit,"offset":offset,"missing_ids":missing})))
    }).await?.ok_or_else(ApiError::missing)?;Ok(Json(value))
}
pub async fn create(State(s):State<AppState>,a:Auth,Json(v):Json<Value>)->ApiResult<Json<Value>> {
    a.require("control")?;let v=validate(v)?;
    let name=v["name"].as_str().ok_or_else(||ApiError::bad("缺少播放列表名称"))?.to_owned();
    let query=v.get("smart_query").map(Value::to_string);
    let tracks=if query.is_some(){json!([])}else{v.get("track_ids").cloned().ok_or_else(||ApiError::bad("缺少曲目或智能规则"))?};
    let id=uuid::Uuid::new_v4().to_string();let pid=id.clone();
    s.db.run(move|c|{c.execute("INSERT INTO playlists(id,name,user_id,tracks,smart_query) VALUES(?1,?2,?3,?4,?5)",params![pid,name,a.id,tracks.to_string(),query])?;Ok(())}).await?;
    Ok(Json(json!({"id":id})))
}
pub async fn update(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Value>)->ApiResult<Json<Value>> {
    a.require("control")?;let v=validate(v)?;
    let item=s.db.run(move|c|{let tx=c.transaction()?;
        let Some(stored)=tx.query_row("SELECT id,name,tracks,smart_query FROM playlists WHERE id=?1 AND user_id=?2",params![id,a.id],row).optional()? else{return Ok(Err(ApiError::missing()));};
        let mut item=record(stored)?;let smart=item.get("smart_query").is_some();
        if let Some(expected)=v.get("expected") {if item["name"]!=expected["name"]||item["track_ids"]!=expected["track_ids"] {return Ok(Err(ApiError(StatusCode::CONFLICT,"playlist_edit_conflict".into(),"歌单已在其他位置修改，请读取最新歌单后重新编辑".into())));}}
        if (smart&&v.get("track_ids").is_some())||(!smart&&v.get("smart_query").is_some()){return Ok(Err(ApiError::bad("不能直接切换手动／智能歌单类型，请新建歌单")));}
        for key in ["name","track_ids","smart_query"] {if let Some(value)=v.get(key){item[key]=value.clone();}}
        if id.starts_with("favorites-"){item["name"]=json!("我喜欢");}
        tx.execute("UPDATE playlists SET name=?1,tracks=?2,smart_query=?3 WHERE id=?4 AND user_id=?5",params![item["name"].as_str().unwrap(),item["track_ids"].to_string(),item.get("smart_query").map(Value::to_string),id,a.id])?;
        crate::playlist_sync::changed(&tx,&id,&item)?;tx.commit()?;
        Ok(Ok(item))
    }).await??;
    Ok(Json(item))
}
pub async fn delete(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<StatusCode> {
    a.require("control")?;s.db.run(move|c|{let tx=c.transaction()?;if tx.query_row("SELECT EXISTS(SELECT 1 FROM playlists WHERE id=?1 AND user_id=?2)",params![id,a.id],|r|r.get::<_,bool>(0))?{tx.execute("UPDATE jobs SET status='cancelled' WHERE kind='playlist_sync' AND cursor=?1",[&id])?;tx.execute("DELETE FROM playlists WHERE id=?1 AND user_id=?2",params![id,a.id])?;}tx.commit()?;Ok(())}).await?;Ok(StatusCode::NO_CONTENT)
}
