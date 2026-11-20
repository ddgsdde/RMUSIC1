//! Ephemeral fan-out of one decoder-side analysis per actual playback instance.
//! No decoding, music fetch, file write or per-view analysis worker exists in Core.
use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use axum::{extract::{Path,State},Json};
use serde::Deserialize;
use serde_json::{json,Value};
use std::{collections::HashMap,time::{Duration,Instant}};
use tokio::sync::mpsc;
pub(crate) struct Entry { at:Instant, report:Value, owner:mpsc::Sender<Value> }
#[derive(Default)]
pub(crate) struct Registry { entries:HashMap<String,Entry>, leases:HashMap<String,Instant>, browsers:HashMap<String,BrowserEntry> }
struct BrowserEntry {at:Instant,report:Value,session:String}
#[derive(Deserialize,Default)]
pub struct Request { #[serde(default)] after:Option<f64> }
pub async fn read(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(request):Json<Request>)->ApiResult<Json<Value>> {
    a.require("read")?;
    if request.after.is_some_and(|v|!v.is_finite()||v<0.) {return Err(ApiError::bad("可视化游标无效"));}
    let public_zone=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
    let zone=crate::device_groups::leader(&s,&public_zone).await?.unwrap_or_else(||public_zone.clone());
    let supported=s.endpoints.read().await.get(&zone.endpoint_id).is_some_and(|e|e["online"]==true&&e["capabilities"]["visualization_v1"]==true);
    if !supported||zone.status!="playing" {return Ok(Json(json!({"supported":supported,"frames":[],"playback_id":public_zone.playback_id})));}
    let owner=s.bridges.read().await.get(&zone.endpoint_id).cloned();
    let browser_owner=s.browser_owners.read().await.get(&zone.endpoint_id).cloned();
    let now=Instant::now();let mut registry=s.visualizations.lock().await;
    registry.entries.retain(|_,v|now.duration_since(v.at)<Duration::from_secs(3));registry.browsers.retain(|_,v|now.duration_since(v.at)<Duration::from_secs(3));registry.leases.retain(|_,at|now.duration_since(*at)<Duration::from_secs(3));
    if registry.leases.get(&zone.endpoint_id).is_none_or(|at|now.duration_since(*at)>=Duration::from_secs(1))&&registry.leases.len()<128 {
        let sent=if let Some(owner)=&owner {owner.try_send(json!({"type":"visualization_lease","milliseconds":3000})).is_ok()}
            else if browser_owner.is_some(){s.emit("browser_visualization_lease",json!({"endpoint_id":zone.endpoint_id,"milliseconds":3000}));true}else{false};
        if sent {registry.leases.insert(zone.endpoint_id.clone(),now);}
    }
    let entry=registry.entries.get(&zone.endpoint_id).filter(|e|owner.as_ref().is_some_and(|owner|e.owner.same_channel(owner))).map(|e|(&e.report,e.at))
        .or_else(||registry.browsers.get(&zone.endpoint_id).filter(|e|browser_owner.as_ref()==Some(&e.session)).map(|e|(&e.report,e.at)))
        .filter(|(r,_)|r["core_playback_id"].as_str()==zone.playback_id.as_deref()&&r["track_id"].as_str()==zone.track.as_ref().map(|t|t.id.as_str()));
    let report=entry.map(|(r,_)|r);
    let mut frames=report.and_then(|r|r["frames"].as_array()).into_iter().flatten().filter(|f|f["position"].as_f64().is_some_and(|p|request.after.is_none_or(|after|p>after))).cloned().collect::<Vec<_>>();
    if zone.id!=public_zone.id {for frame in &mut frames{frame["playback_id"]=json!(public_zone.playback_id);}}
    Ok(Json(json!({"version":1,"supported":true,"playback_id":public_zone.playback_id,"track_id":zone.track.map(|t|t.id),"position":report.map(|r|&r["position"]),"age_ms":entry.map(|(_,at)|at.elapsed().as_secs_f64()*1000.),"frames":frames})))
}
pub async fn publish(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Value>)->ApiResult<axum::http::StatusCode> {
    a.require("control")?;if !valid(&v){return Err(ApiError::bad("可视化帧无效或超出范围"));}
    let zone=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
    crate::zones::transfer::authorize_output(&s,&a.session_hash,&zone.endpoint_id).await?;
    if zone.status!="playing"||!crate::zones::report_matches(&s,&zone,v["track_id"].as_str(),v["core_playback_id"].as_str()).await{return Ok(axum::http::StatusCode::NO_CONTENT);}
    if s.browser_owners.read().await.get(&zone.endpoint_id)!=Some(&a.session_hash){return Err(ApiError::bad("可视化报告须来自当前输出会话"));}
    let now=Instant::now();let mut registry=s.visualizations.lock().await;
    registry.browsers.retain(|_,e|now.duration_since(e.at)<Duration::from_secs(3));
    if registry.leases.get(&zone.endpoint_id).is_some_and(|at|now.duration_since(*at)<Duration::from_secs(3))&&(registry.browsers.len()<128||registry.browsers.contains_key(&zone.endpoint_id)) {
        let frames=v["frames"].as_array().unwrap().iter().map(|f|json!({"playback_id":f["playback_id"],"position":f["position"],"bands":f["bands"],"energy":f["energy"],"peak":f["peak"]})).collect::<Vec<_>>();
        let report=json!({"version":1,"core_playback_id":v["core_playback_id"],"track_id":v["track_id"],"position":v["position"],"frames":frames});
        registry.browsers.insert(zone.endpoint_id,BrowserEntry{at:now,report,session:a.session_hash});
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}
pub async fn lease(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("control")?;let zone=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
    crate::zones::transfer::authorize_output(&s,&a.session_hash,&zone.endpoint_id).await?;
    let active=zone.status=="playing"&&s.visualizations.lock().await.leases.get(&zone.endpoint_id).is_some_and(|at|at.elapsed()<Duration::from_secs(3));
    Ok(Json(json!({"active":active})))
}

pub(crate) async fn expire(s:&AppState) { let now=Instant::now();let mut registry=s.visualizations.lock().await;registry.entries.retain(|_,e|now.duration_since(e.at)<Duration::from_secs(3));registry.leases.retain(|_,at|now.duration_since(*at)<Duration::from_secs(3)); }
pub(crate) async fn disconnected(s:&AppState,id:&str) { let mut registry=s.visualizations.lock().await;registry.entries.remove(id);registry.browsers.remove(id);registry.leases.remove(id); }
fn valid(v:&Value)->bool {
    v["version"]==1 && v["position"].as_f64().is_some_and(|p|p.is_finite()&&p>=0.) && v["frames"].as_array().is_some_and(|frames|frames.len()<=24&&frames.iter().all(|f|
        f["playback_id"]==v["core_playback_id"]&&f["position"].as_f64().is_some_and(|p|p.is_finite()&&p>=0.&&p<=v["position"].as_f64().unwrap()+0.05&&p>=v["position"].as_f64().unwrap()-1.)&&
        f["bands"].as_array().is_some_and(|bands|bands.len()==8&&bands.iter().all(|v|v.as_u64().is_some_and(|n|n<=255)))&&["energy","peak"].iter().all(|key|f[*key].as_u64().is_some_and(|n|n<=255))))
}
pub(crate) async fn accept(s:&AppState,owner:&mpsc::Sender<Value>,v:Value) {
    if !valid(&v){return;}
    let (Some(endpoint),Some(zone))=(v["endpoint_id"].as_str(),v["zone_id"].as_str())else{return;};
    if !s.bridges.read().await.get(endpoint).is_some_and(|current|current.same_channel(owner)){return;}
    let Some(zone)=s.zones.read().await.get(zone).cloned()else{return;};
    if zone.endpoint_id!=endpoint||zone.status!="playing"||!crate::zones::report_matches(s,&zone,v["track_id"].as_str(),v["core_playback_id"].as_str()).await{return;}
    let mut registry=s.visualizations.lock().await;let now=Instant::now();
    registry.entries.retain(|_,e|now.duration_since(e.at)<Duration::from_secs(3));
    // Reject unsolicited reports. No event broadcast: unrelated clients never receive visual traffic.
    if !registry.leases.get(endpoint).is_some_and(|at|now.duration_since(*at)<Duration::from_secs(3)){return;}
    if registry.entries.len()<128||registry.entries.contains_key(endpoint){
        let frames=v["frames"].as_array().unwrap().iter().map(|f|json!({"playback_id":f["playback_id"],"position":f["position"],"bands":f["bands"],"energy":f["energy"],"peak":f["peak"]})).collect::<Vec<_>>();
        let report=json!({"version":1,"core_playback_id":v["core_playback_id"],"track_id":v["track_id"],"position":v["position"],"frames":frames});
        registry.entries.insert(endpoint.into(),Entry{at:now,report,owner:owner.clone()});
    }
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn rejects_unbounded_malformed_and_stale_frames(){let value=json!({"version":1,"position":1.,"core_playback_id":"p","frames":[{"playback_id":"p","position":1.,"bands":[1,2,3,4,5,6,7,8],"energy":12,"peak":22}]});assert!(valid(&value));for key in ["energy","peak"]{let mut bad=value.clone();bad["frames"][0][key]=json!(256);assert!(!valid(&bad));}let mut bad=value.clone();bad["frames"][0]["playback_id"]=json!("old");assert!(!valid(&bad));let mut bad=value.clone();bad["frames"]=json!(vec![value["frames"][0].clone();25]);assert!(!valid(&bad));}
}
