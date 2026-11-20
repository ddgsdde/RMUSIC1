//! Bounded next-track preparation and a device-frame timeline for continuous output.
use super::*;
use anyhow::Context;
use std::collections::{BTreeMap, VecDeque};
#[cfg(feature="adaptive-sync")]
mod rate;
type Heard=(String,f64,PlaybackRequest,Vec<CompletedTrack>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub channel_mask: u32,
    pub exclusive: bool,
    pub float_output: bool,
    pub bit_depth: u16,
}
impl SequenceFormat {
    // Android decoders preserve integer PCM32; DSP consumes its exact f64 conversion below.
    fn decode_float(self,request:&PlaybackRequest)->bool {(!cfg!(target_os="android")&&request.dsp.enabled)||(self.float_output&&self.bit_depth==64)}
    fn frame_bytes(self,request:&PlaybackRequest)->usize {self.channels as usize*if self.bit_depth==1{1}else if self.decode_float(request){8}else{4}}
    fn validate(self) -> Result<()> {
        if self.bit_depth==1 {
            if !self.exclusive||self.float_output||!(1..=6).contains(&self.channels)||![352800,384000,705600,768000,1411200,1536000,2822400,3072000].contains(&self.sample_rate){bail!("invalid native DSD byte-frame format");}
            return Ok(());
        }
        if !(8000..=768000).contains(&self.sample_rate)
            || !(1..=32).contains(&self.channels)
            || !([16, 18, 20, 24, 32].contains(&self.bit_depth)||(self.float_output&&self.bit_depth==64))
        {
            bail!("invalid continuous output format");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct CompletedTrack {
    pub playback_id: String,
    pub position: f64,
    /// Actual heard output frames, independently of the programme position.
    pub frames: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct PlaybackUpdate {
    pub playback_id: String,
    pub position: f64,
    pub ended: Vec<CompletedTrack>,
    pub signal_path: SignalPath,
    pub meter: Meter,
    pub continuity_lost: bool,
    /// Heard output frames within this track; position may use a different source rate.
    pub output_frames:u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct SequencePosition {
    pub playback_id: String,
    pub position: f64,
    /// Actual heard output frames within this track.
    pub frames: u64,
}
/// Same captured output cursor expressed in independent programme coordinates.
#[derive(Clone,Debug)]
pub struct SourceClockPosition {
    pub position:SequencePosition,pub source_frame:u64,pub source_fraction:f64,pub source_per_output:f64,
}
/// Map device-clock gaps to submitted audio. Idle device frames are never source progress.
#[cfg(any(windows, test))]
#[derive(Default)]
pub(super) struct DeviceClock {
    submitted: u64,
    idle: u64,
    heard: u64,
    pub continuity_lost: bool,
}
#[cfg(any(windows, test))]
impl DeviceClock {
    pub fn advance(&mut self, device_frames: u64, source_written: u64, detect_gap: bool) -> u64 {
        if device_frames > self.submitted {
            let missing = device_frames - self.submitted;
            self.idle += missing;
            self.submitted = device_frames;
            self.continuity_lost |= detect_gap && missing > 2;
        }
        self.heard = self
            .heard
            .max(device_frames.saturating_sub(self.idle).min(source_written));
        self.heard
    }
    pub fn submitted(&mut self, frames: u64) {
        self.submitted += frames;
    }
}
pub type CompletionCheck = Arc<dyn Fn(f64) -> Result<()> + Send + Sync>;
#[derive(Debug)]
pub struct PreloadCancelled;
impl std::fmt::Display for PreloadCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("next-track preparation cancelled")
    }
}
impl std::error::Error for PreloadCancelled {}
/// End this device session normally; Core will issue the next play at its native format.
#[derive(Debug)]
pub struct PreloadFormatChange;
impl std::fmt::Display for PreloadFormatChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("track format changes require device renegotiation")
    }
}
impl std::error::Error for PreloadFormatChange {}
struct Prepared {
    id: String,
    request: PlaybackRequest,
    decoder: SourceDecoder,
    primer: Vec<u8>,
    cursor: usize,
    check: Option<CompletionCheck>,
    eof_verified: bool,
}
// Only the bridge's private loopback proxy accepts this request. It retains the media authority.
fn conversion_input(request:&PlaybackRequest,format:SequenceFormat,limits:&OutputLimits)->Result<(String,f64)> {
    let bits=limits.max_bit_depth.unwrap_or(format.bit_depth.min(32)).min(format.bit_depth.min(32));
    let needed=request.dsd_pcm.is_some()||request.sample_rate!=format.sample_rate||request.channels!=format.channels||(request.source_bit_depth>bits&&request.source_integer_lossless);
    let url=url::Url::parse(&request.input).ok().filter(|url|url.scheme()=="http"&&url.host_str()==Some("127.0.0.1")&&url.path().starts_with("/audio/"));
    let remote=url.as_ref().is_some_and(|url|url.query_pairs().any(|(k,v)|k=="roaming"&&v=="1"));
    let core=url.as_ref().and_then(|url|url.query_pairs().find(|(k,_)|k=="core_pcm").map(|(_,v)|v=="render"));
    let convert=core.map_or(limits.conversion_location==ConversionLocation::Core&&needed, |required|required||needed);
    if !remote && !convert { return Ok((request.input.clone(),request.position)); }
    let mut input=url::Url::parse(&request.input).map_err(|_|anyhow::anyhow!("本地独立文件不能使用 Core 转换，请选择播放端转换"))?;
    if input.scheme()!="http"||input.host_str()!=Some("127.0.0.1")||!input.path().starts_with("/audio/"){bail!("当前音源没有受控 Core 转换通道");}
    input.query_pairs_mut().append_pair("conversion","core").append_pair("rate",&format.sample_rate.to_string()).append_pair("bits",&bits.to_string()).append_pair("channels",&format.channels.to_string()).append_pair("start",&request.position.to_string());
    Ok((input.into(),0.))
}
enum SourceDecoder {
    #[cfg(target_os="android")]
    Android(Box<dyn super::android::PcmDecoder>),
    Pcm(FfmpegDecoder),
    Dop(crate::dsd::DsdReader<crate::decoder::FfmpegBytes>,crate::dsd::DopEncoder),
    NativeDsd(crate::dsd::DsdReader<crate::decoder::FfmpegBytes>),
}
impl SourceDecoder {
    fn open(request:&PlaybackRequest,format:SequenceFormat,limits:&OutputLimits,mut register:impl FnMut(DecoderAbort)->Result<()>)->Result<Self> {
        #[cfg(target_os="android")] {
            if format.exclusive&&request.dsp.enabled {bail!("USB 原样输出不支持 DSP");}
            if format.exclusive&&request.transitions.enabled() {bail!("USB 原样输出不支持淡入淡出");}
            if request.dsp.enabled&&(request.impulse.is_some()||request.dsp.convolution.is_some()){bail!("Android 卷积尚未接入，请先关闭卷积");}
            if request.is_dsd(){bail!("Android 连续预载暂不支持 DSD");}
            if format.exclusive&&(request.sample_rate!=format.sample_rate||request.channels!=format.channels) {return Err(PreloadFormatChange.into());}
            let decoder=super::android::backend()?.decoder(request,format)?;
            register(decoder.abort_handle())?;
            return Ok(Self::Android(decoder));
        }
        #[cfg(not(target_os="android"))] {
        if let Some(rate)=request.dsd_pcm {
            // Validate the raw container before invoking its PCM decoder. Never interpret
            // DoP packets as PCM or silently admit DST / mismatched channel declarations.
            let mut source=crate::decoder::FfmpegBytes::open(&request.input,0)?;
            register(source.abort_handle())?;
            let layout=crate::dsd::DsdLayout::read(&mut source)?;
            if layout.sample_rate!=rate||layout.channels!=request.channels {
                bail!("DSD container disagrees with the verified source format");
            }
        }
        if format.bit_depth==1&&!request.is_dsd(){bail!("native DSD buffers cannot carry PCM");}
        if request.is_dsd() {
            request.validate_dsd_processing()?;
            let native=format.bit_depth==1;
            if !format.exclusive||format.float_output||(!native&&format.bit_depth<24)||format.sample_rate!=request.sample_rate||format.channels!=request.channels {bail!("DSD requires exact-rate exclusive output and an explicit compatible container");}
            let reader=if native{crate::dsd::open_native(&request.input,request.position,register)?}else{crate::dsd::open(&request.input,request.position,register)?};
            let rate=if native{reader.layout().sample_rate/8}else{reader.layout().dop_rate()?};
            if rate!=format.sample_rate||reader.layout().channels!=format.channels {bail!("DSD source disagrees with its verified output format");}
            let mask=reader.layout().channel_mask;
            if mask==0||(format.channel_mask!=0&&mask!=format.channel_mask)||(format.channels>2&&format.channel_mask==0) {bail!("DSD source channel order cannot be preserved by this device format");}
            return if native{Ok(Self::NativeDsd(reader))}else{Ok(Self::Dop(reader,crate::dsd::DopEncoder::new(format.channels)?))};
        }
        let (input,position)=conversion_input(request,format,limits)?;
        let extent=(input==request.input).then_some(request.flac_samples).flatten().map(|samples|(samples,request.sample_rate));
        let decoder=FfmpegDecoder::open_flac(&input,position,format.sample_rate,format.channels,
            if format.decode_float(request){PcmEncoding::Float64}else{PcmEncoding::Signed32},extent)?;
        register(decoder.abort_handle())?;Ok(Self::Pcm(decoder))
        }
    }
    fn abort_handle(&self)->DecoderAbort {match self{Self::Pcm(decoder)=>decoder.abort_handle(),Self::Dop(reader,_)|Self::NativeDsd(reader)=>reader.abort_handle(),#[cfg(target_os="android")] Self::Android(decoder)=>decoder.abort_handle()}}
    fn source_time(&self)->Option<(f64,f64)> {match self{Self::Dop(reader,_)|Self::NativeDsd(reader)=>Some((reader.position(),reader.layout().duration())),Self::Pcm(_)=>None,#[cfg(target_os="android")] Self::Android(_)=>None}}
    fn read_frames(&mut self,output:&mut[u8],frame_bytes:usize)->Result<usize> {
        match self {#[cfg(target_os="android")] Self::Android(decoder)=>decoder.read_frames(output,frame_bytes),Self::Pcm(decoder)=>decoder.read_frames(output,frame_bytes),Self::NativeDsd(reader)=>{
            if frame_bytes!=reader.layout().channels as usize{bail!("invalid native DSD frame size");}reader.read_native(output)
        },Self::Dop(reader,encoder)=>{
            if frame_bytes!=reader.layout().channels as usize*4 {bail!("invalid DoP frame size");}
            reader.read_dop(encoder,output,crate::dsd::DopContainer::LeftAligned32,false)
        }}
    }
}
struct Slot {
    abort: Option<DecoderAbort>,
    ready: Option<Prepared>,
    failed: bool,
}
struct Boundary {
    id: String,
    request: PlaybackRequest,
    start: u64,
    end: Option<u64>,
    #[cfg(target_os="android")]
    decoder_processing:Vec<String>,
    #[cfg(feature="adaptive-sync")]
    output_start:Option<u64>,
    #[cfg(feature="adaptive-sync")]
    output_end:Option<u64>,
}
struct State {
    limits:OutputLimits,
    format: SequenceFormat,
    generation: u64,
    serial: u64,
    closed: bool,
    started: bool,
    claimed: bool,
    initial_check: Option<CompletionCheck>,
    on_configured: Option<Box<dyn FnOnce(PlaybackSequence) -> Result<()> + Send>>,
    queue_complete: bool,
    slots: BTreeMap<u64, Slot>,
    history: VecDeque<Boundary>,
    programme_end:Option<u64>,
    #[cfg(feature="adaptive-sync")]
    rate:Option<rate::Rate>,
}
#[derive(Clone)]
pub struct PlaybackSequence {
    state: Arc<Mutex<State>>,
}
/// Reserve in queue order, then prepare on a worker thread. Dropping this reservation cancels it.
pub struct Preload {
    sequence: PlaybackSequence,
    generation: u64,
    serial: u64,
    format: SequenceFormat,
    dsd:bool,
    finished: bool,
    attempted: bool,
}
impl PlaybackSequence {
    pub fn output_limits(&self,limits:OutputLimits)->Result<()> {self.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?.limits=limits;Ok(())}
    pub fn new(id: String, request: PlaybackRequest, format: SequenceFormat) -> Result<Self> {
        Self::checked(id, request, format, None)
    }
    pub fn checked(
        id: String,
        request: PlaybackRequest,
        format: SequenceFormat,
        check: Option<CompletionCheck>,
    ) -> Result<Self> {
        format.validate()?;
        validate(&request)?;
        if id.is_empty() {
            bail!("continuous playback requires a track instance ID");
        }
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                limits:OutputLimits::default(),
                format,
                generation: 0,
                serial: 0,
                closed: false,
                started: false,
                claimed: false,
                initial_check: check,
                on_configured: None,
                queue_complete: true,
                slots: BTreeMap::new(),
                history: VecDeque::from([Boundary {
                    id,
                    request,
                    start: 0,
                    end: None,
                    #[cfg(target_os="android")]
                    decoder_processing:Vec::new(),
                    #[cfg(feature="adaptive-sync")]
                    output_start:Some(0),
                    #[cfg(feature="adaptive-sync")]
                    output_end:None,
                }]),
                programme_end:None,
                #[cfg(feature="adaptive-sync")]
                rate:None,
            })),
        })
    }
    /// Explicit opt-in, before preparing any source. No UI/Core policy enables this yet.
    #[cfg(feature="adaptive-sync")]
    pub fn enable_rate_adjustment(&self,ppm:f64)->Result<()> {
        let mut state=self.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;
        if state.started||state.closed||!state.slots.is_empty()||state.history[0].request.is_dsd()||state.format.bit_depth==1 {bail!("PCM rate adjustment must be selected before source preparation");}
        state.rate=Some(rate::Rate::new(ppm)?);Ok(())
    }
    #[cfg(feature="adaptive-sync")]
    pub fn set_rate_ppm(&self,ppm:f64)->Result<()> {
        self.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?.rate.as_mut().context("PCM rate adjustment is not enabled")?.set_ppm(ppm)
    }
    pub fn rate_adjustment_active(&self)->bool {
        #[cfg(feature="adaptive-sync")] {return self.state.lock().map_or(true,|state|state.rate.is_some());}
        #[cfg(not(feature="adaptive-sync"))] {false}
    }
    pub fn rate_adjustment_supported()->bool {cfg!(feature="adaptive-sync")}
    pub(super) fn mark_rate_adjustment(&self,path:&mut SignalPath) {
        if self.rate_adjustment_active(){path.bit_perfect_eligible=false;path.processing.push("Adaptive PCM sinc f64; output/source clocks separated".into());}
    }
    fn decode_format(&self,format:SequenceFormat)->SequenceFormat {
        if self.rate_adjustment_active(){SequenceFormat{float_output:true,bit_depth:64,..format}}else{format}
    }
    pub fn format(&self) -> Result<SequenceFormat> {
        Ok(self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?
            .format)
    }
    /// Called once on the output thread after device negotiation, before reading any source PCM.
    /// Reserve next sources here so their primer uses the actual device format.
    pub fn on_configured(
        &self,
        callback: impl FnOnce(PlaybackSequence) -> Result<()> + Send + 'static,
    ) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
        if state.started || state.closed || state.on_configured.is_some() {
            bail!("sequence startup callback already set or started");
        }
        state.on_configured = Some(Box::new(callback));
        Ok(())
    }
    pub fn reserve(&self) -> Result<Preload> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
        if state.closed {
            bail!("playback sequence has ended");
        }
        if state.slots.len() >= 8 {
            bail!("at most eight next tracks may be prepared");
        }
        state.serial += 1;
        let serial = state.serial;
        state.slots.insert(
            serial,
            Slot {
                abort: None,
                ready: None,
                failed: false,
            },
        );
        Ok(Preload {
            sequence: self.clone(),
            generation: state.generation,
            serial,
            format: state.format,
            dsd:state.history.front().unwrap().request.is_dsd(),
            finished: false,
            attempted: false,
        })
    }
    /// A rolling queue may await another bounded reservation window instead of declaring EOF.
    pub fn queue_complete(&self, complete: bool) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
        if state.closed {
            bail!("playback sequence has ended");
        }
        state.queue_complete = complete;
        Ok(())
    }
    /// An exclusive full packet cannot wait forever for a window that requires unheard progress.
    /// End this device run at its last decoded source; the core can open the next source normally.
    #[cfg(any(windows,target_os="android",target_os="macos",test))]
    pub(super) fn finish_window_if_empty(&self) -> bool {
        if let Ok(mut state) = self.state.lock() {
            if !state.queue_complete && state.slots.is_empty() {
                state.queue_complete = true;
                return true;
            }
        }
        false
    }
    /// Replace unsubmitted reservations while keeping the active decoder and device PCM.
    pub fn refresh_queue(&self) -> Result<Vec<String>> {
        let (slots, committed) = {
            let mut state=self.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;
            state.generation+=1;
            if !state.closed {state.queue_complete=false;}
            let committed=state.history.iter().map(|boundary|boundary.id.clone()).collect();
            (std::mem::take(&mut state.slots),committed)
        };
        for slot in slots.into_values() {if let Some(abort)=slot.abort {abort.abort();}}
        Ok(committed)
    }
    /// Invalidates pending queue work without changing already submitted device samples.
    pub fn cancel_next(&self) {
        let slots = if let Ok(mut state) = self.state.lock() {
            state.generation += 1;
            std::mem::take(&mut state.slots)
        } else {
            BTreeMap::new()
        };
        for slot in slots.into_values() {
            if let Some(abort) = slot.abort {
                abort.abort();
            }
        }
    }
    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.cancel_next();
    }
    pub(super) fn guard(&self) -> Result<SequenceRun> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
        if state.claimed || state.started || state.closed {
            bail!("playback sequence cannot be restarted");
        }
        state.claimed = true;
        Ok(SequenceRun(self.clone()))
    }
    /// Read actual heard progress without consuming the output worker's end notifications.
    pub fn position(&self, control: &PlaybackControl) -> Result<SequencePosition> {
        Ok(self.measured(control,false)?.0)
    }
    /// Resolve the exact programme frame paired with a driver clock sample. Retired boundaries fail.
    pub fn position_at_frame(&self,control:&PlaybackControl,played:u64)->Result<SequencePosition> {
        let state=self.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;
        let written=control.frames_written.load(Ordering::Acquire);
        if played>written{bail!("programme clock exceeds submitted frames");}
        #[cfg(feature="adaptive-sync")]
        if state.rate.is_some(){return Ok(rate::checkpoint(&state,control,played,false)?.0);}
        let current=state.history.iter().rev().find(|track|track.start<=played&&(track.start<written||written==0)).context("programme clock boundary retired")?;
        let frames=played.min(current.end.unwrap_or(written)).min(state.programme_end.unwrap_or(written)).saturating_sub(current.start);
        Ok(SequencePosition{playback_id:current.id.clone(),position:current.request.position_after(frames,state.format.sample_rate as f64),frames})
    }
    pub fn source_clock_at_frame(&self,control:&PlaybackControl,played:u64)->Result<SourceClockPosition> {
        let state=self.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;
        if played>control.frames_written.load(Ordering::Acquire)||played>control.programme_end.load(Ordering::Acquire){bail!("source clock exceeds submitted programme frames");}
        #[cfg(feature="adaptive-sync")]
        if state.rate.is_some(){return rate::source_clock(&state,control,played);}
        drop(state);bail!("adaptive source clock is unavailable")
    }
    /// Include completed boundaries still in the worker timeline. No notifications are consumed.
    pub fn checkpoint(&self,control:&PlaybackControl)->Result<(SequencePosition,Vec<CompletedTrack>)> {
        self.measured(control,true)
    }
    fn measured(&self,control:&PlaybackControl,include_ended:bool)->Result<(SequencePosition,Vec<CompletedTrack>)> {
        let state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
        let written = control.frames_written.load(Ordering::Relaxed);
        let played = control.frames_played.load(Ordering::Relaxed).min(written);
        #[cfg(feature="adaptive-sync")]
        if state.rate.is_some(){return rate::checkpoint(&state,control,played,include_ended);}
        let current = state
            .history
            .iter()
            .rev()
            .find(|track| track.start <= played && track.start < written)
            .unwrap_or(&state.history[0]);
        let frames = played
            .min(current.end.unwrap_or(written))
            .min(state.programme_end.unwrap_or(written))
            .saturating_sub(current.start);
        let ended=if include_ended{state.history.iter().filter_map(|track|{
            let end=track.end?;if end>played||track.start>=written{return None;}
            let frames=end.min(state.programme_end.unwrap_or(written)).saturating_sub(track.start);
            Some(CompletedTrack{playback_id:track.id.clone(),position:track.request.position_after(frames,state.format.sample_rate as f64),frames})
        }).collect()}else{Vec::new()};
        Ok((SequencePosition {
            playback_id: current.id.clone(),
            position: current.request.position_after(frames,state.format.sample_rate as f64),
            frames,
        },ended))
    }
    pub(super) fn configure(&self, format: SequenceFormat) -> Result<()> {
        format.validate()?;
        let (slots, callback) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
            if state.started || state.closed {
                bail!("playback sequence cannot be restarted");
            }
            state.started = true;
            let mut previous=state.format;previous.channel_mask=format.channel_mask;
            let slots = if previous != format {
                state.generation += 1;
                std::mem::take(&mut state.slots)
            } else {
                BTreeMap::new()
            };
            state.format=format;
            #[cfg(feature="adaptive-sync")]
            if let Some(rate)=&mut state.rate {rate.configure(format)?;}
            (slots, state.on_configured.take())
        };
        for slot in slots.into_values() {
            if let Some(abort) = slot.abort {
                abort.abort();
            }
        }
        if let Some(callback) = callback {
            callback(self.clone())?;
        }
        Ok(())
    }
    fn next(&self, start:u64) -> Result<Next> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
        let Some((&key, slot)) = state.slots.first_key_value() else {
            if !state.queue_complete {
                return Ok(Next::Pending);
            }
            state.closed = true;
            return Ok(Next::End);
        };
        if slot.failed {
            state.closed = true;
            return Ok(Next::End);
        }
        if slot.ready.is_none() {
            return Ok(Next::Pending);
        }
        let next=state.slots.remove(&key).unwrap().ready.unwrap();
        if let Some(last)=state.history.back_mut() {last.end=Some(start);}
        state.history.push_back(Boundary {
            id:next.id.clone(),request:next.request.clone(),start,end:None,
            #[cfg(target_os="android")] decoder_processing:Vec::new(),
            #[cfg(feature="adaptive-sync")] output_start:None,
            #[cfg(feature="adaptive-sync")] output_end:None,
        });
        Ok(Next::Track(next))
    }
    #[cfg(target_os="android")]
    fn set_decoder_processing(&self,id:&str,processing:Vec<String>)->Result<()> {
        let mut state=self.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;
        let boundary=state.history.iter_mut().find(|boundary|boundary.id==id).context("decoder timeline identity missing")?;
        boundary.decoder_processing=processing;Ok(())
    }
    #[cfg(target_os="android")]
    pub(crate) fn decoder_processing(&self,id:&str)->Result<Vec<String>> {
        let state=self.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;
        Ok(state.history.iter().find(|boundary|boundary.id==id).map(|boundary|boundary.decoder_processing.clone()).unwrap_or_default())
    }
    #[cfg(test)]
    pub(super) fn heard(
        &self,
        control: &PlaybackControl,
        final_end: bool,
    ) -> Result<Heard> {Ok(self.heard_with_frames(control,final_end)?.0)}
    pub(super) fn heard_with_frames(&self,control:&PlaybackControl,final_end:bool)->Result<(Heard,u64)> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
        #[cfg(feature="adaptive-sync")]
        if state.rate.is_some(){return rate::heard(&mut state,control,final_end);}
        let rate = state.format.sample_rate as f64;
        let written = control.frames_written.load(Ordering::Relaxed);
        let played = control.frames_played.load(Ordering::Relaxed).min(written);
        let mut ended = Vec::new();
        while state.history.len() > 1
            && state.history[1].start <= played
            && (state.history[1].start < written || final_end)
        {
            let old = state.history.pop_front().unwrap();
            let frames = old.end.unwrap() - old.start;
            ended.push(CompletedTrack {
                playback_id: old.id,
                position: old.request.position_after(frames,rate),
                frames,
            });
        }
        let programme_end=state.programme_end.unwrap_or(written);
        let current = state
            .history
            .front_mut()
            .context("continuous playback timeline is empty")?;
        let end = current.end.unwrap_or(programme_end);
        let frames = played.min(end).saturating_sub(current.start);
        let position = current.request.position_after(frames,rate);
        if final_end && played == written && current.end.is_none() {
            current.end = Some(programme_end);
            ended.push(CompletedTrack {
                playback_id: current.id.clone(),
                position,
                frames,
            });
        }
        Ok(((current.id.clone(), position, current.request.clone(), ended),frames))
    }
}
pub(super) struct SequenceRun(PlaybackSequence);
impl Drop for SequenceRun {
    fn drop(&mut self) {
        self.0.close();
    }
}
fn validate(request: &PlaybackRequest) -> Result<()> {
    if let Some(rate)=request.dsd_pcm {
        if ![2822400,3072000,5644800,6144000,11289600,12288000,22579200,24576000].contains(&rate)
            ||request.source_integer_lossless||request.source_bit_depth!=32 {
            bail!("invalid DSD to PCM decoding request");
        }
    }
    request.validate_dsd_processing()?;
    if !request.position.is_finite()
        || request.position < 0.
        || request.sample_rate < 8000
        || request.channels == 0
        || request
            .duration
            .is_some_and(|duration| !duration.is_finite() || duration < 0.)
    {
        bail!("invalid queued playback request");
    }
    request.transitions.validate()?;
    Ok(())
}
fn processor(request:&PlaybackRequest,format:SequenceFormat)->Result<DspProcessor> {
    require_loaded_impulse(request)?;
    let mut dsp=DspProcessor::new(request.dsp.clone(),format.sample_rate,format.channels as usize,request.measured_lufs)?;
    if request.dsp.enabled {
        if let Some(impulse)=&request.impulse {
            let impulse=impulse.validate_output(format.sample_rate,format.channels,Some(format.channel_mask))?;
            dsp.attach_convolution(impulse.sample_rate(),impulse.kernels().to_vec())?;
        }
    }Ok(dsp)
}
impl Preload {
    pub fn native_dsd(&self)->bool {self.format.bit_depth==1}
    /// Keep an external probe failure as an ordered barrier rather than cancelling its slot.
    pub fn fail(mut self) {
        self.attempted = true;
    }
    pub fn prepare(self, id: String, request: PlaybackRequest) -> Result<()> {
        self.checked(id, request, None)
    }
    pub fn checked(
        mut self,
        id: String,
        mut request: PlaybackRequest,
        check: Option<CompletionCheck>,
    ) -> Result<()> {
        self.attempted = true;
        validate(&request)?;
        if id.is_empty() {
            bail!("prepared track requires an instance ID");
        }
        if self.dsd!=request.is_dsd(){return Err(PreloadFormatChange.into());}
        let limits=self.sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?.limits.clone();
        let desired_rate=if request.is_dsd(){request.sample_rate}else{limits.rate(request.sample_rate)?};
        let desired_channels=limits.channels.unwrap_or(if cfg!(target_os="android"){request.channels.min(2)}else{request.channels});
        let desired_bits=if request.is_dsd(){request.source_bit_depth}else{request.source_bit_depth.min(limits.max_bit_depth.unwrap_or(32))};
        let rate_matches=desired_rate==self.format.sample_rate||(!request.is_dsd()&&limits.rates(request.sample_rate).contains(&self.format.sample_rate));
        #[cfg(target_os="android")] if self.format.exclusive&&limits.max_bit_depth.is_some_and(|bits|request.source_bit_depth>bits){bail!("USB 原样模式不允许位深转换");}
        if (self.format.exclusive || cfg!(any(target_os="macos",target_os="android")))
            && (!rate_matches
                || desired_channels != self.format.channels
                || desired_bits > self.format.bit_depth)
        {
            return Err(PreloadFormatChange.into());
        }
        DspProcessor::new(
            request.dsp.clone(),
            self.format.sample_rate,
            self.format.channels as usize,
            request.measured_lufs,
        )?;
        if request.dsp.enabled {if let Some(impulse)=&request.impulse {
            let state=self.sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?;
            if state.started {impulse.validate_output(self.format.sample_rate,self.format.channels,Some(state.format.channel_mask))?;}
        }}
        let decode_format=self.sequence.decode_format(self.format);
        let mut decoder = SourceDecoder::open(&request,decode_format,&limits,|abort| {
            let mut state = self
                .sequence
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
            if state.generation != self.generation || !state.slots.contains_key(&self.serial) {
                return Err(PreloadCancelled.into());
            }
            state.slots.get_mut(&self.serial).unwrap().abort = Some(abort);Ok(())
        })?;
        if let Some((position,duration))=decoder.source_time(){request.position=position;request.duration=Some(duration);}
        let frame_bytes = decode_format.frame_bytes(&request);
        // Half a second, capped at 16 MiB per track. FFmpeg's pipe supplies bounded backpressure.
        let frames = (self.format.sample_rate as usize / 2)
            .min(16 * 1024 * 1024 / frame_bytes)
            .max(1);
        let mut primer = vec![0; frames * frame_bytes];
        let decoded = decoder.read_frames(&mut primer, frame_bytes);
        let mut state = self
            .sequence
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
        if state.generation != self.generation || !state.slots.contains_key(&self.serial) {
            return Err(PreloadCancelled.into());
        }
        let actual = decoded?;
        primer.truncate(actual * frame_bytes);
        state.slots.get_mut(&self.serial).unwrap().ready = Some(Prepared {
            id,
            request,
            decoder,
            primer,
            cursor: 0,
            check,
            eof_verified: false,
        });
        self.finished = true;
        Ok(())
    }
}
impl Drop for Preload {
    fn drop(&mut self) {
        if !self.finished {
            let slot = self.sequence.state.lock().ok().and_then(|mut state| {
                if self.attempted {
                    state
                        .slots
                        .get_mut(&self.serial)
                        .and_then(|slot| {
                            slot.failed = true;
                            slot.abort.take()
                        })
                        .map(|abort| Slot {
                            abort: Some(abort),
                            ready: None,
                            failed: true,
                        })
                } else {
                    state.slots.remove(&self.serial)
                }
            });
            if let Some(slot) = slot {
                if let Some(abort) = slot.abort {
                    abort.abort();
                }
            }
        }
    }
}
enum Next {
    Track(Prepared),
    Pending,
    End,
}
pub(super) enum Read {
    Data(Chunk),
    Pending,
    End,
}
pub(super) struct Chunk {
    pub bytes: Vec<u8>,
    pub frames: usize,
    pub meter: Meter,
}
fn output_bytes(format:SequenceFormat,values:&[f64],gain:impl Fn(usize)->f64)->Vec<u8> {
    let mut output=Vec::with_capacity(values.len()*if format.float_output&&format.bit_depth==64{8}else{4});
    for (i,value) in values.iter().enumerate() {
        if format.float_output&&format.bit_depth==64 {output.extend_from_slice(&(*value*gain(i)).clamp(-1.,1.).to_le_bytes());}
        else if format.float_output {let mut value=*value*gain(i);if format.bit_depth<32{let scale=(1_u64<<(format.bit_depth-1)) as f64;value=(value*scale).round().clamp(-scale,scale-1.)/scale;}output.extend_from_slice(&(value.clamp(-1.,1.) as f32).to_le_bytes());}
        else {let shift=32-format.bit_depth.min(32);let scale=(1_u64<<(format.bit_depth.min(32)-1)) as f64;let quantized=(*value*gain(i)*scale).round().clamp(-scale,scale-1.) as i64;output.extend_from_slice(&((quantized<<shift) as i32).to_le_bytes());}
    }
    output
}
pub(super) struct PcmStream {
    analyzer: Option<crate::visualization::Analyzer>,
    current: Prepared,
    sequence: PlaybackSequence,
    control: Arc<PlaybackControl>,
    format: SequenceFormat,
    decoded: u64,
    track_frames: u64,
    dsp: DspProcessor,
    dsp_config: DspConfig,
    max_bits:Option<u16>,
    impulse:Option<Arc<crate::impulse::ImpulseBank>>,
    draining:bool,
    decode_format:SequenceFormat,
    #[cfg(feature="adaptive-sync")]
    rate_buffer:Option<rate::Buffer>,
}
impl PcmStream {
    pub fn open(
        mut request: PlaybackRequest,
        sequence: PlaybackSequence,
        control: Arc<PlaybackControl>,
        format: SequenceFormat,
    ) -> Result<Self> {
        if sequence
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?
            .history[0]
            .request
            != request
        {
            bail!("initial source does not match its playback timeline");
        }
        sequence.configure(format)?;
        validate(&request)?;
        let result = (|| -> Result<Self> {
            let dsp = processor(&request,format)?;
            let dsp_config = request.dsp.clone();
            let impulse=request.impulse.clone();
            let decode_format=sequence.decode_format(format);
            let limits=sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?.limits.clone();
            let decoder = SourceDecoder::open(&request,decode_format,&limits,|abort| {
            control.attach_decoder(abort)?;
            if control.stopped.load(Ordering::Acquire){control.stop();bail!("DSD/PCM preparation cancelled");}Ok(())
            })?;
            if let Some((position,duration))=decoder.source_time(){request.position=position;request.duration=Some(duration);}
            let (id, check) = {
                let mut state = sequence
                    .state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("playback sequence lock poisoned"))?;
                state.history[0].request=request.clone();
                (state.history[0].id.clone(), state.initial_check.take())
            };
            let max_bits=sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?.limits.max_bit_depth;
            Ok(Self {
                analyzer: None,
                current: Prepared {
                    id,
                    request,
                    decoder,
                    primer: Vec::new(),
                    cursor: 0,
                    check,
                    eof_verified: false,
                },
                sequence: sequence.clone(),
                control,
                format,
                decoded: 0,
                track_frames: 0,
                dsp,
                dsp_config,
                max_bits,
                impulse,
                draining:false,
                decode_format,
                #[cfg(feature="adaptive-sync")]
                rate_buffer:if sequence.rate_adjustment_active(){Some(rate::Buffer::new(&sequence)?)}else{None},
            })
        })();
        if result.is_err() {
            sequence.close();
        }
        result
    }
    /// Returns a single track's segment; callers can fill one device packet from multiple segments.
    pub fn read(&mut self, max_frames: usize) -> Result<Read> {
        if max_frames==0 {bail!("PCM output buffer must contain at least one frame");}
        #[cfg(feature="adaptive-sync")]
        if let Some(mut buffer)=self.rate_buffer.take(){let result=buffer.read(self,max_frames);self.rate_buffer=Some(buffer);return result;}
        self.read_raw(max_frames)
    }
    fn encode(&self,values:&[f64],gain:impl Fn(usize)->f64)->Vec<u8> {
        if self.sequence.rate_adjustment_active() {values.iter().enumerate().flat_map(|(i,value)|(value*gain(i)).to_le_bytes()).collect()}
        else {output_bytes(SequenceFormat{bit_depth:self.format.bit_depth.min(self.max_bits.unwrap_or(64)),..self.format},values,gain)}
    }
    fn read_raw(&mut self, max_frames: usize) -> Result<Read> {
        if max_frames == 0 {
            bail!("PCM output buffer must contain at least one frame");
        }
        loop {
            if self.control.stopped.load(Ordering::Acquire) {
                return Ok(Read::End);
            }
            if self.draining {
                let(values,meter)=self.dsp.drain_convolution(max_frames);
                if values.is_empty(){return Ok(Read::End);}
                let frames=values.len()/self.format.channels as usize;let volume=self.control.volume();
                return Ok(Read::Data(Chunk{bytes:self.encode(&values,|_|volume),frames,meter}));
            }
            if self.current.request.dsp.enabled && self.current.request.dsp.normalize {
                let id=&self.current.id;
                if let Some((measured,peak))=self.control.take_loudness(id)? {
                    let limited=self.current.request.dsp.peak_limited_measurement(Some(measured),peak);
                    if limited!=self.current.request.measured_lufs {
                        if self.track_frames == 0 { self.dsp.update_loudness(limited); }
                        else { self.dsp.smooth_loudness(measured,peak); }
                        self.current.request.measured_lufs=limited;
                        if let Some(boundary)=self.sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?.history.iter_mut().find(|b|&b.id==id){boundary.request.measured_lufs=limited;}
                    }
                }
            }
            let request = &self.current.request;
            let frame_bytes = self.decode_format.frame_bytes(request);
            let mut bytes = vec![0; max_frames * frame_bytes];
            let frames = if self.current.eof_verified {
                0
            } else if self.current.cursor < self.current.primer.len() {
                let available = (self.current.primer.len() - self.current.cursor) / frame_bytes;
                let frames = max_frames.min(available);
                let count = frames * frame_bytes;
                bytes[..count].copy_from_slice(
                    &self.current.primer[self.current.cursor..self.current.cursor + count],
                );
                self.current.cursor += count;
                frames
            } else {
                self.current.decoder.read_frames(&mut bytes, frame_bytes)?
            };
            if frames > 0 {
                #[cfg(target_os="android")]
                if let SourceDecoder::Android(decoder)=&self.current.decoder {
                    self.sequence.set_decoder_processing(&self.current.id,decoder.processing())?;
                }
                if request.is_dsd() {
                    if self.control.volume_setting()!=1. {bail!("原样 DSD 不支持软件音量；请使用 DAC 音量控制");}
                    bytes.truncate(frames*frame_bytes);self.decoded+=frames as u64;self.track_frames+=frames as u64;
                    return Ok(Read::Data(Chunk{bytes,frames,meter:Meter::default()}));
                }
                let volume = self.control.volume();
                let channels = self.format.channels as usize;
                let track_frames=self.track_frames;let rate=self.format.sample_rate;let convolution=self.dsp.has_convolution();
                let envelope=|frame:usize|request.transitions.gain(request.position+(track_frames+frame as u64) as f64/rate as f64,request.duration);
                let gain=|i:usize|volume*if convolution{1.}else{envelope(i/channels)};
                let (values, meter) = if self.decode_format.decode_float(request) {
                    let mut values = bytes[..frames * frame_bytes]
                        .chunks_exact(8)
                        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
                        .collect::<Vec<_>>();
                    let meter = if convolution {self.dsp.process_enveloped(&mut values,envelope)?}else{self.dsp.process(&mut values)?};
                    (values, meter)
                } else {
                    let mut values = bytes[..frames * frame_bytes]
                        .chunks_exact(4)
                        .map(|b| i32::from_le_bytes(b.try_into().unwrap()) as f64 / 2147483648.)
                        .collect::<Vec<_>>();
                    let meter = if request.dsp.enabled {self.dsp.process(&mut values)?}else{Meter {
                        peak: values.iter().map(|v| v.abs()).fold(0., f64::max),
                        ..Default::default()
                    }};
                    (values, meter)
                };
                if self.control.visualization.active() {
                    self.analyzer.get_or_insert_with(Default::default).feed(&self.control.visualization,&self.current.id,request.position+self.track_frames as f64/self.format.sample_rate as f64,self.format.sample_rate,channels,&values);
                } else { self.analyzer=None; }
                let output=self.encode(&values,gain);
                self.decoded += frames as u64;
                self.track_frames += frames as u64;
                return Ok(Read::Data(Chunk {
                    bytes: output,
                    frames,
                    meter,
                }));
            }
            if !self.current.eof_verified {
                if let Some(check) = &self.current.check {
                    check(
                        self.current.request.position_after(self.track_frames,self.format.sample_rate as f64),
                    )?;
                }
                self.current.eof_verified = true;
            }
            match self.sequence.next(self.decoded)? {
                Next::Pending => return Ok(Read::Pending),
                Next::End => {
                    self.sequence.state.lock().map_err(|_|anyhow::anyhow!("playback sequence lock poisoned"))?.programme_end=Some(self.decoded);
                    if !self.sequence.rate_adjustment_active(){self.control.programme_end.store(self.decoded,Ordering::Release);}
                    self.draining=true;
                },
                Next::Track(next) => {
                    if self.dsp_config != next.request.dsp||self.impulse!=next.request.impulse {
                        self.dsp = processor(&next.request,self.format)?;
                        self.dsp_config = next.request.dsp.clone();
                        self.impulse=next.request.impulse.clone();
                    } else {
                        self.dsp.update_loudness(next.request.measured_lufs);
                    }
                    self.control.attach_decoder(next.decoder.abort_handle())?;
                    self.current = next;
                    self.track_frames = 0;
                }
            }
        }
    }
}
impl Drop for PcmStream {
    fn drop(&mut self) {
        self.sequence.close();
        if let Ok(mut decoder) = self.control.decoder.lock() {
            decoder.take();
        }
    }
}
#[cfg(test)]
#[path = "sequence_tests.rs"]
mod tests;

#[cfg(test)] mod precision_tests {
    use super::*;
    #[test] fn closure_effective_precision_quantizes_after_gain_without_changing_frames(){
        let format=SequenceFormat{sample_rate:48000,channels:2,channel_mask:0,exclusive:true,float_output:false,bit_depth:16};
        let values=[0.123456,-0.123456,0.9,-0.9];let bytes=output_bytes(format,&values,|_|0.5);
        assert_eq!(bytes.len(),16);for value in bytes.chunks_exact(4){assert_eq!(i32::from_le_bytes(value.try_into().unwrap())&65535,0);}
        let float=output_bytes(SequenceFormat{float_output:true,..format},&values,|_|0.5);
        for value in float.chunks_exact(4){let sample=f32::from_le_bytes(value.try_into().unwrap()) as f64;assert_eq!((sample*32768.).fract(),0.);}
    }
}
