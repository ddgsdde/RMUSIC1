use super::*;
use output::android::{PcmOutput,PcmState};
struct Sink {id:String,controls:Value,revision:u64,closed:bool,prefill:u64,media:Option<SequenceMedia>,pending:Option<MediaAction>,last_media:String,last_present:std::time::Instant,last_played:u64}
pub(super) fn open(backend:&AndroidAudio,request:&PlaybackRequest,config:&OutputConfig,control:&PlaybackControl)->Result<Box<dyn PcmOutput>> {
    jni_controls::attach_worker()?;
    let id=uuid::Uuid::new_v4().to_string();let controls=controls(&id,control);
    let media=backend.1.state::<SequenceBindings>().0.lock().map_err(|_|anyhow::anyhow!("Android media lock unavailable"))?.remove(&(control as *const PlaybackControl as usize))
        .filter(|(owner,_)|owner.upgrade().is_some_and(|owner|std::ptr::eq(owner.as_ref(),control))).map(|(_,media)|media);
    let initial=media.as_ref().map(|media|{let position=media.sequence.position(control)?;(media.metadata)(&position.playback_id)}).transpose()?;
    backend.call("play",json!({"id":id,"input":request.input,"position":request.position,"sample_rate":request.sample_rate,"channels":request.channels,
        "bits":request.source_bit_depth,"lossless":request.source_integer_lossless,"external_pcm":true,"mode":config.android_mode.unwrap_or_default(),"device":config.device_id,
        "buffer_ms":config.buffer_ms,"scheduled":control.start.is_scheduled(),"controls":controls,"title":initial.as_ref().map(|v|v.title.as_str()).unwrap_or("R-Music"),"artist":initial.as_ref().map(|v|v.artist.as_str()).unwrap_or(""),
        "track_id":initial.as_ref().map(|v|v.track_id.as_str()).unwrap_or(""),"media_id":initial.as_ref().map(|v|v.playback_id.as_str()).unwrap_or(&id),"duration":request.duration,"updates":backend.3}))?;
    let mut sink=Sink{id,controls,revision:0,closed:false,prefill:1,media,pending:None,last_media:String::new(),last_present:std::time::Instant::now(),last_played:0};
    let result=(||->Result<()> {let deadline=std::time::Instant::now()+Duration::from_secs(5);loop {
        let state=sink.call("status",json!({"id":sink.id}))?;let _=parse(&state)?;
        if state["prepared"]==true&&state["sample_rate"]==request.sample_rate&&state["channels"]==request.channels {sink.prefill=state["prefill_frames"].as_u64().unwrap_or(1).max(1);return Ok(());}
        if std::time::Instant::now()>deadline {bail!("Android PCM 初始化超时");}
        std::thread::sleep(Duration::from_millis(5));
    }})();
    if let Err(error)=result {let _=sink.close();return Err(error);}Ok(Box::new(sink))
}
fn controls(id:&str,control:&PlaybackControl)->Value {json!({"id":id,"paused":control.paused.load(Ordering::Acquire),"volume":control.volume_setting(),"muted":control.muted.load(Ordering::Acquire)})}
fn parse(state:&Value)->Result<PcmState> {
    if let Some(error)=state["error"].as_str(){return Err(output::DeviceUnavailable(error.into()).into());}
    Ok(PcmState{clock:state.get("clock").filter(|v|!v.is_null()).map(|v|serde_json::from_value(v.clone())).transpose()?,written:state["written"].as_u64().unwrap_or(0),played:state["played"].as_u64().unwrap_or(0),ended:state["ended"]==true,cancelled:state["cancelled"]==true,
        continuity_lost:state["continuity_lost"].as_bool().unwrap_or_else(||state["underruns"].as_u64().unwrap_or(0)>0),path:SignalPath{output:state["output"].as_str().unwrap_or("Android AudioTrack").into(),
        sample_rate:state["sample_rate"].as_u64().unwrap_or(0) as u32,channels:state["channels"].as_u64().unwrap_or(0) as u16,
        exclusive:state["exclusive"]==true,bit_depth:state["bits"].as_u64().unwrap_or(0) as u16,sample_type:state["sample_type"].as_str().unwrap_or("int").into(),mixer_sample_rate:None,
        bit_perfect_eligible:state["eligible"]==true,processing:state["processing"].as_array().map(|rows|rows.iter().filter_map(|v|v.as_str().map(str::to_owned)).collect()).unwrap_or_default()}})
}
impl Sink {fn call(&self,command:&str,args:Value)->Result<Value>{jni_controls::call(command,args)}}
impl PcmOutput for Sink {
    fn poll(&mut self,control:&PlaybackControl)->Result<PcmState> {
        let wanted=controls(&self.id,control);
        let state=if wanted!=self.controls {self.controls=wanted.clone();self.call("controls",wanted)?}else{self.call("status",json!({"id":self.id}))?};
        let revision=state["media_revision"].as_u64().unwrap_or(0);
        if revision>self.revision {
            self.revision=revision;let action=state["media_action"].as_str().unwrap_or("");
            let target=state["media_target"].as_str().unwrap_or("");
            control.frames_written.store(state["written"].as_u64().unwrap_or(0),Ordering::Relaxed);
            let valid=self.media.as_ref().map(|media|media.sequence.position_at_frame(control,state["played"].as_u64().unwrap_or(0)).is_ok_and(|position|position.playback_id==target)).unwrap_or(true);
            if valid {
                if self.media.is_some(){
                    control.set_paused(action!="play");
                    self.pending=Some(MediaAction{playback_id:target.into(),action:action.into(),position:state["media_position"].as_f64().unwrap_or(0.)});
                }else if action=="stop"{control.stop();}else{control.set_paused(action=="pause");}
                self.controls=controls(&self.id,control);
            }else{self.controls=Value::Null;}
        }
        self.last_played=state["played"].as_u64().unwrap_or(0);parse(&state)
    }
    fn push(&mut self,bytes:&[u8])->Result<usize> {jni_controls::write_pcm(&self.id,bytes)}
    fn drain(&mut self)->Result<()> {self.call("drain",json!({"id":self.id}))?;Ok(())}
    fn start(&mut self)->Result<()> {self.call("start",json!({"id":self.id}))?;Ok(())}
    fn prefill_frames(&self)->u64 {self.prefill}
    fn present(&mut self,update:&output::PlaybackUpdate)->Result<()> {
        if let Some(media)=&self.media {
            if self.last_media!=update.playback_id||self.controls["paused"]!=true&&self.last_present.elapsed()>=Duration::from_secs(1) {
                let metadata=(media.metadata)(&update.playback_id)?;let mut body=serde_json::to_value(metadata)?;
                body["id"]=json!(self.id);body["position"]=json!(update.position);
                body["played"]=json!(self.last_played);
                self.call("metadata",body)?;self.last_media=update.playback_id.clone();self.last_present=std::time::Instant::now();
            }
            if let Some(action)=self.pending.take(){(media.action)(update.clone(),action);}
        }Ok(())
    }
    fn close(&mut self)->Result<PcmState> {
        if !self.closed {self.call("stop",json!({"id":self.id}))?;self.closed=true;}
        let deadline=std::time::Instant::now()+Duration::from_secs(3);
        loop {let state=self.call("status",json!({"id":self.id}))?;if state["complete"]==true{self.last_played=state["played"].as_u64().unwrap_or(0);let final_state=parse(&state);let _=self.call("release",json!({"id":self.id}));return final_state;}
            if std::time::Instant::now()>deadline {bail!("Android PCM 退出超时");}std::thread::sleep(Duration::from_millis(5));}
    }
}
impl Drop for Sink {fn drop(&mut self){if !self.closed {let _=self.close();}}}
