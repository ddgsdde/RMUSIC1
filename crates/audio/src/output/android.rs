//! Android's application-owned MediaCodec/AudioTrack adapter. No host decoder process is used.
use super::*;
use anyhow::Context;
use std::{path::Path, sync::OnceLock};
pub trait PcmDecoder:Send {
    fn read_frames(&mut self,output:&mut[u8],frame_bytes:usize)->Result<usize>;
    fn abort_handle(&self)->crate::decoder::DecoderAbort;
    fn processing(&self)->Vec<String> {Vec::new()}
    fn channel_mask(&self)->Option<u32>{None}
}
#[derive(serde::Deserialize)]
pub struct PcmClock {pub counter:u64,pub programme_frames:u64,pub time_ns:u64,pub before_ns:u64,pub after_ns:u64,pub running:bool}
#[derive(Clone,serde::Serialize)]
pub struct Media {pub track_id:String,pub playback_id:String,pub title:String,pub artist:String,pub duration:f64}
#[derive(Clone)]
pub struct MediaAction {pub playback_id:String,pub action:String,pub position:f64}
#[derive(Clone)]
pub struct SequenceMedia {
    pub sequence:PlaybackSequence,
    pub metadata:Arc<dyn Fn(&str)->Result<Media>+Send+Sync>,
    pub action:Arc<dyn Fn(PlaybackUpdate,MediaAction)+Send+Sync>,
}
pub struct PcmState {pub written:u64,pub played:u64,pub ended:bool,pub cancelled:bool,pub continuity_lost:bool,pub path:SignalPath,pub clock:Option<PcmClock>}
pub trait PcmOutput:Send {
    fn poll(&mut self,control:&PlaybackControl)->Result<PcmState>;
    fn push(&mut self,bytes:&[u8])->Result<usize>;
    fn drain(&mut self)->Result<()>;
    fn close(&mut self)->Result<PcmState>;
    fn start(&mut self)->Result<()> {Ok(())}
    fn prefill_frames(&self)->u64 {1}
    fn present(&mut self,_:&PlaybackUpdate)->Result<()> {Ok(())}
}

pub trait Backend: Send + Sync {
    fn bind_sequence(&self,_:&Arc<PlaybackControl>,_:SequenceMedia)->Result<()> {Ok(())}
    fn transport(&self,_:&str,_:Option<serde_json::Value>)->Result<Option<String>> {Ok(None)}
    fn devices(&self)->Result<Vec<DeviceInfo>>;
    fn probe(&self,path:&Path)->Result<crate::probe::AudioMetadata>;
    fn decoder(&self,request:&PlaybackRequest,format:SequenceFormat)->Result<Box<dyn PcmDecoder>>;
    fn play(&self,request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,update:&mut dyn FnMut(SignalPath,Meter))->Result<()>;
    fn pcm_output(&self,_:&PlaybackRequest,_:&OutputConfig,_:&PlaybackControl)->Result<Box<dyn PcmOutput>> {bail!("Android continuous output adapter unavailable")}
}
static BACKEND:OnceLock<Arc<dyn Backend>>=OnceLock::new();
pub fn install(backend:Arc<dyn Backend>)->Result<()> {
    BACKEND.set(backend).map_err(|_|anyhow::anyhow!("Android audio backend already initialized"))
}
pub fn available()->bool {BACKEND.get().is_some()}
pub fn backend()->Result<&'static Arc<dyn Backend>> {BACKEND.get().context("Android audio backend is not initialized")}

fn read_batch(stream:&mut super::sequence::PcmStream,pending:&mut Vec<u8>,frames:usize,channels:usize,meter:&mut Meter)->Result<bool> {
    use super::sequence::Read;
    let width=channels*4;
    while pending.len()<frames*width {
        match stream.read(frames-pending.len()/width)? {
            Read::Data(chunk)=>{let peak=if pending.is_empty(){0.}else{meter.peak};pending.extend_from_slice(&chunk.bytes);*meter=chunk.meter;meter.peak=meter.peak.max(peak);},
            Read::Pending=>return Ok(false),
            Read::End=>return Ok(true),
        }
    }Ok(false)
}

