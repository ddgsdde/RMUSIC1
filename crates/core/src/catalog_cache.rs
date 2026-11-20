//! Persistent catalog snapshots and revisioned, field-level human overrides.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
pub const FIELDS: &[&str] = &[
    "description",
    "image_url",
    "genre",
    "language",
    "label",
    "release_date",
    "birthday",
    "birthplace",
    "country",
    "alias",
    "foreign_name",
    "company",
    "instrument",
    "tag",
];
pub fn image(c: &rusqlite::Connection, profile: &mut Value) {
    if let Some(url) = profile["image_url"].as_str() {
        if let Ok(local) = crate::media_assets::register(c, url) {
            profile["image_url"] = json!(local);
        }
    }
}
pub async fn profile(s: &AppState, kind: &str, id: &str) -> ApiResult<Value> {
    let (kind, id) = (kind.to_owned(), id.to_owned());
    Ok(s.db.read(move|c|{let row=c.query_row("SELECT automatic,manual,revision,updated_at FROM catalog_profiles WHERE kind=?1 AND entity_id=?2",params![kind,id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).optional()?;
 let (mut value,manual,revision,updated)=row.map(|(a,m,r,t)|(serde_json::from_str::<Value>(&a).unwrap_or(json!({})),serde_json::from_str::<Value>(&m).unwrap_or(json!({})),r,t)).unwrap_or((json!({}),json!({}),0,0));
 let mut provenance=value.as_object_mut().and_then(|m|m.remove("_provenance")).unwrap_or(json!({}));let alternatives=value.as_object_mut().and_then(|m|m.remove("_alternatives")).unwrap_or(json!({}));
 if let Some(fields)=manual.as_object(){for(k,v)in fields{value[k]=v.clone();provenance[k]=json!("manual");}}
 Ok(json!({"profile":value,"manual":manual,"revision":revision,"updated_at":updated,"provenance":provenance,"field_sources":alternatives}))}).await?)
}
pub async fn save(s: &AppState, kind: &str, id: &str, mut profile: Value) -> ApiResult<()> {
    let kind = kind.to_owned();
    let id = id.to_owned();
    let saved=s.db.run(move|c|{
 if let Some(expected)=profile.as_object_mut().and_then(|v|v.remove("_alias_members")) {let mut actual=crate::catalog_aliases::component(c,&kind,&id)?;actual.sort();let mut expected:Vec<String>=serde_json::from_value(expected)?;expected.sort();if actual!=expected{return Ok((false,0));}}
 image(c,&mut profile);
 if let Some(rows)=profile["_alternatives"]["image_url"].as_array_mut(){for row in rows{if let Some(url)=row["value"].as_str(){if let Ok(stable)=crate::media_assets::register(c,url){row["value"]=json!(stable);}}}}

 if let Some(url)=profile["image_url"].as_str(){crate::artwork_recovery::bind(c,&kind,&id,url)?;}
 let old=c.query_row("SELECT automatic,manual FROM catalog_profiles WHERE kind=?1 AND entity_id=?2",params![kind,id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
 if let Some((old,manual))=old {let manual:Value=serde_json::from_str(&manual)?;let mut previous:Value=serde_json::from_str(&old)?;if let Some(fields)=profile.as_object(){for(k,v)in fields{if !v.is_null() && !v.as_str().is_some_and(str::is_empty) && !v.as_array().is_some_and(Vec::is_empty) && !v.as_object().is_some_and(serde_json::Map::is_empty) && !(kind=="track" && k=="image_url" && manual.get(k)==Some(v)){if ["_provenance","_alternatives"].contains(&k.as_str()){if let Some(fields)=v.as_object(){for(f,v)in fields{previous[k][f]=v.clone();}}}else{previous[k]=v.clone();}}}}profile=previous;}
 c.execute("INSERT INTO catalog_profiles(kind,entity_id,automatic)VALUES(?1,?2,?3) ON CONFLICT(kind,entity_id)DO UPDATE SET automatic=excluded.automatic,updated_at=unixepoch() WHERE catalog_profiles.automatic<>excluded.automatic",params![kind,id,profile.to_string()])?;let changed=if kind=="album"{crate::album_artwork::refresh(c,&id)?}else{0};Ok((true,changed))}).await?;
    if !saved.0 {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "catalog_alias_conflict".into(),
            "资料关联已变化，请刷新".into(),
        ));
    }
    if saved.1>0 {s.refresh_queue_metadata().await?;s.emit("library",json!({"reason":"album_artwork"}));}
    Ok(())
}
#[derive(Deserialize)]
pub struct Edit {
    pub expected_revision: i64,
    pub fields: Value,
    #[serde(default)]
    pub unlock: Vec<String>,
}
pub async fn edit(
    State(s): State<AppState>,
    a: Auth,
    Path((kind, id)): Path<(String, String)>,
    Json(mut edit): Json<Edit>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    if !["album", "artist", "track"].contains(&kind.as_str())
        || id.len() > 512
        || edit.expected_revision < 0
        || edit.expected_revision == i64::MAX
    {
        return Err(ApiError::bad("资料身份或修订号无效"));
    }
    let target = id.clone();
    let category = kind.clone();
    let exists =
        s.db.read(move |c| {
            let condition = if category == "track" {
                "t.id=?1".into()
            } else {
                format!("t.id IN ({})", crate::catalog_detail::direct_ids(&category))
            };
            Ok(c.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM tracks t WHERE {condition})"),
                [target],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?;
    if !exists {
        return Err(ApiError::missing());
    }
    let fields = edit
        .fields
        .as_object()
        .ok_or_else(|| ApiError::bad("资料须为字段对象"))?;
    for (k, v) in fields {
        if !FIELDS.contains(&k.as_str()) || v.as_str().is_none_or(|v| v.len() > 80000) {
            return Err(ApiError::bad("资料字段无效或过长"));
        }
        if k == "image_url" {
            let url = v.as_str().unwrap();
            if !url.is_empty()
                && !url.starts_with("https://")
                && !["/api/v1/covers/", "/api/v1/artwork/"]
                    .iter()
                    .any(|prefix| {
                        url.strip_prefix(prefix).is_some_and(|key| {
                            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
                        })
                    })
            {
                return Err(ApiError::bad("封面必须使用HTTPS或Core图片地址"));
            }
        }
    }
    if edit.unlock.len() > FIELDS.len() || edit.unlock.iter().any(|k| !FIELDS.contains(&k.as_str()))
    {
        return Err(ApiError::bad("解除保护字段无效"));
    }
    let key = id.clone();
    let k = kind.clone();
    let result=s.db.run(move|c|{let tx=c.transaction()?;
 let row=tx.query_row("SELECT automatic,manual,revision FROM catalog_profiles WHERE kind=?1 AND entity_id=?2",params![k,key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?))).optional()?.unwrap_or(("{}".into(),"{}".into(),0));if row.2!=edit.expected_revision{return Ok(false);}
 let mut automatic:Value=serde_json::from_str(&row.0)?;
 let mut manual:Value=serde_json::from_str(&row.1)?;
 let cover_changed=k=="track" && (edit.fields.get("image_url").is_some() || edit.unlock.iter().any(|f|f=="image_url"));
 if cover_changed && automatic.get("image_url").is_none() && manual.get("image_url").is_none(){
  let old=tx.query_row("SELECT cover_url FROM tracks WHERE id=?1",[&key],|r|r.get::<_,Option<String>>(0))?;automatic["image_url"]=json!(old.unwrap_or_default());
 }
 image(&tx,&mut edit.fields);for(field,value)in edit.fields.as_object().unwrap(){manual[field]=value.clone();}for key in edit.unlock{manual.as_object_mut().unwrap().remove(&key);}
 tx.execute("INSERT INTO catalog_profiles(kind,entity_id,automatic,manual,revision)VALUES(?1,?2,?3,?4,1) ON CONFLICT(kind,entity_id)DO UPDATE SET automatic=excluded.automatic,manual=excluded.manual,revision=revision+1,updated_at=unixepoch()",params![k,key,automatic.to_string(),manual.to_string()])?;
 if cover_changed {let cover=manual.get("image_url").or_else(||automatic.get("image_url")).and_then(Value::as_str).filter(|s|!s.is_empty());tx.execute("UPDATE tracks SET cover_url=?1 WHERE id=?2",params![cover,key])?;}if k=="album"{crate::album_artwork::refresh(&tx,&key)?;}else if k=="track"&&cover_changed{let track=tx.query_row("SELECT * FROM tracks WHERE id=?1",[&key],crate::models::Track::from_row)?;crate::cached_artwork::refresh(&tx,&track)?;}tx.commit()?;Ok(true)}).await?;
    if !result {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "metadata_conflict".into(),
            "资料已在其他设备修改，请重新载入后再保存".into(),
        ));
    }
    if kind == "track" || kind == "album" {
        s.refresh_queue_metadata().await?;
    }
    s.emit("library", json!({"reason":"catalog_metadata"}));
    Ok(Json(profile(&s, &kind, &id).await?))
}
pub async fn get(
    State(s): State<AppState>,
    a: Auth,
    Path((kind, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    if !["album", "artist", "track"].contains(&kind.as_str()) || id.len() > 512 {
        return Err(ApiError::bad("资料身份无效"));
    }
    Ok(Json(profile(&s, &kind, &id).await?))
}
pub async fn load_page(s: &AppState, key: String, page: u32) -> ApiResult<Option<Value>> {
    Ok(s.db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT data FROM catalog_pages WHERE cache_key=?1 AND page=?2",
                params![key, page],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| serde_json::from_str(&v).ok()))
        })
        .await?)
}
/// Snapshots remain persistent; age only controls a lightweight metadata update.
pub async fn page_fresh(s: &AppState, key: String, page: u32) -> ApiResult<bool> {
    Ok(s.db
        .read(move |c| {
            Ok(c.query_row(
        "SELECT updated_at>=unixepoch()-86400 FROM catalog_pages WHERE cache_key=?1 AND page=?2",
        params![key, page], |r| r.get::<_,bool>(0)
    ).optional()?.unwrap_or(false))
        })
        .await?)
}
pub async fn save_page(
    s: &AppState,
    key: String,
    page: u32,
    value: &Value,
    refresh: bool,
) -> ApiResult<()> {
    let data = value.to_string();
    s.db.run(move|c|{let tx=c.transaction()?;if refresh{tx.execute("DELETE FROM catalog_pages WHERE cache_key=?1",[&key])?;}tx.execute("INSERT INTO catalog_pages(cache_key,page,data)VALUES(?1,?2,?3) ON CONFLICT(cache_key,page)DO UPDATE SET data=excluded.data,updated_at=unixepoch()",params![key,page,data])?;tx.commit()?;Ok(())}).await?;
    Ok(())
}
