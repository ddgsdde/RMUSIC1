use anyhow::{bail, Context, Result};
use rmusic_audio::{
    fade::FadeConfig,
    output::{self, OutputConfig, PlaybackControl, PlaybackRequest},
    probe::AudioMetadata,
    impulse::{ConvolutionConfig,ImpulseBank},
    DspConfig, DspProcessor,
};

fn measured_loudness(dsp: &DspConfig, track: &Value) -> Option<f64> {
    dsp.peak_limited_measurement(track["integrated_lufs"].as_f64(),
        track["true_peak_dbfs"].as_f64().or_else(|| track["provider_data"]["analysis"]["loudness"]["true_peak_dbfs"].as_f64()))
}
use rmusic_bridge::{connection::ConnectionProfile, media_proxy::MediaProxy};
use rmusic_bridge::sequence::Preparation;
use rmusic_bridge::listens::{Owner as ListenOwner,Store as ListenStore};
use serde_json::{json, Value};
use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use tokio::{sync::mpsc::UnboundedSender, task::JoinHandle};
mod sequence;
mod scheduled;
use rmusic_bridge::sequence::recovery::Restored;
use rmusic_bridge::listens::sync::Sync as ListenSync;
#[cfg(test)]
mod impulse_tests;
#[cfg(test)]
mod preparation_tests;

#[derive(Clone)]
struct Audio {
    metadata: AudioMetadata,
    proxy: Arc<MediaProxy>,
}
struct Plan {
    #[cfg(target_os="android")] metadata:Value,
    zone_id: String,
    track_id: String,
    core_playback_id: Option<String>,
    position: f64,
    dsp: DspConfig,
    impulse:Option<Arc<ImpulseBank>>,
    measured_lufs: Option<f64>,
    transitions: FadeConfig,
    control: Arc<PlaybackControl>,
}
impl Plan {
    fn validate(&self,audio:&Audio)->Result<()> {
        DspProcessor::new(
            self.dsp.clone(),
            audio.metadata.output_rate(),
            audio.metadata.channels as usize,
            self.measured_lufs,
        )?;
        Ok(())
    }
    fn request(&self, audio: &Audio,mode:output::DsdOutput) -> Result<PlaybackRequest> {
        self.validate(audio)?;
        if active_convolution(&self.dsp).is_some()&&self.impulse.is_none(){bail!("卷积响应尚未加载");}
        let decode_dsd=cfg!(target_os="macos")&&audio.metadata.is_dsd()&&mode==output::DsdOutput::Disabled;
        Ok(PlaybackRequest {flac_samples:audio.metadata.flac_samples,dsd_pcm: decode_dsd.then_some(audio.metadata.sample_rate),
            input: audio.proxy.url.clone(),
            position: self.position,
            sample_rate: if decode_dsd {audio.metadata.dsd_pcm_rate()} else {audio.metadata.output_rate_for(mode)},
            channels: audio.metadata.channels,
            source_integer_lossless: audio.metadata.source_integer_lossless,
            source_bit_depth: if decode_dsd {32} else {audio.metadata.bit_depth},
            dsp: self.dsp.clone(),
            impulse:self.impulse.clone(),
            measured_lufs: self.measured_lufs,
            volume: self.control.volume_setting(),
            duration: audio.metadata.duration,
            transitions: self.transitions.clone(),
        })
    }
    fn error(&self, error: impl std::fmt::Display) -> Value {
        json!({"type":"error","zone_id":self.zone_id,"track_id":self.track_id,"core_playback_id":self.core_playback_id,"error":error.to_string()})
    }
}
struct Pending {
    plan: Plan,
    task: Option<JoinHandle<Result<Audio>>>,
    audio: Option<Audio>,
    queue_task: Option<JoinHandle<Result<Preparation>>>,
    queue: Option<Option<sequence::Initial>>,
    queue_refresh:Option<String>,
    group_start_id:Option<String>,
    measured_version:u32,
    impulse_task:Option<JoinHandle<Result<Arc<ImpulseBank>>>>,
    impulse_ready:bool,
}
fn active_convolution(dsp:&DspConfig)->Option<&ConvolutionConfig> {
    dsp.convolution.as_ref().filter(|_|dsp.enabled)
}
impl Pending {
    fn new(mut plan:Plan,task:Option<JoinHandle<Result<Audio>>>,audio:Option<Audio>)->Self {
        if active_convolution(&plan.dsp).is_none(){plan.impulse=None;}
        let impulse_ready=active_convolution(&plan.dsp).is_none()||plan.impulse.is_some();
        Self {plan,task,audio,queue_task:None,queue:None,queue_refresh:None,group_start_id:None,measured_version:1,impulse_task:None,impulse_ready}
    }
    fn set_dsp(&mut self,dsp:DspConfig) {
        if active_convolution(&self.plan.dsp)!=active_convolution(&dsp) {
            if let Some(task)=self.impulse_task.take(){task.abort();}
            self.plan.impulse=None;
            self.impulse_ready=active_convolution(&dsp).is_none();
        }
        self.plan.dsp=dsp;
    }
}
pub(super) struct Running {
    id: String,
    request: PlaybackRequest,
    pub zone_id: String,
    control: Arc<PlaybackControl>,
    thread: Option<std::thread::JoinHandle<()>>,
    audio: Audio,
    track_id: String,
    core_playback_id: Option<String>,
    sequence: Option<sequence::Run>,
}
impl Running {
    pub(crate) fn scheduled(&self) -> bool {
        self.control.start.is_scheduled()
    }

    fn current(&self)->(PlaybackRequest,Audio,Option<String>,String) {
        self.sequence.as_ref().and_then(|run|run.snapshot(None))
            .unwrap_or_else(||(self.request.clone(),self.audio.clone(),self.core_playback_id.clone(),self.track_id.clone()))
    }
    pub fn visualization_lease(&self, milliseconds: u64) { self.control.visualization.renew(milliseconds); }
    pub fn observation(&self,audio:bool)->Value {
        let (_,_,id,track)=self.current();let position=self.position();
        let frames=if audio {self.visualization_lease(600);id.as_ref().map(|id|self.control.visualization.preview(id,position)).unwrap_or_default()}else{vec![]};
        json!({"playback_id":id,"track_id":track,"position":position,"playing":!self.control.paused.load(Ordering::Relaxed),"supported":true,"frames":frames})
    }
    pub fn visualization_report(&self) -> Option<Value> {
        if self.control.paused.load(Ordering::Relaxed) { return None; }
        let (_,_,id,track)=self.current();let id=id?;let position=self.position();
        let frames=self.control.visualization.snapshot(&id,position);
        if frames.is_empty(){return None;}
        Some(json!({"type":"visualization","zone_id":self.zone_id,"core_playback_id":id,"track_id":track,"position":position,"frames":frames,"version":1}))
    }
    pub fn position(&self) -> f64 {
        if let Some(sequence) = &self.sequence { return sequence.position().unwrap_or(self.request.position); }
        self.request.position
            + self.control.seconds(self.request.sample_rate)
    }
    pub(crate) fn buffered(&self, report:&Value) -> (Option<f64>, Option<f64>, bool) {
        let (_, audio, playback, track) = self.current();
        if playback.as_deref()!=report["core_playback_id"].as_str() || report["track_id"]!=track {
            return (None,None,false);
        }
        if report["status"]=="playing" {
            if let Some(position)=report["position"].as_f64(){audio.proxy.advance_playback(position);}
        }
        let rate = self.control.output_sample_rate.load(Ordering::Acquire);
        let ahead = self.control.frames_written.load(Ordering::Acquire)
            .saturating_sub(self.control.frames_played.load(Ordering::Acquire));
        let until = (rate > 0 && ahead > 0).then(|| self.position() + ahead as f64 / rate as f64);
        let until = until.into_iter().chain(audio.proxy.buffered_audio_seconds()).reduce(f64::max);
        (audio.proxy.buffered_fraction(), until, audio.proxy.filling_current())
    }
    pub fn clock(&self, endpoint: &str, elapsed: Duration) -> Value {
        let now=output::clock::host_ns();let device=now.and_then(|now|self.control.device_clock.fresh_sample(now));let programme=device.as_ref().and_then(|sample|self.sequence.as_ref()?.programme_clock(sample));
        json!({"type":"clock","endpoint_id":endpoint,"zone_id":self.zone_id,"position":self.position(),
            "device_clock":device,"programme_clock":programme,"monotonic_ns":elapsed.as_nanos() as u64,"device_clock_ns":self.control.clock_ns.load(Ordering::Relaxed),
            "buffered_frames":self.control.frames_written.load(Ordering::Relaxed).saturating_sub(self.control.frames_played.load(Ordering::Relaxed))})
    }
    async fn stop(mut self)->Result<Option<Value>> {
        self.control.stop();
        if let Some(sequence) = &self.sequence { sequence.cancel(); }
        if let Some(thread) = self.thread.take() {
            tokio::task::spawn_blocking(move || thread.join()).await.context("output join task failed")?
                .map_err(|_|anyhow::anyhow!("output worker panicked"))?;
        }
        self.sequence.as_ref().map(|sequence|sequence.final_report()).transpose().map(Option::flatten)
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.control.stop();
        if let Some(sequence) = &self.sequence { sequence.cancel(); }
    }
}

