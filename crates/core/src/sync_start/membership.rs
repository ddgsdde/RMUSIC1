//! Membership changes drain the old cohort, then reuse the ordinary common-start barrier.
use super::*;

pub(super) async fn prepare(job:&Job,registry:&Registry,s:&AppState,ids:&[String])->ApiResult<Vec<Member>> {
    let now=crate::clock::now_ns();let mut result=Vec::new();let mut endpoints=HashSet::new();
    let (index,position)=control::playhead(job);let template=&job.members[0].zone;
    let track=if job.queue.is_some(){template.queue.get(index)}else{template.track.as_ref()}.ok_or_else(ApiError::missing)?;
    for id in ids {
        let retained=job.members.iter().find(|m|m.zone.id==*id);
        let original=s.zones.read().await.get(id).cloned().ok_or_else(ApiError::missing)?;
        if !endpoints.insert(original.endpoint_id.clone())||registry.jobs.values().any(|other|other.request.request_id!=job.request.request_id&&other.active()&&(other.owns_zone(id)||other.owns_endpoint(&original.endpoint_id))) {return Err(ApiError::bad("音区或输出已被其他分组占用"));}
        if retained.is_none() {
            if original.status!="stopped"||original.playback_id.is_some()||job.members.iter().any(|m|m.zone.endpoint_id==original.endpoint_id) {return Err(ApiError::bad("加入的音区须使用独立且已停止的输出"));}
            if s.zones.read().await.values().any(|z|z.id!=*id&&z.endpoint_id==original.endpoint_id&&(z.status!="stopped"||z.playback_id.is_some())) {return Err(ApiError::bad("该输出仍被其他音区使用"));}
            let zid=id.clone();if s.db.run(move|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM sequence_recovery WHERE zone_id=?1)",[zid],|r|r.get::<_,bool>(0))?)).await? {return Err(ApiError::bad("音区仍有恢复计划，请先停止"));}
        }
        let endpoint=s.endpoints.read().await.get(&original.endpoint_id).cloned().unwrap_or(Value::Null);
        rate_adjustment::supported(&job.request,&endpoint).map_err(|e|ApiError::bad(e.to_string()))?;
        let tx=s.bridges.read().await.get(&original.endpoint_id).cloned().ok_or_else(||ApiError::bad("输出未连接"))?;
        let estimate=calibration(s,&endpoint,&tx,now).await.map_err(|e|ApiError::bad(e.to_string()))?;
        if job.request.measured_version==2&&(endpoint["capabilities"]["group_output_v2"]!=true||endpoint["capabilities"]["measured_listens_v2"]!=true){return Err(ApiError::bad("输出尚未接入分组双时间协议"));}
        if job.queue.is_some()&&endpoint["capabilities"]["scheduled_queue_v1"]!=true{return Err(ApiError::bad("输出尚未接入连续组队列"));}
        if retained.is_some_and(|m|!tx.same_channel(&m.tx)||estimate["host_clock_id"]!=m.token["host_clock_id"]) {return Err(ApiError::bad("原输出连接或时钟已变化"));}
        let mut zone=retained.map_or_else(||original.clone(),|m|m.zone.clone());zone.track=Some(track.clone());zone.position=position;
        if job.queue.is_some(){zone.queue=template.queue.clone();zone.queue_index=index;zone.repeat="off".into();zone.shuffle=false;zone.autoplay.enabled=false;}
        else if let Some(i)=zone.queue.iter().position(|t|t.id==track.id){zone.queue_index=i;}else{zone.queue.push(track.clone());zone.queue_index=zone.queue.len()-1;}
        crate::zones::processing_allowed(s,&zone).await?;
        let manual_power=group_power(&endpoint).map_err(|e|ApiError::bad(e.to_string()))?;
        if let Some(member)=retained {let mut member=member.clone();member.manual_power=manual_power;result.push(member);}
        else {
            zone.renew_playback();let generation=job.members.iter().filter_map(|m|m.token["generation"].as_u64()).max().unwrap()+1;
            let token=json!({"version":1,"plan_id":job.request.request_id,"generation":generation,"core_clock_id":estimate["core_clock_id"],"host_clock_id":estimate["host_clock_id"],"expires_host_ns":host_time(&estimate,now+PREPARE_NS)?});
            result.push(Member{requested_rate_ppm:0.,clock:clock_feedback::Binding::Unseen,playback_id:zone.playback_id.clone().unwrap(),uncertainty:estimate["network_uncertainty_ns"].as_u64().unwrap_or(250_000_000),host_deadline:0,original,zone,tx,token,manual_power,phase:"paused".into(),rate:0,frames:0,gate:Value::Null,last_seen:now,final_report:true});
        }
    }Ok(result)
}

pub(super) async fn apply(job:&mut Job,s:&AppState,mut next:Job,index:usize,position:f64)->anyhow::Result<()> {
    let mut targets=next.control.as_mut().unwrap().targets.take().unwrap();
    // Copy the final common playhead, not the earlier snapshot taken when the request arrived.
    let template=next.members[0].zone.clone();
    for member in &mut targets {
        if next.queue.is_some(){member.zone.queue=template.queue.clone();member.zone.queue_index=index;member.zone.track=template.queue.get(index).cloned();}
        else{member.zone.track=template.track.clone();}
        member.frames=0;member.rate=0;member.phase="paused".into();member.final_report=true;
    }
    let mut zones=job.members.iter().filter(|m|!targets.iter().any(|target|target.zone.id==m.zone.id)).map(|m|{let mut zone=m.zone.clone();zone.status="stopped".into();zone.playback_id=None;zone}).collect::<Vec<_>>();
    next.members=targets;control::set_playhead(&mut next,index,position);
    if let Some(queue)=&mut next.queue{queue.begin_epoch(index,position,&next.members);}
    zones.extend(next.members.iter().map(|m|m.zone.clone()));
    let indices=next.queue.as_ref().map_or(Vec::new(),|q|q.indices());
    control::apply(job,s,next,indices,zones,None).await
}
