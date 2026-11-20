//! Process-lifetime sleep protection. Platform callbacks never borrow an output worker.
use super::{DeviceUnavailable,PlaybackControl};
use anyhow::Result;
use std::{sync::{atomic::{AtomicBool,Ordering},mpsc,Arc,Mutex,OnceLock,Weak}};

use std::sync::atomic::AtomicU64;
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize,PartialEq,Eq)]
pub struct Stamp {pub host_id:String,pub generation:u64}
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub struct GroupStamp {pub host_id:String,pub generation:u64,pub wake_generation:u64}
type Persist=Arc<dyn Fn(u64,bool)->Result<()>+Send+Sync>;
struct Gate {transition:Mutex<()>,generation:AtomicU64,wake_generation:AtomicU64,saved:AtomicU64,held:AtomicBool,suspended:AtomicBool,host:Mutex<String>,persist:Mutex<Option<Persist>>,error:Mutex<Option<String>>}
enum Work {Cancel(Arc<PlaybackControl>),State(u64,bool),Acknowledge(u64,mpsc::Sender<std::result::Result<(),String>>),Barrier(mpsc::Sender<std::result::Result<(),String>>)}
struct Registry {controls:Mutex<Vec<Weak<PlaybackControl>>>,gate:Arc<Gate>,cancel:mpsc::Sender<Work>}
fn persist(gate:&Gate,generation:u64,held:bool)->std::result::Result<(),String> {
    let writer=gate.persist.lock().map_err(|_|"电源记录锁不可用".to_string())?.clone();
    let result=writer.map_or(Ok(()),|write|write(generation,held).map_err(|e|e.to_string()));
    if result.is_ok(){gate.saved.fetch_max(generation,Ordering::AcqRel);}
    if let Ok(mut error)=gate.error.lock(){*error=result.as_ref().err().cloned();}
    result
}
impl Registry {
    fn new()->std::result::Result<Self,String> {
        let (cancel,receiver)=mpsc::channel();
        let gate=Arc::new(Gate{transition:Mutex::new(()),generation:AtomicU64::new(0),wake_generation:AtomicU64::new(0),saved:AtomicU64::new(0),held:AtomicBool::new(false),suspended:AtomicBool::new(false),host:Mutex::new(String::new()),persist:Mutex::new(None),error:Mutex::new(None)});
        let worker=gate.clone();
        std::thread::Builder::new().name("audio-power-cancel".into()).spawn(move||{while let Ok(work)=receiver.recv(){match work {
            Work::Cancel(control)=>control.stop(),
            Work::State(generation,held)=>{if worker.generation.load(Ordering::Acquire)==generation&&worker.held.load(Ordering::Acquire)==held{let _=persist(&worker,generation,held);}},
            Work::Barrier(reply)=>{let _=reply.send(worker.error.lock().map_err(|_|"电源记录锁不可用".to_string()).and_then(|e|e.clone().map_or(Ok(()),Err)));},
            Work::Acknowledge(generation,reply)=>{
                let mut result=if worker.generation.load(Ordering::Acquire)!=generation||worker.suspended.load(Ordering::Acquire){Err("挂起状态已变化或尚未唤醒，请再次播放".into())}else{persist(&worker,generation,false)};
                if result.is_ok(){
                    let released={let _guard=worker.transition.lock().unwrap_or_else(|e|e.into_inner());if worker.generation.load(Ordering::Acquire)==generation&&!worker.suspended.load(Ordering::Acquire){worker.held.store(false,Ordering::Release);true}else{worker.held.store(true,Ordering::Release);false}};
                    if !released{let _=persist(&worker,worker.generation.load(Ordering::Acquire),true);result=Err("播放确认期间再次挂起".into());}
                }
                let _=reply.send(result);
            }
        }}}).map_err(|e|e.to_string())?;
        Ok(Self{controls:Mutex::new(Vec::new()),gate,cancel})
    }
    fn configure(&self,host:String,generation:u64,held:bool,writer:Persist)->Result<()> {
        *self.gate.host.lock().map_err(|_|DeviceUnavailable("电源记录锁不可用".into()))?=host;
        *self.gate.persist.lock().map_err(|_|DeviceUnavailable("电源记录锁不可用".into()))?=Some(writer);
        self.gate.generation.store(generation,Ordering::Release);self.gate.saved.store(generation,Ordering::Release);self.gate.held.store(held,Ordering::Release);Ok(())
    }
    fn flush(&self)->Result<()> {self.flush_for(std::time::Duration::from_secs(10))}
    fn flush_for(&self,timeout:std::time::Duration)->Result<()> {let (reply,wait)=mpsc::channel();self.cancel.send(Work::Barrier(reply))?;wait.recv_timeout(timeout)?.map_err(|e|DeviceUnavailable(e).into())}
    fn acknowledge(&self,generation:u64)->Result<()> {let (reply,wait)=mpsc::channel();self.cancel.send(Work::Acknowledge(generation,reply))?;wait.recv_timeout(std::time::Duration::from_secs(10))?.map_err(|e|DeviceUnavailable(e).into())}
    fn check(&self,control:&PlaybackControl)->Result<()> {if control.system_suspended.load(Ordering::Acquire)||self.gate.held.load(Ordering::Acquire)||self.gate.suspended.load(Ordering::Acquire){return Err(DeviceUnavailable("系统挂起后已保留播放位置，等待手动播放确认".into()).into());}Ok(())}
    fn group_stamp(&self)->Result<GroupStamp> {
        let _guard=self.gate.transition.lock().map_err(|_|DeviceUnavailable("电源状态锁不可用".into()))?;
        Ok(GroupStamp{host_id:self.gate.host.lock().map_err(|_|DeviceUnavailable("电源记录锁不可用".into()))?.clone(),generation:self.gate.generation.load(Ordering::Acquire),wake_generation:self.gate.wake_generation.load(Ordering::Acquire)})
    }
    fn admit_group(&self,stamp:Option<&GroupStamp>,control:&PlaybackControl)->Result<()> {
        {
            let _guard=self.gate.transition.lock().map_err(|_|DeviceUnavailable("电源状态锁不可用".into()))?;
            if self.gate.suspended.load(Ordering::Acquire)||control.system_interrupted(){return Err(DeviceUnavailable("系统尚未唤醒或该输出已挂起，请重新手动起播分组".into()).into());}
            if let Some(stamp)=stamp {
                if *self.gate.host.lock().map_err(|_|DeviceUnavailable("电源记录锁不可用".into()))?!=stamp.host_id||self.gate.generation.load(Ordering::Acquire)!=stamp.generation||self.gate.wake_generation.load(Ordering::Acquire)!=stamp.wake_generation {
                    return Err(DeviceUnavailable("分组请求早于当前挂起／唤醒状态，请等待端点校准后重新手动起播".into()).into());
                }
            }else if self.gate.held.load(Ordering::Acquire)||self.gate.wake_generation.load(Ordering::Acquire)>0 {
                return Err(DeviceUnavailable("挂起后分组起播需要匹配开发 Core 的手动授权，等待状态已保留".into()).into());
            }
        }
        if self.gate.held.load(Ordering::Acquire) {
            let stamp=stamp.ok_or_else(||DeviceUnavailable("系统再次挂起，请重新手动起播分组".into()))?;
            self.cancel.send(Work::State(stamp.generation,true))?;
            self.flush()?;self.acknowledge(stamp.generation)?;
        }
        self.check(control)
    }
    fn interrupt(&self,control:&Arc<PlaybackControl>) {
        if control.system_suspended.swap(true,Ordering::AcqRel){return;}
        control.paused.store(true,Ordering::Release);control.stopped.store(true,Ordering::Release);
        // Child cancellation can acquire a process mutex; keep it off the OS callback.
        let _=self.cancel.send(Work::Cancel(control.clone()));
    }
    fn track(&self,control:&Arc<PlaybackControl>)->Result<()> {
        let mut controls=self.controls.lock().map_err(|_|DeviceUnavailable("系统电源状态锁不可用".into()))?;
        controls.retain(|control|control.strong_count()>0);
        let weak=Arc::downgrade(control);
        if !controls.iter().any(|current|current.ptr_eq(&weak)){controls.push(weak);}
        if self.gate.suspended.load(Ordering::Acquire){self.gate.held.store(true,Ordering::Release);let _=self.cancel.send(Work::State(self.gate.generation.load(Ordering::Acquire),true));self.interrupt(control);}
        Ok(())
    }
    fn event(&self,kind:u32) {
        let _guard=self.gate.transition.lock().unwrap_or_else(|e|e.into_inner());
        if kind==4 { // PBT_APMSUSPEND
            self.gate.suspended.store(true,Ordering::Release);
            if let Ok(mut controls)=self.controls.lock(){
                let generation=self.gate.generation.fetch_add(1,Ordering::AcqRel)+1;
                let active=controls.iter().filter_map(Weak::upgrade).any(|control|!control.stopped.load(Ordering::Acquire)&&!control.power_done.load(Ordering::Acquire));
                let held=self.gate.held.load(Ordering::Acquire)||active;self.gate.held.store(held,Ordering::Release);let _=self.cancel.send(Work::State(generation,held));
                controls.retain(|weak|{if let Some(control)=weak.upgrade(){if !control.stopped.load(Ordering::Acquire)&&!control.power_done.load(Ordering::Acquire){self.interrupt(&control);}true}else{false}});
            }
        }else if matches!(kind,7|18) { // Waking alone never removes a durable playback hold.
            if self.gate.suspended.swap(false,Ordering::AcqRel){self.gate.wake_generation.fetch_add(1,Ordering::AcqRel);}
        }
    }
}
static REGISTRY:OnceLock<std::result::Result<Registry,String>>=OnceLock::new();
// Registration and callback data deliberately live until process exit, including in-flight callbacks.
static REGISTRATION:OnceLock<std::result::Result<usize,String>>=OnceLock::new();
#[cfg(windows)] mod windows;
#[cfg(all(target_os="linux",feature="alsa-output"))] mod linux;
fn platform_register()->std::result::Result<usize,String> {
    #[cfg(windows)] {windows::register()}
    #[cfg(all(target_os="linux",feature="alsa-output"))] {linux::register()}
}
pub fn supported()->bool {
    #[cfg(windows)] {REGISTRATION.get().is_some_and(|r|r.is_ok())}
    #[cfg(all(target_os="linux",feature="alsa-output"))] {linux::available()}
}
#[cfg(all(target_os="linux",feature="alsa-output"))]
pub struct Checkpoint(Arc<PlaybackControl>);
#[cfg(all(target_os="linux",feature="alsa-output"))]
impl Drop for Checkpoint {fn drop(&mut self){self.0.sleep_checkpoint_pending.store(false,Ordering::Release);}}
#[cfg(all(target_os="linux",feature="alsa-output"))]
pub fn checkpoint(control:&Arc<PlaybackControl>)->Checkpoint {control.sleep_checkpoint_pending.store(true,Ordering::Release);Checkpoint(control.clone())}
pub fn watch(control:&Arc<PlaybackControl>)->Result<()> {
    let registry=REGISTRY.get_or_init(Registry::new).as_ref().map_err(|e|DeviceUnavailable(format!("无法监听系统挂起：{e}")))?;
    REGISTRATION.get_or_init(platform_register).as_ref().map_err(|e|DeviceUnavailable(format!("无法监听系统挂起：{e}")))?;
    registry.track(control)
}
pub fn configure(host:String,generation:u64,held:bool,writer:impl Fn(u64,bool)->Result<()>+Send+Sync+'static)->Result<()> {
    if REGISTRATION.get().is_some(){return Err(DeviceUnavailable("电源保护必须在播放前初始化".into()).into());}
    let registry=REGISTRY.get_or_init(Registry::new).as_ref().map_err(|e|DeviceUnavailable(e.clone()))?;
    registry.configure(host,generation,held,Arc::new(writer))?;
    // Subscribe before any playback/recovery admission, including while idle.
    watch(&Arc::new(PlaybackControl::default()))
}
pub fn held_stamp()->Result<Option<Stamp>> {
    let Some(Ok(registry))=REGISTRY.get() else{return Ok(None)};
    if !registry.gate.held.load(Ordering::Acquire){return Ok(None);}
    let generation=registry.gate.generation.load(Ordering::Acquire);
    if registry.gate.saved.load(Ordering::Acquire)<generation{registry.cancel.send(Work::State(generation,true))?;}
    registry.flush()?;
    let generation=registry.gate.generation.load(Ordering::Acquire);
    if generation>registry.gate.saved.load(Ordering::Acquire){return Err(DeviceUnavailable("挂起状态正在保存，请稍后重试".into()).into());}
    Ok(Some(Stamp{host_id:registry.gate.host.lock().map_err(|_|DeviceUnavailable("电源记录锁不可用".into()))?.clone(),generation}))
}
/// Captured in an authenticated clock exchange and copied only by an explicit group action.
pub fn group_stamp()->Result<Option<GroupStamp>> {
    let Some(Ok(registry))=REGISTRY.get() else{return Ok(None)};
    let stamp=registry.group_stamp()?;Ok((!stamp.host_id.is_empty()).then_some(stamp))
}
pub fn admit_group(stamp:Option<&GroupStamp>,control:&PlaybackControl)->Result<()> {
    let Some(Ok(registry))=REGISTRY.get() else{return Err(DeviceUnavailable("系统电源保护尚未初始化".into()).into())};
    registry.admit_group(stamp,control)
}
pub fn acknowledge(stamp:&Stamp)->Result<()> {
    let Some(Ok(registry))=REGISTRY.get() else{return Ok(())};
    if *registry.gate.host.lock().map_err(|_|DeviceUnavailable("电源记录锁不可用".into()))?!=stamp.host_id{return Err(DeviceUnavailable("挂起确认的主机不匹配".into()).into());}
    registry.acknowledge(stamp.generation)
}
pub(super) fn check(control:&PlaybackControl)->Result<()> {
    let result=if let Some(Ok(registry))=REGISTRY.get(){registry.check(control)}else if control.system_suspended.load(Ordering::Acquire){Err(DeviceUnavailable("系统已挂起，输出已停止；唤醒后请手动播放".into()).into())}else{Ok(())};
    if result.is_err(){control.power_done.store(true,Ordering::Release);}result
}
pub(super) fn finish(control:&PlaybackControl,result:Result<()>)->Result<()> {control.power_done.store(true,Ordering::Release);if control.system_interrupted(){if let Some(Ok(registry))=REGISTRY.get(){let _=registry.flush();}}check(control)?;result}

