//! Explicit, audited entity aliases. Only human-confirmed edges connect provider catalogs.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
pub fn component(c: &Connection, kind: &str, id: &str) -> anyhow::Result<Vec<String>> {
    let rows=c.prepare("WITH RECURSIVE linked(id) AS(SELECT ?2 UNION SELECT CASE WHEN a.left_id=l.id THEN a.right_id ELSE a.left_id END FROM catalog_aliases a JOIN linked l ON a.left_id=l.id OR a.right_id=l.id WHERE a.kind=?1) SELECT id FROM linked LIMIT 33")?.query_map(params![kind,id],|r|r.get(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    anyhow::ensure!(rows.len() <= 32, "实体关联超过32个，请先整理关联");
    Ok(rows)
}
fn card(c: &Connection, kind: &str, id: &str) -> anyhow::Result<Option<Value>> {
    let permission=crate::accounts::scope::predicate("t");
    let sql = if kind == "artist" {
        "SELECT a.name,t.source FROM track_artists a JOIN tracks t ON t.id=a.track_id WHERE a.artist_id=?1 AND {permission} LIMIT 1"
    } else {
        "SELECT t.album,t.source FROM tracks t WHERE (t.album_id=?1 OR (substr(?1,1,8)='release:' AND t.source||':'||COALESCE(json_extract(t.provider_data,'$.release_id'),json_extract(t.provider_data,'$.album_mid'))=substr(?1,9))) AND {permission} LIMIT 1"
    };
    let row = c
        .query_row(&sql.replace("{permission}",&permission), [id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .optional()?;
    Ok(row.map(|(name,source)|json!({"id":id,"name":name,"source":source,"basis":"manual_confirmation_required"})))
}
pub fn direct(kind: &str) -> String {
    let linked=format!("WITH RECURSIVE linked(entity_id) AS (SELECT ?1 UNION SELECT CASE WHEN a.left_id=l.entity_id THEN a.right_id ELSE a.left_id END FROM catalog_aliases a JOIN linked l ON a.left_id=l.entity_id OR a.right_id=l.entity_id WHERE a.kind='{kind}')");
    if kind == "artist" {
        return format!("{linked} SELECT track_id FROM track_artists WHERE artist_id IN(SELECT entity_id FROM linked)");
    }
    format!("{linked}, direct(id) AS (SELECT t.id FROM tracks t JOIN linked l ON t.album_id=l.entity_id UNION SELECT a.track_id FROM music_assertions a JOIN music_entities e ON e.id=a.entity_id JOIN linked l ON substr(l.entity_id,1,8)='release:' AND e.namespace||':'||e.external_id=substr(l.entity_id,9) WHERE e.kind='release' AND a.role='release' AND a.basis='source_identity') SELECT id FROM direct UNION SELECT copy.id FROM tracks origin JOIN tracks copy ON copy.recording_id=origin.recording_id AND copy.source='local' AND origin.recording_id<>'' AND json_extract(copy.provider_data,'$.cached_from.track_id')=origin.id AND json_extract(copy.provider_data,'$.cached_from.source')=origin.source WHERE origin.id IN (SELECT id FROM direct) UNION SELECT origin.id FROM tracks copy JOIN tracks origin ON origin.id=json_extract(copy.provider_data,'$.cached_from.track_id') AND origin.source=json_extract(copy.provider_data,'$.cached_from.source') AND origin.recording_id=copy.recording_id AND origin.recording_id<>'' WHERE copy.source='local' AND copy.id IN (SELECT id FROM direct)")
}
#[derive(Default, Deserialize)]
pub struct Search {
    #[serde(default)]
    q: String,
}
fn validate(kind: &str, id: &str) -> ApiResult<()> {
    if !["artist", "album"].contains(&kind) || id.is_empty() || id.len() > 256 {
        return Err(ApiError::bad("资料实体无效"));
    }
    Ok(())
}
pub async fn get(
    State(s): State<AppState>,
    a: Auth,
    Path((kind, id)): Path<(String, String)>,
    Query(q): Query<Search>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    validate(&kind, &id)?;
    if q.q.len() > 200 {
        return Err(ApiError::bad("搜索过长"));
    }
    Ok(Json(s.db.read(move|c|{let root=card(c,&kind,&id)?;let ids=component(c,&kind,&id)?;let mut linked=ids.iter().filter_map(|id|card(c,&kind,id).transpose()).collect::<anyhow::Result<Vec<_>>>()?;for item in &mut linked{let other=item["id"].as_str().unwrap_or("");item["direct"]=json!(c.query_row("SELECT EXISTS(SELECT 1 FROM catalog_aliases WHERE kind=?1 AND ((left_id=?2 AND right_id=?3) OR (right_id=?2 AND left_id=?3)))",params![kind,id,other],|r|r.get::<_,bool>(0))?);}
 let sql=if kind=="artist"{"SELECT DISTINCT artist_id,name FROM track_artists WHERE instr(lower(name),lower(?1))>0 ORDER BY name COLLATE NOCASE,artist_id LIMIT 20"}else{"SELECT DISTINCT CASE WHEN COALESCE(json_extract(provider_data,'$.release_id'),json_extract(provider_data,'$.album_mid')) IS NOT NULL THEN 'release:'||source||':'||COALESCE(json_extract(provider_data,'$.release_id'),json_extract(provider_data,'$.album_mid')) ELSE album_id END,album FROM tracks WHERE trim(album)<>'' AND instr(lower(album),lower(?1))>0 ORDER BY album COLLATE NOCASE LIMIT 20"};
 let candidates=if q.q.trim().is_empty(){vec![]}else{c.prepare(sql)?.query_map([q.q],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?.iter().filter(|id|!ids.contains(id)).filter_map(|id|card(c,&kind,id).transpose()).collect::<anyhow::Result<Vec<_>>>()?};
 let revision=c.query_row("SELECT COALESCE(MAX(id),0) FROM catalog_alias_audit WHERE kind=?1",[kind],|r|r.get::<_,i64>(0))?;
 Ok(json!({"entity":root,"items":linked,"candidates":candidates,"revision":revision,"automatic_merge":false}))}).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    other_id: String,
    linked: bool,
    expected_revision: u64,
    #[serde(default)]
    preview_only: bool,
}
pub async fn put(
    State(s): State<AppState>,
    a: Auth,
    Path((kind, id)): Path<(String, String)>,
    Json(q): Json<Edit>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    validate(&kind, &id)?;
    validate(&kind, &q.other_id)?;
    if id == q.other_id {
        return Err(ApiError::bad("不能关联自身"));
    }
    let value=s.db.run(move|c|{let tx=c.transaction()?;let revision=tx.query_row("SELECT COALESCE(MAX(id),0) FROM catalog_alias_audit WHERE kind=?1",[&kind],|r|r.get::<_,i64>(0).map(|v|v as u64))?;
 if revision!=q.expected_revision{return Ok(Err(ApiError(StatusCode::CONFLICT,"catalog_alias_conflict".into(),"资料关联已变化，请重新读取".into())));}
 let (Some(left),Some(right))=(card(&tx,&kind,&id)?,card(&tx,&kind,&q.other_id)?)else{return Ok(Err(ApiError::missing()));};
 let before_left=component(&tx,&kind,&id)?;let before_right=component(&tx,&kind,&q.other_id)?;
 let mut affected=before_left.clone();affected.extend(before_right.clone());affected.sort();affected.dedup();
 if q.linked { for node in &affected { if component(&tx,&kind,node)?.len()==1 {tx.execute("INSERT INTO catalog_alias_profile_baselines(kind,entity_id,data) VALUES(?1,?2,COALESCE((SELECT automatic FROM catalog_profiles WHERE kind=?1 AND entity_id=?2),'{}')) ON CONFLICT(kind,entity_id) DO UPDATE SET data=excluded.data",params![kind,node])?;} } }
 let (lo,hi)=if id<q.other_id{(id.clone(),q.other_id.clone())}else{(q.other_id.clone(),id.clone())};
 if q.linked{tx.execute("INSERT OR IGNORE INTO catalog_aliases(kind,left_id,right_id,user_id,created_at)VALUES(?1,?2,?3,?4,unixepoch())",params![kind,lo,hi,a.id])?;}else{tx.execute("DELETE FROM catalog_aliases WHERE kind=?1 AND left_id=?2 AND right_id=?3",params![kind,lo,hi])?;}
 let members=match component(&tx,&kind,&id){Ok(v)=>v,Err(_)=>return Ok(Err(ApiError::bad("实体关联不能超过32个，请先整理已有关系")))};
 if q.preview_only{tx.rollback()?;return Ok(Ok(json!({"left":left,"right":right,"members":members,"preview_only":true,"files_unchanged":true,"revision":revision})));}
 if !q.linked {for node in &affected {tx.execute("UPDATE catalog_profiles SET automatic=COALESCE((SELECT data FROM catalog_alias_profile_baselines b WHERE b.kind=catalog_profiles.kind AND b.entity_id=catalog_profiles.entity_id),automatic),updated_at=unixepoch() WHERE kind=?1 AND entity_id=?2",params![kind,node])?;
 // The source binding must move with restored metadata, including an absent image.
 let image=tx.query_row("SELECT CASE WHEN json_type(manual,'$.image_url') IS NOT NULL THEN json_extract(manual,'$.image_url') ELSE json_extract(automatic,'$.image_url') END FROM catalog_profiles WHERE kind=?1 AND entity_id=?2",params![kind,node],|r|r.get::<_,Option<String>>(0)).optional()?.flatten();
 tx.execute("DELETE FROM artwork_entities WHERE kind=?1 AND entity_id=?2",params![kind,node])?;
 if let Some(url)=image {crate::artwork_recovery::bind(&tx,&kind,node,&url)?;}
 }}
 tx.execute("INSERT INTO catalog_alias_audit(kind,left_id,right_id,linked,user_id,created_at)VALUES(?1,?2,?3,?4,?5,unixepoch())",params![kind,lo,hi,q.linked,a.id])?;let revision=tx.last_insert_rowid();tx.commit()?;Ok(Ok(json!({"revision":revision,"files_unchanged":true,"automatic_merge":false})))
 }).await??;
    if value["preview_only"] != true {
        s.emit("library", json!({"reason":"catalog_alias"}));
    }
    Ok(Json(value))
}
