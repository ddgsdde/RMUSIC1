use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::Zone,
    state::AppState,
};
use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    response::{Response,IntoResponse},Json,
};
use serde::{Deserialize,Serialize};
use serde_json::{json, Value};
pub(crate) mod transfer;
pub(crate) mod snapshots;

#[derive(Deserialize)]
pub struct ListQuery {view:Option<String>}

pub async fn list(State(s): State<AppState>, a: Auth, Query(q):Query<ListQuery>) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let data=if q.view.as_deref()==Some("state") {snapshots::list(&s).await} else {s.zone_list().await};
    let mut event=crate::accounts::scope::event(&s,&a,json!({"type":"zones","data":data})).await?.unwrap();
    let mut value=event["data"].take();
    if let Some(endpoint)=crate::lan_bridges::endpoint(&a){
        if let Some(items)=value["items"].as_array_mut(){items.retain(|z|z["endpoint_id"]==endpoint);}
    }
    Ok(Json(value))
}
pub async fn get(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Zone>> {
    a.require("read")?;
    if let Some(key)=crate::portable::control_id(&s,&a,&id).await{return Ok(Json(crate::portable::zone(&s,&a,&key).await?));}
    Ok(Json(
        s.zones
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or_else(ApiError::missing)?,
    ))
}
#[derive(Deserialize)]
pub struct NewZone {
    name: String,
    endpoint_id: Option<String>,
}
pub async fn create(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<NewZone>,
) -> ApiResult<Json<Zone>> {
    a.require("control")?;
    let _lock = s.zone_lock.lock().await;
    if v.name.trim().is_empty() || v.name.len() > 100 {
        return Err(ApiError::bad("请输入音区名称"));
    }
    let endpoint = v.endpoint_id.unwrap_or_else(|| "browser-local".into());
    crate::device_groups::endpoint_guard(&s,&endpoint).await?;
    if !s.endpoints.read().await.contains_key(&endpoint) {
        return Err(ApiError::bad("播放端点不存在"));
    }
    crate::accounts::scope::check(&s,&a,"endpoint",&endpoint).await?;
    let z = Zone::new(uuid::Uuid::new_v4().to_string(), v.name, endpoint);
    crate::accounts::scope::claim(&s,&a,"zone",&z.id).await?;
    s.save_zone(z.clone()).await?;
    Ok(Json(z))
}
pub async fn update(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Json<Zone>> {
    a.require("control")?;
    if let Some(key)=crate::portable::control_id(&s,&a,&id).await{return Ok(Json(crate::portable::command(&s,&a,&key,"PATCH","",v).await?));}
    if let Some(zone)=crate::device_groups::patch(&s,&a,&id,&v).await?{return Ok(Json(zone));}
    let _lock = s.zone_lock.lock().await;
    crate::device_groups::member_guard(&s,&id,None).await?;
    if let Some(zone)=crate::sync_start::volume_patch(&s,&id,&v).await?{return Ok(Json(zone));}
    crate::sync_start::cancel_zone(&s,&id).await?;
    let mut z = s
        .zones
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(ApiError::missing)?;
    if let Some(name) = v["name"].as_str() {
        if name.trim().is_empty() || name.len() > 100 {
            return Err(ApiError::bad("名称无效"));
        }
        z.name = name.into();
    }
    if let Some(volume) = v["volume"].as_f64() {
        if !(0.0..=1.0).contains(&volume) {
            return Err(ApiError::bad("音量范围为 0 到 1"));
        }
        let endpoint=s.endpoints.read().await.get(&z.endpoint_id).cloned();
        z.volume=endpoint.as_ref().and_then(|e|serde_json::from_value::<rmusic_audio::output::OutputConfig>(e["output_config"].clone()).ok()).map_or(volume,|c|c.limits.volume(volume));
    }
    if let Some(muted) = v["muted"].as_bool() {
        z.muted = muted;
    }
    if let Some(repeat) = v["repeat"].as_str() {
        if !["off", "one", "all"].contains(&repeat) {
            return Err(ApiError::bad("循环模式无效"));
        }
        z.repeat = repeat.into();
    }
    if let Some(shuffle) = v["shuffle"].as_bool() {
        if shuffle && !z.shuffle {shuffle_tail(&mut z);}
        z.shuffle = shuffle;
    }
    if let Some(value)=v.get("transitions") {
        let transitions:crate::models::Transitions=serde_json::from_value(value.clone()).map_err(|_|ApiError::bad("淡入淡出设置无效"))?;
        if transitions.fade_in_ms>10000||transitions.fade_out_ms>10000 {return Err(ApiError::bad("淡入淡出最长十秒"));}
        z.transitions=transitions;
    }
    if v.get("transitions").is_some(){processing_allowed(&s,&z).await?;}
    if let Some(autoplay)=v.get("autoplay") {
        let config:crate::models::Autoplay=serde_json::from_value(autoplay.clone()).map_err(|_|ApiError::bad("自动续播设置无效"))?;
        if !crate::connectors::valid_id(&config.source) {return Err(ApiError::bad("推荐来源无效"));}
        z.autoplay=config;
    }
    if ["autoplay","repeat","shuffle","endpoint_id"].iter().any(|key|v.get(*key).is_some()) {crate::autoplay::cancel(&s,&z.id).await;}
    if let Some(endpoint) = v["endpoint_id"].as_str() {
        crate::device_groups::endpoint_guard(&s,endpoint).await?;
        if !s.endpoints.read().await.contains_key(endpoint) {
            return Err(ApiError::bad("端点不存在"));
        }
        let mut candidate=z.clone();candidate.endpoint_id=endpoint.into();processing_allowed(&s,&candidate).await?;
        dispatch(&s, &z, "stop").await?;
        crate::stats::finish(&s,&z,"disconnect").await?;
        z.status = "stopped".into();
        z.playback_id=None;
        z.endpoint_id = endpoint.into();
    }
    if ["volume","muted","endpoint_id"].iter().any(|key|v.get(*key).is_some()) {
        processing_allowed(&s,&z).await?;
        dispatch(&s, &z, "volume").await?;
    }
    if v.get("transitions").is_some() && z.playback_id.is_some() && crate::render_media::supported(&s,&z.endpoint_id).await {
        dispatch(&s,&z,"dsp").await?;
    }
    s.save_zone(z.clone()).await?;
    crate::autoplay::prepare(&s,&z).await;
    Ok(Json(z))
}
pub async fn delete(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    a.require("control")?;
    if let Some(g)=crate::device_groups::by_zone(&s,&id).await?{return crate::device_groups::delete(State(s),a,Path(g.id)).await;}
    let _lock = s.zone_lock.lock().await;
    crate::device_groups::member_guard(&s,&id,None).await?;
    crate::sync_start::cancel_zone(&s,&id).await?;
    let z = s
        .zones
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(ApiError::missing)?;
    dispatch(&s, &z, "stop").await?;
    crate::autoplay::cancel(&s,&z.id).await;
    crate::stats::finish(&s,&z,"delete").await?;
    let zid = id.clone();
    s.db.run(move |c| {
        c.execute("DELETE FROM zones WHERE id=?1", [zid])?;
        Ok(())
    })
    .await?;
    s.zones.write().await.remove(&id);
    crate::sequence::invalidate(&s, &id).await?;
    s.emit("zones_changed", json!({}));
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize,Serialize,Default)]
pub struct Command {
    #[serde(default)] exact_version:bool,
    control:Option<ControlIdentity>,
    track_id: Option<String>,
    queue_index: Option<usize>,
    position: Option<f64>,
    status: Option<String>,
    playback_id: Option<String>,
    frames: Option<u64>,
    sample_rate: Option<u32>,
    measured_version: Option<u32>,
    #[serde(default)]
    recoverable: bool,
    #[serde(default)]
    device_failed: bool,
    #[serde(default)]
    system_suspend:Option<crate::sequence::recovery::SystemSuspend>,
}
#[derive(Deserialize,Serialize)]
struct ControlIdentity {endpoint_id:String,track_id:String,playback_id:String}
pub(crate) fn paused_system_report_matches(z:&Zone,track:Option<&str>,playback:Option<&str>)->bool {
    z.status=="paused"&&playback.is_some()&&z.playback_id.as_deref()==playback&&z.track.as_ref().map(|t|t.id.as_str())==track
}
pub async fn report_matches(s:&AppState,z:&Zone,track_id:Option<&str>,playback_id:Option<&str>)->bool {
    let guarded=s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["playback_instance"]==true);
    if track_id.is_some_and(|id|z.track.as_ref().map(|t|t.id.as_str())!=Some(id)) {return false;}
    if let Some(id)=playback_id {return z.playback_id.as_deref()==Some(id)&&(!guarded||track_id.is_some());}
    !guarded
}
fn selection_stamp(z:&Zone)->String {
    crate::auth::hash(&json!([z.endpoint_id,z.playback_id,z.status,if z.status=="playing"{0}else{z.updated_at},z.queue_index,z.queue.iter().map(|t|&t.id).collect::<Vec<_>>()]).to_string())
}
pub async fn command(state:State<AppState>,a:Auth,path:Path<(String,String)>,payload:Option<Json<Command>>)->ApiResult<Response> {
    if path.0.1=="ended" {
        // Core owns an accepted natural EOF through persistence and next-output dispatch.
        tokio::spawn(command_internal(state,a,path,payload,false)).await.map_err(anyhow::Error::from)?
    } else {command_internal(state,a,path,payload,false).await}
}
pub(crate) async fn command_internal(
    State(s): State<AppState>,
    a: Auth,
    Path((id, action)): Path<(String, String)>,
    payload: Option<Json<Command>>,member:bool,
) -> ApiResult<Response> {
    a.require("control")?;
    let v = payload.map(|v| v.0).unwrap_or_default();
    if let Some(key)=crate::portable::control_id(&s,&a,&id).await{
        return Ok(Json(crate::portable::command(&s,&a,&key,"POST",&action,serde_json::to_value(&v).map_err(anyhow::Error::from)?).await?).into_response());
    }
    if !member {if let Some(zone)=crate::device_groups::command(&s,&a,&id,&action,&serde_json::to_value(&v).map_err(anyhow::Error::from)?).await? {return Ok(Json(zone).into_response());}}
    // Optional source preflight must never stall progress, stop, or another zone.
    let mut prepared = if action=="play" && !v.exact_version && v.control.is_none() {
        if let Some(track_id)=&v.track_id {
            let stamp={let zones=s.zones.read().await;selection_stamp(zones.get(&id).ok_or_else(ApiError::missing)?)};
            let original=crate::library::get_track(&s,track_id.clone()).await?;
            Some((stamp,crate::music_playback_policy::select(&s,&a.id,original).await?))
        }else{None}
    }else{None};
    let _lock = s.zone_lock.lock().await;
    if !member && !matches!(action.as_str(),"progress"|"ended") {crate::device_groups::member_guard(&s,&id,None).await?;}
    if let Some((stamp,_))=&prepared {let zones=s.zones.read().await;if zones.get(&id).map(selection_stamp).as_ref()!=Some(stamp){return Err(ApiError(StatusCode::CONFLICT,"version_preparation_stale".into(),"播放准备期间音区已变化，请重新选择".into()));}}
    if let Some(control)=&v.control {
        if !matches!(action.as_str(),"play"|"pause"|"stop")||v.track_id.is_some()||v.queue_index.is_some()||v.frames.is_some()||v.sample_rate.is_some()||v.recoverable||v.system_suspend.is_some() {
            return Err(ApiError::bad("带身份的系统控制只允许当前曲目的播放、暂停或停止"));
        }
        let zones=s.zones.read().await;let z=zones.get(&id).ok_or_else(ApiError::missing)?;
        if z.endpoint_id!=control.endpoint_id||z.playback_id.as_deref()!=Some(control.playback_id.as_str())||z.track.as_ref().map(|t|t.id.as_str())!=Some(control.track_id.as_str()) {
            return Err(ApiError(StatusCode::CONFLICT,"stale_playback".into(),"播放实例已变化，系统控制已拒绝".into()));
        }
        if v.position.is_some_and(|p|!p.is_finite()||p<0.||z.track.as_ref().is_some_and(|t|t.duration>0.&&p>t.duration)) {return Err(ApiError::bad("播放位置无效"));}
        drop(zones);
        if crate::sync_start::active_zone(&s,&id).await {return Err(ApiError::bad("整组播放须使用整组控制"));}
    }
    let timing=v.measured_version.unwrap_or(1);
    if ![1,2].contains(&timing)||(timing==2&&(!matches!(action.as_str(),"progress"|"ended")||v.frames.is_none()||v.sample_rate.is_none())){return Err(ApiError::bad("输出计时版本或帧报告无效"));}
    if action=="ended" {if let Some(zone)=crate::sync_start::queue::ended(&s,&a,&id,v.track_id.as_deref(),v.playback_id.as_deref(),v.position,v.frames,v.sample_rate,timing).await?{return Ok(Json(zone).into_response());}}
    if matches!(action.as_str(),"progress"|"ended")&&crate::sync_start::active_zone(&s,&id).await{return Err(ApiError::bad("整组节目进度由当前网桥连接报告"));}
    if !matches!(action.as_str(),"progress"|"ended"){crate::sync_start::cancel_zone(&s,&id).await?;}
    let mut z = s
        .zones
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(ApiError::missing)?;
    if action=="ended" && v.track_id.is_some() && v.playback_id.is_some() && (z.status!="playing" || !report_matches(&s,&z,v.track_id.as_deref(),v.playback_id.as_deref()).await) {
        return Ok(Json(z).into_response());
    }
    let device=match(v.frames,v.sample_rate) {
        (None,None)=>None,
        (Some(frames),Some(rate)) if matches!(action.as_str(),"progress"|"ended") && (8000..=rmusic_audio::MAX_CLOCK_RATE).contains(&rate)
            && frames as f64/rate as f64<=86400. && v.position.is_some_and(|p|p.is_finite()&&p>=0.)=>{
                crate::sequence::authorize(&s,&a.session_hash,&z.endpoint_id).await?;Some((frames,rate))
            },
        _=>return Err(ApiError::bad("实际帧报告须包含有效位置、帧数和采样率"))
    };
    let measured_pause=device.is_some()&&z.status=="paused"&&v.status.as_deref().is_none_or(|status|status=="paused");
    let continuous=s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["continuous_pcm"]==true);
    if v.device_failed&&(!v.recoverable||action!="progress") {return Err(ApiError::bad("设备中断须使用可恢复进度报告"));}
    if let Some(stamp)=&v.system_suspend {stamp.validate()?;if !v.device_failed{return Err(ApiError::bad("系统挂起须使用设备中断报告"));}}
    if action=="progress"&&v.recoverable {
        if v.device_failed&&!s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["recovery_controls"]==true){return Err(ApiError::bad("设备中断须由支持恢复控制的端点报告"));}
        if device.is_some()||v.position.is_some()||v.status.as_deref().is_some_and(|status|status!="paused")||!s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["continuous_recovery"]==true) {
            return Err(ApiError::bad("可恢复中断须由持久恢复端点单独报告"));
        }
        crate::sequence::authorize(&s,&a.session_hash,&z.endpoint_id).await?;
        if report_matches(&s,&z,v.track_id.as_deref(),v.playback_id.as_deref()).await||(v.system_suspend.is_some()&&paused_system_report_matches(&z,v.track_id.as_deref(),v.playback_id.as_deref())) {
            if let Some(stamp)=&v.system_suspend {if !crate::sequence::recovery::new_system_suspend(&s,&z.id,stamp).await?{return Ok(StatusCode::NO_CONTENT.into_response());}}
            crate::stats::finish(&s,&z,"disconnect").await?;
            if v.device_failed {crate::sequence::recovery::suspend_device_with_stamp(&s,&z,v.system_suspend).await?;}
            else{crate::sequence::recovery::suspend(&s,&z).await?;}
        }
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if action == "progress" {
        let stale_transport = if z.status == "playing" {
            v.status.as_deref() == Some("paused")
        } else {
            !measured_pause
        };
        if !report_matches(&s, &z, v.track_id.as_deref(), v.playback_id.as_deref()).await
            || (stale_transport && s.endpoints.read().await.get(&z.endpoint_id)
                .is_some_and(|e| e["capabilities"]["playback_instance"] == true))
        {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    }

    if matches!(action.as_str(),"progress"|"ended")&&timing!=crate::stats::output::version(&s,&z).await? {
        if action=="progress"{return Ok(StatusCode::NO_CONTENT.into_response());}
        return Err(ApiError::bad("播放实例的计时版本不匹配"));
    }
    if action=="progress"&&device.is_some()&&z.playback_id.is_some(){if let Some(endpoint)=s.endpoints.write().await.get_mut(&z.endpoint_id){endpoint["ready_playback_id"]=json!(z.playback_id);}}
    let mut old=z.clone();
    if v.control.is_some() {if let Some(position)=v.position {z.position=position;old.position=position;old.updated_at=chrono::Utc::now().timestamp_millis();}}
    let mut device_finished=false;
    if action=="ended" {
        let track_id=v.track_id.as_ref().ok_or_else(||ApiError::bad("缺少结束播放的歌曲 ID"))?;
        if z.status!="playing" || !report_matches(&s,&z,Some(track_id),v.playback_id.as_deref()).await {
            return Ok(Json(z).into_response());
        }
        if let Some(position)=v.position {
            if !position.is_finite() || position<0. {return Err(ApiError::bad("播放位置无效"));}
            old.position=position;
        } else if let Some(track)=&z.track {
            if track.duration>0. {old.position=track.duration;}
        }
        if let Some((frames,rate))=device {
            let accepted=if timing==2 {crate::stats::output::progress(&s,&old,frames,rate,true).await?}else{crate::stats::finish_device(&s,&old,frames,rate).await?};
            if !accepted{return Err(ApiError::bad("实际帧超出核心计时范围"));}
            device_finished=true;
        }
    }
    let restart=action=="play"&&(v.track_id.is_some()||v.queue_index.is_some());
    let pending_resume=action=="play"&&!restart&&crate::sequence::recovery::pending_resume(&s,&z,&a.id).await?;
    let mut actual = action.clone();
    match action.as_str() {
        "play" => {
            if let Some(id) = v.track_id {
                let original=crate::library::get_track(&s,id).await?;
                let (selected,used_policy)=if v.exact_version{(original,true)}else{prepared.take().map(|(_,selected)|selected).unwrap_or((original,false))};
                let t=if used_policy{selected}else{crate::catalog::prefer_queue(&s,vec![selected]).await?.remove(0)};
                if let Some(i) = z.queue.iter().position(|q| q.id == t.id) {
                    if used_policy { z.queue[i] = t; }
                    z.queue_index = i;
                } else {
                    z.queue.push(t);
                    z.queue_index = z.queue.len() - 1;
                }
                z.position = 0.;
            }
            if let Some(i) = v.queue_index {
                if i >= z.queue.len() {
                    return Err(ApiError::bad("队列位置超出范围"));
                }
                z.queue_index = i;
                z.position = 0.;
            }
            if z.queue.is_empty() {
                return Err(ApiError::bad("播放队列为空"));
            }
            z.track = z.queue.get(z.queue_index).cloned();
            z.status = "playing".into();
        }
        "pause" => {
            if z.status == "playing" && !continuous && !(v.control.is_some()&&v.position.is_some()) {
                z.position = current_position(&z);
            }
            z.status = "paused".into();
        }
        "stop" => {
            z.status = "stopped".into();
            z.playback_id=None;
            z.position = 0.;
        }
        "seek" => {
            let p = v.position.ok_or_else(|| ApiError::bad("缺少播放位置"))?;
            if !p.is_finite()
                || p < 0.
                || z.track
                    .as_ref()
                    .is_some_and(|t| t.duration > 0. && p > t.duration)
            {
                return Err(ApiError::bad("播放位置无效"));
            }
            z.position = p;
            if z.track.is_some()&&z.status!="stopped"{z.renew_playback();}
        }
        "next" | "previous" | "ended" => {
            if z.queue.is_empty() {
                return Err(ApiError::bad("队列为空"));
            }
            actual=advance(&s,&mut z,action=="previous").await;
        }
        "progress" => {
            if let Some(p) = v.position {
                if p.is_finite() && p >= 0. {
                    z.position = p;
                }
            }
            if let Some(status) = &v.status {
                if !["playing", "paused", "stopped"].contains(&status.as_str()) {
                    return Err(ApiError::bad("播放状态无效"));
                }
                z.status = status.clone();
            }
        }
        _ => return Err(ApiError::missing()),
    }
    if action=="play"&&!pending_resume&&(restart||old.status=="stopped"||old.playback_id.is_none()){z.renew_playback();}
    z.updated_at = chrono::Utc::now().timestamp_millis();
    if pending_resume {
        processing_allowed(&s,&z).await?;crate::autoplay::cancel(&s,&z.id).await;
        s.save_zone_action(z.clone(),Some("play")).await?;
        // An adapter waiting for an explicit device retry has no active restore task.
        // Keep the durable intent if its socket disappears while delivering this wake-up.
        if s.bridges.read().await.contains_key(&z.endpoint_id) {
            if let Err(error)=dispatch(&s,&z,"resume").await {tracing::debug!(status=%error.0,"pending resume wake deferred until registration");}
        }
        return Ok(Json(z).into_response());
    }
    if action=="progress"{
        if let Some((frames,rate))=device {
            let endpoint=z.endpoint_id.clone();
            let accepted=if timing==2 {let accepted=s.publish_output_progress(z,frames,rate).await?;if !accepted{return Err(ApiError::bad("输出帧与当前播放实例不符"));}accepted}else{s.publish_device_progress(z,frames,rate).await};
            if accepted {
                if let Some(endpoint)=s.endpoints.write().await.get_mut(&endpoint){if endpoint["kind"]=="browser"{endpoint["last_seen_ms"]=json!(chrono::Utc::now().timestamp_millis());}}
            }
        }else if continuous&&v.position.is_none()&&v.status.is_some(){s.publish_control_progress(z).await?;}
        else{s.publish_progress(z).await;}
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    if matches!(action.as_str(),"play"|"pause"|"stop"|"previous"|"seek") {crate::autoplay::cancel(&s,&z.id).await;}
    if actual=="play"{let others:Vec<_>=s.zones.read().await.values().filter(|other|other.id!=z.id&&other.endpoint_id==z.endpoint_id).cloned().collect();for mut other in others{crate::autoplay::cancel(&s,&other.id).await;if other.status=="playing"{other.status="paused".into();other.playback_id=None;dispatch(&s,&other,"stop").await?;crate::stats::record_progress(&s,&other).await?;s.save_zone(other).await?;}}}
    if action != "progress" && action != "ended" {
        dispatch(&s, &z, &actual).await?;
    }
    if !device_finished&&(matches!(action.as_str(),"next"|"previous"|"stop"|"ended")||restart) {
        let reason=if restart {if old.track.as_ref().map(|t|&t.id)==z.track.as_ref().map(|t|&t.id){"replay"}else{"replaced"}}else{action.as_str()};
        crate::stats::finish(&s,&old,reason).await?;
    }
    if action=="play" {crate::radio::set_owner(&s,&z.id,&a.id,restart).await?;}
    if action=="seek" {crate::stats::reposition(&s,&z).await?;}
    else if continuous&&(action=="pause" || action=="play"&&!restart&&old.playback_id==z.playback_id) {crate::stats::control_status(&s,&z).await?;}
    else if action=="pause" {let mut measured=old;measured.status="paused".into();crate::stats::record_progress(&s,&measured).await?;}
    else {crate::stats::record_progress(&s,&z).await?;}
    s.save_zone_action(z.clone(),Some(&action)).await?;
    // Stopping the bridge cancels its pending EOF HTTP call. Publish the final state first.
    if action=="ended" {dispatch(&s,&z,&actual).await?;}
    crate::autoplay::prepare(&s,&z).await;
    Ok(Json(z).into_response())
}
pub async fn advance(s:&AppState,z:&mut Zone,previous:bool)->String {
    if z.repeat=="one" && !previous {}
    else if previous {z.queue_index=z.queue_index.saturating_sub(1);}
    else if z.queue_index+1<z.queue.len() {z.queue_index+=1;}
    else if z.repeat=="all" {z.queue_index=0;if z.shuffle {shuffle_tail(z);}}
    else if crate::autoplay::continue_queue(s,z).await {return "play".into();}
    else {z.status="stopped".into();z.playback_id=None;z.position=0.;return "stop".into();}
    z.position=0.;z.track=z.queue.get(z.queue_index).cloned();z.status="playing".into();z.renew_playback();"play".into()
}
pub(crate) fn shuffle_tail(z:&mut Zone) {
    use rand::seq::SliceRandom;
    let start=(z.queue_index+1).min(z.queue.len());z.queue[start..].shuffle(&mut rand::thread_rng());
}
pub(crate) fn current_position(z: &Zone) -> f64 {
    let p =
        z.position + (chrono::Utc::now().timestamp_millis() - z.updated_at).max(0) as f64 / 1000.;
    z.track
        .as_ref()
        .filter(|t| t.duration > 0.)
        .map(|t| p.min(t.duration))
        .unwrap_or(p)
}
#[derive(Deserialize,Serialize)]
pub struct Queue {
    track_ids: Vec<String>,
    queue_index: Option<usize>,
    insertion_index: Option<usize>,
    shuffle: Option<bool>,
    #[serde(default)]
    play_next: bool,
}
#[derive(Deserialize,Default)]
pub struct QueueView { view:Option<String> }
async fn queue_response(s:&AppState,zone:Zone,view:&QueueView)->Value {
    if view.view.as_deref()==Some("state") { let zones=s.zones.read().await;snapshots::state(zones.get(&zone.id).unwrap_or(&zone)) } else { serde_json::to_value(zone).expect("zone JSON") }
}
pub async fn queue_put(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(view):Query<QueueView>,Json(v):Json<Queue>)->ApiResult<Json<Value>> {
    Ok(Json(queue_response(&s,queue_change(&s,a,id,v,false).await?.0,&view).await))
}
pub async fn queue_post(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(view):Query<QueueView>,Json(v):Json<Queue>)->ApiResult<Json<Value>> {
    Ok(Json(queue_response(&s,queue_change(&s,a,id,v,true).await?.0,&view).await))
}
async fn queue_change(
    s: &AppState,
    a: Auth,
    id: String,
    v: Queue,
    append: bool,
) -> ApiResult<Json<Zone>> {
    a.require("control")?;
    if v.play_next && (!append || v.insertion_index.is_some() || v.queue_index.is_some()) {
        return Err(ApiError::bad("下一首播放只能用于追加，不能同时指定队列位置"));
    }
    if v.track_ids.len() > 10000 {
        return Err(ApiError::bad("单次队列上限为一万首"));
    }
    if let Some(key)=crate::portable::control_id(&s,&a,&id).await{
        let method=if append{"POST"}else{"PUT"};
        return Ok(Json(crate::portable::command(s,&a,&key,method,"queue",serde_json::to_value(&v).map_err(anyhow::Error::from)?).await?));
    }
    if let Some(zone)=crate::device_groups::queue(s,&a,&id,&serde_json::to_value(&v).map_err(anyhow::Error::from)?,append).await?{return Ok(Json(zone));}
    let _lock = s.zone_lock.lock().await;
    crate::device_groups::member_guard(s,&id,None).await?;
    if v.play_next && crate::sync_start::active_zone(s,&id).await {
        return Err(ApiError(axum::http::StatusCode::CONFLICT,"group_queue_active".into(),"请先结束整组同步播放后编辑单设备队列".into()));
    }
    crate::sync_start::cancel_zone(&s,&id).await?;
    let mut z = s
        .zones
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(ApiError::missing)?;
    let old=z.clone();
    if !append {if let Some(shuffle)=v.shuffle {z.shuffle=shuffle;}}
    let mut tracks = vec![];
    for id in v.track_ids {
        tracks.push(crate::library::get_track(s, id).await?);
    }
    let tracks=crate::catalog::prefer_queue(s,tracks).await?;
    let retained=if append&&!v.play_next&&v.insertion_index.is_none(){crate::sequence::append(s,&mut z,&tracks).await?}else{false};
    if append&&!retained {
        let index = (if v.play_next {Some(if z.track.is_some(){z.queue_index.saturating_add(1)}else{0})}else{v.insertion_index})
            .unwrap_or(z.queue.len())
            .min(z.queue.len());
        if v.play_next && z.track.is_none() { z.queue_index=0; }
        else if index <= z.queue_index && !z.queue.is_empty() {
            z.queue_index += tracks.len();
        }
        z.queue.splice(index..index, tracks);
    } else if !append {
        let current = z.track.as_ref().map(|t| t.id.clone());
        z.queue = tracks;
        if let Some(i) = v.queue_index {
            if i >= z.queue.len() {
                return Err(ApiError::bad("队列位置无效"));
            }
            z.queue_index = i;
            z.track = z.queue.get(i).cloned();
            z.position = 0.;
            if z.status!="stopped"{z.renew_playback();}else{z.playback_id=None;}
        } else if let Some(i) = z.queue.iter().position(|t| Some(&t.id) == current.as_ref()) {
            z.queue_index = i;
        } else {
            z.queue_index = 0;
            z.track = None;
            z.status = "stopped".into();
            z.playback_id=None;
            z.position = 0.;
            dispatch(s, &z, "stop").await?;
        }
    }
    if z.queue.len() > 10000 {
        return Err(ApiError::bad("队列上限为一万首"));
    }
    let resume_radio=append&&z.status=="stopped"&&z.queue_index+1<z.queue.len()&&crate::autoplay::waiting(s,&z.id).await;
    crate::autoplay::cancel(s,&z.id).await;
    if resume_radio {
        let action=advance(s,&mut z,false).await;
        z.updated_at=chrono::Utc::now().timestamp_millis();
        dispatch(s,&z,&action).await?;
        crate::stats::record_progress(s,&z).await?;
    }
    // Existing tail order includes explicit next-play and drag choices. Appending must preserve it.
    if z.shuffle && !append {shuffle_tail(&mut z);}
    if !append&&(v.queue_index.is_some()||old.track.as_ref().map(|t|&t.id)!=z.track.as_ref().map(|t|&t.id)) {
        crate::stats::finish(s,&old,"replaced").await?;
        crate::radio::set_owner(s,&z.id,&a.id,true).await?;
        if z.status=="playing" {dispatch(s,&z,"play").await?;crate::stats::record_progress(s,&z).await?;}
    }
    s.save_zone_action(z.clone(),Some(if retained{"queue_append"}else{"queue"})).await?;
    crate::autoplay::prepare(s,&z).await;
    Ok(Json(z))
}
pub async fn queue_clear(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Zone>> {
    queue_change(
        &s,
        a,
        id,
        Queue {
            track_ids: vec![],
            queue_index: None,
            insertion_index: None,
            shuffle: None,
            play_next: false,
        },
        false,
    )
    .await
}
#[derive(Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueueReorder {
    expected_queue: Vec<String>,
    expected_playback_id: Option<String>,
    order: Vec<usize>,
}
/// Permute occurrences, never reselect a song by ID (a queue may contain duplicates).
pub async fn queue_reorder(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(view):Query<QueueView>,Json(v):Json<QueueReorder>)->ApiResult<Json<Value>> {
    a.require("control")?;
    if v.order.len()>10000 || v.expected_queue.len()>10000 {return Err(ApiError::bad("队列上限为一万首"));}
    if let Some(key)=crate::portable::control_id(&s,&a,&id).await{
        let zone=crate::portable::command(&s,&a,&key,"POST","queue/reorder",serde_json::to_value(&v).map_err(anyhow::Error::from)?).await?;
        return Ok(Json(queue_response(&s,zone,&view).await));
    }
    if let Some(zone)=crate::device_groups::reorder(&s,&a,&id,&serde_json::to_value(&v).map_err(anyhow::Error::from)?).await?{return Ok(Json(queue_response(&s,zone,&view).await));}
    let _lock=s.zone_lock.lock().await;
    crate::device_groups::member_guard(&s,&id,None).await?;
    let mut z=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
    if z.playback_id!=v.expected_playback_id || z.queue.iter().map(|t|&t.id).ne(v.expected_queue.iter()) {
        return Err(ApiError(StatusCode::CONFLICT,"queue_edit_conflict".into(),"队列已在其他设备变化，请按最新队列重新拖动".into()));
    }
    let mut seen=vec![false;z.queue.len()];
    if v.order.len()!=seen.len() || v.order.iter().any(|&i|i>=seen.len()||std::mem::replace(&mut seen[i],true)) {
        return Err(ApiError::bad("排序必须包含原队列的每一个位置且不重复"));
    }
    if crate::sync_start::active_zone(&s,&id).await {return Err(ApiError(StatusCode::CONFLICT,"group_queue_active".into(),"请先结束整组同步播放后编辑单设备队列".into()));}
    if v.order.iter().copied().eq(0..z.queue.len()) {return Ok(Json(queue_response(&s,z,&view).await));}
    let current=z.queue_index;
    z.queue=v.order.iter().map(|&i|z.queue[i].clone()).collect();
    z.queue_index=v.order.iter().position(|&i|i==current).unwrap_or(0);
    crate::autoplay::cancel(&s,&id).await;
    s.save_zone_action(z.clone(),Some("queue")).await?;
    crate::autoplay::prepare(&s,&z).await;
    Ok(Json(queue_response(&s,z,&view).await))
}
pub async fn dsp(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Json<Zone>> {
    a.require("control")?;
    if !v.is_object() {
        return Err(ApiError::bad("DSP 设置必须为对象"));
    }
    let v=crate::models::complete_dsp(v);
    for key in ["enabled","normalize"] {
        if !v[key].is_boolean(){return Err(ApiError::bad(format!("DSP 参数 {key} 必须为布尔值")));}
    }
    if !v["bands"].is_array(){return Err(ApiError::bad("均衡器必须为数组"));}
    let convolution=crate::impulse_assets::configuration(&v)?;
    let config:rmusic_audio::DspConfig=serde_json::from_value(v.clone()).map_err(|_|ApiError::bad("均衡滤波器类型或 DSP 配置无效"))?;
    if config.sample_rate.is_some_and(|rate| !rmusic_audio::output::PCM_RATES.contains(&rate)) {
        return Err(ApiError::bad("DSP 目标采样率不受支持"));
    }
    if !config.valid_eq_size() {
        return Err(ApiError::bad("最多 4 个 EQ 模块，每个最多 32 段，合计最多 64 段"));
    }
    if config.eq_stages().flat_map(|(_, bands)| bands).any(|band| !band.valid_slope()) {
        return Err(ApiError::bad("低通和高通的斜率可选 12、24、36、48 dB/oct"));
    }
    if let Some(modules) = v.get("modules") {
        let modules = modules.as_array().ok_or_else(|| ApiError::bad("DSP 模块必须为列表"))?;
        let mut selected = std::collections::HashSet::new();
        for module in modules {
            let name = module.as_str().ok_or_else(|| ApiError::bad("DSP 模块名称无效"))?;
            if !matches!(name, "eq"|"rate"|"stereo"|"gain"|"convolution"|"loudness"|"fades") || !selected.insert(name) {
                return Err(ApiError::bad("无法重复添加此 DSP 模块"));
            }
        }
        if !selected.contains("eq") && (!config.bands.is_empty() || !config.extra_eq.is_empty()) {
            return Err(ApiError::bad("EQ 模块已移除，请清空该模块的频段"));
        }
    }
    if config.stereo.as_ref().is_some_and(|stereo| !stereo.valid()) {
        return Err(ApiError::bad("串音比例须为 0–50%，分频须为 200–2000 Hz"));
    }
    for (key, min, max) in [
        ("preamp_db", -60., 24.),
        ("balance", -1., 1.),
        ("target_lufs", -40., -5.),
    ] {
        {
            let n=v[key].as_f64().ok_or_else(||ApiError::bad(format!("DSP 参数 {key} 必须为数字")))?;
            if !(min..=max).contains(&n) {
                return Err(ApiError::bad(format!("DSP 参数 {key} 超出范围")));
            }
        }
    }
    for bands in std::iter::once(&v["bands"]).chain(v["extra_eq"].as_array().into_iter().flatten().map(|stage| &stage["bands"])).filter_map(Value::as_array) {
        if bands.len() > 32 {
            return Err(ApiError::bad("均衡器最多 32 段"));
        }
        for b in bands {
            if !b["enabled"].is_boolean(){return Err(ApiError::bad("均衡器启用状态必须为布尔值"));}
            for (key, min, max) in [
                ("frequency", 10., 192000.),
                ("gain_db", -30., 30.),
                ("q", 0.1, 30.),
            ] {
                let n = b[key]
                    .as_f64()
                    .ok_or_else(|| ApiError::bad("均衡器参数不完整"))?;
                if !(min..=max).contains(&n) {
                    return Err(ApiError::bad("均衡器参数超出范围"));
                }
            }
        }
    }
    if let Some(selection)=&convolution{crate::impulse_assets::validate_selection(&s,selection).await?;}
    if let Some(z)=crate::device_groups::dsp(&s,&a,&id,&v).await?{return Ok(Json(z));}
    let lock = s.zone_lock.clone().lock_owned().await;
    crate::device_groups::member_guard(&s,&id,None).await?;
    crate::sync_start::cancel_zone(&s,&id).await?;
    let mut z = s
        .zones
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(ApiError::missing)?;
    if let Some(selection)=&convolution{crate::impulse_assets::validate_selection(&s,selection).await?;}
    z.dsp = v;
    let z=tokio::spawn(async move{
        let _lock=lock;dispatch(&s,&z,"dsp").await?;s.save_zone(z.clone()).await?;Ok::<_,ApiError>(z)
    }).await.map_err(|error|ApiError::from(anyhow::Error::from(error)))??;
    Ok(Json(z))
}
pub(crate) async fn processing_allowed(s:&AppState,z:&Zone)->ApiResult<()> {
    if z.dsp["enabled"] == true && (z.dsp["eq_bypass"] == true
        || z.dsp["extra_eq"].as_array().is_some_and(|stages| !stages.is_empty())
        || z.dsp["stereo"].is_object()
        || z.dsp["bands"].as_array().is_some_and(|bands| bands.iter().any(|band| band["slope_db_oct"].as_u64().is_some_and(|slope| slope != 12))))
        && !crate::render_media::supported(s, &z.endpoint_id).await
    {
        return Err(ApiError::bad("此输出未接入 Core DSP，无法应用高级滤波或立体声设置"));
    }
    if z.dsp["enabled"]==true&&z.dsp.get("convolution").is_some_and(|value|!value.is_null()) {
        crate::impulse_assets::configuration(&z.dsp)?;
        if !s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|endpoint|endpoint["capabilities"]["convolution_v1"]==true||endpoint["capabilities"]["core_pcm_dsp_v1"]==true){return Err(ApiError::bad("当前输出不支持卷积播放，请关闭卷积或选择支持的输出"));}
    }
    let endpoints=s.endpoints.read().await;
    if let Some(endpoint)=endpoints.get(&z.endpoint_id){
        if endpoint["capabilities"]["dsp"]==false&&z.dsp["enabled"]==true{return Err(ApiError::bad("此输出由设备管理 DSP，请先关闭 R-Music DSP"));}
        if endpoint["capabilities"]["transitions"]==false&&(z.transitions.fade_in_ms>0||z.transitions.fade_out_ms>0){return Err(ApiError::bad("此输出不执行 R-Music 淡入淡出，请先关闭淡入淡出"));}
    }Ok(())
}
pub async fn endpoints(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let mut items: Vec<_> = s.endpoints.read().await.values().cloned().collect();
    let owners=crate::accounts::scope::owners(&s,"endpoint").await?;let realm=crate::accounts::scope::realm(&a);
    items.retain(|e|owners.get(e["id"].as_str().unwrap_or("")).map(String::as_str).unwrap_or("family")==realm);
    for endpoint in &mut items{crate::device_groups::decorate(&s,endpoint).await;}
    let bridges=s.bridges.read().await.clone();let now=crate::clock::now_ns();
    for endpoint in &mut items {if endpoint["capabilities"]["host_clock_v1"]==true {endpoint["clock_calibration"]=if let Some(tx)=bridges.get(endpoint["id"].as_str().unwrap_or("")){s.host_clock_estimate(endpoint["id"].as_str().unwrap_or(""),tx,now).await}else{Value::Null};}}
    crate::portable::overlay(&mut items,crate::portable::endpoints(&s,&a).await);
    Ok(Json(json!({"items":items})))
}
#[derive(Deserialize)]pub struct BrowserEndpoint{id:String,name:String,#[serde(default)]visualization_v1:bool,#[serde(default)]registration_id:Option<String>,#[serde(default)]output_config:Option<Value>,#[serde(default)]platform:String,#[serde(default)]playback_instance:bool,#[serde(default)]continuous_pcm:bool,#[serde(default)]continuous_recovery:bool,#[serde(default)]measured_listens_v2:bool,#[serde(default)]measured_listens_recovery_v2:bool,#[serde(default)]recovery_controls:bool,#[serde(default)]convolution_v1:bool}
pub async fn browser_register(State(s):State<AppState>,a:Auth,Json(v):Json<BrowserEndpoint>)->ApiResult<Json<Value>>{
    a.require("control")?;if !v.id.starts_with("browser-")||v.id.len()>100||!v.id.chars().all(|c|c.is_ascii_alphanumeric()||c=='-')||v.name.is_empty()||v.name.len()>100{return Err(ApiError::bad("浏览器端点参数无效"));}
    let endpoint=json!({"id":v.id,"name":v.name,"kind":"browser","online":true,"verified":false,"last_seen_ms":chrono::Utc::now().timestamp_millis(),"capabilities":{"range":true,"dsp":"browser_or_native_client","strict_sync":false,"playback_instance":v.playback_instance,"continuous_pcm":v.continuous_pcm&&v.playback_instance,"continuous_recovery":v.continuous_recovery&&v.continuous_pcm&&v.playback_instance}});
    if v.registration_id.as_ref().is_some_and(|id|id.is_empty()||id.len()>100||!id.chars().all(|c|c.is_ascii_alphanumeric()||c=='-')) {return Err(ApiError::bad("浏览器注册实例无效"));}
    let mut endpoint=endpoint;endpoint["registration_id"]=json!(v.registration_id);endpoint["capabilities"]["visualization_v1"]=json!(v.visualization_v1);endpoint["capabilities"]["recovery_controls"]=json!(v.recovery_controls&&endpoint["capabilities"]["continuous_recovery"]==true);
    endpoint["capabilities"]["measured_listens_v2"]=json!(v.measured_listens_v2&&v.continuous_pcm&&v.playback_instance);
    endpoint["capabilities"]["measured_listens_recovery_v2"]=json!(v.measured_listens_recovery_v2&&endpoint["capabilities"]["measured_listens_v2"]==true&&endpoint["capabilities"]["continuous_recovery"]==true);
    endpoint["capabilities"]["convolution_v1"]=json!(v.convolution_v1&&v.continuous_pcm&&v.playback_instance);
    if v.platform=="android" {if let Some(config)=v.output_config{let config:rmusic_audio::output::OutputConfig=serde_json::from_value(config).map_err(|_|ApiError::bad("Android 输出配置无效"))?;config.limits.validate().map_err(|e|ApiError::bad(e.to_string()))?;endpoint["output_config"]=json!(config);}}
    crate::accounts::scope::claim(&s,&a,"endpoint",&v.id).await?;
    crate::accounts::scope::claim(&s,&a,"device",&v.id).await?;
    crate::devices::remember_client(&s,&mut endpoint,if v.platform=="android"{"android"}else{"browser"}).await?;
    let _lock=s.zone_lock.lock().await;
    if let Some(closed)=s.endpoints.read().await.get(&v.id).filter(|e|v.registration_id.is_some()&&e["closed_registration_id"]==json!(v.registration_id)).cloned(){return Ok(Json(closed));}
    if crate::devices::is_removed(&s,&v.id).await? {return Ok(Json(json!({"id":v.id,"name":v.name,"kind":"browser","online":false,"removed":true,"capabilities":{}})));}
    crate::sequence::recovery::register_listener(&s,&v.id,&a.id,&a.session_hash).await?;
    let previous=s.endpoints.read().await.get(&v.id).cloned();
    let same_owner=s.browser_owners.read().await.get(&v.id)==Some(&a.session_hash);
    if same_owner&&previous.as_ref().is_some_and(|old|old["registration_id"]==endpoint["registration_id"]) {
        for key in ["playback","device_volume","ready_playback_id"] {if let Some(value)=previous.as_ref().and_then(|old|old.get(key)){endpoint[key]=value.clone();}}
    }
    let changed=previous.as_ref().is_none_or(|old|old["online"]!=true||old["name"]!=endpoint["name"]||old["capabilities"]!=endpoint["capabilities"]||old["registration_id"]!=endpoint["registration_id"]);
    s.endpoints.write().await.insert(v.id.clone(),endpoint.clone());
    s.browser_owners.write().await.insert(v.id.clone(),a.session_hash.clone());
    let first=if a.role=="external"{None}else{s.zones.read().await.values().find(|z|z.endpoint_id=="browser-local").cloned()};
    if let Some(mut z)=first{z.endpoint_id=v.id.clone();z.status="stopped".into();z.playback_id=None;s.save_zone(z).await?;}
    else if !s.zones.read().await.values().any(|z|z.endpoint_id==v.id){let zone=Zone::new(uuid::Uuid::new_v4().to_string(),v.name,v.id);crate::accounts::scope::claim(&s,&a,"zone",&zone.id).await?;s.save_zone(zone).await?;}
    if changed {s.emit("endpoints",json!({}));}
    Ok(Json(endpoint))
}
#[derive(Deserialize)]pub struct BrowserDisconnect {registration_id:Option<String>}
pub async fn browser_disconnect(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Query(v):Query<BrowserDisconnect>)->ApiResult<StatusCode> {
    a.require("control")?;
    let _lock=s.zone_lock.lock().await;
    if s.browser_owners.read().await.get(&id)!=Some(&a.session_hash){return Ok(StatusCode::NO_CONTENT);}
    if s.endpoints.read().await.get(&id).is_some_and(|e|!e["registration_id"].is_null()&&e["registration_id"]!=json!(v.registration_id)){return Ok(StatusCode::NO_CONTENT);}
    let zones:Vec<_>=s.zones.read().await.values().filter(|z|z.endpoint_id==id).cloned().collect();
    for zone in zones {crate::autoplay::cancel(&s,&zone.id).await;crate::stats::finish(&s,&zone,"disconnect").await?;crate::sequence::recovery::suspend(&s,&zone).await?;}
    s.browser_owners.write().await.remove(&id);
    if let Some(endpoint)=s.endpoints.write().await.get_mut(&id){endpoint["online"]=json!(false);endpoint["closed_registration_id"]=json!(v.registration_id);}
    s.emit("endpoints",json!({}));Ok(StatusCode::NO_CONTENT)
}
pub fn monitor_browsers(s:AppState){tokio::spawn(async move{let mut interval=tokio::time::interval(std::time::Duration::from_secs(30));loop{interval.tick().await;let _lock=s.zone_lock.lock().await;let now=chrono::Utc::now().timestamp_millis();let offline:Vec<_>=s.endpoints.read().await.values().filter(|e|e["kind"]=="browser"&&e["online"]==true&&e["last_seen_ms"].as_i64().is_some_and(|seen|now-seen>90000)).filter_map(|e|e["id"].as_str().map(String::from)).collect();if offline.is_empty(){continue;}for id in &offline{if let Some(e)=s.endpoints.write().await.get_mut(id){e["online"]=json!(false);}}let zones:Vec<_>=s.zones.read().await.values().filter(|z|offline.contains(&z.endpoint_id)).cloned().collect();for z in zones{crate::autoplay::cancel(&s,&z.id).await;if z.status=="playing"{let _=crate::stats::finish(&s,&z,"disconnect").await;}if let Err(error)=crate::sequence::recovery::suspend(&s,&z).await{tracing::warn!(%error,zone_id=z.id,"transport suspension failed");}}s.emit("endpoints",json!({}));}});}
pub async fn dispatch(s: &AppState, z: &Zone, kind: &str) -> ApiResult<()> {
    if matches!(kind,"play"|"seek"|"dsp"){if let Some(p)=crate::accounts::scope::zone_policy(s,&z.id).await?{if let Some(t)=z.track.as_ref(){crate::accounts::scope::track(s,&p,&t.id).await?;}}}
    if matches!(kind,"play"|"seek"|"dsp"){processing_allowed(s,z).await?;}
    let Some(tx) = s.bridges.read().await.get(&z.endpoint_id).cloned() else {
        if z.endpoint_id != "browser-local" && !matches!(kind,"stop"|"pause") && !s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["kind"]=="browser"&&e["online"]==true) {
            let endpoint=s.endpoints.read().await.get(&z.endpoint_id).cloned();
            let message=match endpoint {
                Some(ref endpoint) if endpoint["kind"]=="browser" => "客户端输出离线，请打开对应客户端并重新连接",
                Some(ref endpoint) if endpoint["online"]!=true => "所选远程设备已离线，请切换到本机音区或重新连接该设备",
                Some(_) => "网桥连接已断开，请重新连接对应设备",
                None if z.endpoint_id.starts_with("browser-") => "客户端输出尚未注册，请在对应客户端重新连接或选择本机音区",
                None => "所选设备尚未连接，请检查设备连接状态",
            };
            return Err(ApiError::bad(message));
        }
        return Ok(());
    };
    let mut v = json!({"type":kind,"zone_id":z.id,"playback_id":z.playback_id,"endpoint_id":z.endpoint_id,"track":z.track,"position":z.position,"volume":z.volume,"muted":z.muted,"dsp":z.dsp,"transitions":z.transitions});
    let rendered = crate::render_media::supported(s, &z.endpoint_id).await;
    if rendered { v["core_pcm_dsp"]=json!(true); }
    let durable=s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["continuous_recovery"]==true);
    if kind == "play" || (durable||rendered)&&matches!(kind,"seek"|"dsp")&&z.playback_id.is_some() {
        v["paused"]=json!(z.status=="paused");
        if let Some(t) = &z.track {
            let owner=s.bridge_owners.read().await.get(&z.endpoint_id).cloned();
            let api_path = crate::render_media::path(s,z,t,owner).await?;
            let relative = s.endpoints.read().await.get(&z.endpoint_id)
                .is_some_and(|e|e["capabilities"]["media_url_api_relative"]==true);
            let base = s
                .config
                .public_url
                .as_deref()
                .unwrap_or("http://127.0.0.1:7800");
            v["url"] = json!(if relative {api_path} else {format!("{base}{}/{api_path}",s.config.base_path)});
        }
    }
    tx.try_send(v)
        .map_err(|_| ApiError::bad("网桥连接已中断"))?;
    Ok(())
}
