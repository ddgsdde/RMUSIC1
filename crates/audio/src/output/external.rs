//! Host-supplied native drivers. The shared library owns decoding and programme accounting;
//! a platform workspace owns device I/O. No UI, service process, or platform SDK lives here.
use super::{buffered::ReadAhead, sequence::{PcmStream, Read}, *};
use std::{sync::OnceLock, time::Duration};
mod buffering;
mod feed;
mod clock_watch;
use buffering::Buffering;
use feed::Feed;

pub struct ProgrammeClock {pub device_frame:u64,pub programme_frames:u64,pub time_ns:u64,pub before_ns:u64,pub after_ns:u64}
pub struct Progress {
    pub programme_clock:Option<ProgrammeClock>,
    pub queued: u64,
    pub capacity: u32,
    pub quantum: u32,
    pub played: u64,
    pub device_frame: u64,
    pub device_time_ns: u64,
    pub host_before_ns: u64,
    pub host_after_ns: u64,
    pub running: bool,
    pub continuity_lost: bool,
}
/// Methods run only on the output worker. Device callbacks must never call this trait.
pub trait Driver {
    fn format(&self) -> SequenceFormat;
    fn path(&self, source: &PlaybackRequest, volume: f64) -> SignalPath;
    fn write(&mut self, bytes: &[u8]) -> Result<usize>;
    /// Publish EOF with the last submitted packet, before its final frames become visible.
    fn write_final(&mut self, bytes: &[u8]) -> Result<usize> {
        let frames=self.write(bytes)?;
        let format=self.format();
        let width=usize::from(format.channels)*if format.float_output&&format.bit_depth==64{8}else{4};
        if frames*width==bytes.len(){self.finish_input();}Ok(frames)
    }
    fn start(&mut self) -> Result<()>;
    fn start_at(&mut self,_:u64)->Result<()> {bail!("此原生驱动不支持计划起播")}
    fn pause(&mut self, paused: bool);
    /// User transport pause may release exclusive hardware; source rebuffering must not.
    fn user_pause(&mut self, _paused: bool) -> Result<()> { Ok(()) }
    fn finish_input(&mut self);
    fn progress(&mut self) -> Result<Progress>;
    /// Yield to native consumption when the backend provides a callback wake.
    /// The default retains the polling contract for existing external fixtures.
    fn wait_for_output(&mut self) -> Result<()> {
        std::thread::sleep(Duration::from_millis(2)); Ok(())
    }
    /// End a platform's ready-PCM deadline before allocating metadata or reporting.
    fn finish_cycle(&mut self) {}
}
pub trait Backend: Send + Sync {
    fn scheduled(&self)->bool {false}
    fn devices(&self) -> Result<Vec<DeviceInfo>>;
    fn open(&self, request: &PlaybackRequest, config: &OutputConfig) -> Result<Box<dyn Driver>>;
    fn prepare_decode_thread(&self) {}
}
static BACKEND: OnceLock<Box<dyn Backend>> = OnceLock::new();
pub fn install(backend: Box<dyn Backend>) -> Result<()> {
    BACKEND.set(backend).map_err(|_| anyhow::anyhow!("native backend already installed"))
}
pub fn available() -> bool { BACKEND.get().is_some() }
pub fn scheduled_available()->bool {BACKEND.get().is_some_and(|b|b.scheduled())}
pub(super) fn backend() -> Result<&'static dyn Backend> {
    BACKEND.get().map(Box::as_ref).ok_or_else(|| anyhow::anyhow!("native backend not installed"))
}
pub(super) fn play_sequence(
    request: PlaybackRequest, config: OutputConfig, control: Arc<PlaybackControl>,
    sequence: PlaybackSequence, mut update: impl FnMut(PlaybackUpdate),
) -> Result<()> {
    if request.is_dsd() { bail!("此 CoreAudio 开发后端尚未开放 DoP／原生 DSD"); }
    let scheduled=control.start.setup(false)?;
    if scheduled&&!scheduled_available(){bail!("此原生驱动不支持计划起播");}
    let _run = sequence.guard()?;
    control.initialize_volume(request.volume)?;
    if control.stopped.load(Ordering::Acquire) { return Ok(()); }
    let mut driver = backend()?.open(&request, &config)?;
    let format = driver.format();
    control.output_sample_rate.store(format.sample_rate, Ordering::Release);
    config.resync(&control)?;
    let stream = PcmStream::open(request, sequence.clone(), control.clone(), format)?;
    let mut stream = ReadAhead::new(stream, control.clone(), format)?;
    let feedback = clock::Session::new(control.clone());
    let mut feed = Feed::new(format);
    let mut started = false;
    let mut started_at = None;
    let mut clock_watch = clock_watch::ClockWatch::default();
    let mut reported_gap=false;
    let mut buffering=Buffering::new(format.sample_rate,config.buffer_ms);
    let mut last_report=std::time::Instant::now()-Duration::from_secs(1);
    let mut reported_buffering=false;
    let mut max_progress=Duration::ZERO;let mut max_pump=Duration::ZERO;let mut max_wait=Duration::ZERO;
    let mut loop_at=std::time::Instant::now();let mut max_loop_gap=Duration::ZERO;let mut max_report=Duration::ZERO;
    let diagnostic=std::env::var_os("RMUSIC_AUDIO_DIAGNOSTICS").or_else(||std::env::var_os("RMUSIC_PLAYBACK_DIAGNOSTICS")).is_some();
    let mut last_played=0;let mut advanced_at=std::time::Instant::now();let mut waiting_reported=false;
    let mut user_paused=false;
    let result = (|| -> Result<()> {
        loop {
            max_loop_gap=max_loop_gap.max(loop_at.elapsed());loop_at=std::time::Instant::now();
            if control.stopped.load(Ordering::Acquire) { break; }
            let paused = control.paused.load(Ordering::Acquire);
            if user_paused!=paused {driver.user_pause(paused)?;user_paused=paused;}
            driver.pause(paused || buffering.active());
            let progress_at=std::time::Instant::now();
            let progress = driver.progress()?;
            max_progress=max_progress.max(progress_at.elapsed());
            if diagnostic {
                if paused || progress.played!=last_played {
                    if waiting_reported {eprintln!("CoreAudio programme progress resumed: played={} paused={paused}",progress.played);}
                    advanced_at=std::time::Instant::now();waiting_reported=false;
                } else if !waiting_reported && advanced_at.elapsed()>=Duration::from_secs(2) {
                    let source=stream.diagnostics();
                    eprintln!("CoreAudio programme waiting: current={} started={started} buffering={} played={} written={} queued={} capacity={} pending_bytes={} eof={} waiting_window={} buffered_packets={} reading_us={}",sequence.position(&control).map(|p|p.playback_id).unwrap_or_default(),buffering.active(),progress.played,control.frames_written.load(Ordering::Acquire),progress.queued,progress.capacity,feed.pending_bytes(),feed.ended(),stream.waiting_for_next(),source.buffered_packets,source.reading_us);
                    waiting_reported=true;
                }
                last_played=progress.played;
            }
            if buffering.timed_out(paused,std::time::Instant::now()) {
                return Err(SourceUnavailable("音源缓冲等待超时；已保留播放位置，请检查连接后恢复".into()).into());
            }
            if buffering.resume(progress.queued,progress.capacity,progress.quantum,feed.ended()) {
                eprintln!("CoreAudio rebuffering resumed without discarding programme frames");
                driver.pause(paused);
            }
            let written = control.frames_written.load(Ordering::Acquire);
            if progress.continuity_lost&&!reported_gap {
                let source=stream.diagnostics();
                reported_gap=true;eprintln!("CoreAudio feed diagnostic: played={} written={} pending_bytes={} eof={} waiting_window={} max_loop_gap_us={} max_report_us={} max_progress_us={} max_pump_us={} max_wait_us={} buffered_packets={} reading_us={} max_read_us={}",progress.played,written,feed.pending_bytes(),feed.ended(),stream.waiting_for_next(),max_loop_gap.as_micros(),max_report.as_micros(),max_progress.as_micros(),max_pump.as_micros(),max_wait.as_micros(),source.buffered_packets,source.reading_us,source.max_read_us);
            }
            if progress.played > written { bail!("native output cursor exceeds submitted frames"); }
            control.frames_played.fetch_max(progress.played, Ordering::Release);
            control.clock_ns.store(progress.device_time_ns, Ordering::Relaxed);
            let clock_age = (progress.device_time_ns != 0)
                .then(|| clock::host_ns().and_then(|now| now.checked_sub(progress.host_after_ns)))
                .flatten().map(Duration::from_nanos);
            if paused {clock_watch=Default::default();if started {started_at=Some(std::time::Instant::now());}}
            if !paused && started_at.is_some_and(|time:std::time::Instant|time.elapsed()>Duration::from_secs(2))
                && clock_watch.stalled(std::time::Instant::now(), clock_age) {
                return Err(DeviceClockInterrupted("CoreAudio 回调持续停止，已保留播放位置".into()).into());
            }
            if let Some(capture)=progress.programme_clock {
                feedback.record_programme(clock::Source::CoreAudioFrames,capture.device_frame,u64::from(format.sample_rate),capture.time_ns,clock::Domain::MachAbsolute,Some(capture.before_ns),Some(capture.after_ns),progress.running,Some(clock::Programme{frames:capture.programme_frames,sample_rate:format.sample_rate}));
            }else if progress.device_time_ns>0 {
                feedback.record_programme(clock::Source::CoreAudioFrames,progress.device_frame,u64::from(format.sample_rate),progress.device_time_ns,clock::Domain::MachAbsolute,Some(progress.host_before_ns),Some(progress.host_after_ns),progress.running,None);
            }
            // Refill ready PCM before timeline/report work can consume its deadline.
            // Keep the existing ring capacity, decoding and sample bytes unchanged.
            if !paused {
                let pump_at=std::time::Instant::now();
                let room=u64::from(progress.capacity).saturating_sub(progress.queued);
                if feed.pump(driver.as_mut(),&mut stream,&sequence,&control,room)? && started {
                    let current=driver.progress()?;
                    if buffering.wait(current.queued,current.capacity,current.quantum,std::time::Instant::now()) {
                        driver.pause(true); eprintln!("CoreAudio rebuffering: source data pending");
                    }
                }
                if !started && Buffering::primed(control.frames_written.load(Ordering::Acquire), progress.capacity, feed.ended()) {
                    if scheduled {
                        driver.finish_cycle();
                        let Some(deadline)=control.start.wait_for_commit(&control)? else{break};
                        driver.start_at(deadline)?;
                        if !control.start.wait(&control)?{break;}
                    }else{driver.start()?;}
                    started = true; started_at=Some(std::time::Instant::now());
                }
                max_pump=max_pump.max(pump_at.elapsed());
            }
            driver.finish_cycle();
            let final_end = feed.submitted_all() && progress.played == control.frames_written.load(Ordering::Acquire);
            if final_end || reported_buffering!=buffering.active() || last_report.elapsed()>=Duration::from_millis(20) {
            let report_at=std::time::Instant::now();
            let ((id, position, source, completed), output_frames) = sequence.heard_with_frames(&control, final_end)?;
            let mut signal_path = driver.path(&source, control.volume());
            config.limits.signal_path(&source, &mut signal_path);
            if buffering.active() { signal_path.processing.push("Playback buffering: source data pending".into()); }
            if progress.continuity_lost { signal_path.processing.push("CoreAudio underrun; continuity lost".into()); }
            update(PlaybackUpdate { playback_id: id, position, ended: completed, output_frames,
                signal_path, meter: feed.meter(), continuity_lost: progress.continuity_lost });
            reported_buffering=buffering.active();
            last_report=std::time::Instant::now();
            max_report=max_report.max(report_at.elapsed());
            }
            if final_end { break; }
            let wait_at=std::time::Instant::now();
            driver.wait_for_output()?;
            max_wait=max_wait.max(wait_at.elapsed());
        }
        Ok(())
    })();
    // Stop decoder and join its bounded read-ahead before device teardown, on every exit path.
    driver.finish_cycle();driver.pause(true);
    if diagnostic {
        eprintln!("CoreAudio feed completed: max_loop_gap_us={} max_report_us={} max_progress_us={} max_pump_us={} max_wait_us={} continuity_lost={reported_gap}",max_loop_gap.as_micros(),max_report.as_micros(),max_progress.as_micros(),max_pump.as_micros(),max_wait.as_micros());
    }
    drop(stream);
    drop(driver);
    result.map_err(|error| {
        if buffering.active() && error.is::<crate::decoder::DecodeFailure>() {
            SourceUnavailable(format!("音源在缓冲时中断，已保留播放位置：{error}")).into()
        } else { error }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Captured { bytes: Vec<u8>, started: bool }
    struct FakeBackend(Arc<Mutex<Captured>>);
    struct FakeDriver { captured: Arc<Mutex<Captured>>, format: SequenceFormat }
    impl Backend for FakeBackend {
        fn devices(&self)->Result<Vec<DeviceInfo>> { Ok(vec![]) }
        fn open(&self,request:&PlaybackRequest,_:&OutputConfig)->Result<Box<dyn Driver>> {
            Ok(Box::new(FakeDriver{captured:self.0.clone(),format:SequenceFormat{sample_rate:request.sample_rate,channels:2,channel_mask:3,exclusive:false,float_output:false,bit_depth:32}}))
        }
    }
    impl Driver for FakeDriver {
        fn format(&self)->SequenceFormat {self.format}
        fn path(&self,_:&PlaybackRequest,_:f64)->SignalPath {SignalPath{output:"memory fixture".into(),sample_rate:self.format.sample_rate,channels:2,exclusive:false,bit_depth:32,sample_type:"integer".into(),mixer_sample_rate:None,bit_perfect_eligible:false,processing:vec![]}}
        fn write(&mut self,bytes:&[u8])->Result<usize> {self.captured.lock().unwrap().bytes.extend(bytes);Ok(bytes.len()/8)}
        fn start(&mut self)->Result<()> {self.captured.lock().unwrap().started=true;Ok(())}
        fn pause(&mut self,_:bool) {}
        fn finish_input(&mut self) {}
        fn progress(&mut self)->Result<Progress> {let c=self.captured.lock().unwrap();Ok(Progress{programme_clock:None,queued:0,capacity:4800,quantum:512,played:if c.started{c.bytes.len() as u64/8}else{0},device_frame:0,device_time_ns:0,host_before_ns:0,host_after_ns:0,running:c.started,continuity_lost:false})}
    }
    #[test]
    fn external_natural_eof_keeps_transport_alive_and_preserves_integer_samples() {
        let root=tempfile::tempdir().unwrap();let file=root.path().join("source.wav");
        let samples:[i16;8]=[1,-1,16384,-16384,32767,-32768,0,256];
        let mut wav=b"RIFF".to_vec();wav.extend(52u32.to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());wav.extend(2u16.to_le_bytes());wav.extend(48000u32.to_le_bytes());wav.extend(192000u32.to_le_bytes());wav.extend(4u16.to_le_bytes());wav.extend(16u16.to_le_bytes());wav.extend(b"data");wav.extend(16u32.to_le_bytes());
        for value in samples {wav.extend(value.to_le_bytes());}std::fs::write(&file,wav).unwrap();
        let captured=Arc::new(Mutex::new(Captured::default()));install(Box::new(FakeBackend(captured.clone()))).unwrap();
        let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:file.to_string_lossy().into_owned(),position:0.,sample_rate:48000,channels:2,source_integer_lossless:true,source_bit_depth:16,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(4./48000.),transitions:Default::default()};
        let format=SequenceFormat{sample_rate:48000,channels:2,channel_mask:3,exclusive:false,float_output:false,bit_depth:32};
        let sequence=PlaybackSequence::new("fixture".into(),request.clone(),format).unwrap();
        sequence.queue_complete(false).unwrap();
        let control=Arc::new(PlaybackControl::default());
        let weak=Arc::downgrade(&control);let done=Arc::new(std::sync::atomic::AtomicBool::new(false));let finished=done.clone();
        let deadline=std::thread::spawn(move||{for _ in 0..100{if finished.load(Ordering::Acquire){return;}std::thread::sleep(Duration::from_millis(10));}if let Some(control)=weak.upgrade(){control.stop();}});
        let mut ended=Vec::new();
        super::super::play_sequence(request.clone(),OutputConfig::default(),control.clone(),sequence,|event|ended.extend(event.ended)).unwrap();
        done.store(true,Ordering::Release);deadline.join().unwrap();
        assert!(!control.stopped.load(Ordering::Acquire),"natural EOF must reach the bridge Finished report");
        assert_eq!(control.frames_played.load(Ordering::Acquire),4);
        assert_eq!(ended.len(),1);assert_eq!(ended[0].frames,4);
        let expected:Vec<u8>=samples.into_iter().flat_map(|value|((value as i32)<<16).to_le_bytes()).collect();
        assert_eq!(captured.lock().unwrap().bytes,expected);

        *captured.lock().unwrap()=Captured::default();
        let bad=root.path().join("bad.wav");std::fs::write(&bad,b"unreadable audio fixture").unwrap();
        let invalid=PlaybackRequest{input:bad.to_string_lossy().into_owned(),..request.clone()};
        let sequence=PlaybackSequence::new("bad".into(),invalid.clone(),format).unwrap();sequence.queue_complete(false).unwrap();
        assert!(super::super::play_sequence(invalid,OutputConfig::default(),Arc::new(PlaybackControl::default()),sequence,|_|{}).is_err());

        *captured.lock().unwrap()=Captured::default();
        let ogg=root.path().join("quad.ogg");
        std::fs::write(&ogg,include_bytes!("../../tests/fixtures/quad-vorbis.ogg")).unwrap();
        let quad=PlaybackRequest{input:ogg.to_string_lossy().into_owned(),sample_rate:44100,channels:4,source_integer_lossless:false,source_bit_depth:0,duration:Some(0.05),..request};
        let sequence=PlaybackSequence::new("quad".into(),quad.clone(),SequenceFormat{sample_rate:44100,..format}).unwrap();sequence.queue_complete(false).unwrap();
        let next=Arc::new(PlaybackControl::default());
        super::super::play_sequence(quad,OutputConfig::default(),next.clone(),sequence,|_|{}).unwrap();
        assert!(next.frames_played.load(Ordering::Acquire)>0,"a bad source must not prevent the next four-channel OGG from decoding to the stereo output");
        assert!(!next.stopped.load(Ordering::Acquire));
    }
}