#[cfg(test)]
mod tests {
    use super::*;
    const HOST:&str="e70465ba-68d8-444d-b8ed-7bfb90243872";
    #[test]
    fn group_power_old_or_sleeping_intent_cannot_clear_the_wake_hold() {
        let registry=Registry::new().unwrap();registry.configure(HOST.into(),0,false,Arc::new(|_,_|Ok(()))).unwrap();
        let old=Arc::new(PlaybackControl::default());registry.track(&old).unwrap();let before=registry.group_stamp().unwrap();registry.event(4);let sleeping=registry.group_stamp().unwrap();
        let fresh=Arc::new(PlaybackControl::default());assert!(registry.admit_group(Some(&sleeping),&fresh).is_err());registry.event(18);let awake=registry.group_stamp().unwrap();registry.event(7);assert_eq!(registry.group_stamp().unwrap(),awake);
        for stamp in [None,Some(&before),Some(&sleeping)]{assert!(registry.admit_group(stamp,&fresh).is_err());assert!(registry.check(&fresh).is_err());}
        registry.admit_group(Some(&awake),&fresh).unwrap();assert!(registry.check(&fresh).is_ok());assert!(registry.check(&old).is_err());
        assert!(registry.admit_group(Some(&before),&fresh).is_err(),"an ordinary acknowledgement cannot revive an old group intent");
    }
    #[test]
    fn group_power_persisted_hold_requires_current_host_and_a_durable_acknowledgement() {
        let registry=Registry::new().unwrap();let saved=Arc::new(Mutex::new((9,true)));let target=saved.clone();registry.configure(HOST.into(),9,true,Arc::new(move|g,h|{*target.lock().unwrap()=(g,h);Ok(())})).unwrap();
        let control=PlaybackControl::default();let stamp=registry.group_stamp().unwrap();let mut wrong=stamp.clone();wrong.host_id="other-host".into();assert!(registry.admit_group(Some(&wrong),&control).is_err());assert_eq!(*saved.lock().unwrap(),(9,true));
        registry.admit_group(Some(&stamp),&control).unwrap();assert_eq!(*saved.lock().unwrap(),(9,false));
        let failed=Registry::new().unwrap();let failing=Arc::new(AtomicBool::new(true));let writer=failing.clone();failed.configure(HOST.into(),9,true,Arc::new(move|_,_|{if writer.load(Ordering::Acquire){anyhow::bail!("QA storage failure");}Ok(())})).unwrap();assert!(failed.admit_group(Some(&stamp),&control).is_err());assert!(failed.check(&control).is_err());
        failing.store(false,Ordering::Release);failed.admit_group(Some(&stamp),&control).unwrap();assert!(failed.check(&control).is_ok(),"storage recovery must allow a new manual acknowledgement");
    }
    #[test]
    fn group_power_idle_sleep_rejects_pre_sleep_commands_even_without_a_held_output() {
        let registry=Registry::new().unwrap();registry.configure(HOST.into(),0,false,Arc::new(|_,_|Ok(()))).unwrap();let before=registry.group_stamp().unwrap();registry.event(4);registry.event(18);
        let control=PlaybackControl::default();assert!(registry.check(&control).is_ok());assert!(registry.admit_group(Some(&before),&control).is_err());assert!(registry.admit_group(None,&control).is_err());registry.admit_group(Some(&registry.group_stamp().unwrap()),&control).unwrap();
    }
    #[test]
    fn hold_survives_a_registry_restart_and_obsolete_writes_cannot_reinstate_it() {
        let saved=Arc::new(Mutex::new((4,false)));let target=saved.clone();let registry=Registry::new().unwrap();
        registry.configure(HOST.into(),4,false,Arc::new(move|g,h|{*target.lock().unwrap()=(g,h);Ok(())})).unwrap();
        let control=Arc::new(PlaybackControl::default());registry.track(&control).unwrap();registry.event(4);registry.event(18);registry.flush().unwrap();assert_eq!(*saved.lock().unwrap(),(5,true));
        drop(registry);
        let next=Registry::new().unwrap();let target=saved.clone();next.configure(HOST.into(),5,true,Arc::new(move|g,h|{*target.lock().unwrap()=(g,h);Ok(())})).unwrap();
        let pending=Arc::new(PlaybackControl::default());next.track(&pending).unwrap();assert!(next.check(&pending).is_err());assert!(next.acknowledge(4).is_err());
        next.acknowledge(5).unwrap();assert!(next.check(&pending).is_ok());assert_eq!(*saved.lock().unwrap(),(5,false));
        next.cancel.send(Work::State(5,true)).unwrap();next.flush().unwrap();assert_eq!(*saved.lock().unwrap(),(5,false));
    }
    #[test]
    fn a_new_suspend_during_acknowledgement_keeps_memory_and_storage_held() {
        let registry=Registry::new().unwrap();let gate=Arc::downgrade(&registry.gate);let once=Arc::new(AtomicBool::new(true));let trigger=once.clone();let saved=Arc::new(Mutex::new((4,true)));let target=saved.clone();
        registry.configure(HOST.into(),4,true,Arc::new(move|g,h|{*target.lock().unwrap()=(g,h);if !h&&trigger.swap(false,Ordering::AcqRel){let gate=gate.upgrade().unwrap();gate.generation.fetch_add(1,Ordering::AcqRel);gate.held.store(true,Ordering::Release);gate.suspended.store(true,Ordering::Release);}Ok(())})).unwrap();
        assert!(registry.acknowledge(4).is_err());assert_eq!(*saved.lock().unwrap(),(5,true));assert!(registry.check(&PlaybackControl::default()).is_err());
        registry.event(18);registry.acknowledge(5).unwrap();assert_eq!(*saved.lock().unwrap(),(5,false));
    }
    #[test]
    fn failed_durable_acknowledgement_cannot_allow_output() {
        let registry=Registry::new().unwrap();registry.configure(HOST.into(),2,true,Arc::new(|_,_|anyhow::bail!("QA storage failure"))).unwrap();
        assert!(registry.acknowledge(2).is_err());assert!(registry.check(&PlaybackControl::default()).is_err());
    }
    #[test]
    fn idle_or_completed_output_does_not_require_an_extra_play_after_sleep() {
        let registry=Registry::new().unwrap();registry.event(4);registry.event(18);registry.flush().unwrap();assert!(registry.check(&PlaybackControl::default()).is_ok());
        let completed=Arc::new(PlaybackControl::default());completed.power_done.store(true,Ordering::Release);registry.track(&completed).unwrap();registry.event(4);registry.event(18);registry.flush().unwrap();assert!(registry.check(&PlaybackControl::default()).is_ok());
        registry.event(4);let late=Arc::new(PlaybackControl::default());registry.track(&late).unwrap();registry.event(18);registry.flush().unwrap();assert!(registry.check(&late).is_err());
    }
    #[test]
    fn suspend_is_sticky_and_resume_requires_explicit_acknowledgement() {
        let registry=Registry::new().unwrap();let control=Arc::new(PlaybackControl::default());
        control.frames_written.store(48,Ordering::Relaxed);control.frames_played.store(32,Ordering::Relaxed);control.set_volume(0.25).unwrap();control.muted.store(true,Ordering::Relaxed);
        registry.track(&control).unwrap();registry.event(4);
        assert!(control.paused.load(Ordering::Acquire)&&control.stopped.load(Ordering::Acquire));
        assert!(finish(&control,Ok(())).unwrap_err().is::<DeviceUnavailable>());
        assert!(control.system_interrupted());
        let request=super::super::PlaybackRequest{flac_samples: None, dsd_pcm: None, input:"power-qa-source-must-not-open".into(),position:0.,sample_rate:48000,channels:2,source_integer_lossless:true,source_bit_depth:32,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:None,transitions:Default::default()};
        let config=super::super::OutputConfig{device_id:Some("power-qa-device-must-not-open".into()),..Default::default()};
        assert!(super::super::play(request.clone(),config.clone(),control.clone(),|_,_|panic!("unexpected output")).unwrap_err().to_string().contains("挂起"));
        let format=super::super::SequenceFormat{sample_rate:48000,channels:2,channel_mask:3,exclusive:false,float_output:true,bit_depth:32};
        let sequence=super::super::PlaybackSequence::new("power-qa".into(),request.clone(),format).unwrap();
        assert!(super::super::play_sequence(request,config,control.clone(),sequence,|_|panic!("unexpected output")).unwrap_err().to_string().contains("挂起"));
        assert_eq!(control.frames_played.load(Ordering::Relaxed),32);assert_eq!(control.volume_setting(),0.25);assert!(control.muted.load(Ordering::Relaxed));
        let pending=Arc::new(PlaybackControl::default());registry.track(&pending).unwrap();assert!(check(&pending).is_err());
        registry.event(18);registry.event(7);assert!(check(&control).is_err());assert!(check(&pending).is_err());
        let next=Arc::new(PlaybackControl::default());registry.track(&next).unwrap();assert!(registry.check(&next).is_err(),"唤醒和重连不能自动允许新的输出请求");
        registry.acknowledge(1).unwrap();assert!(registry.check(&next).is_ok());assert!(registry.check(&control).is_err());
    }
    #[test]
    fn suspend_interrupts_a_blocked_decoder_without_an_audio_device() {
        use crate::decoder::{FfmpegDecoder,PcmEncoding};
        use std::{net::TcpListener,time::Duration};
        let registry=Registry::new().unwrap();let control=Arc::new(PlaybackControl::default());registry.track(&control).unwrap();
        let server=TcpListener::bind("127.0.0.1:0").unwrap();let address=server.local_addr().unwrap();
        let (accepted,connected)=mpsc::channel();let (release,wait)=mpsc::channel();
        let server_thread=std::thread::spawn(move||{let (socket,_)=server.accept().unwrap();accepted.send(()).unwrap();let _=wait.recv_timeout(Duration::from_secs(10));drop(socket);});
        let mut decoder=FfmpegDecoder::open(&format!("http://{address}/blocked.wav"),0.,48000,2,PcmEncoding::Signed32).unwrap();
        *control.decoder.lock().unwrap()=Some(decoder.abort_handle());
        let (done,finished)=mpsc::channel();let worker=std::thread::spawn(move||{done.send(decoder.read_frames(&mut [0;1024],8).is_err()).unwrap();});
        connected.recv_timeout(Duration::from_secs(5)).unwrap();registry.event(4);
        let interrupted=finished.recv_timeout(Duration::from_secs(1));let _=release.send(());server_thread.join().unwrap();worker.join().unwrap();
        assert_eq!(interrupted.unwrap(),true);assert!(finish(&control,Err(crate::decoder::DecodeFailure("cancelled".into()).into())).unwrap_err().is::<DeviceUnavailable>());
    }
    #[test]
    #[cfg(windows)]
    fn actual_windows_subscription_needs_no_playback_or_system_sleep() {
        let control=Arc::new(PlaybackControl::default());watch(&control).unwrap();assert!(check(&control).is_ok());assert!(REGISTRATION.get().unwrap().as_ref().is_ok_and(|handle|*handle!=0));
    }
    #[test]
    fn stop_before_decoder_installation_still_cancels_that_child() {
        use crate::decoder::{FfmpegDecoder,PcmEncoding};
        use std::{net::TcpListener,time::Duration};
        let control=Arc::new(PlaybackControl::default());control.stop();
        let server=TcpListener::bind("127.0.0.1:0").unwrap();let address=server.local_addr().unwrap();
        let (accepted,connected)=mpsc::channel();let (release,wait)=mpsc::channel();
        let server_thread=std::thread::spawn(move||{let (socket,_)=server.accept().unwrap();accepted.send(()).unwrap();let _=wait.recv_timeout(Duration::from_secs(10));drop(socket);});
        let mut decoder=FfmpegDecoder::open(&format!("http://{address}/late.wav"),0.,48000,2,PcmEncoding::Signed32).unwrap();
        connected.recv_timeout(Duration::from_secs(5)).unwrap();control.attach_decoder(decoder.abort_handle()).unwrap();
        let (done,finished)=mpsc::channel();let worker=std::thread::spawn(move||{done.send(decoder.read_frames(&mut [0;1024],8).is_err()).unwrap();});
        let interrupted=finished.recv_timeout(Duration::from_secs(1));let _=release.send(());server_thread.join().unwrap();worker.join().unwrap();assert_eq!(interrupted.unwrap(),true);
    }
}
