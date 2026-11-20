use anyhow::{Context, Result, bail};
use rmusic_audio::{Meter, output::{self, DeviceInfo, OutputConfig, PlaybackControl, PlaybackRequest, SignalPath}, probe::AudioMetadata};
use serde_json::{Value, json};
use std::{path::Path, sync::{Arc, Mutex, Weak, atomic::Ordering}, time::Duration};
use tauri::{Manager, Wry, ipc::{Channel,InvokeResponseBody}, plugin::{Builder, PluginHandle, TauriPlugin}};
mod jni_controls;
pub(super) fn storage_call(command:&str,args:Value)->Result<Value>{jni_controls::call(command,args)}
pub(super) async fn choose_storage(app:&tauri::AppHandle)->Result<Value>{let handle=app.state::<FileExportHandle>().0.clone();Ok(handle.run_mobile_plugin_async::<Value>("chooseOfflineDirectory",json!({})).await?)}
pub(super) fn remote_media(state:Option<&Value>)->Result<()> {jni_controls::call("remote_media",json!({"state":state}))?;Ok(())}
pub(super) fn device_name()->Result<String>{jni_controls::device_name()}
pub(super) fn discovery()->Result<Value>{jni_controls::call("discovery",json!({}))}
pub(super) fn quality(values:Option<Value>)->Result<Value>{jni_controls::call("quality",json!({"values":values}))}
pub(super) fn download_notice(value:Value)->Result<Value>{jni_controls::call("download_notice",value)}
pub(super) fn session(action:&str,profile:&str,core:Option<&str>,token:Option<&str>)->Result<Value>{jni_controls::call("session",json!({"action":action,"profile_id":profile,"core_id":core,"token":token}))}
pub(super) fn source_quality(device:Option<&str>)->Result<Value>{jni_controls::call("quality",json!({"device_id":device}))}
mod decoder;
mod artwork;
pub(super) use artwork::update as update_artwork;
mod lifecycle;
pub(crate) use lifecycle::{network_changed_since,foreground};
mod pcm_output;
pub(super) use output::android::{Media,MediaAction,SequenceMedia};
#[derive(Default)] struct SequenceBindings(Mutex<std::collections::HashMap<usize,(Weak<PlaybackControl>,SequenceMedia)>>);
pub(super) fn bind_sequence(app:&tauri::AppHandle,control:&Arc<PlaybackControl>,media:SequenceMedia)->Result<()> {
    let state=app.state::<SequenceBindings>();let mut bindings=state.0.lock().map_err(|_|anyhow::anyhow!("Android sequence media lock unavailable"))?;
    bindings.retain(|_,(owner,_)|owner.strong_count()>0);
    bindings.insert(Arc::as_ptr(control) as usize,(Arc::downgrade(control),media));Ok(())
}
pub(super) async fn validate_core(core:&rmusic_bridge::sequence::Config)->Result<Value> {
    let status:Value=core.profile.http()?.get(core.profile.endpoint("api/v1/status")?).timeout(Duration::from_secs(5)).send().await?.error_for_status()?.json().await?;
    if status["capabilities"]["guarded_transport_v1"]!=true {bail!("Android 系统媒体控制需要支持 guarded_transport_v1 的新版 Core，请使用配套开发 Core");}Ok(status)
}
#[cfg(debug_assertions)] mod decoder_audit;
#[cfg(debug_assertions)] mod lifecycle_audit;
#[cfg(debug_assertions)] mod queue_audit;
#[cfg(debug_assertions)] mod endpoint_audit;

