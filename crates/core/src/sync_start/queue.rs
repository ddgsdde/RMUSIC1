//! Finite ordered PCM groups; individual endpoints never issue independent listen claims.
use super::*;
use crate::models::Track;
use std::collections::BTreeMap;
mod transport;
pub(super) mod durable;
pub(crate) mod final_receipt;
pub(super) use transport::{abort,advance,final_report,rearm};
pub(crate) use transport::ended;

#[derive(Clone,Default)]
struct Progress {frames:u64,rate:u32,position:f64,ended:bool}
#[derive(Clone)]
pub(super) struct Queue {
    pub(super) jump_generation:u64,
    pub(super) logical:Vec<String>,
    pub(super) segment:usize,
    pub(super) started:Option<u64>,
    pub(super) known:Vec<HashMap<String,usize>>,
    heard:Vec<BTreeMap<usize,Progress>>,
    pub(super) position:f64,
    pub(super) credited:BTreeMap<usize,f64>,
    pub(super) completed:HashSet<usize>,
    positions:BTreeMap<usize,f64>,
}
impl Queue {
    pub(super) fn new(members:&[Member],length:usize)->Self {
        Self{jump_generation:0,position:members[0].zone.position,positions:BTreeMap::new(),credited:BTreeMap::new(),completed:HashSet::new(),logical:(0..length).map(|_|uuid::Uuid::new_v4().to_string()).collect(),segment:0,started:None,
            known:members.iter().map(|m|HashMap::from([(m.playback_id.clone(),0)])).collect(),
            heard:members.iter().map(|_|BTreeMap::from([(0,Progress::default())])).collect()}
    }
    pub(super) fn indices(&self)->Vec<usize>{self.credited.keys().copied().chain(self.heard.iter().flat_map(|reports|reports.keys().copied())).collect::<std::collections::BTreeSet<_>>().into_iter().collect()}
    fn current_seconds(&self,index:usize)->f64 {self.heard.iter().filter_map(|reports|reports.get(&index)).filter(|p|p.rate>0).map(|p|p.frames as f64/p.rate as f64).fold(0.,f64::max)}
    fn source_position(&self,index:usize)->f64 {
        let base=if index==self.segment{self.position}else{0.};
        if self.heard.iter().any(|reports|reports.contains_key(&index)){self.heard.iter().filter_map(|reports|reports.get(&index)).filter(|p|p.rate>0).map(|p|p.position).fold(base,f64::max)}else{self.positions.get(&index).copied().unwrap_or(base)}
    }
    pub(super) fn total_seconds(&self)->f64 {self.indices().into_iter().map(|index|self.credited.get(&index).copied().unwrap_or(0.)+self.current_seconds(index)).sum()}
    pub(super) fn source_ended(&self,member:usize,index:usize)->bool {self.heard[member].get(&index).is_some_and(|p|p.ended)}
    pub(super) fn seal_epoch(&mut self){
        if self.started.is_some(){for index in self.indices(){let seconds=self.current_seconds(index);*self.credited.entry(index).or_default()+=seconds;
            if self.heard.iter().any(|reports|reports.contains_key(&index)){self.positions.insert(index,self.source_position(index));}
            if self.heard.iter().all(|reports|reports.get(&index).is_some_and(|p|p.ended)){self.completed.insert(index);}
        }}
        for reports in &mut self.heard{reports.clear();}self.started=None;
    }
    pub(super) fn begin_epoch(&mut self,index:usize,position:f64,members:&[Member]){
        self.seal_epoch();self.segment=index;self.position=position;self.positions.insert(index,position);
        self.known=members.iter().map(|m|HashMap::from([(m.playback_id.clone(),index)])).collect();
        self.heard=members.iter().map(|_|BTreeMap::from([(index,Progress::default())])).collect();
    }
    pub(super) fn restart_from(&mut self,index:usize,position:f64,members:&[Member]) {
        self.seal_epoch();self.jump_generation+=1;
        let mut reset=self.indices().into_iter().filter(|old|*old>=index).collect::<Vec<_>>();if !reset.contains(&index){reset.push(index);}
        for old in reset {self.logical[old]=uuid::Uuid::new_v4().to_string();self.completed.remove(&old);self.positions.remove(&old);}
        self.begin_epoch(index,position,members);
    }
}
pub(crate) async fn register(s:&AppState,group:&str,zone:&str,items:Vec<(String,usize)>)->ApiResult<()> {
    let mut registry=s.sync_starts.lock().await;let job=registry.jobs.get_mut(group).filter(|j|j.active()).ok_or_else(ApiError::missing)?;
    let i=job.members.iter().position(|m|m.zone.id==zone).ok_or_else(ApiError::missing)?;
    durable::issue(s,job,i,&items).await?;
    let q=job.queue.as_mut().ok_or_else(ApiError::missing)?;
    for (playback,index) in items {q.known[i].insert(playback,index);}
    Ok(())
}
fn record(job:&mut Job,i:usize,zone:&Zone,frames:u64,rate:u32,ended:bool)->ApiResult<()> {
    let now=crate::clock::now_ns();let m=&job.members[i];let q=job.queue.as_ref().unwrap();let index=zone.queue_index;
    let known=zone.playback_id.as_deref().and_then(|id|q.known[i].get(id));
    let seconds=frames as f64/rate.max(1) as f64;let base=if index==q.segment{q.position}else{0.};
    let prior=q.heard[i].get(&index).filter(|p|p.rate>0);let prior_frames=prior.map_or(0,|p|p.frames);let prior_position=prior.map_or(base,|p|p.position);
    let epsilon=zone.position.abs().max(prior_position.abs()).max(1.)*f64::EPSILON*4.;
    let source_valid=if job.request.measured_version==2 {
        let delta=frames.saturating_sub(prior_frames) as f64/rate.max(1) as f64;let tolerance=2./rate.max(1) as f64;
        ((zone.position-base)-seconds).abs()<=seconds*0.0005+tolerance&&((zone.position-prior_position)-delta).abs()<=delta*0.0005+tolerance&&(frames!=prior_frames||(zone.position-prior_position).abs()<=epsilon)
    }else{(zone.position-base-seconds).abs()<=0.002};
    if known!=Some(&index)||zone.track.as_ref().map(|t|&t.id)!=m.zone.queue.get(index).map(|t|&t.id)
        || index<q.segment||!(8000..=768000).contains(&rate)||rate!=m.rate||!zone.position.is_finite()
        || !source_valid||seconds>86400.
        || q.heard[i].get(&index).is_some_and(|p|frames<p.frames||p.rate>0&&(p.rate!=rate||zone.position+epsilon<p.position))
        || !job.dispatched||q.started.is_none()
        || now.saturating_add(m.uncertainty).saturating_add(1_000_000)<job.deadline.unwrap_or(u64::MAX)
    {return Err(ApiError::bad("组队列实例、实际帧或起播时刻无效"));}
    let q=job.queue.as_mut().unwrap();let p=q.heard[i].entry(index).or_default();p.frames=frames;p.rate=rate;p.position=p.position.max(zone.position);p.ended|=ended;
    let seconds:f64=q.heard[i].values().filter(|p|p.rate>0).map(|p|p.frames as f64/p.rate as f64).sum();
    if seconds>now.saturating_sub(q.started.unwrap()) as f64/1e9+1.{return Err(ApiError::bad("组队列消费超出实际时间"));}
    Ok(())
}
pub(super) fn listening(job:&Job,index:usize)->(Zone,f64) {
    let q=job.queue.as_ref().unwrap();let seconds=q.current_seconds(index);
    let mut zone=Zone::new(format!("sync:{}:{index}",job.request.request_id),"同步播放组".into(),String::new());
    zone.track=job.members[0].zone.queue.get(index).cloned();zone.playback_id=Some(q.logical[index].clone());zone.status=if control::draining(job)||job.phase=="paused"{"paused"}else{"playing"}.into();
    zone.position=q.source_position(index);
    (zone,seconds)
}
pub(super) fn statistic(tx:&rusqlite::Transaction<'_>,job:&Job,index:usize,now:i64,reason:Option<&str>)->anyhow::Result<()> {
    let q=job.queue.as_ref().unwrap();let(zone,seconds)=listening(job,index);let base=if index==q.segment{q.position}else{0.};
    crate::stats::group_output::record_tx(tx,&zone,&job.listener,base,seconds,now,reason)
}
fn statistics(tx:&rusqlite::Transaction<'_>,job:&Job,indices:&[usize],now:i64,reason:Option<&str>)->anyhow::Result<()> {
    let q=job.queue.as_ref().unwrap();
    if job.deadline.is_none()&&q.started.is_none()&&q.credited.is_empty(){return Ok(());}
    for &index in indices {
        if (!q.heard.iter().any(|reports|reports.contains_key(&index))||job.deadline.is_none()&&q.started.is_none())
            &&!tx.query_row("SELECT EXISTS(SELECT 1 FROM listen_claims WHERE playback_id=?1)",[&q.logical[index]],|r|r.get::<_,bool>(0))? {continue;}
        let ended=q.completed.contains(&index)||q.heard.iter().all(|reports|reports.get(&index).is_some_and(|p|p.ended));
        statistic(tx,job,index,now,if ended{Some("ended")}else{reason})?;
    }Ok(())
}
type Receipt=(String,String,String,String);
pub(super) async fn apply(job:&mut Job,s:&AppState,next:Job,indices:Vec<usize>,mut zones:Vec<Zone>,receipt:Option<Receipt>,reason:Option<&str>)->anyhow::Result<()> {
    s.revise_queues(&mut zones).await;
    let prepared=durable::preparing(s,&next).await?;let previous=job.clone();let stored=next.clone();let saved=zones.clone();let reason=reason.map(str::to_owned);let now=chrono::Utc::now().timestamp_millis();
    s.db.run(move|c|{let tx=c.transaction()?;control::rebase_tx(&tx,&previous,&stored,now)?;statistics(&tx,&stored,&indices,now,reason.as_deref())?;durable::checkpoint_tx(&tx,&stored,&indices,now)?;durable::prepare_tx(&tx,&prepared,now)?;
        for zone in saved{tx.execute("UPDATE zones SET data=?2 WHERE id=?1",rusqlite::params![zone.id,serde_json::to_string(&zone)?])?;}
        if let Some((zone,request,owner,hash))=receipt{tx.execute("INSERT INTO sequence_receipts(zone_id,request_id,owner,payload_hash,created_at) VALUES(?1,?2,?3,?4,?5)",rusqlite::params![zone,request,owner,hash,now])?;tx.execute("DELETE FROM sequence_receipts WHERE zone_id=?1 AND rowid NOT IN (SELECT rowid FROM sequence_receipts WHERE zone_id=?1 ORDER BY rowid DESC LIMIT 128)",[zone])?;}
        control::record_tx(&tx,&stored,now)?;
        tx.execute("INSERT INTO sync_starts(id,request,result,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET result=excluded.result,updated_at=excluded.updated_at",rusqlite::params![stored.request.request_id,serde_json::to_string(&stored.request)?,stored.value().to_string(),now])?;
        tx.commit()?;Ok(())
    }).await?;
    *job=next;
    if !zones.is_empty(){s.zones.write().await.extend(zones.into_iter().map(|zone|(zone.id.clone(),zone)));s.emit("zones_changed",json!({}));}
    s.emit("sync_start",job.value());crate::device_groups::project(s,&job.value()).await?;s.emit("stats",json!({"group_id":job.request.request_id}));Ok(())
}
pub(super) async fn listen(job:&Job,s:&AppState,reason:Option<&str>)->anyhow::Result<()> {
    let job=job.clone();let indices=job.queue.as_ref().unwrap().indices();let reason=reason.map(str::to_owned);
    s.db.run(move|c|{let tx=c.transaction()?;statistics(&tx,&job,&indices,chrono::Utc::now().timestamp_millis(),reason.as_deref())?;tx.commit()?;Ok(())}).await
}
pub(crate) async fn handoff(s:&AppState,group:&str,previous:&Zone,ended:&[(Zone,u64)],current:&Zone,rate:u32,frames:u64,version:u32,request:&str,owner:&str,hash:&str)->ApiResult<bool> {
    let mut registry=s.sync_starts.lock().await;let Some(job)=registry.jobs.get_mut(group).filter(|j|j.active()&&j.queue.is_some())else{return Ok(false);};
    if version!=job.request.measured_version{return Err(ApiError::bad("分组交接计时版本不符"));}
    let Some(i)=job.members.iter().position(|m|m.zone.id==previous.id&&m.zone.playback_id==previous.playback_id&&m.zone.queue_index==previous.queue_index)else{return Ok(false);};
    if !s.bridges.read().await.get(&previous.endpoint_id).is_some_and(|tx|tx.same_channel(&job.members[i].tx)){return Ok(false);}
    let mut next=job.clone();let mut indices=Vec::new();
    for (zone,frames) in ended{record(&mut next,i,zone,*frames,rate,true)?;indices.push(zone.queue_index);}
    record(&mut next,i,current,frames,rate,false)?;indices.push(current.queue_index);indices.sort_unstable();indices.dedup();
    next.members[i].zone=current.clone();next.members[i].frames=frames;next.members[i].last_seen=crate::clock::now_ns();let draining=control::draining(&next);
    if !draining{next.members[i].phase="playing".into();next.phase="playing".into();}
    apply(job,s,next,indices,if draining{Vec::new()}else{vec![current.clone()]},Some((previous.id.clone(),request.into(),owner.into(),hash.into())),None).await?;Ok(true)
}
pub(super) async fn report(job:&mut Job,s:&AppState,i:usize,v:&Value)->anyhow::Result<()> {
    if job.members[i].zone.playback_id.as_deref()!=v["core_playback_id"].as_str(){return Ok(());}
    if job.members[i].phase=="ended"{return Ok(());}
    let mut next=job.clone();let mut indices=Vec::new();
    if let (Some(frames),Some(position),Some(rate))=(v["frames"].as_u64(),v["position"].as_f64(),v["sample_rate"].as_u64().and_then(|n|u32::try_from(n).ok())){
        let mut zone=next.members[i].zone.clone();zone.position=position;
        if record(&mut next,i,&zone,frames,rate,false).is_ok(){zone.updated_at=chrono::Utc::now().timestamp_millis();next.members[i].zone=zone;next.members[i].frames=frames;next.members[i].last_seen=crate::clock::now_ns();indices.push(next.members[i].zone.queue_index);}else{next=job.clone();}
    }
    if v["type"]=="error"||v["continuity_lost"]==true {
        if !indices.is_empty(){apply(job,s,next,indices,Vec::new(),None,None).await?;}
        if v["type"]!="error"{return control::recover_underrun(job,s).await;}
        return abort(job,s,"端点连续组输出中断",false).await;
    }
    if v["status"]!="playing"||indices.is_empty(){return Ok(());}
    let draining=control::draining(&next);if !draining{next.members[i].phase="playing".into();next.phase="playing".into();}let zone=next.members[i].zone.clone();apply(job,s,next,indices,if draining{Vec::new()}else{vec![zone]},None,None).await?;
    if !draining&&v["type"]=="state"&&v["track_id"].as_str()==job.members[i].zone.track.as_ref().map(|track|track.id.as_str())&&v["signal_path"].is_object(){
        let endpoint=&job.members[i].zone.endpoint_id;if let Some(row)=s.endpoints.write().await.get_mut(endpoint){row["playback"]=v.clone();}s.emit("endpoint_signal",json!({"endpoint_id":endpoint,"playback":v}));
    }Ok(())
}

