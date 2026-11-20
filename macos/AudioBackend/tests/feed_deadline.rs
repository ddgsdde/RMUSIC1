//! Explicit software jitter injection; only the BlackHole UID is ever opened.
use anyhow::Result;
use rmusic_audio::output::{self,external::{Backend,Driver,Progress},*};
use std::{sync::{Arc,Mutex},time::{Duration,SystemTime}};

struct DelayedBackend(Arc<Mutex<Vec<u8>>>);
struct DelayedDriver {driver:Box<dyn Driver>,captured:Arc<Mutex<Vec<u8>>>}
impl DelayedDriver {
    fn capture(&self,bytes:&[u8],frames:usize) {
        let format=self.driver.format();let width=usize::from(format.channels)*if format.float_output&&format.bit_depth==64{8}else{4};
        self.captured.lock().unwrap().extend_from_slice(&bytes[..frames*width]);
    }
}
impl Backend for DelayedBackend {
    fn devices(&self)->Result<Vec<DeviceInfo>> {rmusic_coreaudio::CoreAudio.devices()}
    fn prepare_decode_thread(&self) {rmusic_coreaudio::CoreAudio.prepare_decode_thread();}
    fn open(&self,request:&PlaybackRequest,config:&OutputConfig)->Result<Box<dyn Driver>> {
        Ok(Box::new(DelayedDriver{driver:rmusic_coreaudio::CoreAudio.open(request,config)?,captured:self.0.clone()}))
    }
}
impl Driver for DelayedDriver {
    fn format(&self)->SequenceFormat {self.driver.format()}
    fn path(&self,source:&PlaybackRequest,volume:f64)->SignalPath {self.driver.path(source,volume)}
    fn write(&mut self,bytes:&[u8])->Result<usize> {
        let frames=self.driver.write(bytes)?;self.capture(bytes,frames);Ok(frames)
    }
    fn write_final(&mut self,bytes:&[u8])->Result<usize> {
        let frames=self.driver.write_final(bytes)?;self.capture(bytes,frames);Ok(frames)
    }
    fn start(&mut self)->Result<()> {self.driver.start()}
    fn pause(&mut self,paused:bool) {self.driver.pause(paused)}
    fn wait_for_output(&mut self)->Result<()> {self.driver.wait_for_output()}
    fn finish_cycle(&mut self) {self.driver.finish_cycle()}
    fn finish_input(&mut self) {self.driver.finish_input()}
    fn progress(&mut self)->Result<Progress> {
        // A delayed producer wake/progress cycle. The actual HAL keeps running;
        // the PCM source and driver capacity are identical in the A/B runs.
        std::thread::sleep(Duration::from_millis(16));self.driver.progress()
    }
}
struct Fixture(std::path::PathBuf);
impl Drop for Fixture {fn drop(&mut self) {let _=std::fs::remove_file(&self.0);}}
#[test]
#[ignore = "opt-in actual BlackHole HAL; never a physical output"]
fn ready_pcm_survives_delayed_producer_cycles_without_larger_buffer() {
    let stamp=SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
    let file=Fixture(std::env::temp_dir().join(format!("rmusic-feed-{}-{stamp}.wav",std::process::id())));
    let frames=48000u32*5;let bytes=frames*6;let mut expected=Vec::with_capacity(frames as usize*8);let mut floating=Vec::with_capacity(frames as usize*16);
    let mut wav=b"RIFF".to_vec();wav.extend((bytes+36).to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());wav.extend(2u16.to_le_bytes());wav.extend(48000u32.to_le_bytes());wav.extend(288000u32.to_le_bytes());wav.extend(6u16.to_le_bytes());wav.extend(24u16.to_le_bytes());wav.extend(b"data");wav.extend(bytes.to_le_bytes());
    for frame in 0..frames {let value=((frame*7919)%131071)as i32-65535;
        for sample in [value,-value] {wav.extend(&sample.to_le_bytes()[..3]);expected.extend((sample<<8).to_le_bytes());floating.extend((f64::from(sample)/8388608.).to_le_bytes());}
    }
    std::fs::write(&file.0,wav).unwrap();
    let captured=Arc::new(Mutex::new(Vec::with_capacity(expected.len())));
    output::external::install(Box::new(DelayedBackend(captured.clone()))).unwrap();
    let mut request=PlaybackRequest{flac_samples:None,dsd_pcm:None,input:file.0.to_string_lossy().into(),position:0.,sample_rate:48000,channels:2,source_integer_lossless:true,source_bit_depth:24,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(5.),transitions:Default::default()};
    let config=OutputConfig{device_id:Some("BlackHole2ch_UID".into()),buffer_ms:100,..Default::default()};
    for float_output in [false,true] {
        captured.lock().unwrap().clear();request.dsp.enabled=float_output;
        let format=SequenceFormat{sample_rate:48000,channels:2,channel_mask:3,exclusive:false,float_output,bit_depth:if float_output{64}else{32}};
        let sequence=PlaybackSequence::new("jitter-qa".into(),request.clone(),format).unwrap();sequence.queue_complete(false).unwrap();
        let control=Arc::new(PlaybackControl::default());let mut lost=false;let mut final_frames=0;
        output::play_sequence(request.clone(),config.clone(),control.clone(),sequence,|event|{lost|=event.continuity_lost;for end in event.ended {final_frames+=end.frames;}}).unwrap();
        assert_eq!(final_frames,u64::from(frames));assert_eq!(control.frames_played.load(std::sync::atomic::Ordering::Acquire),u64::from(frames));
        assert!(!lost,"ready PCM must refill all available driver room before another delayed cycle");
        let reference=if float_output{&floating}else{&expected};let actual=captured.lock().unwrap();
        assert_eq!(actual.len(),reference.len());assert!(actual.as_slice()==reference.as_slice(),"PCM24 / unity f64 DSP samples must be exact");
        eprintln!("HAL PCM reference exact: float64={float_output} frames={frames} bytes={}",actual.len());
        assert_eq!(Arc::strong_count(&control),1,"read-ahead and output owners must be joined");
    }
}
