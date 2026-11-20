use super::*;
use anyhow::Context;
use ::alsa::{device_name::HintIter,pcm::{Access,Format,HwParams,State,TstampType,PCM},Direction,ValueOr};
use std::{ffi::CString,time::Duration};
mod native;

pub fn devices()->Result<Vec<DeviceInfo>> {
    let mut result=Vec::new();
    for hint in HintIter::new(None,&CString::new("pcm")?)? {
        if hint.direction==Some(Direction::Capture){continue;}
        if let Some(id)=hint.name {
            if !result.iter().any(|device:&DeviceInfo|device.id==id){
                result.push(DeviceInfo{default:id=="default",name:hint.desc.unwrap_or_else(||id.clone()).replace('\n'," · "),id,mix_rate:0,channels:0});
            }
        }
    }
    result.sort_by(|a,b|b.default.cmp(&a.default).then(a.name.cmp(&b.name)));Ok(result)
}

fn precision(format:Format)->u16 {match format {Format::S16LE=>16,Format::S24LE|Format::S243LE=>24,_=>32}}
fn narrow(sample:i32,bits:u16)->i32 {
    let limit=1i64<<(bits-1);
    (sample as f64/(1u64<<(32-bits)) as f64).round().clamp(-limit as f64,(limit-1) as f64) as i32
}
fn encode(samples:&[i32],format:Format)->Vec<u8> {
    let mut bytes=Vec::with_capacity(samples.len()*if format==Format::S243LE {3}else if format==Format::S16LE {2}else{4});
    for &sample in samples {
        match format {
            Format::S16LE=>bytes.extend_from_slice(&(narrow(sample,16) as i16).to_le_bytes()),
            Format::S24LE=>bytes.extend_from_slice(&narrow(sample,24).to_le_bytes()),
            Format::S243LE=>bytes.extend_from_slice(&narrow(sample,24).to_le_bytes()[..3]),
            _=>bytes.extend_from_slice(&sample.to_le_bytes()),
        }
    }
    bytes
}
fn clock(pcm:&PCM,control:&PlaybackControl,feedback:&clock::Session,programme_pcm:bool)->Result<()> {
    let before=clock::host_ns();let status=pcm.status()?;let after=clock::host_ns();
    if status.get_state()==State::Disconnected {return Err(DeviceUnavailable("ALSA output device disconnected".into()).into());}
    let written=control.frames_written.load(Ordering::Relaxed);
    let played=written.saturating_sub(status.get_delay().max(0) as u64);
    control.frames_played.fetch_max(played,Ordering::Release);
    let stamp=status.get_htstamp();
    feedback.record_programme(clock::Source::AlsaFrames,played,control.output_sample_rate.load(Ordering::Relaxed) as u64,(stamp.tv_sec.max(0) as u64).saturating_mul(1_000_000_000).saturating_add(stamp.tv_nsec.max(0) as u64),clock::Domain::Monotonic,before,after,matches!(status.get_state(),State::Running|State::Draining),(programme_pcm&&played<=control.programme_end.load(Ordering::Acquire)).then_some(clock::Programme{frames:played,sample_rate:control.output_sample_rate.load(Ordering::Relaxed)}));
    control.clock_ns.store((stamp.tv_sec.max(0) as u64).saturating_mul(1_000_000_000).saturating_add(stamp.tv_nsec.max(0) as u64),Ordering::Relaxed);
    Ok(())
}
struct DecoderLease<'a>(&'a PlaybackControl);
impl Drop for DecoderLease<'_>{fn drop(&mut self){if let Ok(mut decoder)=self.0.decoder.lock(){decoder.take();}}}