pub(super) fn play_sequence(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,sequence:PlaybackSequence,mut update:impl FnMut(PlaybackUpdate))->Result<()> {
    use super::sequence::PcmStream;
    control.register_output_thread();
    if request.is_dsd() {bail!("Android 连续输出暂不支持 DSD");}
    if request.dsp.enabled {
        if request.impulse.is_some()||request.dsp.convolution.is_some(){bail!("Android 卷积尚未接入，请先关闭卷积");}
    }
    if config.android_mode==Some(AndroidMode::UsbBitperfect) {
        if request.dsp.enabled {bail!("USB 原样输出不支持 DSP；请关闭处理或明确选择系统／兼容输出");}
        if request.transitions.enabled() {bail!("USB 原样输出不支持淡入淡出；请关闭处理或明确选择系统／兼容输出");}
    }
    if !(1..=8).contains(&request.channels)||request.sample_rate>768000 {bail!("Android 连续 PCM 支持一至八声道源及最高768kHz");}
    let scheduled=control.start.setup(false)?;
    if sequence.rate_adjustment_active()&&config.android_mode==Some(AndroidMode::UsbBitperfect){bail!("USB 原样输出不允许同步变速；请选择系统输出或关闭漂移校正");}
    control.initialize_volume(request.volume)?;
    let _run=sequence.guard()?;
    let target=config.desired_format(&request)?;
    if !(1..=2).contains(&target.channels){bail!("Android 系统输出请选择单／双声道");}
    let format=SequenceFormat{sample_rate:target.sample_rate,channels:target.channels,channel_mask:0,exclusive:config.android_mode==Some(AndroidMode::UsbBitperfect),float_output:false,
        bit_depth:if config.android_mode==Some(AndroidMode::Compatibility){16}else{32}};
    let source=PlaybackRequest{sample_rate:format.sample_rate,channels:format.channels,..request.clone()};let mut stream=PcmStream::open(request,sequence.clone(),control.clone(),format)?;
    config.resync(&control)?;
    let mut sink=backend()?.pcm_output(&source,&config,&control)?;
    let mut pending=Vec::new();let mut eof=false;let mut draining=false;let mut meter=Meter::default();let mut last_played=0;let mut last_written=0;
    let batch=(format.sample_rate as usize/20).clamp(256,65_536/(format.channels as usize*4));
    let mut idle=None;let mut paused_since=None;let progress=std::cell::Cell::new(std::time::Instant::now());
    let clock=clock::Session::new(control.clone());
    let mut publish=|state:PcmState,final_end:bool,meter:Meter,sink:&mut dyn PcmOutput|->Result<()> {
        if state.played>state.written||state.played<last_played||state.written<last_written {bail!("Android PCM 帧计数失效");}
        if state.played!=last_played {progress.set(std::time::Instant::now());}
        last_played=state.played;last_written=state.written;
        control.frames_written.store(state.written,Ordering::Relaxed);control.frames_played.store(state.played,Ordering::Release);
        control.output_sample_rate.store(format.sample_rate,Ordering::Relaxed);
        if let Some(sample)=state.clock.filter(|s|s.programme_frames<=state.written&&s.before_ns<=s.after_ns&&s.time_ns<=s.after_ns&&s.after_ns-s.time_ns<=500_000_000) {
            clock.record_programme(clock::Source::AndroidFrames,sample.counter,format.sample_rate as u64,sample.time_ns,clock::Domain::Monotonic,Some(sample.before_ns),Some(sample.after_ns),sample.running,
                Some(clock::Programme{frames:sample.programme_frames,sample_rate:format.sample_rate}));
        }else{control.device_clock.invalidate(clock.epoch());}
        let ((id,position,source,ended),output_frames)=sequence.heard_with_frames(&control,final_end)?;
        let mut path=state.path;path.bit_perfect_eligible &= source.source_integer_lossless&&source.source_bit_depth>0&&source.source_bit_depth<=path.bit_depth&&source.sample_rate==format.sample_rate&&source.channels==format.channels&&control.volume()==1.&&!source.dsp.enabled&&!source.transitions.enabled();
        path.processing.extend(sequence.decoder_processing(&id)?);
        if source.transitions.enabled(){path.processing.push("Fade envelope".into());}
        if source.dsp.enabled {
            path.processing.push("DSP f64".into());
            if source.dsp.normalize&&!meter.normalization_applied{path.processing.push("Normalization pending measured LUFS".into());}
        }
        config.limits.signal_path(&source,&mut path);
        sequence.mark_rate_adjustment(&mut path);
        let event=PlaybackUpdate{output_frames,playback_id:id,position,ended,signal_path:path,meter,continuity_lost:state.continuity_lost};
        update(event.clone());sink.present(&event)?;Ok(())
    };
    let result=(||->Result<()> {
    if scheduled {
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(4);
        loop {
            if control.stopped.load(Ordering::Acquire){return Ok(());}
            if std::time::Instant::now()>deadline{bail!("Android 分组预缓冲超时");}
            if !pending.is_empty(){let count=sink.push(&pending)?;if count==pending.len(){pending.clear();}else if count!=0{bail!("Android PCM queue accepted a partial block");}}
            let state=sink.poll(&control)?;
            if state.cancelled{control.stop();return Ok(());}
            if state.played!=0{bail!("Android 计划起播前消费了节目帧");}
            let buffered=state.written;let ready=buffered>=sink.prefill_frames()||eof&&buffered>0&&pending.is_empty();publish(state,false,meter.clone(),sink.as_mut())?;
            if ready{break;}
            if pending.is_empty()&&!eof{eof=read_batch(&mut stream,&mut pending,batch,format.channels as usize,&mut meter)?;if eof&&pending.is_empty()&&buffered==0{bail!("分组歌曲没有可播放的 PCM 帧");}}
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        if !control.start.wait(&control)?{return Ok(());}
        sink.start()?;progress.set(std::time::Instant::now());
    }
    loop {
        let state=sink.poll(&control)?;let ended=state.ended;let cancelled=state.cancelled;
        let drained=state.played==state.written;
        publish(state,ended&&eof,meter.clone(),sink.as_mut())?;
        if control.stopped.load(Ordering::Acquire)||cancelled {control.stop();break;}
        if ended {if !eof||!drained {bail!("Android PCM 提前结束");}break;}
        if control.paused.load(Ordering::Acquire) {
            progress.set(std::time::Instant::now());
            let since=paused_since.get_or_insert_with(std::time::Instant::now);
            let interval=if since.elapsed()<std::time::Duration::from_millis(200){std::time::Duration::from_millis(20)}else{std::time::Duration::from_secs(1)};
            std::thread::park_timeout(interval);continue;
        }
        paused_since=None;
        if pending.is_empty()&&!eof {
            match read_batch(&mut stream,&mut pending,batch,format.channels as usize,&mut meter) {
                Ok(end)=>eof=end,Err(_) if control.stopped.load(Ordering::Acquire)=>break,Err(error)=>return Err(error),
            }
            if pending.is_empty()&&!eof&&drained {let since=idle.get_or_insert_with(std::time::Instant::now);if since.elapsed()>std::time::Duration::from_millis(500){sequence.finish_window_if_empty();}}
            else{idle=None;}
        }
        // AudioTrack can keep an unconfirmed tail while the decoder waits for the network.
        // Measure a stalled output only while PCM is available or the final tail is draining.
        if pending.is_empty()&&!eof {
            progress.set(std::time::Instant::now());
        }else if !drained&&progress.get().elapsed()>std::time::Duration::from_secs(3){
            return Err(DeviceUnavailable("Android PCM 播放头停止推进".into()).into());
        }
        let accepted=if !pending.is_empty() {let count=sink.push(&pending)?;if count!=0&&count!=pending.len(){bail!("Android PCM queue accepted a partial block");}if count>0{pending.clear();}count>0}else{false};
        if eof&&pending.is_empty()&&!draining{sink.drain()?;draining=true;}
        if !accepted{std::thread::sleep(std::time::Duration::from_millis(2));}
    }Ok(())})();
    let closed=sink.close().and_then(|state|publish(state,eof&&result.is_ok()&&!control.stopped.load(Ordering::Acquire),meter,sink.as_mut()));
    result.and(closed)
}

/// Instrumentation-only bounded read of the real shared stream; no AudioTrack or device output.
/// Eight MiB permits an 18-second stereo 48 kHz regression past the HTTP replay windows.
#[cfg(debug_assertions)]
pub fn audit_queue(requests:Vec<PlaybackRequest>,control:Arc<PlaybackControl>,exclusive:bool)->Result<Vec<u8>> {
    use super::sequence::{PcmStream,Read};
    let mut requests=requests.into_iter();let first=requests.next().context("missing first audit source")?;
    let format=SequenceFormat{sample_rate:first.sample_rate,channels:first.channels,channel_mask:0,exclusive,float_output:false,bit_depth:32};
    let sequence=PlaybackSequence::new("first".into(),first.clone(),format)?;let _run=sequence.guard()?;
    let mut stream=PcmStream::open(first,sequence.clone(),control.clone(),format)?;
    for (index,request) in requests.enumerate(){sequence.reserve()?.prepare(format!("next-{index}"),request)?;}
    sequence.queue_complete(true)?;
    let mut bytes=Vec::new();let deadline=std::time::Instant::now()+std::time::Duration::from_secs(15);
    loop {
        if std::time::Instant::now()>deadline {bail!("Android audit timed out");}
        match stream.read(257)? {
            Read::Data(chunk)=>{if bytes.len()+chunk.bytes.len()>8*1024*1024 {bail!("Android audit PCM budget exceeded");}bytes.extend_from_slice(&chunk.bytes);}
            Read::Pending=>std::thread::sleep(std::time::Duration::from_millis(1)),
            Read::End=>return Ok(bytes),
        }
    }
}