pub(super) async fn resolve(s:&AppState,request:&Request)->ApiResult<Vec<Track>> {
    if request.queue.is_empty(){return Ok(vec![crate::catalog::pcm_track(s,crate::library::get_track(s,request.track_id.clone()).await?).await?]);}
    if request.queue.len()>10000||request.queue.first()!=Some(&request.track_id)||request.queue.iter().any(|id|id.is_empty()||id.len()>200){return Err(ApiError::bad("组队列须以起播曲目开头，最多一万首"));}
    let ids=request.queue.clone();
    let tracks=s.db.run(move|c|{
        let mut query=c.prepare("SELECT * FROM tracks WHERE id=?1 AND available=1")?;
        let mut known=HashMap::new();let mut tracks=Vec::with_capacity(ids.len());
        for id in ids {
            if !known.contains_key(&id){let track=query.query_row([&id],Track::from_row).optional()?;known.insert(id.clone(),track);}
            let Some(track)=known[&id].clone()else{return Ok(None);};tracks.push(track);
        }Ok(Some(tracks))
    }).await?.ok_or_else(ApiError::missing)?;
    let mut resolved=HashMap::new();let mut result=Vec::with_capacity(tracks.len());
    for track in tracks {if !resolved.contains_key(&track.id){resolved.insert(track.id.clone(),crate::catalog::pcm_track(s,track.clone()).await?);}result.push(resolved[&track.id].clone());}
    Ok(result)
}
pub(crate) fn same_format(a:&Track,b:&Track)->bool {
    (8000..=768000).contains(&a.sample_rate)&&(1..=32).contains(&a.channels)&&(2..=64).contains(&a.bit_depth)&&a.sample_rate==b.sample_rate&&a.channels==b.channels&&a.bit_depth==b.bit_depth
}
/// The public idle zone stays untouched until every member is prepared and committed.
/// sequence::authorize already checked the credential owning this endpoint.
pub(crate) async fn timing(s:&AppState,group:&str)->Option<u32>{s.sync_starts.lock().await.jobs.get(group).map(|job|job.request.measured_version)}
pub(crate) async fn preparation(s:&AppState,zone:&str,endpoint:&str,track:&str,playback:&str)->Option<(String,Zone)> {
    let registry=s.sync_starts.lock().await;
    let job=registry.jobs.values().find(|j|matches!(j.phase.as_str(),"preparing"|"committed"|"playing")&&!j.request.queue.is_empty()&&j.members.iter().any(|m|m.zone.id==zone&&m.zone.endpoint_id==endpoint&&m.zone.playback_id.as_deref()==Some(playback)&&m.zone.track.as_ref().is_some_and(|t|t.id==track)))?;
    let member=job.members.iter().find(|m|m.zone.id==zone)?;
    if !s.bridges.read().await.get(endpoint).is_some_and(|tx|tx.same_channel(&member.tx)){return None;}
    let mut pending=member.zone.clone();pending.status="playing".into();
    Some((job.request.request_id.clone(),pending))
}
