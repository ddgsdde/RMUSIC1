//! Group transport freezes the old output epoch before preparing a common playhead.
use super::*;

#[derive(Clone,Copy,Deserialize,Serialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
pub enum Action {Pause,Resume,Seek,Jump,Members,Resync}
#[derive(Clone,Deserialize,Serialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    request_id:String,action:Action,
    #[serde(default,skip_serializing_if="Option::is_none")]zone_ids:Option<Vec<String>>,
    #[serde(default,skip_serializing_if="Option::is_none")]from_zone_ids:Option<Vec<String>>,
    #[serde(default,skip_serializing_if="Option::is_none")]position:Option<f64>,
    #[serde(default,skip_serializing_if="Option::is_none")]queue_index:Option<usize>,
    #[serde(default,skip_serializing_if="Option::is_none")]from_queue_index:Option<usize>,
}
#[derive(Clone)]
pub(super) struct Control {
    request:Request,revision:u64,state:String,error:Option<String>,
    pub(super) targets:Option<Vec<Member>>,
    pub(super) epoch_position:f64,
    logical:String,credit:f64,ever_started:bool,was_playing:bool,incomplete:bool,
}
impl Control {
    pub(super) fn value(&self)->Value {json!({"request_id":self.request.request_id,"action":self.request.action,"revision":self.revision,"state":self.state,"error":self.error,"incomplete_final_reports":self.incomplete,"target_zone_ids":self.request.zone_ids})}
    pub(super) fn committed(&mut self){self.state="applied".into();self.ever_started=true;}
    pub(super) fn failed(&mut self,error:&str){self.state="failed".into();self.error=Some(error.into());}
}
pub(super) fn draining(job:&Job)->bool {matches!(job.phase.as_str(),"pausing"|"seeking")}
pub(super) fn base(job:&Job)->f64 {job.control.as_ref().map_or(job.request.position,|c|c.epoch_position)}
pub(super) fn credit(job:&Job)->f64 {job.control.as_ref().map_or(0.,|c|c.credit)}
pub(super) fn ever_started(job:&Job)->bool {job.control.as_ref().is_some_and(|c|c.ever_started)}
pub(super) fn logical(job:&Job)->&str {job.control.as_ref().map_or(job.request.request_id.as_str(),|c|c.logical.as_str())}
pub(super) fn renew(job:&mut Job,position:f64){let control=job.control.as_mut().unwrap();control.logical=uuid::Uuid::new_v4().to_string();control.epoch_position=position;}
/// A new output epoch keeps its listen group and resets the actual-position baseline.
pub(super) fn rebase_tx(tx:&rusqlite::Transaction<'_>,previous:&Job,next:&Job,now:i64)->anyhow::Result<()> {
    if let (Some(old),Some(new))=(&previous.queue,&next.queue) {if old.jump_generation!=new.jump_generation {
        for index in old.indices() {
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM listen_claims WHERE playback_id=?1)",[&old.logical[index]],|r|r.get::<_,bool>(0))? {
                let ended=old.completed.contains(&index)||previous.members.iter().enumerate().all(|(i,_)|old.source_ended(i,index));
                let request=&next.control.as_ref().unwrap().request;let reason=if ended{"ended"}else if request.queue_index<request.from_queue_index{"previous"}else{"next"};
                queue::statistic(tx,previous,index,now,Some(reason))?;
            }
        }return Ok(());
    }}
    let sources=if let (Some(old),Some(new))=(&previous.queue,&next.queue){
        old.logical.iter().zip(&new.logical).enumerate().filter(|(_, (old,new))|old!=new).map(|(index,(old,_))|(old.clone(),queue::listening(next,index).0)).collect::<Vec<_>>()
    }else if logical(previous)!=logical(next){vec![(logical(previous).to_owned(),next.listening().0)]}else{Vec::new()};
    for (old,mut zone) in sources {
        let issued:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM listen_claims WHERE playback_id=?1)",[&old],|r|r.get(0))?;
        if issued{zone.updated_at=now;zone.status="paused".into();crate::stats::restore_sequence_tx(tx,&next.listener,&old,Some(&old),&zone)?;}
    }Ok(())
}
pub(super) fn playhead(job:&Job)->(usize,f64) {
    if job.queue.is_none(){return (0,job.members.iter().map(|m|m.zone.position).fold(0.,f64::max));}
    let member=job.members.iter().max_by(|a,b|a.zone.queue_index.cmp(&b.zone.queue_index).then_with(||a.zone.position.total_cmp(&b.zone.position))).unwrap();
    (member.zone.queue_index,member.zone.position)
}
pub(super) fn record_tx(tx:&rusqlite::Transaction<'_>,job:&Job,now:i64)->anyhow::Result<()> {
    if let Some(control)=&job.control {tx.execute("INSERT OR IGNORE INTO sync_controls(id,group_id,request,created_at) VALUES(?1,?2,?3,?4)",rusqlite::params![control.request.request_id,job.request.request_id,serde_json::to_string(&control.request)?,now])?;}Ok(())
}
pub(super) async fn apply(job:&mut Job,s:&AppState,next:Job,indices:Vec<usize>,zones:Vec<Zone>,reason:Option<&str>)->anyhow::Result<()> {
    if next.queue.is_some(){return queue::apply(job,s,next,indices,zones,None,reason).await;}
    let previous=job.clone();let stored=next.clone();let saved=zones.clone();let reason=reason.map(str::to_owned);let now=chrono::Utc::now().timestamp_millis();
    s.db.run(move|c|{let tx=c.transaction()?;
        rebase_tx(&tx,&previous,&stored,now)?;
        if stored.deadline.is_some()||ever_started(&stored){let(zone,seconds)=stored.listening();crate::stats::group_queue_tx(&tx,&zone,&stored.listener,seconds,now,reason.as_deref())?;}
        for zone in saved {tx.execute("UPDATE zones SET data=?2 WHERE id=?1",rusqlite::params![zone.id,serde_json::to_string(&zone)?])?;}
        record_tx(&tx,&stored,now)?;
        tx.execute("UPDATE sync_starts SET result=?2,updated_at=?3 WHERE id=?1",rusqlite::params![stored.request.request_id,stored.value().to_string(),now])?;tx.commit()?;Ok(())
    }).await?;
    *job=next;if !zones.is_empty(){s.zones.write().await.extend(zones.into_iter().map(|z|(z.id.clone(),z)));s.emit("zones_changed",json!({}));}
    s.emit("sync_start",job.value());crate::device_groups::project(s,&job.value()).await?;s.emit("stats",json!({"group_id":job.request.request_id}));Ok(())
}
pub async fn command(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(mut request):Json<Request>)->ApiResult<(StatusCode,Json<Value>)> {
    a.require("control")?;request.request_id=uuid::Uuid::parse_str(&request.request_id).map_err(|_|ApiError::bad("控制请求须包含 UUID"))?.to_string();
    if matches!(request.action,Action::Seek|Action::Jump) {
        if request.position.is_none_or(|p|!p.is_finite()||p<0.||p>86400.)||request.queue_index.is_none(){return Err(ApiError::bad("定位须指定当前曲序与有效曲内位置"));}
        if (request.action==Action::Jump)!=request.from_queue_index.is_some(){return Err(ApiError::bad("跳曲须同时指定原曲序，曲内定位不接受原曲序参数"));}
    }else if request.position.is_some()||request.queue_index.is_some()||request.from_queue_index.is_some(){return Err(ApiError::bad("暂停或继续不接受定位参数"));}
    if request.action==Action::Members {
        for ids in [&mut request.zone_ids,&mut request.from_zone_ids] {let ids=ids.as_mut().ok_or_else(||ApiError::bad("改组须指定当前与目标音区"))?;ids.sort();if !(1..=4).contains(&ids.len())||ids.iter().any(|id|id.is_empty()||id.len()>200)||ids.windows(2).any(|pair|pair[0]==pair[1]){return Err(ApiError::bad("改组须指定一至四个不重复音区"));}}
        if request.zone_ids==request.from_zone_ids{return Err(ApiError::bad("音区成员没有变化"));}
    }else if request.zone_ids.is_some()||request.from_zone_ids.is_some(){return Err(ApiError::bad("只有改组接受成员参数"));}
    let encoded=serde_json::to_string(&request).map_err(anyhow::Error::from)?;let rid=request.request_id.clone();
    let _zone=s.zone_lock.lock().await;let mut registry=s.sync_starts.lock().await;
    let previous=s.db.run(move|c|Ok(c.query_row("SELECT group_id,request FROM sync_controls WHERE id=?1",[rid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?)).await?;
    if let Some((group,body))=previous {
        if group!=id||body!=encoded{return Err(conflict("控制请求 ID 已用于其他内容"));}
        if let Some(job)=registry.jobs.get(&id){return Ok((StatusCode::OK,Json(job.value())));}
        drop(registry);return Ok((StatusCode::OK,saved(s.clone(),id).await?));
    }
    let targets=if request.action==Action::Members {
        let job=registry.jobs.get(&id).ok_or_else(ApiError::missing)?;
        if !job.active()||draining(job){return Err(conflict("分组已结束或正在等待输出收尾，请查询后重试"));}
        let mut current=job.members.iter().map(|m|m.zone.id.clone()).collect::<Vec<_>>();current.sort();
        if request.from_zone_ids.as_ref()!=Some(&current){return Err(conflict("分组成员已变化，请刷新后重试"));}
        for id in request.zone_ids.as_ref().unwrap(){crate::device_groups::member_guard(&s,id,None).await?;}
        Some(membership::prepare(job,&registry,&s,request.zone_ids.as_ref().unwrap()).await?)
    }else{None};
    let job=registry.jobs.get_mut(&id).ok_or_else(ApiError::missing)?;
    if !job.active()||draining(job){return Err(conflict("分组已结束或正在等待输出收尾，请查询后重试"));}
    let (index,_)=playhead(job);
    if request.action==Action::Seek {
        let track=if job.queue.is_some(){job.members[0].zone.queue.get(index)}else{job.members[0].zone.track.as_ref()}.ok_or_else(ApiError::missing)?;
        if request.queue_index!=Some(index)||track.duration>0.&&request.position.unwrap()>=track.duration||job.queue.as_ref().is_some_and(|q|q.completed.contains(&index)){return Err(conflict("曲目已变化或定位不在当前曲目内，请刷新后重试"));}
    }
    if request.action==Action::Jump {
        if job.queue.is_none()||request.from_queue_index!=Some(index){return Err(conflict("当前曲序已变化或不是连续组队列，请刷新后重试"));}
        let target=request.queue_index.unwrap();let track=job.members[0].zone.queue.get(target).ok_or_else(||ApiError::bad("目标曲序超出队列"))?;
        if target==index||track.duration>0.&&request.position.unwrap()>=track.duration{return Err(ApiError::bad("请选择不同曲序和有效曲内位置；同曲定位请使用 seek"));}
    }
    let mut next=job.clone();let old=next.control.as_ref();
    if matches!(request.action,Action::Resume|Action::Seek|Action::Jump|Action::Resync) {for member in &mut next.members {
        let endpoint=s.endpoints.read().await.get(&member.zone.endpoint_id).cloned().unwrap_or(Value::Null);
        member.manual_power=group_power(&endpoint).map_err(|e|ApiError::bad(e.to_string()))?;
    }}
    next.control=Some(Control{request:request.clone(),revision:old.map_or(1,|c|c.revision+1),state:"pending".into(),error:None,epoch_position:base(job),logical:logical(job).to_owned(),credit:credit(job),ever_started:ever_started(job)||job.dispatched,was_playing:job.phase!="paused",incomplete:false,targets});
    if request.action==Action::Resume&&job.phase=="paused" {
        let (_,position)=playhead(job);queue::rearm(&mut next,&s,index,position).await?;*job=next;
    }else if request.action==Action::Resume||matches!(request.action,Action::Pause|Action::Resync)&&job.phase=="paused" {
        next.control.as_mut().unwrap().state="applied".into();apply(job,&s,next,Vec::new(),Vec::new(),None).await?;
    }else if request.action==Action::Members&&job.phase=="paused" {
        let (_,position)=playhead(job);next.control.as_mut().unwrap().state="applied".into();membership::apply(job,&s,next,index,position).await?;
    }else if matches!(request.action,Action::Seek|Action::Jump)&&job.phase=="paused" {
        let target=if request.action==Action::Jump{request.queue_index.unwrap()}else{index};
        if request.action==Action::Jump{jump_playhead(&mut next,target,request.position.unwrap());}else{seek_playhead(&mut next,target,request.position.unwrap());}
        next.control.as_mut().unwrap().state="applied".into();let zones=next.members.iter().map(|m|m.zone.clone()).collect();let indices=next.queue.as_ref().map_or(Vec::new(),|q|q.indices());apply(job,&s,next,indices,zones,None).await?;
    }else {
        next.phase=if request.action==Action::Pause{"pausing"}else{"seeking"}.into();next.settle_until=Some(crate::clock::now_ns()+QUIET_NS);
        for member in &mut next.members{if !member.final_report{member.phase="pausing".into();}}
        apply(job,&s,next,Vec::new(),Vec::new(),None).await?;crate::sequence::discard_group_preparation(&s,&id).await;
        retry_stop(job);finish(job,&s).await?;
    }
    Ok((StatusCode::ACCEPTED,Json(job.value())))
}
fn conflict(message:&str)->ApiError {ApiError(StatusCode::CONFLICT,"control_conflict".into(),message.into())}
pub(super) async fn realign(job:&mut Job,s:&AppState)->anyhow::Result<()> {
    begin_realign(job,s,false).await
}
async fn begin_realign(job:&mut Job,s:&AppState,buffering:bool)->anyhow::Result<()> {
    if job.phase!="playing"||draining(job){return Ok(());}
    let mut next=job.clone();next.drift.buffer_recovering=buffering;let revision=next.control.as_ref().map_or(1,|c|c.revision+1);
    next.control=Some(Control{request:Request{request_id:uuid::Uuid::new_v4().to_string(),action:Action::Resync,zone_ids:None,from_zone_ids:None,position:None,queue_index:None,from_queue_index:None},
        revision,state:"pending".into(),error:None,epoch_position:base(job),logical:logical(job).into(),credit:credit(job),ever_started:true,was_playing:true,incomplete:false,targets:None});
    next.phase="seeking".into();next.settle_until=Some(crate::clock::now_ns()+QUIET_NS);
    for member in &mut next.members{if !member.final_report{member.phase="pausing".into();}}
    apply(job,s,next,Vec::new(),Vec::new(),None).await?;
    crate::sequence::discard_group_preparation(s,&job.request.request_id).await;retry_stop(job);finish(job,s).await
}
/// Recover the whole group through the existing final-frame barrier, never by changing PCM.
pub(super) async fn recover_underrun(job:&mut Job,s:&AppState)->anyhow::Result<()> {
    if draining(job){return Ok(());}
    let now=crate::clock::now_ns();
    if job.request.measured_version!=2||job.queue.is_none()||job.phase!="playing"{return job.abort(s,"输出欠载，请重新预缓冲后起播",false).await;}
    if job.drift.buffer_recoveries>=3||job.drift.last_buffer_recovery>0&&now.saturating_sub(job.drift.last_buffer_recovery)<30_000_000_000{return job.abort(s,"输出反复欠载，已停止整组；请增加缓冲或检查网络后恢复",false).await;}
    job.drift.buffer_recoveries+=1;job.drift.last_buffer_recovery=now;
    begin_realign(job,s,true).await
}
fn retry_stop(job:&Job){for member in job.members.iter().filter(|m|!m.final_report){let _=member.tx.try_send(json!({"type":"cancel_start","zone_id":member.zone.id,"playback_id":member.playback_id,"start":member.token}));}}
pub(super) fn reset_rate(job:&mut Job) {
    job.sync_monitor=Default::default();
    job.drift=if job.control.as_ref().is_some_and(|c|c.request.action==Action::Resync){job.drift.rearmed()}else{Default::default()};job.rate_request=None;for member in &mut job.members{member.requested_rate_ppm=0.;}
}
pub(super) fn set_playhead(job:&mut Job,index:usize,position:f64) {
    reset_rate(job);
    if let Some(queue)=&mut job.queue{queue.segment=index;queue.position=position;}else if let Some(control)=&mut job.control{control.epoch_position=position;}
    for member in &mut job.members{
        if job.queue.is_some(){member.zone.queue_index=index;member.zone.track=member.zone.queue.get(index).cloned();}
        member.zone.position=position;member.zone.status="paused".into();member.zone.playback_id=None;
    }
}
fn seek_playhead(job:&mut Job,index:usize,position:f64) {
    if let Some(queue)=&mut job.queue{queue.begin_epoch(index,position,&job.members);queue.logical[index]=uuid::Uuid::new_v4().to_string();}
    else{job.control.as_mut().unwrap().logical=uuid::Uuid::new_v4().to_string();}
    set_playhead(job,index,position);
}
fn jump_playhead(job:&mut Job,index:usize,position:f64){job.queue.as_mut().unwrap().restart_from(index,position,&job.members);set_playhead(job,index,position);}
pub(super) async fn finish(job:&mut Job,s:&AppState)->anyhow::Result<()> {
    if !draining(job){return Ok(());}
    if job.members.iter().any(|m|!m.final_report){
        if job.settle_until.is_some_and(|until|crate::clock::now_ns()<until){return Ok(());}
        let mut next=job.clone();next.control.as_mut().unwrap().incomplete=true;next.control.as_mut().unwrap().failed("部分输出未确认停止，整组控制失败");
        next.phase="failed".into();next.error=Some("部分输出未确认最终帧，未继续起播".into());next.settle_until=None;
        let zones=stopped_zones(s,&next).await;let indices=next.queue.as_ref().map_or(Vec::new(),|q|q.indices());apply(job,s,next,indices,zones,Some("stop")).await?;retry_stop(job);return Ok(());
    }
    let (mut index,mut position)=if job.drift.buffer_recovering&&job.control.as_ref().is_some_and(|c|c.request.action==Action::Resync){
        // Replaying a short overlap is preferable to skipping the lagging output's unheard audio.
        job.members.iter().map(|m|(m.zone.queue_index,m.zone.position)).min_by(|a,b|a.0.cmp(&b.0).then_with(||a.1.total_cmp(&b.1))).unwrap_or_else(||playhead(job))
    }else{playhead(job)};let mut next=job.clone();
    let eof=job.members.iter().enumerate().any(|(i,m)|{
        (job.queue.is_none()||m.zone.queue_index==index)&&m.zone.position==position&&job.queue.as_ref().map_or(m.phase=="ended",|q|q.source_ended(i,index))
    });
    if let Some(queue)=&mut next.queue {
        let ended=queue.indices().into_iter().filter(|old|*old<index&&(0..job.members.len()).any(|i|queue.source_ended(i,*old))).collect::<Vec<_>>();
        queue.completed.extend(ended);if eof{queue.completed.insert(index);}queue.seal_epoch();
    }
    else{let seconds=next.members.iter().filter(|m|m.rate>0).map(|m|m.frames as f64/m.rate as f64).fold(0.,f64::max);next.control.as_mut().unwrap().credit+=seconds;}
    let jump=next.control.as_ref().unwrap().request.action==Action::Jump;
    let ended=!jump&&eof&&(next.queue.is_none()||index+1>=next.queue.as_ref().unwrap().logical.len());
    if eof&&!ended&&!jump{index+=1;position=0.;}
    next.settle_until=None;next.dispatched=false;next.deadline=None;next.phase=if ended{"ended"}else{"paused"}.into();
    for m in &mut next.members {m.frames=0;m.rate=0;m.phase=if ended{"ended"}else{"paused"}.into();}
    set_playhead(&mut next,index,position);
    if ended{for member in &mut next.members{member.zone.status="stopped".into();}}
    let members=next.control.as_ref().unwrap().request.action==Action::Members;
    let control=next.control.as_mut().unwrap();let seek=control.request.action==Action::Seek;
    if members&&ended{control.targets=None;control.failed("播放已结束，请重新起播后调整成员");}
    if seek&&(control.request.queue_index!=Some(index)||ended){control.failed("曲目已变化，请重新选择定位");}
    else if seek{position=control.request.position.unwrap();seek_playhead(&mut next,index,position);}
    else if jump{index=control.request.queue_index.unwrap();position=control.request.position.unwrap();jump_playhead(&mut next,index,position);}
    let resync=next.control.as_ref().unwrap().request.action==Action::Resync;
    let resume=(seek||jump||members||resync)&&!ended&&next.control.as_ref().unwrap().state!="failed"&&next.control.as_ref().unwrap().was_playing;
    if !resume&&next.control.as_ref().unwrap().state!="failed"{next.control.as_mut().unwrap().state="applied".into();}
    let zones=next.members.iter().map(|m|m.zone.clone()).collect();let indices=next.queue.as_ref().map_or(Vec::new(),|q|q.indices());if members&&!ended{membership::apply(job,s,next,index,position).await?;}else{apply(job,s,next,indices,zones,if ended{Some("ended")}else{None}).await?;}
    if resume {if let Err(error)=queue::rearm(job,s,index,position).await{let mut next=job.clone();next.control.as_mut().unwrap().failed(&error.to_string());apply(job,s,next,Vec::new(),Vec::new(),None).await?;}}
    Ok(())
}
pub(super) async fn stopped_zones(s:&AppState,job:&Job)->Vec<Zone> {
    let _=s;job.members.iter().map(|member|{let mut zone=member.zone.clone();zone.status="stopped".into();zone.playback_id=None;zone}).collect()
}
pub(super) async fn handle_report(job:&mut Job,s:&AppState,index:usize,v:&Value)->anyhow::Result<()> {
    if job.phase=="paused"{return Ok(());}
    if matches!(v["type"].as_str(),Some("start_state"|"start_error")) {
        if v["start"]!=job.members[index].token||v["core_playback_id"]!=job.members[index].playback_id{return Ok(());}
        if v["type"]=="start_error"{return job.abort(s,"端点拒绝整组停止",false).await;}
        if v["final"]==true{job.final_report(s,index,v).await?;}
    }else if job.queue.is_some(){queue::report(job,s,index,v).await?;}
    else if !job.members[index].final_report {
        if v["type"]=="error"||v["continuity_lost"]==true{return job.abort(s,"端点输出中断",false).await;}
        if let (Some(frames),Some(position))=(v["frames"].as_u64(),v["position"].as_f64()) {
            let m=&job.members[index];if m.rate>0&&v["sample_rate"]==m.rate&&frames>=m.frames&&position.is_finite()&&(position-(base(job)+frames as f64/m.rate as f64)).abs()<=0.002 {
                let mut next=job.clone();next.members[index].frames=frames;next.members[index].zone.position=position;
                if v["status"]=="ended"{next.members[index].phase="ended".into();}
                apply(job,s,next,Vec::new(),Vec::new(),None).await?;
            }
        }
    }
    finish(job,s).await
}
pub(super) async fn legacy_final(job:&mut Job,s:&AppState,index:usize,v:&Value)->anyhow::Result<()> {
    let now=crate::clock::now_ns();let m=&job.members[index];
    if job.settle_until.is_none_or(|until|now>=until)||m.final_report||v["type"]!="start_state"||v["version"]!=1||v["start"]!=m.token||v["final"]!=true||!matches!(v["phase"].as_str(),Some("cancelled"|"expired")){return Ok(());}
    let Some(frames)=v["programme_frames_played"].as_u64().filter(|frames|*frames>=m.frames)else{return Ok(());};
    let Some(rate)=v["sample_rate"].as_u64().and_then(|n|u32::try_from(n).ok()).filter(|n|(*n==0&&frames==0)||(8000..=768000).contains(n))else{return Ok(());};
    if m.rate>0&&m.rate!=rate||v["gate"]["expires_host_ns"]!=m.token["expires_host_ns"]{return Ok(());}
    let Some(released)=v["gate"]["released_host_ns"].as_u64()else{return Ok(());};
    if (frames>0||released>0)&&(!job.dispatched||!valid_gate(m,v,"released")){return Ok(());}
    let seconds=if rate==0{0.}else{frames as f64/rate as f64};if seconds>86400.||seconds>now.saturating_sub(job.deadline.unwrap_or(now)) as f64/1e9+1.{return Ok(());}
    let mut next=job.clone();next.members[index].frames=frames;next.members[index].rate=rate;next.members[index].zone.position=base(job)+seconds;next.members[index].gate=v["gate"].clone();next.members[index].final_report=true;
    apply(job,s,next,Vec::new(),Vec::new(),None).await
}
pub(super) async fn tick(job:&mut Job,s:&AppState)->anyhow::Result<bool> {
    if draining(job){retry_stop(job);finish(job,s).await?;return Ok(true);}
    Ok(job.phase=="paused")
}
