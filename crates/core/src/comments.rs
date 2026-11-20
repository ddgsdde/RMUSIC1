//! Read-only, capability-based song discussion access. No upstream write RPC is exposed.
use crate::{auth::Auth,error::{ApiError,ApiResult},models::Track,state::AppState};
use axum::{extract::{Path,Query,State},Json};
use serde::Deserialize;
use serde_json::{json,Value};

#[derive(Default,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Options {kind:Option<String>,page:Option<u32>,page_size:Option<u32>,cursor:Option<String>,refresh:bool}

fn origin(track:&Track)->Option<(String,String)> {
    let cached=&track.provider_data["cached_from"];
    if let (Some(source),Some(id))=(cached["source"].as_str(),cached["track_id"].as_str()){return Some((source.into(),id.into()));}
    if track.source!="local" {return Some((track.source.clone(),track.id.clone()));}
    for key in ["provider_metadata","qqmusic_metadata"] {
        let data=&track.provider_data[key];
        if data["matched"]==true {
            if let (Some(source),Some(id))=(data["source"].as_str(),data["track"]["id"].as_str()){return Some((source.into(),id.into()));}
            if let (Some("qqmusic"),Some(mid))=(data["source"].as_str(),data["mid"].as_str()){return Some(("qqmusic".into(),format!("qqmusic:{mid}")));}
        }
    }
    None
}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(q):Query<Options>)->ApiResult<Json<Value>> {
    a.require("read")?;
    let kind=q.kind.as_deref().unwrap_or("recommend");let page=q.page.unwrap_or(1);let size=q.page_size.unwrap_or(20);let cursor=q.cursor.unwrap_or_default();
    if !matches!(kind,"recommend"|"hot"|"new")||!(1..=1000).contains(&page)||!(1..=50).contains(&size)||cursor.len()>128||!cursor.bytes().all(|v|v.is_ascii_digit()){return Err(ApiError::bad("评论类型或分页参数无效"));}
    let track=crate::library::get_track(&s,id).await?;
    let Some((source,track_id))=origin(&track) else{return Ok(Json(json!({"supported":false,"items":[],"notice":"此歌曲尚未关联提供乐评的音源"})));};
    if !crate::accounts::scope::online_allowed(&source){return Ok(Json(json!({"supported":false,"items":[],"notice":"此账号未授权该在线平台，仅提供已保存的音乐资料"})));}
    crate::plugin::check_enabled(&s,&source).await?;
    let descriptor=crate::connectors::manifest(&s,&source).await?;
    if !descriptor["capabilities"].as_array().is_some_and(|v|v.iter().any(|c|c=="comments_read_v1")){return Ok(Json(json!({"supported":false,"source":source,"items":[],"notice":"此音源暂不提供乐评读取"})));}
    Ok(Json(s.plugin.call(&source,"comments",json!({"track_id":track_id,"kind":kind,"page":page,"page_size":size,"cursor":cursor,"refresh":q.refresh})).await?))
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn local_comments_require_explicit_match_and_cached_music_keeps_origin(){
        let mut track:Track=serde_json::from_value(json!({"id":"local-one","title":"Title","artist":"Artist","album":"Album","source":"local","format":"flac","sample_rate":44100,"bit_depth":16,"channels":2,"bitrate":0,"duration":10,"track_number":0,"disc_number":1,"album_artist":"Artist","recording_id":"local-one","available":true,"created_at":1,"provider_data":{}})).unwrap();
        assert!(origin(&track).is_none());
        track.provider_data=json!({"provider_metadata":{"matched":false,"source":"qqmusic","mid":"Wrong123"}});assert!(origin(&track).is_none());
        track.provider_data["provider_metadata"]["matched"]=json!(true);assert_eq!(origin(&track),Some(("qqmusic".into(),"qqmusic:Wrong123".into())));
        track.provider_data=json!({"cached_from":{"source":"example","track_id":"example:original"}});assert_eq!(origin(&track),Some(("example".into(),"example:original".into())));
    }
}
