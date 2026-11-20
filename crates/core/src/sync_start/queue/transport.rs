use super::*;

fn live_indices(job:&Job)->Vec<usize> {
    let q=job.queue.as_ref().unwrap();q.indices().into_iter().filter(|index|job.deadline.is_some()||q.credited.contains_key(index)||q.heard.iter().any(|reports|reports.get(index).is_some_and(|p|p.rate>0||p.ended))).collect()
}
pub(in crate::sync_start) async fn abort(job:&mut Job,s:&AppState,reason:&str,cancelled:bool)->anyhow::Result<()> {
    if !job.active(){return Ok(());}
    let mut next=job.clone();next.phase=if cancelled{"cancelled"}else{"failed"}.into();next.error=Some(reason.into());
    if let Some(control)=&mut next.control{control.failed(reason);}
    next.settle_until=(!next.members.iter().all(|m|m.final_report)).then(||crate::clock::now_ns()+QUIET_NS);
    for m in &mut next.members {if m.phase!="ended"{m.phase="cancelled".into();}}
    let zones=if job.control.is_some(){control::stopped_zones(s,job).await}else if job.deadline.is_some(){job.members.iter().map(|m|if job.dispatched{let mut zone=m.zone.clone();zone.status="stopped".into();zone.playback_id=None;zone}else{m.original.clone()}).collect()}else{Vec::new()};
    let indices=live_indices(job);apply(job,s,next,indices,zones,None,Some("stop")).await?;
    crate::sequence::discard_group_preparation(s,&job.request.request_id).await;
    for m in job.members.iter().filter(|m|!m.final_report){let _=m.tx.try_send(json!({"type":"cancel_start","zone_id":m.zone.id,"playback_id":m.playback_id,"start":m.token}));}
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FinalSource {playback_id:String,track_id:String,position:f64,frames:u64,ended:bool}
pub(in crate::sync_start) async fn final_report(job:&mut Job,s:&AppState,i:usize,v:&Value)->anyhow::Result<()> {
    if report_timing(v)!=Some(job.request.measured_version){return Ok(());}
    let now=crate::clock::now_ns();let m=&job.members[i];
    if Some(m.playback_id.as_str())!=v["core_playback_id"].as_str(){return Ok(());}
    if !(matches!(job.phase.as_str(),"cancelled"|"failed")||control::draining(job))||job.settle_until.is_none_or(|until|now>=until)||m.final_report||v["type"]!="start_state"||v["version"]!=1||v["start"]!=m.token||v["final"]!=true||!matches!(v["phase"].as_str(),Some("cancelled"|"expired")){return Ok(());}
    let Some(frames)=v["programme_frames_played"].as_u64()else{return Ok(());};
    let Some(rate)=v["sample_rate"].as_u64().and_then(|n|u32::try_from(n).ok()).filter(|n|(*n==0&&frames==0)||(8000..=768000).contains(n))else{return Ok(());};
    if m.rate>0&&m.rate!=rate||v["gate"]["expires_host_ns"]!=m.token["expires_host_ns"]{return Ok(());}
    let Some(released)=v["gate"]["released_host_ns"].as_u64()else{return Ok(());};
    if (frames>0||released>0)&&(!job.dispatched||!valid_gate(m,v,"released")){return Ok(());}
    let Some(array)=v["sequence_final"].as_array().filter(|array|array.len()<=9)else{return Ok(());};
    let Ok(sources)=serde_json::from_value::<Vec<FinalSource>>(json!(array))else{return Ok(());};
    let mut next=job.clone();let mut indices=Vec::new();let mut previous=None;
    let unreleased=frames==0&&released==0;
    if unreleased&&next.queue.as_ref().unwrap().heard[i].values().any(|p|p.frames>0){return Ok(());}
    for (offset,source) in sources.iter().enumerate(){
        let Some(&index)=next.queue.as_ref().unwrap().known[i].get(&source.playback_id)else{return Ok(());};
        if previous.is_some_and(|old|index!=old+1)||offset+1<sources.len()&&!source.ended||next.members[i].zone.queue.get(index).is_none_or(|t|t.id!=source.track_id){return Ok(());}
        let mut zone=next.members[i].zone.clone();zone.queue_index=index;zone.track=zone.queue.get(index).cloned();zone.playback_id=Some(source.playback_id.clone());zone.position=source.position;
        if unreleased{
            let q=next.queue.as_mut().unwrap();let base=if index==q.segment{q.position}else{0.};
            if index<q.segment||source.frames!=0||!source.position.is_finite()||(source.position-base).abs()>0.002||(next.request.measured_version==2&&source.position!=base){return Ok(());}
            q.heard[i].insert(index,Progress{frames:0,rate,position:source.position,ended:source.ended});
        }else if record(&mut next,i,&zone,source.frames,rate,source.ended).is_err(){return Ok(());}
        if index>=next.members[i].zone.queue_index{next.members[i].zone=zone;next.members[i].frames=source.frames;}
        indices.push(index);previous=Some(index);
    }
    if let (Some(first),Some(last))=(indices.first(),indices.last()){if *first>m.zone.queue_index||*last<m.zone.queue_index{return Ok(());}}
    let q=next.queue.as_ref().unwrap();let total:u64=q.heard[i].range(q.segment..).map(|(_,p)|p.frames).sum();
    if total!=frames{return Ok(());}
    next.members[i].final_report=true;next.members[i].gate=v["gate"].clone();
    if next.members.iter().all(|m|m.final_report){next.settle_until=None;}
    if unreleased&&next.deadline.is_none(){indices.clear();}
    let reason=if control::draining(&next){None}else{Some("stop")};
    apply(job,s,next,indices,Vec::new(),None,reason).await
}
/// Existing /zones/{id}/ended returns a zone; repeated old EOF never advances a new segment.
pub(crate) async fn ended(s:&AppState,a:&Auth,id:&str,track:Option<&str>,playback:Option<&str>,position:Option<f64>,frames:Option<u64>,rate:Option<u32>,version:u32)->ApiResult<Option<Zone>> {
    let public=s.zones.read().await.get(id).cloned().ok_or_else(ApiError::missing)?;
    let mut registry=s.sync_starts.lock().await;
    let Some(job)=registry.jobs.values_mut().find(|j|j.queue.is_some()&&j.members.iter().any(|m|m.zone.id==id)&&(j.active()||playback.is_some_and(|pb|j.queue.as_ref().unwrap().known.iter().any(|known|known.contains_key(pb)))))else{return Ok(None);};
    let i=job.members.iter().position(|m|m.zone.id==id).unwrap();
    crate::sequence::authorize(s,&a.session_hash,&job.members[i].zone.endpoint_id).await?;
    if !s.bridges.read().await.get(&job.members[i].zone.endpoint_id).is_some_and(|tx|tx.same_channel(&job.members[i].tx)){return Ok(Some(public));}
    if !job.active()||job.members[i].zone.playback_id.as_deref()!=playback{return Ok(Some(public));}
    if version!=job.request.measured_version{return Err(ApiError::bad("分组结束计时版本不符"));}
    if control::draining(job){
        let (Some(position),Some(frames),Some(rate))=(position,frames,rate)else{return Err(ApiError::bad("组队列结束须包含实际帧、采样率和位置"));};
        let mut next=job.clone();let mut zone=next.members[i].zone.clone();zone.position=position;
        if zone.track.as_ref().map(|t|t.id.as_str())!=track{return Err(ApiError::bad("组队列结束曲目不符"));}
        record(&mut next,i,&zone,frames,rate,true)?;let index=zone.queue_index;
        next.members[i].zone=zone;next.members[i].frames=frames;next.members[i].phase="ended".into();
        apply(job,s,next,vec![index],Vec::new(),None,None).await?;return Ok(Some(public));
    }
    if job.members[i].phase=="ended"{if job.members.iter().all(|m|m.phase=="ended"){advance(job,s).await?;}return Ok(Some(public));}
    if job.members[i].zone.track.as_ref().map(|t|t.id.as_str())!=track{return Err(ApiError::bad("组队列结束曲目不符"));}
    let (Some(position),Some(frames),Some(rate))=(position,frames,rate)else{return Err(ApiError::bad("组队列结束须包含实际帧、采样率和位置"));};
    let mut next=job.clone();let mut zone=next.members[i].zone.clone();zone.position=position;
    record(&mut next,i,&zone,frames,rate,true)?;let index=zone.queue_index;
    next.members[i].zone=zone.clone();next.members[i].frames=frames;next.members[i].phase="ended".into();next.members[i].final_report=true;next.members[i].last_seen=crate::clock::now_ns();
    zone.status="stopped".into();zone.playback_id=None;
    let unexpected=next.members[i].zone.queue.get(index+1).is_some_and(|t|same_format(next.members[i].zone.track.as_ref().unwrap(),t));
    let all=next.members.iter().all(|m|m.phase=="ended");let finished=all&&index+1>=next.queue.as_ref().unwrap().logical.len();
    if finished{next.phase="ended".into();next.settle_until=None;}
    apply(job,s,next,vec![index],vec![zone.clone()],None,None).await?;
    if unexpected{abort(job,s,"连续组在预载曲序中提前结束",false).await?;}
    else if all&&!finished{if let Err(error)=advance(job,s).await{abort(job,s,&error.to_string(),false).await?;}}
    Ok(Some(zone))
}
pub(in crate::sync_start) async fn advance(job:&mut Job,s:&AppState)->anyhow::Result<()> {
    let index=job.members[0].zone.queue_index+1;
    rearm(job,s,index,0.).await
}
pub(in crate::sync_start) async fn rearm(job:&mut Job,s:&AppState,index:usize,position:f64)->anyhow::Result<()> {
    let estimates=job.calibrations(s).await?;
    let permits=job.members.iter().map(|m|m.tx.clone().try_reserve_owned()).collect::<Result<Vec<_>,_>>().map_err(|_|anyhow::anyhow!("组队列换格式控制队列繁忙"))?;
    let now=crate::clock::now_ns();let mut next=job.clone();control::reset_rate(&mut next);next.expires=now+PREPARE_NS;next.deadline=None;next.dispatched=false;next.phase="preparing".into();next.error=None;
    let queued=next.queue.is_some();
    for (m,estimate) in next.members.iter_mut().zip(estimates){
        if queued{m.zone.queue_index=index;m.zone.track=m.zone.queue.get(index).cloned();}
        m.zone.position=position;m.zone.status="stopped".into();m.zone.renew_playback();m.playback_id=m.zone.playback_id.clone().unwrap();
        m.token=json!({"version":1,"plan_id":next.request.request_id,"generation":m.token["generation"].as_u64().unwrap()+1,"core_clock_id":estimate["core_clock_id"],"host_clock_id":estimate["host_clock_id"],"expires_host_ns":host_time(&estimate,next.expires)?});
        if let Some(stamp)=&m.manual_power{m.token["group_power"]=stamp.clone();}
        m.clock=clock_feedback::Binding::Unseen;m.phase="preparing".into();m.frames=0;m.rate=0;m.host_deadline=0;m.gate=Value::Null;m.last_seen=now;m.final_report=false;
    }
    if let Some(q)=&mut next.queue{
        q.begin_epoch(index,position,&next.members);
        if next.control.is_some()&&!q.completed.contains(&index){q.logical[index]=uuid::Uuid::new_v4().to_string();}
    }else{control::renew(&mut next,position);}
    let commands=next.prepare_commands(s).await?;
    if crate::clock::now_ns()>=next.expires{anyhow::bail!("组队列重新准备已超期");}
    control::apply(job,s,next,Vec::new(),Vec::new(),None).await?;
    crate::sequence::discard_group_preparation(s,&job.request.request_id).await;
    // No await after dispatch: retiring the old worker can cancel its HTTP EOF request.
    for (permit,command) in permits.into_iter().zip(commands){permit.send(command);}
    Ok(())
}