fn open_pcm(request:&PlaybackRequest,config:&OutputConfig,control:&PlaybackControl)->Result<(PCM,u32,Format,usize,bool,u16)> {
    request.validate_dop(config)?;
    if !(20..=2000).contains(&config.buffer_ms)||request.sample_rate<8000||request.channels==0||request.channels>32||!request.position.is_finite()||request.position<0. {bail!("invalid ALSA output format or buffer");}
    request.transitions.validate()?;control.initialize_volume(request.volume)?;
    let device=config.device_id.as_deref().unwrap_or("default");
    if config.exclusive&&!device.starts_with("hw:"){bail!("ALSA exclusive output requires an explicit hw: device");}
    let pcm=PCM::new(device,Direction::Playback,true).with_context(||format!("cannot open ALSA output {device}"))?;
    let target=config.desired_format(request)?;
    let (rate,format,period,can_pause)={
        let params=HwParams::any(&pcm)?;params.set_access(Access::RWInterleaved)?;params.set_channels(target.channels as u32)?;
        let rate=if config.exclusive||config.limits.max_sample_rate.is_some(){let mut chosen=None;for rate in if request.is_dsd(){vec![request.sample_rate]}else{config.limits.rates(request.sample_rate)}{if params.set_rate(rate,ValueOr::Nearest).is_ok(){chosen=Some(rate);break;}}chosen.context("ALSA 没有符合上限的采样率")?}else{params.set_rate_near(target.sample_rate,ValueOr::Nearest)?};
        let minimum=if request.is_dsd(){24}else if config.exclusive {if request.source_bit_depth==0 {32}else{request.source_bit_depth.min(32)}.min(config.limits.max_bit_depth.unwrap_or(32))}else{16};
        let format=[Format::S32LE,Format::S24LE,Format::S243LE,Format::S16LE].into_iter().find(|format|precision(*format)>=minimum&&params.test_format(*format).is_ok()).context("ALSA device cannot preserve the required PCM precision")?;
        params.set_format(format)?;
        let wanted=(rate as u64*config.buffer_ms as u64/1000).max(64) as i64;
        params.set_period_size_near((wanted/4).clamp(32,8192),ValueOr::Nearest)?;params.set_buffer_size_near(wanted)?;
        pcm.hw_params(&params)?;let actual=pcm.hw_params_current()?;
        if config.exclusive&&(actual.get_rate()?!=rate||actual.get_channels()?!=target.channels as u32){bail!("ALSA exclusive device changed sample rate or channels");}
        (actual.get_rate()?,actual.get_format()?,actual.get_period_size()?.clamp(32,8192) as usize,actual.can_pause())
    };
    {
        let params=pcm.sw_params_current()?;params.set_start_threshold(if control.start.is_scheduled(){params.get_boundary()?}else{1})?;params.set_avail_min(period as i64)?;params.set_tstamp_mode(true)?;params.set_tstamp_type(TstampType::Monotonic)?;pcm.sw_params(&params)?;
    }
    pcm.prepare()?;control.output_sample_rate.store(rate,Ordering::Relaxed);
    config.resync(control)?;
    Ok((pcm,rate,format,period,can_pause,target.channels))
}
pub fn play(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,mut update:impl FnMut(SignalPath,Meter))->Result<()> {
    request.validate_dop(&config)?;
    if request.is_dsd(){return play_single_impulse(request,config,control,update);}
    require_loaded_impulse(&request)?;
    if request.dsp.enabled&&request.impulse.is_some(){return play_single_impulse(request,config,control,update);}
    let (pcm,rate,format,period,can_pause,_channels)=open_pcm(&request,&config,&control)?;
    let mut feedback=clock::Session::new(control.clone());
    let device=config.device_id.as_deref().unwrap_or("default");
    let processing=request.dsp.enabled;
    let mut decoder=FfmpegDecoder::open(&request.input,request.position,rate,request.channels,if processing {PcmEncoding::Float64}else{PcmEncoding::Signed32})?;
    *control.decoder.lock().map_err(|_|anyhow::anyhow!("decoder lock poisoned"))?=Some(decoder.abort_handle());let _lease=DecoderLease(&control);
    if control.stopped.load(Ordering::Acquire){control.stop();return Ok(());}
    let mut dsp=DspProcessor::new(request.dsp.clone(),rate,request.channels as usize,request.measured_lufs)?;
    let mut path=SignalPath{output:format!("ALSA {device}"),sample_rate:rate,channels:request.channels,exclusive:config.exclusive,bit_depth:precision(format),sample_type:"integer".into(),mixer_sample_rate:None,bit_perfect_eligible:false,processing:Vec::new()};
    if !config.exclusive{path.processing.push("ALSA plugin/shared route; downstream format unverified".into());}
    if rate!=request.sample_rate{path.processing.push(format!("FFmpeg rate conversion {} → {} Hz",request.sample_rate,rate));}
    if request.source_bit_depth>path.bit_depth{path.processing.push(format!("PCM precision conversion {} → {} bit",request.source_bit_depth,path.bit_depth));}
    if processing{path.processing.push("DSP f64 → integer PCM".into());}
    if request.transitions.enabled(){path.processing.push("Fade envelope".into());}
    if request.dsp.normalize&&request.measured_lufs.is_none(){path.processing.push("Normalization pending measured LUFS".into());}
    if !request.source_integer_lossless{path.processing.push("Decoded source to signed 32-bit PCM".into());}
    if !can_pause{path.processing.push("ALSA pause drains the configured buffer".into());}
    let input_bytes=request.channels as usize*if processing{8}else{4};let mut input=vec![0;period*input_bytes];
    let frame_bytes=request.channels as usize*if format==Format::S243LE{3}else if format==Format::S16LE{2}else{4};
    let io=pcm.io_bytes();let mut pending=Vec::new();let mut cursor=0;let mut eof=false;let mut hardware_paused=false;let mut meter=Meter::default();
    let result=(||->Result<()>{loop {
        if control.stopped.load(Ordering::Acquire){break;}
        if control.paused.load(Ordering::Relaxed){
            if !hardware_paused&&pcm.state()==State::Running{
                if can_pause{pcm.pause(true)?;hardware_paused=true;}else{
                    match pcm.drain(){Ok(())=>{clock(&pcm,&control,&feedback,!request.is_dsd())?;pcm.prepare()?;},Err(error) if error.errno()==11=>{},Err(error)=>return Err(error.into())}
                }
            }else if pcm.state()==State::Draining{
                match pcm.drain(){Ok(())=>{clock(&pcm,&control,&feedback,!request.is_dsd())?;pcm.prepare()?;},Err(error) if error.errno()==11=>{},Err(error)=>return Err(error.into())}
            }
            clock(&pcm,&control,&feedback,!request.is_dsd())?;update(path.clone(),meter.clone());std::thread::sleep(Duration::from_millis(20));continue;
        }
        if hardware_paused{pcm.pause(false)?;hardware_paused=false;}
        clock(&pcm,&control,&feedback,!request.is_dsd())?;
        if eof&&cursor==pending.len(){
            match pcm.drain(){Ok(())=>{control.frames_played.store(control.frames_written.load(Ordering::Relaxed),Ordering::Release);clock(&pcm,&control,&feedback,!request.is_dsd())?;break;},Err(error) if error.errno()==11=>{let _=pcm.wait(Some(20));continue;},Err(error)=>return Err(error.into())}
        }
        if cursor==pending.len(){
            let decoded=decoder.read_frames(&mut input,input_bytes);if control.stopped.load(Ordering::Acquire){break;}let frames=decoded?;
            if frames==0{eof=true;continue;}
            let volume=control.volume();let start=control.frames_written.load(Ordering::Relaxed);
            path.bit_perfect_eligible=config.exclusive&&!processing&&!request.transitions.enabled()&&volume==1.&&request.source_integer_lossless&&rate==request.sample_rate&&request.source_bit_depth>0&&path.bit_depth>=request.source_bit_depth;
            path.processing.retain(|stage|stage!="Software volume");if volume!=1.{path.processing.push("Software volume".into());}
            let gain=|i:usize|volume*request.transitions.gain(request.position+(start+i as u64/request.channels as u64) as f64/rate as f64,request.duration);
            let samples=if processing{
                let mut values:Vec<f64>=input[..frames*input_bytes].chunks_exact(8).map(|b|f64::from_le_bytes(b.try_into().unwrap())).collect();meter=dsp.process(&mut values)?;
                values.iter().enumerate().map(|(i,x)|(*x*gain(i)*2147483648.).round().clamp(i32::MIN as f64,i32::MAX as f64) as i32).collect::<Vec<_>>()
            }else{
                let values:Vec<i32>=input[..frames*input_bytes].chunks_exact(4).map(|b|i32::from_le_bytes(b.try_into().unwrap())).collect();meter.peak=values.iter().map(|x|(*x as f64/2147483648.).abs()).fold(0.,f64::max);meter.clipped_samples=0;
                if volume==1.&&!request.transitions.enabled(){values}else{values.iter().enumerate().map(|(i,x)|(*x as f64*gain(i)).round().clamp(i32::MIN as f64,i32::MAX as f64) as i32).collect()}
            };
            pending=encode(&samples,format);cursor=0;
            if control.paused.load(Ordering::Relaxed){continue;}
        }
        match io.writei(&pending[cursor..]){
            Ok(frames)=>{cursor+=frames*frame_bytes;control.frames_written.fetch_add(frames as u64,Ordering::Relaxed);clock(&pcm,&control,&feedback,!request.is_dsd())?;update(path.clone(),meter.clone());if frames==0{let _=pcm.wait(Some(20));}},
            Err(error) if error.errno()==11=>{let _=pcm.wait(Some(20));},
            Err(error) if error.errno()==32=>{control.frames_played.store(control.frames_written.load(Ordering::Relaxed),Ordering::Release);pcm.prepare()?;feedback.restart();if !path.processing.iter().any(|stage|stage=="ALSA underrun recovery"){path.processing.push("ALSA underrun recovery".into());}},
            Err(error)=>return Err(error.into()),
        }
    }Ok(())})();
    let _=pcm.drop();result
}