struct PreparedNext {
    track: String,
    task: JoinHandle<Result<Audio>>,
}
impl Drop for PreparedNext {
    fn drop(&mut self) { self.task.abort(); }
}

#[derive(Default)]
pub(super) struct Playback {
    prepared_next: Option<PreparedNext>,
    running: Option<Running>,
    pending: Option<Pending>,
    // Joining the old output is independent of cancelling a newer source probe.
    retiring: Option<JoinHandle<()>>,
    sequence_config: Option<sequence::Config>,
    listener:Option<(Arc<ListenStore>,ListenOwner)>,
    recovery_enabled:bool,
    recovery:Option<JoinHandle<Result<Option<Restored>>>>,
    recovery_controls:std::collections::HashMap<String,Value>,
    clock_identity:Option<scheduled::Identity>,
    scheduled:Option<scheduled::Ticket>,
    retired_starts:std::collections::VecDeque<(Instant,scheduled::Ticket)>,
    start_generation:u64,
    output_reports:Option<UnboundedSender<Value>>,
}
fn position(value: f64) -> Result<f64> {
    if !value.is_finite() || value < 0. {
        bail!("position must be finite and nonnegative")
    }
    Ok(value)
}
impl Playback {
    pub fn connected(profile:ConnectionProfile, token:String, endpoint:String)->Result<Self> {
        Ok(Self { prepared_next:None, sequence_config:Some(sequence::Config::new(profile,token,endpoint)?),listener:None,recovery_enabled:false,recovery:None,recovery_controls:Default::default(),running:None,pending:None,retiring:None,clock_identity:None,scheduled:None,retired_starts:Default::default(),start_generation:0,output_reports:None })
    }
    pub fn attach_media(&mut self, media:Option<Arc<dyn crate::media_proxy::MediaResolver>>) {
        if let Some(config)=&mut self.sequence_config {config.media=media;}
    }
    pub fn attach_journal(&mut self,store:Arc<ListenStore>,owner:ListenOwner){self.listener=Some((store,owner));}
    pub fn enable_recovery(&mut self,enabled:bool){self.recovery_enabled=enabled;}
    pub fn begin_recovery(&mut self,sync:Arc<ListenSync>) {
        self.begin_recovery_after(sync, Duration::ZERO);
    }
    pub fn begin_recovery_after(&mut self,sync:Arc<ListenSync>,delay:Duration) {
        if !self.recovery_enabled||self.recovery.is_some()||self.running.is_some()||self.pending.is_some(){return;}
        if let (Some(config),Some((store,owner)))=(&self.sequence_config,&self.listener) {
            let config=config.clone();let store=store.clone();let owner=owner.clone();
            self.recovery=Some(tokio::spawn(async move {
                if !delay.is_zero() {tokio::time::sleep(delay).await;}
                rmusic_bridge::sequence::recovery::restore(config,store,owner,sync).await
            }));
        }
    }
    pub fn request_recovery(&mut self,sync:Arc<ListenSync>) {self.cancel_recovery();self.begin_recovery(sync);}
    /// Retain its durable cursor while joining a failed source. A new queued command
    /// owns its own preparation and must not be cancelled by the retired run's report.
    pub async fn finish_source_failure(&mut self,report:&Value) {
        if self.pending.is_none()&&self.accepts(report){self.shutdown().await;}
    }
    fn cancel_recovery(&mut self) {if let Some(task)=self.recovery.take(){task.abort();}self.recovery_controls.clear();}
    pub fn close_cursors(&self) {
        if self.recovery_enabled {if let (Some(config),Some((store,owner)))=(&self.sequence_config,&self.listener) {
            if let Err(error)=rmusic_bridge::listens::cursor::close(store,owner,&config.endpoint){eprintln!("Cursor closure failed: {error:#}");}
        }}
    }
    fn retire_cursor(&self){if let Some(run)=self.running.as_ref().and_then(|r|r.sequence.as_ref()){run.retire_cursor();}}
    pub fn running(&self) -> Option<&Running> {
        self.running.as_ref()
    }
    pub fn accepts(&self, report: &Value) -> bool {
        if report["type"]=="start_state"&&report["final"]==true{return true;}
        if report["type"]=="state"&&self.running.as_ref().is_some_and(|r|r.control.start.is_scheduled()&&r.control.start.snapshot().is_none_or(|s|s.phase!=output::start::Phase::Released)){return false;}
        self.running
            .as_ref()
            .is_some_and(|current| report["playback_id"].as_str() == Some(&current.id))
    }
    fn cancel_pending(&mut self) {
        if let Some(mut pending) = self.pending.take() {
            pending.plan.control.stop();
            if let Some(task) = pending.task.take() {
                task.abort();
            }
            if let Some(task) = pending.queue_task.take() { task.abort(); }
            if let Some(task) = pending.impulse_task.take() { task.abort(); }
        }
    }
    fn retire(&mut self) {
        if let Some(current) = self.running.take() {
            current.control.stop();
            debug_assert!(self.retiring.is_none());
            let final_start=self.final_start(&current.control);let reports=self.output_reports.clone();let control=current.control.clone();
            self.retiring = Some(tokio::spawn(async move{
                match current.stop().await {
                    Ok(sequence)=>if let Some((command,finished,receipt,journal))=final_start {
                        let mut report=scheduled::final_report(&command,&control);
                        for (key,value) in sequence.as_ref().and_then(Value::as_object).into_iter().flatten(){report[key]=value.clone();}
                        if let Some(journal)=journal {let saved=report.clone();if let Err(error)=tokio::task::spawn_blocking(move||journal.capture(&saved)).await.unwrap_or_else(|error|Err(error.into())){eprintln!("Group final persistence failed: {error:#}");}}
                        if let Ok(mut cached)=receipt.lock(){*cached=Some(report.clone());}else{eprintln!("Final start receipt lock poisoned");return;}
                        finished.store(true,Ordering::Release);
                        if let Some(reports)=reports{let _=reports.send(report);}
                    },
                    Err(error)=>eprintln!("Output retirement failed: {error:#}"),
                }
            }));
        }
    }
    pub(crate) fn client_command(&mut self,kind:&str,cmd:&Value,profile:&ConnectionProfile)->Result<()> {
        if kind=="preload" {
            let Some(track)=cmd["next_track"].as_str() else {self.prepared_next=None;return Ok(())};
            if self.running.as_ref().is_some_and(|run|run.current().1.proxy.filling_current()) {
                self.prepared_next=None;
                return Ok(());
            }
            if self.prepared_next.as_ref().is_some_and(|p|p.track==track){return Ok(())}
            let config=self.sequence_config.clone().context("缺少本机播放会话")?;
            let encoded=url::form_urlencoded::byte_serialize(track.as_bytes()).collect::<String>();
            let source=profile.media_url(&format!("api/v1/media/{encoded}"))?.to_string();
            let next=track.to_owned();
            let task=tokio::spawn(async move{
                let (proxy,metadata)=config.prepare_media(&next,&source).await?;
                proxy.prefetch_start().await?;
                Ok(Audio{metadata,proxy})
            });
            self.prepared_next=Some(PreparedNext{track:track.into(),task});
            return Ok(());
        }
        if kind=="stop"{self.prepared_next=None;}
        self.command(kind,cmd,profile)?;
        if let Some(pending)=&mut self.pending { pending.queue=Some(None); }
        Ok(())
    }
    pub fn command(
        &mut self,
        kind: &str,
        command: &Value,
        profile: &ConnectionProfile,
    ) -> Result<()> {
        if std::env::var_os("RMUSIC_AUDIO_DIAGNOSTICS").or_else(||std::env::var_os("RMUSIC_PLAYBACK_DIAGNOSTICS")).is_some() && matches!(kind,"play"|"pause"|"resume"|"cancel_preload") {
            eprintln!("Playback command {kind}: core={} position={} paused={}",command["playback_id"],command["position"],command["paused"]);
        }
        if kind=="group_volume"{return self.group_volume(command);}
        if kind=="group_rate"{return self.group_rate(command);}
        if kind=="commit_start"{return self.commit_start(command);}
        if kind=="cancel_start"{return self.cancel_start(command);}
        let preparing=kind=="prepare_start";
        if preparing&&self.duplicate_start(command)?{self.replay_start_report();return Ok(());}
        if self.cancel_waiting_start(kind,command)?&&!matches!(kind,"stop"|"flush"){return Ok(());}
        let kind=if preparing{"play"}else{kind};
        if self.recovery.is_some()&&matches!(kind,"pause"|"resume"|"volume") {
            if kind=="volume"&&command["volume"].as_f64().is_none_or(|v|!v.is_finite()||!(0. ..=1.).contains(&v)){bail!("volume requires a value in 0..1");}
            let controls=self.recovery_controls.entry(command["zone_id"].as_str().unwrap_or("").into()).or_insert_with(||json!({}));
            if kind=="volume" {controls["volume"]=command["volume"].clone();if let Some(muted)=command.get("muted"){controls["muted"]=muted.clone();}}
            else{controls["paused"]=json!(kind=="pause");}
        }
        match kind {
            "play" => {
                let timing=if preparing {scheduled::timing(command)?}else{match command.get("measured_version"){None=>1,Some(v) if v.as_u64()==Some(1)=>1,Some(v) if v.as_u64()==Some(2)=>2,_=>bail!("invalid restored listening timing version")}};
                if let Some(control) = self.control().filter(|control| !control.stopped.load(Ordering::Acquire)) {
                    let same = self.pending.as_ref().is_some_and(|p| p.plan.core_playback_id.as_deref().is_some_and(|id| command["playback_id"] == id))
                        || self.running.as_ref().is_some_and(|r| r.sequence.as_ref().is_some_and(|run|run.owns_control(command["playback_id"].as_str()))
                            || (r.thread.as_ref().is_some_and(|t|!t.is_finished()) && r.current().2.as_deref().is_some_and(|id| command["playback_id"] == id)));
                    if same&&!preparing&&command["reload_core_dsp"]!=true&&!control.start.is_scheduled() { control.set_paused(false); return Ok(()); }
                }
                let input = command["url"]
                    .as_str()
                    .context("play requires ticket media URL")?;
                let input = profile.media_url(input)?.to_string();
                let rendered=MediaProxy::core_pcm(&input).is_some();
                let control = Arc::new(PlaybackControl::default());
                control.set_volume(command["volume"].as_f64().unwrap_or(1.))?;
                control.muted.store(
                    command["muted"].as_bool().unwrap_or(false),
                    Ordering::Relaxed,
                );
                control.paused.store(
                    command["paused"].as_bool().unwrap_or(false),
                    Ordering::Relaxed,
                );
                let transitions: FadeConfig = serde_json::from_value(
                    command
                        .get("transitions")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                )?;
                transitions.validate()?;
                #[cfg(any(windows,target_os="linux"))] output::power::watch(&control)?;
                let dsp = if rendered { DspConfig::default() } else {
                    serde_json::from_value(command.get("dsp").cloned().unwrap_or_else(|| json!({})))?
                };
                let plan = Plan {
                    #[cfg(target_os="android")] metadata:command["track"].clone(),
                    zone_id: command["zone_id"].as_str().unwrap_or("").to_owned(),
                    track_id: command["track"]["id"].as_str().unwrap_or("").to_owned(),
                    core_playback_id: command["playback_id"].as_str().map(String::from),
                    position: position(command["position"].as_f64().unwrap_or(0.))?,
                    measured_lufs: measured_loudness(&dsp, &command["track"]),
                    dsp,
                    impulse:None,
                    transitions: if rendered { FadeConfig::default() } else { transitions },
                    control,
                };
                if let Some(selection)=&plan.dsp.convolution{selection.validate()?;}
                if preparing{self.arm_start(command,&plan.control)?;}else{self.forget_start();}
                self.cancel_recovery();self.retire_cursor();if preparing{self.close_cursors();}
                if let Some(current)=&self.running {if let Some(run)=&current.sequence{run.mark_replacement(current.current().3==plan.track_id);}}
                self.cancel_pending();
                self.retire();
                let profile = profile.clone();
                let media = self.sequence_config.clone();
                let track = plan.track_id.clone();
                let prepared=self.prepared_next.take().filter(|p|p.track==track&&command["reload_core_dsp"]!=true);
                let task = tokio::spawn(async move {
                    if let Some(mut prepared)=prepared {
                        if let Ok(Ok(audio))=(&mut prepared.task).await {return Ok(audio);}
                    }
                    let (proxy,metadata) = if let Some(config)=media {
                        config.prepare_media(&track,&input).await?
                    } else {
                        let proxy=Arc::new(MediaProxy::start(&profile,&input,None).await?);
                        let metadata=rmusic_audio::probe::probe_async(std::path::Path::new(&proxy.url)).await?;
                        (proxy,rmusic_bridge::sequence::core_rendered_metadata(&input,metadata))
                    };
                    Ok(Audio { metadata, proxy })
                });
                self.pending = Some(Pending::new(plan,Some(task),None));
                self.pending.as_mut().unwrap().measured_version=timing;
                if preparing{let pending=self.pending.as_mut().unwrap();
                    if command["group_queue"]==true{pending.group_start_id=command["start"]["plan_id"].as_str().map(String::from);}
                    else{pending.queue=Some(None);}
                }
            }
            "pause" | "resume" => {
                if let Some(control) = self.control() {
                    control.set_paused(kind == "pause");
                }
            }
            "loudness" => {
                let (Some(id), Some(measured)) = (command["playback_id"].as_str(), command["integrated_lufs"].as_f64()) else { return Ok(()); };
                let peak = command["true_peak_dbfs"].as_f64();
                if let Some(pending) = &mut self.pending {
                    if pending.plan.core_playback_id.as_deref() == Some(id) {
                        pending.plan.measured_lufs = pending.plan.dsp.peak_limited_measurement(Some(measured), peak);
                        pending.plan.control.set_loudness("single",measured,peak)?;
                    }
                    pending.plan.control.set_loudness(id,measured,peak)?;
                }
                if let Some(run) = &self.running {
                    if let Some(sequence) = &run.sequence {
                        if sequence.owns_control(Some(id)) { run.control.set_loudness(id,measured,peak)?; }
                    } else if run.core_playback_id.as_deref() == Some(id) {
                        run.control.set_loudness("single",measured,peak)?;
                    }
                }
            }
            "volume" => {
                if let Some(control) = self.control() {
                    control.set_volume(
                        command["volume"]
                            .as_f64()
                            .context("volume requires value")?,
                    )?;
                    if let Some(muted) = command["muted"].as_bool() {
                        control.muted.store(muted, Ordering::Relaxed);
                    }
                }
            }
            "stop" | "flush" => {
                if kind=="stop"{if let Some(run)=self.running.as_ref().and_then(|r|r.sequence.as_ref()){run.mark_stop();}}
                self.cancel_recovery();self.close_cursors();
                self.cancel_pending();
                self.retire();
            }
            "seek" | "dsp" => {
                if command["core_pcm_dsp"]==true && command["url"].is_string() {
                    let mut restart=command.clone();
                    restart["reload_core_dsp"]=json!(true);
                    if kind=="dsp" {
                        if let Some(run)=self.running.as_ref().filter(|r|r.current().2.as_deref()==command["playback_id"].as_str()) {
                            restart["position"]=json!(run.position());
                            restart["paused"]=json!(run.control.paused.load(Ordering::Acquire));
                        }
                    }
                    return self.command("play",&restart,profile);
                }
                let target = if kind == "seek" {
                    Some(position(
                        command["position"]
                            .as_f64()
                            .context("seek requires position")?,
                    )?)
                } else {
                    None
                };
                let dsp: Option<DspConfig> = if kind == "dsp" {
                    Some(serde_json::from_value(command["dsp"].clone())?)
                } else {
                    None
                };
                if let Some(selection)=dsp.as_ref().and_then(|dsp|dsp.convolution.as_ref()){selection.validate()?;}
                if self.running.is_none()&&self.pending.is_none(){if let Some(dsp)=&dsp{DspProcessor::new(dsp.clone(),768000,2,None)?;}}
                if self.running.is_none()&&self.pending.is_none()&&command["url"].is_string(){
                    let result=self.command("play",command,profile);if result.is_ok(){self.close_cursors();}return result;
                }
                if let Some(pending) = &mut self.pending {
                    if let Some(task) = pending.queue_task.take() { task.abort(); }
                    pending.queue = None;
                    if let (Some(dsp), Some(audio)) = (&dsp, &pending.audio) {
                        DspProcessor::new(
                            dsp.clone(),
                            audio.metadata.output_rate(),
                            audio.metadata.channels as usize,
                            pending.plan.measured_lufs,
                        )?;
                    }
                    if let Some(target) = target {
                        pending.plan.position = target;
                        pending.plan.core_playback_id=command["playback_id"].as_str().map(String::from);
                    }
                    if let Some(dsp) = dsp {
                        pending.set_dsp(dsp);
                    }
                } else if let Some(current) = &self.running {
                    let (request,audio,core_playback_id,track_id) = current.sequence.as_ref().and_then(|run|run.for_command(command)).unwrap_or_else(||current.current());
                    let current_position=if core_playback_id==current.current().2 {current.position()}
                        else {command["position"].as_f64().unwrap_or(request.position)};
                    let control = Arc::new(PlaybackControl::default());
                    control.set_volume(current.control.volume_setting())?;
                    control.muted.store(
                        current.control.muted.load(Ordering::Relaxed),
                        Ordering::Relaxed,
                    );
                    control.paused.store(
                        current.control.paused.load(Ordering::Relaxed),
                        Ordering::Relaxed,
                    );
                    let dsp=dsp.unwrap_or_else(||request.dsp.clone());
                    #[cfg(any(windows,target_os="linux"))] output::power::watch(&control)?;
                    let impulse=if active_convolution(&dsp)==active_convolution(&request.dsp){request.impulse.clone()}else{None};
                    let plan = Plan {
                        #[cfg(target_os="android")] metadata:command["track"].clone(),
                        zone_id: current.zone_id.clone(),
                        track_id,
                        core_playback_id: if kind=="seek" {command["playback_id"].as_str().map(String::from)}else{core_playback_id},
                        position: target.unwrap_or(current_position),
                        dsp,
                        impulse,
                        measured_lufs: request.measured_lufs,
                        transitions: request.transitions.clone(),
                        control,
                    };
                    plan.validate(&audio)?;
                    self.retire();
                    self.pending = Some(Pending::new(plan,None,Some(audio)));
                }
                self.cancel_recovery();self.close_cursors();
            }
            "cancel_preload" => {
                if let Some(pending)=&mut self.pending {
                    if command["preserve_current"]==true&&command["zone"]["playback_id"].as_str()==pending.plan.core_playback_id.as_deref() {
                        if let Some(task)=pending.queue_task.take(){task.abort();}
                        pending.queue=None;pending.queue_refresh=command["plan_id"].as_str().map(String::from);
                    } else if pending.queue.as_ref().and_then(|q|q.as_ref()).is_some_and(|q|command["plan_id"]==q.plan_id) {
                        if let Some(task)=pending.queue_task.take(){task.abort();} pending.queue=None;
                    }
                }
                if let Some(current) = &self.running {
                    if current.sequence.as_ref().is_some_and(|run|run.matches(command["plan_id"].as_str())) {
                        if command["preserve_current"]==true&&current.sequence.as_ref().unwrap().owns_control(command["zone"]["playback_id"].as_str()) {
                            current.sequence.as_ref().unwrap().refresh_queue()?;
                            return Ok(());
                        }
                        current.sequence.as_ref().unwrap().retire_cursor();
                        let zone = &command["zone"];
                        if zone["endpoint_id"] != self.sequence_config.as_ref().map(|config|config.endpoint.as_str()).unwrap_or("")
                            || !matches!(zone["status"].as_str(),Some("playing"|"paused")) {
                            self.cancel_pending(); self.retire(); return Ok(());
                        }
                        let Some((request,audio,core_playback_id,track_id)) = current.sequence.as_ref().unwrap().snapshot(zone["playback_id"].as_str()) else {
                            self.cancel_pending(); self.retire(); return Ok(());
                        };
                        let heard = current.sequence.as_ref().unwrap().current_id();
                        let target = if heard.as_deref() == core_playback_id.as_deref() { current.position() }
                            else { zone["position"].as_f64().unwrap_or(0.) };
                        let control = Arc::new(PlaybackControl::default());
                        control.set_volume(current.control.volume_setting())?;
                        control.muted.store(current.control.muted.load(Ordering::Relaxed),Ordering::Relaxed);
                        control.set_paused(current.control.paused.load(Ordering::Relaxed));
                        #[cfg(any(windows,target_os="linux"))] output::power::watch(&control)?;
                        let transitions:FadeConfig=serde_json::from_value(zone["transitions"].clone())?;
                        transitions.validate()?;
                        let dsp:DspConfig=serde_json::from_value(zone["dsp"].clone())?;
                        let impulse=if active_convolution(&dsp)==active_convolution(&request.dsp){request.impulse}else{None};
                        let plan = Plan { #[cfg(target_os="android")] metadata:command["track"].clone(),zone_id:current.zone_id.clone(),track_id,core_playback_id,position:target,
                            dsp,impulse,measured_lufs:zone["integrated_lufs"].as_f64(),transitions,control };
                        self.retire();
                        self.pending = Some(Pending::new(plan,None,Some(audio)));
                    }
                }
            }
            "registered" | "ping" | "clock" | "hello" => {}
            _ => bail!("unsupported bridge command: {kind}"),
        }
        Ok(())
    }
    pub fn volume_limits(&self,limits:&output::OutputLimits)->Result<()> {if let Some(control)=self.control(){control.configure_limits(limits)?;}Ok(())}
    fn control(&self) -> Option<&Arc<PlaybackControl>> {
        self.pending
            .as_ref()
            .map(|p| &p.plan.control)
            .or_else(|| self.running.as_ref().map(|r| &r.control))
    }
    /// Preparation and output teardown wait here; the socket loop stays available.
    pub async fn update(&mut self) -> Option<Value> {
        if let Some(pending) = &mut self.pending {
            if !pending.impulse_ready&&pending.impulse_task.is_none() {
                let Some(config)=self.sequence_config.clone() else {
                    let report=pending.plan.error("加载卷积响应需要已认证的核心连接");self.cancel_pending();return Some(report);
                };
                let zone=pending.plan.zone_id.clone();let selection=active_convolution(&pending.plan.dsp).unwrap().clone();
                pending.impulse_task=Some(tokio::spawn(async move{config.load_impulses(&zone,&selection).await}));
            }
            if pending.audio.is_some() && pending.queue.is_none() && pending.queue_task.is_none() {
                if let (Some(config),Some(id)) = (&self.sequence_config,&pending.plan.core_playback_id) {
                    let config=config.clone();let zone=pending.plan.zone_id.clone();let track=pending.plan.track_id.clone();let id=id.clone();let version=pending.measured_version;
                    let refresh=pending.queue_refresh.take();
                    pending.queue_task=Some(tokio::spawn(async move {
                        if let Some(plan)=refresh {config.refresh_queue_timed(&zone,&track,&id,version,&plan,&[]).await}
                        else {config.negotiate_timed(&zone,&track,&id,version).await}
                    }));
                } else { pending.queue=Some(None); }
            }
        }
        let (probe,queue,impulse)=match self.pending.as_mut() {Some(pending)=>(pending.task.as_mut(),pending.queue_task.as_mut(),pending.impulse_task.as_mut()),None=>(None,None,None)};
        let has_probe=probe.is_some();let has_queue=queue.is_some();let has_impulse=impulse.is_some();
        let has_recovery=self.recovery.is_some();
        let group_complete=self.running.as_mut().and_then(|run|run.sequence.as_mut()).and_then(|run|run.group_completion());let has_group_complete=group_complete.is_some();
        tokio::select! {
            _=async {let _=group_complete.unwrap().await;},if has_group_complete=>{self.retire();None},
            result=async {self.recovery.as_mut().unwrap().await},if has_recovery=>{
                self.recovery=None;
                match result.context("recovery task failed").and_then(|r|r) {
                    Ok(Some(mut restored))=>{
                        if let Some(controls)=self.recovery_controls.remove(restored.command["zone_id"].as_str().unwrap_or("")) {
                            for (key,value) in controls.as_object().into_iter().flatten(){restored.command[key]=value.clone();}
                        }
                        let failure=json!({"type":"error","zone_id":restored.command["zone_id"],"track_id":restored.command["track"]["id"],"core_playback_id":restored.command["playback_id"]});
                        match self.command("play",&restored.command,&restored.profile) {
                            Ok(())=>{eprintln!("Durable queue recovered at {:.6}s",restored.command["position"].as_f64().unwrap_or(0.));None},
                            Err(error)=>{let _=restored.handle.retire();let mut report=failure;report["error"]=json!(error.to_string());Some(report)}
                        }
                    },
                    Ok(None)=>{self.recovery_controls.clear();None},
                    Err(error)=>{eprintln!("Durable queue recovery deferred: {error:#}");self.recovery_controls.clear();
                        if let Some(failure)=error.downcast_ref::<rmusic_bridge::sequence::recovery::BoundFailure>(){if failure.report["device_failed"]!=true{let _=failure.handle.retire();}Some(failure.report.clone())}else{None}}
                }
            },
            _ = async {self.retiring.as_mut().unwrap().await}, if self.retiring.is_some() => {self.retiring = None; None},
            result = async {probe.unwrap().await}, if has_probe => {
                    let pending = self.pending.as_mut().unwrap(); pending.task = None;
                    match result.context("audio probe task failed").and_then(|result|result) {
                        Ok(audio) => {
                            if std::env::var_os("RMUSIC_AUDIO_DIAGNOSTICS").or_else(||std::env::var_os("RMUSIC_PLAYBACK_DIAGNOSTICS")).is_some(){eprintln!("Playback source ready: core={} rate={} channels={}",pending.plan.core_playback_id.as_deref().unwrap_or(""),audio.metadata.sample_rate,audio.metadata.channels);}
                            pending.audio = Some(audio); None
                        },
                        Err(error) => {
                            let network=error.downcast_ref::<reqwest::Error>().is_some_and(|e|e.is_connect()||e.is_timeout()||e.is_body()||e.status().is_some_and(|s|s.is_server_error()));
                            let unavailable = error.is::<rmusic_bridge::sequence::UnavailableProgramme>();
                            let mut report=pending.plan.error(error);
                            report["source_failed"]=json!(network);
                            if unavailable { report["source_unavailable"] = json!(true); }
                            report["position"]=json!(pending.plan.position);
                            self.cancel_pending(); Some(report)
                        },
                    }
                },
            result = async {queue.unwrap().await}, if has_queue => {
                    let pending=self.pending.as_mut().unwrap();pending.queue_task=None;
                    match result.context("queue preparation task failed").and_then(|r|r) {
                        Ok(Preparation::Ready(queue))=>{pending.queue=Some(Some(queue));None},
                        Ok(Preparation::Unsupported)=>{pending.queue=Some(None);None},
                        // A newer Core command or stop superseded this preparation. Never open
                        // obsolete audio or report a cancellation as a failure of the current run.
                        Ok(Preparation::Stale(zone))=>{if std::env::var_os("RMUSIC_AUDIO_DIAGNOSTICS").or_else(||std::env::var_os("RMUSIC_PLAYBACK_DIAGNOSTICS")).is_some(){eprintln!("Playback preparation superseded: requested={} current={} status={}",pending.plan.core_playback_id.as_deref().unwrap_or(""),zone["playback_id"],zone["status"]);}self.cancel_pending();None},
                        Err(error)=>{let report=pending.plan.error(error);self.cancel_pending();Some(report)}
                    }
                },
            result=async {impulse.unwrap().await},if has_impulse=>{
                let pending=self.pending.as_mut().unwrap();pending.impulse_task=None;
                match result.context("impulse loading task failed").and_then(|r|r) {
                    Ok(bank)=>{pending.plan.impulse=Some(bank);pending.impulse_ready=true;None},
                    Err(error)=>{let report=pending.plan.error(error);self.cancel_pending();Some(report)}
                }
            },
            else => std::future::pending().await,
        }
    }
    fn take_ready(&mut self) -> Option<Pending> {
        if self.retiring.is_none() && self.pending.as_ref().is_some_and(|p| p.audio.is_some() && p.impulse_ready && (self.sequence_config.is_none() || p.queue.is_some())) {
            self.pending.take()
        } else {
            None
        }
    }
    pub fn start_ready(
        &mut self,
        output: &OutputConfig,
        reports: &UnboundedSender<Value>,
    ) -> Option<Value> {
        self.output_reports=Some(reports.clone());
        let pending = self.take_ready()?;
        if std::env::var_os("RMUSIC_AUDIO_DIAGNOSTICS").or_else(||std::env::var_os("RMUSIC_PLAYBACK_DIAGNOSTICS")).is_some(){eprintln!("Playback output ready: core={} paused={}",pending.plan.core_playback_id.as_deref().unwrap_or(""),pending.plan.control.paused.load(Ordering::Acquire));}
        let prepared_group=pending.queue.as_ref().and_then(|queue|queue.as_ref()).and_then(|initial|initial.group_start_id.as_ref());
        if pending.queue.as_ref().and_then(|queue|queue.as_ref()).is_some_and(|initial|initial.measured_version!=pending.measured_version)||prepared_group!=pending.group_start_id.as_ref(){pending.plan.control.stop();return Some(pending.plan.error("分组队列准备身份不符或核心不支持连续组输出"));}
        let audio = pending.audio.unwrap();
        let request = match pending.plan.request(&audio,output.dsd_output) {
            Ok(request) => request,
            Err(error) => {pending.plan.control.stop();return Some(pending.plan.error(error));},
        };
        if pending.plan.control.start.is_scheduled()&&request.is_dsd(){pending.plan.control.stop();return Some(pending.plan.error("计划起播当前仅支持 PCM"));}
        let control=pending.plan.control.clone();
        if let (Some(config),Some(Some(initial))) = (&self.sequence_config,pending.queue) {
            let mut failure=pending.plan.error("");
            let adjusted=self.rate_adjustment_requested(&control);
            match sequence::spawn(config.clone(),request,pending.plan,audio,output.clone(),reports.clone(),initial,self.listener.clone(),self.recovery_enabled,adjusted) {
                Ok(running)=>{self.starting_output(&control);self.running=Some(running);},
                Err(error)=>{control.stop();failure["error"]=json!(error.to_string());return Some(failure);},
            }
            return None;
        }
        self.starting_output(&control);
        self.running = Some(spawn_playback(
            request,
            pending.plan,
            audio,
            output.clone(),
            reports.clone(),
        ));
        None
    }
    pub async fn shutdown(&mut self) {
        self.prepared_next=None;
        self.cancel_recovery();
        self.cancel_pending();
        self.retire();
        if let Some(retiring) = self.retiring.take() {
            let _ = retiring.await;
        }
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.cancel_recovery();
        self.cancel_pending();
    }
}
fn spawn_playback(
    request: PlaybackRequest,
    plan: Plan,
    audio: Audio,
    output: OutputConfig,
    reports: UnboundedSender<Value>,
) -> Running {
    let id = uuid::Uuid::new_v4().to_string();
    let worker_id = id.clone();
    let worker_control = plan.control.clone();
    let worker_request = request.clone();
    let worker_zone = plan.zone_id.clone();
    let worker_track = plan.track_id.clone();
    let core_playback_id=plan.core_playback_id.clone();
    let start = request.position;
    let source_rate = request.sample_rate;
    let duration=request.duration;let worker_proxy=audio.proxy.clone();
    #[cfg(target_os="linux")] let sleep_checkpoint=output::power::checkpoint(&worker_control);
    let thread = std::thread::spawn(move || {
        #[cfg(target_os="linux")] let _sleep_checkpoint=sleep_checkpoint;
        let mut last_report = Instant::now() - Duration::from_secs(1);
        let mut emit = |mut path:output::SignalPath, meter, continuity_lost| {
                worker_proxy.annotate(&mut path);
                if last_report.elapsed() >= Duration::from_secs(1)||continuity_lost {
                    let position = start + worker_control.seconds(source_rate);
                    let mut report=json!({"type":"state","playback_id":worker_id,"core_playback_id":core_playback_id,"zone_id":worker_zone,"track_id":worker_track,
                    "status":if worker_control.paused.load(Ordering::Relaxed){"paused"}else{"playing"},"position":position,"duration":duration,"signal_path":path,"meter":meter});
                    if worker_control.start.is_scheduled(){report["continuity_lost"]=json!(continuity_lost);}
                    let _=reports.send(start_frames(&worker_control,source_rate,report));
                    last_report = Instant::now();
                }
            };
        let scheduled = worker_control.start.is_scheduled();
        let result=if scheduled || cfg!(target_os = "macos") {
            let format=output::SequenceFormat{sample_rate:worker_request.sample_rate,channels:worker_request.channels,channel_mask:0,exclusive:output.exclusive,float_output:worker_request.dsp.enabled,bit_depth:32};
            output::PlaybackSequence::new(core_playback_id.clone().unwrap_or_else(||worker_id.clone()),worker_request.clone(),format)
                .and_then(|sequence|output::play_sequence(worker_request,output,worker_control.clone(),sequence,|event|emit(event.signal_path,event.meter,scheduled && event.continuity_lost)))
        }else{output::play(worker_request,output,worker_control.clone(),|path,meter|emit(path,meter,false))};
        if !worker_control.stopped.load(Ordering::Relaxed)||worker_control.system_interrupted() {
            let position = start + worker_control.seconds(source_rate);
            let result=worker_proxy.finish_playback(result,position,duration);
            let report = match result {
                Ok(()) => {
                    json!({"type":"state","playback_id":worker_id,"core_playback_id":core_playback_id,"zone_id":worker_zone,"track_id":worker_track,"status":"ended","position":position})
                }
                Err(error) => {
                    json!({"type":"error","playback_id":worker_id,"core_playback_id":core_playback_id,"zone_id":worker_zone,"track_id":worker_track,"position":position,"source_failed":worker_proxy.source_failed(),"error":error.to_string()})
                }
            };
            let _ = reports.send(start_frames(&worker_control,source_rate,report));
        }
    });
    Running {
        id,
        request,
        zone_id: plan.zone_id,
        track_id: plan.track_id,
        core_playback_id: plan.core_playback_id,
        sequence: None,
        control: plan.control,
        thread: Some(thread),
        audio,
    }
}

