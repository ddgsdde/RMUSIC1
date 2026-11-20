//! Continuous native output; core commits are serialized independently of the control socket.
use super::*;
use rmusic_audio::output::{PlaybackSequence, PlaybackUpdate, SequenceFormat, SequencePosition};
use std::{collections::{HashMap,VecDeque}, sync::Mutex};
use tokio::sync::{mpsc, watch};
mod listens;
mod cursor;
mod group;
mod capture;
mod report_gate;
#[cfg(target_os="android")] mod android_media;
use rmusic_bridge::listens::cursor::{Handle as CursorHandle,Recorder as CursorRecorder,Boundary as CursorBoundary};
use std::sync::atomic::AtomicU8;

pub(super) use rmusic_bridge::sequence::{Config,Initial};
use rmusic_bridge::sequence::{HandoffTimingConflict,Item,Preparation};
#[derive(Clone)]
struct Entry {
    #[cfg(target_os="android")] metadata:Value,
    track: String,
    request: Option<PlaybackRequest>,
    audio: Option<Audio>,
    listen_session_id:Option<String>,
}
type Tracks = Arc<Mutex<HashMap<String, Entry>>>;
type Known = Arc<Mutex<VecDeque<String>>>;
async fn yield_to_current_output(control: &PlaybackControl) -> Result<()> {
    // Give the first decoder priority, but a short source cannot fill the HAL ring
    // until its successor is ready. Preparing that successor must not require sound.
    let _ = tokio::time::timeout(Duration::from_millis(250), async {
        while control.frames_played.load(Ordering::Acquire) == 0 {
            if control.stopped.load(Ordering::Acquire) { return; }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }).await;
    if control.stopped.load(Ordering::Acquire) { return Err(output::PreloadCancelled.into()); }
    Ok(())
}
enum Update {
    Progress(PlaybackUpdate),
    Finished(Result<()>, SequencePosition),
    Refresh,
    QueueChanged(Vec<String>),
}
pub(super) struct Run {
    group:Option<group::Shared>,
    finish_reason:Arc<AtomicU8>,
    cursor:Option<CursorHandle>,
    plan_id: String,
    sequence: PlaybackSequence,
    control: Arc<PlaybackControl>,
    tracks: Tracks,
    known: Known,
    cancelled: watch::Sender<bool>,
    queue_changes: watch::Sender<Vec<String>>,
    coordinator: JoinHandle<()>,
}
impl Run {
    pub fn group_completion(&mut self)->Option<&mut JoinHandle<()>> {self.group.as_ref()?;Some(&mut self.coordinator)}
    pub fn set_rate_ppm(&self,ppm:f64)->Result<()> {
        if self.group.is_none()||!self.sequence.rate_adjustment_active()||self.coordinator.is_finished()||self.control.stopped.load(Ordering::Acquire)||self.control.paused.load(Ordering::Relaxed){bail!("group rate output is unavailable");}
        #[cfg(feature="adaptive-sync")] {self.sequence.set_rate_ppm(ppm)}
        #[cfg(not(feature="adaptive-sync"))] {let _=ppm;bail!("PCM rate adjustment is not compiled in")}
    }
    pub fn final_report(&self)->Result<Option<Value>> {
        self.group.as_ref().map(|ledger|ledger.lock().map_err(|_|anyhow::anyhow!("group frame lock poisoned"))?.finish(&self.sequence,&self.control,&self.tracks)).transpose()
    }
    pub fn mark_stop(&self){self.finish_reason.store(1,Ordering::Release);}
    pub fn mark_replacement(&self,replay:bool){self.finish_reason.store(if replay{3}else{2},Ordering::Release);}
    pub fn retire_cursor(&self){if let Some(cursor)=&self.cursor {if let Err(error)=cursor.retire(){eprintln!("Cursor retirement failed: {error:#}");}}}
    pub fn matches(&self, id: Option<&str>) -> bool {
        id == Some(&self.plan_id)
    }
    pub fn owns_control(&self,id:Option<&str>)->bool {
        !self.coordinator.is_finished()&&id.is_some_and(|id|self.known.lock().is_ok_and(|known|known.iter().any(|value|value==id)))
    }
    pub fn current_id(&self) -> Option<String> {
        self.sequence
            .position(&self.control)
            .ok()
            .map(|p| p.playback_id)
    }
    pub fn programme_clock(&self,sample:&output::clock::Sample)->Option<Value> {
        let programme=sample.programme?;
        if !sample.running||programme.sample_rate!=self.control.output_sample_rate.load(Ordering::Acquire)||self.control.paused.load(Ordering::Relaxed)||self.control.stopped.load(Ordering::Acquire){return None;}
        let adjusted=self.sequence.rate_adjustment_active();
        let source=if adjusted{Some(self.sequence.source_clock_at_frame(&self.control,programme.frames).ok()?)}else{None};
        let position=if let Some(source)=&source{source.position.clone()}else{self.sequence.position_at_frame(&self.control,programme.frames).ok()?};
        let tracks=self.tracks.lock().ok()?;let entry=tracks.get(&position.playback_id)?;
        if entry.request.as_ref()?.is_dsd(){return None;}
        let mut value=json!({"version":if adjusted{2}else{1},"device_epoch":sample.epoch,"device_counter":sample.counter,"output_frame":programme.frames,"sample_rate":programme.sample_rate,"core_playback_id":position.playback_id,"track_id":entry.track,"position":position.position});
        if let Some(source)=source{value["source_frame"]=json!(source.source_frame);value["source_fraction"]=json!(source.source_fraction);value["source_per_output"]=json!(source.source_per_output);value["track_output_frames"]=json!(position.frames);}
        else{value["source_frames"]=json!(position.frames);}Some(value)
    }
    pub fn position(&self) -> Option<f64> {
        self.sequence
            .position(&self.control)
            .ok()
            .map(|p| p.position)
    }
    pub fn snapshot(
        &self,
        id: Option<&str>,
    ) -> Option<(PlaybackRequest, Audio, Option<String>, String)> {
        let id = id.map(String::from).or_else(|| self.current_id())?;
        let tracks = self.tracks.lock().ok()?;
        let entry = tracks.get(&id)?;
        Some((
            entry.request.clone()?,
            entry.audio.clone()?,
            Some(id),
            entry.track.clone(),
        ))
    }
    pub fn for_command(
        &self,
        command: &Value,
    ) -> Option<(PlaybackRequest, Audio, Option<String>, String)> {
        if let Some(snapshot) = self.snapshot(command["playback_id"].as_str()) {
            return Some(snapshot);
        }
        let track = command["track"]["id"].as_str()?;
        let id = self
            .tracks
            .lock()
            .ok()?
            .iter()
            .find(|(_, entry)| entry.track == track)
            .map(|(id, _)| id.clone())?;
        self.snapshot(Some(&id))
    }
    pub fn refresh_queue(&self)->Result<()> {
        let committed=self.sequence.refresh_queue()?;
        let _=self.queue_changes.send(committed);
        Ok(())
    }
    pub fn cancel(&self) {
        let _ = self.cancelled.send(true);
        self.sequence.cancel_next();
        self.coordinator.abort();
    }
}
impl Drop for Run {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn stage(
    sequence: &PlaybackSequence,
    items: Vec<Item>,
    config: &Config,
    template: &PlaybackRequest,
    tracks: &Tracks,
    cancelled: &watch::Receiver<bool>,
    queue_changes: &watch::Receiver<Vec<String>>,
    runtime: &tokio::runtime::Handle,
    control:&Arc<PlaybackControl>,
) -> Result<bool> {
    for item in items {
        if queue_changes.has_changed().unwrap_or(true) {return Ok(false);}
        if tracks
            .lock()
            .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?
            .contains_key(&item.playback_id)
        {
            continue;
        }
        if cfg!(target_os="macos")&&config.media.is_some()&&tracks.lock().map_err(|_|anyhow::anyhow!("sequence tracks lock poisoned"))?.len()>=2{return Ok(false);}
        let reservation = match sequence.reserve() {
            Ok(slot) => slot,
            Err(_) => return Ok(false),
        };
        let track = item.track["id"]
            .as_str()
            .context("preloaded track has no ID")?
            .to_owned();
        tracks
            .lock()
            .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?
            .insert(
                item.playback_id.clone(),
                Entry {
                    #[cfg(target_os="android")] metadata:item.track.clone(),
                    track,
                    request: None,
                    audio: None,
                    listen_session_id:item.listen_session_id,
                },
            );
        let config = config.clone();
        let mut template = template.clone();
        let tracks = tracks.clone();
        let mut cancelled = cancelled.clone();
        let mut queue_changes=queue_changes.clone();
        let control=control.clone();
        runtime.spawn(async move {
            let id = item.playback_id;
            let mut reservation = Some(reservation);
            let task = async {
                if cfg!(target_os="macos")&&config.media.is_some(){yield_to_current_output(&control).await?;}
                let source = config.profile.media_url(&item.media_path)?.to_string();
                let (proxy,metadata)=config.prepare_media(item.track["id"].as_str().unwrap_or(""),&source).await?;
                template.input = proxy.url.clone();
                template.flac_samples = metadata.flac_samples;
                template.position = 0.;
                let decode_dsd=cfg!(target_os="macos")&&metadata.is_dsd();
                template.dsd_pcm=decode_dsd.then_some(metadata.sample_rate);
                template.sample_rate = if decode_dsd {metadata.dsd_pcm_rate()}else{metadata.output_rate_for(if reservation.as_ref().context("missing preload reservation")?.native_dsd(){output::DsdOutput::Native}else{output::DsdOutput::Dop})};
                template.channels = metadata.channels;
                template.source_integer_lossless = metadata.source_integer_lossless;
                template.source_bit_depth = if decode_dsd {32}else{metadata.bit_depth};
                template.duration = metadata.duration;
                template.measured_lufs = measured_loudness(&template.dsp, &item.track);
                let audio = Audio {
                    metadata,
                    proxy: proxy.clone(),
                };
                {
                    let mut tracks = tracks
                        .lock()
                        .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?;
                    if let Some(entry) = tracks.get_mut(&id) {
                        entry.request = Some(template.clone());
                        entry.audio = Some(audio);
                    }
                }
                let duration = template.duration;
                let decoder_id = id.clone();
                let reservation = reservation.take().unwrap();
                let completion_proxy=proxy.clone();
                let result=tokio::task::spawn_blocking(move || {
                    reservation.checked(
                        decoder_id,
                        template,
                        Some(Arc::new(move |position| {
                            completion_proxy.finish_playback(Ok(()), position, duration)
                        })),
                    )
                })
                .await?;
                if result.as_ref().err().is_some_and(|e|e.is::<output::PreloadFormatChange>()){proxy.prefetch_start().await?;}
                result?;
                Ok::<_, anyhow::Error>(())
            };
            let result = tokio::select! {
                result=task=>Some(result),
                _=cancelled.changed()=>None,
                _=queue_changes.changed()=>None
            };
            if let Some(Err(error)) = result {
                if let Some(reservation) = reservation.take() {
                    reservation.fail();
                }
                if !*cancelled.borrow() && !error.is::<output::PreloadFormatChange>() {
                    eprintln!("Next source preparation failed ({id}): {error}");
                }
            }
        });
    }
    Ok(true)
}
fn report_boundary(
    report: &rmusic_audio::output::CompletedTrack,
    tracks: &Tracks,
) -> Result<Value> {
    let tracks = tracks
        .lock()
        .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?;
    let entry = tracks
        .get(&report.playback_id)
        .context("unknown completed source")?;
    Ok(
        json!({"playback_id":report.playback_id,"track_id":entry.track,"position":report.position,"frames":report.frames}),
    )
}
fn current(position: &SequencePosition, tracks: &Tracks) -> Result<Value> {
    let tracks = tracks
        .lock()
        .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?;
    let entry = tracks
        .get(&position.playback_id)
        .context("unknown current source")?;
    Ok(
        json!({"playback_id":position.playback_id,"track_id":entry.track,"position":position.position,"frames":position.frames}),
    )
}
fn failure_sources(confirmed:&str,latest:Option<&Value>,tracks:&Tracks)->Vec<(String,Option<String>)> {
    let Ok(tracks)=tracks.lock() else{return vec![(confirmed.into(),None)]};
    let id=latest.and_then(|zone|zone["playback_id"].as_str().filter(|id|tracks.get(*id).is_some_and(|entry|zone["track"]["id"]==entry.track))).unwrap_or(confirmed);
    let mut failures=vec![(id.into(),tracks.get(id).map(|entry|entry.track.clone()))];
    // If HTTP is unavailable, the live control socket can still match exactly one of these
    // bounded owned instances. A newer seek/queue has a different ID and rejects them all.
    if latest.is_none(){failures.extend(tracks.iter().filter(|(other,_)|other.as_str()!=id).map(|(id,entry)|(id.clone(),Some(entry.track.clone()))));}
    failures
}
/// Keep the exact boundary body while the output worker queues later measured updates.
/// Cancellation drops the current HTTP operation; permanent protocol/auth errors stop retrying.
async fn confirmed_request(config:&Config,zone:&str,action:&str,body:&Value,cancelled:&mut watch::Receiver<bool>)->Result<Option<Value>> {
    for attempt in 0..5 {
        if *cancelled.borrow(){return Ok(None);}
        if attempt>0 {
            tokio::select! {
                _=cancelled.changed()=>return Ok(None),
                _=tokio::time::sleep(Duration::from_secs(1<<(attempt-1)))=>{}
            }
        }
        let result=tokio::select! {
            _=cancelled.changed()=>return Ok(None),
            result=config.request_json(zone,action,body)=>result
        };
        match result {
            Ok(response)=>return Ok(Some(response)),
            Err(error)=>{
                let retry=error.downcast_ref::<reqwest::Error>().is_some_and(|error|!error.status().is_some_and(|status|status.is_client_error()));
                if !retry||attempt==4 {return Err(error);}
                eprintln!("Continuous {action} acknowledgement unavailable; recovery attempt {} of 4",attempt+1);
            }
        }
    }
    unreachable!("five bounded control attempts")
}
pub(super) fn spawn(
    config: Config,
    request: PlaybackRequest,
    plan: Plan,
    audio: Audio,
    output: OutputConfig,
    reports: UnboundedSender<Value>,
    initial: Initial,
    listener:Option<(Arc<ListenStore>,ListenOwner)>,
    recovery_enabled:bool,
    pcm_rate_adjustment:bool,
) -> Result<Running> {
    initial.validate_timing()?;let measured_version=initial.measured_version;
    if measured_version==2&&request.is_dsd(){anyhow::bail!("output timing v2 requires PCM");}
    let group_start_id=initial.group_start_id.clone();
    if group_start_id.is_some()&&(!plan.control.start.is_scheduled()||request.is_dsd()){anyhow::bail!("group sequence requires scheduled PCM output");}
    let listener=if group_start_id.is_some(){None}else{listener};
    let recovery_enabled=recovery_enabled&&initial.continuous_recovery&&group_start_id.is_none();
    let group=group_start_id.as_ref().map(|_|Arc::new(Mutex::new(group::Ledger::default())));
    let id = uuid::Uuid::new_v4().to_string();
    let core_id = plan
        .core_playback_id
        .clone()
        .context("continuous playback requires core instance")?;
    let checkpoint=if recovery_enabled {listener.as_ref().map(|(store,owner)|CursorRecorder::begin_timed(store.clone(),owner.clone(),config.endpoint.clone(),plan.zone_id.clone(),initial.plan_id.clone(),request.sample_rate,
        CursorBoundary {playback_id:core_id.clone(),track_id:plan.track_id.clone(),position:request.position,frames:0},measured_version)).transpose()?}else{None};
    let cursor=checkpoint.as_ref().map(|record|record.handle.clone());
    let proxy = audio.proxy.clone();
    let duration = request.duration;
    let sequence = PlaybackSequence::checked(
        core_id.clone(),
        request.clone(),
        SequenceFormat {
            sample_rate: request.sample_rate,
            channels: request.channels,
            channel_mask:0,
            exclusive: output.exclusive,
            float_output: request.dsp.enabled,
            bit_depth: if request.is_dsd()&&output.dsd_output==output::DsdOutput::Native{1}else{32},
        },
        Some(Arc::new(move |position| {
            proxy.finish_playback(Ok(()), position, duration)
        })),
    )?;
    if pcm_rate_adjustment {
        if group_start_id.is_none()||measured_version!=2{bail!("PCM rate adjustment requires a v2 group");}
        #[cfg(feature="adaptive-sync")] sequence.enable_rate_adjustment(0.)?;
        #[cfg(not(feature="adaptive-sync"))] bail!("PCM rate adjustment is not compiled in");
    }
    let tracks = Arc::new(Mutex::new(HashMap::from([(
        core_id.clone(),
        Entry {
            #[cfg(target_os="android")] metadata:plan.metadata.clone(),
            track: plan.track_id.clone(),
            request: Some(request.clone()),
            audio: Some(audio.clone()),
            listen_session_id:initial.listen_session_id,
        },
    )])));
    let (cancelled, cancellation) = watch::channel(false);
    let (queue_changes, queue_updates)=watch::channel(Vec::new());
    let known=Arc::new(Mutex::new(VecDeque::from([core_id.clone()])));
    let runtime = tokio::runtime::Handle::current();
    let config_start = config.clone();
    let tracks_start = tracks.clone();
    let cancellation_start = cancellation.clone();
    let queue_start=queue_updates.clone();
    let template = request.clone();
    let runtime_start = runtime.clone();
    let items = initial.items.clone();
    let has_more = initial.has_more;
    let start_control=plan.control.clone();
    sequence.on_configured(move |sequence| {
        let all=stage(
            &sequence,
            items,
            &config_start,
            &template,
            &tracks_start,
            &cancellation_start,
            &queue_start,
            &runtime_start,
            &start_control,
        )?;
        sequence.queue_complete(!has_more&&all)
    })?;
    let (updates, update_rx) = mpsc::unbounded_channel();
    #[cfg(target_os="android")]
    android_media::bind(&config,&plan,&sequence,&tracks,group_start_id.as_deref())?;
    let coordinator = Coordinator {
        measured_version,group:group.clone(),group_start_id,
        config,
        zone: plan.zone_id.clone(),
        worker: id.clone(),
        confirmed: core_id,
        plan_id: initial.plan_id.clone(),
        sequence: sequence.clone(),
        control: plan.control.clone(),
        tracks: tracks.clone(),
        known: known.clone(),
        template: request.clone(),
        cancelled: cancellation,
        queue_changes:queue_updates,
        reports,
        updates: update_rx,
        runtime,
        cancel_sender: cancelled.clone(),
        cursor:cursor.clone(),
        refresh:initial.refresh,
    };
    let coordinator = tokio::spawn(coordinator.run());
    let run_control = plan.control.clone();
    let worker_sequence = sequence.clone();
    let worker_control = plan.control.clone();
    let worker_request = request.clone();
    let worker_tracks=tracks.clone();
    let worker_group=group.clone();
    let sampler=listener.map(|(store,owner)|listens::Sampler::new(store,owner,measured_version));
    let finish_reason=Arc::new(AtomicU8::new(0));let worker_reason=finish_reason.clone();
    let worker_id=id.clone();
    #[cfg(target_os="linux")] let sleep_checkpoint=output::power::checkpoint(&worker_control);
    let thread = std::thread::spawn(move || {
        #[cfg(target_os="linux")] let _sleep_checkpoint=sleep_checkpoint;
        eprintln!("Continuous output worker started ({worker_id})");
        let mut report_gate = report_gate::ReportGate::new();
        let ready=checkpoint.as_ref().map(|record|record.handle.ready()).transpose();
        let capture_sequence=worker_sequence.clone();let capture_control=worker_control.clone();let capture_updates=updates.clone();let capture_tracks=worker_tracks.clone();
        let mut capture=capture::Relay::new((sampler,checkpoint),move |(sampler,checkpoint),update|{
                let rate=capture_sequence.format().map(|format|format.sample_rate).unwrap_or(0);
                if let Some(ledger)=&worker_group {
                    if let Err(error)=ledger.lock().map_err(|_|anyhow::anyhow!("group frame lock poisoned")).and_then(|mut ledger|ledger.update(&update,&capture_tracks,rate)){
                        eprintln!("Group frame capture failed: {error:#}");capture_control.stop();
                    }
                }
                if let Some(sampler)=sampler.as_mut(){if let Err(error)=sampler.update(&update,rate,&capture_tracks){eprintln!("Listening capture failed: {error:#}");}}
                if let Some(recorder)=checkpoint.as_mut(){if let Err(error)=cursor::update(recorder,&update,rate,&capture_tracks,||{
                    Ok(sampler.as_mut().map(|sampler|sampler.checkpoint_rows(&update.playback_id)).unwrap_or_default())
                }){eprintln!("Playback cursor capture failed: {error:#}");}}
                if report_gate.due(&update,Instant::now()){let _=capture_updates.send(Update::Progress(update));}
        });
        let mut capture_error=None;
        let result = match ready {
            Err(error)=>Err(error),
            Ok(Some(false))=>Err(anyhow::anyhow!("播放游标未持久保存或已撤销")),
            _=>output::play_sequence(
            worker_request,
            output,
            worker_control.clone(),
            worker_sequence.clone(),
            |update| {
                if let Err(error)=capture.push(update){capture_error=Some(error);worker_control.stop();}
            },
        )};
        let captured=capture.finish();
        let result=capture_error.map_or(result,Err);
        let (mut sampler,mut checkpoint)=match captured {Ok(state)=>state,Err(error)=>{
            worker_control.stop();if let Ok(position)=worker_sequence.position(&worker_control){let _=updates.send(Update::Finished(Err(error),position));}return;
        }};
        if let Some(sampler)=sampler.as_mut(){let position=worker_sequence.position(&worker_control).ok();let rate=worker_sequence.format().map(|f|f.sample_rate).unwrap_or(0);
            let reason=if result.is_ok()&&!worker_control.stopped.load(Ordering::Acquire){"ended"}else{match worker_reason.load(Ordering::Acquire){1=>"stop",2=>"replaced",3=>"replay",_=>"interrupted"}};
            if let Err(error)=sampler.stop(position.as_ref(),rate,&worker_tracks,reason){eprintln!("Listening final checkpoint failed: {error:#}");}}
        if let (Some(recorder),Ok(position))=(checkpoint.as_mut(),worker_sequence.position(&worker_control)) {let rate=worker_sequence.format().map(|f|f.sample_rate).unwrap_or(0);
            let terminal=result.is_ok()&&!worker_control.stopped.load(Ordering::Acquire);
            let rows=sampler.as_mut().map(|sampler|sampler.checkpoint_rows(&position.playback_id)).unwrap_or_default();
            if let Err(error)=cursor::finish(recorder,&position,rate,&worker_tracks,terminal,rows){eprintln!("Playback final cursor failed: {error:#}");}}
        if !worker_control.stopped.load(Ordering::Acquire)||worker_control.system_interrupted() {
            if let Ok(position) = worker_sequence.position(&worker_control) {
                let _ = updates.send(Update::Finished(
                    result,
                    position,
                ));
            }
        }
    });
    Ok(Running {
        id,
        request,
        zone_id: plan.zone_id,
        control: plan.control,
        thread: Some(thread),
        audio,
        track_id: plan.track_id,
        core_playback_id: plan.core_playback_id,
        sequence: Some(Run {
            group,
            finish_reason,
            cursor,
            plan_id: initial.plan_id,
            sequence,
            control: run_control,
            tracks,
            known,
            cancelled,
            queue_changes,
            coordinator,
        }),
    })
}

struct Coordinator {
    measured_version:u32,
    group:Option<group::Shared>,
    group_start_id:Option<String>,
    refresh:bool,
    cursor:Option<CursorHandle>,
    config: Config,
    zone: String,
    worker: String,
    confirmed: String,
    plan_id: String,
    sequence: PlaybackSequence,
    control: Arc<PlaybackControl>,
    tracks: Tracks,
    known: Known,
    template: PlaybackRequest,
    cancelled: watch::Receiver<bool>,
    queue_changes:watch::Receiver<Vec<String>>,
    reports: UnboundedSender<Value>,
    updates: mpsc::UnboundedReceiver<Update>,
    runtime: tokio::runtime::Handle,
    cancel_sender: watch::Sender<bool>,
}
impl Coordinator {
    fn stop(&self) {
        self.control.stop();
        self.sequence.cancel_next();
        let _ = self.cancel_sender.send(true);
    }
    fn send(
        &self,
        kind: &str,
        status: &str,
        position: &SequencePosition,
        extra: Value,
    ) -> Result<()> {
        let track = current(position, &self.tracks)?;
        let mut report = json!({"type":kind,"status":status,"playback_id":self.worker,"zone_id":self.zone,
            "core_playback_id":position.playback_id,"track_id":track["track_id"],"position":position.position});
        if self.measured_version==2&&kind=="state"{report["measured_version"]=json!(2);}
        for (key, value) in extra.as_object().into_iter().flatten() {
            report[key] = value.clone();
        }
        let _ = self.reports.send(report);
        Ok(())
    }
    async fn progress(&mut self, mut update: PlaybackUpdate) -> Result<()> {
        let _request = self
            .tracks
            .lock()
            .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?
            .get(&update.playback_id)
            .and_then(|e| e.request.clone())
            .context("missing current source request")?;
        // Reserved sources may need another HAL format and never enter this run.
        // Only heard sources own resume commands while their Core receipt is pending.
        {
            let mut known = self.known.lock().map_err(|_| anyhow::anyhow!("sequence identity lock poisoned"))?;
            if !known.iter().any(|id| id == &update.playback_id) {
                known.push_back(update.playback_id.clone());
                while known.len() > 128 { known.pop_front(); }
            }
        }
        let rate = self.sequence.format()?.sample_rate;
        let position = SequencePosition {
            playback_id: update.playback_id.clone(),
            position: update.position,
            frames: update.output_frames,
        };
        // The output's final callback also contains the final current source. Its terminal EOF
        // follows any prior atomic handoff and is never included as a second current source.
        let ended: Vec<_> = update
            .ended
            .into_iter()
            .filter(|e| e.playback_id != position.playback_id)
            .collect();
        if !ended.is_empty() {
            let boundaries = ended
                .iter()
                .map(|e| report_boundary(e, &self.tracks))
                .collect::<Result<Vec<_>>>()?;
            let mut body = json!({"endpoint_id":self.config.endpoint,"request_id":uuid::Uuid::new_v4().to_string(),
                "plan_id":self.plan_id,"from_playback_id":self.confirmed,"sample_rate":rate,
                "ended":boundaries,"current":current(&position,&self.tracks)?});
            if self.measured_version==2{body["measured_version"]=json!(2);}
            let Some(response) = confirmed_request(&self.config,&self.zone,"handoff",&body,&mut self.cancelled).await? else {return Ok(());};
            if response["accepted"] != true
                || response["zone"]["playback_id"] != position.playback_id
                || !matches!(
                    response["zone"]["status"].as_str(),
                    Some("playing" | "paused")
                )
            {
                if let Some(cursor)=&self.cursor {cursor.retire()?;}
                self.stop();
                return Ok(());
            }
            self.confirmed = position.playback_id.clone();
            if let Some(ledger)=&self.group{ledger.lock().map_err(|_|anyhow::anyhow!("group frame lock poisoned"))?.acknowledge(&ended)?;}
            {
                let mut tracks = self
                    .tracks
                    .lock()
                    .map_err(|_| anyhow::anyhow!("sequence tracks lock poisoned"))?;
                for track in &ended {
                    tracks.remove(&track.playback_id);
                }
            }
            self.refill(None).await?;
        }
        if self.confirmed == position.playback_id {
            if let Some(audio)=self.tracks.lock().map_err(|_|anyhow::anyhow!("sequence tracks lock poisoned"))?.get(&position.playback_id).and_then(|e|e.audio.as_ref()){audio.proxy.annotate(&mut update.signal_path);}
            self.send("state",if self.control.paused.load(Ordering::Relaxed){"paused"}else{"playing"},&position,
                json!({"signal_path":update.signal_path,"meter":update.meter,"continuity_lost":update.continuity_lost,"frames":position.frames,"sample_rate":rate}))?;
        }
        Ok(())
    }
    async fn refill(&mut self,committed:Option<Vec<String>>)->Result<()> {
        if self.queue_changes.has_changed().unwrap_or(true) {return Ok(());}
        let mut changes=self.queue_changes.clone();
        if self.sequence.format().is_err() {return Ok(());}
        let track=self.tracks.lock().map_err(|_|anyhow::anyhow!("sequence tracks lock poisoned"))?
            .get(&self.confirmed).map(|entry|entry.track.clone()).context("missing current sequence source")?;
        let next=tokio::select! {
            _=self.cancelled.changed()=>return Ok(()),
            _=changes.changed()=>return Ok(()),
            result=async {
                if let Some(committed)=&committed {self.config.refresh_queue_timed(&self.zone,&track,&self.confirmed,self.measured_version,&self.plan_id,committed).await}
                else {self.config.negotiate_timed(&self.zone,&track,&self.confirmed,self.measured_version).await}
            }=>match result?{Preparation::Ready(next)=>Some(next),Preparation::Stale(_)|Preparation::Unsupported=>None},
        };
        let Some(next)=next else {self.stop();return Ok(());};
        if next.measured_version!=self.measured_version||next.plan_id!=self.plan_id||next.group_start_id!=self.group_start_id {self.stop();return Ok(());}
        self.refresh=next.refresh;
        if changes.has_changed().unwrap_or(true) {return Ok(());}
        if let Some(committed)=committed {
            let known=self.known.lock().map_err(|_|anyhow::anyhow!("sequence identity lock poisoned"))?;
            self.tracks.lock().map_err(|_|anyhow::anyhow!("sequence tracks lock poisoned"))?.retain(|id,_|known.contains(id)||committed.contains(id));
        }
        let all=stage(&self.sequence,next.items,&self.config,&self.template,&self.tracks,&self.cancelled,&changes,&self.runtime,&self.control)?;
        let _=self.sequence.queue_complete(!next.has_more&&all);
        Ok(())
    }
    async fn run(mut self) {
        let mut refresh=tokio::time::interval_at(tokio::time::Instant::now()+Duration::from_secs(1),Duration::from_secs(1));
        loop {
            let update = tokio::select! {
                _=self.cancelled.changed()=>break,
                changed=self.queue_changes.changed()=>if changed.is_ok(){Some(Update::QueueChanged(self.queue_changes.borrow_and_update().clone()))}else{None},
                update=self.updates.recv()=>update,
                _=refresh.tick(),if self.refresh=>Some(Update::Refresh)
            };
            let Some(update) = update else { break };
            let terminal = matches!(&update, Update::Finished(..));
            let refilled=matches!(&update,Update::Refresh|Update::QueueChanged(_))||matches!(&update,Update::Progress(update) if !update.ended.is_empty());
            let result=match update {
                Update::Progress(update)=>self.progress(update).await,
                Update::Refresh=>self.refill(None).await,
                Update::QueueChanged(committed)=>match self.refill(Some(committed)).await {
                    Ok(())=>Ok(()),
                    Err(error)=>{
                        eprintln!("Next queue preparation deferred until the current output ends: {error:#}");
                        let _=self.sequence.queue_complete(true);self.refresh=false;Ok(())
                    }
                },
                Update::Finished(result,position)=>match result {
                    Ok(()) if self.confirmed==position.playback_id=>{
                        match current(&position,&self.tracks) {
                            Ok(mut body)=>{body["sample_rate"]=json!(self.sequence.format().map(|f|f.sample_rate).unwrap_or(0));if self.measured_version==2{body["measured_version"]=json!(2);}
                                confirmed_request(&self.config,&self.zone,"ended",&body,&mut self.cancelled).await.and_then(|response|{
                                    if response.is_some(){if let Some(cursor)=&self.cursor{cursor.retire()?;}}Ok(())})},
                            Err(error)=>Err(error)
                        }
                    },
                    Ok(())=>Err(anyhow::anyhow!("continuous source ended before core handoff")),
                    Err(error)=>{
                        let source=self.tracks.lock().ok().and_then(|tracks|tracks.get(&position.playback_id).and_then(|entry|
                            entry.audio.as_ref().map(|audio|(audio.proxy.clone(),entry.request.as_ref().and_then(|request|request.duration)))));
                        if let Some((proxy,duration))=source {proxy.finish_playback(Err(error),position.position,duration)}
                        else {Err(error)}
                    }
                }
            };
            // Preparing after a handoff starts a fresh interval; do not replay idle ticks.
            if refilled {refresh.reset();}
            if let Err(error) = result {
                let device_failed=output::is_device_unavailable(&error);
                #[cfg(not(any(windows,target_os="linux")))] let system_suspend=Value::Null;
                #[cfg(any(windows,target_os="linux"))] let system_suspend=if device_failed {tokio::task::spawn_blocking(output::power::held_stamp).await.ok().and_then(|v|v.ok()).flatten().map_or(Value::Null,|v|json!(v))}else{Value::Null};
                let source_failed=error.is::<output::SourceUnavailable>()||error.is::<rmusic_audio::decoder::DecodeFailure>();
                let handoff_timing = error.is::<HandoffTimingConflict>();
                let retryable=handoff_timing||error.is::<output::SourceUnavailable>()||error.is::<output::DeviceClockInterrupted>();
                // A valid live boundary can miss the arrival-time budget. Preserve its device
                // checkpoint for verified recovery, while leaving the rejected statistics untouched.
                let recoverable=self.cursor.is_some()&&(handoff_timing||device_failed||source_failed||error.downcast_ref::<reqwest::Error>().is_some_and(|error|!error.status().is_some_and(|status|status.is_client_error()&&!matches!(status.as_u16(),404|409))));
                if !recoverable {if let Some(cursor)=&self.cursor {let _=cursor.retire();}}
                self.stop();
                // An acknowledgement may have been lost after the core advanced. Attribute the
                // terminal error to that owned source, without touching a newer seek or queue.
                let latest=if handoff_timing||error.downcast_ref::<reqwest::Error>().is_some(){self.config.zone(&self.zone).await.ok()}else{None};
                for (failed,track) in failure_sources(&self.confirmed,latest.as_ref(),&self.tracks) {
                let _ = self.reports.send(
                    json!({"type":"error","playback_id":self.worker,"zone_id":self.zone,
                    "core_playback_id":failed,"track_id":track,"recoverable":recoverable,"device_failed":device_failed,"source_failed":source_failed,"retryable":retryable,"system_suspend":system_suspend,"error":error.to_string()}),
                );
                }
                break;
            }
            if terminal || (self.control.stopped.load(Ordering::Acquire)&&!self.control.system_interrupted()) {
                break;
            }
        }
        let _ = self.cancel_sender.send(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn preload_can_proceed_when_a_short_current_source_has_not_started() {
        let control = PlaybackControl::default();
        tokio::time::timeout(Duration::from_secs(1), yield_to_current_output(&control))
            .await.expect("next preparation must not depend on audible frames").unwrap();
        assert_eq!(control.frames_played.load(Ordering::Acquire), 0);
        control.stop();
        assert!(yield_to_current_output(&control).await.unwrap_err().is::<output::PreloadCancelled>());
    }
    #[tokio::test]
    async fn group_eof_retires_after_coordinator_confirmation_and_captures_only_after_worker_join() {
        let(profile,server)=super::super::tests::server(axum::Router::new()).await;
        let mut running=super::super::tests::running(&profile).await;let control=running.control.clone();let id=uuid::Uuid::new_v4().to_string();running.core_playback_id=Some(id.clone());
        let sequence=PlaybackSequence::new(id.clone(),running.request.clone(),SequenceFormat{sample_rate:48000,channels:2,channel_mask:0,exclusive:false,float_output:false,bit_depth:32}).unwrap();
        let(dir,owner)=(tempfile::tempdir().unwrap(),ListenOwner{core_id:"qa-core".into(),user_id:"qa-user".into()});let store=ListenStore::open(dir.path()).unwrap();
        let(mut playback,core,host)=(Playback::connected(profile.clone(),"qa-token".into(),"qa-endpoint".into()).unwrap(),uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string());playback.attach_journal(store.clone(),owner.clone());playback.clock_identity(&core,&host);
        let command=json!({"type":"prepare_start","measured_version":2,"group_queue":true,"playback_id":id,"zone_id":"z","track":{"id":running.track_id},"start":{"version":1,"plan_id":uuid::Uuid::new_v4().to_string(),"generation":1,"core_clock_id":core,"host_clock_id":host,"expires_host_ns":output::clock::host_ns().unwrap()+5_000_000_000u64}});
        playback.arm_start(&command,&control).unwrap();playback.starting_output(&control);
        let(confirmed,confirmation)=tokio::sync::oneshot::channel();let(release,worker)=std::sync::mpsc::channel();let(cancelled,_)=watch::channel(false);let joined=Arc::new(std::sync::atomic::AtomicBool::new(false));let done=joined.clone();
        running.thread=Some(std::thread::spawn(move||{worker.recv().unwrap();done.store(true,Ordering::Release);}));
        running.sequence=Some(Run{group:Some(Arc::new(Mutex::new(group::Ledger::default()))),finish_reason:Arc::new(AtomicU8::new(0)),cursor:None,plan_id:"qa-plan".into(),sequence,control:control.clone(),tracks:Arc::new(Mutex::new(HashMap::from([(id.clone(),Entry{track:running.track_id.clone(),request:Some(running.request.clone()),audio:Some(running.audio.clone()),listen_session_id:None})]))),known:Arc::new(Mutex::new(VecDeque::from([id]))),cancelled,queue_changes:watch::channel(Vec::new()).0,coordinator:tokio::spawn(async move{confirmation.await.unwrap();})});
        playback.running=Some(running);let(reports,mut received)=tokio::sync::mpsc::unbounded_channel();playback.output_reports=Some(reports);
        assert!(tokio::time::timeout(Duration::from_millis(25),playback.update()).await.is_err());assert!(!control.stopped.load(Ordering::Acquire));assert!(received.try_recv().is_err());
        confirmed.send(()).unwrap();let result=tokio::time::timeout(Duration::from_secs(1),playback.update()).await;
        // Unblock the real join even when the old implementation fails to notice completion.
        release.send(()).unwrap();assert!(result.is_ok(),"confirmed natural group EOF must wake retirement without a command or session shutdown");
        assert!(playback.running.is_none());assert!(received.try_recv().is_err());
        tokio::time::timeout(Duration::from_secs(1),playback.update()).await.unwrap();assert!(joined.load(Ordering::Acquire));
        let final_report=received.try_recv().unwrap();assert_eq!(final_report["final"],true);assert_eq!(final_report["start"],command["start"]);assert_eq!(final_report["measured_version"],2);assert_eq!(final_report["programme_frames_played"],0);
        let saved=rmusic_bridge::listens::group::pending(&store,&owner).unwrap();assert_eq!(saved.len(),1);assert_eq!(saved[0].body["report"],final_report);assert!(playback.retiring.is_none());server.abort();
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn body(socket:&mut tokio::net::TcpStream,action:&str)->Value {
        let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n") {headers.push(socket.read_u8().await.unwrap());}
        let headers=String::from_utf8(headers).unwrap();
        assert!(headers.starts_with(&format!("POST /prefix/api/v1/zones/z/{action} ")));assert!(headers.contains("Bearer qa-session"));
        let size=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse::<usize>().unwrap())).unwrap();
        let mut bytes=vec![0;size];socket.read_exact(&mut bytes).await.unwrap();serde_json::from_slice(&bytes).unwrap()
    }
    fn config(address:std::net::SocketAddr)->Config {
        Config::new(ConnectionProfile {core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}/prefix"),core_id:Some("qa-core".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]},"qa-session".into(),"endpoint".into()).unwrap()
    }
    #[test]
    fn terminal_ack_failure_targets_the_owned_core_source_and_rejects_new_seek_identity() {
        let tracks=Arc::new(Mutex::new(HashMap::from([
            ("a".into(),Entry {track:"track-a".into(),request:None,audio:None,listen_session_id:None}),
            ("b".into(),Entry {track:"track-b".into(),request:None,audio:None,listen_session_id:None}),
        ])));
        assert_eq!(failure_sources("a",Some(&json!({"playback_id":"b","track":{"id":"track-b"}})),&tracks),vec![("b".into(),Some("track-b".into()))]);
        for latest in [json!({"playback_id":"new-seek","track":{"id":"track-b"}}),json!({"playback_id":null,"status":"stopped"}),json!({"playback_id":"b","track":{"id":"another-track"}})] {
            assert_eq!(failure_sources("a",Some(&latest),&tracks),vec![("a".into(),Some("track-a".into()))]);
        }
        assert_eq!(failure_sources("a",None,&tracks),vec![("a".into(),Some("track-a".into())),("b".into(),Some("track-b".into()))]);
    }
    #[tokio::test]
    async fn control_recovery_retries_the_frozen_handoff_and_final_eof_beyond_initial_attempts() {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let config=config(listener.local_addr().unwrap());
        let server=tokio::spawn(async move {
            let mut requests=Vec::new();
            for action in ["handoff","ended"] {for attempt in 0..4 {
                let (mut socket,_)=tokio::time::timeout(Duration::from_secs(4),listener.accept()).await.unwrap().unwrap();requests.push(body(&mut socket,action).await);
                let response=if action=="handoff" {json!({"accepted":true,"applied":false,"zone":{"playback_id":"b","status":"paused"}})}else{json!({"playback_id":"newer","status":"playing"})};
                if attempt<3 {
                    if action=="handoff" {socket.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();}
                    else{socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();}
                }else{let response=response.to_string();socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();}
            }}requests
        });
        let (_cancelled,mut cancellation)=watch::channel(false);
        let handoff=json!({"request_id":uuid::Uuid::new_v4().to_string(),"ended":[{"playback_id":"a","frames":96000}],"current":{"playback_id":"b","frames":12000}});
        let ended=json!({"playback_id":"b","track_id":"track-b","frames":144000,"position":3.,"sample_rate":48000});
        assert_eq!(confirmed_request(&config,"z","handoff",&handoff,&mut cancellation).await.unwrap().unwrap()["zone"]["status"],"paused");
        assert_eq!(confirmed_request(&config,"z","ended",&ended,&mut cancellation).await.unwrap().unwrap()["playback_id"],"newer");
        let requests=server.await.unwrap();assert_eq!(&requests[..4],&vec![handoff;4]);assert_eq!(&requests[4..],&vec![ended;4]);
    }
    #[tokio::test]
    async fn permanent_control_auth_error_does_not_enter_recovery() {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let config=config(listener.local_addr().unwrap());
        let server=tokio::spawn(async move {
            let(mut socket,_)=listener.accept().await.unwrap();body(&mut socket,"handoff").await;
            socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(150),listener.accept()).await.is_err());
        });
        let(_cancelled,mut cancellation)=watch::channel(false);
        let error=confirmed_request(&config,"z","handoff",&json!({}),&mut cancellation).await.unwrap_err();
        assert_eq!(error.downcast_ref::<reqwest::Error>().unwrap().status(),Some(reqwest::StatusCode::FORBIDDEN));server.await.unwrap();
    }
    #[tokio::test]
    async fn unavailable_control_exhausts_five_bounded_batches_without_changing_the_request() {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let config=config(listener.local_addr().unwrap());
        let original=json!({"request_id":uuid::Uuid::new_v4().to_string(),"current":{"playback_id":"b","frames":12000}});let expected=original.clone();
        let server=tokio::spawn(async move {
            for _ in 0..15 {
                let(mut socket,_)=tokio::time::timeout(Duration::from_secs(12),listener.accept()).await.unwrap().unwrap();assert_eq!(body(&mut socket,"handoff").await,expected);
                socket.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            }
            assert!(tokio::time::timeout(Duration::from_millis(150),listener.accept()).await.is_err());
        });
        let(_cancelled,mut cancellation)=watch::channel(false);
        let error=tokio::time::timeout(Duration::from_secs(25),confirmed_request(&config,"z","handoff",&original,&mut cancellation)).await.unwrap().unwrap_err();
        assert_eq!(error.downcast_ref::<reqwest::Error>().unwrap().status(),Some(reqwest::StatusCode::SERVICE_UNAVAILABLE));server.await.unwrap();
    }
    #[tokio::test]
    async fn cancelling_control_recovery_closes_the_in_flight_tcp_response() {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let config=config(listener.local_addr().unwrap());
        let entered=Arc::new(tokio::sync::Notify::new());let notify=entered.clone();
        let server=tokio::spawn(async move {
            let(mut socket,_)=listener.accept().await.unwrap();body(&mut socket,"handoff").await;notify.notify_one();
            let mut byte=[0];assert_eq!(tokio::time::timeout(Duration::from_secs(3),socket.read(&mut byte)).await.unwrap().unwrap(),0);
        });
        let(cancelled,mut cancellation)=watch::channel(false);
        let request=tokio::spawn(async move {confirmed_request(&config,"z","handoff",&json!({}),&mut cancellation).await});
        tokio::time::timeout(Duration::from_secs(3),entered.notified()).await.unwrap();cancelled.send(true).unwrap();
        assert!(request.await.unwrap().unwrap().is_none());server.await.unwrap();
    }
    #[tokio::test]
    async fn queue_edits_preserve_output_while_processing_changes_replace_it() {
        let profile = ConnectionProfile {core_public_key:None,
            id: "qa".into(),
            name: "qa".into(),
            address: "http://127.0.0.1:9".into(),
            skip_certificate_verification: false, certificate_sha256: None,
            alternatives: vec![],
            core_id: None,
        };
        let audio = Audio {
            proxy: Arc::new(
                MediaProxy::start(&profile, "http://127.0.0.1:9/api/v1/media/a", None)
                    .await
                    .unwrap(),
            ),
            metadata: AudioMetadata {
                flac_samples:None,
                duration: Some(30.),
                format: "wav".into(),
                sample_rate: 48000,
                channels: 2,
                bit_depth: 16,
                bitrate: 0,
                quality_rank: 0,
                source_integer_lossless: true,
            },
        };
        let control = Arc::new(PlaybackControl::default());
        control.set_volume(0.4).unwrap();
        control.paused.store(true, Ordering::Relaxed);
        control.muted.store(true, Ordering::Relaxed);
        let plan = Plan {
            zone_id: "z".into(),
            track_id: "a".into(),
            core_playback_id: Some("core-a".into()),
            position: 0.,
            dsp: DspConfig::default(),
            impulse:None,
            measured_lufs: None,
            transitions: FadeConfig::default(),
            control: control.clone(),
        };
        let request = plan.request(&audio,output::DsdOutput::Disabled).unwrap();
        let sequence = PlaybackSequence::new(
            "core-a".into(),
            request.clone(),
            SequenceFormat {
                sample_rate: 48000,
                channels: 2,
                channel_mask:0,
                exclusive: false,
                float_output: false,
                bit_depth: 32,
            },
        )
        .unwrap();
        let tracks = Arc::new(Mutex::new(HashMap::from([(
            "core-a".into(),
            Entry {
                track: "a".into(),
                request: Some(request.clone()),
                audio: Some(audio.clone()),
                listen_session_id:None,
            },
        )])));
        let (cancelled, _) = watch::channel(false);
        let run = Run {
            group:None,
            finish_reason:Arc::new(AtomicU8::new(0)),
            cursor:None,
            plan_id: "plan-a".into(),
            sequence,
            control: control.clone(),
            tracks,
            known:Arc::new(Mutex::new(VecDeque::from(["core-a".into()]))),
            cancelled,
            queue_changes:watch::channel(Vec::new()).0,
            coordinator: tokio::spawn(std::future::pending()),
        };
        let configuration = Config::new(profile.clone(), "qa-token".into(), "endpoint".into()).unwrap();
        stage(&run.sequence, vec![Item { playback_id: "core-b".into(), track: json!({"id":"b"}), media_path: "api/v1/media/b".into(), listen_session_id: None }],
            &configuration, &request, &run.tracks, &run.cancelled.subscribe(), &run.queue_changes.subscribe(), &tokio::runtime::Handle::current(), &control).unwrap();
        assert!(!run.owns_control(Some("core-b")), "an unplayed next reservation must start a new output when Core sends play");
        let mut playback =
            Playback::connected(profile.clone(), "qa-token".into(), "endpoint".into()).unwrap();
        playback.running = Some(Running {
            id: "worker".into(),
            request,
            zone_id: "z".into(),
            control,
            thread: None,
            audio,
            track_id: "a".into(),
            core_playback_id: Some("core-a".into()),
            sequence: Some(run),
        });
        // Output can finish while its immutable handoff is still awaiting acknowledgement.
        playback.command("play",&json!({"playback_id":"core-a"}),&profile).unwrap();
        assert!(playback.pending.is_none(),"resume must not replay a source while handoff confirmation is pending");
        let original = playback.running.as_ref().unwrap().control.clone();
        playback.command("cancel_preload",&json!({"plan_id":"plan-a","preserve_current":true,"zone":{"endpoint_id":"endpoint","status":"playing","playback_id":"core-a","position":0.,"dsp":DspConfig::default(),"transitions":FadeConfig::default()}}),&profile).unwrap();
        assert!(!original.stopped.load(Ordering::Acquire),"queue edits must not stop the current audio output");
        assert!(playback.pending.is_none()&&playback.retiring.is_none(),"queue edits must not reopen the decoder or output device");
        assert!(Arc::ptr_eq(&original,&playback.running.as_ref().unwrap().control));
        playback.command("pause",&json!({}),&profile).unwrap();
        playback.command("cancel_preload",&json!({"plan_id":"plan-a","zone":{"endpoint_id":"endpoint","status":"paused","playback_id":"core-a","position":0.,
            "dsp":{"enabled":true,"preamp_db":-6.},"transitions":{"fade_in_ms":200,"fade_out_ms":300},"integrated_lufs":-18.5}}),&profile).unwrap();
        let pending = playback.pending.as_ref().unwrap();
        assert!(pending.plan.dsp.enabled);
        assert_eq!(pending.plan.dsp.preamp_db, -6.);
        assert_eq!(pending.plan.transitions.fade_in_ms, 200);
        assert_eq!(pending.plan.transitions.fade_out_ms, 300);
        assert_eq!(pending.plan.measured_lufs, Some(-18.5));
        assert_eq!(pending.plan.control.volume_setting(), 0.4);
        assert!(pending.plan.control.paused.load(Ordering::Relaxed));
        assert!(pending.plan.control.muted.load(Ordering::Relaxed));
        playback.shutdown().await;
    }
    #[tokio::test]
    async fn lost_acknowledgement_body_retries_the_identical_commit_and_accepts_a_receipt() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut bodies = vec![];
            for index in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![];
                let (header_end, length) = loop {
                    let mut buffer = [0; 2048];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]);
                        assert!(header.starts_with("POST /prefix/api/v1/zones/z/handoff HTTP/1.1"));
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|n| n.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < header_end + length {
                    let mut buffer = [0; 2048];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                bodies.push(
                    serde_json::from_slice::<Value>(&bytes[header_end..header_end + length])
                        .unwrap(),
                );
                if index == 0 {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 512\r\nConnection: close\r\n\r\n{").await.unwrap();
                } else {
                    let body = json!({"accepted":true,"applied":false}).to_string();
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                }
                socket.shutdown().await.unwrap();
            }
            bodies
        });
        let profile = ConnectionProfile {core_public_key:None,
            id: "qa".into(),
            name: "qa".into(),
            address: format!("http://{address}/prefix"),
            skip_certificate_verification: false, certificate_sha256: None,
            alternatives: vec![],
            core_id: None,
        };
        let config = Config::new(profile, "isolated-qa-token".into(), "endpoint".into()).unwrap();
        let body =
            json!({"request_id":uuid::Uuid::new_v4().to_string(),"current":{"playback_id":"next"}});
        let ack = config.commit("z", &body).await.unwrap();
        assert_eq!(ack["accepted"], true);
        assert_eq!(ack["applied"], false);
        let requests = server.await.unwrap();
        assert_eq!(requests, vec![body.clone(), body]);
    }
    #[tokio::test]
    async fn group_wire_v2_refill_rejected_during_pause_stops_without_reporting_failure() {
        use axum::{Router,routing::{get,post},Json};
        for version in [1,2] {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let app=Router::new().route("/prefix/api/v1/status",get(||async{Json(json!({"core_id":"qa-core","capabilities":{"measured_listens_v2":true,"measured_listens_online_v2":true,"measured_listens_socket_v2":true,"group_output_v2":true}}))})).route("/prefix/api/v1/zones/z/prepare",post(||async{Json(json!({"accepted":false,"zone":{"id":"z","status":"paused","playback_id":"a"}}))}));
        let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
        let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:"unused".into(),position:0.,sample_rate:768000,channels:2,source_integer_lossless:true,source_bit_depth:32,dsp:DspConfig::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(8.),transitions:FadeConfig::default()};
        let sequence=PlaybackSequence::new("a".into(),request.clone(),SequenceFormat{sample_rate:768000,channels:2,channel_mask:0,exclusive:true,float_output:false,bit_depth:32}).unwrap();let control=Arc::new(PlaybackControl::default());let(cancel_sender,cancelled)=watch::channel(false);let(reports,mut received)=mpsc::unbounded_channel();let(_,updates)=mpsc::unbounded_channel();
        let (_queue_keepalive,queue_changes)=watch::channel(Vec::new());
        let mut coordinator=Coordinator{measured_version:version,group:Some(Arc::new(Mutex::new(group::Ledger::default()))),group_start_id:Some("group".into()),refresh:false,cursor:None,config:config(address),zone:"z".into(),worker:"worker".into(),confirmed:"a".into(),plan_id:"plan".into(),sequence,control:control.clone(),tracks:Arc::new(Mutex::new(HashMap::from([("a".into(),Entry{track:"track".into(),request:Some(request.clone()),audio:None,listen_session_id:None})]))),known:Arc::new(Mutex::new(VecDeque::from(["a".into()]))),template:request,cancelled,queue_changes,reports,updates,runtime:tokio::runtime::Handle::current(),cancel_sender};
        let result=coordinator.refill(None).await;assert!(result.is_ok(),"an authoritative pause rejects refill without a playback failure: {result:?}");assert!(control.stopped.load(Ordering::Acquire));assert!(received.try_recv().is_err());assert_eq!(control.frames_written.load(Ordering::Acquire),0);server.abort();
        }
    }

    #[tokio::test]
    async fn queue_preparation_failure_keeps_current_output_until_its_natural_end() {
        use axum::{Router,routing::post,Json,http::StatusCode};
        let entered=Arc::new(tokio::sync::Notify::new());let signal=entered.clone();
        let ends=Arc::new(std::sync::atomic::AtomicUsize::new(0));let count=ends.clone();
        let app=Router::new().route("/prefix/api/v1/zones/z/prepare",post(move||{let signal=signal.clone();async move{signal.notify_one();StatusCode::SERVICE_UNAVAILABLE}}))
            .route("/prefix/api/v1/zones/z/ended",post(move|Json(body):Json<Value>|{let count=count.clone();async move{assert_eq!(body["playback_id"],"a");count.fetch_add(1,Ordering::Relaxed);Json(json!({"status":"stopped"}))}}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
        let request=PlaybackRequest{flac_samples:None,dsd_pcm:None,input:"unused".into(),position:0.,sample_rate:48000,channels:2,source_integer_lossless:true,source_bit_depth:24,dsp:DspConfig::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(8.),transitions:FadeConfig::default()};
        let sequence=PlaybackSequence::new("a".into(),request.clone(),SequenceFormat{sample_rate:48000,channels:2,channel_mask:0,exclusive:true,float_output:false,bit_depth:32}).unwrap();
        let control=Arc::new(PlaybackControl::default());let(cancel_sender,cancelled)=watch::channel(false);let(reports,mut received)=mpsc::unbounded_channel();let(send_update,updates)=mpsc::unbounded_channel();let(queue_change,queue_changes)=watch::channel(Vec::new());
        let coordinator=Coordinator{measured_version:1,group:None,group_start_id:None,refresh:false,cursor:None,config:config(address),zone:"z".into(),worker:"worker".into(),confirmed:"a".into(),plan_id:"plan".into(),sequence,control:control.clone(),tracks:Arc::new(Mutex::new(HashMap::from([("a".into(),Entry{track:"track".into(),request:Some(request.clone()),audio:None,listen_session_id:None})]))),known:Arc::new(Mutex::new(VecDeque::from(["a".into()]))),template:request,cancelled,queue_changes,reports,updates,runtime:tokio::runtime::Handle::current(),cancel_sender};
        let running=tokio::spawn(coordinator.run());queue_change.send(vec!["a".into()]).unwrap();
        tokio::time::timeout(Duration::from_secs(2),entered.notified()).await.unwrap();
        send_update.send(Update::Finished(Ok(()),SequencePosition{playback_id:"a".into(),position:8.,frames:384000})).unwrap();
        tokio::time::timeout(Duration::from_secs(2),running).await.unwrap().unwrap();
        assert!(!control.stopped.load(Ordering::Acquire));assert_eq!(ends.load(Ordering::Relaxed),1);assert!(received.try_recv().is_err());server.abort();
    }

}
