//! CoreAudio HAL output adapter; the shared engine retains decoding, DSP and track accounting.
use anyhow::{bail, Result};
use rmusic_audio::output::{external::{Backend, Driver, Progress,ProgrammeClock}, DeviceInfo, DeviceUnavailable, OutputConfig, PlaybackRequest, SequenceFormat, SignalPath};
use std::{ffi::{c_char, c_void, CStr, CString}, ptr::NonNull};
pub const SYSTEM_DEFAULT_UID: &str = "rmusic:system-default";
#[repr(C)]
struct Device { uid: [c_char;512], name: [c_char;512], rate:u32, channels:u32, is_default:u32 }
#[repr(C)]
#[derive(Default)]
struct Snapshot { played:u64, device_frame:u64, device_ns:u64, before_ns:u64, after_ns:u64, queued:u64, running:u32, underrun:u32, capacity:u32, quantum:u32,capture_frame:u64,capture_ns:u64,capture_before:u64,capture_after:u64,programme_frames:u64,capture_valid:u32 }
#[repr(C)]
#[derive(Default,Debug)]
struct FeedDiagnostic {producer_gap_ns:u64,callback_late_ns:u64,queued:u32,requested:u32}
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
pub struct DeviceVolume { pub supported: u32, pub value: f32, pub muted: u32, pub mute_supported: u32 }
#[repr(C)]
#[derive(Default,Clone,Copy,serde::Serialize)]
pub struct PhysicalFormat { pub min_rate:f64,pub max_rate:f64,pub bits:u32,pub channels:u32,pub floating:u32 }
#[repr(C)]
#[derive(Default)]
struct RawCapabilities {count:u32,rate_writable:u32,exclusive:u32}
#[derive(serde::Serialize)]
pub struct Capabilities {pub formats:Vec<PhysicalFormat>,pub sample_rates:Vec<u32>,pub bit_depths:Vec<u16>,pub rate_writable:bool,pub exclusive:bool,pub dsd:Vec<String>,pub conversion_locations:Vec<String>}
impl Capabilities {
    fn pcm_format(&self, request:&PlaybackRequest, config:&OutputConfig)->Result<SequenceFormat> {
        let mut target=config.desired_format(request)?;
        let ceiling=config.limits.max_sample_rate.unwrap_or(768000);
        // Automatic channel routing preserves the source layout when the device can
        // carry it. Otherwise the shared decoder performs an explicit downmix.
        // An explicit user channel setting is never silently replaced.
        if config.limits.channels.is_none() && !request.is_dsd() {
            let supported=self.formats.iter().filter(|f|f.min_rate<=f64::from(ceiling)&&f.channels>0)
                .map(|f|f.channels.min(u32::from(target.channels)) as u16).max();
            if let Some(channels)=supported {target.channels=channels;}
        }
        target.sample_rate=self.rate(request.sample_rate,ceiling,target.channels)?;
        Ok(target)
    }
    fn rate(&self,source:u32,ceiling:u32,channels:u16)->Result<u32> {
        let supports=|rate:u32|self.formats.iter().any(|f|rate as f64>=f.min_rate&&rate as f64<=f.max_rate&&f.channels>=u32::from(channels));
        if source<=ceiling&&supports(source){return Ok(source);}
        let mut candidates:Vec<u32>=self.sample_rates.iter().copied().filter(|r|*r<=ceiling&&supports(*r)).collect();
        // Exact source rate wins. Otherwise avoid discarding source bandwidth where the ceiling allows it.
        let family=if source%44100==0{44100}else{48000};
        candidates.sort_by_key(|r|(*r<source,*r%family!=0,source.abs_diff(*r)));
        candidates.first().copied().ok_or_else(||anyhow::anyhow!("设备没有符合采样率上限与声道配置的已报告格式（上限 {ceiling} Hz，{channels} 声道；音源 {source} Hz）"))
    }
}
pub fn capabilities(uid:&str)->Result<Capabilities>{
    anyhow::ensure!(!uid.is_empty(),"必须指定设备 UID");let uid=CString::new(uid)?;
    let mut formats=vec![PhysicalFormat::default();512];let mut raw=RawCapabilities::default();
    checked(unsafe{rm_capabilities(uid.as_ptr(),formats.as_mut_ptr(),512,&mut raw)})?;
    formats.truncate(raw.count as usize);
    let mut rates:Vec<u32>=rmusic_audio::output::PCM_RATES.iter().copied().filter(|r|formats.iter().any(|f|*r as f64>=f.min_rate&&*r as f64<=f.max_rate)).collect();
    for f in &formats {for rate in [f.min_rate,f.max_rate]{if rate.fract()==0.&&(8000. ..=768000.).contains(&rate){if rmusic_audio::output::PCM_RATES.contains(&(rate as u32)){rates.push(rate as u32);}}}}
    rates.sort_unstable();rates.dedup();
    let mut bits:Vec<u16>=formats.iter().filter_map(|f|{let n=if f.floating!=0 {if f.bits==64{32}else{24}}else{f.bits};[16,24,32].contains(&n).then_some(n as u16)}).collect();bits.sort_unstable();bits.dedup();
    Ok(Capabilities{formats,sample_rates:rates,bit_depths:bits,rate_writable:raw.rate_writable!=0,exclusive:raw.exclusive!=0,dsd:vec![],conversion_locations:vec!["device".into(),"core".into()]})
}
unsafe extern "C" {
    fn rm_device_volume(uid:*const c_char, state:*mut DeviceVolume)->i32;
    fn rm_set_device_volume(uid:*const c_char, value:f32, state:*mut DeviceVolume)->i32;
    fn rm_set_device_mute(uid:*const c_char, value:u32, state:*mut DeviceVolume)->i32;
    fn rm_capabilities(uid:*const c_char,formats:*mut PhysicalFormat,capacity:u32,result:*mut RawCapabilities)->i32;
    fn rm_devices(items:*mut Device,capacity:u32,count:*mut u32)->i32;
    fn rm_open(uid:*const c_char,rate:u32,channels:u32,exclusive:u32,bits:u32,floating:u32,buffer_ms:u32,output:*mut *mut c_void)->i32;
    fn rm_user_pause(output:*mut c_void,paused:u32)->i32;
    fn rm_rate(output:*mut c_void)->u32;
    fn rm_route_bits(output:*mut c_void)->u32;
    fn rm_route_float(output:*mut c_void)->u32;
    fn rm_precision(output:*mut c_void)->u32;
    fn rm_write(output:*mut c_void,bytes:*const u8,frames:u32)->u32;
    fn rm_write_final(output:*mut c_void,bytes:*const u8,frames:u32)->u32;
    fn pthread_set_qos_class_self_np(class:u32,relative_priority:i32)->i32;
    fn rm_start(output:*mut c_void)->i32;
    fn rm_start_at(output:*mut c_void,presentation_ns:u64)->i32;
    fn rm_pause(output:*mut c_void,paused:u32);
    fn rm_wait(output:*mut c_void,milliseconds:u32)->i32;
    fn rm_finish_cycle(output:*mut c_void);
    fn rm_underrun_frame(output:*mut c_void)->u64;
    fn rm_feed_diagnostic(output:*mut c_void,diagnostic:*mut FeedDiagnostic);
    fn rm_finish(output:*mut c_void);
    fn rm_progress(output:*mut c_void,progress:*mut Snapshot)->i32;
    fn rm_close(output:*mut c_void);
}
fn checked(status:i32)->Result<()> {
    if status==0 {return Ok(());}
    if status == -70003 {
        return Err(rmusic_audio::output::DeviceClockInterrupted("设备时钟中断，已保留实际消费位置".into()).into());
    }
    if status == -70012 {return Err(DeviceUnavailable("系统默认输出已改变；设备刷新后按新默认路由重新协商格式并恢复。".into()).into());}
    let reason=match status {
        -70001=>"所选设备已断开、不可用或声道配置已改变",
        -70002=>"设备格式或采样率已改变，不能继续当前输出",
        -70006=>"CoreAudio 回调缓冲布局与已协商格式不一致",
        -70007=>"设备实际采样率已被改变",
        -70008=>"设备输出声道已被改变",
        -70009=>"CoreAudio 回调没有提供有效的设备时间戳",
        -70010=>"CoreAudio 回调时间或设备帧发生回退",
        -70011=>"CoreAudio 消费位置记录队列已满",
        -70013=>"CoreAudio 供帧唤醒通道不可用",
        -70005=>"设备正被其他输出独占，无法接管",
        -70004=>"无法为音频输出分配缓冲",
        _=>"设备拒绝输出，可能被其他程序独占或不支持当前格式",
    };
    Err(DeviceUnavailable(format!("{reason}（CoreAudio {status}）。检查设备后手动恢复。未自动切换设备。")).into())
}
pub struct CoreAudio;
impl Backend for CoreAudio {
    fn scheduled(&self)->bool {true}
    fn devices(&self)->Result<Vec<DeviceInfo>> {
        let mut devices:Vec<Device>=(0..256).map(|_|Device{uid:[0;512],name:[0;512],rate:0,channels:0,is_default:0}).collect();
        let mut count=0;checked(unsafe{rm_devices(devices.as_mut_ptr(),256,&mut count)})?;
        let mut devices:Vec<_>=devices.into_iter().take(count as usize).map(|d|DeviceInfo {
            id:unsafe{CStr::from_ptr(d.uid.as_ptr())}.to_string_lossy().into_owned(),
            name:unsafe{CStr::from_ptr(d.name.as_ptr())}.to_string_lossy().into_owned(),
            mix_rate:d.rate,channels:d.channels as u16,default:d.is_default!=0,
        }).collect();
        devices.sort_by(|a,b|b.default.cmp(&a.default).then(a.name.cmp(&b.name)));
        if let Some(mut route)=devices.iter().find(|d|d.default).cloned(){route.id=SYSTEM_DEFAULT_UID.into();route.name="系统默认输出".into();devices.insert(0,route);}
        Ok(devices)
    }
    fn prepare_decode_thread(&self){unsafe{pthread_set_qos_class_self_np(0x19,0);}}
    fn open(&self,request:&PlaybackRequest,config:&OutputConfig)->Result<Box<dyn Driver>> {
        unsafe{pthread_set_qos_class_self_np(0x21,0);}
        let capabilities=capabilities(config.device_id.as_deref().unwrap_or(""))?;
        let target=capabilities.pcm_format(request,config)?;
        let uid=CString::new(config.device_id.as_deref().unwrap_or(""))?;
        let mut pointer=std::ptr::null_mut();
        checked(unsafe{rm_open(uid.as_ptr(),target.sample_rate,u32::from(target.channels),config.exclusive.into(),u32::from(target.bit_depth),target.float_output.into(),config.buffer_ms,&mut pointer)})?;
        let pointer=NonNull::new(pointer).ok_or_else(||anyhow::anyhow!("CoreAudio returned no output"))?;
        let mut output=Output{pointer,reported_underrun:false,format:SequenceFormat{bit_depth:if target.float_output{64}else{32},..target},precision:unsafe{rm_precision(pointer.as_ptr())} as u16,route_bits:unsafe{rm_route_bits(pointer.as_ptr())},route_float:unsafe{rm_route_float(pointer.as_ptr())}!=0,core_decode:config.limits.conversion_location==rmusic_audio::output::ConversionLocation::Core};
        output.format.sample_rate=unsafe{rm_rate(pointer.as_ptr())};
        if config.limits.max_sample_rate.is_some_and(|rate|output.format.sample_rate>rate) {bail!("系统设备采样率超过所设上限；请调整系统格式或选择独占模式");}
        if !output.format.float_output {output.format.bit_depth=output.precision;}
        Ok(Box::new(output))
    }
}
struct Output {core_decode:bool,reported_underrun:bool,pointer:NonNull<c_void>,format:SequenceFormat,precision:u16,route_bits:u32,route_float:bool}
impl Drop for Output {fn drop(&mut self){unsafe{rm_close(self.pointer.as_ptr())}}}
impl Driver for Output {
    fn format(&self)->SequenceFormat {self.format}
    fn path(&self,source:&PlaybackRequest,volume:f64)->SignalPath {
        let mut processing=vec![format!("HAL client: {} / {} bit container",if self.format.float_output{"float"}else{"integer"},if self.format.float_output{64}else{32}),"Position: HAL output timestamps plus reported device latency; hardware unverified".into()];
        processing.push(format!("HAL driver: {} {} bit",if self.route_float{"float"}else{"integer"},self.route_bits));
        if !self.format.exclusive {processing.push("CoreAudio shared/system mixing route".into());}
        if let Some(rate)=source.dsd_pcm {processing.push(format!("DSD PCM decode ({}): {} Hz / 1 bit → {} Hz PCM",if self.core_decode{"core"}else{"device"},rate,self.format.sample_rate));}
        if source.dsd_pcm.is_none()&&source.sample_rate!=self.format.sample_rate {processing.push(format!("FFmpeg rate conversion {} → {} Hz",source.sample_rate,self.format.sample_rate));}
        if source.channels!=self.format.channels {processing.push(format!("FFmpeg channel conversion {} → {}",source.channels,self.format.channels));}
        if source.dsp.enabled {processing.push("Shared DSP f64".into());}
        if source.dsp.enabled && source.dsp.normalize && source.measured_lufs.is_none() {
            processing.push("Normalization pending measured LUFS".into());
        }
        if source.transitions.enabled(){processing.push("Shared fade envelope".into());}
        if volume!=1. {processing.push("Software volume".into());}
        if source.source_bit_depth>self.precision {processing.push(format!("Device route precision {} → {} bit",source.source_bit_depth,self.precision));}
        SignalPath{output:"CoreAudio HAL".into(),sample_rate:self.format.sample_rate,channels:self.format.channels,
            exclusive:self.format.exclusive,bit_depth:self.precision,sample_type:if self.route_float{"float"}else{"integer"}.into(),
            mixer_sample_rate:(!self.format.exclusive).then_some(self.format.sample_rate),bit_perfect_eligible:false,processing}
    }
    fn write(&mut self,bytes:&[u8])->Result<usize>{let width=usize::from(self.format.channels)*if self.format.float_output{8}else{4};if bytes.len()%width!=0{bail!("unaligned CoreAudio frame");}Ok(unsafe{rm_write(self.pointer.as_ptr(),bytes.as_ptr(),(bytes.len()/width) as u32)} as usize)}
    fn write_final(&mut self,bytes:&[u8])->Result<usize>{
        let width=usize::from(self.format.channels)*if self.format.float_output{8}else{4};
        anyhow::ensure!(bytes.len()%width==0,"unaligned CoreAudio frame");
        Ok(unsafe{rm_write_final(self.pointer.as_ptr(),bytes.as_ptr(),(bytes.len()/width) as u32)} as usize)
    }
    fn start(&mut self)->Result<()>{checked(unsafe{rm_start(self.pointer.as_ptr())})}
    fn start_at(&mut self,deadline:u64)->Result<()>{checked(unsafe{rm_start_at(self.pointer.as_ptr(),deadline)})}
    fn pause(&mut self,paused:bool){unsafe{rm_pause(self.pointer.as_ptr(),paused.into())}}
    fn user_pause(&mut self,paused:bool)->Result<()>{checked(unsafe{rm_user_pause(self.pointer.as_ptr(),paused.into())})}
    fn wait_for_output(&mut self)->Result<()>{checked(unsafe{rm_wait(self.pointer.as_ptr(),10)})}
    fn finish_cycle(&mut self){unsafe{rm_finish_cycle(self.pointer.as_ptr())}}
    fn finish_input(&mut self){unsafe{rm_finish(self.pointer.as_ptr())}}
    fn progress(&mut self)->Result<Progress>{let mut s=Snapshot::default();checked(unsafe{rm_progress(self.pointer.as_ptr(),&mut s)})?;if s.underrun!=0 && !self.reported_underrun {self.reported_underrun=true;let mut diagnostic=FeedDiagnostic::default();unsafe{rm_feed_diagnostic(self.pointer.as_ptr(),&mut diagnostic)};eprintln!("CoreAudio continuity lost after {} programme frames at {} Hz: {:?}",unsafe{rm_underrun_frame(self.pointer.as_ptr())},self.format.sample_rate,diagnostic);}
        Ok(Progress{programme_clock:(s.capture_valid!=0).then_some(ProgrammeClock{device_frame:s.capture_frame,programme_frames:s.programme_frames,time_ns:s.capture_ns,before_ns:s.capture_before,after_ns:s.capture_after}),queued:s.queued,capacity:s.capacity,quantum:s.quantum,played:s.played,device_frame:s.device_frame,device_time_ns:s.device_ns,host_before_ns:s.before_ns,host_after_ns:s.after_ns,running:s.running!=0,continuity_lost:s.underrun!=0})}
}
pub fn install()->Result<()> {rmusic_audio::output::external::install(Box::new(CoreAudio))}

