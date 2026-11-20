//! Versions are views over provenanced assertions. Playback IDs are never rewritten.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::Track,
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

fn conflict() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "version_revision_conflict".into(),
        "版本资料已改变，请重新读取后确认".into(),
    )
}
pub(crate) fn anchor(c: &Connection, id: &str) -> anyhow::Result<Option<String>> {
    // Manual correction overrides tags; conflicting assertions at the strongest level stay unresolved.
    let keys=c.prepare("SELECT DISTINCT a.entity_id,CASE a.basis WHEN 'manual' THEN 0 WHEN 'file_tags' THEN 1 ELSE 2 END AS priority FROM music_assertions a JOIN music_entities e ON e.id=a.entity_id WHERE a.track_id=?1 AND e.kind='recording' AND a.role='recording' AND a.basis IN('manual','file_tags','source_identity') AND (a.basis!='source_identity' OR EXISTS(SELECT 1 FROM tracks t WHERE t.id=a.track_id AND (e.external_id GLOB t.source||':*' OR (t.source='local' AND json_extract(t.provider_data,'$.cached_from.track_id')=e.external_id)))) ORDER BY priority,a.entity_id")?.query_map([id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let Some(first) = keys.first() else {
        return Ok(None);
    };
    Ok(if keys.iter().filter(|v| v.1 == first.1).count() == 1 {
        Some(first.0.clone())
    } else {
        None
    })
}
fn edition(c: &Connection, id: &str) -> anyhow::Result<Value> {
    Ok(c.query_row("SELECT revision,kind,label,notes FROM music_editions WHERE track_id=?1",[id],|r|Ok(json!({"revision":r.get::<_,i64>(0)?,"kind":r.get::<_,String>(1)?,"label":r.get::<_,String>(2)?,"notes":r.get::<_,String>(3)?}))).optional()?.unwrap_or(json!({"revision":0,"kind":"unspecified","label":"","notes":""})))
}
#[derive(Deserialize)]
#[serde(default)]
pub struct Page {pub offset:u32,pub other_offset:u32,pub limit:u32,pub q:String,pub source:String,pub sort:String,pub available:Option<bool>}
impl Default for Page {fn default()->Self{Self{offset:0,other_offset:0,limit:20,q:String::new(),source:String::new(),sort:"title".into(),available:None}}}
pub async fn list(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(page):Query<Page>)->ApiResult<Json<Value>> {
 a.require("read")?;
 if page.limit==0||page.limit>100||page.offset>10_000_000||page.other_offset>10_000_000||page.q.len()>200||page.source.len()>64||!["title","source","added"].contains(&page.sort.as_str()){return Err(ApiError::bad("版本分页或筛选无效"));}
 let permission=crate::accounts::scope::predicate("t");
 let result=s.db.read(move|c|{
  let track=c.query_row("SELECT * FROM tracks WHERE id=?1",[&id],Track::from_row).optional()?;
  let Some(track)=track else{return Ok(None)};let key=anchor(c,&id)?;
  let (tracks,total)=crate::music_version_query::page_scoped(c,&id,key.as_deref(),&page,false,&permission)?;
  let mut items=vec![];for t in tracks {items.push(json!({"quality_evidence":crate::identity_workflow::quality(&t),"identity":crate::music_identity::graph(c,&t.id)?,"edition":edition(c,&t.id)?,"same_recording":key.is_some(),"track":t}));}
  let preference=if let Some(key)=&key{c.query_row("SELECT track_id,revision FROM music_version_preferences WHERE user_id=?1 AND entity_id=?2",params![a.id,key],|r|Ok(json!({"track_id":r.get::<_,Option<String>>(0)?,"revision":r.get::<_,i64>(1)?}))).optional()?.unwrap_or(json!({"track_id":null,"revision":0}))}else{json!({"track_id":null,"revision":0})};
  let (related,other_total)=crate::music_version_query::page_scoped(c,&id,key.as_deref(),&page,true,&permission)?;
  Ok(Some(json!({"track_id":track.id,"recording_entity_id":key,"identity_resolved":key.is_some(),"items":items,"total":total,"offset":page.offset,"limit":page.limit,"preferred":preference,"other_recordings":related,"other_page":crate::music_version_query::navigation(page.other_offset,page.limit,other_total),"automatic_merge":false,"candidate_scan_limited":false,"edition":edition(c,&id)?})))
 }).await?.ok_or_else(ApiError::missing)?;Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preference {
    expected_revision: u64,
    track_id: Option<String>,
}
pub async fn prefer(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(input): Json<Preference>,
) -> ApiResult<Json<Value>> {
    a.require("control")?;
    if input.expected_revision >= i64::MAX as u64 {
        return Err(ApiError::bad("版本修订无效"));
    }
    let result=s.db.run(move|c|{let tx=c.transaction()?;let Some(key)=anchor(&tx,&id)? else{return Ok(Err(ApiError::bad("录音身份尚未唯一确认，不能设置跨版本偏好")));};
        if let Some(target)=&input.track_id {if anchor(&tx,target)?!=Some(key.clone()){return Ok(Err(ApiError::bad("目标没有相同且唯一的录音身份")));}}
        let revision=tx.query_row("SELECT revision FROM music_version_preferences WHERE user_id=?1 AND entity_id=?2",params![a.id,key],|r|r.get::<_,i64>(0)).optional()?.unwrap_or(0);
        if revision as u64!=input.expected_revision{return Ok(Err(conflict()));}
        tx.execute("INSERT INTO music_version_preferences(user_id,entity_id,track_id,revision) VALUES(?1,?2,?3,1) ON CONFLICT(user_id,entity_id) DO UPDATE SET track_id=excluded.track_id,revision=revision+1",params![a.id,key,input.track_id])?;
        tx.commit()?;Ok(Ok(json!({"revision":revision+1,"track_id":input.track_id,"applies_to":"recommendations","playback_ids_unchanged":true})))
    }).await??;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edition {
    expected_revision: u64,
    kind: String,
    label: String,
    notes: String,
}
pub async fn edit(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(input): Json<Edition>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    if input.expected_revision >= i64::MAX as u64
        || ![
            "unspecified",
            "original",
            "live",
            "remaster",
            "remix",
            "cover",
            "instrumental",
            "radio_edit",
            "other",
        ]
        .contains(&input.kind.as_str())
        || input.label.len() > 200
        || input.notes.len() > 2000
        || input.label.chars().any(char::is_control)
    {
        return Err(ApiError::bad("版本标签或备注无效"));
    }
    let result=s.db.run(move|c|{let tx=c.transaction()?;
        if !tx.query_row("SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",[&id],|r|r.get::<_,bool>(0))?{return Ok(Err(ApiError::missing()));}
        let before=edition(&tx,&id)?;if before["revision"].as_u64()!=Some(input.expected_revision){return Ok(Err(conflict()));}
        tx.execute("INSERT INTO music_editions(track_id,revision,kind,label,notes) VALUES(?1,1,?2,?3,?4) ON CONFLICT(track_id) DO UPDATE SET revision=revision+1,kind=excluded.kind,label=excluded.label,notes=excluded.notes",params![id,input.kind,input.label,input.notes])?;
        let after=edition(&tx,&id)?;tx.execute("INSERT INTO music_edition_audit(track_id,user_id,before_data,after_data,created_at) VALUES(?1,?2,?3,?4,unixepoch())",params![id,a.id,before.to_string(),after.to_string()])?;tx.commit()?;Ok(Ok(after))
    }).await??;
    s.emit("music_versions", json!({}));
    Ok(Json(result))
}

/// Preference can only reorder an existing verified candidate, never silently add an audio URL.
pub(crate) async fn order(
    s: &AppState,
    user: String,
    tracks: Vec<Track>,
) -> anyhow::Result<Vec<Track>> {
    s.db.read(move|c|{
        let mut result=vec![];let mut seen=std::collections::HashSet::new();let mut recordings=std::collections::HashSet::new();
        for mut track in tracks {
            if let Some(key)=anchor(c,&track.id)? {
                if !recordings.insert(key.clone()){continue;}
                let preferred=c.query_row("SELECT track_id FROM music_version_preferences WHERE user_id=?1 AND entity_id=?2",params![user,key],|r|r.get::<_,Option<String>>(0)).optional()?.flatten();
                if let Some(id)=preferred {if anchor(c,&id)?==Some(key){if let Some(chosen)=c.query_row("SELECT * FROM tracks WHERE id=?1 AND available=1",[id],Track::from_row).optional()?{track=chosen;}}}
            }
            if seen.insert(track.id.clone()){result.push(track);}
        }
        Ok(result)
    }).await
}