#[derive(Clone)]
struct Binding {
    core:Option<(rmusic_bridge::sequence::Config,super::sequence::Context,String)>,
    track_id:String,title:String,artist:String,
}
#[derive(Default)]
struct Bindings(Mutex<Option<(Weak<PlaybackControl>,Binding)>>);
pub(super) fn bind(app:&tauri::AppHandle,control:&Arc<PlaybackControl>,core:Option<(rmusic_bridge::sequence::Config,super::sequence::Context,String)>,track_id:String,title:String,artist:String)->Result<()> {
    *app.state::<Bindings>().0.lock().map_err(|_|anyhow::anyhow!("Android media binding lock unavailable"))?=Some((Arc::downgrade(control),Binding{core,track_id,title,artist}));Ok(())
}
type Updates=Arc<Mutex<Option<(String,tokio::sync::watch::Sender<Value>)>>>;
struct AndroidAudio(PluginHandle<Wry>,tauri::AppHandle,Updates,Channel<Value>);
impl AndroidAudio {
    fn call(&self, command:&'static str, args:Value)->Result<Value> {
        if matches!(command,"controls"|"stop") {
            return jni_controls::call(command,args).map_err(|error|output::DeviceUnavailable(format!("Android 原生控制失败：{error}")).into());
        }
        let handle=self.0.clone();
        // Keep the mobile response receiver alive after a deadline: Tauri's callback owns it.
        tauri::async_runtime::block_on(async move {
            let task=tauri::async_runtime::spawn(async move {handle.run_mobile_plugin_async::<Value>(command,args).await});
            Ok(tokio::time::timeout(Duration::from_secs(25),task).await.context("Android audio command timed out")??? )
        })
    }
}
impl output::android::Backend for AndroidAudio {
    fn bind_sequence(&self,control:&Arc<PlaybackControl>,media:SequenceMedia)->Result<()> {bind_sequence(&self.1,control,media)}
    fn transport(&self,id:&str,state:Option<Value>)->Result<Option<String>> {let value=jni_controls::call("transport",json!({"id":id,"state":state}))?;Ok(value["action"].as_str().map(String::from))}
    fn pcm_output(&self,request:&PlaybackRequest,config:&OutputConfig,control:&PlaybackControl)->Result<Box<dyn output::android::PcmOutput>> {pcm_output::open(self,request,config,control)}
    fn decoder(&self,request:&PlaybackRequest,format:output::SequenceFormat)->Result<Box<dyn output::android::PcmDecoder>> {decoder::open(request,format)}
    fn devices(&self)->Result<Vec<DeviceInfo>> {
        let rows=self.call("devices",json!({}))?;
        rows.as_array().context("Invalid Android devices")?.iter().map(|row|Ok(DeviceInfo {
            id:row["id"].as_str().context("Missing device ID")?.into(),
            name:row["name"].as_str().context("Missing device name")?.into(),
            default:false,mix_rate:0,channels:0,
        })).collect()
    }
    fn probe(&self,path:&Path)->Result<AudioMetadata> {
        if let Some(metadata)=decoder::probe_native(path)? {return Ok(metadata);}
        let mut metadata:AudioMetadata=serde_json::from_value(self.call("probe",json!({"input":path.to_string_lossy()}))?)?;
        metadata.source_integer_lossless=false;
        metadata.quality_rank=rmusic_audio::probe::quality_rank(&metadata.format,metadata.sample_rate,metadata.bit_depth,metadata.bitrate);
        Ok(metadata)
    }
    fn play(&self,request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,update:&mut dyn FnMut(SignalPath,Meter))->Result<()> {
        let id=uuid::Uuid::new_v4().to_string();
        let binding=self.1.state::<Bindings>().0.lock().ok().and_then(|mut saved| {
            if saved.as_ref().is_some_and(|(owner,_)|owner.upgrade().is_some_and(|owner|Arc::ptr_eq(&owner,&control))) {
                saved.take().map(|(_,binding)|binding)
            }else{None}
        });
        if binding.as_ref().is_none_or(|binding|binding.core.is_none()) {
            let format=output::SequenceFormat {sample_rate:request.sample_rate,channels:request.channels,channel_mask:0,
                exclusive:config.android_mode==Some(output::AndroidMode::UsbBitperfect),float_output:false,
                bit_depth:if config.android_mode==Some(output::AndroidMode::Compatibility){16}else{32}};
            let sequence=output::PlaybackSequence::new(id.clone(),request.clone(),format)?;
            let media=Media {track_id:binding.as_ref().map(|b|b.track_id.clone()).unwrap_or_default(),playback_id:id,title:binding.as_ref().map(|binding|binding.title.clone()).unwrap_or_else(||"R-Music".into()),
                artist:binding.as_ref().map(|binding|binding.artist.clone()).unwrap_or_default(),duration:request.duration.unwrap_or(0.)};
            let actions=control.clone();
            bind_sequence(&self.1,&control,SequenceMedia {sequence:sequence.clone(),metadata:Arc::new(move |_|Ok(media.clone())),
                action:Arc::new({let app=self.1.clone();move |_,action| {if app.state::<super::DesktopState>().travel.owns(&app.state::<super::DesktopState>().playback_controls.lock().unwrap().profile_id.clone().unwrap_or_default()){app.state::<super::DesktopState>().travel.system(action.action.clone(),Some(action.position));}else if action.action=="stop"{actions.stop();}else{actions.set_paused(action.action=="pause");}}})})?;
            return output::play_sequence(request,config,control,sequence,|event|update(event.signal_path,event.meter));
        }
        if request.dsp.enabled||request.transitions.enabled() {bail!("Android DSP／淡入淡出请使用连续 PCM 队列");}
        if let Some((core,_,_))=binding.as_ref().and_then(|binding|binding.core.as_ref()) {
            tauri::async_runtime::block_on(async {
                let status:Value=core.profile.http()?.get(core.profile.endpoint("api/v1/status")?).timeout(Duration::from_secs(5))
                    .send().await?.error_for_status()?.json().await?;
                if status["capabilities"]["guarded_transport_v1"]!=true {bail!("Android 系统媒体控制需要支持 guarded_transport_v1 的新版 Core，请使用配套开发 Core");}
                Ok::<_,anyhow::Error>(())
            })?;
        }
        let (send,receive)=tokio::sync::watch::channel(Value::Null);
        *self.2.lock().map_err(|_|anyhow::anyhow!("Android updates lock unavailable"))?=Some((id.clone(),send));
        let controls=||json!({"id":id,"paused":control.paused.load(Ordering::Acquire),"volume":control.volume_setting(),"muted":control.muted.load(Ordering::Acquire)});
        let mut last_controls=controls();
        let started=self.call("play",json!({"id":id,"input":request.input,"position":request.position,
            "sample_rate":request.sample_rate,"channels":request.channels,"bits":request.source_bit_depth,
            "lossless":request.source_integer_lossless,"mode":config.android_mode.unwrap_or_default(),
            "device":config.device_id,"buffer_ms":config.buffer_ms,"controls":last_controls,
            "duration":request.duration,"title":binding.as_ref().map(|b|b.title.as_str()).unwrap_or("R-Music"),
            "artist":binding.as_ref().map(|b|b.artist.as_str()).unwrap_or(""),"updates":self.3}));
        if let Err(error)=started {self.2.lock().ok().map(|mut saved|saved.take());return Err(error);}
        let mut media_revision=0;
        let mut terminal=false;
        let result=(|| {
            loop {
                let stopped=control.stopped.load(Ordering::Acquire);
                let current=controls();
                let state=if stopped {self.call("stop",json!({"id":id}))?}
                    else if current!=last_controls {last_controls=current.clone();self.call("controls",current)?}
                    else {receive.borrow().clone()};
                terminal=state["cancelled"]==true||state["ended"]==true;
                let rate=state["sample_rate"].as_u64().unwrap_or(0) as u32;
                let written=state["written"].as_u64().unwrap_or(0);
                let played=state["played"].as_u64().unwrap_or(0).min(written);
                control.output_sample_rate.store(rate,Ordering::Relaxed);
                control.frames_written.store(written,Ordering::Relaxed);
                control.frames_played.store(played,Ordering::Release);
                let revision=state["media_revision"].as_u64().unwrap_or(0);
                if revision>media_revision {
                    media_revision=revision;
                    let action=state["media_action"].as_str().unwrap_or("");
                    if action=="stop" {control.stop();} else {control.set_paused(action=="pause");}
                    last_controls=controls();
                    if let Some((core,context,track))=binding.as_ref().and_then(|binding|binding.core.as_ref()) {
                        let position=request.position+control.seconds(request.sample_rate);
                        tauri::async_runtime::block_on(async {
                            let zone:Value=core.post(&context.zone_id,action,&json!({"control":{"track_id":track,"playback_id":context.playback_id,
                                "endpoint_id":context.endpoint_id},"position":position})).await?.error_for_status()?.json().await?;
                            if action!="stop"&&(zone["playback_id"]!=context.playback_id||zone["track"]["id"]!=*track||zone["endpoint_id"]!=context.endpoint_id) {
                                bail!("核心播放实例已变化，请重新选择歌曲");
                            }
                            Ok::<_,anyhow::Error>(())
                        })?;
                    }
                }
                if rate>0 {
                    update(SignalPath {
                        output:state["output"].as_str().unwrap_or("Android AudioTrack").into(),sample_rate:rate,
                        channels:state["channels"].as_u64().unwrap_or(0) as u16,
                        exclusive:state["exclusive"].as_bool().unwrap_or(false),
                        bit_depth:state["bits"].as_u64().unwrap_or(0) as u16,
                        sample_type:state["sample_type"].as_str().unwrap_or("int").into(),mixer_sample_rate:None,
                        bit_perfect_eligible:state["eligible"].as_bool().unwrap_or(false),
                        processing:state["processing"].as_array().map(|rows|rows.iter().filter_map(|v|v.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
                    },Meter::default());
                }
                if let Some(error)=state["error"].as_str() {
                    if state["kind"]=="device" {return Err(output::DeviceUnavailable(error.into()).into());}
                    bail!("{error}");
                }
                if state["cancelled"]==true {control.stop();}
                if control.stopped.load(Ordering::Acquire)||state["ended"]==true {return Ok(());}
                std::thread::sleep(Duration::from_millis(50));
            }
        })();
        if result.is_err()&&!terminal {let _=self.call("stop",json!({"id":id}));}
        self.2.lock().ok().map(|mut saved| {if saved.as_ref().is_some_and(|(owner,_)|owner==&id){saved.take();}});
        let _=jni_controls::call("release",json!({"id":id}));result
    }
}
struct FileExportHandle(PluginHandle<Wry>);
pub(super) async fn save_download(app:&tauri::AppHandle,path:&Path,name:&str,mime:&str)->Result<String>{
 let handle=app.state::<FileExportHandle>().0.clone();let result=handle.run_mobile_plugin_async::<Value>("saveDownload",json!({"path":path,"name":name,"mime":mime})).await?;Ok(result["uri"].as_str().unwrap_or("Downloads/R-Music").to_owned())
}
pub fn init()->TauriPlugin<Wry> {
    Builder::new("android-audio").setup(|app,api| {
        lifecycle::install(app);
        let handle=api.register_android_plugin("com.rmusic.android.development","AudioPlugin")?;
        app.manage(FileExportHandle(handle.clone()));
        app.manage(Bindings::default());
        app.manage(SequenceBindings::default());
        let updates:Updates=Default::default();let target=updates.clone();
        // One app-lifetime native channel; late snapshots cannot reach a replacement worker.
        let channel=Channel::<Value>::new(move|body| {
            if let InvokeResponseBody::Json(body)=body {if let Ok(state)=serde_json::from_str::<Value>(&body) {
                if let Ok(saved)=target.lock(){if let Some((id,send))=saved.as_ref().filter(|(id,_)|state["id"]==*id){let _=id;send.send_replace(state);}}
            }}Ok(())
        });
        output::android::install(Arc::new(AndroidAudio(handle,app.clone(),updates,channel)))?;
        Ok(())
    }).build()
}