/// Read only: driver-exposed device volume does not prove an analogue attenuation stage.
pub fn device_volume(uid:&str)->Result<DeviceVolume> {
    anyhow::ensure!(!uid.is_empty(),"必须指定设备 UID，不能回退到系统默认输出");
    let uid=CString::new(uid)?;let mut state=DeviceVolume::default();
    let status=unsafe{rm_device_volume(uid.as_ptr(),&mut state)};
    anyhow::ensure!(status==0,"无法读取设备音量（CoreAudio {status}）");Ok(state)
}
pub fn set_device_volume(uid:&str,value:f64)->Result<DeviceVolume> {
    anyhow::ensure!(!uid.is_empty()&&value.is_finite()&&(0. ..=1.).contains(&value),"设备 UID 或音量无效");
    let uid=CString::new(uid)?;let mut state=DeviceVolume::default();
    let status=unsafe{rm_set_device_volume(uid.as_ptr(),value as f32,&mut state)};
    anyhow::ensure!(status==0,"设备音量写入失败（CoreAudio {status}）；没有启用软件音量，请重新读取设备状态");
    anyhow::ensure!(state.supported!=0,"设备音量控件已消失；没有启用软件音量");Ok(state)
}
pub fn set_device_mute(uid: &str, muted: bool) -> Result<DeviceVolume> {
    anyhow::ensure!(!uid.is_empty(), "缺少输出设备 UID");
    let uid = CString::new(uid)?;
    let mut state = DeviceVolume::default();
    let status = unsafe { rm_set_device_mute(uid.as_ptr(), u32::from(muted), &mut state) };
    anyhow::ensure!(status == 0, "无法切换设备静音（CoreAudio {status}）");
    Ok(state)
}
#[cfg(test)] mod volume_tests {
    #[test] fn system_default_is_an_explicit_route_with_the_real_devices_capabilities() {
        use rmusic_audio::output::external::Backend;
        let devices=super::CoreAudio.devices().unwrap();
        let Some(physical)=devices.iter().find(|d|d.default&&d.id!=super::SYSTEM_DEFAULT_UID) else{return};
        let route=devices.iter().find(|d|d.id==super::SYSTEM_DEFAULT_UID).unwrap();
        assert_eq!((route.mix_rate,route.channels),(physical.mix_rate,physical.channels));
        let alias=super::capabilities(super::SYSTEM_DEFAULT_UID).unwrap();
        let actual=super::capabilities(&physical.id).unwrap();
        assert_eq!(alias.sample_rates,actual.sample_rates);
        assert_eq!(alias.bit_depths,actual.bit_depths);
        assert!(super::capabilities("never-a-real-uid").is_err());
    }
    #[test] fn invalid_device_or_level_never_selects_the_default_device() {
        assert!(super::device_volume("").is_err());
        for level in [f64::NAN,-0.1,1.1] {assert!(super::set_device_volume("never-a-real-uid",level).is_err());}
        assert!(super::set_device_volume("never-a-real-uid",0.5).is_err());
    }
}

