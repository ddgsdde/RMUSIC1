use crate::{
    decoder::{DecoderAbort, FfmpegDecoder, PcmEncoding},
    DspConfig, DspProcessor, Meter,
};
use anyhow::{bail, Result};
use serde::{Deserialize,Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};
const UNSET_VOLUME: u64 = u64::MAX;
/// Output resources disappeared or stopped responding. This is not a source/codec error.
#[derive(Debug)]
pub struct DeviceUnavailable(pub String);
impl std::fmt::Display for DeviceUnavailable {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(&self.0)}
}
impl std::error::Error for DeviceUnavailable {}
/// A stalled callback on the already selected output can be reopened.
#[derive(Debug)]
pub struct DeviceClockInterrupted(pub String);
impl std::fmt::Display for DeviceClockInterrupted {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(&self.0)}
}
impl std::error::Error for DeviceClockInterrupted {}
/// A streaming source stopped supplying frames; keep its durable cursor for explicit recovery.
#[derive(Debug)]
pub struct SourceUnavailable(pub String);
impl std::fmt::Display for SourceUnavailable {
    fn fmt(&self, f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for SourceUnavailable {}
pub fn is_device_unavailable(error:&anyhow::Error)->bool {
    if error.is::<DeviceUnavailable>() || error.is::<DeviceClockInterrupted>() {return true;}
    #[cfg(all(windows,feature="asio-output"))]
    if let Some(error)=error.downcast_ref::<asio_sys::AsioError>() {
        return matches!(error,asio_sys::AsioError::NoDrivers|asio_sys::AsioError::HardwareMalfunction|asio_sys::AsioError::HardwareStuck);
    }
    #[cfg(windows)]
    if let Some(error)=error.downcast_ref::<wasapi::WasapiError>() {
        return match error {
            wasapi::WasapiError::DeviceNotFound(_)=>true,
            wasapi::WasapiError::Windows(error)=>matches!(error.code().0 as u32,
                0x88890004|0x88890026|0x88890010|0x80070490),
            _=>false,
        };
    }
    #[cfg(all(target_os="linux",feature="alsa-output"))]
    if let Some(error)=error.downcast_ref::<::alsa::Error>() {
        return matches!(error.errno(),2|19|25|86);
    }
    false
}
pub mod clock;
#[cfg(target_os="macos")]
pub mod external;
#[cfg(target_os="android")]
pub mod android;
pub mod start;
mod limits;
pub use limits::{OutputLimits,VolumeControl,ConversionLocation,PCM_RATES};
mod sequence;
#[cfg(not(target_os="android"))]
mod buffered;
#[cfg(windows)]
mod continuous;
#[cfg(any(windows,all(target_os="linux",feature="alsa-output")))]
pub mod power;
#[cfg(windows)]
mod wasapi_format;
#[cfg(all(windows,feature="asio-output"))]
mod asio;
pub use sequence::{CompletedTrack,CompletionCheck,PlaybackSequence,PlaybackUpdate,Preload,PreloadCancelled,PreloadFormatChange,SequenceFormat,SequencePosition,SourceClockPosition};

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub default: bool,
    pub mix_rate: u32,
    pub channels: u16,
}
pub fn build_info()->serde_json::Value {
    serde_json::json!({"audio_version":env!("CARGO_PKG_VERSION"),"asio_output":cfg!(all(windows,feature="asio-output")),"native_dsd_output":cfg!(any(all(windows,feature="asio-output"),all(target_os="linux",feature="alsa-output")))})
}
pub fn native_available()->bool {
    #[cfg(target_os="android")] {return android::available();}
    #[cfg(target_os="macos")] {return external::available();}
    #[cfg(not(any(target_os="android",target_os="macos")))] {cfg!(any(windows,all(target_os="linux",feature="alsa-output")))}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    #[serde(flatten)]
    pub limits:OutputLimits,
    pub device_id: Option<String>,
    pub exclusive: bool,
    pub buffer_ms: u32,
    pub dsd_output:DsdOutput,
    pub android_mode:Option<AndroidMode>,
}
#[derive(Debug,Clone,Copy,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum AndroidMode {#[default] System,UsbBitperfect,Compatibility}
#[derive(Debug,Clone,Copy,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum DsdOutput {#[default] Disabled,Dop,Native}
impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            limits:OutputLimits::default(),
            device_id: None,
            exclusive: false,
            buffer_ms: 100,
            dsd_output:DsdOutput::Disabled,
            android_mode:None,
        }
    }
}
impl OutputConfig {
    pub fn is_asio(&self) -> bool {self.device_id.as_deref().is_some_and(|id|id.starts_with("asio:"))}
    pub fn normalized(mut self) -> Self {if self.is_asio(){self.exclusive=true;}self}
    pub fn backend(&self) -> &'static str {
        if cfg!(windows) {if self.is_asio(){"asio"}else{"wasapi"}}
        else if cfg!(target_os="linux"){"alsa"}else if cfg!(target_os="android"){"android_audiotrack"}else if cfg!(target_os="macos"){"coreaudio"}else{"unsupported"}
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackRequest {
    /// Exact native FLAC STREAMINFO count; advisory durations never populate this.
    pub flac_samples: Option<u64>,
    /// Original one-bit source clock when intentionally decoding DSD to PCM.
    /// None retains existing PCM or explicit raw DSD semantics.
    pub dsd_pcm: Option<u32>,
    pub input: String,
    pub position: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub source_integer_lossless: bool,
    pub source_bit_depth: u16,
    pub dsp: DspConfig,
    pub impulse: Option<Arc<crate::impulse::ImpulseBank>>,
    pub measured_lufs: Option<f64>,
    pub volume: f64,
    pub duration: Option<f64>,
    pub transitions: crate::fade::FadeConfig,
}
impl PlaybackRequest {
    /// The verified native probe reserves one-bit source precision for raw DSD containers.
    pub fn is_dsd(&self)->bool {self.source_bit_depth==1&&!self.source_integer_lossless}
    fn position_after(&self,frames:u64,rate:f64)->f64 {
        let position=self.position+frames as f64/rate;
        if self.is_dsd(){self.duration.map_or(position,|duration|position.min(duration))}else{position}
    }
    pub(super) fn validate_dop(&self,config:&OutputConfig)->Result<()> {
        if !self.is_dsd(){return Ok(());}
        if config.dsd_output==DsdOutput::Disabled {bail!("DSD 输出未开启；请为兼容设备显式选择 DoP 或原生 DSD");}
        if config.dsd_output==DsdOutput::Native {
            let supported=(cfg!(all(windows,feature="asio-output"))&&config.is_asio())||(cfg!(all(target_os="linux",feature="alsa-output"))&&config.device_id.as_deref().is_some_and(|id|id.starts_with("hw:")));
            if !supported{bail!("原生 DSD 需要兼容的 Windows ASIO 驱动或 Linux ALSA hw: 设备");}
        }
        if !config.exclusive||config.device_id.as_deref().is_none_or(str::is_empty){bail!("DSD 需要明确选择设备并启用独占输出");}
        self.validate_dsd_processing()?;Ok(())
    }
    fn validate_dsd_processing(&self)->Result<()> {
        if self.is_dsd()&&(self.dsp.enabled||self.transitions.enabled()||self.volume!=1.) {bail!("原样 DSD 不支持 DSP、淡入淡出或软件音量；请关闭处理并将音量设为 100%，使用 DAC 调节音量");}
        Ok(())
    }
}
pub struct PlaybackControl {
    pub visualization: crate::visualization::Tap,
    pub stopped: AtomicBool,
    pub paused: AtomicBool,
    #[cfg(any(target_os="android",test))]
    output_thread:Mutex<Option<std::thread::Thread>>,
    #[cfg(any(windows,all(target_os="linux",feature="alsa-output")))]
    system_suspended: AtomicBool,
    #[cfg(any(windows,all(target_os="linux",feature="alsa-output")))]
    power_done: AtomicBool,
    #[cfg(all(target_os="linux",feature="alsa-output"))]
    sleep_checkpoint_pending: AtomicBool,
    pub frames_written: AtomicU64,
    pub frames_played: AtomicU64,
    /// Output cursor at programme end; physical FIR tail frames are excluded.
    pub programme_end: AtomicU64,
    pub clock_ns: AtomicU64,
    pub device_clock: clock::Feedback,
    pub start: start::Gate,
    pub output_sample_rate: AtomicU32,
    pub volume_bits: AtomicU64,
    volume_max:AtomicU64,
    volume_fixed:AtomicBool,
    pub muted: AtomicBool,
    loudness:Mutex<std::collections::VecDeque<(String,f64,Option<f64>)>>,
    decoder: Mutex<Option<DecoderAbort>>,
}
fn require_loaded_impulse(request:&PlaybackRequest)->Result<()> {
    if request.dsp.enabled&&request.dsp.convolution.is_some()&&request.impulse.is_none(){bail!("卷积响应尚未加载，已拒绝播放");}Ok(())
}
impl Default for PlaybackControl {
    fn default() -> Self {
        Self {
            visualization: Default::default(),
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            #[cfg(any(target_os="android",test))]
            output_thread:Mutex::new(None),
            loudness:Default::default(),
            #[cfg(any(windows,all(target_os="linux",feature="alsa-output")))]
            system_suspended: AtomicBool::new(false),
            #[cfg(any(windows,all(target_os="linux",feature="alsa-output")))]
            power_done: AtomicBool::new(false),
            #[cfg(all(target_os="linux",feature="alsa-output"))]
            sleep_checkpoint_pending: AtomicBool::new(false),
            frames_written: AtomicU64::new(0),
            frames_played: AtomicU64::new(0),
            programme_end: AtomicU64::new(u64::MAX),
            clock_ns: AtomicU64::new(0),
            device_clock: clock::Feedback::default(),
            start: start::Gate::default(),
            output_sample_rate: AtomicU32::new(0),
            volume_bits: AtomicU64::new(UNSET_VOLUME),
            volume_max:AtomicU64::new(1_f64.to_bits()),
            volume_fixed:AtomicBool::new(false),
            muted: AtomicBool::new(false),
            decoder: Mutex::new(None),
        }
    }
}
impl PlaybackControl {
    pub fn configure_limits(&self,limits:&OutputLimits)->Result<()> {
        limits.validate()?;self.volume_max.store(limits.volume_max.to_bits(),Ordering::Relaxed);self.volume_fixed.store(limits.volume_control!=VolumeControl::Software,Ordering::Relaxed);
        if limits.volume_control!=VolumeControl::Software {self.muted.store(false,Ordering::Relaxed);}
        if self.volume_bits.load(Ordering::Relaxed)!=UNSET_VOLUME{self.set_volume(self.volume_setting())?;}Ok(())
    }
    fn limited_volume(&self,value:f64)->f64{if self.volume_fixed.load(Ordering::Relaxed){1.}else{value.min(f64::from_bits(self.volume_max.load(Ordering::Relaxed)))}}
    pub fn set_loudness(&self,id:&str,measured:f64,true_peak:Option<f64>)->Result<()> {
        if id.is_empty()||!measured.is_finite(){bail!("invalid measured loudness");}
        let mut updates=self.loudness.lock().map_err(|_|anyhow::anyhow!("loudness control lock poisoned"))?;
        if let Some(entry)=updates.iter_mut().find(|entry|entry.0==id) {
            entry.1=measured; entry.2=true_peak;
        } else {
            // A continuous queue can hold multiple measured successor instances.
            if updates.len() >= 16 { updates.pop_front(); }
            updates.push_back((id.into(),measured,true_peak));
        }
        Ok(())
    }
    fn take_loudness(&self,id:&str)->Result<Option<(f64,Option<f64>)>> {
        let mut updates=self.loudness.lock().map_err(|_|anyhow::anyhow!("loudness control lock poisoned"))?;
        Ok(updates.iter().position(|entry|entry.0==id).and_then(|i|updates.remove(i)).map(|(_,lufs,peak)|(lufs,peak)))
    }
    pub fn system_interrupted(&self)->bool {
        #[cfg(any(windows,all(target_os="linux",feature="alsa-output")))] {self.system_suspended.load(Ordering::Acquire)}
        #[cfg(not(any(windows,all(target_os="linux",feature="alsa-output"))))] {false}
    }
    fn attach_decoder(&self,decoder:DecoderAbort)->Result<()> {
        *self.decoder.lock().map_err(|_|anyhow::anyhow!("decoder lock poisoned"))?=Some(decoder.clone());
        if self.stopped.load(Ordering::Acquire){decoder.abort();}
        Ok(())
    }
    pub fn seconds(&self,source_rate:u32)->f64 {
        let played=self.frames_played.load(Ordering::Acquire);
        let rate=self.output_sample_rate.load(Ordering::Relaxed);
        played.min(self.frames_written.load(Ordering::Relaxed)).min(self.programme_end.load(Ordering::Acquire)) as f64
            / if rate==0 {source_rate}else{rate} as f64
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        #[cfg(any(target_os="android",test))] self.wake_output();
        self.start.cancel();
        if let Ok(decoder) = self.decoder.lock() {
            if let Some(decoder) = decoder.as_ref() { decoder.abort(); }
        }
    }
    pub fn set_paused(&self,paused:bool) {
        self.paused.store(paused,Ordering::Release);
        #[cfg(any(target_os="android",test))] self.wake_output();
    }
    #[cfg(any(target_os="android",test))]
    pub fn register_output_thread(&self) {*self.output_thread.lock().unwrap()=Some(std::thread::current());}
    #[cfg(any(target_os="android",test))]
    pub fn wake_output(&self) {if let Some(thread)=self.output_thread.lock().unwrap().as_ref(){thread.unpark();}}
    pub fn set_volume(&self, volume: f64) -> Result<()> {
        if !volume.is_finite() || !(0. ..=1.).contains(&volume) {
            bail!("volume must be 0..1")
        }
        self.volume_bits.store(self.limited_volume(volume).to_bits(), Ordering::Relaxed);
        Ok(())
    }
    pub fn initialize_volume(&self, volume: f64) -> Result<()> {
        if !volume.is_finite() || !(0. ..=1.).contains(&volume) {
            bail!("volume must be 0..1")
        }
        // A command accepted before the device opens takes precedence over the request snapshot.
        let _ = self.volume_bits.compare_exchange(UNSET_VOLUME,self.limited_volume(volume).to_bits(),Ordering::Relaxed,Ordering::Relaxed);
        Ok(())
    }
    pub fn volume_setting(&self) -> f64 {
        let bits=self.volume_bits.load(Ordering::Relaxed);
        if bits==UNSET_VOLUME {1.} else {f64::from_bits(bits)}
    }
    pub fn volume(&self) -> f64 {
        if self.muted.load(Ordering::Relaxed) {
            0.
        } else {
            self.volume_setting()
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct SignalPath {
    pub output: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub exclusive: bool,
    pub bit_depth: u16,
    pub sample_type: String,
    pub mixer_sample_rate: Option<u32>,
    pub bit_perfect_eligible: bool,
    pub processing: Vec<String>,
}

#[cfg(windows)]
pub fn devices() -> Result<Vec<DeviceInfo>> {
    use wasapi::*;
    initialize_mta().ok()?;
    let enumerator = DeviceEnumerator::new()?;
    let default_id = enumerator
        .get_default_device(&Direction::Render)?
        .get_id()?;
    let collection = enumerator.get_device_collection(&Direction::Render)?;
    let mut result = Vec::new();
    for i in 0..collection.get_nbr_devices()? {
        let device = collection.get_device_at_index(i)?;
        let mix = device.get_iaudioclient()?.get_mixformat()?;
        let id = device.get_id()?;
        result.push(DeviceInfo {
            default: id == default_id,
            id,
            name: device.get_friendlyname()?,
            mix_rate: mix.get_samplespersec(),
            channels: mix.get_nchannels(),
        });
    }
    #[cfg(feature="asio-output")]
    result.extend(asio::devices());
    Ok(result)
}
#[cfg(windows)]
pub fn play_sequence(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,sequence:PlaybackSequence,update:impl FnMut(PlaybackUpdate))->Result<()> {
    config.prepare(&request,&control,&sequence)?;
    request.validate_dop(&config)?;
    power::watch(&control)?;power::check(&control)?;
    let finishing=control.clone();
    let result=if config.is_asio() {
        #[cfg(feature="asio-output")]
        {asio::play_sequence(request,config.normalized(),control,sequence,update)}
        #[cfg(not(feature="asio-output"))]
        bail!("当前程序未编译 ASIO 输出支持");
    }else{continuous::play_sequence(request,config,control,sequence,update)};
    finishing.start.cancel();
    power::finish(&finishing,result)
}
#[cfg(all(target_os="linux",feature="alsa-output"))]
mod alsa;
#[cfg(all(target_os="linux",feature="alsa-output"))]
pub use alsa::devices;
#[cfg(all(target_os="linux",feature="alsa-output"))]
pub fn play(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,update:impl FnMut(SignalPath,Meter))->Result<()> {
    power::check(&control)?;
    if control.stopped.load(Ordering::Acquire){return Ok(());}
    if config.limits.active()||request.dsp.enabled&&request.dsp.normalize{return play_single_impulse(request,config,control,update);}
    if control.start.setup(request.is_dsd())?{bail!("计划起播须使用连续输出 API");}
    power::watch(&control)?;power::check(&control)?;let finishing=control.clone();power::finish(&finishing,alsa::play(request,config,control,update))
}
#[cfg(all(target_os="linux",feature="alsa-output"))]
pub fn play_sequence(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,sequence:PlaybackSequence,update:impl FnMut(PlaybackUpdate))->Result<()> {
    config.prepare(&request,&control,&sequence)?;power::watch(&control)?;power::check(&control)?;let finishing=control.clone();{let result=alsa::play_sequence(request,config,control,sequence,update);finishing.start.cancel();power::finish(&finishing,result)}
}
#[cfg(target_os="macos")]
pub fn devices()->Result<Vec<DeviceInfo>> {external::backend()?.devices()}
#[cfg(target_os="macos")]
pub fn play(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,update:impl FnMut(SignalPath,Meter))->Result<()> {
    play_single_impulse(request,config,control,update)
}
#[cfg(target_os="macos")]
pub fn play_sequence(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,sequence:PlaybackSequence,update:impl FnMut(PlaybackUpdate))->Result<()> {
    config.prepare(&request,&control,&sequence)?;
    external::play_sequence(request,config,control,sequence,update)
}
#[cfg(target_os="android")]
pub fn devices()->Result<Vec<DeviceInfo>> {android::backend()?.devices()}
#[cfg(target_os="android")]
pub fn play(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,mut update:impl FnMut(SignalPath,Meter))->Result<()> {
    if request.is_dsd(){bail!("Android DSD 输出尚未接入");}
    if config.limits.active()||request.dsp.enabled&&request.dsp.normalize{return play_single_impulse(request,config,control,update);}
    control.initialize_volume(request.volume)?;
    android::backend()?.play(request,config,control,&mut update)
}
#[cfg(not(any(windows,target_os="android",target_os="macos",all(target_os="linux",feature="alsa-output"))))]
pub fn devices() -> Result<Vec<DeviceInfo>> {
    bail!("native output backend is not available on this platform")
}
#[cfg(target_os="android")]
pub fn play_sequence(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,sequence:PlaybackSequence,update:impl FnMut(PlaybackUpdate))->Result<()> {config.prepare(&request,&control,&sequence)?;android::play_sequence(request,config,control,sequence,update)}
#[cfg(not(any(windows,target_os="android",target_os="macos",all(target_os="linux",feature="alsa-output"))))]
pub fn play_sequence(_request:PlaybackRequest,_config:OutputConfig,_control:Arc<PlaybackControl>,_sequence:PlaybackSequence,_update:impl FnMut(PlaybackUpdate))->Result<()> {
    bail!("continuous output backend is not available on this platform")
}

#[cfg(any(windows,target_os="android",target_os="macos",all(target_os="linux",feature="alsa-output")))]
fn play_single_impulse(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,mut update:impl FnMut(SignalPath,Meter))->Result<()> {
    let format=SequenceFormat{sample_rate:request.sample_rate,channels:request.channels,channel_mask:0,exclusive:config.exclusive,float_output:request.dsp.enabled,bit_depth:if request.is_dsd()&&config.dsd_output==DsdOutput::Native{1}else{32}};
    let sequence=PlaybackSequence::new("single".into(),request.clone(),format)?;
    play_sequence(request,config,control,sequence,move|state|update(state.signal_path,state.meter))
}

#[cfg(windows)]
pub fn play(
    request: PlaybackRequest,
    config: OutputConfig,
    control: Arc<PlaybackControl>,
    update: impl FnMut(SignalPath, Meter),
) -> Result<()> {
    power::watch(&control)?;power::check(&control)?;
    let finishing=control.clone();
    if control.start.setup(request.is_dsd())?{bail!("计划起播须使用连续输出 API");}
    power::finish(&finishing,play_windows(request,config,control,update))
}
#[cfg(windows)]
fn play_windows(
    request: PlaybackRequest,
    config: OutputConfig,
    control: Arc<PlaybackControl>,
    mut update: impl FnMut(SignalPath, Meter),
) -> Result<()> {
    if config.limits.active()||request.dsp.enabled&&request.dsp.normalize{return play_single_impulse(request,config,control,update);}

    request.validate_dop(&config)?;
    require_loaded_impulse(&request)?;
    if request.is_dsd(){return play_single_impulse(request,config,control,update);}
    if config.is_asio(){return play_single_impulse(request,config.normalized(),control,update);}
    if request.dsp.enabled&&request.impulse.is_some(){return play_single_impulse(request,config,control,update);}
    use wasapi::*;
    initialize_mta().ok()?;
    if !(20..=2000).contains(&config.buffer_ms) {
        bail!("buffer must be 20..2000 ms")
    }
    if !request.volume.is_finite() || !(0. ..=1.).contains(&request.volume) {
        bail!("invalid volume")
    }
    let enumerator = DeviceEnumerator::new()?;
    let device = match &config.device_id {
        Some(id) => enumerator.get_device(id)?,
        None => enumerator.get_default_device(&Direction::Render)?,
    };
    request.transitions.validate()?;
    let mut client = device.get_iaudioclient()?;
    // Exclusive unity-gain bypass uses integer PCM. Shared mode explicitly includes the Windows mixer.
    control.initialize_volume(request.volume)?;
    control.output_sample_rate.store(request.sample_rate,Ordering::Relaxed);
    let processing = request.dsp.enabled;
    let (bits, sample_type) = if processing {
        (32, SampleType::Float)
    } else {
        (32, SampleType::Int)
    };
    let desired = WaveFormat::new(
        bits,
        bits,
        &sample_type,
        request.sample_rate as usize,
        request.channels as usize,
        None,
    );
    let (format, mode) = if config.exclusive {
        let supported = wasapi_format::exclusive(&desired, request.source_bit_depth,
            |candidate| client.is_supported_exclusive_with_quirks(candidate))?;
        let (_, minimum) = client.get_device_period()?;
        let period = client.calculate_aligned_period_near(
            (config.buffer_ms as i64 * 10000).max(minimum),
            None,
            &supported,
        )?;
        (
            supported,
            StreamMode::EventsExclusive { period_hns: period },
        )
    } else {
        (
            desired,
            StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: config.buffer_ms as i64 * 10000,
            },
        )
    };
    client.initialize_client(&format, &Direction::Render, &mode)?;
    let event = client.set_get_eventhandle()?;
    let render = client.get_audiorenderclient()?;
    let clock = client.get_audioclock()?;
    let clock_frequency = clock.get_frequency()?;
    let feedback=clock::Session::new(control.clone());
    let mut decoder = FfmpegDecoder::open(
        &request.input,
        request.position,
        request.sample_rate,
        request.channels,
        if processing {
            PcmEncoding::Float64
        } else {
            PcmEncoding::Signed32
        },
    )?;
    control.attach_decoder(decoder.abort_handle())?;
    if control.stopped.load(Ordering::Acquire) { control.stop(); return Ok(()); }
    let mut dsp = DspProcessor::new(
        request.dsp.clone(),
        request.sample_rate,
        request.channels as usize,
        request.measured_lufs,
    )?;
    let mut path = SignalPath {
        output: device.get_friendlyname()?,
        sample_rate: request.sample_rate,
        channels: request.channels,
        exclusive: config.exclusive,
        bit_depth: format.get_validbitspersample(),
        sample_type: if processing {"float"} else {"integer"}.into(),
        mixer_sample_rate: if config.exclusive {None}else{client.get_mixformat().ok().map(|f|f.get_samplespersec())},
        bit_perfect_eligible: config.exclusive && !processing && !request.transitions.enabled() && request.source_integer_lossless
            && request.source_bit_depth > 0 && request.source_bit_depth <= format.get_validbitspersample(),
        processing: {
            let mut stages = Vec::new();
            if request.dsp.enabled {
                stages.push("DSP f64".into());
            }
            if request.transitions.enabled() { stages.push("Fade envelope".into()); }
            if request.dsp.normalize && request.measured_lufs.is_none() {
                stages.push("Normalization pending measured LUFS".into());
            }
            if !config.exclusive {
                stages.push("Windows shared mixer / compatible rate conversion".into());
            }
            if !request.source_integer_lossless {
                stages.push(
                    if processing {
                        "Decoded source to float64 PCM"
                    } else {
                        "Decoded source to signed 32-bit PCM"
                    }
                    .into(),
                );
            }
            stages
        },
    };
    let capacity = client.get_buffer_size()? as usize;
    let input_bytes = request.channels as usize * if processing { 8 } else { 4 };
    let mut bytes = vec![0u8; capacity * input_bytes];
    let mut started = false;
    let mut paused = false;
    let mut eof = false;
    loop {
        if control.stopped.load(Ordering::Relaxed) {
            break;
        }
        let wanted_pause = control.paused.load(Ordering::Relaxed);
        if wanted_pause != paused && started {
            if wanted_pause {
                client.stop_stream()?;
            } else {
                client.start_stream()?;
            }
            paused = wanted_pause;
        }
        if wanted_pause {
            std::thread::sleep(std::time::Duration::from_millis(20));
            continue;
        }
        if started {
            // A disconnected device may no longer signal its event.
            let _ = event.wait_for_event(20);
        }
        if control.stopped.load(Ordering::Acquire) { break; }
        let before=clock::host_ns();
        let (position, qpc_100ns) = clock.get_position()?;
        feedback.record(clock::Source::WasapiTicks,position,clock_frequency,qpc_100ns.saturating_mul(100),clock::Domain::Qpc,before,clock::host_ns(),started&&!control.paused.load(Ordering::Relaxed));
        control.frames_played.store(
            (((position as u128 * request.sample_rate as u128) / clock_frequency as u128) as u64)
                .min(control.frames_written.load(Ordering::Relaxed)),
            Ordering::Release,
        );
        control
            .clock_ns
            .store(qpc_100ns.saturating_mul(100), Ordering::Relaxed);
        let space = client.get_available_space_in_frames()? as usize;
        if eof {
            // Shared-mode padding can empty before the device clock reaches the last sample.
            if client.get_current_padding()? == 0
                && control.frames_played.load(Ordering::Acquire)>=control.frames_written.load(Ordering::Relaxed) {
                break;
            }
            continue;
        }
        if space == 0 {
            continue;
        }
        let decoded = decoder.read_frames(&mut bytes[..space * input_bytes], input_bytes);
        if control.stopped.load(Ordering::Acquire) { break; }
        let frames = decoded?;
        if frames == 0 {
            eof = true;
            continue;
        }
        let mut meter = Meter::default();
        if !processing {
            meter.peak = bytes[..frames * input_bytes]
                .chunks_exact(4)
                .map(|b| i32::from_le_bytes(b.try_into().unwrap()) as f64 / 2147483648.)
                .map(f64::abs)
                .fold(0., f64::max);
        }
        let volume = control.volume();
        path.bit_perfect_eligible =
            config.exclusive && !processing && !request.transitions.enabled() && volume == 1. && request.source_integer_lossless
                && request.source_bit_depth > 0 && request.source_bit_depth <= format.get_validbitspersample();
        path.processing.retain(|stage| stage != "Software volume");
        if volume != 1. {
            path.processing.push("Software volume".into());
        }
        let start=control.frames_written.load(Ordering::Relaxed);
        let gain=|sample:usize|volume * request.transitions.gain(request.position+(start+sample as u64/request.channels as u64) as f64/request.sample_rate as f64,request.duration);
        let mut output = if processing {
            let mut samples: Vec<f64> = bytes[..frames * input_bytes]
                .chunks_exact(8)
                .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
                .collect();
            meter = dsp.process(&mut samples)?;
            samples
                .iter()
                .enumerate().flat_map(|(i,x)| ((*x * gain(i)).clamp(-1., 1.) as f32).to_le_bytes())
                .collect::<Vec<_>>()
        } else if volume == 1. && !request.transitions.enabled() {
            bytes[..frames * input_bytes].to_vec()
        } else {
            bytes[..frames * input_bytes]
                .chunks_exact(4)
                .enumerate().flat_map(|(i,b)| {
                    let sample = i32::from_le_bytes(b.try_into().unwrap());
                    ((sample as f64 * gain(i))
                        .round()
                        .clamp(i32::MIN as f64, i32::MAX as f64) as i32)
                        .to_le_bytes()
                })
                .collect::<Vec<_>>()
        };
        let output_frames = if config.exclusive && frames < space {
            output.resize(space * request.channels as usize * 4, 0);
            eof = true;
            space
        } else {
            frames
        };
        let packed = wasapi_format::pack(&format, &output)?;
        render.write_to_device(output_frames, &packed, None)?;
        control
            .frames_written
            .fetch_add(frames as u64, Ordering::Relaxed);
        update(path.clone(), meter);
        if !started {
            client.start_stream()?;
            started = true;
        }
    }
    if started {
        client.stop_stream()?;
        client.reset_stream()?;
    }
    Ok(())
}

#[cfg(not(any(windows,target_os="android",target_os="macos",all(target_os="linux",feature="alsa-output"))))]
pub fn play(
    _request: PlaybackRequest,
    _config: OutputConfig,
    _control: Arc<PlaybackControl>,
    _update: impl FnMut(SignalPath, Meter),
) -> Result<()> {
    bail!("native output backend is not enabled on this platform")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_clock_uses_negotiated_rate_and_excludes_unwritten_padding(){
        let control=PlaybackControl::default();control.frames_written.store(44100,Ordering::Relaxed);control.frames_played.store(88200,Ordering::Relaxed);
        control.output_sample_rate.store(44100,Ordering::Relaxed);assert_eq!(control.seconds(96000),1.);
        control.output_sample_rate.store(0,Ordering::Relaxed);assert_eq!(control.seconds(96000),44100./96000.);
    }
    #[test]
    fn device_initialization_keeps_a_newer_volume_and_mute_command() {
        let control=PlaybackControl::default();
        assert_eq!(control.volume(),1.);
        control.initialize_volume(0.8).unwrap();
        assert_eq!(control.volume(),0.8);
        control.set_volume(0.2).unwrap();
        control.muted.store(true,Ordering::Relaxed);
        control.initialize_volume(0.9).unwrap();
        assert_eq!(control.volume_setting(),0.2);
        assert_eq!(control.volume(),0.);
        control.muted.store(false,Ordering::Relaxed);
        assert_eq!(control.volume(),0.2);
    }
    #[test]
    fn volume_command_wins_a_race_with_device_initialization() {
        for _ in 0..32 {
            let control=Arc::new(PlaybackControl::default());
            let ready=Arc::new(std::sync::Barrier::new(2));
            let worker_control=control.clone();let worker_ready=ready.clone();
            let worker=std::thread::spawn(move||{worker_ready.wait();worker_control.initialize_volume(0.8).unwrap();});
            ready.wait();control.set_volume(0.2).unwrap();worker.join().unwrap();
            assert_eq!(control.volume(),0.2);
        }
    }
    #[test]
    fn invalid_volume_does_not_modify_the_current_setting() {
        let control=PlaybackControl::default();
        for value in [f64::NAN,f64::INFINITY,-0.1,1.1] {
            assert!(control.initialize_volume(value).is_err());
            assert!(control.set_volume(value).is_err());
            assert_eq!(control.volume(),1.);
        }
        control.initialize_volume(0.3).unwrap();
        assert_eq!(control.volume(),0.3);
    }
}
