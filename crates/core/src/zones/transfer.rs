//! Prepare the destination silently, then move the queue. Source survives a failed admission.
use super::*;
use std::time::{Duration,Instant};

#[derive(Deserialize)]
pub struct Request { target_zone_id:String, expected_playback_id:Option<String> }
fn stamp(z:&Zone)->Value {json!([z.endpoint_id,z.playback_id,z.status,z.queue_index,z.queue.iter().map(|t|&t.id).collect::<Vec<_>>(),z.repeat,z.shuffle,z.autoplay,z.track.as_ref().map(|t|&t.id)])}
fn position(z:&Zone)->f64 {if z.status=="playing"{super::current_position(z)}else{z.position}}
fn conflict()->ApiError {ApiError(StatusCode::CONFLICT,"transfer_changed".into(),"流转期间播放状态已改变，原设备保留，请重新选择".into())}

pub async fn start(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Request>)->ApiResult<Json<Value>> {
    a.require("control")?;
    // A dropped HTTP response must not strand a half-prepared destination.
    tokio::spawn(async move {run(&s,&a,&id,v).await}).await.map_err(|e|ApiError::from(anyhow::Error::from(e)))?
}
async fn run(s:&AppState,a:&Auth,id:&str,v:Request)->ApiResult<Json<Value>> {
    if crate::device_groups::by_zone(s,id).await?.is_some()||crate::device_groups::by_zone(s,&v.target_zone_id).await?.is_some(){
        return crate::device_groups::transfer::run(s,a,id,&v.target_zone_id,v.expected_playback_id).await;
    }
    for zone in [id,v.target_zone_id.as_str()]{crate::accounts::scope::check(s,a,"zone",zone).await?;}
    let (source,old_target,candidate)={
        let _lock=s.zone_lock.lock().await;
        let zones=s.zones.read().await;
        let source=zones.get(id).cloned().ok_or_else(ApiError::missing)?;
        let old=zones.get(&v.target_zone_id).cloned().ok_or_else(ApiError::missing)?;
        if source.id==old.id||source.endpoint_id==old.endpoint_id {return Err(ApiError::bad("请选择另一个媒体设备"));}
        if source.playback_id!=v.expected_playback_id {return Err(conflict());}
        if source.queue.is_empty()||source.track.is_none() {return Err(ApiError::bad("来源设备没有可流转的播放队列"));}
        if zones.values().any(|z|z.id!=old.id&&z.endpoint_id==old.endpoint_id&&z.status=="playing") {return Err(ApiError::bad("目标输出的其他音区正在播放，请先停止"));}
        drop(zones);
        crate::device_groups::member_guard(s,&source.id,None).await?;crate::device_groups::member_guard(s,&old.id,None).await?;
        if crate::device_groups::by_zone(s,&source.id).await?.is_some()||crate::device_groups::by_zone(s,&old.id).await?.is_some(){return Err(ApiError::bad("组合播放请直接选择组合设备并播放队列；暂不支持把正在播放的整组流转到单设备。"));}
        if crate::sync_start::active_zone(s,&source.id).await||crate::sync_start::active_zone(s,&old.id).await {return Err(ApiError::bad("请先结束分组播放，再流转到单个设备"));}
        if !s.endpoints.read().await.get(&old.endpoint_id).is_some_and(|e|e["online"]==true&&e["capabilities"]["playback_instance"]==true) {return Err(ApiError::bad("目标设备离线或未支持播放实例确认，请打开并连接目标客户端"));}
        let mut target=old.clone();target.queue=source.queue.clone();target.queue_index=source.queue_index;target.track=source.track.clone();target.position=position(&source);
        target.repeat=source.repeat.clone();target.shuffle=source.shuffle;target.autoplay=source.autoplay.clone();
        target.status=if source.status=="stopped"{"stopped"}else{"paused"}.into();
        target.playback_id=if target.status=="stopped"{None}else{Some(uuid::Uuid::new_v4().to_string())};target.updated_at=chrono::Utc::now().timestamp_millis();
        super::processing_allowed(s,&target).await?;
        crate::autoplay::cancel(s,&old.id).await;
        crate::stats::finish(s,&old,"transfer").await?;
        super::dispatch(s,&target,if target.playback_id.is_some(){"play"}else{"stop"}).await?;
        s.save_zone_action(target.clone(),Some("queue")).await?;
        (source,old,target)
    };
    let result=finish(s,&source,&candidate).await;
    if result.is_err() {
        let _lock=s.zone_lock.lock().await;
        let current=s.zones.read().await.get(&candidate.id).cloned();
        if current.as_ref().is_some_and(|z|z.playback_id==candidate.playback_id&&stamp(z)==stamp(&candidate)) {
            let mut restored=old_target;restored.updated_at=chrono::Utc::now().timestamp_millis();
            if restored.playback_id.is_some(){restored.renew_playback();}
            s.save_zone_action(restored.clone(),Some("queue")).await?;
            if let Err(error)=super::dispatch(s,&restored,if restored.playback_id.is_some(){"play"}else{"stop"}).await {tracing::warn!(message=%error.2,"destination restore failed");}
        }
    }
    result
}
async fn finish(s:&AppState,source:&Zone,candidate:&Zone)->ApiResult<Json<Value>> {
    let deadline=Instant::now()+Duration::from_secs(15);
    loop {
        {
            let _lock=s.zone_lock.lock().await;
            let zones=s.zones.read().await;
            let mut origin=zones.get(&source.id).cloned().ok_or_else(conflict)?;
            let mut target=zones.get(&candidate.id).cloned().ok_or_else(conflict)?;
            if stamp(&origin)!=stamp(source)||stamp(&target)!=stamp(candidate){return Err(conflict());}
            drop(zones);
            let endpoint=s.endpoints.read().await.get(&target.endpoint_id).cloned().ok_or_else(conflict)?;
            if endpoint["online"]!=true {return Err(ApiError::bad("目标设备已离线，原设备继续保留"));}
            let ready=target.playback_id.is_none()||endpoint["ready_playback_id"].as_str()==target.playback_id.as_deref()
                || endpoint["playback"]["core_playback_id"].as_str()==target.playback_id.as_deref()&&matches!(endpoint["playback"]["status"].as_str(),Some("playing"|"paused"));
            if ready {
                // Keep target hardware volume, DSP and format policy; only listening state travels.
                target.status=source.status.clone();target.position=position(&origin);
                origin.position=target.position;origin.status="stopped".into();origin.playback_id=None;
                let now=chrono::Utc::now().timestamp_millis();target.updated_at=now;origin.updated_at=now;
                if target.playback_id.is_some()&&(target.position-candidate.position).abs()>0.3 {target.renew_playback();}
                super::dispatch(s,&target,if target.playback_id.is_some(){"play"}else{"stop"}).await?;
                crate::stats::finish(s,&origin,"transfer").await?;
                crate::autoplay::cancel(s,&origin.id).await;
                super::dispatch(s,&origin,"stop").await?;
                s.revise_queues(std::slice::from_mut(&mut target)).await;
                let saved=[origin.clone(),target.clone()];
                s.db.run(move|c|{let tx=c.transaction()?;for zone in saved {tx.execute("UPDATE zones SET data=?2 WHERE id=?1",rusqlite::params![zone.id,serde_json::to_string(&zone)?])?;crate::sequence::recovery::reconcile(&tx,&zone,Some("queue"))?;}tx.commit()?;Ok(())}).await?;
                {let mut zones=s.zones.write().await;zones.insert(origin.id.clone(),origin.clone());zones.insert(target.id.clone(),target.clone());}
                crate::sequence::invalidate_zone(s,&origin.id,Some(&origin)).await;crate::sequence::invalidate_zone(s,&target.id,Some(&target)).await;
                s.emit("zones_changed",json!({}));
                return Ok(Json(json!({"source":origin,"target":target})));
            }
        }
        if Instant::now()>=deadline{return Err(ApiError::bad("目标设备未能准备播放，请检查音频输出或浏览器自动播放权限；原设备保留"));}
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub async fn ready(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Value>)->ApiResult<StatusCode> {
    a.require("control")?;let _lock=s.zone_lock.lock().await;
    let z=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
    authorize_output(&s,&a.session_hash,&z.endpoint_id).await?;
    if z.playback_id.is_some()&&super::report_matches(&s,&z,v["track_id"].as_str(),v["playback_id"].as_str()).await {
        if let Some(endpoint)=s.endpoints.write().await.get_mut(&z.endpoint_id){endpoint["ready_playback_id"]=json!(z.playback_id);}
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) async fn authorize_output(s:&AppState,session:&str,id:&str)->ApiResult<()> {
    let endpoint=s.endpoints.read().await.get(id).cloned().ok_or_else(ApiError::missing)?;
    if endpoint["online"]!=true||endpoint["capabilities"]["playback_instance"]!=true{return Err(ApiError::bad("当前输出离线或没有播放实例能力"));}
    let owner=if endpoint["kind"]=="browser" {s.browser_owners.read().await.get(id).cloned()}else{s.bridge_owners.read().await.get(id).cloned()};
    if owner.as_deref()!=Some(session){return Err(ApiError::forbidden());}Ok(())
}