#[cfg(test)] mod format_tests {
    use super::*;
    fn fixture()->Capabilities{Capabilities{formats:vec![PhysicalFormat{min_rate:44100.,max_rate:44100.,bits:24,channels:2,floating:0},PhysicalFormat{min_rate:48000.,max_rate:48000.,bits:24,channels:2,floating:0},PhysicalFormat{min_rate:96000.,max_rate:96000.,bits:24,channels:2,floating:0}],sample_rates:vec![44100,48000,96000],bit_depths:vec![24],rate_writable:true,exclusive:false,dsd:vec![],conversion_locations:vec![]}}
    #[test]fn source_clock_wins_and_ceiling_is_strict(){let caps=fixture();assert_eq!(caps.rate(44100,96000,2).unwrap(),44100);assert_eq!(caps.rate(192000,48000,2).unwrap(),48000);assert_eq!(caps.rate(88200,96000,2).unwrap(),96000);assert!(caps.rate(44100,32000,2).is_err());assert!(caps.rate(44100,48000,6).is_err());}
    #[test]fn multichannel_source_adapts_to_stereo_device_without_raising_rate_ceiling(){
        let request=PlaybackRequest{flac_samples:None,dsd_pcm:None,input:"never-opened".into(),position:0.,sample_rate:44100,channels:4,source_integer_lossless:false,source_bit_depth:0,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(1.),transitions:Default::default()};
        let mut config=OutputConfig::default();config.limits.max_sample_rate=Some(48000);config.limits.max_bit_depth=Some(24);
        let target=fixture().pcm_format(&request,&config).unwrap();
        assert_eq!((target.sample_rate,target.channels),(44100,2));
        let target=fixture().pcm_format(&PlaybackRequest{sample_rate:96000,..request.clone()},&config).unwrap();
        assert_eq!((target.sample_rate,target.channels),(48000,2));
        config.limits.max_sample_rate=Some(32000);assert!(fixture().pcm_format(&request,&config).is_err());
    }
    #[test]fn real_capability_inventory_is_read_only(){let devices=CoreAudio.devices().unwrap();for device in devices{let caps=capabilities(&device.id).unwrap();assert!(caps.dsd.is_empty());assert!(caps.formats.len()<=512);}}
}
