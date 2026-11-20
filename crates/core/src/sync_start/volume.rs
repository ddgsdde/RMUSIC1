//! Volume-only zone edits keep the active cohort and its output/statistics identity.
use super::*;

/// Caller holds zone_lock. Other zone edits retain their ordinary cancellation semantics.
pub(crate) async fn patch(s:&AppState,id:&str,value:&Value)->ApiResult<Option<Zone>> {
    let Some(fields)=value.as_object().filter(|v|!v.is_empty()&&v.keys().all(|k|matches!(k.as_str(),"volume"|"muted")))else{return Ok(None);};
    let mut registry=s.sync_starts.lock().await;
    let Some(job)=registry.jobs.values_mut().find(|job|job.active()&&job.owns_zone(id))else{return Ok(None);};
    let volume=fields.get("volume").map(|v|v.as_f64().filter(|v|v.is_finite()&&(0. ..=1.).contains(v)).ok_or_else(||ApiError::bad("音量范围为 0 到 1"))).transpose()?;
    let muted=fields.get("muted").map(|v|v.as_bool().ok_or_else(||ApiError::bad("静音设置须为布尔值"))).transpose()?;
    let index=job.members.iter().position(|m|m.zone.id==id).unwrap();let member=&job.members[index];
    let conflict=||ApiError(StatusCode::CONFLICT,"group_volume_busy".into(),"请等待整组起播或收尾完成后调节音量".into());
    if !matches!(job.phase.as_str(),"playing"|"paused"){return Err(conflict());}
    let sending=job.phase=="playing"&&member.phase!="ended";
    let permit=if sending {
        if s.endpoints.read().await.get(&member.zone.endpoint_id).is_none_or(|e|e["capabilities"]["group_volume_v1"]!=true){return Err(ApiError(StatusCode::CONFLICT,"group_volume_unsupported".into(),"请更新此输出端后再调节分组音量".into()));}
        if !valid_gate(member,&json!({"gate":member.gate}),"released")||!s.bridges.read().await.get(&member.zone.endpoint_id).is_some_and(|tx|tx.same_channel(&member.tx)){return Err(conflict());}
        Some(member.tx.clone().try_reserve_owned().map_err(|_|conflict())?)
    }else{None};
    let mut zone=s.zones.read().await.get(id).cloned().ok_or_else(ApiError::missing)?;
    if let Some(volume)=volume{zone.volume=volume;}if let Some(muted)=muted{zone.muted=muted;}
    let command=json!({"type":"group_volume","zone_id":zone.id,"endpoint_id":zone.endpoint_id,"playback_id":member.playback_id,"start":member.token,"measured_version":job.request.measured_version,"group_queue":job.queue.is_some(),"volume":zone.volume,"muted":zone.muted});
    let mut next=job.clone();next.members[index].zone.volume=zone.volume;next.members[index].zone.muted=zone.muted;
    next.members[index].original.volume=zone.volume;next.members[index].original.muted=zone.muted;
    let saved=zone.clone();let stored=next.clone();let at=chrono::Utc::now().timestamp_millis();
    s.db.run(move|c|{let tx=c.transaction()?;
        tx.execute("UPDATE zones SET data=?2 WHERE id=?1",rusqlite::params![saved.id,serde_json::to_string(&saved)?])?;
        tx.execute("UPDATE sync_starts SET result=?2,updated_at=?3 WHERE id=?1",rusqlite::params![stored.request.request_id,stored.value().to_string(),at])?;tx.commit()?;Ok(())
    }).await?;
    *job=next;s.zones.write().await.insert(zone.id.clone(),zone.clone());
    if let Some(permit)=permit{permit.send(command);}
    s.emit("zones_changed",json!({}));Ok(Some(zone))
}

/// All members of a logical device change together, after every channel is reserved.
/// Caller holds zone_lock. No command is sent before the SQLite transaction commits.
pub(crate) async fn patch_all(s:&AppState,ids:&[String],volume:f64,muted:bool)->ApiResult<()> {
 let mut registry=s.sync_starts.lock().await;
 let found=registry.jobs.iter().find(|(_,job)|job.active()&&job.members.iter().any(|m|ids.contains(&m.zone.id))).map(|(id,job)|(id.clone(),job.clone()));
 let mut updates=Vec::new();let mut commands=Vec::new();let mut next=found.as_ref().map(|(_,job)|job.clone());
 if let Some(job)=&mut next {
  if !matches!(job.phase.as_str(),"playing"|"paused"){return Err(ApiError(StatusCode::CONFLICT,"group_volume_busy".into(),"请等待组合起播或收尾完成后调节音量。".into()));}
  for member in &mut job.members {
   if !ids.contains(&member.zone.id){return Err(ApiError::bad("设备组合的输出成员已变化"));}
   let mut zone=s.zones.read().await.get(&member.zone.id).cloned().ok_or_else(ApiError::missing)?;zone.volume=volume;zone.muted=muted;
   if job.phase=="playing"&&member.phase!="ended" {
    if s.endpoints.read().await.get(&member.zone.endpoint_id).is_none_or(|e|e["capabilities"]["group_volume_v1"]!=true)||!valid_gate(member,&json!({"gate":member.gate}),"released")||!s.bridges.read().await.get(&member.zone.endpoint_id).is_some_and(|tx|tx.same_channel(&member.tx)){return Err(ApiError(StatusCode::CONFLICT,"group_volume_busy".into(),"组合输出尚未全部就绪，音量未改变。".into()));}
    let permit=member.tx.clone().try_reserve_owned().map_err(|_|ApiError::bad("输出控制队列繁忙，音量未改变。"))?;
    commands.push((permit,json!({"type":"group_volume","zone_id":zone.id,"endpoint_id":zone.endpoint_id,"playback_id":member.playback_id,"start":member.token,"measured_version":job.request.measured_version,"group_queue":job.queue.is_some(),"volume":volume,"muted":muted})));
   }
   member.zone.volume=volume;member.zone.muted=muted;member.original.volume=volume;member.original.muted=muted;updates.push(zone);
  }
 } else {
  for id in ids{let mut zone=s.zones.read().await.get(id).cloned().ok_or_else(ApiError::missing)?;zone.volume=volume;zone.muted=muted;updates.push(zone);}
 }
 let saved=updates.clone();let stored=next.clone();
 s.db.run(move|c|{let tx=c.transaction()?;for zone in saved{tx.execute("UPDATE zones SET data=?2 WHERE id=?1",rusqlite::params![zone.id,serde_json::to_string(&zone)?])?;}if let Some(job)=stored{tx.execute("UPDATE sync_starts SET result=?2,updated_at=?3 WHERE id=?1",rusqlite::params![job.request.request_id,job.value().to_string(),chrono::Utc::now().timestamp_millis()])?;}tx.commit()?;Ok(())}).await?;
 if let Some(job)=next{registry.jobs.insert(job.request.request_id.clone(),job);}
 s.zones.write().await.extend(updates.into_iter().map(|zone|(zone.id.clone(),zone)));
 for (permit,command) in commands {permit.send(command);}s.emit("zones_changed",json!({}));Ok(())
}
