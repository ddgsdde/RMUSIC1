//! Native client continuous output. The WebView displays state; the worker owns core reports.
use super::*;
use anyhow::Context as _;
use rmusic_audio::output::{PlaybackSequence, PlaybackUpdate, SequenceFormat, SequencePosition};
use rmusic_bridge::sequence::{Config, Initial, Item};
use std::{collections::VecDeque,time::{Duration,Instant}};
use tokio::sync::{mpsc,watch};
mod recovery;
mod listens;
mod cursor;
mod visualization;
pub(super) mod persistent;

#[derive(Clone,Deserialize)]
pub(super) struct Context {
    pub zone_id:String,
    pub endpoint_id:String,
    pub playback_id:String,
}
pub(super) struct Source {
    pub track:NativeTrack,
    pub metadata:rmusic_audio::probe::AudioMetadata,
    pub proxy:Arc<MediaProxy>,
    pub cache_id:String,
    pub _lease:Arc<CacheLease>,
    pub metadata_started:std::sync::atomic::AtomicBool,
    #[cfg(target_os="android")] pub quality:Value,
}
struct Entry {track:String,source:Option<Arc<Source>>,request:Option<PlaybackRequest>,listen_session_id:Option<String>}
type Entries=Arc<Mutex<HashMap<String,Entry>>>;
type Publish=Arc<dyn Fn(Value)+Send+Sync>;
pub(super) struct Player {
    pub(super) measured_version:u32,
    pub epoch:u64,
    pub core:Config,
    pub context:Context,
    pub plan_id:String,
    pub owner:Option<ListenOwner>,
    cursor:Option<rmusic_bridge::listens::cursor::Handle>,
    finish_reason:Arc<std::sync::atomic::AtomicU8>,
    id:String,
    alias:Arc<Mutex<String>>,
    sequence:PlaybackSequence,
    control:Arc<PlaybackControl>,
    entries:Entries,
    known:Arc<Mutex<VecDeque<String>>>,
    report:Arc<Mutex<Value>>,
    journal:recovery::Shared,
    metadata_tasks:crate::cache_tail::MetadataTasks,
    recovery_gate:Arc<tokio::sync::Mutex<()>>,
    output:OutputConfig,
    thread:Option<std::thread::JoinHandle<()>>,
    task:tokio::task::JoinHandle<()>,
    cancelled:watch::Sender<bool>,
}
impl Player {
    pub fn alive(&self)->bool {!self.control.stopped.load(Ordering::Acquire)&&self.thread.as_ref().is_some_and(|thread|!thread.is_finished())}
    pub fn contains(&self,id:&str)->bool {self.known.lock().is_ok_and(|ids|ids.iter().any(|known|known==id))}
    /// Only reuse a source admitted by this exact plan, connection and output policy.
    pub fn prepared_source(&self,_id:&str,track:&str,output:&OutputConfig)->Option<Arc<Source>> {
        if &self.output!=output||!self.acknowledged_end() {return None;}
        let entries=self.entries.lock().ok()?;
        let source=entries.values().find(|entry|entry.track==track)?.source.clone()?;
        #[cfg(target_os="android")] if source.quality!=super::android::source_quality(output.device_id.as_deref()).ok()?{return None;}
        Some(source)
    }
    pub fn acknowledged_end(&self)->bool {self.report.lock().is_ok_and(|report|report["status"]=="ended")&&self.journal.lock().is_ok_and(|journal|journal.terminal&&journal.pending.is_none())}
    pub fn matches(&self,profile_id:&str,context:&Context,_output:&OutputConfig)->bool {
        self.core.profile.id==profile_id&&self.context.zone_id==context.zone_id&&self.context.endpoint_id==context.endpoint_id

    }
    pub fn alias(&self,id:String)->CommandResult<()> {*self.alias.lock().map_err(err)?=id;Ok(())}
    pub fn position(&self)->f64 {self.sequence.position(&self.control).map(|p|p.position).unwrap_or(0.)}
    pub fn current_id(&self)->Option<String> {self.sequence.position(&self.control).ok().map(|p|p.playback_id)}
    pub fn recovery(&self)->recovery::Recovery {
        recovery::Recovery {journal:self.journal.clone(),sequence:self.sequence.clone(),control:self.control.clone(),entries:self.entries.clone(),context:self.context.clone(),plan_id:self.plan_id.clone(),gate:self.recovery_gate.clone()}
    }
    pub fn status(&self)->Value {
        let mut status=self.report.lock().map(|value|value.clone()).unwrap_or_else(|_|json!({"status":"error","error":"播放状态锁不可用"}));
        if let Ok(position)=self.sequence.position(&self.control) {
            status["core_playback_id"]=json!(position.playback_id);status["position"]=json!(position.position);status["frames_played"]=json!(position.frames);
            if let Ok(entries)=self.entries.lock(){if let Some(entry)=entries.get(&position.playback_id){
                status["track_id"]=json!(entry.track);
                if let Some(source)=&entry.source {status["cache_id"]=json!(source.cache_id);status["duration"]=json!(source.metadata.duration);}
            }}
        }
        status["playback_id"]=json!(self.alias.lock().ok().map(|alias|alias.clone()));status["sequence_id"]=json!(self.id);
        status["output_config"]=json!(self.output);
        status["measured_version"]=json!(self.measured_version);
        status["core_managed"]=json!(true);status["zone_id"]=json!(self.context.zone_id);status["offline"]=json!(false);status["exclusive"]=json!(self.output.exclusive);
        if self.control.stopped.load(Ordering::Acquire)&&!matches!(status["status"].as_str(),Some("error"|"ended")){status["status"]=json!("stopped");}
        status["paused"]=json!(self.control.paused.load(Ordering::Relaxed));status["volume"]=json!(self.control.volume_setting());status["muted"]=json!(self.control.muted.load(Ordering::Relaxed));status
    }
    pub fn cancel(&self) {
        if !self.journal.lock().is_ok_and(|journal|journal.terminal){crate::cache_tail::cancel(&self.metadata_tasks);}
        self.control.stop();let _=self.cancelled.send(true);self.sequence.cancel_next();self.task.abort();
    }
    pub fn finish_as(&self,reason:&str) {
        let code=match reason {"stop"=>1,"replaced"=>2,"replay"=>3,_=>0};
        self.finish_reason.store(code,Ordering::Release);
        if code!=0 {if let Some(cursor)=&self.cursor {let _=cursor.retire();}}
    }
    pub fn take_thread(&mut self)->Option<std::thread::JoinHandle<()>> {self.thread.take()}
}
fn verify_source(preparation:&Preparation,source:Arc<Source>) {
    if cfg!(target_os="android"){return;}
    if preparation.cache.lock().ok().is_some_and(|cache|cache.manifest.entries.get(&source.cache_id).is_some_and(|entry|entry.complete&&entry.verified)){return;}
    let complete=preparation.cache.lock().ok().is_some_and(|cache|cache.manifest.entries.get(&source.cache_id).is_some_and(|entry|entry.complete&&!entry.verified));
    if complete {let cache=preparation.cache.clone();let app=preparation.app.clone();
        tokio::task::spawn_blocking(move || {if let Err(error)=upgrade::verify(&cache,&source.cache_id){let _=app.emit("native-cache-error",error.to_string());}});}
    else if source.metadata.is_dsd()&&!source.metadata_started.swap(true,Ordering::AcqRel) {
        let cache=preparation.cache.clone();let app=preparation.app.clone();let profile=preparation.profile.clone();
        let id=source.cache_id.clone();let track_id=source.track.id.clone();let token=preparation.token.clone();let lease=source._lease.clone();
        crate::cache_tail::spawn(&preparation.metadata_tasks,preparation.metadata_generation,async move {
            let result=async {
                // Long songs may outlive the original media ticket. Resolve identity before authenticating again.
                let profile=profile.resolve().await?;
                let url=media_ticket(&profile,&token,&track_id).await?;
                crate::cache_tail::complete_metadata(cache.clone(),id.clone(),profile,url).await
            }.await;
            match result {
                Ok(true)=>{tokio::task::spawn_blocking(move||{let _lease=lease;if let Err(error)=upgrade::verify(&cache,&id){let _=app.emit("native-cache-error",error.to_string());}});},
                Err(error)=>{let _=app.emit("native-cache-error",error.to_string());},
                Ok(false)=>{},
            }
        });
    }
}
impl Drop for Player {fn drop(&mut self){self.cancel();}}

