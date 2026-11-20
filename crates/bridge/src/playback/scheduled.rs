//! Connection-bound PCM prepare/commit. A retired generation cannot be replayed.
use super::*;
use rmusic_audio::output::start::Phase;
use serde::Deserialize;
#[derive(Clone,PartialEq,Eq,Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {version:u32,plan_id:String,generation:u64,core_clock_id:String,host_clock_id:String,expires_host_ns:u64,#[serde(default)]group_power:Option<Value>}
pub(super) struct Identity {core:String,host:String}
pub(super) type Receipt=Arc<std::sync::Mutex<Option<Value>>>;
pub(super) struct Ticket {rate_expires:Option<u64>,token:Token,command:Value,control:Arc<PlaybackControl>,last_phase:String,cancelled:bool,expired:bool,worker_started:bool,finished:Arc<std::sync::atomic::AtomicBool>,receipt:Receipt,journal:Option<rmusic_bridge::listens::group::Recorder>}
pub(super) fn timing(command:&Value)->Result<u32>{
    let version=match command.get("measured_version"){None=>1,Some(value)=>match value.as_u64(){Some(1)=>1,Some(2) if command["group_queue"]==true=>2,_=>bail!("invalid scheduled output timing version")}};
    let adjusted=command.get("pcm_rate_adjustment").map(|v|v.as_bool().context("PCM rate adjustment requires a boolean")).transpose()?.unwrap_or(false);
    let automatic=command.get("automatic_rate_adjustment").map(|v|v.as_bool().context("automatic rate adjustment requires a boolean")).transpose()?.unwrap_or(false);
    if automatic&&!adjusted{bail!("automatic rate adjustment requires explicit PCM adjustment");}
    if adjusted&&(version!=2||!Playback::rate_adjustment_supported()||command["track"]["bit_depth"]==1||matches!(command["track"]["format"].as_str(),Some("dsf"|"dff"))){bail!("PCM rate adjustment requires supported v2 PCM group output");}Ok(version)
}
fn rate_expiry(command:&Value,automatic:bool,now:u64)->Result<Option<u64>> {
    if automatic {let expiry=command["rate_expires_host_ns"].as_u64().filter(|n|*n>now&&n.saturating_sub(now)<=2_000_000_000).context("automatic rate target requires a live bounded host lease")?;Ok(Some(expiry))}
    else if command.get("rate_expires_host_ns").is_some(){bail!("manual rate target cannot carry an automatic lease")}
    else{Ok(None)}
}
fn report(command:&Value,control:&PlaybackControl,phase:&str,final_report:bool)->Value {
    let played=control.frames_played.load(Ordering::Acquire).min(control.frames_written.load(Ordering::Acquire)).min(control.programme_end.load(Ordering::Acquire));
    let mut value=json!({"type":"start_state","version":1,"start":command["start"],"phase":phase,"final":final_report,"zone_id":command["zone_id"],"track_id":command["track"]["id"],"core_playback_id":command["playback_id"],"gate":control.start.snapshot(),"sample_rate":control.output_sample_rate.load(Ordering::Acquire),"programme_frames_played":played,"buffered_frames":control.frames_written.load(Ordering::Relaxed).saturating_sub(played)});
    if command["measured_version"]==2{value["measured_version"]=json!(2);}
    if final_report&&command["group_queue"]==true{value["sequence_final"]=json!([]);}value
}
pub(super) fn final_report(command:&Value,control:&PlaybackControl)->Value {report(command,control,"cancelled",true)}
impl Playback {
    pub fn rate_adjustment_supported()->bool {cfg!(feature="adaptive-sync")&&output::PlaybackSequence::rate_adjustment_supported()}
    pub(super) fn rate_adjustment_requested(&self,control:&Arc<PlaybackControl>)->bool {self.scheduled.as_ref().is_some_and(|t|Arc::ptr_eq(&t.control,control)&&t.command["pcm_rate_adjustment"]==true)}
    pub async fn admit_start(&self,command:&Value)->Result<()> {
        #[cfg(any(windows,target_os="linux"))] {
            let control=self.match_start(command,false)?;let token=self.token(command)?;
            let stamp=token.group_power.map(serde_json::from_value::<output::power::GroupStamp>).transpose()?;
            tokio::task::spawn_blocking(move||output::power::admit_group(stamp.as_ref(),&control)).await??;
        }
        Ok(())
    }
    pub(super) fn starting_output(&mut self,control:&Arc<PlaybackControl>) {if let Some(ticket)=self.scheduled.as_mut().filter(|t|Arc::ptr_eq(&t.control,control)){ticket.worker_started=true;}}
    pub(super) fn final_start(&self,control:&Arc<PlaybackControl>)->Option<(Value,Arc<std::sync::atomic::AtomicBool>,Receipt,Option<rmusic_bridge::listens::group::Recorder>)> {self.scheduled.iter().chain(self.retired_starts.iter().map(|(_,ticket)|ticket)).find(|t|Arc::ptr_eq(&t.control,control)).map(|t|(t.command.clone(),t.finished.clone(),t.receipt.clone(),t.journal.clone()))}
    pub fn clock_identity(&mut self,core:&str,host:&str) {
        if self.clock_identity.as_ref().is_some_and(|id|id.core==core&&id.host==host){return;}
        self.cancel_scheduled();self.retired_starts.clear();self.start_generation=0;self.clock_identity=Some(Identity{core:core.into(),host:host.into()});
    }
    fn token(&self,command:&Value)->Result<Token> {
        timing(command)?;
        let token:Token=serde_json::from_value(command["start"].clone())?;
        if token.version!=1||token.generation==0||uuid::Uuid::parse_str(&token.plan_id).is_err(){bail!("invalid scheduled start token");}
        if !self.clock_identity.as_ref().is_some_and(|id|token.core_clock_id==id.core&&token.host_clock_id==id.host){bail!("scheduled start clock identity changed");}Ok(token)
    }
    pub fn duplicate_start(&self,command:&Value)->Result<bool> {
        let token=self.token(command)?;
        if token.generation<=self.start_generation {
            if self.scheduled.as_ref().is_some_and(|ticket|ticket.token==token&&ticket.command==*command){return Ok(true);}
            bail!("scheduled start generation was retired");
        }Ok(false)
    }
    pub(super) fn arm_start(&mut self,command:&Value,control:&Arc<PlaybackControl>)->Result<()> {
        let token=self.token(command)?;
        if token.generation<=self.start_generation||uuid::Uuid::parse_str(command["playback_id"].as_str().unwrap_or("")).is_err()||command["zone_id"].as_str().is_none_or(|s|s.is_empty()||s.len()>200)||command["track"]["id"].as_str().is_none_or(|s|s.is_empty()||s.len()>200)||command["paused"]==true{bail!("invalid scheduled playback instance");}
        control.start.arm(token.expires_host_ns)?;self.start_generation=token.generation;
        self.forget_start();
        let journal=if timing(command)?==2 {self.listener.as_ref().zip(self.sequence_config.as_ref()).map(|((store,owner),config)|rmusic_bridge::listens::group::Recorder::new(store.clone(),owner.clone(),config.endpoint.clone()))}else{None};
        self.scheduled=Some(Ticket{rate_expires:None,token,command:command.clone(),control:control.clone(),last_phase:String::new(),cancelled:false,expired:false,worker_started:false,finished:Default::default(),receipt:Default::default(),journal});Ok(())
    }
    pub(super) fn commit_start(&mut self,command:&Value)->Result<()> {
        let control=self.match_start(command,false)?;let deadline=command["deadline_host_ns"].as_u64().context("commit requires host deadline")?;
        let snapshot=control.start.snapshot().context("start gate unavailable")?;
        if snapshot.phase==Phase::Released&&snapshot.deadline_host_ns==deadline{self.replay_start_report();return Ok(());}
        control.start.commit(deadline)?;self.replay_start_report();Ok(())
    }
    pub(super) fn group_volume(&mut self,command:&Value)->Result<()> {
        let volume=command["volume"].as_f64().filter(|v|v.is_finite()&&(0. ..=1.).contains(v)).context("group volume requires 0..1")?;
        let muted=command["muted"].as_bool().context("group mute requires a boolean")?;
        let control=self.match_start(command,false)?;
        if control.start.snapshot().is_none_or(|s|s.phase!=Phase::Released){bail!("group volume requires released output");}
        control.set_volume(volume)?;control.muted.store(muted,Ordering::Relaxed);Ok(())
    }
    pub(super) fn group_rate(&mut self,command:&Value)->Result<()> {
        let ppm=command["ppm"].as_f64().filter(|v|v.is_finite()&&v.abs()<=500.).context("group rate requires finite ±500 ppm")?;
        let control=self.match_start(command,false)?;
        if timing(command)?!=2||self.scheduled.as_ref().is_none_or(|t|t.command["pcm_rate_adjustment"]!=true)||control.start.snapshot().is_none_or(|s|s.phase!=Phase::Released){bail!("group rate requires opted-in released v2 output");}
        let automatic=self.scheduled.as_ref().unwrap().command["automatic_rate_adjustment"]==true;
        let expires=rate_expiry(command,automatic,output::clock::host_ns().context("host clock unavailable")?)?;
        self.running.as_ref().and_then(|r|r.sequence.as_ref()).context("group sequence unavailable")?.set_rate_ppm(ppm)?;self.scheduled.as_mut().unwrap().rate_expires=expires;Ok(())
    }
    pub fn check_rate_lease(&mut self)->Result<()> {
        let Some(now)=output::clock::host_ns() else{return Ok(());};
        let Some(ticket)=self.scheduled.as_mut().filter(|t|t.rate_expires.is_some_and(|expiry|now>=expiry)) else{return Ok(());};
        ticket.rate_expires=None;
        if let Some(run)=self.running.as_ref().filter(|r|Arc::ptr_eq(&r.control,&ticket.control)&&!r.control.stopped.load(Ordering::Acquire)&&!r.control.paused.load(Ordering::Relaxed)).and_then(|r|r.sequence.as_ref()){run.set_rate_ppm(0.)?;eprintln!("Automatic PCM rate lease expired; producer target reset to 0 ppm");}Ok(())
    }
    fn match_start(&self,command:&Value,allow_stopped:bool)->Result<Arc<PlaybackControl>> {
        let token=self.token(command)?;let ticket=self.scheduled.as_ref().filter(|ticket|ticket.token==token&&!ticket.cancelled&&!ticket.expired&&ticket.command["playback_id"]==command["playback_id"]&&ticket.command["zone_id"]==command["zone_id"]).context("scheduled start instance changed")?;
        let control=ticket.control.clone();
        if !self.control().is_some_and(|current|Arc::ptr_eq(current,&control))||!allow_stopped&&control.stopped.load(Ordering::Acquire){bail!("scheduled start was stopped");}Ok(control)
    }
    pub(super) fn cancel_start(&mut self,command:&Value)->Result<()> {
        let token=self.token(command)?;
        self.prune_retired_starts();
        if let Some((_,ticket))=self.retired_starts.iter().find(|(_,ticket)|ticket.token==token&&ticket.command["playback_id"]==command["playback_id"]&&ticket.command["zone_id"]==command["zone_id"]) {
            let value=if ticket.worker_started {
                if !ticket.finished.load(Ordering::Acquire){return Ok(());}
                ticket.receipt.lock().map_err(|_|anyhow::anyhow!("retired receipt lock poisoned"))?.clone().context("retired final receipt unavailable")?
            }else{report(&ticket.command,&ticket.control,if ticket.expired{"expired"}else{"cancelled"},true)};
            if !ticket.worker_started {if let Some(journal)=&ticket.journal{journal.capture(&value)?;}}
            self.output_reports.as_ref().context("output receipt channel unavailable")?.send(value).context("retired receipt replay failed")?;
            return Ok(());
        }
        if self.scheduled.as_ref().is_some_and(|t|t.token==token&&(t.cancelled||t.expired)&&t.command["playback_id"]==command["playback_id"]&&t.command["zone_id"]==command["zone_id"]){self.replay_start_report();return Ok(());}
        let _=self.match_start(command,true)?;self.cancel_scheduled();self.replay_start_report();Ok(())
    }
    fn cancel_scheduled(&mut self) {
        if let Some(ticket)=&mut self.scheduled{ticket.cancelled=true;ticket.control.stop();}
        if self.control().is_some_and(|control|control.start.is_scheduled()){self.cancel_pending();self.retire();}
    }
    pub(super) fn cancel_waiting_start(&mut self,kind:&str,command:&Value)->Result<bool> {
        let waiting=self.scheduled.as_ref().is_some_and(|ticket|ticket.control.start.snapshot().is_some_and(|s|s.phase!=Phase::Released));
        if kind=="stop"||kind=="flush"||waiting&&matches!(kind,"pause"|"seek"|"dsp"|"volume"|"cancel_preload"){
            if kind=="volume"&&command["volume"].as_f64().is_none_or(|v|!v.is_finite()||!(0. ..=1.).contains(&v)){bail!("volume requires a value in 0..1");}
            if kind=="seek"{position(command["position"].as_f64().context("seek requires position")?)?;}
            if kind=="dsp"{let _:DspConfig=serde_json::from_value(command["dsp"].clone())?;}
            self.cancel_scheduled();self.replay_start_report();return Ok(true);
        }Ok(false)
    }
    fn prune_retired_starts(&mut self){self.retired_starts.retain(|(retired,_)|retired.elapsed()<Duration::from_secs(6));}
    pub(super) fn forget_start(&mut self){
        self.prune_retired_starts();
        if let Some(ticket)=self.scheduled.take(){
            if self.clock_identity.as_ref().is_some_and(|id|ticket.token.core_clock_id==id.core&&ticket.token.host_clock_id==id.host){self.retired_starts.push_back((Instant::now(),ticket));}
        }
        while self.retired_starts.len()>64{self.retired_starts.pop_front();}
    }
    pub(super) fn replay_start_report(&mut self){if let Some(ticket)=&mut self.scheduled{ticket.last_phase.clear();}}
    pub fn start_report(&mut self)->Option<Value> {
        self.prune_retired_starts();
        let ticket=self.scheduled.as_mut()?;let control=ticket.control.clone();let snapshot=control.start.snapshot();
        let expired=snapshot.is_some_and(|s|matches!(s.phase,Phase::Armed|Phase::Ready|Phase::Committed))&&output::clock::host_ns().is_some_and(|now|now>=ticket.token.expires_host_ns);
        if expired {ticket.expired=true;control.stop();}
        let phase=if ticket.expired{"expired".into()}else if ticket.cancelled{"cancelled".into()}else{snapshot.map(|s|serde_json::to_value(s.phase).unwrap().as_str().unwrap().to_owned()).unwrap_or_else(||"cancelled".into())};
        let final_frames=matches!(phase.as_str(),"cancelled"|"expired")&&(!ticket.worker_started||ticket.finished.load(Ordering::Acquire));let state=format!("{phase}:{final_frames}");
        if ticket.last_phase==state{return None;}ticket.last_phase=state;
        let report=if final_frames&&ticket.worker_started {
            let mut cached=ticket.receipt.lock().ok()?.clone()?;cached["phase"]=json!(phase);cached
        }else{report(&ticket.command,&control,&phase,final_frames)};
        if final_frames&&!ticket.worker_started {if let Some(journal)=&ticket.journal {if let Err(error)=journal.capture(&report){eprintln!("Group final persistence failed: {error:#}");}}}
        if expired{self.cancel_pending();self.retire();}Some(report)
    }
    pub fn start_error(command:&Value,error:impl std::fmt::Display)->Value {json!({"type":"start_error","version":1,"start":command["start"],"zone_id":command["zone_id"],"core_playback_id":command["playback_id"],"error":error.to_string()})}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drift_controller_native_lease_requires_finite_future_deadline_and_manual_separation(){
        for value in [json!(null),json!(999),json!(1000),json!(2_000_001_001u64),json!("2000")]{assert!(rate_expiry(&json!({"rate_expires_host_ns":value}),true,1000).is_err());}
        assert_eq!(rate_expiry(&json!({"rate_expires_host_ns":1_500_001_000u64}),true,1000).unwrap(),Some(1_500_001_000));assert!(rate_expiry(&json!({"rate_expires_host_ns":2000}),false,1000).is_err());assert_eq!(rate_expiry(&json!({}),false,1000).unwrap(),None);
    }
    #[tokio::test]
    async fn group_rate_rejects_invalid_unreleased_and_stale_targets_without_stopping_output() {
        let(profile,server)=super::super::tests::server(axum::Router::new()).await;let running=super::super::tests::running(&profile).await;let control=running.control.clone();
        let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let mut playback=Playback::default();playback.clock_identity(&core,&host);let mut cmd=command(&core,&host,1);cmd["measured_version"]=json!(2);cmd["group_queue"]=json!(true);
        for bad in [{let mut c=cmd.clone();c["pcm_rate_adjustment"]=json!("yes");c},{let mut c=cmd.clone();c["pcm_rate_adjustment"]=json!(true);c["track"]["bit_depth"]=json!(1);c}] {assert!(playback.duplicate_start(&bad).is_err());assert_eq!(playback.start_generation,0);}
        let mut opt=cmd.clone();opt["pcm_rate_adjustment"]=json!(true);assert_eq!(timing(&opt).is_ok(),Playback::rate_adjustment_supported());
        playback.arm_start(&cmd,&control).unwrap();playback.running=Some(running);let mut target=cmd.clone();target["type"]=json!("group_rate");target["ppm"]=json!(500.);
        for bad in [target.clone(),{let mut c=target.clone();c["ppm"]=json!(501.);c},{let mut c=target.clone();c["start"]["generation"]=json!(2);c},{let mut c=target.clone();c["playback_id"]=json!(uuid::Uuid::new_v4().to_string());c}] {
            assert!(playback.command("group_rate",&bad,&profile).is_err());assert!(!control.stopped.load(Ordering::Acquire));assert_eq!(control.start.snapshot().unwrap().phase,Phase::Armed);assert_eq!(playback.scheduled.as_ref().unwrap().command,cmd);
        }server.abort();
    }
    #[tokio::test]
    async fn group_volume_rejects_unreleased_invalid_and_stale_commands_without_retiring_output() {
        let(profile,server)=super::super::tests::server(axum::Router::new()).await;let running=super::super::tests::running(&profile).await;let control=running.control.clone();control.set_volume(0.7).unwrap();control.muted.store(false,Ordering::Relaxed);
        let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let mut playback=Playback::default();playback.clock_identity(&core,&host);let cmd=command(&core,&host,1);playback.arm_start(&cmd,&control).unwrap();playback.running=Some(running);
        let mut volume=cmd.clone();volume["type"]=json!("group_volume");volume["volume"]=json!(0.2);volume["muted"]=json!(true);
        for bad in [volume.clone(),{let mut v=volume.clone();v["volume"]=json!(2.);v},{let mut v=volume.clone();v["muted"]=json!("yes");v},{let mut v=volume.clone();v["start"]["generation"]=json!(2);v},{let mut v=volume.clone();v["playback_id"]=json!(uuid::Uuid::new_v4().to_string());v}] {
            let error=playback.command("group_volume",&bad,&profile).unwrap_err();if bad==volume{assert!(error.to_string().contains("released output"));}assert_eq!(control.volume_setting(),0.7);assert!(!control.muted.load(Ordering::Relaxed));assert!(!control.stopped.load(Ordering::Acquire));assert_eq!(control.start.snapshot().unwrap().phase,Phase::Armed);assert_eq!(playback.scheduled.as_ref().unwrap().command,cmd);
        }server.abort();
    }
    fn command(core:&str,host:&str,generation:u64)->Value {json!({"type":"prepare_start","playback_id":uuid::Uuid::new_v4().to_string(),"zone_id":"z","track":{"id":"t"},"start":{"version":1,"plan_id":uuid::Uuid::new_v4().to_string(),"generation":generation,"core_clock_id":core,"host_clock_id":host,"expires_host_ns":output::clock::host_ns().unwrap()+5_000_000_000u64}})}
    #[tokio::test]
    async fn group_final_v2_journal_captures_joined_frames_and_keeps_the_armed_account() {
        let(profile,server)=super::super::tests::server(axum::Router::new()).await;let mut running=super::super::tests::running(&profile).await;let control=running.control.clone();let dir=tempfile::tempdir().unwrap();let store=ListenStore::open(dir.path()).unwrap();let owner=ListenOwner{core_id:"qa-core".into(),user_id:"old".into()};let next_owner=ListenOwner{user_id:"new".into(),..owner.clone()};
        let mut playback=Playback::connected(profile.clone(),"token".into(),"endpoint".into()).unwrap();playback.attach_journal(store.clone(),owner.clone());let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();playback.clock_identity(&core,&host);let mut cmd=command(&core,&host,1);cmd["measured_version"]=json!(2);cmd["group_queue"]=json!(true);playback.arm_start(&cmd,&control).unwrap();playback.starting_output(&control);
        let(reports,mut received)=tokio::sync::mpsc::unbounded_channel();playback.output_reports=Some(reports);control.output_sample_rate.store(768000,Ordering::Release);control.frames_written.store(100,Ordering::Release);control.frames_played.store(27,Ordering::Release);let(release,wait)=std::sync::mpsc::channel();let worker=control.clone();running.thread=Some(std::thread::spawn(move||{wait.recv().unwrap();worker.frames_played.store(43,Ordering::Release);}));playback.running=Some(running);
        playback.cancel_start(&cmd).unwrap();assert_eq!(playback.start_report().unwrap()["final"],false);assert!(rmusic_bridge::listens::group::pending(&store,&owner).unwrap().is_empty());playback.attach_journal(store.clone(),next_owner.clone());release.send(()).unwrap();playback.update().await;let final_state=tokio::time::timeout(Duration::from_secs(2),received.recv()).await.unwrap().unwrap();assert_eq!(final_state["programme_frames_played"],43);
        let first=rmusic_bridge::listens::group::pending(&store,&owner).unwrap().remove(0).body;assert_eq!(first["report"],final_state);assert!(rmusic_bridge::listens::group::pending(&store,&next_owner).unwrap().is_empty());control.frames_played.store(99,Ordering::Release);playback.replay_start_report();playback.start_report();assert_eq!(rmusic_bridge::listens::group::pending(&store,&owner).unwrap()[0].body,first);
        let next_control=Arc::new(PlaybackControl::default());let mut next=command(&core,&host,2);next["measured_version"]=json!(2);next["group_queue"]=json!(true);playback.arm_start(&next,&next_control).unwrap();next_control.stop();assert_eq!(playback.start_report().unwrap()["final"],true);assert_eq!(rmusic_bridge::listens::group::pending(&store,&next_owner).unwrap().len(),1);
        server.abort();
    }
    #[tokio::test]
    async fn scheduled_protocol_final_frames_wait_for_worker_join_and_keep_retired_identity() {
        let(profile,server)=super::super::tests::server(axum::Router::new().route("/api/v1/media/fast",axum::routing::get(||async{super::super::tests::wav()}))).await;
        let mut running=super::super::tests::running(&profile).await;let control=running.control.clone();let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let cmd=command(&core,&host,1);
        let(reports,mut received)=tokio::sync::mpsc::unbounded_channel();let mut playback=Playback::default();playback.clock_identity(&core,&host);playback.arm_start(&cmd,&control).unwrap();playback.starting_output(&control);playback.output_reports=Some(reports);
        control.output_sample_rate.store(48000,Ordering::Release);control.frames_written.store(100,Ordering::Release);control.frames_played.store(27,Ordering::Release);
        let(release,wait)=std::sync::mpsc::channel();let worker=control.clone();running.thread=Some(std::thread::spawn(move||{wait.recv().unwrap();worker.frames_played.store(43,Ordering::Release);}));playback.running=Some(running);
        playback.cancel_start(&cmd).unwrap();let early=playback.start_report().unwrap();assert_eq!(early["final"],false);assert_eq!(early["programme_frames_played"],27);assert!(received.try_recv().is_err());
        let ticket=playback.scheduled.take().unwrap();release.send(()).unwrap();playback.update().await;
        let final_state=tokio::time::timeout(Duration::from_secs(2),received.recv()).await.unwrap().unwrap();assert_eq!(final_state["final"],true);assert_eq!(final_state["programme_frames_played"],43);assert_eq!(final_state["start"],cmd["start"]);assert_eq!(final_state["core_playback_id"],cmd["playback_id"]);assert!(playback.accepts(&final_state));
        playback.scheduled=Some(ticket);control.frames_played.store(99,Ordering::Release);playback.replay_start_report();
        assert_eq!(playback.start_report().unwrap(),final_state,"retry must reuse the immutable joined receipt");playback.forget_start();server.abort();
    }
    #[tokio::test]
    async fn scheduled_protocol_old_final_receipt_survives_new_start_before_or_after_join() {
        let(profile,server)=super::super::tests::server(axum::Router::new()).await;
        for joined in [false,true] {
            let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let mut old=command(&core,&host,1);old["group_queue"]=json!(true);
            let old_control=Arc::new(PlaybackControl::default());let(reports,mut received)=tokio::sync::mpsc::unbounded_channel();let mut playback=Playback::default();playback.clock_identity(&core,&host);playback.output_reports=Some(reports);
            playback.arm_start(&old,&old_control).unwrap();playback.starting_output(&old_control);old_control.stop();
            old_control.output_sample_rate.store(48000,Ordering::Release);old_control.frames_written.store(100,Ordering::Release);old_control.frames_played.store(43,Ordering::Release);
            let mut expected=final_report(&old,&old_control);expected["sequence_final"]=json!([{"playback_id":old["playback_id"],"track_id":"t","position":43./48000.,"frames":43,"ended":false}]);
            let ticket=playback.scheduled.as_mut().unwrap();ticket.cancelled=true;let finished=ticket.finished.clone();let receipt=ticket.receipt.clone();
            if joined{*receipt.lock().unwrap()=Some(expected.clone());finished.store(true,Ordering::Release);}
            let next=command(&core,&host,2);let running=super::super::tests::running(&profile).await;let current=running.control.clone();playback.running=Some(running);playback.arm_start(&next,&current).unwrap();
            assert!(playback.cancel_start(&old).is_ok(),"retired final receipt was forgotten by a newer start");
            if !joined{assert!(received.try_recv().is_err(),"must wait for the old worker join");*receipt.lock().unwrap()=Some(expected.clone());finished.store(true,Ordering::Release);playback.cancel_start(&old).unwrap();}
            assert_eq!(received.try_recv().unwrap(),expected);playback.cancel_start(&old).unwrap();assert_eq!(received.try_recv().unwrap(),expected);assert!(!current.stopped.load(Ordering::Acquire));assert_eq!(playback.scheduled.as_ref().unwrap().command,next);assert_eq!(playback.start_generation,2);
            let mut forged=old.clone();forged["zone_id"]=json!("foreign-zone");assert!(playback.cancel_start(&forged).is_err());assert!(received.try_recv().is_err());assert!(!current.stopped.load(Ordering::Acquire));
            assert!(playback.duplicate_start(&old).is_err());assert!(playback.commit_start(&old).is_err());
        }
        server.abort();
    }
    #[tokio::test]
    async fn scheduled_protocol_retired_receipts_expire_and_clock_changes_clear_them_without_stopping_normal_output() {
        let(profile,server)=super::super::tests::server(axum::Router::new()).await;let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let old=command(&core,&host,1);
        let(reports,mut received)=tokio::sync::mpsc::unbounded_channel();let mut playback=Playback::default();playback.clock_identity(&core,&host);playback.output_reports=Some(reports);
        let old_control=Arc::new(PlaybackControl::default());playback.arm_start(&old,&old_control).unwrap();old_control.stop();playback.forget_start();
        let running=super::super::tests::running(&profile).await;let current=running.control.clone();playback.running=Some(running);
        playback.cancel_start(&old).unwrap();let value=received.try_recv().unwrap();assert_eq!(value["final"],true);assert_eq!(value["programme_frames_played"],0);assert_eq!(value["start"],old["start"]);assert!(!current.stopped.load(Ordering::Acquire));
        playback.retired_starts.front_mut().unwrap().0=Instant::now()-Duration::from_secs(7);assert!(playback.cancel_start(&old).is_err());assert!(received.try_recv().is_err());assert!(!current.stopped.load(Ordering::Acquire));
        let next=command(&core,&host,2);playback.arm_start(&next,&Arc::new(PlaybackControl::default())).unwrap();playback.forget_start();assert!(!playback.retired_starts.is_empty());
        playback.clock_identity(&uuid::Uuid::new_v4().to_string(),&host);assert!(playback.retired_starts.is_empty());assert!(playback.cancel_start(&next).is_err());assert!(received.try_recv().is_err());assert!(!current.stopped.load(Ordering::Acquire));server.abort();
    }
    #[tokio::test]
    async fn scheduled_protocol_cancel_stopped_worker_still_joins_and_confirms_final_frames() {
        let(profile,server)=super::super::tests::server(axum::Router::new()).await;
        let running=super::super::tests::running(&profile).await;let control=running.control.clone();
        let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let cmd=command(&core,&host,1);
        let(reports,mut received)=tokio::sync::mpsc::unbounded_channel();let mut playback=Playback::default();playback.clock_identity(&core,&host);
        playback.arm_start(&cmd,&control).unwrap();playback.starting_output(&control);playback.output_reports=Some(reports);playback.running=Some(running);
        control.output_sample_rate.store(48000,Ordering::Release);control.frames_written.store(53,Ordering::Release);control.frames_played.store(41,Ordering::Release);control.stop();
        let mut commit=cmd.clone();commit["deadline_host_ns"]=json!(output::clock::host_ns().unwrap()+100_000_000u64);assert!(playback.commit_start(&commit).is_err());
        assert!(playback.cancel_start(&cmd).is_ok(),"stopped output still requires the original cancellation join");
        playback.update().await;let final_state=tokio::time::timeout(Duration::from_secs(2),received.recv()).await.unwrap().unwrap();
        assert_eq!(final_state["final"],true);assert_eq!(final_state["programme_frames_played"],41);assert_eq!(final_state["start"],cmd["start"]);server.abort();
    }
    #[test]
    fn scheduled_protocol_binds_clock_instance_generation_and_duplicate_preparation() {
        let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let mut playback=Playback::default();let first=command(&core,&host,1);assert!(playback.duplicate_start(&first).is_err());playback.clock_identity(&core,&host);
        let control=Arc::new(PlaybackControl::default());playback.arm_start(&first,&control).unwrap();assert!(playback.duplicate_start(&first).unwrap());
        let mut altered=first.clone();altered["position"]=json!(1.);assert!(playback.duplicate_start(&altered).is_err());
        let second=command(&core,&host,2);assert!(!playback.duplicate_start(&second).unwrap());playback.arm_start(&second,&Arc::new(PlaybackControl::default())).unwrap();assert!(playback.duplicate_start(&first).is_err());
        playback.clock_identity(&uuid::Uuid::new_v4().to_string(),&host);assert!(playback.duplicate_start(&second).is_err());assert_eq!(control.start.snapshot().unwrap().released_host_ns,0);
    }
    #[test]
    fn scheduled_protocol_expiry_cancels_unprepared_sources_and_rejects_commit() {
        let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let mut playback=Playback::default();playback.clock_identity(&core,&host);let mut cmd=command(&core,&host,1);let control=Arc::new(PlaybackControl::default());playback.arm_start(&cmd,&control).unwrap();
        cmd["deadline_host_ns"]=json!(output::clock::host_ns().unwrap()+100_000_000u64);assert!(playback.commit_start(&cmd).is_err());assert_eq!(control.start.snapshot().unwrap().released_host_ns,0);
        playback.scheduled.as_mut().unwrap().token.expires_host_ns=0;let report=playback.start_report().unwrap();assert_eq!(report["phase"],"expired");assert!(control.stopped.load(Ordering::Acquire));assert!(playback.start_report().is_none());
    }
    #[test]
    fn group_wire_v2_native_scheduled_reports_and_duplicate_ownership_keep_timing() {
        let core=uuid::Uuid::new_v4().to_string();let host=uuid::Uuid::new_v4().to_string();let mut playback=Playback::default();playback.clock_identity(&core,&host);
        let mut cmd=command(&core,&host,1);cmd["group_queue"]=json!(true);cmd["measured_version"]=json!(2);let control=Arc::new(PlaybackControl::default());
        let mut bad=cmd.clone();bad["measured_version"]=json!(3);assert!(playback.arm_start(&bad,&control).is_err());assert!(playback.scheduled.is_none());
        playback.arm_start(&cmd,&control).unwrap();assert!(playback.duplicate_start(&cmd).unwrap());bad=cmd.clone();bad["measured_version"]=json!(1);assert!(playback.duplicate_start(&bad).is_err());
        let ready=report(&cmd,&control,"ready",false);assert_eq!(ready["measured_version"],2);assert_eq!(ready["programme_frames_played"],0);
        let final_value=final_report(&cmd,&control);assert_eq!(final_value["measured_version"],2);assert_eq!(final_value["sequence_final"],json!([]));assert_eq!(final_value["start"],cmd["start"]);
        bad=cmd.clone();bad["group_queue"]=json!(false);assert!(timing(&bad).is_err());assert_eq!(control.frames_written.load(Ordering::Relaxed),0);
    }

}