fn start_frames(control:&PlaybackControl,fallback:u32,mut report:Value)->Value {
    if control.start.is_scheduled(){report["frames"]=json!(control.frames_played.load(Ordering::Acquire).min(control.frames_written.load(Ordering::Acquire)).min(control.programme_end.load(Ordering::Acquire)));let rate=control.output_sample_rate.load(Ordering::Acquire);report["sample_rate"]=json!(if rate==0{fallback}else{rate});}report
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    pub(super) fn profile(address: String) -> ConnectionProfile {
        ConnectionProfile {core_public_key:None,
            id: "test".into(),
            name: "test".into(),
            address,
            skip_certificate_verification: false, certificate_sha256: None,
            alternatives: vec![],
            core_id: None,
        }
    }
    pub(super) fn wav() -> Vec<u8> {
        let size = 48000u32 * 2;
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend((36 + size).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(size.to_le_bytes());
        bytes.resize(44 + size as usize, 0);
        bytes
    }
    pub(super) async fn server(router: Router) -> (ConnectionProfile, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let profile = profile(format!("http://{}", listener.local_addr().unwrap()));
        (
            profile,
            tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            }),
        )
    }
    pub(super) fn play(profile: &ConnectionProfile, track: &str, source: &str) -> Value {
        json!({"type":"play","playback_id":format!("core-{track}"),"url":format!("{}/api/v1/media/{source}",profile.address),"zone_id":"z","track":{"id":track},"volume":0.8,"muted":true})
    }
    async fn prepared(playback: &mut Playback) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while playback.pending.as_ref().unwrap().audio.is_none() {
                assert!(playback.update().await.is_none());
            }
        })
        .await
        .unwrap();
    }
    pub(super) async fn running(profile: &ConnectionProfile) -> Running {
        let proxy = Arc::new(
            MediaProxy::start(
                profile,
                &format!("{}/api/v1/media/test", profile.address),
                None,
            )
            .await
            .unwrap(),
        );
        let control = Arc::new(PlaybackControl::default());
        control.set_volume(0.3).unwrap();
        control.muted.store(true, Ordering::Relaxed);
        control.set_paused(true);
        let audio = Audio {
            proxy,
            metadata: AudioMetadata {
                flac_samples:None,
                duration: Some(20.),
                format: "flac".into(),
                sample_rate: 48000,
                channels: 2,
                bit_depth: 24,
                bitrate: 0,
                quality_rank: 0,
                source_integer_lossless: true,
            },
        };
        let plan = Plan {
            zone_id: "z".into(),
            track_id: "same".into(),
            core_playback_id:Some("core-old".into()),
            position: 0.,
            dsp: Default::default(),
            impulse:None,
            measured_lufs: None,
            transitions: Default::default(),
            control,
        };
        Running {
            id: "old".into(),
            request: plan.request(&audio,output::DsdOutput::Disabled).unwrap(),
            zone_id: plan.zone_id,
            track_id: plan.track_id,
            core_playback_id:plan.core_playback_id,
            sequence: None,
            control: plan.control,
            thread: None,
            audio,
        }
    }
    #[tokio::test]
    async fn idle_seek_validates_before_cancelling_durable_recovery() {
        use rmusic_bridge::listens::cursor::{Boundary,Recorder};
        let dir=tempfile::tempdir().unwrap();
        let store=ListenStore::open(dir.path()).unwrap();
        let owner=ListenOwner {core_id:"core".into(),user_id:"user".into()};
        let recorder=Recorder::begin(store.clone(),owner.clone(),"endpoint".into(),"z".into(),"old-plan".into(),48000,
            Boundary {playback_id:"old-source".into(),track_id:"old-track".into(),position:0.25,frames:12000}).unwrap();
        assert!(recorder.handle.ready().unwrap());
        let profile=profile("http://127.0.0.1:9".into());
        let mut playback=Playback::connected(profile.clone(),"token".into(),"endpoint".into()).unwrap();
        playback.attach_journal(store,owner);playback.enable_recovery(true);
        playback.recovery=Some(tokio::spawn(std::future::pending()));
        for (kind,command) in [
            ("seek",json!({"position":-1.})),
            ("seek",json!({"position":0.5,"url":"http://foreign.invalid/api/v1/media/a"})),
            ("dsp",json!({"dsp":{"preamp_db":100.}})),
        ] {
            assert!(playback.command(kind,&command,&profile).is_err());
            assert!(playback.recovery.is_some());
            assert!(recorder.handle.ready().unwrap());
            assert!(playback.pending.is_none());
        }
        let mut command=play(&profile,"seek","a");
        command["position"]=json!(0.5);command["paused"]=json!(true);
        playback.command("seek",&command,&profile).unwrap();
        let pending=playback.pending.as_ref().unwrap();
        assert_eq!(pending.plan.position,0.5);
        assert_eq!(pending.plan.core_playback_id.as_deref(),Some("core-seek"));
        assert!(pending.plan.control.paused.load(Ordering::Relaxed));
        assert!(playback.recovery.is_none());
        assert!(!recorder.handle.ready().unwrap());
        playback.shutdown().await;
    }
    #[tokio::test]
    async fn seek_keeps_mute_and_pause_before_the_replacement_starts() {
        let profile = profile("http://127.0.0.1:9".into());
        let mut playback = Playback {
            prepared_next:None,
            clock_identity:None,scheduled:None,retired_starts:Default::default(),start_generation:0,output_reports:None,
            running: Some(running(&profile).await),
            pending: None, retiring: None, sequence_config: None, listener:None,
            recovery:None,recovery_controls:Default::default(),recovery_enabled:false,
        };
        playback
            .command("seek", &json!({"position":1.,"playback_id":"core-seek"}), &profile)
            .unwrap();
        assert!(
            playback.take_ready().is_none(),
            "old output must finish before admission"
        );
        assert!(playback.update().await.is_none());
        let pending = playback.take_ready().unwrap();
        assert_eq!(pending.plan.core_playback_id.as_deref(),Some("core-seek"));
        assert!(pending.plan.control.muted.load(Ordering::Relaxed));
        assert!(pending.plan.control.paused.load(Ordering::Relaxed));
        let request = pending
            .plan
            .request(pending.audio.as_ref().unwrap(),output::DsdOutput::Disabled)
            .unwrap();
        assert_eq!(request.position, 1.);
        assert_eq!(request.volume, 0.3);
    }
    #[tokio::test]
    async fn delayed_real_probe_uses_the_latest_pause_volume_seek_and_dsp_commands() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let e = entered.clone();
        let r = release.clone();
        let (profile, server) = server(Router::new().route(
            "/api/v1/media/slow",
            get(move || {
                let e = e.clone();
                let r = r.clone();
                async move {
                    e.notify_one();
                    r.notified().await;
                    wav()
                }
            }),
        ))
        .await;
        let mut playback = Playback::default();
        playback
            .command("play", &play(&profile, "slow", "slow"), &profile)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        playback.command("pause", &json!({}), &profile).unwrap();
        playback
            .command("volume", &json!({"volume":0.2,"muted":false}), &profile)
            .unwrap();
        playback
            .command("seek", &json!({"position":0.25,"playback_id":"core-seek"}), &profile)
            .unwrap();
        playback
            .command(
                "dsp",
                &json!({"dsp":{"enabled":true,"preamp_db":-6.}}),
                &profile,
            )
            .unwrap();
        release.notify_one();
        prepared(&mut playback).await;
        let pending = playback.take_ready().unwrap();
        let request = pending
            .plan
            .request(pending.audio.as_ref().unwrap(),output::DsdOutput::Disabled)
            .unwrap();
        assert_eq!(request.sample_rate, 48000);
        assert_eq!(request.channels, 1);
        assert_eq!(request.position, 0.25);
        assert_eq!(pending.plan.core_playback_id.as_deref(),Some("core-seek"));
        assert_eq!(request.volume, 0.2);
        assert!(request.dsp.enabled);
        assert_eq!(request.dsp.preamp_db, -6.);
        assert!(pending.plan.control.paused.load(Ordering::Relaxed));
        assert!(!pending.plan.control.muted.load(Ordering::Relaxed));
        server.abort();
    }
    #[tokio::test]
    async fn replacing_a_delayed_probe_cannot_admit_its_stale_result() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let e = entered.clone();
        let r = release.clone();
        let (profile, server) = server(
            Router::new()
                .route("/api/v1/media/fast", get(|| async { wav() }))
                .route(
                    "/api/v1/media/slow",
                    get(move || {
                        let e = e.clone();
                        let r = r.clone();
                        async move {
                            e.notify_one();
                            r.notified().await;
                            wav()
                        }
                    }),
                ),
        )
        .await;
        let mut playback = Playback::default();
        playback
            .command("play", &play(&profile, "old", "slow"), &profile)
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        let old_control = playback.control().unwrap().clone();
        let old_task = playback
            .pending
            .as_ref()
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .abort_handle();
        playback
            .command("play", &play(&profile, "new", "fast"), &profile)
            .unwrap();
        release.notify_one();
        prepared(&mut playback).await;
        assert!(old_control.stopped.load(Ordering::Relaxed));
        assert!(old_task.is_finished());
        let pending = playback.take_ready().unwrap();
        assert_eq!(pending.plan.track_id, "new");
        playback.shutdown().await;
        server.abort();
    }
    #[tokio::test]
    async fn cancelling_a_new_source_does_not_bypass_the_old_output_join() {
        let (profile, server) =
            server(Router::new().route("/api/v1/media/fast", get(|| async { wav() }))).await;
        let mut old = running(&profile).await;
        let old_control = old.control.clone();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        old.thread = Some(std::thread::spawn(move || {
            let _ = hold.recv();
        }));
        let mut playback = Playback {
            prepared_next:None,
            clock_identity:None,scheduled:None,retired_starts:Default::default(),start_generation:0,output_reports:None,
            running: Some(old),
            pending: None, retiring: None, sequence_config: None, listener:None,
            recovery:None,recovery_controls:Default::default(),recovery_enabled:false,
        };
        assert!(playback.accepts(&json!({"playback_id":"old","track_id":"same"})));
        playback
            .command("play", &play(&profile, "same", "fast"), &profile)
            .unwrap();
        prepared(&mut playback).await;
        assert!(old_control.stopped.load(Ordering::Relaxed));
        assert!(playback.take_ready().is_none());
        playback.command("stop", &json!({}), &profile).unwrap();
        playback
            .command("play", &play(&profile, "same", "fast"), &profile)
            .unwrap();
        prepared(&mut playback).await;
        assert!(
            playback.take_ready().is_none(),
            "cancel/replay must keep the existing teardown barrier"
        );
        assert!(
            !playback.accepts(&json!({"playback_id":"old","track_id":"same"})),
            "same-track reports from a retired worker are stale"
        );
        playback.command("pause", &json!({}), &profile).unwrap();
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), playback.update())
            .await
            .unwrap();
        let pending = playback.take_ready().unwrap();
        assert!(pending.plan.control.paused.load(Ordering::Relaxed));
        playback.shutdown().await;
        server.abort();
    }
    #[tokio::test]
    async fn invalid_controls_do_not_stop_or_modify_the_current_output() {
        let profile = profile("http://127.0.0.1:9".into());
        let mut playback = Playback {
            prepared_next:None,
            clock_identity:None,scheduled:None,retired_starts:Default::default(),start_generation:0,output_reports:None,
            running: Some(running(&profile).await),
            pending: None, retiring: None, sequence_config: None, listener:None,
            recovery:None,recovery_controls:Default::default(),recovery_enabled:false,
        };
        for (kind, command) in [
            ("seek", json!({"position":-1.})),
            ("volume", json!({"volume":2.,"muted":false})),
            ("dsp", json!({"dsp":{"preamp_db":100.}})),
            (
                "play",
                json!({"url":"https://other.invalid/audio","track":{"id":"bad"}}),
            ),
            (
                "play",
                json!({"url":format!("{}/api/v1/media/test",profile.address),"position":-1.}),
            ),
        ] {
            assert!(playback.command(kind, &command, &profile).is_err());
            let current = playback.running().unwrap();
            assert!(!current.control.stopped.load(Ordering::Relaxed));
            assert!(current.control.muted.load(Ordering::Relaxed));
            assert_eq!(current.control.volume_setting(), 0.3);
            assert!(playback.pending.is_none());
            assert!(playback.retiring.is_none());
        }
        playback.shutdown().await;
    }
    #[tokio::test]
    async fn invalid_audio_header_recovers_from_a_fresh_media_ticket() {
        use axum::{http::{HeaderMap, header}, Json};
        use std::sync::atomic::AtomicUsize;

        let tickets = Arc::new(AtomicUsize::new(0));
        let requests = tickets.clone();
        let (profile, server) = server(Router::new()
            .route("/api/v1/media/broken", get(|| async { "not audio" }))
            .route("/api/v1/library/tracks/song/ticket", get(move |headers: HeaderMap| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer test-token");
                requests.fetch_add(1, Ordering::Relaxed);
                Json(json!({"api_path":"api/v1/media/song?ticket=fresh"}))
            }))
            .route("/api/v1/media/song", get(|| async { ([(header::CONTENT_TYPE, "audio/wav")], wav()) })))
            .await;
        let config = crate::sequence::Config::new(profile.clone(), "test-token".into(), "test-output".into()).unwrap();
        let invalid = profile.media_url("api/v1/media/broken").unwrap();
        let (_, metadata) = config.prepare_media("song", invalid.as_str()).await.unwrap();
        assert_eq!(metadata.format, "wav");
        assert_eq!(metadata.sample_rate, 48000);
        assert_eq!(tickets.load(Ordering::Relaxed), 1);
        server.abort();
    }

    #[tokio::test]
    async fn invalid_audio_cache_is_bypassed_after_the_first_probe() {
        use axum::{http::header, Json};
        use std::io::Write;

        struct InvalidCache(std::path::PathBuf);
        impl crate::media_proxy::MediaResolver for InvalidCache {
            fn resolve<'a>(
                &'a self,
                _: &'a ConnectionProfile,
                _: &'a str,
                _: &'a str,
            ) -> futures_util::future::BoxFuture<'a, Result<Arc<MediaProxy>>> {
                Box::pin(async { Ok(Arc::new(MediaProxy::local_file(&self.0, Box::new(()))?)) })
            }
        }

        let mut cache = tempfile::NamedTempFile::new().unwrap();
        cache.write_all(b"not audio").unwrap();
        let (profile, server) = server(Router::new()
            .route("/api/v1/library/tracks/song/ticket", get(|| async {
                Json(json!({"api_path":"api/v1/media/song?ticket=fresh"}))
            }))
            .route("/api/v1/media/song", get(|| async { ([(header::CONTENT_TYPE, "audio/wav")], wav()) })))
            .await;
        let mut config = crate::sequence::Config::new(profile.clone(), "test-token".into(), "test-output".into()).unwrap();
        config.media = Some(Arc::new(InvalidCache(cache.path().to_owned())));
        let original = profile.media_url("api/v1/media/song").unwrap();
        let (proxy, metadata) = config.prepare_media("song", original.as_str()).await.unwrap();
        assert!(!proxy.serves_local_file());
        assert_eq!(metadata.sample_rate, 48000);
        server.abort();
    }

    #[tokio::test]
    async fn failed_real_probe_reports_the_source_track_and_zone() {
        let (profile, server) =
            server(Router::new().route("/api/v1/media/broken", get(|| async { "not audio" })))
                .await;
        let mut playback = Playback::default();
        playback
            .command("play", &play(&profile, "broken", "broken"), &profile)
            .unwrap();
        let report = tokio::time::timeout(Duration::from_secs(5), playback.update())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(report["type"], "error");
        assert_eq!(report["track_id"], "broken");
        assert_eq!(report["zone_id"], "z");
        assert_eq!(report["core_playback_id"],"core-broken");
        assert!(playback.pending.is_none());
        assert!(playback.running().is_none());
        server.abort();
    }
}
