//! Idle-zone PCM cohort start. Driver/DAC precision is not asserted.
pub(crate) mod queue;
pub(crate) mod control;
mod membership;
mod clock_feedback;
mod drift;
mod policy;
mod volume;
pub(crate) mod rate_adjustment;
pub(crate) use volume::{patch as volume_patch,patch_all as volume_all};
pub(crate) mod recovery;
use crate::{auth::Auth,error::{ApiError,ApiResult},models::Zone,state::AppState};
use axum::{extract::{Path,Query,State},http::StatusCode,Json};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::collections::{HashMap,HashSet};
use tokio::sync::mpsc;
use rusqlite::OptionalExtension;
const PREPARE_NS:u64=5_000_000_000;
const QUIET_NS:u64=6_000_000_000;
fn legacy_timing()->u32{1}
fn is_legacy_timing(v:&u32)->bool{*v==1}
fn no_rate_adjustment(value:&bool)->bool{!*value}
#[derive(Clone,Serialize,Deserialize,PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Request {#[serde(default,skip_serializing_if="Option::is_none")]sync_policy:Option<policy::Policy>,#[serde(default,skip_serializing_if="no_rate_adjustment")]automatic_rate_adjustment:bool,#[serde(default,skip_serializing_if="no_rate_adjustment")]pcm_rate_adjustment:bool,#[serde(default="legacy_timing",skip_serializing_if="is_legacy_timing")]measured_version:u32,request_id:String,zone_ids:Vec<String>,track_id:String,#[serde(default)]position:f64,#[serde(default,skip_serializing_if="Vec::is_empty")]queue:Vec<String>}
#[derive(Default)]
pub struct Registry {jobs:HashMap<String,Job>,generation:u64}
#[derive(Clone)]
struct Member {requested_rate_ppm:f64,clock:clock_feedback::Binding,original:Zone,zone:Zone,playback_id:String,uncertainty:u64,host_deadline:u64,tx:mpsc::Sender<Value>,token:Value,manual_power:Option<Value>,phase:String,rate:u32,frames:u64,gate:Value,last_seen:u64,final_report:bool}
#[derive(Clone)]
struct Job {sync_monitor:policy::Monitor,drift:drift::Controller,rate_request:Option<String>,control:Option<control::Control>,queue:Option<queue::Queue>,request:Request,controller:String,listener:String,created:u64,expires:u64,deadline:Option<u64>,phase:String,error:Option<String>,dispatched:bool,members:Vec<Member>,settle_until:Option<u64>}
impl Registry {
    fn compact(&mut self,now:u64){
        self.jobs.retain(|_,job|job.active()||job.settle_until.is_some()||now.saturating_sub(job.created)<60_000_000_000);
        let mut terminal=self.jobs.iter().filter(|(_,job)|!job.active()&&job.settle_until.is_none()).map(|(id,job)|(job.created,id.clone())).collect::<Vec<_>>();
        terminal.sort();let excess=terminal.len().saturating_sub(128);for(_,id)in terminal.into_iter().take(excess){self.jobs.remove(&id);}
    }
}
impl Job {
    fn reserved(&self)->impl Iterator<Item=&Member> {self.members.iter().chain(self.control.iter().flat_map(|c|c.targets.iter().flatten()))}
    fn owns_zone(&self,id:&str)->bool{self.reserved().any(|m|m.zone.id==id)}
    fn owns_endpoint(&self,id:&str)->bool{self.reserved().any(|m|m.zone.endpoint_id==id)}
    fn active(&self)->bool {matches!(self.phase.as_str(),"preparing"|"committed"|"playing"|"pausing"|"paused"|"seeking")}
    fn value(&self)->Value {let mut value=json!({"sync_policy":self.request.sync_policy,"sync_monitor":self.sync_monitor.value,"id":self.request.request_id,"track_id":self.request.track_id,"title":self.members[0].zone.track.as_ref().map(|t|&t.title),"position":self.request.position,"phase":self.phase,"error":self.error,"created_at_core_ns":self.created,"expires_at_core_ns":self.expires,"deadline_core_ns":self.deadline,"may_have_started":self.dispatched||control::ever_started(self),"transport":self.control.as_ref().map(|c|c.value()),"buffer_recovery":{"count":self.drift.buffer_recoveries,"resume_policy":"earliest_confirmed","active":self.drift.buffer_recovering&&matches!(self.phase.as_str(),"seeking"|"preparing"|"committed"),"changes_samples":false},"playhead":{"queue_index":control::playhead(self).0,"position":control::playhead(self).1},"listened_seconds":self.queue.as_ref().map_or_else(||control::credit(self)+self.listening().1,|q|q.total_seconds()),"strict_sync":false,"statistics_scope":"group",
        "queue":self.queue.as_ref().map(|q|json!({"length":q.logical.len(),"segment_start":q.segment})),"settled":!self.active()&&self.settle_until.is_none(),"members":self.members.iter().map(|m|json!({"zone_id":m.zone.id,"endpoint_id":m.zone.endpoint_id,"playback_id":m.playback_id,"current_playback_id":m.zone.playback_id,"queue_index":m.zone.queue_index,"phase":m.phase,"deadline_host_ns":m.host_deadline,"sample_rate":m.rate,"frames":m.frames,"position":m.zone.position,"gate":m.gate,"final_report_received":m.final_report})).collect::<Vec<_>>()});if self.request.measured_version==2{value["measured_version"]=json!(2);}rate_adjustment::decorate(self,&mut value);value}
    fn listening(&self)->(Zone,f64) {
        let seconds=self.members.iter().filter(|m|m.rate>0).map(|m|m.frames as f64/m.rate as f64).fold(0.,f64::max);
        let mut zone=self.members[0].zone.clone();zone.id=format!("sync:{}",self.request.request_id);zone.name="同步播放组".into();zone.playback_id=Some(control::logical(self).to_owned());zone.status=if control::draining(self)||self.phase=="paused"{"paused"}else{"playing"}.into();zone.position=control::base(self)+seconds;(zone,seconds)
    }
    async fn listen(&self,s:&AppState,reason:Option<&str>)->anyhow::Result<()> {if self.queue.is_some(){return queue::listen(self,s,reason).await;}let(zone,seconds)=self.listening();crate::stats::record_group_progress(s,&zone,&self.listener,seconds,reason).await?;Ok(())}
    async fn persist(&self,s:&AppState)->anyhow::Result<()> {
        let prepared=queue::durable::preparing(s,self).await?;let stored=self.clone();let now=chrono::Utc::now().timestamp_millis();
        s.db.run(move|c|{let tx=c.transaction()?;if stored.phase=="preparing"{for member in &stored.members{tx.execute("DELETE FROM sync_group_holds WHERE group_id IN (SELECT group_id FROM sync_group_holds WHERE zone_id=?1)",[&member.zone.id])?;}}control::record_tx(&tx,&stored,now)?;queue::durable::checkpoint_tx(&tx,&stored,&[],now)?;queue::durable::prepare_tx(&tx,&prepared,now)?;tx.execute("INSERT INTO sync_starts(id,request,result,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET result=excluded.result,updated_at=excluded.updated_at",rusqlite::params![stored.request.request_id,serde_json::to_string(&stored.request)?,stored.value().to_string(),now])?;tx.commit()?;Ok(())}).await?;s.emit("sync_start",self.value());crate::device_groups::project(s,&self.value()).await?;Ok(())
    }
    async fn abort(&mut self,s:&AppState,reason:&str,cancelled:bool)->anyhow::Result<()> {
        if self.queue.is_some(){return queue::abort(self,s,reason,cancelled).await;}
        if !self.active(){return Ok(());}
        if let Some(c)=&mut self.control{c.failed(reason);}
        self.phase=if cancelled{"cancelled"}else{"failed"}.into();self.error=Some(reason.into());
        self.settle_until=(!self.members.iter().all(|m|m.final_report)).then(||crate::clock::now_ns()+QUIET_NS);
        for member in &self.members {if member.phase!="ended"{let _=member.tx.try_send(json!({"type":"cancel_start","zone_id":member.zone.id,"playback_id":member.playback_id,"start":member.token}));}}
        for member in &mut self.members {
            if member.phase=="ended"{continue;}
            member.phase="cancelled".into();
        }
        if self.control.is_some(){if self.deadline.is_some()||control::ever_started(self){self.listen(s,Some("stop")).await?;}save_zones(s,control::stopped_zones(s,self).await).await?;}
        else if self.deadline.is_some(){self.listen(s,Some("stop")).await?;let zones=self.members.iter().map(|m|if self.dispatched{let mut z=m.zone.clone();z.status="stopped".into();z.playback_id=None;z}else{m.original.clone()}).collect();save_zones(s,zones).await?;}
        if !self.request.queue.is_empty(){crate::sequence::discard_group_preparation(s,&self.request.request_id).await;}
        self.persist(s).await
    }
    async fn final_report(&mut self,s:&AppState,index:usize,v:&Value)->anyhow::Result<()> {
        if self.queue.is_some(){return queue::final_report(self,s,index,v).await;}
        if control::draining(self){return control::legacy_final(self,s,index,v).await;}
        let now=crate::clock::now_ns();let member=&self.members[index];
        if !matches!(self.phase.as_str(),"cancelled"|"failed")||self.settle_until.is_none_or(|until|now>=until)||member.final_report||v["type"]!="start_state"||v["version"]!=1||v["start"]!=member.token||v["final"]!=true||!matches!(v["phase"].as_str(),Some("cancelled"|"expired")){return Ok(());}
        let Some(frames)=v["programme_frames_played"].as_u64().filter(|n|*n>=member.frames)else{return Ok(());};
        let Some(rate)=v["sample_rate"].as_u64().and_then(|n|u32::try_from(n).ok()).filter(|n|(*n==0&&frames==0)||(8000..=768000).contains(n))else{return Ok(());};
        if member.rate>0&&member.rate!=rate||v["gate"]["expires_host_ns"]!=member.token["expires_host_ns"]{return Ok(());}
        let released=v["gate"]["released_host_ns"].as_u64();if released.is_none(){return Ok(());}
        if frames>0||released!=Some(0){if !self.dispatched||!valid_gate(member,v,"released"){return Ok(());}}
        let seconds=if rate==0{0.}else{frames as f64/rate as f64};
        if seconds>86400.||seconds>now.saturating_sub(self.deadline.unwrap_or(now)) as f64/1e9+1.{return Ok(());}
        let base=control::base(self);let member=&mut self.members[index];member.frames=frames;member.rate=rate;member.zone.position=base+seconds;member.gate=v["gate"].clone();member.final_report=true;
        if self.dispatched {let(zone,seconds)=self.listening();crate::stats::correct_group_progress(s,&zone,seconds).await?;}
        if self.members.iter().all(|m|m.final_report){self.settle_until=None;}
        self.persist(s).await
    }
    async fn prepare_commands(&self,s:&AppState)->anyhow::Result<Vec<Value>> {
    let mut commands=Vec::new();
    for member in &self.members {
        let owner=s.bridge_owners.read().await.get(&member.zone.endpoint_id).cloned();
        let api=crate::render_media::path(s,&member.zone,member.zone.track.as_ref().unwrap(),owner)
            .await.map_err(|error|anyhow::anyhow!("{}",error.2))?;
        let relative=s.endpoints.read().await.get(&member.zone.endpoint_id).is_some_and(|e|e["capabilities"]["media_url_api_relative"]==true);let media=if relative{api}else{format!("{}{}/{api}",s.config.public_url.as_deref().unwrap_or("http://127.0.0.1:7800"),s.config.base_path)};
        let mut command=json!({"group_queue":self.queue.is_some(),"type":"prepare_start","zone_id":member.zone.id,"endpoint_id":member.zone.endpoint_id,"playback_id":member.playback_id,"track":member.zone.track,"url":media,"position":member.zone.position,"volume":member.zone.volume,"muted":member.zone.muted,"dsp":member.zone.dsp,"transitions":member.zone.transitions,"start":member.token});if self.request.measured_version==2{command["measured_version"]=json!(2);}if self.request.pcm_rate_adjustment{command["pcm_rate_adjustment"]=json!(true);}if self.request.automatic_rate_adjustment{command["automatic_rate_adjustment"]=json!(true);}commands.push(command);
    }
        Ok(commands)
    }
    async fn calibrations(&self,s:&AppState)->anyhow::Result<Vec<Value>> {
        let now=crate::clock::now_ns();let mut result=Vec::new();
        for member in &self.members {
            if !s.bridges.read().await.get(&member.zone.endpoint_id).is_some_and(|tx|tx.same_channel(&member.tx)){anyhow::bail!("网桥连接已更换");}
            let endpoint=s.endpoints.read().await.get(&member.zone.endpoint_id).cloned().unwrap_or(Value::Null);if self.request.measured_version==2&&(endpoint["capabilities"]["group_output_v2"]!=true||endpoint["capabilities"]["measured_listens_v2"]!=true){anyhow::bail!("group output v2 capability changed");}rate_adjustment::supported(&self.request,&endpoint)?;let estimate=calibration(s,&endpoint,&member.tx,now).await?;
            if estimate["host_clock_id"]!=member.token["host_clock_id"]||estimate["core_clock_id"]!=member.token["core_clock_id"]{anyhow::bail!("端点时钟身份已更换");}result.push(estimate);
        }Ok(result)
    }
    async fn commit(&mut self,s:&AppState)->anyhow::Result<()> {
        if self.queue.is_some(){for m in &self.members{if !crate::sequence::group_prepared(s,&m.zone.id,&m.playback_id,&self.request.request_id).await{anyhow::bail!("组队列预载实例已失效");}}}
        if !s.session_active(self.controller.clone()).await{anyhow::bail!("控制凭证已失效");}
        let estimates=self.calibrations(s).await?;
        let permits=self.members.iter().map(|m|m.tx.clone().try_reserve_owned()).collect::<Result<Vec<_>,_>>().map_err(|_|anyhow::anyhow!("网桥控制队列繁忙"))?;
        self.deadline=Some(0);
        for member in &mut self.members {
            member.zone.status="playing".into();member.zone.updated_at=chrono::Utc::now().timestamp_millis();
        }
        self.listen(s,None).await?;
        save_zones(s,self.members.iter().map(|m|m.zone.clone()).collect()).await?;
        let now=crate::clock::now_ns();let lead=500_000_000+estimates.iter().map(|e|e["network_rtt_ns"].as_u64().unwrap_or(500_000_000)).max().unwrap_or(0)*2;let deadline=now+lead;
        if deadline>=self.expires{anyhow::bail!("整组准备已超过起播期限");}
        let host_deadlines=estimates.iter().zip(&self.members).map(|(e,m)|{let time=host_time(e,deadline)?;if time>=m.token["expires_host_ns"].as_u64().unwrap(){anyhow::bail!("端点起播期限已过期");}Ok(time)}).collect::<anyhow::Result<Vec<_>>>()?;
        for (member,estimate) in self.members.iter_mut().zip(&estimates){member.uncertainty=estimate["network_uncertainty_ns"].as_u64().unwrap_or(250_000_000);}
        for (member,time) in self.members.iter_mut().zip(&host_deadlines){member.host_deadline=*time;}
        if let Some(queue)=&mut self.queue{queue.started.get_or_insert(deadline);}
        self.deadline=Some(deadline);self.phase="committed".into();self.dispatched=true;if let Some(c)=&mut self.control{c.committed();}self.persist(s).await?;
        let now=crate::clock::now_ns();
        if now>=deadline{self.dispatched=false;anyhow::bail!("提交前已错过起播时刻");}
        for ((permit,member),host_deadline) in permits.into_iter().zip(&mut self.members).zip(host_deadlines){member.last_seen=now;permit.send(json!({"type":"commit_start","zone_id":member.zone.id,"playback_id":member.playback_id,"start":member.token,"deadline_host_ns":host_deadline}));}
        self.persist(s).await
    }
}
async fn calibration(s:&AppState,endpoint:&Value,tx:&mpsc::Sender<Value>,now:u64)->anyhow::Result<Value> {
    let e=s.host_clock_estimate(endpoint["id"].as_str().unwrap_or(""),tx,now).await;
    if endpoint["online"]!=true||endpoint["capabilities"]["scheduled_start_v1"]!=true||endpoint["capabilities"]["continuous_pcm"]!=true||e["valid_until_core_ns"].as_u64().is_none_or(|n|n<=now)||e["host_minus_core_ns"].as_i64().is_none()||e["network_rtt_ns"].as_u64().is_none_or(|n|n>500_000_000)||e["core_clock_id"].as_str().is_none_or(|id|uuid::Uuid::parse_str(id).is_err())||e["host_clock_id"].as_str().is_none_or(|id|uuid::Uuid::parse_str(id).is_err()){anyhow::bail!("端点不支持计划起播或宿主时钟校准无效");}Ok(e)
}
fn valid_gate(member:&Member,v:&Value,phase:&str)->bool {
    let gate=&v["gate"];if gate["phase"]!=phase||gate["expires_host_ns"]!=member.token["expires_host_ns"]||["expires_host_ns","deadline_host_ns","ready_host_ns","released_host_ns"].iter().any(|key|gate[*key].as_u64().is_none()){return false;}
    if phase=="ready"{gate["deadline_host_ns"]==0&&gate["released_host_ns"]==0&&gate["ready_host_ns"].as_u64().unwrap()>0&&gate["ready_host_ns"].as_u64().unwrap()<member.token["expires_host_ns"].as_u64().unwrap()}
    else{gate["deadline_host_ns"]==member.host_deadline&&(member.host_deadline..=member.host_deadline.saturating_add(20_000_000)).contains(&gate["released_host_ns"].as_u64().unwrap())}
}
fn host_time(estimate:&Value,core:u64)->anyhow::Result<u64> {Ok(u64::try_from(core as i128+estimate["host_minus_core_ns"].as_i64().ok_or_else(||anyhow::anyhow!("时钟偏移无效"))? as i128)?)}
pub(crate) fn power_stamp(value:&Value)->Option<Value> {
    let host=value["host_id"].as_str().filter(|id|uuid::Uuid::parse_str(id).is_ok())?;
    Some(json!({"host_id":host,"generation":value["generation"].as_u64()?,"wake_generation":value["wake_generation"].as_u64()?}))
}
fn group_power(endpoint:&Value)->anyhow::Result<Option<Value>> {
    if endpoint["capabilities"]["group_power_v1"]!=true{return Ok(None);}
    if endpoint["group_power_clock_id"]!=endpoint["clock_calibration"]["host_clock_id"]{anyhow::bail!("端点电源状态尚未校准，请稍后重新手动起播");}
    power_stamp(&endpoint["group_power"]).map(Some).ok_or_else(||anyhow::anyhow!("端点尚未报告当前挂起／唤醒代数，请稍后重新手动起播"))
}
async fn save_zones(s:&AppState,mut zones:Vec<Zone>)->anyhow::Result<()> {
    s.revise_queues(&mut zones).await;
    let saved=zones.clone();s.db.run(move|c|{let tx=c.transaction()?;for z in saved{tx.execute("INSERT INTO zones(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",rusqlite::params![z.id,serde_json::to_string(&z)?])?;}tx.commit()?;Ok(())}).await?;
    s.zones.write().await.extend(zones.into_iter().map(|z|(z.id.clone(),z)));s.emit("zones_changed",json!({}));Ok(())
}
pub async fn start(State(s):State<AppState>,a:Auth,Json(request):Json<Request>)->ApiResult<(StatusCode,Json<Value>)> {start_inner(s,a,request,None).await}
pub(crate) async fn start_for_device_group(s:AppState,a:Auth,request:Request,group:String)->ApiResult<(StatusCode,Json<Value>)>{start_inner(s,a,request,Some(group)).await}
async fn start_inner(s:AppState,a:Auth,mut request:Request,device_group:Option<String>)->ApiResult<(StatusCode,Json<Value>)> {
    a.require("control")?;
    if uuid::Uuid::parse_str(&request.request_id).is_err()||!(2..=4).contains(&request.zone_ids.len())||request.zone_ids.iter().any(|id|id.is_empty()||id.len()>200)||!request.position.is_finite()||request.position<0.{return Err(ApiError::bad("分组起播须包含请求 UUID、二至四个音区和有效位置"));}
    if request.sync_policy.as_ref().is_some_and(|p|!p.valid()){return Err(ApiError::bad("同步容差须为100–50000微秒，预热为1000–10000毫秒"));}
    if ![1,2].contains(&request.measured_version){return Err(ApiError::bad("分组计时版本无效"));}
    if request.automatic_rate_adjustment&&(!request.pcm_rate_adjustment||request.measured_version!=2){return Err(ApiError::bad("实验自动校正须显式启用 v2 PCM 变速"));}
    if request.measured_version==2&&request.queue.is_empty(){request.queue.push(request.track_id.clone());}
    request.request_id=uuid::Uuid::parse_str(&request.request_id).unwrap().to_string();
    request.zone_ids.sort();if request.zone_ids.windows(2).any(|pair|pair[0]==pair[1]){return Err(ApiError::bad("音区不能重复"));}
    let _lock=s.zone_lock.lock().await;let id=request.request_id.clone();let encoded=serde_json::to_string(&request).map_err(anyhow::Error::from)?;
    let saved=s.db.run(move|c|Ok(c.query_row("SELECT request,result FROM sync_starts WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?)).await?;
    if let Some((old,result))=saved{if old!=encoded{return Err(ApiError::bad("请求 ID 已用于另一项播放"));}return Ok((StatusCode::OK,Json(serde_json::from_str(&result).map_err(anyhow::Error::from)?)));}
    for id in &request.zone_ids {crate::device_groups::member_guard(&s,id,device_group.as_deref()).await?;}
    let tracks=queue::resolve(&s,&request).await?;let track=tracks[0].clone();
    if !track.available||track.sample_rate>768_000||track.bit_depth==1||matches!(track.format.as_str(),"dsf"|"dff")||track.duration>0.&&request.position>=track.duration{return Err(ApiError::bad("当前分组起播只接受可用的 PCM 音源和曲内位置"));}
    let mut registry=s.sync_starts.lock().await;registry.compact(crate::clock::now_ns());
    if registry.jobs.values().filter(|j|j.active()).count()>=32{return Err(ApiError::bad("正在准备的分组过多"));}
    // Format segments can advance a job beyond the registry's last new-group generation.
    registry.generation=registry.jobs.values().flat_map(|job|job.members.iter()).filter_map(|member|member.token["generation"].as_u64()).max().unwrap_or(0).max(registry.generation);
    registry.jobs.retain(|_,j|j.active()||j.settle_until.is_some());registry.generation+=1;let generation=registry.generation;let now=crate::clock::now_ns();let mut members=Vec::new();let mut endpoints=HashSet::new();
    for id in &request.zone_ids {
        let original=s.zones.read().await.get(id).cloned().ok_or_else(ApiError::missing)?;
        if original.status!="stopped"||original.playback_id.is_some()||!endpoints.insert(original.endpoint_id.clone()){return Err(ApiError::bad("请选择使用不同端点且已停止的音区"));}
        if registry.jobs.values().any(|j|j.active()&&(j.owns_zone(id)||j.owns_endpoint(&original.endpoint_id))){return Err(ApiError::bad("音区或端点正在另一组播放中"));}
        if s.zones.read().await.values().any(|z|z.id!=*id&&z.endpoint_id==original.endpoint_id&&(z.status!="stopped"||z.playback_id.is_some())){return Err(ApiError::bad("该端点仍被其他音区使用"));}
        let zid=id.clone();if s.db.run(move|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM sequence_recovery WHERE zone_id=?1)",[zid],|r|r.get::<_,bool>(0))?)).await?{return Err(ApiError::bad("音区仍有恢复计划，请先停止"));}
        let endpoint=s.endpoints.read().await.get(&original.endpoint_id).cloned().unwrap_or(Value::Null);if matches!(endpoint["kind"].as_str(),Some("browser"|"android"))||endpoint["capabilities"]["platform"]=="android"{return Err(ApiError::bad("网页与安卓不作为同步播放成员。"));}let tx=s.bridges.read().await.get(&original.endpoint_id).cloned().ok_or_else(||ApiError::bad("网桥未连接"))?;let estimate=calibration(&s,&endpoint,&tx,now).await.map_err(|e|ApiError::bad(e.to_string()))?;
        if request.sync_policy.is_some()&&["device_clock_v1","programme_clock_v1"].iter().any(|key|endpoint["capabilities"][*key]!=true){return Err(ApiError::bad("受监控同步需要全部输出提供配对的节目与设备时钟"));}
        rate_adjustment::supported(&request,&endpoint).map_err(|e|ApiError::bad(e.to_string()))?;
        if request.measured_version==2&&(endpoint["capabilities"]["group_output_v2"]!=true||endpoint["capabilities"]["measured_listens_v2"]!=true){return Err(ApiError::bad("所有端点均须声明分组双时间能力"));}
        if !request.queue.is_empty()&&endpoint["capabilities"]["scheduled_queue_v1"]!=true{return Err(ApiError::bad("端点尚未接入连续组队列"));}
        let mut zone=original.clone();zone.renew_playback();zone.track=Some(track.clone());zone.position=request.position;
        if !request.queue.is_empty(){zone.queue=tracks.clone();zone.queue_index=0;zone.repeat="off".into();zone.shuffle=false;zone.autoplay.enabled=false;}
        else if let Some(i)=zone.queue.iter().position(|t|t.id==track.id){zone.queue_index=i;}else{zone.queue.push(track.clone());zone.queue_index=zone.queue.len()-1;}
        crate::zones::processing_allowed(&s,&zone).await?;
        let manual_power=group_power(&endpoint).map_err(|e|ApiError::bad(e.to_string()))?;
        let mut token=json!({"version":1,"plan_id":request.request_id,"generation":generation,"core_clock_id":estimate["core_clock_id"],"host_clock_id":estimate["host_clock_id"],"expires_host_ns":host_time(&estimate,now+PREPARE_NS)?});
        if let Some(stamp)=&manual_power{token["group_power"]=stamp.clone();}
        members.push(Member{requested_rate_ppm:0.,clock:clock_feedback::Binding::Unseen,playback_id:zone.playback_id.clone().unwrap(),uncertainty:estimate["network_uncertainty_ns"].as_u64().unwrap_or(250_000_000),host_deadline:0,original,zone,tx,token,manual_power,phase:"preparing".into(),rate:0,frames:0,gate:Value::Null,last_seen:now,final_report:false});
    }
    let permits=members.iter().map(|m|m.tx.clone().try_reserve_owned()).collect::<Result<Vec<_>,_>>().map_err(|_|ApiError::bad("网桥控制队列繁忙"))?;
    let queue=(!request.queue.is_empty()).then(||queue::Queue::new(&members,request.queue.len()));
    let job=Job{sync_monitor:Default::default(),drift:Default::default(),rate_request:None,control:None,queue,request,controller:a.session_hash,listener:a.id,created:now,expires:now+PREPARE_NS,deadline:None,phase:"preparing".into(),error:None,dispatched:false,members,settle_until:None};
    let commands=job.prepare_commands(&s).await?;
    job.persist(&s).await?;let value=job.value();let id=job.request.request_id.clone();registry.jobs.insert(id,job);for (permit,command) in permits.into_iter().zip(commands){permit.send(command);}Ok((StatusCode::ACCEPTED,Json(value)))
}
#[derive(Deserialize)]
#[serde(default)]
pub struct List {limit:u32,offset:u32,active:bool,recoverable:bool}
impl Default for List {fn default()->Self{Self{limit:50,offset:0,active:false,recoverable:false}}}
pub async fn list(State(s):State<AppState>,a:Auth,Query(q):Query<List>)->ApiResult<Json<Value>> {
    a.require("read")?;if !(1..=100).contains(&q.limit)||q.offset>100000{return Err(ApiError::bad("分组列表分页范围无效"));}
    let result=s.db.run(move|c|{
        let condition="(?1=0 OR json_extract(result,'$.phase') IN ('preparing','committed','playing','pausing','paused','seeking') OR (?2=1 AND json_extract(result,'$.phase')='interrupted' AND EXISTS(SELECT 1 FROM sync_group_holds h WHERE h.group_id=sync_starts.id) AND EXISTS(SELECT 1 FROM sync_output_epochs e WHERE e.group_id=sync_starts.id AND e.listener_id=?3)))";
        let total=c.query_row(&format!("SELECT COUNT(*) FROM sync_starts WHERE {condition}"),rusqlite::params![q.active,q.recoverable,a.id],|r|r.get::<_,i64>(0))?;
        let rows=c.prepare(&format!("SELECT result FROM sync_starts WHERE {condition} ORDER BY updated_at DESC,id DESC LIMIT ?4 OFFSET ?5"))?.query_map(rusqlite::params![q.active,q.recoverable,a.id,q.limit,q.offset],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let items=rows.into_iter().map(|v|serde_json::from_str::<Value>(&v)).collect::<Result<Vec<_>,_>>()?;Ok(json!({"items":items,"total":total}))
    }).await?;Ok(Json(result))
}
pub async fn get(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("read")?;saved(s,id).await
}
async fn saved(s:AppState,id:String)->ApiResult<Json<Value>> {
    let job={s.sync_starts.lock().await.jobs.get(&id).cloned()};
    if let Some(job)=job{let mut value=job.value();value["clock_diagnostics"]=clock_feedback::snapshot(&job,&s).await;return Ok(Json(value));}
    let result=s.db.run(move|c|Ok(c.query_row("SELECT result FROM sync_starts WHERE id=?1",[id],|r|r.get::<_,String>(0)).optional()?)).await?.ok_or_else(ApiError::missing)?;let mut value:Value=serde_json::from_str(&result).map_err(anyhow::Error::from)?;value["clock_diagnostics"]=clock_feedback::inactive();if value["sync_monitor"].is_object(){value["sync_monitor"]["state"]=json!("inactive");value["sync_monitor"]["observed_known_bound_ns"]=Value::Null;}Ok(Json(value))
}
pub(crate) async fn observe_device_clock(s:&AppState,id:&str,tx:&mpsc::Sender<Value>){clock_feedback::observe(s,id,tx).await;}
pub async fn cancel(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("control")?;let _lock=s.zone_lock.lock().await;let mut registry=s.sync_starts.lock().await;
    let group=id.clone();s.db.run(move|c|{c.execute("DELETE FROM sync_group_holds WHERE group_id=?1",[group])?;Ok(())}).await?;
    if let Some(job)=registry.jobs.get_mut(&id){job.abort(&s,"用户取消整组播放",true).await?;return Ok(Json(job.value()));}drop(registry);saved(s.clone(),id).await
}
/// Caller holds zone_lock. Any ordinary edit/transport operation revokes the whole cohort.
pub async fn active_zone(s:&AppState,id:&str)->bool {s.sync_starts.lock().await.jobs.values().any(|j|j.active()&&j.owns_zone(id))}
pub async fn cancel_zone(s:&AppState,id:&str)->anyhow::Result<()> {let mut registry=s.sync_starts.lock().await;for job in registry.jobs.values_mut().filter(|j|j.active()&&j.owns_zone(id)){job.abort(s,"音区控制已改变，整组播放撤销",true).await?;}let id=id.to_owned();s.db.run(move|c|{if c.query_row("SELECT EXISTS(SELECT 1 FROM sync_group_holds WHERE zone_id=?1)",[&id],|r|r.get::<_,bool>(0))?{c.execute("DELETE FROM sync_group_holds WHERE group_id IN (SELECT group_id FROM sync_group_holds WHERE zone_id=?1)",[id])?;}Ok(())}).await?;Ok(())}
pub async fn peer_changed(s:&AppState,id:&str)->anyhow::Result<()> {let mut registry=s.sync_starts.lock().await;for job in registry.jobs.values_mut().filter(|j|j.active()&&j.owns_endpoint(id)){job.abort(s,"网桥连接已改变",false).await?;}Ok(())}
/// Caller holds zone_lock and has verified that tx currently owns the endpoint.
fn report_timing(v:&Value)->Option<u32>{match v.get("measured_version"){None=>Some(1),Some(value)=>value.as_u64().and_then(|v|u32::try_from(v).ok()).filter(|v|[1,2].contains(v))}}
pub async fn report(s:&AppState,endpoint:&str,tx:&mpsc::Sender<Value>,v:&Value)->anyhow::Result<bool> {
    let mut registry=s.sync_starts.lock().await;
    let Some(job)=registry.jobs.values_mut().find(|j|j.members.iter().any(|m|m.zone.endpoint_id==endpoint&&m.tx.same_channel(tx)&&m.zone.id==v["zone_id"]&&(Some(m.playback_id.as_str())==v["core_playback_id"].as_str()||j.queue.as_ref().is_some_and(|q|v["core_playback_id"].as_str().is_some_and(|id|q.known[j.members.iter().position(|member|member.zone.id==m.zone.id).unwrap()].contains_key(id))))))else{return Ok(false);};
    if matches!(v["type"].as_str(),Some("state"|"start_state"))&&report_timing(v)!=Some(job.request.measured_version){return Ok(true);}
    let index=job.members.iter().position(|m|m.zone.endpoint_id==endpoint).unwrap();
    if control::draining(job)||job.phase=="paused"{control::handle_report(job,s,index,v).await?;return Ok(true);}
    if !job.active(){
        if job.phase=="ended"&&v["type"]=="start_state"&&v["start"]==job.members[index].token&&v["phase"]=="released"&&valid_gate(&job.members[index],v,"released"){job.members[index].gate=v["gate"].clone();job.persist(s).await?;}else{job.final_report(s,index,v).await?;}return Ok(true);
    }
    if matches!(v["type"].as_str(),Some("start_state"|"start_error")) {
        if Some(job.members[index].playback_id.as_str())!=v["core_playback_id"].as_str(){return Ok(true);}
        if v["version"]!=1||v["start"]!=job.members[index].token{return Ok(true);}
        // The joined receipt may follow an already committed per-member natural EOF.
        if v["type"]=="start_state"&&v["final"]==true&&job.queue.is_some()&&job.members[index].phase=="ended"&&job.members[index].final_report{return Ok(true);}
        if v["type"]=="start_error"||matches!(v["phase"].as_str(),Some("cancelled"|"expired")){
            let reason=if v["type"]=="start_error"{v["error"].as_str().filter(|s|!s.is_empty()&&s.len()<=1000).unwrap_or("端点拒绝起播")}else{"端点拒绝、取消或超期"};
            job.abort(s,reason,false).await?;job.final_report(s,index,v).await?;return Ok(true);
        }
        if v["phase"]=="ready"&&job.phase=="preparing" {
            let rate=v["sample_rate"].as_u64().and_then(|n|u32::try_from(n).ok()).filter(|n|(8000..=768_000).contains(n));
            if rate.is_none()||v["programme_frames_played"]!=0||!valid_gate(&job.members[index],v,"ready"){return Ok(true);}
            job.members[index].phase="ready".into();job.members[index].rate=rate.unwrap();job.members[index].gate=v["gate"].clone();
            if job.members.iter().all(|m|m.phase=="ready"){if let Err(error)=job.commit(s).await{job.abort(s,&error.to_string(),false).await?;}}
        }else if v["phase"]=="released"&&job.dispatched&&valid_gate(&job.members[index],v,"released") {job.members[index].gate=v["gate"].clone();if job.members[index].phase!="ended"{job.members[index].phase="playing".into();}job.phase="playing".into();}
        job.persist(s).await?;return Ok(true);
    }
    if job.queue.is_some(){queue::report(job,s,index,v).await?;return Ok(true);}
    if v["type"]=="error"||v["continuity_lost"]==true{
        let epoch_base=control::base(job);let m=&mut job.members[index];if job.dispatched&&v["sample_rate"]==m.rate{if let (Some(frames),Some(position))=(v["frames"].as_u64(),v["position"].as_f64()){if frames>=m.frames&&position.is_finite()&&(position-(epoch_base+frames as f64/m.rate as f64)).abs()<=0.002{m.zone.position=position;m.frames=frames;}}}
        job.abort(s,"端点输出中断",false).await?;return Ok(true);}
    if !job.dispatched||crate::clock::now_ns().saturating_add(job.members[index].uncertainty).saturating_add(1_000_000)<job.deadline.unwrap_or(u64::MAX){job.abort(s,"端点提前消费节目",false).await?;return Ok(true);}
    let epoch_base=control::base(job);let member=&mut job.members[index];let Some(frames)=v["frames"].as_u64()else{return Ok(true);};let Some(position)=v["position"].as_f64().filter(|p|p.is_finite())else{return Ok(true);};
    if v["sample_rate"]!=member.rate||frames<member.frames||(position-(epoch_base+frames as f64/member.rate as f64)).abs()>0.002{return Ok(true);}
    member.zone.position=position;member.last_seen=crate::clock::now_ns();
    if v["status"]=="ended" {
        member.frames=frames;member.phase="ended".into();member.final_report=true;member.zone.status="stopped".into();member.zone.playback_id=None;save_zones(s,vec![member.zone.clone()]).await?;
    }else if v["status"]=="playing"{member.frames=frames;member.phase="playing".into();s.zones.write().await.insert(member.zone.id.clone(),member.zone.clone());}
    let ended=job.members.iter().all(|m|m.phase=="ended");job.listen(s,if ended{Some("ended")}else{None}).await?;
    if ended{job.phase="ended".into();}job.persist(s).await?;Ok(true)
}
pub async fn tick(s:&AppState)->anyhow::Result<()> {
    let _lock=s.zone_lock.lock().await;let mut registry=s.sync_starts.lock().await;let now=crate::clock::now_ns();
    for job in registry.jobs.values_mut() {
        if !job.active(){if let Some(until)=job.settle_until {if now>=until{job.settle_until=None;job.persist(s).await?;}else{for member in job.members.iter().filter(|m|!m.final_report){let _=member.tx.try_send(json!({"type":"cancel_start","zone_id":member.zone.id,"playback_id":member.playback_id,"start":member.token}));}}}continue;}
        if job.phase=="preparing"&&now>=job.expires {job.abort(s,"整组准备超期",false).await?;continue;}
        if !s.session_active(job.controller.clone()).await{job.abort(s,"控制凭证已失效",false).await?;continue;}
        if control::draining(job){control::tick(job,s).await?;continue;}
        if job.queue.is_some()&&job.members.iter().all(|m|m.phase=="ended"){if let Err(error)=queue::advance(job,s).await{job.abort(s,&error.to_string(),false).await?;}continue;}
        if let Err(error)=job.calibrations(s).await{job.abort(s,&error.to_string(),false).await?;continue;}
        if control::tick(job,s).await?{continue;}
        drift::tick(job,s).await?;
        policy::tick(job,s).await?;
        if job.dispatched&&now>job.deadline.unwrap_or(now)+QUIET_NS&&job.members.iter().any(|m|m.phase!="ended"&&now.saturating_sub(m.last_seen)>QUIET_NS){job.abort(s,"端点输出进度超时",false).await?;}
    }registry.compact(now);Ok(())
}
pub fn monitor(s:AppState){tokio::spawn(async move{let mut timer=tokio::time::interval(std::time::Duration::from_millis(250));loop{timer.tick().await;if let Err(error)=tick(&s).await{tracing::warn!(%error,"group start monitor failed");}}});}
pub fn restart(c:&mut rusqlite::Connection)->anyhow::Result<()> {
    let rows=c.prepare("SELECT id,result FROM sync_starts")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;let tx=c.transaction()?;
    for (id,data) in rows {let mut result:Value=serde_json::from_str(&data)?;if !matches!(result["phase"].as_str(),Some("preparing"|"committed"|"playing"|"pausing"|"paused"|"seeking")){if result["settled"]==false{result["settled"]=json!(true);tx.execute("UPDATE sync_starts SET result=?2 WHERE id=?1",rusqlite::params![id,result.to_string()])?;}continue;}
        recovery::hold_tx(&tx,&id,&result)?;
        for m in result["members"].as_array().into_iter().flatten(){let zone=m["zone_id"].as_str().unwrap_or("");let data=tx.query_row("SELECT data FROM zones WHERE id=?1",[zone],|r|r.get::<_,String>(0)).optional()?;if let Some(data)=data{let mut z:Zone=serde_json::from_str(&data)?;if z.playback_id.is_some()&&z.playback_id.as_deref()==m["current_playback_id"].as_str().or_else(||m["playback_id"].as_str()){z.status="paused".into();z.playback_id=None;tx.execute("UPDATE zones SET data=?2 WHERE id=?1",rusqlite::params![zone,serde_json::to_string(&z)?])?;}}}
        tx.execute("UPDATE play_sessions SET finished_at=?2,finish_reason='disconnect',last_status='stopped' WHERE (zone_id=?1 OR zone_id GLOB ?3) AND finished_at IS NULL",rusqlite::params![format!("sync:{id}"),chrono::Utc::now().timestamp_millis(),format!("sync:{id}:*")])?;
        if result["transport"].is_object(){result["transport"]["state"]=json!("failed");result["transport"]["error"]=json!("Core 重启，控制未重放");}
        if result["rate_adjustment"]["automatic"]==true{result["rate_adjustment"]["controller"]["state"]=json!("inactive");for member in result["members"].as_array_mut().into_iter().flatten(){member["requested_rate_ppm"]=json!(0.);}}
        result["phase"]=json!("interrupted");result["settled"]=json!(true);result["error"]=json!("Core 重启，整组计划不自动恢复");tx.execute("UPDATE sync_starts SET result=?2 WHERE id=?1",rusqlite::params![id,result.to_string()])?;
    }tx.commit()?;Ok(())
}
