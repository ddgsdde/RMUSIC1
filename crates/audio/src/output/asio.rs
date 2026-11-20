//! ASIO PCM/DoP/native DSD output. Decode on the worker, never on the driver callback.
use super::sequence::{PcmStream, Read};
use super::*;
use anyhow::Context;
use asio_sys::{Asio, AsioDriverEvent, AsioMessageSelectors, Driver};
use std::{collections::VecDeque, sync::{mpsc, OnceLock}, time::{Duration, Instant}};

unsafe extern "C" {
    fn rmusic_asio_io_format(native_dsd:i32)->i32;
    fn rmusic_asio_set_rate(rate: f64) -> i32;
    fn rmusic_asio_position(frames: *mut u64, nanos: *mut u64) -> i32;
    fn rmusic_asio_channel_type(channel: i32, sample_type: *mut i32) -> i32;
}
static HOST: OnceLock<Mutex<Asio>> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
fn host() -> &'static Mutex<Asio> {HOST.get_or_init(||Mutex::new(Asio::new()))}
pub fn devices() -> Vec<DeviceInfo> {
    host().lock().map(|host|host.driver_names().into_iter().map(|name|DeviceInfo {
        id:format!("asio:{name}"),name:format!("ASIO · {name}"),default:false,mix_rate:0,channels:0,
    }).collect()).unwrap_or_default()
}
struct Active;
impl Drop for Active {fn drop(&mut self){ACTIVE.store(false,Ordering::Release);}}
struct ComApartment;
impl Drop for ComApartment {fn drop(&mut self){wasapi::deinitialize();}}
struct IoMode {driver:Driver,native:bool}
impl Drop for IoMode {
    fn drop(&mut self) {if self.native {let _=self.driver.stop();let _=self.driver.dispose_buffers();let _=unsafe{rmusic_asio_io_format(0)};}}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SampleFormat {bytes:usize,bits:u16,float:bool,big_endian:bool}
impl SampleFormat {
    fn from_type(kind:i32)->Result<Self> {
        let big_endian=kind<16||kind==33;
        let (bytes,bits,float)=match kind {
            0|16=>(2,16,false),1|17=>(3,24,false),2|18=>(4,32,false),
            3|19=>(4,32,true),4|20=>(8,64,true),8|24=>(4,16,false),11|27=>(4,24,false),
            9|25=>(4,18,false),10|26=>(4,20,false),
            32|33=>(1,1,false),
            40=>bail!("ASIO DSD NER8 carries 8-bit DSD samples and cannot preserve a one-bit DSF/DFF stream"),
            _=>bail!("ASIO sample type {kind} is not supported"),
        };
        Ok(Self{bytes,bits,float,big_endian})
    }
    fn validate_precision(self,request:&PlaybackRequest)->Result<()> {
        if self.bits==1 {if !request.is_dsd(){bail!("ASIO native DSD buffers cannot carry PCM");}return Ok(());}
        if request.is_dsd()&&(self.float||self.bits<24){bail!("ASIO DoP 需要至少 24 位的整数驱动容器");}
        let required=if request.source_bit_depth==0{32}else{request.source_bit_depth};
        if !self.float&&!request.dsp.enabled&&self.bits<required {bail!("ASIO device offers {} bits; source requires {required} bits",self.bits);}
        if self.float&&self.bits==32&&request.source_integer_lossless&&!request.dsp.enabled&&required>24 {bail!("ASIO float32 cannot preserve the source's {required}-bit integer precision");}
        Ok(())
    }
    /// Interleaved internal s32/f32 -> planar driver packets; padding is always zero.
    fn pack(self,input:&[u8],frames:usize,period:usize,channels:usize)->Vec<u8> {
        if self.bits==1 {
            let mut packet=vec![if self.big_endian{0x69}else{0x96};period*channels];
            for frame in 0..frames {for channel in 0..channels {let byte=input[frame*channels+channel];packet[channel*period+frame]=if self.big_endian{byte}else{byte.reverse_bits()};}}
            return packet;
        }
        let mut packet=vec![0;period*channels*self.bytes];
        let input_width=if self.float&&self.bytes==8{8}else{4};
        for frame in 0..frames {for channel in 0..channels {
            let offset=(frame*channels+channel)*input_width;
            let mut sample=[0_u8;8];
            if self.float {
                sample[..input_width].copy_from_slice(&input[offset..offset+input_width]);
            } else {
                let value=i32::from_le_bytes(input[offset..offset+4].try_into().unwrap());
                let value=if self.bits==32{value}else{
                    let limit=1_i64<<(self.bits-1);
                    (value as f64/(1_u64<<(32-self.bits)) as f64).round().clamp(-limit as f64,(limit-1) as f64) as i32
                };
                sample[..4].copy_from_slice(&value.to_le_bytes());
            }
            if self.big_endian {sample[..self.bytes].reverse();}
            let target=(channel*period+frame)*self.bytes;
            packet[target..target+self.bytes].copy_from_slice(&sample[..self.bytes]);
        }}
        packet
    }
    fn stamp_dop(self,buffers:&[[usize;2]],index:usize,period:usize,programme:usize,phase:u64,muted:bool) {
        for frame in 0..period {
            let marker=if (phase+frame as u64)%2==0 {5_u32}else{250_u32};
            for pair in buffers {
                let sample=unsafe{std::slice::from_raw_parts_mut((pair[index] as *mut u8).add(frame*self.bytes),self.bytes)};
                let mut raw=[0;4];raw[..self.bytes].copy_from_slice(sample);if self.big_endian{raw[..self.bytes].reverse();}
                let mut word=u32::from_le_bytes(raw);
                let shift=if self.bits==32{8}else{0};
                let payload=if muted||frame>=programme{0x6969}else{(word>>shift)&0xffff};
                word=(payload|marker<<16)<<shift;
                if self.bits==24&&self.bytes==4&&marker==250 {word|=0xff000000;}
                let mut raw=word.to_le_bytes();if self.big_endian{raw[..self.bytes].reverse();}sample.copy_from_slice(&raw[..self.bytes]);
            }
        }
    }
}
struct Packet {bytes:Vec<u8>,frames:usize}
struct Span {device_start:u64,source_start:u64,frames:u64}
struct Timeline {spans:VecDeque<Span>,heard:u64,stride:u64}
impl Timeline {
    fn advance(&mut self,device_frame:u64)->u64 {
        while let Some(span)=self.spans.front() {
            self.heard=self.heard.max(span.source_start+(device_frame.saturating_sub(span.device_start)/self.stride).min(span.frames));
            if device_frame<span.device_start+span.frames*self.stride {break;}
            self.spans.pop_front();
        }
        self.heard
    }
    fn submitted(&mut self,device_start:u64,source_start:u64,frames:u64) {
        self.spans.push_back(Span{device_start,source_start,frames});
    }
}
struct State {failed:AtomicBool,ended:AtomicBool,drained:AtomicBool,continuity_lost:AtomicBool,device_frame:AtomicU64,callbacks:AtomicU64}
impl Default for State {
    fn default()->Self {Self{failed:AtomicBool::new(false),ended:AtomicBool::new(false),drained:AtomicBool::new(false),continuity_lost:AtomicBool::new(false),device_frame:AtomicU64::new(0),callbacks:AtomicU64::new(0)}}
}
struct Callback {
    buffers:Vec<[usize;2]>,plane_bytes:usize,latency:u64,receiver:mpsc::Receiver<Packet>,
    state:Arc<State>,control:Arc<PlaybackControl>,timeline:Timeline,last_index:Option<i32>,last_position:u64,
    dop:Option<SampleFormat>,dop_frames:u64,
    native_silence:Option<u8>,
    clock_feedback:Option<(u64,u64)>,
}
impl Callback {
    fn render(&mut self,index:i32,clock:impl FnOnce()->Result<(u64,u64)>) {
        if !(0..=1).contains(&index)||self.last_index==Some(index){return;}
        self.last_index=Some(index);
        let index=index as usize;
        let phase=self.dop_frames;
        if let Some(format)=self.dop {
            let period=self.plane_bytes/format.bytes;
            format.stamp_dop(&self.buffers,index,period,0,phase,true);self.dop_frames=self.dop_frames.wrapping_add(period as u64);
        }else{for pair in &self.buffers {unsafe{std::ptr::write_bytes(pair[index] as *mut u8,self.native_silence.unwrap_or(0),self.plane_bytes);}}}
        let before=super::clock::host_ns();
        let Ok((position,stamp))=clock() else {if let Some((epoch,_))=self.clock_feedback{self.control.device_clock.invalidate(epoch);}self.state.failed.store(true,Ordering::Release);return;};
        if position<self.last_position {if let Some((epoch,_))=self.clock_feedback{self.control.device_clock.invalidate(epoch);}self.state.failed.store(true,Ordering::Release);return;}
        let after=super::clock::host_ns();let heard=self.timeline.advance(position);
        let programme=(self.native_silence.is_none()&&self.dop.is_none()&&!self.state.continuity_lost.load(Ordering::Relaxed)&&!self.control.paused.load(Ordering::Relaxed)&&!self.control.stopped.load(Ordering::Acquire)&&heard<=self.control.programme_end.load(Ordering::Acquire)).then_some(super::clock::Programme{frames:heard,sample_rate:self.control.output_sample_rate.load(Ordering::Relaxed)});
        if let Some((epoch,frequency))=self.clock_feedback{super::clock::Session::record_programme_epoch(&self.control,epoch,if self.native_silence.is_some(){super::clock::Source::AsioDsdBits}else{super::clock::Source::AsioFrames},position,frequency,stamp,super::clock::Domain::DriverOpaque,before,after,true,programme);}
        self.last_position=position;
        self.state.device_frame.store(position,Ordering::Relaxed);
        self.state.callbacks.fetch_add(1,Ordering::Relaxed);
        self.control.clock_ns.store(stamp,Ordering::Relaxed);
        self.control.frames_played.store(heard,Ordering::Release);
        if self.control.stopped.load(Ordering::Acquire)||self.state.failed.load(Ordering::Acquire)||self.control.paused.load(Ordering::Relaxed){return;}
        if (self.dop.is_some()||self.native_silence.is_some())&&self.control.volume_setting()!=1.{return;}
        match self.receiver.try_recv() {
            Ok(packet)=>{
                for (channel,pair) in self.buffers.iter().enumerate(){unsafe {
                    std::ptr::copy_nonoverlapping(packet.bytes.as_ptr().add(channel*self.plane_bytes),pair[index] as *mut u8,self.plane_bytes);
                }}
                if let Some(format)=self.dop {format.stamp_dop(&self.buffers,index,self.plane_bytes/format.bytes,packet.frames,phase,self.control.muted.load(Ordering::Relaxed));}
                if let Some(silence)=self.native_silence {if self.control.muted.load(Ordering::Relaxed){for pair in &self.buffers {unsafe{std::ptr::write_bytes(pair[index] as *mut u8,silence,self.plane_bytes);}}}}
                let source=self.control.frames_written.fetch_add(packet.frames as u64,Ordering::Relaxed);
                self.timeline.submitted(position+self.latency,source,packet.frames as u64);
            }
            Err(mpsc::TryRecvError::Empty) if !self.state.ended.load(Ordering::Acquire)=>{self.state.continuity_lost.store(true,Ordering::Relaxed);}
            Err(_)=>{if self.timeline.spans.is_empty(){self.state.drained.store(true,Ordering::Release);}}
        }
    }
}
struct Run {driver:Driver,callback:asio_sys::BufferCallbackId,event:asio_sys::DriverEventCallbackId}
impl Drop for Run {
    fn drop(&mut self) {
        let _=self.driver.stop();
        self.driver.remove_callback(self.callback);
        self.driver.remove_event_callback(self.event);
        let _=self.driver.dispose_buffers();
    }
}
fn event_supported(selector:i32)->bool {
    [AsioMessageSelectors::kAsioResetRequest,AsioMessageSelectors::kAsioResyncRequest,
        AsioMessageSelectors::kAsioLatenciesChanged,AsioMessageSelectors::kAsioBufferSizeChange]
        .iter().any(|value|*value as i32==selector)
}

pub fn play_sequence(request:PlaybackRequest,config:OutputConfig,control:Arc<PlaybackControl>,sequence:PlaybackSequence,mut update:impl FnMut(PlaybackUpdate))->Result<()> {
    control.start.setup(request.is_dsd())?;
    request.validate_dop(&config)?;let dsd=request.is_dsd();let native=dsd&&config.dsd_output==DsdOutput::Native;let dop=dsd&&!native;
    let _sequence_run=sequence.guard()?;
    if !(20..=2000).contains(&config.buffer_ms){bail!("buffer must be 20..2000 ms");}
    if ACTIVE.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return Err(DeviceUnavailable("ASIO 已由本进程的另一个音区使用".into()).into());}
    let _active=Active;
    wasapi::initialize_sta().ok()?;
    let _com=ComApartment;
    let name=config.device_id.as_deref().and_then(|id|id.strip_prefix("asio:")).context("ASIO requires an explicit driver")?;
    let driver=host().lock().map_err(|_|anyhow::anyhow!("ASIO host lock poisoned"))?.load_driver(name)
        .map_err(|error|DeviceUnavailable(format!("无法打开 ASIO 驱动 {name}: {error}")))?;
    let _mode=IoMode{driver:driver.clone(),native};
    if unsafe{rmusic_asio_io_format(i32::from(native))}!=0 {bail!("ASIO 驱动不支持或未确认请求的 {} 模式",if native{"原生 DSD"}else{"PCM"});}
    let target=config.desired_format(&request)?;
    let rate=if dsd{request.sample_rate}else{config.limits.rates(request.sample_rate).into_iter().find(|r|driver.can_sample_rate(*r as f64).unwrap_or(false)).context("ASIO 没有符合所设上限的采样率")?};
    let driver_rate=rate as f64*if native{8.}else{1.};
    if !driver.can_sample_rate(driver_rate)?{bail!("ASIO driver does not support {driver_rate} Hz");}
    // Do not use asio-sys's rate fallback: it starts a temporary buffer before we can clear it.
    if unsafe{rmusic_asio_set_rate(driver_rate)}!=0||driver.sample_rate()?!=driver_rate {
        bail!("ASIO driver did not apply the requested {driver_rate} Hz; set its hardware clock before retrying");
    }
    if driver.channels()?.outs<i32::from(target.channels){bail!("ASIO device has fewer output channels than the source");}
    let mut formats=Vec::new();
    for channel in 0..target.channels {
        let mut kind=0;
        if unsafe{rmusic_asio_channel_type(i32::from(channel),&mut kind)}!=0{bail!("cannot query ASIO output channel {channel}");}
        formats.push(SampleFormat::from_type(kind)?);
    }
    let format=formats[0];
    if formats.iter().any(|value|*value!=format){bail!("ASIO output channels have different sample formats");}
    if native!=(format.bits==1){bail!("ASIO driver channel containers disagree with the selected PCM/DSD mode");}
    let mut required=request.clone();if !dsd{required.source_bit_depth=required.source_bit_depth.min(config.limits.max_bit_depth.unwrap_or(32));}
    format.validate_precision(&required)?;
    let streams=driver.prepare_output_stream(None,target.channels as usize,None)?;
    let output=streams.output.context("ASIO driver did not create output buffers")?;
    let driver_period=usize::try_from(output.buffer_size).ok().filter(|size|*size>0).context("invalid ASIO period")?;
    if native&&driver_period%8!=0 {bail!("ASIO native DSD period must contain complete bytes");}
    let period=driver_period/if native{8}else{1};
    let packet_bytes=period.checked_mul(target.channels as usize).and_then(|size|size.checked_mul(format.bytes)).filter(|size|*size<=32*1024*1024).context("ASIO buffers exceed 32 MiB")?;
    let latency=u64::try_from(driver.latencies()?.output).context("invalid ASIO output latency")?;
    if latency as f64>driver_rate*10. {bail!("invalid ASIO output latency");}
    // Pointers belong to the stopped driver. Run removes the callback before disposing them.
    let buffers:Vec<[usize;2]>=output.buffer_infos.iter().map(|info|info.buffers.map(|pointer|pointer as usize)).collect();
    if buffers.len()!=target.channels as usize||buffers.iter().flatten().any(|pointer|*pointer==0){bail!("invalid ASIO output buffers");}
    let plane_bytes=period*format.bytes;
    let native_silence=if native{Some(if format.big_endian{0x69}else{0x96})}else{None};
    for pair in &buffers {for pointer in pair {unsafe{std::ptr::write_bytes(*pointer as *mut u8,native_silence.unwrap_or(0),plane_bytes);}}}
    if dop {for index in 0..2{format.stamp_dop(&buffers,index,period,0,index as u64*period as u64,true);}}
    let capacity=((u64::from(rate)*u64::from(config.buffer_ms)/1000).div_ceil(period as u64) as usize).clamp(2,64).min((32*1024*1024/packet_bytes).max(1));
    let(sender,receiver)=mpsc::sync_channel::<Packet>(capacity);
    let state=Arc::new(State::default());
    let feedback=super::clock::Session::new(control.clone());
    let mut renderer=Callback{clock_feedback:Some((feedback.epoch(),driver_rate as u64)),buffers,plane_bytes,latency,receiver,state:state.clone(),control:control.clone(),
        timeline:Timeline{spans:VecDeque::with_capacity((latency/driver_period as u64) as usize+4),heard:0,stride:if native{8}else{1}},last_index:None,last_position:0,dop:if dop{Some(format)}else{None},dop_frames:0,native_silence};
    let callback=driver.add_callback(move|info| {
        renderer.render(info.buffer_index,|| {
            let(mut position,mut stamp)=(0,0);
            if unsafe{rmusic_asio_position(&mut position,&mut stamp)}!=0{bail!("ASIO sample clock unavailable");}
            Ok((position,stamp))
        });
    });
    let event_state=state.clone();let expected_rate=driver_rate;
    let event=driver.add_event_callback(move|event| {
        match event {
            AsioDriverEvent::SampleRateChanged(rate)=>{if rate!=expected_rate {event_state.failed.store(true,Ordering::Release);}}
            AsioDriverEvent::Message{selector:AsioMessageSelectors::kAsioSelectorSupported,value}=>return event_supported(value),
            AsioDriverEvent::Message{selector,..}=>{if event_supported(selector as i32){event_state.failed.store(true,Ordering::Release);}}
        }false
    });
    let run=Run{driver,callback,event};
    control.initialize_volume(request.volume)?;
    let output_format=SequenceFormat{sample_rate:rate,channels:target.channels,channel_mask:if target.channels==2{3}else{0},exclusive:true,float_output:format.float,bit_depth:format.bits};
    control.output_sample_rate.store(output_format.sample_rate,Ordering::Relaxed);
    config.resync(&control)?;
    let mut pcm=PcmStream::open(request,sequence.clone(),control.clone(),output_format)?;
    let mut meter=Meter::default();let mut started=false;let mut eof=false;let mut packet=None;
    let mut bytes=Vec::new();let mut frames=0;let mut waiting_since=None;
    let mut publish=|final_end:bool,meter:Meter|->Result<()> {
        let((id,position,source,ended),output_frames)=sequence.heard_with_frames(&control,final_end)?;
        let mut path=super::continuous::path(&source,output_format,&format!("ASIO {name}"),None,control.volume());
        if state.continuity_lost.load(Ordering::Relaxed){path.processing.push("ASIO underrun / continuity lost".into());path.bit_perfect_eligible=false;}
        config.limits.signal_path(&source,&mut path);
        sequence.mark_rate_adjustment(&mut path);
        update(PlaybackUpdate{output_frames,playback_id:id,position,ended,signal_path:path,meter,continuity_lost:state.continuity_lost.load(Ordering::Relaxed)});Ok(())
    };
    // A blocked source read must still be cancelled when the driver stops responding.
    let monitor_done=Arc::new(AtomicBool::new(false));let done=monitor_done.clone();
    let watched=state.clone();let watched_control=control.clone();let running=Arc::new(AtomicBool::new(false));let watched_running=running.clone();
    let timeout=Duration::from_secs_f64((period as f64/output_format.sample_rate as f64*3.).max(2.));
    let monitor=std::thread::spawn(move|| {
        let mut previous=(0,0);let mut changed=Instant::now();
        while !done.load(Ordering::Acquire) {
            if !watched_running.load(Ordering::Acquire){changed=Instant::now();}
            else {
                let current=(watched.callbacks.load(Ordering::Relaxed),watched.device_frame.load(Ordering::Relaxed));
                if current.0!=previous.0&&current.1!=previous.1{changed=Instant::now();previous=current;}
                if changed.elapsed()>timeout{watched.failed.store(true,Ordering::Release);}
                if watched.failed.load(Ordering::Acquire){watched_control.stop();break;}
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let result=(||->Result<()> {loop {
        if state.failed.load(Ordering::Acquire){return Err(DeviceUnavailable("ASIO 设备停止响应或请求重新初始化，请重新选择设备后播放".into()).into());}
        if control.stopped.load(Ordering::Acquire){break;}
        if dsd&&control.volume_setting()!=1.{bail!("原样 DSD 不支持软件音量；请使用 DAC 音量控制");}
        publish(false,meter.clone())?;
        if eof&&packet.is_none()&&(!started||state.drained.load(Ordering::Acquire)){publish(true,meter.clone())?;break;}
        if control.paused.load(Ordering::Relaxed)||(eof&&packet.is_none()) {std::thread::sleep(Duration::from_millis(5));continue;}
        if packet.is_none() {
            let mut waiting=false;
            while frames<period&&!eof {
                match pcm.read(period-frames)? {
                    Read::Data(chunk)=>{bytes.extend_from_slice(&chunk.bytes);frames+=chunk.frames;meter=chunk.meter;}
                    Read::End=>eof=true,
                    Read::Pending=>{waiting=true;break;}
                }
            }
            if waiting {
                let since=waiting_since.get_or_insert_with(Instant::now);
                if (!started||since.elapsed()>Duration::from_secs(2))&&sequence.finish_window_if_empty(){state.continuity_lost.store(true,Ordering::Relaxed);}
                std::thread::sleep(Duration::from_millis(5));continue;
            }
            waiting_since=None;
            if frames>0 {packet=Some(Packet{bytes:format.pack(&bytes,frames,period,output_format.channels as usize),frames});bytes.clear();frames=0;}
        }
        if let Some(next)=packet.take() {
            match sender.try_send(next) {
                Ok(())=>{if !started{if !control.start.wait(&control)?{break;}run.driver.start()?;started=true;running.store(true,Ordering::Release);}}
                Err(mpsc::TrySendError::Full(next))=>{packet=Some(next);std::thread::sleep(Duration::from_millis(2));}
                Err(mpsc::TrySendError::Disconnected(_))=>bail!("ASIO callback disconnected"),
            }
        }
        if eof&&packet.is_none(){state.ended.store(true,Ordering::Release);}
    }Ok(())})();
    monitor_done.store(true,Ordering::Release);let _=monitor.join();
    drop(run);
    if state.failed.load(Ordering::Acquire){return Err(DeviceUnavailable("ASIO 设备停止响应或要求重置".into()).into());}
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_dsd_ffmpeg_preload_seek_and_production_callbacks_preserve_bits_and_clocks() {
        use crate::dsd::tests::{samples,dsf,dff};
        use std::io::Write;
        let rate=22579200_u32;let first=samples(4099);let second=samples(4097);
        let mut a=dsf(&first,4099*8-3,true);a[56..60].copy_from_slice(&rate.to_le_bytes());
        let mut b=dff(&second);let rate_at=b.windows(4).position(|bytes|bytes==b"FS  ").unwrap()+12;b[rate_at..rate_at+4].copy_from_slice(&rate.to_be_bytes());
        let mut files=Vec::new();for (extension,bytes) in [(".dsf",a),(".dff",b)] {
            let mut file=tempfile::Builder::new().suffix(extension).tempfile().unwrap();file.write_all(&bytes).unwrap();file.flush().unwrap();files.push(file);
        }
        for kind in [32,33] {
            let format=SampleFormat::from_type(kind).unwrap();let byte_rate=rate/8;let period=17;
            let initial=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:files[0].path().to_str().unwrap().into(),position:0.6/byte_rate as f64,sample_rate:byte_rate,channels:2,source_integer_lossless:false,source_bit_depth:1,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:None,transitions:Default::default()};
            let native=OutputConfig{device_id:Some("asio:software-only".into()),exclusive:true,dsd_output:DsdOutput::Native,..Default::default()};initial.validate_dop(&native).unwrap();
            assert!(initial.validate_dop(&OutputConfig{device_id:Some("wasapi:unsupported".into()),..native.clone()}).is_err());
            let stream_format=SequenceFormat{sample_rate:byte_rate,channels:2,channel_mask:3,exclusive:true,float_output:false,bit_depth:1};
            let sequence=PlaybackSequence::new("first".into(),initial.clone(),stream_format).unwrap();
            sequence.reserve().unwrap().prepare("second".into(),PlaybackRequest{input:files[1].path().to_str().unwrap().into(),position:0.,..initial.clone()}).unwrap();
            let control=Arc::new(PlaybackControl::default());let mut stream=PcmStream::open(initial.clone(),sequence.clone(),control.clone(),stream_format).unwrap();
            assert_eq!(sequence.position(&control).unwrap().position,1./byte_rate as f64);
            let mut packets=Vec::new();let mut end=false;
            while !end {
                let mut bytes=Vec::new();let mut frames=0;
                while frames<period&&!end {match stream.read(period-frames).unwrap(){Read::Data(chunk)=>{bytes.extend(chunk.bytes);frames+=chunk.frames;},Read::End=>end=true,Read::Pending=>panic!("native DSD next source was preloaded")}}
                if frames>0{packets.push(Packet{bytes:format.pack(&bytes,frames,period,2),frames});}
            }
            assert_eq!(packets.iter().map(|packet|packet.frames).sum::<usize>(),8195);
            let sizes=packets.iter().map(|packet|packet.frames).collect::<Vec<_>>();
            let mut left=[vec![0;period],vec![0;period]];let mut right=[vec![0;period],vec![0;period]];
            let(sender,receiver)=mpsc::sync_channel(packets.len());for packet in packets{sender.send(packet).unwrap();}
            let state=Arc::new(State::default());state.ended.store(true,Ordering::Release);
            let silence=if kind==32{0x96}else{0x69};
            let feedback=super::super::clock::Session::new(control.clone());
        let mut renderer=Callback{clock_feedback:Some((feedback.epoch(),rate as u64)),buffers:vec![left.each_mut().map(|plane|plane.as_mut_ptr() as usize),right.each_mut().map(|plane|plane.as_mut_ptr() as usize)],plane_bytes:period,latency:13,receiver,state:state.clone(),control:control.clone(),timeline:Timeline{spans:VecDeque::new(),heard:0,stride:8},last_index:None,last_position:0,dop:None,dop_frames:0,native_silence:Some(silence)};
            control.paused.store(true,Ordering::Relaxed);renderer.render(0,||Ok((0,0)));assert!(left[0].iter().all(|byte|*byte==silence));assert_eq!(control.frames_written.load(Ordering::Relaxed),0);
            control.paused.store(false,Ordering::Relaxed);let mut received=vec![Vec::new(),Vec::new()];
            for (i,frames) in sizes.iter().copied().enumerate() {
                let index=(i+1)%2;let clock=(i+1) as u64*period as u64*8;
                renderer.render(index as i32,||Ok((clock,clock*100)));
                for (channel,plane) in [&left[index],&right[index]].into_iter().enumerate(){received[channel].extend(plane[..frames].iter().map(|byte|if kind==32{byte.reverse_bits()}else{*byte}));assert!(plane[frames..].iter().all(|byte|*byte==silence));}
                renderer.render(index as i32,||panic!("duplicate native callback consumed source"));
            }
            let final_clock=(sizes.len()+2) as u64*period as u64*8;
            control.muted.store(true,Ordering::Relaxed);renderer.render(((sizes.len()+1)%2) as i32,||Ok((final_clock,final_clock*100)));
            assert!(state.drained.load(Ordering::Acquire));assert_eq!(control.frames_played.load(Ordering::Acquire),8195);
            for channel in 0..2 {
                let mut expected=first[channel][1..].to_vec();let last=expected.last_mut().unwrap();*last=(*last&0xf8)|(0x69&7);expected.extend(&second[channel]);assert_eq!(received[channel],expected);
            }
let sample=control.device_clock.sample().unwrap();assert_eq!(sample.nominal_frequency_hz,rate as u64);assert_eq!(sample.source,super::super::clock::Source::AsioDsdBits);assert_eq!(sample.timestamp_domain,super::super::clock::Domain::DriverOpaque);assert!(sample.host_read_after_ns>=sample.host_read_before_ns);println!("DEVICE_SAMPLE_QA={}",serde_json::to_string(&sample).unwrap());
            let (_,position,_,ended)=sequence.heard(&control,true).unwrap();assert_eq!(position,4097./byte_rate as f64);
            assert_eq!(ended.iter().map(|row|row.frames).collect::<Vec<_>>(),[4098,4097]);assert_eq!(ended[0].position,(4099.*8.-3.)/rate as f64);
            let path=super::super::continuous::path(&initial,stream_format,"ASIO software-only",None,1.);assert_eq!((path.sample_rate,path.bit_depth,path.sample_type.as_str()),(rate,1,"dsd_native"));assert!(path.bit_perfect_eligible);
            let(sender,receiver)=mpsc::sync_channel(1);renderer.receiver=receiver;
            sender.send(Packet{bytes:format.pack(&[1,127,128,64],2,period,2),frames:2}).unwrap();
            let index=(sizes.len()+2)%2;renderer.render(index as i32,||Ok((final_clock+136,1)));
            assert!(left[index].iter().chain(&right[index]).all(|byte|*byte==silence));
            let written=control.frames_written.load(Ordering::Relaxed);control.set_volume(0.5).unwrap();
            sender.send(Packet{bytes:format.pack(&[1,127,128,64],2,period,2),frames:2}).unwrap();
            renderer.render((1-index) as i32,||Ok((final_clock+272,2)));assert_eq!(control.frames_written.load(Ordering::Relaxed),written);
        }
    }
    #[test]
    fn dop_asio_callbacks_keep_valid_carriers_through_pause_mute_tail_and_idle() {
        for kind in [1,2,11,17,18,27] {
            let format=SampleFormat::from_type(kind).unwrap();let period=3;let width=period*format.bytes;
            let mut left=[vec![0;width],vec![0;width]];let mut right=[vec![0;width],vec![0;width]];
            let(sender,receiver)=mpsc::sync_channel(1);let control=Arc::new(PlaybackControl::default());let state=Arc::new(State::default());
            let mut renderer=Callback{clock_feedback:None,buffers:vec![left.each_mut().map(|plane|plane.as_mut_ptr() as usize),right.each_mut().map(|plane|plane.as_mut_ptr() as usize)],plane_bytes:width,latency:1,receiver,state:state.clone(),control:control.clone(),timeline:Timeline{spans:VecDeque::new(),heard:0,stride:1},last_index:None,last_position:0,dop:Some(format),dop_frames:0,native_silence:None};
            let mut raw=[0;16];crate::dsd::DopEncoder::new(2).unwrap().pack(&[1,0x7f,0x80,0x40,0x23,0x45,0x67,0x89],&mut raw,crate::dsd::DopContainer::LeftAligned32,false).unwrap();
            sender.send(Packet{bytes:format.pack(&raw,2,period,2),frames:2}).unwrap();state.ended.store(true,Ordering::Relaxed);
            let word=|sample:&[u8]| {let mut bytes=[0;4];bytes[..format.bytes].copy_from_slice(sample);if format.big_endian{bytes[..format.bytes].reverse();}let word=u32::from_le_bytes(bytes);if format.bits==32{word>>8}else{word&0xffffff}};
            control.paused.store(true,Ordering::Relaxed);renderer.render(0,||Ok((0,0)));
            for (frame,sample) in left[0].chunks_exact(format.bytes).enumerate(){assert_eq!(word(sample),(if frame%2==0{5}else{250})<<16|0x6969);}
            assert_eq!(control.frames_written.load(Ordering::Relaxed),0);
            control.paused.store(false,Ordering::Relaxed);renderer.render(1,||Ok((3,300)));
            assert_eq!(word(&left[1][..format.bytes]),0xfa0180);assert_eq!(word(&right[1][..format.bytes]),0xfa7f40);
            assert_eq!(word(&left[1][format.bytes..format.bytes*2]),0x052367);assert_eq!(word(&left[1][format.bytes*2..]),0xfa6969);
            renderer.render(1,||panic!("duplicate callback must not advance DoP markers"));
            control.muted.store(true,Ordering::Relaxed);renderer.render(0,||Ok((6,600)));
            assert_eq!(control.frames_written.load(Ordering::Relaxed),2);assert_eq!(control.frames_played.load(Ordering::Acquire),2);
            for (frame,sample) in left[0].chunks_exact(format.bytes).enumerate(){assert_eq!(word(sample),(if frame%2==0{5}else{250})<<16|0x6969);}
            assert!(state.drained.load(Ordering::Acquire));
        }
    }
    #[test]
    fn asio_18_20_bit_ffmpeg_preload_and_callbacks_preserve_source_precision() {
        use std::io::Write;
        let samples=[i16::MIN,i16::MAX,-1,1,0,12345,-23456,7,32766,-32767,2,-2];
        let raw=samples.iter().flat_map(|value|value.to_le_bytes()).collect::<Vec<_>>();
        let mut wav=b"RIFF".to_vec();wav.extend((36+raw.len() as u32).to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(16_u32.to_le_bytes());wav.extend(1_u16.to_le_bytes());wav.extend(2_u16.to_le_bytes());wav.extend(768000_u32.to_le_bytes());wav.extend(3072000_u32.to_le_bytes());wav.extend(4_u16.to_le_bytes());wav.extend(16_u16.to_le_bytes());wav.extend(b"data");wav.extend((raw.len() as u32).to_le_bytes());wav.extend(raw);
        let mut file=tempfile::Builder::new().suffix(".wav").tempfile().unwrap();file.write_all(&wav).unwrap();file.flush().unwrap();
        for (kind,bits) in [(9,18),(25,18),(10,20),(26,20)] {
            let format=SampleFormat::from_type(kind).unwrap();assert_eq!((format.bytes,format.bits,format.float),(4,bits,false));
            let request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:file.path().to_str().unwrap().into(),position:0.,sample_rate:768000,channels:2,source_integer_lossless:true,source_bit_depth:16,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:None,transitions:Default::default()};
            format.validate_precision(&request).unwrap();
            for depth in [0,24,32]{assert!(format.validate_precision(&PlaybackRequest{source_bit_depth:depth,..request.clone()}).is_err());}
            assert!(format.validate_precision(&PlaybackRequest{source_bit_depth:1,source_integer_lossless:false,..request.clone()}).is_err());
            let output=SequenceFormat{sample_rate:768000,channels:2,channel_mask:3,exclusive:true,float_output:false,bit_depth:bits};
            let sequence=PlaybackSequence::new("first".into(),request.clone(),output).unwrap();sequence.reserve().unwrap().prepare("second".into(),request.clone()).unwrap();
            let control=Arc::new(PlaybackControl::default());control.initialize_volume(1.).unwrap();
            let mut stream=PcmStream::open(request,sequence.clone(),control.clone(),output).unwrap();
            let period=5;let width=period*4;let mut packets=Vec::new();let mut end=false;
            while !end {let mut bytes=Vec::new();let mut frames=0;while frames<period&&!end {match stream.read(period-frames).unwrap(){Read::Data(chunk)=>{bytes.extend(chunk.bytes);frames+=chunk.frames;},Read::End=>end=true,Read::Pending=>panic!("next source already preloaded")}}if frames>0{packets.push(Packet{bytes:format.pack(&bytes,frames,period,2),frames});}}
            assert_eq!(packets.iter().map(|packet|packet.frames).collect::<Vec<_>>(),[5,5,2]);
            let mut left=[vec![255;width],vec![255;width]];let mut right=[vec![255;width],vec![255;width]];
            let(sender,receiver)=mpsc::sync_channel(3);for packet in packets{sender.send(packet).unwrap();}
            let state=Arc::new(State::default());state.ended.store(true,Ordering::Release);
            let mut renderer=Callback{clock_feedback:None,buffers:vec![left.each_mut().map(|plane|plane.as_mut_ptr() as usize),right.each_mut().map(|plane|plane.as_mut_ptr() as usize)],plane_bytes:width,latency:1,receiver,state:state.clone(),control:control.clone(),timeline:Timeline{spans:VecDeque::new(),heard:0,stride:1},last_index:None,last_position:0,dop:None,dop_frames:0,native_silence:None};
            control.paused.store(true,Ordering::Relaxed);renderer.render(0,||Ok((0,0)));assert_eq!(control.frames_written.load(Ordering::Relaxed),0);assert!(left[0].iter().chain(&right[0]).all(|byte|*byte==0));control.paused.store(false,Ordering::Relaxed);
            let mut actual=Vec::new();for (i,frames) in [5,5,2].into_iter().enumerate(){let index=(i+1)%2;renderer.render(index as i32,||Ok(((i+1) as u64*5,1)));for frame in 0..frames {for plane in [&left[index],&right[index]]{let mut bytes:[u8;4]=plane[frame*4..frame*4+4].try_into().unwrap();if kind<16{bytes.reverse();}actual.push(i32::from_le_bytes(bytes));}}assert!(left[index][frames*4..].iter().chain(&right[index][frames*4..]).all(|byte|*byte==0));}
            assert_eq!(actual,samples.iter().chain(&samples).map(|value|i32::from(*value)*(1_i32<<(bits-16))).collect::<Vec<_>>());
            renderer.render(0,||Ok((21,2)));assert!(state.drained.load(Ordering::Acquire));assert_eq!(control.frames_played.load(Ordering::Acquire),12);
            assert_eq!(sequence.heard(&control,true).unwrap().3.iter().map(|row|row.frames).collect::<Vec<_>>(),[6,6]);
        }
    }
    #[test]
    fn asio_planar_integer_vectors_preserve_channels_precision_and_zero_tail() {
        let samples=[i32::MIN,i32::MAX,0x12345600,-256,0,256];
        let input=samples.iter().flat_map(|sample|sample.to_le_bytes()).collect::<Vec<_>>();
        assert_eq!(SampleFormat::from_type(17).unwrap().pack(&input,3,4,2),
            [0,0,128,86,52,18,0,0,0,0,0,0,255,255,127,255,255,255,1,0,0,0,0,0]);
        assert_eq!(SampleFormat::from_type(27).unwrap().pack(&input,3,3,2),
            [0,0,128,255,86,52,18,0,0,0,0,0,255,255,127,0,255,255,255,255,1,0,0,0]);
        assert_eq!(SampleFormat::from_type(1).unwrap().pack(&input,3,3,2),
            [128,0,0,18,52,86,0,0,0,127,255,255,255,255,255,0,0,1]);
        let packed=SampleFormat::from_type(18).unwrap().pack(&input,3,3,2);
        assert_eq!(packed,samples.iter().step_by(2).chain(samples.iter().skip(1).step_by(2)).flat_map(|sample|sample.to_le_bytes()).collect::<Vec<_>>());
        for kind in [40,99]{assert!(SampleFormat::from_type(kind).is_err());}
    }
    #[test]
    fn asio_device_clock_excludes_padding_pause_and_underrun_silence() {
        let mut timeline=Timeline{spans:VecDeque::new(),heard:0,stride:1};
        timeline.submitted(128,0,3);timeline.submitted(384,3,257);
        for (device,expected) in [(0,0),(128,0),(130,2),(256,3),(383,3),(384,3),(400,19),(641,260),(900,260)]{assert_eq!(timeline.advance(device),expected);}
        assert!(timeline.spans.is_empty());
    }
    #[test]
    fn asio_float_containers_and_required_integer_precision_are_explicit() {
        let input=[-1_f64,0.5,0.,1.].iter().flat_map(|sample|sample.to_le_bytes()).collect::<Vec<_>>();
        let packed=SampleFormat::from_type(20).unwrap().pack(&input,2,2,2);
        assert_eq!(packed,[-1_f64,0.,0.5,1.].iter().flat_map(|sample|sample.to_le_bytes()).collect::<Vec<_>>());
        let mut request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:String::new(),position:0.,sample_rate:768000,channels:2,source_integer_lossless:true,source_bit_depth:32,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:None,transitions:Default::default()};
        assert!(SampleFormat::from_type(17).unwrap().validate_precision(&request).is_err());
        request.source_bit_depth=0;assert!(SampleFormat::from_type(17).unwrap().validate_precision(&request).is_err());
        assert!(SampleFormat::from_type(19).unwrap().validate_precision(&request).is_err());
        request.source_bit_depth=32;assert!(SampleFormat::from_type(20).unwrap().validate_precision(&request).is_ok());
        request.source_bit_depth=24;assert!(SampleFormat::from_type(17).unwrap().validate_precision(&request).is_ok());
        let config=OutputConfig{device_id:Some("asio:explicit".into()),..Default::default()}.normalized();
        assert!(config.exclusive);assert_eq!(config.backend(),"asio");
    }
    #[test]
    fn programme_clock_asio_768khz_callback_preserves_exact_pcm_planes() {
        use std::io::Write;
        fn source(raw:&[u8],bits:u16,codec:u16)->tempfile::NamedTempFile {
            let mut file=tempfile::Builder::new().suffix(".wav").tempfile().unwrap();let size=raw.len() as u32;
            for bytes in [b"RIFF".as_slice(),&(36+size).to_le_bytes(),b"WAVEfmt ",&16_u32.to_le_bytes(),&codec.to_le_bytes(),&2_u16.to_le_bytes(),&768000_u32.to_le_bytes(),&(768000*2*u32::from(bits/8)).to_le_bytes(),&(2*(bits/8)).to_le_bytes(),&bits.to_le_bytes(),b"data",&size.to_le_bytes()]{file.write_all(bytes).unwrap();}
            file.write_all(raw).unwrap();file.flush().unwrap();file
        }
        let samples=(0..1040).map(|i|match i%5 {0=>i32::MIN,1=>i32::MAX,2=>1,3=>-1,_=>i*7919}).collect::<Vec<_>>();
        let raw=samples.iter().flat_map(|sample|sample.to_le_bytes()).collect::<Vec<_>>();
        let first=source(&raw[..514*4],32,1);let second=source(&raw[514*4..],32,1);
        let mut request=PlaybackRequest{flac_samples: None, dsd_pcm: None, input:first.path().to_str().unwrap().into(),position:0.,sample_rate:768000,channels:2,source_integer_lossless:true,source_bit_depth:32,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:None,transitions:Default::default()};
        let format=SequenceFormat{sample_rate:768000,channels:2,channel_mask:3,exclusive:true,float_output:true,bit_depth:64};
        let sequence=PlaybackSequence::new("first".into(),request.clone(),format).unwrap();
        let initial=request.clone();request.input=second.path().to_str().unwrap().into();
        sequence.reserve().unwrap().prepare("second".into(),request).unwrap();
        let control=Arc::new(PlaybackControl::default());control.initialize_volume(1.).unwrap();
        let mut pcm=PcmStream::open(initial.clone(),sequence.clone(),control.clone(),format).unwrap();
        let mut packets=Vec::new();let mut end=false;
        while !end {
            let mut bytes=Vec::new();let mut frames=0;
            while frames<256&&!end {match pcm.read(256-frames).unwrap(){Read::Data(chunk)=>{bytes.extend(chunk.bytes);frames+=chunk.frames;},Read::End=>end=true,Read::Pending=>panic!("both local sources were prepared")}}
            if frames>0{packets.push(Packet{bytes:SampleFormat::from_type(20).unwrap().pack(&bytes,frames,256,2),frames});}
        }
        assert_eq!(packets.iter().map(|packet|packet.frames).collect::<Vec<_>>(),[256,256,8]);
        let mut left=[vec![255_u8;2048],vec![255_u8;2048]];let mut right=[vec![255_u8;2048],vec![255_u8;2048]];
        let(sender,receiver)=mpsc::sync_channel(3);let state=Arc::new(State::default());
        control.output_sample_rate.store(768000,Ordering::Release);
        let feedback=super::super::clock::Session::new(control.clone());
        let mut renderer=Callback{clock_feedback:Some((feedback.epoch(),768000)),buffers:vec![left.each_mut().map(|buffer|buffer.as_mut_ptr() as usize),right.each_mut().map(|buffer|buffer.as_mut_ptr() as usize)],plane_bytes:2048,latency:16,receiver,state:state.clone(),control:control.clone(),timeline:Timeline{spans:VecDeque::with_capacity(4),heard:0,stride:1},last_index:None,last_position:0,dop:None,dop_frames:0,native_silence:None};
        for packet in packets {sender.send(packet).unwrap();}
        state.ended.store(true,Ordering::Release);control.paused.store(true,Ordering::Relaxed);
        renderer.render(0,||Ok((0,100)));assert_eq!(control.frames_written.load(Ordering::Relaxed),0);assert!(left[0].iter().all(|byte|*byte==0));
        control.paused.store(false,Ordering::Relaxed);
        let mut played=Vec::new();
        for (index,clock,frames) in [(1,256,256),(0,512,256),(1,768,8)] {
            renderer.render(index,||Ok((clock,clock*100)));let index=index as usize;
            for frame in 0..frames {for plane in [&left[index],&right[index]]{played.push(f64::from_le_bytes(plane[frame*8..frame*8+8].try_into().unwrap()));}}
            assert!(left[index][frames*8..].iter().all(|byte|*byte==0));
            renderer.render(index as i32,||panic!("duplicate buffer callbacks must not read the clock or consume audio"));
        }
        renderer.render(0,||Ok((1024,102400)));assert!(state.drained.load(Ordering::Acquire));
        assert_eq!(control.frames_written.load(Ordering::Relaxed),520);assert_eq!(control.frames_played.load(Ordering::Acquire),520);
        assert_eq!(played,samples.iter().map(|sample|*sample as f64/2147483648.).collect::<Vec<_>>());
let sample=control.device_clock.sample().unwrap();assert_eq!(sample.programme,Some(super::super::clock::Programme{frames:520,sample_rate:768000}));assert_eq!(sample.nominal_frequency_hz,768000);assert_eq!(sample.source,super::super::clock::Source::AsioFrames);assert_eq!(sample.timestamp_domain,super::super::clock::Domain::DriverOpaque);assert!(sample.host_read_after_ns>=sample.host_read_before_ns);println!("DEVICE_SAMPLE_QA={}",serde_json::to_string(&sample).unwrap());
        let(_,position,_,ended)=sequence.heard(&control,true).unwrap();
        assert_eq!(position,263./768000.);assert_eq!(ended.iter().map(|track|track.frames).collect::<Vec<_>>(),[257,263]);
        assert!(!state.continuity_lost.load(Ordering::Relaxed));
        renderer.render(1,||bail!("driver clock disappeared"));assert!(state.failed.load(Ordering::Acquire));assert!(left[1].iter().all(|byte|*byte==0));
        assert!(is_device_unavailable(&asio_sys::AsioError::NoDrivers.into()));
        assert!(!is_device_unavailable(&asio_sys::AsioError::InvalidInput.into()));
        // Float64 sources also bypass s32/f32 quantization, including next-track primers.
        let values=[0.1234567890123_f64,1./(1_u64<<40) as f64];
        let raw=values.iter().flat_map(|value|value.to_le_bytes()).collect::<Vec<_>>();
        let file=source(&raw,64,3);
        let request=PlaybackRequest{input:file.path().to_str().unwrap().into(),source_integer_lossless:false,source_bit_depth:64,..initial};
        let sequence=PlaybackSequence::new("f64-first".into(),request.clone(),format).unwrap();
        sequence.reserve().unwrap().prepare("f64-second".into(),request.clone()).unwrap();
        let mut pcm=PcmStream::open(request,sequence,Arc::new(PlaybackControl::default()),format).unwrap();
        for _ in 0..2 {let Read::Data(chunk)=pcm.read(1).unwrap() else {panic!("expected one float64 frame")};assert_eq!(chunk.bytes,raw);}
    }
}
