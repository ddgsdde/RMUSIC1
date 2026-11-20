//! Raw ALSA DSD words; source accounting remains in eight-bit frames.
use super::*;
use crate::output::sequence::{PcmStream,Read};

#[derive(Clone,Copy)]
struct Packing {format:Format,width:usize,big_endian:bool}
impl Packing {
    fn new(format:Format)->Self {
        let (width,big_endian)=match format {Format::DSDU8=>(1,true),Format::DSDU16BE=>(2,true),Format::DSDU16LE=>(2,false),Format::DSDU32BE=>(4,true),Format::DSDU32LE=>(4,false),_=>unreachable!()};
        Self{format,width,big_endian}
    }
    fn encode(self,source:&[u8],frames:usize,channels:usize)->Vec<u8> {
        let words=frames.div_ceil(self.width);let mut output=vec![0x69;words*self.width*channels];
        for word in 0..words {for channel in 0..channels {for byte in 0..self.width {
            let frame=word*self.width+byte;
            if frame<frames {let target=(word*channels+channel)*self.width+if self.big_endian{byte}else{self.width-1-byte};output[target]=source[frame*channels+channel];}
        }}}
        output
    }
}
fn open(request:&PlaybackRequest,config:&OutputConfig,control:&PlaybackControl)->Result<(PCM,Packing,usize,bool)> {
    request.validate_dop(config)?;request.transitions.validate()?;
    if !(20..=2000).contains(&config.buffer_ms){bail!("buffer must be 20..2000 ms");}
    control.initialize_volume(request.volume)?;
    let device=config.device_id.as_deref().context("native DSD needs an explicit ALSA device")?;
    let pcm=PCM::new(device,Direction::Playback,true).with_context(||format!("cannot open ALSA native DSD output {device}"))?;
    let mut selected=None;
    for format in [Format::DSDU32BE,Format::DSDU32LE,Format::DSDU16BE,Format::DSDU16LE,Format::DSDU8] {
        let packing=Packing::new(format);let rate=request.sample_rate/packing.width as u32;
        if rate*packing.width as u32!=request.sample_rate{continue;}
        let params=HwParams::any(&pcm)?;params.set_access(Access::RWInterleaved)?;params.set_channels(request.channels as u32)?;
        if params.set_format(format).is_err()||params.set_rate(rate,ValueOr::Nearest).is_err(){continue;}
        let wanted=(u64::from(rate)*u64::from(config.buffer_ms)/1000).max(64) as i64;
        params.set_period_size_near((wanted/4).clamp(32,8192),ValueOr::Nearest)?;params.set_buffer_size_near(wanted)?;
        pcm.hw_params(&params)?;let actual=pcm.hw_params_current()?;
        if actual.get_rate()?!=rate||actual.get_format()?!=format||actual.get_channels()?!=request.channels as u32{bail!("ALSA changed the requested native DSD format");}
        selected=Some((packing,actual.get_period_size()?.clamp(32,8192) as usize,actual.can_pause()));break;
    }
    let (packing,period,can_pause)=selected.context("ALSA device cannot preserve the requested native DSD bit clock and container")?;
    {let params=pcm.sw_params_current()?;params.set_start_threshold(1)?;params.set_avail_min(period as i64)?;params.set_tstamp_mode(true)?;params.set_tstamp_type(TstampType::Monotonic)?;pcm.sw_params(&params)?;}
    pcm.prepare()?;control.output_sample_rate.store(request.sample_rate,Ordering::Relaxed);
    Ok((pcm,packing,period,can_pause))
}
fn clock(pcm:&PCM,control:&PlaybackControl,physical_words:u64,width:usize,feedback:&clock::Session)->Result<()> {
    let before=clock::host_ns();let status=pcm.status()?;let after=clock::host_ns();
    if status.get_state()==State::Disconnected{return Err(DeviceUnavailable("ALSA native DSD device disconnected".into()).into());}
    let heard=physical_words.saturating_sub(status.get_delay().max(0) as u64).saturating_mul(width as u64).min(control.frames_written.load(Ordering::Relaxed));
    control.frames_played.fetch_max(heard,Ordering::Release);
    let stamp=status.get_htstamp();feedback.record(clock::Source::AlsaNativeWords,physical_words.saturating_sub(status.get_delay().max(0) as u64),control.output_sample_rate.load(Ordering::Relaxed) as u64/width as u64,(stamp.tv_sec.max(0) as u64).saturating_mul(1_000_000_000).saturating_add(stamp.tv_nsec.max(0) as u64),clock::Domain::Monotonic,before,after,matches!(status.get_state(),State::Running|State::Draining));control.clock_ns.store((stamp.tv_sec.max(0) as u64).saturating_mul(1_000_000_000).saturating_add(stamp.tv_nsec.max(0) as u64),Ordering::Relaxed);Ok(())
}
pub(super) fn play_sequence(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,sequence:PlaybackSequence,mut update:impl FnMut(PlaybackUpdate))->Result<()> {
    if control.stopped.load(Ordering::Acquire){return Ok(());}
    let _run=sequence.guard()?;
    let (pcm,packing,period,can_pause)=open(&request,&config,&control)?;
    let mut feedback=clock::Session::new(control.clone());
    let format=SequenceFormat{sample_rate:request.sample_rate,channels:request.channels,channel_mask:super::channel_mask(&pcm,request.channels),exclusive:true,float_output:false,bit_depth:1};
    let mut stream=PcmStream::open(request,sequence.clone(),control.clone(),format)?;
    let device=config.device_id.as_deref().unwrap();let channels=format.channels as usize;let frame_bytes=packing.width*channels;let source_period=period*packing.width;
    let io=pcm.io_bytes();let mut raw=Vec::new();let mut pending=Vec::new();let mut valid=0;let mut cursor=0;let mut eof=false;let mut paused=false;let mut physical_words=0;let mut underrun=false;
    let mut publish=|terminal:bool,underrun:bool|->Result<()> {
        let ((id,position,_,ended),output_frames)=sequence.heard_with_frames(&control,terminal)?;
        let mut stages=vec![format!("DSD {} Hz → ALSA native {}; no PCM conversion",format.sample_rate*8,packing.format),format!("ALSA word clock {} Hz / {} DSD bits",format.sample_rate/packing.width as u32,packing.width*8)];
        if !can_pause{stages.push("ALSA pause drains the configured buffer".into());}if underrun{stages.push("ALSA underrun recovery".into());}
        update(PlaybackUpdate{output_frames,playback_id:id,position,ended,signal_path:SignalPath{output:format!("ALSA {device}"),sample_rate:format.sample_rate*8,channels:format.channels,exclusive:true,bit_depth:1,sample_type:"dsd_native".into(),mixer_sample_rate:None,bit_perfect_eligible:control.volume()==1.&&!underrun,processing:stages},meter:Meter::default(),continuity_lost:underrun});Ok(())
    };
    let result=(||->Result<()> {loop {
        if control.stopped.load(Ordering::Acquire){break;}
        if control.volume_setting()!=1.{bail!("原样 DSD 不支持软件音量；请使用 DAC 音量控制");}
        if control.paused.load(Ordering::Relaxed){
            if !paused&&matches!(pcm.state(),State::Running|State::Draining) {if can_pause&&pcm.state()==State::Running{pcm.pause(true)?;paused=true;}else{match pcm.drain(){Ok(())=>pcm.prepare()?,Err(error) if error.errno()==11=>{},Err(error)=>return Err(error.into())}}}
            clock(&pcm,&control,physical_words,packing.width,&feedback)?;publish(false,underrun)?;std::thread::sleep(Duration::from_millis(10));continue;
        }
        if paused{pcm.pause(false)?;paused=false;}
        clock(&pcm,&control,physical_words,packing.width,&feedback)?;
        if control.frames_written.load(Ordering::Relaxed)>0{publish(false,underrun)?;}
        if eof&&cursor==pending.len()&&raw.is_empty(){
            if physical_words==0{publish(true,underrun)?;break;}
            match pcm.drain(){Ok(())=>{clock(&pcm,&control,physical_words,packing.width,&feedback)?;publish(true,underrun)?;break;},Err(error) if error.errno()==11=>{let _=pcm.wait(Some(20));continue;},Err(error)=>return Err(error.into())}
        }
        if cursor==pending.len(){
            pending.clear();cursor=0;
            while !eof&&raw.len()/channels<source_period {
                match stream.read(source_period-raw.len()/channels) {
                    Ok(Read::Data(chunk))=>raw.extend_from_slice(&chunk.bytes),Ok(Read::End)=>eof=true,Ok(Read::Pending)=>break,
                    Err(_) if control.stopped.load(Ordering::Acquire)=>break,Err(error)=>return Err(error),
                }
            }
            if control.stopped.load(Ordering::Acquire){break;}
            let frames=raw.len()/channels;valid=if eof{frames}else{frames/packing.width*packing.width};
            if valid==0{if !eof{std::thread::sleep(Duration::from_millis(5));}continue;}
            pending=packing.encode(&raw[..valid*channels],valid,channels);raw.drain(..valid*channels);
            if control.paused.load(Ordering::Relaxed){continue;}
        }
        let silence;if control.muted.load(Ordering::Relaxed){silence=vec![0x69;pending.len()-cursor];}else{silence=Vec::new();}
        let bytes=if silence.is_empty(){&pending[cursor..]}else{&silence};
        match io.writei(bytes) {
            Ok(words)=>{
                let before=cursor/channels;cursor+=words*frame_bytes;let after=cursor/channels;
                physical_words+=words as u64;control.frames_written.fetch_add((after.min(valid)-before.min(valid)) as u64,Ordering::Relaxed);
                clock(&pcm,&control,physical_words,packing.width,&feedback)?;publish(false,underrun)?;if words==0{let _=pcm.wait(Some(20));}
            },
            Err(error) if error.errno()==11=>{let _=pcm.wait(Some(20));},
            Err(error) if error.errno()==32=>{underrun=true;clock(&pcm,&control,physical_words,packing.width,&feedback)?;pcm.prepare()?;feedback.restart();},Err(error)=>return Err(error.into()),
        }
    }Ok(())})();
    let _=pcm.drop();result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_alsa_words_preserve_channel_time_order_and_complete_final_words() {
        let raw=[1,11,2,12,3,13,4,14,5,15];
        for (format,expected) in [
            (Format::DSDU8,raw.to_vec()),
            (Format::DSDU16BE,vec![1,2,11,12,3,4,13,14,5,0x69,15,0x69]),
            (Format::DSDU16LE,vec![2,1,12,11,4,3,14,13,0x69,5,0x69,15]),
            (Format::DSDU32BE,vec![1,2,3,4,11,12,13,14,5,0x69,0x69,0x69,15,0x69,0x69,0x69]),
            (Format::DSDU32LE,vec![4,3,2,1,14,13,12,11,0x69,0x69,0x69,5,0x69,0x69,0x69,15]),
        ] {assert_eq!(Packing::new(format).encode(&raw,5,2),expected);}
    }
    #[test]
    fn native_alsa_real_file_output_keeps_preload_seek_pause_mute_and_byte_clock() {
        use crate::dsd::tests::{samples,dsf,dff};
        use std::io::Write;
        let root=tempfile::tempdir().unwrap();let captured=root.path().join("captured.raw");let config_file=root.path().join("alsa.conf");
        std::fs::write(&config_file,format!("pcm.null {{ type null }}\npcm.hw {{ @args [ CARD DEV SUBDEV ] @args.CARD {{ type string default \"QA\" }} @args.DEV {{ type integer default 0 }} @args.SUBDEV {{ type integer default -1 }} type file slave.pcm null file \"{}\" format raw }}\n",captured.display())).unwrap();
        std::env::set_var("ALSA_CONFIG_PATH",&config_file);
        let rate=22579200_u32;let first=samples(4099);let second=samples(4097);
        let mut a=dsf(&first,4099*8-3,true);a[56..60].copy_from_slice(&rate.to_le_bytes());
        let mut b=dff(&second);let at=b.windows(4).position(|bytes|bytes==b"FS  ").unwrap()+12;b[at..at+4].copy_from_slice(&rate.to_be_bytes());
        let mut files=Vec::new();for (extension,bytes) in [(".dsf",a),(".dff",b)]{let mut file=tempfile::Builder::new().suffix(extension).tempfile_in(root.path()).unwrap();file.write_all(&bytes).unwrap();file.flush().unwrap();files.push(file);}
        let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:files[0].path().to_str().unwrap().into(),position:0.6/(rate/8) as f64,sample_rate:rate/8,channels:2,source_integer_lossless:false,source_bit_depth:1,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:None,transitions:Default::default()};
        let config=OutputConfig{device_id:Some("hw:QA".into()),exclusive:true,dsd_output:DsdOutput::Native,buffer_ms:20,..Default::default()};
        assert!(request.validate_dop(&OutputConfig{device_id:Some("default".into()),..config.clone()}).is_err());
        let format=SequenceFormat{sample_rate:rate/8,channels:2,channel_mask:3,exclusive:true,float_output:false,bit_depth:1};
        let queue=PlaybackSequence::new("first".into(),request.clone(),format).unwrap();
        queue.reserve().unwrap().prepare("second".into(),PlaybackRequest{input:files[1].path().to_str().unwrap().into(),position:0.,..request.clone()}).unwrap();
        let control=Arc::new(PlaybackControl::default());control.paused.store(true,Ordering::Relaxed);let mut ended=Vec::new();let mut held=false;
        play_sequence(request.clone(),config.clone(),control.clone(),queue,|update|{
            assert_eq!((update.signal_path.sample_rate,update.signal_path.bit_depth,update.signal_path.sample_type.as_str()),(rate,1,"dsd_native"));
            assert!(update.signal_path.processing.iter().any(|stage|stage.contains("DSD_U32_BE")));
            if control.paused.load(Ordering::Relaxed){assert_eq!(control.frames_written.load(Ordering::Relaxed),0);assert_eq!(std::fs::metadata(&captured).unwrap().len(),0);assert_eq!(update.position,1./(rate/8) as f64);held=true;control.paused.store(false,Ordering::Relaxed);}
            ended.extend(update.ended);
        }).unwrap();assert!(held);
        let actual=std::fs::read(&captured).unwrap();assert_eq!(actual.len(),8196*2);
        let mut decoded=vec![Vec::new(),Vec::new()];for word in actual.chunks_exact(8){for channel in 0..2{decoded[channel].extend_from_slice(&word[channel*4..channel*4+4]);}}
        for channel in 0..2{let mut expected=first[channel][1..].to_vec();let last=expected.last_mut().unwrap();*last=(*last&0xf8)|(0x69&7);expected.extend(&second[channel]);expected.push(0x69);assert_eq!(decoded[channel],expected);}
        assert_eq!(control.frames_written.load(Ordering::Relaxed),8195);assert_eq!(control.frames_played.load(Ordering::Acquire),8195);
        assert_eq!(ended.iter().map(|row|row.frames).collect::<Vec<_>>(),[4098,4097]);assert_eq!(ended[0].position,(4099.*8.-3.)/rate as f64);assert_eq!(ended[1].position,4097./(rate/8) as f64);
        let muted=Arc::new(PlaybackControl::default());muted.muted.store(true,Ordering::Relaxed);
        let queue=PlaybackSequence::new("muted".into(),request.clone(),format).unwrap();play_sequence(request,config,muted.clone(),queue,|_|{}).unwrap();
        let actual=std::fs::read(&captured).unwrap();assert!(!actual.is_empty()&&actual.iter().all(|byte|*byte==0x69));assert_eq!(muted.frames_written.load(Ordering::Relaxed),4098);
    }
}
