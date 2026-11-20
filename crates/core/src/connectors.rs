//! Provider-independent capability and configuration access.
use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{extract::{Path,State},Json};
use rusqlite::params;
use serde_json::{json,Value};

pub fn valid_id(id:&str)->bool {!id.is_empty()&&id.len()<=64&&id.bytes().all(|c|c.is_ascii_alphanumeric()||b"_-".contains(&c))}
pub async fn manifest(s:&AppState,id:&str)->ApiResult<Value> {
    if !valid_id(id){return Err(ApiError::bad("音源标识无效"));}
    let path=s.config.plugin_dir.join(id).join("manifest.json");
    if tokio::fs::metadata(&path).await.map_err(|_|ApiError::missing())?.len()>64*1024{return Err(ApiError::bad("音源清单过大"));}
    let value:Value=serde_json::from_slice(&tokio::fs::read(path).await.map_err(anyhow::Error::from)?).map_err(anyhow::Error::from)?;
    if value["id"]!=id{return Err(ApiError::bad("音源清单身份不匹配"));}Ok(value)
}
pub async fn providers(s:&AppState,capability:&str)->Vec<String> {
    let Ok(mut dirs)=tokio::fs::read_dir(&s.config.plugin_dir).await else{return vec![];};let mut result=vec![];
    while let Ok(Some(dir))=dirs.next_entry().await {let id=dir.file_name().to_string_lossy().to_string();if let Ok(value)=manifest(s,&id).await {if value["capabilities"].as_array().is_some_and(|c|c.iter().any(|v|v==capability))&&crate::plugin::check_enabled(s,&id).await.is_ok(){result.push(id);}}}
    result.sort();result
}
pub async fn ordered_metadata_providers(s:&AppState)->Vec<String> {
 let enabled=providers(s,"metadata").await;let priority=s.setting("metadata_provider_priority",json!([])).await;
 let mut result=vec![];
 if let Some(ids)=priority.as_array(){for id in ids.iter().filter_map(Value::as_str){if enabled.iter().any(|p|p==id)&&!result.iter().any(|p|p==id){result.push(id.to_owned());}}}
 let legacy=s.setting("metadata_provider",Value::Null).await;
 if let Some(id)=legacy.as_str(){if enabled.iter().any(|p|p==id)&&!result.iter().any(|p|p==id){result.push(id.into());}}
 for id in enabled{if !result.contains(&id){result.push(id);}}result
}
pub async fn metadata_provider(s:&AppState,source:&str)->ApiResult<String> {
    let providers=ordered_metadata_providers(s).await;
    if providers.iter().any(|id|id==source){return Ok(source.into());}
    providers.first().cloned().ok_or_else(||ApiError::bad("尚无已启用的资料连接器"))
}
pub async fn quality(s:&AppState,id:&str)->String {
    let configured=s.setting(&format!("provider:{id}:quality"),Value::Null).await;
    if let Some(value)=configured.as_str(){return if id=="qqmusic"&&matches!(value,"best"|"MASTER"){"native".into()}else{value.into()};}
    // Compatibility migration only: the former global setting belonged to QQ Music.
    if id=="qqmusic" {let old=s.setting("stream_quality",json!("native")).await;if let Some(value)=old.as_str(){return if matches!(value,"best"|"MASTER"){"native".into()}else{value.into()};}}
    manifest(s,id).await.ok().and_then(|m|m["quality_options"].as_array()?.first()?.get("id")?.as_str().map(String::from)).unwrap_or_else(||"best".into())
}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("read")?;let manifest=manifest(&s,&id).await?;
    Ok(Json(json!({"quality":quality(&s,&id).await,"quality_options":manifest["quality_options"],"provider":id})))
}
pub async fn patch(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(value):Json<Value>)->ApiResult<Json<Value>> {
    a.require("admin")?;let descriptor=manifest(&s,&id).await?;
    if value.as_object().is_none_or(|v|v.len()!=1)||value["quality"].as_str().is_none_or(|q|!valid_id(q)||!descriptor["quality_options"].as_array().is_some_and(|items|items.iter().any(|item|item["id"]==q))){return Err(ApiError::bad("此音源未声明该音质选项"));}
    let key=format!("provider:{id}:quality");let quality=value["quality"].clone();let provider=id.clone();s.db.run(move|c|{let tx=c.transaction()?;
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,quality.to_string()])?;
        if provider=="qqmusic"{tx.execute("INSERT INTO settings(key,value) VALUES('stream_quality',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[quality.to_string()])?;}
        tx.execute("DELETE FROM jobs WHERE kind='loudness' AND status IN ('completed','unable','failed') AND cursor IN(SELECT id FROM tracks WHERE source=?1)",[&provider])?;
        tx.execute("UPDATE tracks SET provider_data=json_remove(provider_data,'$.integrated_lufs','$.analysis.loudness','$.provenance.integrated_lufs') WHERE source=?1 AND json_extract(provider_data,'$.provenance.integrated_lufs.source')='ffmpeg_loudnorm_ebu_r128' AND COALESCE(json_extract(provider_data,'$.integrated_lufs_source'),'')!='manual' AND json_type(provider_data,'$.manual_corrections.integrated_lufs') IS NULL",[provider])?;
        tx.commit()?;Ok(())}).await?;
    s.refresh_queue_metadata().await?;
    Ok(Json(json!({"saved":true,"apply":"next_playback"})))
}
