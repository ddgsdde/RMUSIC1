use super::*;
use std::time::{Duration,Instant};

fn signature(z:&Zone)->Value{json!([z.endpoint_id,z.playback_id,z.status,z.queue_index,z.queue.iter().map(|t|&t.id).collect::<Vec<_>>(),z.repeat,z.shuffle])}
fn moved_position(z:&Zone)->f64{if z.status=="playing"{crate::zones::current_position(z)}else{z.position}}
pub(crate) async fn run(s:&AppState,a:&Auth,source_id:&str,target_id:&str,expected:Option<String>)->ApiResult<Json<Value>>{
    family(a,"control")?;
    let _operation=s.device_group_lock.lock().await;
    let (source,previous,mut target,mut source_group,mut target_group)={
        let _lock=s.zone_lock.lock().await;
        for id in [source_id,target_id]{crate::accounts::scope::check(s,a,"zone",id).await?;member_guard(s,id,None).await?;}
        let zones=s.zones.read().await;
        let source=zones.get(source_id).cloned().ok_or_else(ApiError::missing)?;
        let previous=zones.get(target_id).cloned().ok_or_else(ApiError::missing)?;
        if source.id==previous.id||source.endpoint_id==previous.endpoint_id||source.queue.is_empty()||source.track.is_none(){return Err(ApiError::bad("请选择有歌曲的来源和不同的目标设备"));}
        if source.playback_id!=expected{return Err(ApiError(StatusCode::CONFLICT,"transfer_changed".into(),"来源播放状态已改变，请重试".into()));}
        drop(zones);
        crate::accounts::scope::check(s,a,"endpoint",&source.endpoint_id).await?;
        crate::accounts::scope::check(s,a,"endpoint",&previous.endpoint_id).await?;
        let target_group=by_zone(s,target_id).await?;let source_group=by_zone(s,source_id).await?;
        if !s.endpoints.read().await.get(&previous.endpoint_id).is_some_and(|e|e["online"]==true){return Err(ApiError::bad("目标设备离线，原设备保留"));}
        let mut target=previous.clone();
        target.queue=source.queue.clone();target.queue_index=source.queue_index;target.track=source.track.clone();
        target.repeat=source.repeat.clone();target.shuffle=source.shuffle;target.autoplay=source.autoplay.clone();target.position=moved_position(&source);
        target.status=if source.status=="stopped"{"stopped"}else{"paused"}.into();
        target.playback_id=if source.status=="stopped"{None}else{Some(uuid::Uuid::new_v4().to_string())};
        (source,previous,target,source_group,target_group)
    };
    let result=async {
        if target_group.is_none(){
            crate::autoplay::cancel(s,&previous.id).await;
            crate::stats::finish(s,&previous,"transfer").await?;
        }
        if source.status=="playing"{
            if let Some(g)=&mut target_group{
                target.status="playing".into();
                transport::start(s,a,g,&target).await?;
                let deadline=Instant::now()+Duration::from_secs(20);
                loop{
                    let id=g.job_id.clone().ok_or_else(||ApiError::bad("组合没有准备任务"))?;
                    let job=transport::job(s,&id).await?.ok_or_else(ApiError::missing)?;
                    if matches!(job["phase"].as_str(),Some("committed"|"playing")){break;}
                    if matches!(job["phase"].as_str(),Some("failed"|"cancelled"|"completed")){return Err(ApiError::bad("目标组合未能准备播放，来源保留"));}
                    if Instant::now()>=deadline{return Err(ApiError::bad("目标组合准备超时，来源保留"));}
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            }else{
                {
                    let _lock=s.zone_lock.lock().await;
                    crate::zones::processing_allowed(s,&target).await?;
                    crate::zones::dispatch(s,&target,"play").await?;
                    s.save_zone_action(target.clone(),Some("queue")).await?;
                }
                let deadline=Instant::now()+Duration::from_secs(15);
                loop{
                    let endpoint=s.endpoints.read().await.get(&target.endpoint_id).cloned().unwrap_or(Value::Null);
                    if endpoint["online"]!=true{return Err(ApiError::bad("目标设备已离线，来源保留"));}
                    if endpoint["ready_playback_id"].as_str()==target.playback_id.as_deref()
                        || endpoint["playback"]["core_playback_id"].as_str()==target.playback_id.as_deref()&&matches!(endpoint["playback"]["status"].as_str(),Some("playing"|"paused")){break;}
                    if Instant::now()>=deadline{return Err(ApiError::bad("目标设备未能准备播放，来源保留"));}
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        }
        if source.status!="playing"{
            if let Some(g)=&mut target_group{transport::stop(s,a,g).await?;}
        }
        if target_group.is_none()&&source.status=="playing"{
            let current=s.zones.read().await.get(&target.id).cloned().ok_or_else(ApiError::missing)?;
            if signature(&current)!=signature(&target){return Err(ApiError(StatusCode::CONFLICT,"transfer_changed".into(),"准备期间目标已改变，保留当前播放".into()));}
        }
        let latest=s.zones.read().await.get(&source.id).cloned().ok_or_else(ApiError::missing)?;
        if signature(&latest)!=signature(&source){return Err(ApiError(StatusCode::CONFLICT,"transfer_changed".into(),"准备期间来源已改变，保留当前播放".into()));}
        let position=moved_position(&latest);
        if target_group.is_none(){
            let prepared_position=target.position;
            target.status=source.status.clone();target.position=position;
            if target.playback_id.is_some()&&(target.position-prepared_position).abs()>0.3{target.renew_playback();}
            crate::zones::dispatch(s,&target,if target.playback_id.is_some(){"play"}else{"stop"}).await?;
        }else if source.status=="playing"{
            target=s.zones.read().await.get(&target.id).cloned().unwrap_or_else(||target.clone());
        }else{
            if let Some(g)=&mut target_group{g.job_id=None;g.phase="ready".into();save(s,g).await?;}
        }
        if let Some(g)=&mut source_group{transport::stop(s,a,g).await?;}
        else{
            let _lock=s.zone_lock.lock().await;
            crate::stats::finish(s,&latest,"transfer").await?;
            crate::autoplay::cancel(s,&latest.id).await;
            crate::zones::dispatch(s,&latest,"stop").await?;
        }
        let _lock=s.zone_lock.lock().await;
        let mut origin=s.zones.read().await.get(&source.id).cloned().ok_or_else(ApiError::missing)?;
        origin.status="stopped".into();origin.position=position;origin.playback_id=None;
        let now=chrono::Utc::now().timestamp_millis();origin.updated_at=now;target.updated_at=now;
        s.revise_queues(std::slice::from_mut(&mut target)).await;
        let values=[origin.clone(),target.clone()];
        s.db.run(move|c|{let tx=c.transaction()?;for z in values{tx.execute("UPDATE zones SET data=?2 WHERE id=?1",params![z.id,serde_json::to_string(&z)?])?;crate::sequence::recovery::reconcile(&tx,&z,Some("queue"))?;}tx.commit()?;Ok(())}).await?;
        {let mut zones=s.zones.write().await;zones.insert(origin.id.clone(),origin.clone());zones.insert(target.id.clone(),target.clone());}
        crate::sequence::invalidate_zone(s,&origin.id,Some(&origin)).await;
        crate::sequence::invalidate_zone(s,&target.id,Some(&target)).await;
        s.emit("zones_changed",json!({}));s.emit("device_groups",json!({}));
        Ok(Json(json!({"source":origin,"target":target})))
    }.await;
    if result.is_err(){
        let ours=target_group.is_some()||s.zones.read().await.get(&target.id).is_some_and(|z|signature(z)==signature(&target));
        if !ours{return result;}
        if let Some(g)=&mut target_group{let _=transport::stop(s,a,g).await;}
        let mut restore=previous;
        if let Some(g)=&mut target_group{
            restore.playback_id=None;s.save_zone(restore.clone()).await?;
            if restore.status=="playing"{
                if transport::start(s,a,g,&restore).await.is_err(){restore.status="paused".into();s.save_zone(restore.clone()).await?;}
            }
        }else{
            if restore.playback_id.is_some(){restore.renew_playback();}
            s.save_zone(restore.clone()).await?;
            let _=crate::zones::dispatch(s,&restore,if restore.playback_id.is_some(){"play"}else{"stop"}).await;
        }
        s.emit("zones_changed",json!({}));
    }
    result
}