fn sequence_path(request:&PlaybackRequest,format:SequenceFormat,device:&str,can_pause:bool,volume:f64,underrun:bool)->SignalPath {
    if request.is_dsd() {
        return SignalPath{output:format!("ALSA {device}"),sample_rate:format.sample_rate,channels:format.channels,exclusive:format.exclusive,
            bit_depth:format.bit_depth,sample_type:"dsd_dop".into(),mixer_sample_rate:None,
            bit_perfect_eligible:format.exclusive&&volume==1.&&!underrun,
            processing:vec![format!("DSD {} Hz → DoP {} Hz; no PCM conversion",request.sample_rate as u64*16,format.sample_rate)]};
    }
    let mut stages=Vec::new();
    if !format.exclusive{stages.push("ALSA plugin/shared route; downstream format unverified".into());}
    if request.sample_rate!=format.sample_rate{stages.push(format!("FFmpeg rate conversion {} → {} Hz",request.sample_rate,format.sample_rate));}
    if request.channels!=format.channels{stages.push(format!("FFmpeg channel conversion {} → {}",request.channels,format.channels));}
    if request.source_bit_depth>format.bit_depth{stages.push(format!("PCM precision conversion {} → {} bit",request.source_bit_depth,format.bit_depth));}
    if request.dsp.enabled{stages.push("DSP f64 → integer PCM".into());}
    if request.transitions.enabled(){stages.push("Fade envelope".into());}
    if request.dsp.enabled {if let Some(impulse)=request.impulse.as_ref().and_then(|bank|bank.response(format.sample_rate).ok()) {stages.push(format!("FIR f64: {} taps @ {} Hz; original gain",impulse.frames(),impulse.sample_rate()));}}
    if request.dsp.normalize&&request.measured_lufs.is_none(){stages.push("Normalization pending measured LUFS".into());}
    if !request.source_integer_lossless{stages.push("Decoded source to signed 32-bit PCM".into());}
    if !can_pause{stages.push("ALSA pause drains the configured buffer".into());}
    if volume!=1.{stages.push("Software volume".into());}
    if underrun{stages.push("ALSA underrun recovery".into());}
    SignalPath{output:format!("ALSA {device}"),sample_rate:format.sample_rate,channels:format.channels,exclusive:format.exclusive,bit_depth:format.bit_depth,sample_type:"integer".into(),mixer_sample_rate:None,
        bit_perfect_eligible:format.exclusive&&!request.dsp.enabled&&!request.transitions.enabled()&&volume==1.&&request.source_integer_lossless&&request.sample_rate==format.sample_rate&&request.channels==format.channels&&request.source_bit_depth>0&&format.bit_depth>=request.source_bit_depth,processing:stages}
}
fn channel_mask(pcm:&PCM,channels:u16)->u32 {
    let Ok(map)=pcm.get_chmap() else{return 0;};let mut positions=String::new();
    if std::fmt::write(&mut positions,format_args!("{map}")).is_err(){return 0;}
    layout_mask(&positions,channels)
}
fn layout_mask(positions:&str,channels:u16)->u32 {
    let(mut mask,mut last,mut count)=(0,0,0);
    for position in positions.split_whitespace() {
        let bit=match position {"FL"=>1,"FR"=>2,"FC"=>4,"LFE"=>8,"RL"=>16,"RR"=>32,"FLC"=>64,"FRC"=>128,"RC"=>256,"SL"=>512,"SR"=>1024,"TC"=>2048,"TFL"=>4096,"TFC"=>8192,"TFR"=>16384,"TRL"=>32768,"TRC"=>65536,"TRR"=>131072,_=>return 0};
        if bit<=last{return 0;}mask|=bit;last=bit;count+=1;
    }
    if count==channels{mask}else{0}
}
/// Continuous PCM packets share this one ALSA handle; no drain/reopen between prepared tracks.
pub fn play_sequence(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,sequence:PlaybackSequence,mut update:impl FnMut(PlaybackUpdate))->Result<()> {
    let scheduled=control.start.setup(request.is_dsd())?;let mut started=!scheduled;
    if request.is_dsd()&&config.dsd_output==DsdOutput::Native{return native::play_sequence(request,config,control,sequence,update);}
    use super::sequence::{PcmStream,Read};
    let _run=sequence.guard()?;
    let (pcm,rate,format,period,can_pause,channels)=open_pcm(&request,&config,&control)?;
    let mut feedback=clock::Session::new(control.clone());
    let device=config.device_id.as_deref().unwrap_or("default");
    let output_format=SequenceFormat{sample_rate:rate,channels,channel_mask:channel_mask(&pcm,channels),exclusive:config.exclusive,float_output:false,bit_depth:precision(format)};
    let dop=request.is_dsd();
    let stream=PcmStream::open(request,sequence.clone(),control.clone(),output_format)?;
    let mut stream=super::buffered::ReadAhead::new(stream,control.clone(),output_format)?;
    let mut dop_encoder=if dop{Some(crate::dsd::DopEncoder::new(output_format.channels)?)}else{None};
    let frame_bytes=output_format.channels as usize*if format==Format::S243LE{3}else if format==Format::S16LE{2}else{4};
    let io=pcm.io_bytes();let mut pending=Vec::new();let mut cursor=0;let mut eof=false;let mut hardware_paused=false;let mut meter=Meter::default();let mut continuity_lost=false;
    let mut publish=|final_end:bool,meter:Meter,continuity_lost:bool|->Result<()> {
        let((id,position,source,ended),output_frames)=sequence.heard_with_frames(&control,final_end)?;
        let mut signal_path=sequence_path(&source,output_format,device,can_pause,control.volume(),continuity_lost);
        config.limits.signal_path(&source,&mut signal_path);
        sequence.mark_rate_adjustment(&mut signal_path);
        update(PlaybackUpdate{output_frames,playback_id:id,position,ended,signal_path,meter,continuity_lost});Ok(())
    };
    let result=(||->Result<()>{loop {
        if control.stopped.load(Ordering::Acquire){break;}
        if control.paused.load(Ordering::Relaxed){
            if !hardware_paused&&pcm.state()==State::Running {
                if can_pause{pcm.pause(true)?;hardware_paused=true;}
                else{match pcm.drain(){Ok(())=>pcm.prepare()?,Err(error) if error.errno()==11=>{},Err(error)=>return Err(error.into())}}
            }
            clock(&pcm,&control,&feedback,!dop)?;publish(false,meter.clone(),continuity_lost)?;std::thread::sleep(Duration::from_millis(10));continue;
        }
        if hardware_paused{pcm.pause(false)?;hardware_paused=false;}
        clock(&pcm,&control,&feedback,!dop)?;
        if control.frames_written.load(Ordering::Relaxed)>0{publish(false,meter.clone(),continuity_lost)?;}
        if eof&&cursor==pending.len(){
            match pcm.drain(){Ok(())=>{control.frames_played.store(control.frames_written.load(Ordering::Relaxed),Ordering::Release);clock(&pcm,&control,&feedback,!dop)?;publish(true,meter.clone(),continuity_lost)?;break;},Err(error) if error.errno()==11=>{let _=pcm.wait(Some(20));continue;},Err(error)=>return Err(error.into())}
        }
        if cursor==pending.len(){
            pending.clear();cursor=0;let mut frames=0;let mut waiting=false;
            while !eof&&frames<period {
                match stream.read(period-frames){
                    Ok(Read::Data(chunk))=>{pending.extend_from_slice(&chunk.bytes);frames+=chunk.frames;meter=chunk.meter;},
                    Ok(Read::Pending)=>{waiting=true;break;},Ok(Read::End)=>eof=true,
                    Err(error) if control.stopped.load(Ordering::Acquire)=>{let _=error;break;},Err(error)=>return Err(error),
                }
            }
            if control.stopped.load(Ordering::Acquire){break;}
            if let Some(encoder)=dop_encoder.as_mut(){encoder.stamp(&mut pending,crate::dsd::DopContainer::LeftAligned32,frames,control.muted.load(Ordering::Relaxed))?;}
            let samples=pending.chunks_exact(4).map(|b|i32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();pending=encode(&samples,format);
            if control.paused.load(Ordering::Relaxed){continue;}
            if waiting&&pending.is_empty(){std::thread::sleep(Duration::from_millis(5));continue;}
            if pending.is_empty(){continue;}
        }
        if dop&&control.volume_setting()!=1.{bail!("原样 DSD 不支持软件音量；请使用 DAC 音量控制");}
        match io.writei(&pending[cursor..]){
            Ok(frames)=>{cursor+=frames*frame_bytes;control.frames_written.fetch_add(frames as u64,Ordering::Relaxed);if !started&&frames>0{if !control.start.wait(&control)?{break;}pcm.start()?;{let params=pcm.sw_params_current()?;params.set_start_threshold(1)?;pcm.sw_params(&params)?;}started=true;}clock(&pcm,&control,&feedback,!dop)?;publish(false,meter.clone(),continuity_lost)?;if frames==0{let _=pcm.wait(Some(20));}},
            Err(error) if error.errno()==11=>{let _=pcm.wait(Some(20));},
            Err(error) if error.errno()==32=>{continuity_lost=true;control.frames_played.store(control.frames_written.load(Ordering::Relaxed),Ordering::Release);pcm.prepare()?;feedback.restart();},
            Err(error)=>return Err(error.into()),
        }
    }Ok(())})();
    let _=pcm.drop();result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_layout_requires_canonical_order_and_known_positions(){
        assert_eq!(layout_mask("FL FR FC LFE RL RR",6),63);
        for layout in ["FL FR RL RR FC LFE","FR FL","FL FL","FL UNKNOWN","FL FR NA"] {assert_eq!(layout_mask(layout,2),0);}
        assert_eq!(layout_mask("FL FR",6),0);
        assert_eq!(layout_mask("FL FR SL SR",4),1539);
    }
    #[test]
    fn integer_packings_preserve_signed_pcm_alignment(){
        let samples=[i32::MIN,-256,0,256,i32::MAX-255];
        assert_eq!(encode(&samples,Format::S243LE),[0,0,128,255,255,255,0,0,0,1,0,0,255,255,127]);
        assert_eq!(encode(&samples,Format::S16LE),[0,128,0,0,0,0,0,0,255,127]);
        assert_eq!(encode(&[-256,256],Format::S24LE),[255,255,255,255,1,0,0,0]);
    }
    #[test]
    fn narrowing_rounds_symmetrically_and_saturates_without_wrapping(){
        let samples=[-32768,32768,-32767,32767,i32::MIN,i32::MAX];
        let actual:Vec<i16>=encode(&samples,Format::S16LE).chunks_exact(2).map(|bytes|i16::from_le_bytes(bytes.try_into().unwrap())).collect();
        assert_eq!(actual,[-1,1,0,0,i16::MIN,i16::MAX]);
        let actual:Vec<i32>=encode(&[-128,128,-127,127,i32::MIN,i32::MAX],Format::S24LE).chunks_exact(4).map(|bytes|i32::from_le_bytes(bytes.try_into().unwrap())).collect();
        assert_eq!(actual,[-1,1,0,0,-8388608,8388607]);
    }
}