pub(super) struct Start {
    pub app:tauri::AppHandle,pub cache:Arc<Mutex<CacheStore>>,pub profile_id:String,pub token:String,
    pub core:Config,pub context:Context,pub initial:Initial,pub source:Arc<Source>,pub request:PlaybackRequest,
    pub output:OutputConfig,pub control:Arc<PlaybackControl>,pub playback_id:String,
    pub epoch:u64,
    pub listens:Arc<ListenStore>,pub owner:Option<ListenOwner>,
}
enum Update {Progress(PlaybackUpdate),Finished(Result<()>,SequencePosition),Refresh,#[cfg(target_os="android")] System(super::android::MediaAction)}
fn boundary(id:&str,position:f64,frames:u64,entries:&Entries)->Result<Value> {
    let entries=entries.lock().map_err(|_|anyhow::anyhow!("source lock poisoned"))?;
    let entry=entries.get(id).context("unknown playback source")?;
    Ok(json!({"playback_id":id,"track_id":entry.track,"position":position,"frames":frames}))
}
#[derive(Clone)]
struct Preparation {
    control:Arc<PlaybackControl>,
    app:tauri::AppHandle,cache:Arc<Mutex<CacheStore>>,profile_id:String,owner:Option<ListenOwner>,token:String,profile:ConnectionProfile,
    entries:Entries,known:Arc<Mutex<VecDeque<String>>>,template:PlaybackRequest,cancelled:watch::Receiver<bool>,
    metadata_tasks:crate::cache_tail::MetadataTasks,
    metadata_generation:u64,
    output:OutputConfig,
}
impl Preparation {
    fn stage(&self,sequence:&PlaybackSequence,items:Vec<Item>)->Result<bool> {
        for item in items {
            if self.entries.lock().map_err(|_|anyhow::anyhow!("source lock poisoned"))?.contains_key(&item.playback_id){continue;}
            // Prepare only the immediate successor. Further streams would be discarded
            // at a native-rate boundary and compete with the currently audible song.
            if cfg!(target_os="android")&&self.entries.lock().map_err(|_|anyhow::anyhow!("source lock poisoned"))?.len()>=2{return Ok(false);}
            let reservation=match sequence.reserve(){Ok(slot)=>slot,Err(_)=>return Ok(false)};
            let track:NativeTrack=serde_json::from_value(item.track)?;
            self.entries.lock().map_err(|_|anyhow::anyhow!("source lock poisoned"))?.insert(item.playback_id.clone(),Entry{track:track.id.clone(),source:None,request:None,listen_session_id:item.listen_session_id});
            {let mut known=self.known.lock().map_err(|_|anyhow::anyhow!("source identity lock poisoned"))?;known.push_back(item.playback_id.clone());while known.len()>128{known.pop_front();}}
            let mut preparation=self.clone();
            tauri::async_runtime::spawn(async move {
                let id=item.playback_id;let mut reservation=Some(reservation);
                let result=tokio::select! {
                    result=async {
                        #[cfg(target_os="android")]
                        while preparation.control.frames_played.load(Ordering::Relaxed)==0 {
                            if preparation.control.stopped.load(Ordering::Acquire){return Err(output::PreloadCancelled.into());}
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        let source=Arc::new(prepare_source(preparation.app.clone(),preparation.cache.clone(),preparation.profile.clone(),preparation.token.clone(),preparation.profile_id.clone(),preparation.owner.clone(),track,&preparation.output,true,false).await.map_err(anyhow::Error::msg)?);
                        let mut request=preparation.template.clone();request.input=source.proxy.url.clone();request.position=0.;request.sample_rate=source.metadata.output_rate_for(if reservation.as_ref().context("missing preload reservation")?.native_dsd(){output::DsdOutput::Native}else{output::DsdOutput::Dop});
                        request.channels=source.metadata.channels;request.source_bit_depth=source.metadata.bit_depth;request.source_integer_lossless=source.metadata.source_integer_lossless;
                        request.duration=source.metadata.duration;request.measured_lufs=source.track.integrated_lufs;
                        {let mut entries=preparation.entries.lock().map_err(|_|anyhow::anyhow!("source lock poisoned"))?;
                            if let Some(entry)=entries.get_mut(&id){entry.source=Some(source.clone());entry.request=Some(request.clone());}}
                        let proxy=source.proxy.clone();let duration=request.duration;let slot=reservation.take().unwrap();let decoder_id=id.clone();
                        let result=tokio::task::spawn_blocking(move ||slot.checked(decoder_id,request,Some(Arc::new(move |position|proxy.finish_playback(Ok(()),position,duration))))).await?;
                        #[cfg(target_os="android")] if result.as_ref().err().is_some_and(|e|e.is::<output::PreloadFormatChange>()){source.proxy.prefetch_start().await?;}
                        result?;
                        Ok::<_,anyhow::Error>(())
                    }=>Some(result),
                    _=preparation.cancelled.changed()=>None
                };
                if let Some(Err(error))=result {if let Some(slot)=reservation.take(){slot.fail();}
                    if !*preparation.cancelled.borrow()&&!error.is::<output::PreloadFormatChange>(){let _=preparation.app.emit("native-cache-error",format!("下一首预载失败：{error}"));}}
            });
        }Ok(true)
    }
}
pub(super) fn start(start:Start)->CommandResult<NativePlayer> {
    let Start{app,cache,profile_id,token,core,context,initial,source,request,output,control,playback_id,epoch,listens,owner}=start;
    initial.validate_timing().map_err(err)?;let measured_version=initial.measured_version;
    if measured_version==2&&request.is_dsd(){return Err("双时间计时仅支持 PCM".into());}
    let recovery_enabled=initial.continuous_recovery;
    let proxy=source.proxy.clone();let duration=request.duration;
    let sequence=PlaybackSequence::checked(context.playback_id.clone(),request.clone(),SequenceFormat {sample_rate:request.sample_rate,channels:request.channels,channel_mask:0,exclusive:output.exclusive,float_output:request.dsp.enabled,bit_depth:if request.is_dsd()&&output.dsd_output==output::DsdOutput::Native{1}else{32}},
        Some(Arc::new(move |position|proxy.finish_playback(Ok(()),position,duration)))).map_err(err)?;
    let entries=Arc::new(Mutex::new(HashMap::from([(context.playback_id.clone(),Entry{track:source.track.id.clone(),source:Some(source.clone()),request:Some(request.clone()),listen_session_id:initial.listen_session_id.clone()})])));
    let known=Arc::new(Mutex::new(VecDeque::from([context.playback_id.clone()])));
    let report=Arc::new(Mutex::new(json!({"status":"preparing"})));let alias=Arc::new(Mutex::new(playback_id.clone()));let id=uuid::Uuid::new_v4().to_string();
    let journal=recovery::Journal::new_timed(boundary(&context.playback_id,request.position,0,&entries).map_err(err)?,measured_version).map_err(err)?;
    let mut cursor=if recovery_enabled {owner.clone().map(|owner|rmusic_bridge::listens::cursor::Recorder::begin_timed(listens.clone(),owner,context.endpoint_id.clone(),context.zone_id.clone(),initial.plan_id.clone(),request.sample_rate,
        serde_json::from_value(boundary(&context.playback_id,request.position,0,&entries)?)?,measured_version)).transpose().map_err(err)?}else{None};
    let cursor_handle=cursor.as_ref().map(|cursor|cursor.handle.clone());
    let finish_reason=Arc::new(std::sync::atomic::AtomicU8::new(0));let worker_reason=finish_reason.clone();
    let emit_report=report.clone();#[cfg(not(target_os="android"))] let emitter=app.clone();let emitter_alias=alias.clone();let emitter_id=id.clone();
    let publish:Publish=Arc::new(move |mut event| {
        event["playback_id"]=json!(emitter_alias.lock().ok().map(|alias|alias.clone()));event["core_managed"]=json!(true);event["sequence_id"]=json!(emitter_id);
        if let Ok(mut report)=emit_report.lock(){*report=event.clone();}
        #[cfg(not(target_os="android"))] let _=emitter.emit("native-playback",event);
    });
    let (cancelled,cancellation)=watch::channel(false);
    let driver_app=app.clone();
    let metadata_tasks=driver_app.state::<DesktopState>().metadata_tasks.clone();
    let metadata_generation=metadata_tasks.lock().map_err(err)?.generation;
    let preparation=Preparation {control:control.clone(),app,cache,profile_id,owner:owner.clone(),token,profile:core.profile.clone(),entries:entries.clone(),known:known.clone(),template:request.clone(),cancelled:cancellation.clone(),metadata_tasks:metadata_tasks.clone(),metadata_generation,output:output.clone()};
    let startup=preparation.clone();let items=initial.items.clone();let more=initial.has_more;
    sequence.on_configured(move |sequence| {let all= startup.stage(&sequence,items)?;sequence.queue_complete(!more&&all)}).map_err(err)?;
    let (updates,update_rx)=mpsc::unbounded_channel();
    let actor=Actor {measured_version,core:core.clone(),context:context.clone(),plan_id:initial.plan_id.clone(),confirmed:context.playback_id.clone(),sequence:sequence.clone(),entries:entries.clone(),control:control.clone(),
        preparation,publish:publish.clone(),cancel_sender:cancelled.clone(),cancelled:cancellation,updates:update_rx,progress_errors:0,#[cfg(target_os="android")] route_retry:None,journal:journal.clone(),cursor:cursor_handle.clone(),refresh:initial.refresh};
    #[cfg(target_os="android")] {
        let metadata_entries=entries.clone();let commands=updates.clone();let reason=finish_reason.clone();
        super::android::bind_sequence(&driver_app,&control,super::android::SequenceMedia {sequence:sequence.clone(),
            metadata:Arc::new(move|id| {
                let entries=metadata_entries.lock().map_err(|_|anyhow::anyhow!("source metadata lock unavailable"))?;
                let source=entries.get(id).and_then(|entry|entry.source.as_ref()).context("source metadata unavailable")?;
                Ok(super::android::Media{track_id:source.track.id.clone(),playback_id:id.into(),title:source.track.title.clone().unwrap_or_else(||source.track.id.clone()),artist:source.track.artist.clone().unwrap_or_default(),duration:source.metadata.duration.unwrap_or(0.)})
            }),action:Arc::new(move|mut update,action| {
                update.ended.clear();if action.action=="stop" {reason.store(1,Ordering::Release);}
                let _=commands.send(Update::Progress(update));let _=commands.send(Update::System(action));
            })}).map_err(err)?;
    }
    let task=tokio::spawn(actor.run());
    let driver_sequence=sequence.clone();let driver_control=control.clone();let driver_request=request.clone();let driver_output=output.clone();let driver_entries=entries.clone();
    let driver_journal=journal.clone();
    let mut sampler=owner.clone().map(|owner|listens::Sampler::new(listens,owner,measured_version));
    let thread=std::thread::spawn(move || {
        let mut last=Instant::now()-Duration::from_secs(1);
        let result=(||->Result<()> {
        if let Some(cursor)=&cursor {if !cursor.handle.ready()? {anyhow::bail!("播放游标已撤销");}}
        eprintln!("Native continuous output worker started");
        output::play_sequence(driver_request,driver_output,driver_control.clone(),driver_sequence.clone(),|mut update| {
            let rate=driver_sequence.format().map(|format|format.sample_rate).unwrap_or(0);
            if let Some(sampler)=sampler.as_mut() {
                if let Err(error)=sampler.update(&update,rate,&driver_entries){let _=driver_app.emit("native-cache-error",format!("连续播放统计保存失败：{error}"));}
            }
            if let Some(cursor)=cursor.as_mut() {if let Err(error)=cursor::update(cursor,&update,rate,&driver_entries,||Ok(sampler.as_mut().map(|sampler|sampler.checkpoint_rows(&update.playback_id)).unwrap_or_default())) {
                let _=driver_app.emit("native-cache-error",format!("播放游标保存失败：{error}"));
            }}
            if driver_control.stopped.load(Ordering::Acquire){return;}
            if !update.ended.is_empty()||last.elapsed()>=Duration::from_millis(250) {
                if let Ok(entries)=driver_entries.lock(){if let Some(source)=entries.get(&update.playback_id).and_then(|entry|entry.source.as_ref()){source.proxy.annotate(&mut update.signal_path);}}
                if let Ok(current)=boundary(&update.playback_id,update.position,update.output_frames,&driver_entries) {
                        if let Ok(mut journal)=driver_journal.lock(){
                            for ended in &update.ended {if let Ok(value)=boundary(&ended.playback_id,ended.position,ended.frames,&driver_entries){let _=journal.observe(value);}}
                            let _=journal.observe(current.clone());
                        }
                        publish(json!({"status":"playing","track_id":current["track_id"],"core_playback_id":update.playback_id,"position":update.position,
                            "paused":driver_control.paused.load(Ordering::Relaxed),"signal_path":update.signal_path,"meter":update.meter,"continuity_lost":update.continuity_lost}));
                }
                let _=updates.send(Update::Progress(update));last=Instant::now();
            }
        })})();
        if let Some(sampler)=sampler.as_mut() {
            let position=driver_sequence.position(&driver_control).ok();let rate=driver_sequence.format().map(|format|format.sample_rate).unwrap_or(0);
            let reason=match worker_reason.load(Ordering::Acquire) {1=>"stop",2=>"replaced",3=>"replay",_=>if result.is_ok()&&!driver_control.stopped.load(Ordering::Acquire){"ended"}else{"interrupted"}};
            if let Err(error)=sampler.stop(position.as_ref(),rate,&driver_entries,reason){let _=driver_app.emit("native-cache-error",format!("连续播放统计保存失败：{error}"));}
        }
        if let (Some(cursor),Ok(position))=(cursor.as_mut(),driver_sequence.position(&driver_control)) {
            let rate=driver_sequence.format().map(|format|format.sample_rate).unwrap_or(0);
            let rows=sampler.as_mut().map(|sampler|sampler.checkpoint_rows(&position.playback_id)).unwrap_or_default();
            if let Err(error)=cursor::finish(cursor,&position,rate,&driver_entries,result.is_ok()&&!driver_control.stopped.load(Ordering::Acquire),rows) {let _=driver_app.emit("native-cache-error",format!("播放游标保存失败：{error}"));}
        }
        if !driver_control.stopped.load(Ordering::Acquire) {if let Ok(position)=driver_sequence.position(&driver_control) {
            if let Ok(mut journal)=driver_journal.lock(){
                if let Ok(value)=boundary(&position.playback_id,position.position,position.frames,&driver_entries){let _=journal.observe(value);}
                journal.terminal=result.is_ok();
            }
            let _=updates.send(Update::Finished(result,position));
        }}
    });
    visualization::start(core.clone(),context.zone_id.clone(),sequence.clone(),control.clone(),entries.clone());
    Ok(NativePlayer {retired:false,track_id:source.track.id.clone(),playback_id,control:control.clone(),request,output:output.clone(),proxy:Some(source.proxy.clone()),local_file:None,thread:None,
        cache_id:Some(source.cache_id.clone()),report:report.clone(),listen:None,
        sequence:Some(Player {measured_version,epoch,core,context,plan_id:initial.plan_id,owner,cursor:cursor_handle,finish_reason,id,alias,sequence,control,entries,known,report,journal,metadata_tasks,recovery_gate:Default::default(),output,thread:Some(thread),task,cancelled})})
}
pub(super) fn command_error(error:anyhow::Error)->String {
    if let Some(status)=error.downcast_ref::<reqwest::Error>().and_then(|error|error.status()){return format!("HTTP {}: {error}",status.as_u16());}
    error.to_string()
}
struct Actor {
    measured_version:u32,
    refresh:bool,
    core:Config,context:Context,plan_id:String,confirmed:String,sequence:PlaybackSequence,entries:Entries,control:Arc<PlaybackControl>,preparation:Preparation,publish:Publish,
    cancel_sender:watch::Sender<bool>,cancelled:watch::Receiver<bool>,updates:mpsc::UnboundedReceiver<Update>,progress_errors:u32,
    #[cfg(target_os="android")] route_retry:Option<std::time::Instant>,
    journal:recovery::Shared,
    cursor:Option<rmusic_bridge::listens::cursor::Handle>,
}
impl Actor {
    #[cfg(target_os="android")]
    fn continue_at_boundary(&self,zone:&Value) {
        if zone["id"]!=self.context.zone_id||zone["endpoint_id"]!=self.context.endpoint_id||!matches!(zone["status"].as_str(),Some("playing"|"paused")){return;}
        let Ok(track)=serde_json::from_value::<NativeTrack>(zone["track"].clone()) else{return};
        let Some(id)=zone["playback_id"].as_str().filter(|id|*id!=self.confirmed) else{return};
        let app=self.preparation.app.clone();let profile=self.preparation.profile_id.clone();let output=self.preparation.output.clone();let control=self.control.clone();let zone=zone.clone();let id=id.to_owned();
        // A native boundary must keep progressing when Android throttles the WebView.
        // Run outside this actor: retiring the old player aborts its actor task.
        std::thread::spawn(move||tauri::async_runtime::block_on(async move {
            let state=app.state::<DesktopState>();
            let current=state.player.lock().ok().is_some_and(|player|player.as_ref().and_then(|p|p.sequence.as_ref()).is_some_and(|p|
                Arc::ptr_eq(&p.control,&control)&&p.epoch==state.playback_generation.load(Ordering::Acquire)&&p.prepared_source(&id,&track.id,&output).is_some()));
            if !current{return;}
            let Ok(native_output)=serde_json::from_value(serde_json::to_value(&output).unwrap_or(Value::Null)) else{return};
            let context=Context{zone_id:zone["id"].as_str().unwrap().into(),endpoint_id:zone["endpoint_id"].as_str().unwrap().into(),playback_id:id};
            let _=native_play(None,Some(uuid::Uuid::new_v4().to_string()),Some(context.clone()),profile,track,zone["position"].as_f64(),
                serde_json::from_value(zone["dsp"].clone()).ok(),Some(native_output),zone["volume"].as_f64(),zone["muted"].as_bool(),Some(zone["status"]!="playing"),
                serde_json::from_value(zone["transitions"].clone()).ok(),Some(context),app.clone(),state).await;
        }));
    }
    fn stop(&self){crate::cache_tail::cancel(&self.preparation.metadata_tasks);self.control.stop();self.sequence.cancel_next();let _=self.cancel_sender.send(true);}
    #[cfg(target_os="android")]
    async fn refresh_route(&mut self)->Result<()> {
        let state=self.preparation.app.state::<DesktopState>();
        let profile=resolve_connection(&state,&self.preparation.profile_id).await?;
        let current=token(&state,&self.preparation.profile_id).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(self.core.same_listener(&profile,&current,&self.context.endpoint_id),"播放账号或 Core 已切换");
        if !self.core.same_connection(&profile,&current,&self.context.endpoint_id) {
            self.core=Config::new(profile.clone(),current,self.context.endpoint_id.clone())?;
            self.preparation.profile=profile;
        }
        Ok(())
    }
    async fn progress(&mut self,update:PlaybackUpdate)->Result<()> {
        let rate=self.sequence.format()?.sample_rate;let frames=update.output_frames;
        let current=boundary(&update.playback_id,update.position,frames,&self.entries)?;
        let ended:Vec<_>=update.ended.into_iter().filter(|item|item.playback_id!=update.playback_id).collect();
        if !ended.is_empty() {
            let ended_values=ended.iter().map(|item|boundary(&item.playback_id,item.position,item.frames,&self.entries)).collect::<Result<Vec<_>>>()?;
            let mut body=json!({"endpoint_id":self.context.endpoint_id,"request_id":uuid::Uuid::new_v4().to_string(),
                "plan_id":self.plan_id,"from_playback_id":self.confirmed,"sample_rate":rate,"ended":ended_values,"current":current});
            if self.measured_version==2{body["measured_version"]=json!(2);}
            self.journal.lock().map_err(|_|anyhow::anyhow!("playback recovery lock poisoned"))?.begin("handoff",body.clone());
            let response=self.core.commit(&self.context.zone_id,&body).await?;
            self.journal.lock().map_err(|_|anyhow::anyhow!("playback recovery lock poisoned"))?.acknowledge(&response);
            if response["accepted"]!=true||response["zone"]["playback_id"]!=update.playback_id {if let Some(cursor)=&self.cursor {let _=cursor.retire();}self.stop();return Ok(());}
            self.confirmed=update.playback_id.clone();
            for item in &ended {
                let source=self.entries.lock().ok().and_then(|mut entries|entries.remove(&item.playback_id)).and_then(|entry|entry.source);
                if let Some(source)=source {verify_source(&self.preparation,source);}
            }
            self.refill().await?;
        }
        if self.confirmed==update.playback_id {
            let mut body=json!({"track_id":current["track_id"],"playback_id":update.playback_id,"position":update.position,
                "frames":frames,"sample_rate":rate});if self.measured_version==2{body["measured_version"]=json!(2);}
            let response=async {
                let response=self.core.post(&self.context.zone_id,"progress",&body).await?;
                if let Err(error)=response.error_for_status_ref(){
                    let reason=response.json::<Value>().await.unwrap_or(Value::Null);
                    return Err(anyhow::Error::new(error).context(reason["error"]["message"].as_str().unwrap_or("播放进度同步失败").to_owned()));
                }
                Ok(())
            }.await;
            match response {Ok(_)=>self.progress_errors=0,Err(error)=>{
                self.progress_errors+=1;
                if self.progress_errors>=3 || error.downcast_ref::<reqwest::Error>().and_then(|error|error.status()).is_some_and(|status|status.is_client_error()){return Err(error);}
            }}
        }Ok(())
    }
    async fn refill(&mut self)->Result<()> {
        if self.sequence.format().is_err(){return Ok(());}
        let track=self.entries.lock().map_err(|_|anyhow::anyhow!("source lock poisoned"))?.get(&self.confirmed).map(|entry|entry.track.clone()).context("missing current source")?;
        let initial=tokio::select! {_=self.cancelled.changed()=>return Ok(()),result=self.core.prepare_timed(&self.context.zone_id,&track,&self.confirmed,self.measured_version)=>result?};
        let Some(initial)=initial else{self.stop();return Ok(());};
        if initial.measured_version!=self.measured_version||initial.plan_id!=self.plan_id {self.stop();return Ok(());}
        self.refresh=initial.refresh;let all=self.preparation.stage(&self.sequence,initial.items)?;let _=self.sequence.queue_complete(!initial.has_more&&all);Ok(())
    }
    async fn run(mut self) {
        let mut refresh=tokio::time::interval_at(tokio::time::Instant::now()+Duration::from_secs(1),Duration::from_secs(1));
        loop {
            let update=tokio::select! {_=self.cancelled.changed()=>break,update=self.updates.recv()=>update,_=refresh.tick(),if self.refresh=>Some(Update::Refresh)};let Some(update)=update else{break;};
            let terminal=matches!(&update,Update::Finished(..));
            let refilled=matches!(&update,Update::Refresh)||matches!(&update,Update::Progress(update) if !update.ended.is_empty());
            #[cfg(target_os="android")]
            let routed={
                let optional=matches!(&update,Update::Progress(p) if p.ended.is_empty()) || matches!(&update,Update::Refresh);
                if optional && self.route_retry.is_some_and(|attempt| attempt.elapsed()<Duration::from_secs(2) && !super::android::network_changed_since(attempt)) {continue;}
                let result=self.refresh_route().await;
                if optional && result.as_ref().err().is_some_and(|e| e.downcast_ref::<reqwest::Error>().is_some_and(|e| e.is_timeout()||e.is_connect()||e.status().is_some_and(|s|s.is_server_error()))) {
                    // The output worker still checkpoints the heard frames. Publish its next fresh update when a route returns.
                    self.route_retry=Some(Instant::now());continue;
                }
                self.route_retry=None;
                result
            };
            #[cfg(not(target_os="android"))]
            let routed:Result<()>=Ok(());
            let result=match (routed,update) {
                (Err(error),_)=>Err(error),
                (Ok(()),update)=>match update {
                Update::Progress(update)=>self.progress(update).await,
                Update::Refresh=>self.refill().await,
                #[cfg(target_os="android")]
                Update::System(action)=> {
                    if action.playback_id!=self.confirmed {Ok(())}else{
                        let track=self.entries.lock().ok().and_then(|entries|entries.get(&action.playback_id).map(|entry|entry.track.clone()));
                        if let Some(track)=track {
                            let result=self.core.post(&self.context.zone_id,&action.action,&json!({"control":{"endpoint_id":self.context.endpoint_id,"track_id":track,"playback_id":action.playback_id},"position":action.position})).await
                                .and_then(|reply|reply.error_for_status().map_err(anyhow::Error::from));
                            if action.action=="stop" {self.stop();}
                            result.map(|_|())
                        }else{Ok(())}
                    }
                },
                Update::Finished(result,position)=>match result {
                    Ok(()) if position.playback_id==self.confirmed=>{
                        let rate=self.sequence.format().map(|format|format.sample_rate).unwrap_or(0);
                        match boundary(&position.playback_id,position.position,position.frames,&self.entries) {
                            Ok(mut body)=>{body["sample_rate"]=json!(rate);if self.measured_version==2{body["measured_version"]=json!(2);}
                                if let Ok(mut journal)=self.journal.lock(){journal.begin("ended",body.clone());}
                                match self.core.request_json(&self.context.zone_id,"ended",&body).await {
                                    Ok(response)=>{if let Some(cursor)=&self.cursor {let _=cursor.retire();}if let Ok(mut journal)=self.journal.lock(){journal.acknowledge(&response);}if let Some(source)=self.entries.lock().ok().and_then(|entries|entries.get(&position.playback_id).and_then(|entry|entry.source.clone())){verify_source(&self.preparation,source);}
                                        (self.publish)(json!({"status":"ended","track_id":body["track_id"],"core_playback_id":position.playback_id,"position":position.position,"paused":self.control.paused.load(Ordering::Relaxed)}));
                                        #[cfg(target_os="android")] self.continue_at_boundary(&response);
                                        Ok(())},Err(error)=>Err(error)
                                }
                            },Err(error)=>Err(error)
                        }
                    },Ok(())=>Err(anyhow::anyhow!("核心尚未确认实际跨曲")),Err(error)=>Err(error)
                }
            }};
            // Preparing after a handoff starts a fresh interval; do not replay idle ticks.
            if refilled {refresh.reset();}
            if let Err(error)=result {
                let device_failed=output::is_device_unavailable(&error);
                #[cfg(not(windows))] let system_suspend=Value::Null;
                #[cfg(windows)] let system_suspend=if device_failed {tokio::task::spawn_blocking(output::power::held_stamp).await.ok().and_then(|v|v.ok()).flatten().map_or(Value::Null,|v|json!(v))}else{Value::Null};
                self.stop();let position=self.sequence.position(&self.control).ok();
                let current=position.as_ref().and_then(|position|boundary(&position.playback_id,position.position,position.frames,&self.entries).ok());
                let source_failed=position.as_ref().and_then(|p|self.entries.lock().ok().and_then(|entries|entries.get(&p.playback_id).and_then(|entry|entry.source.as_ref()).map(|source|source.proxy.source_failed()))).unwrap_or(false);
                let http_status=error.downcast_ref::<reqwest::Error>().and_then(|error|error.status()).map(|status|status.as_u16());
                if !device_failed&&!error.downcast_ref::<reqwest::Error>().is_some_and(|error|!error.status().is_some_and(|status|status.is_client_error()&&!matches!(status.as_u16(),401|403|404|409))) {if let Some(cursor)=&self.cursor {let _=cursor.retire();}}
                if device_failed&&self.cursor.is_some() {
                    let track=self.entries.lock().ok().and_then(|entries|entries.get(&self.confirmed).map(|entry|entry.track.clone()));
                    if let Some(track)=track {let _=self.core.post(&self.context.zone_id,"progress",&json!({"track_id":track,"playback_id":self.confirmed,"recoverable":true,"device_failed":true,"system_suspend":system_suspend})).await;}
                }
                let recovery=self.journal.lock().ok().map(|journal|journal.reports());
                (self.publish)(json!({"status":"error","track_id":current.as_ref().map(|current|&current["track_id"]),"core_playback_id":position.as_ref().map(|p|&p.playback_id),
                    "position":position.map(|p|p.position),"error":error.to_string(),"device_failed":device_failed,"source_failed":source_failed,"connection_failed":error.downcast_ref::<reqwest::Error>().is_some(),"http_status":http_status,"recovery":recovery}));break;
            }
            if terminal||(self.control.stopped.load(Ordering::Acquire)&&!self.control.system_interrupted()){break;}
        }
        let _=self.cancel_sender.send(true);
    }
}
