//! Driver counter samples. Publication never blocks or allocates on an audio callback.
use super::PlaybackControl;
use serde::{Deserialize,Serialize};
use std::sync::{atomic::{AtomicU64,Ordering},Arc};
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
#[repr(u64)]
pub enum Source {WasapiTicks=1,AlsaFrames,AlsaNativeWords,AsioFrames,AsioDsdBits,AndroidFrames,CoreAudioFrames}
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="snake_case")]
#[repr(u64)]
pub enum Domain {Qpc=1,Monotonic,DriverOpaque,MachAbsolute}
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub struct Programme {pub frames:u64,pub sample_rate:u32}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Sample {
    pub version:u32,pub epoch:u64,pub source:Source,pub timestamp_domain:Domain,pub counter:u64,pub nominal_frequency_hz:u64,
    pub device_time_ns:u64,pub host_read_before_ns:u64,pub host_read_after_ns:u64,pub running:bool,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub programme:Option<Programme>,
}
#[cfg(windows)]
#[link(name="kernel32")]
unsafe extern "system" {fn QueryPerformanceCounter(value:*mut i64)->i32;fn QueryPerformanceFrequency(value:*mut i64)->i32;}
#[cfg(all(any(target_os="linux",target_os="android"),target_pointer_width="64"))]
#[repr(C)]struct Timespec {sec:i64,nsec:i64}
#[cfg(all(any(target_os="linux",target_os="android"),target_pointer_width="64"))]
unsafe extern "C" {fn clock_gettime(id:i32,time:*mut Timespec)->i32;}
pub fn host_basis()->&'static str {
    if cfg!(target_os="macos"){"mach_absolute"}else if cfg!(windows){"qpc"}else if cfg!(all(any(target_os="linux",target_os="android"),target_pointer_width="64")){"monotonic"}else{"relative"}
}
#[cfg(target_os="macos")]
#[link(name="CoreAudio",kind="framework")]
unsafe extern "C" {fn AudioGetCurrentHostTime()->u64;fn AudioConvertHostTimeToNanos(time:u64)->u64;}
pub fn host_ns()->Option<u64> {
    #[cfg(target_os="macos")] {return Some(unsafe{AudioConvertHostTimeToNanos(AudioGetCurrentHostTime())});}
    #[cfg(windows)] {
        static FREQUENCY:std::sync::OnceLock<Option<u64>>=std::sync::OnceLock::new();
        let frequency=(*FREQUENCY.get_or_init(||{let mut n=0;if unsafe{QueryPerformanceFrequency(&mut n)}!=0&&n>0{Some(n as u64)}else{None}}))?;
        let mut n=0;if unsafe{QueryPerformanceCounter(&mut n)}==0||n<0{return None;}
        return u64::try_from(n as u128*1_000_000_000/frequency as u128).ok();
    }
    #[cfg(all(any(target_os="linux",target_os="android"),target_pointer_width="64"))] {
        let mut time=Timespec{sec:0,nsec:0};if unsafe{clock_gettime(1,&mut time)}!=0||time.sec<0||!(0..1_000_000_000).contains(&time.nsec){return None;}
        return u64::try_from(time.sec as u128*1_000_000_000+time.nsec as u128).ok();
    }
    #[cfg(not(any(windows,target_os="macos",all(any(target_os="linux",target_os="android"),target_pointer_width="64"))))] {
        static START:std::sync::OnceLock<std::time::Instant>=std::sync::OnceLock::new();Some(START.get_or_init(std::time::Instant::now).elapsed().as_nanos().min(u64::MAX as u128) as u64)
    }
}
#[derive(Default)]
pub struct Feedback {sequence:AtomicU64,current:AtomicU64,epoch:AtomicU64,meta:AtomicU64,counter:AtomicU64,frequency:AtomicU64,stamp:AtomicU64,before:AtomicU64,after:AtomicU64,programme_frames:AtomicU64,programme_rate:AtomicU64}
impl Feedback {
    /// A stalled decoder must not keep presenting an old callback as a live clock.
    pub fn fresh_sample(&self,now:u64)->Option<Sample>{self.sample().filter(|s|now.checked_sub(s.host_read_after_ns).is_some_and(|age|age<=500_000_000)&&s.host_read_before_ns<=s.host_read_after_ns)}
    fn publish(&self,s:&Sample) {
        if self.current.load(Ordering::SeqCst)!=s.epoch{return;}
        let version=self.sequence.load(Ordering::SeqCst);if version%2!=0||self.sequence.compare_exchange(version,version.wrapping_add(1),Ordering::SeqCst,Ordering::SeqCst).is_err(){return;}
        self.epoch.store(s.epoch,Ordering::SeqCst);self.meta.store(s.source as u64|(s.timestamp_domain as u64)<<8|u64::from(s.running)<<16,Ordering::SeqCst);
        self.counter.store(s.counter,Ordering::SeqCst);self.frequency.store(s.nominal_frequency_hz,Ordering::SeqCst);self.stamp.store(s.device_time_ns,Ordering::SeqCst);self.before.store(s.host_read_before_ns,Ordering::SeqCst);self.after.store(s.host_read_after_ns,Ordering::SeqCst);
        self.programme_frames.store(s.programme.map_or(0,|p|p.frames),Ordering::SeqCst);self.programme_rate.store(s.programme.map_or(0,|p|p.sample_rate as u64),Ordering::SeqCst);
        self.sequence.store(version.wrapping_add(2),Ordering::SeqCst);
    }
    pub fn invalidate(&self,epoch:u64) {let _=self.epoch.compare_exchange(epoch,0,Ordering::SeqCst,Ordering::SeqCst);}
    pub fn sample(&self)->Option<Sample> {
        for _ in 0..3 {
            let current=self.current.load(Ordering::SeqCst);if current==0{return None;}
            let version=self.sequence.load(Ordering::SeqCst);if version%2!=0{continue;}
            let epoch=self.epoch.load(Ordering::SeqCst);let meta=self.meta.load(Ordering::SeqCst);
            let source=match meta&255{1=>Source::WasapiTicks,2=>Source::AlsaFrames,3=>Source::AlsaNativeWords,4=>Source::AsioFrames,5=>Source::AsioDsdBits,6=>Source::AndroidFrames,7=>Source::CoreAudioFrames,_=>return None};
            let domain=match (meta>>8)&255{1=>Domain::Qpc,2=>Domain::Monotonic,3=>Domain::DriverOpaque,4=>Domain::MachAbsolute,_=>return None};
            let sample=Sample{version:1,epoch,source,timestamp_domain:domain,counter:self.counter.load(Ordering::SeqCst),nominal_frequency_hz:self.frequency.load(Ordering::SeqCst),device_time_ns:self.stamp.load(Ordering::SeqCst),host_read_before_ns:self.before.load(Ordering::SeqCst),host_read_after_ns:self.after.load(Ordering::SeqCst),running:meta&(1<<16)!=0,programme:{let rate=self.programme_rate.load(Ordering::SeqCst) as u32;(rate>0).then(||Programme{frames:self.programme_frames.load(Ordering::SeqCst),sample_rate:rate})}};
            if self.sequence.load(Ordering::SeqCst)==version&&epoch==current&&self.current.load(Ordering::SeqCst)==current{return Some(sample);}
        }None
    }
}
pub struct Session {control:Arc<PlaybackControl>,epoch:u64}
impl Session {
    pub fn new(control:Arc<PlaybackControl>)->Self {let _=host_ns();let mut s=Self{control,epoch:0};s.restart();s}
    pub fn epoch(&self)->u64 {self.epoch}
    pub fn restart(&mut self) {static NEXT:AtomicU64=AtomicU64::new(1);self.epoch=NEXT.fetch_add(1,Ordering::Relaxed);self.control.device_clock.current.store(self.epoch,Ordering::SeqCst);}
    pub fn record(&self,source:Source,counter:u64,frequency:u64,stamp:u64,domain:Domain,before:Option<u64>,after:Option<u64>,running:bool) {
        Self::record_epoch(&self.control,self.epoch,source,counter,frequency,stamp,domain,before,after,running);
    }
    pub fn record_programme(&self,source:Source,counter:u64,frequency:u64,stamp:u64,domain:Domain,before:Option<u64>,after:Option<u64>,running:bool,programme:Option<Programme>) {
        Self::record_programme_epoch(&self.control,self.epoch,source,counter,frequency,stamp,domain,before,after,running,programme);
    }
    pub fn record_epoch(control:&PlaybackControl,epoch:u64,source:Source,counter:u64,frequency:u64,stamp:u64,domain:Domain,before:Option<u64>,after:Option<u64>,running:bool) {
        Self::record_programme_epoch(control,epoch,source,counter,frequency,stamp,domain,before,after,running,None);
    }
    pub fn record_programme_epoch(control:&PlaybackControl,epoch:u64,source:Source,counter:u64,frequency:u64,stamp:u64,domain:Domain,before:Option<u64>,after:Option<u64>,running:bool,programme:Option<Programme>) {
        let Some((before,after))=before.zip(after).filter(|(a,b)|a<=b&&frequency>0) else{control.device_clock.invalidate(epoch);return;};
        control.device_clock.publish(&Sample{version:1,epoch,source,timestamp_domain:domain,counter,nominal_frequency_hz:frequency,device_time_ns:stamp,host_read_before_ns:before,host_read_after_ns:after,running,programme});
    }
}
impl Drop for Session {fn drop(&mut self){let _=self.control.device_clock.current.compare_exchange(self.epoch,0,Ordering::SeqCst,Ordering::SeqCst);}}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coreaudio_clock_keeps_its_time_domain_and_frame_pair() {
        let control=Arc::new(PlaybackControl::default());let session=Session::new(control.clone());
        session.record_programme(Source::CoreAudioFrames,96000,48000,2_000_000_000,Domain::MachAbsolute,
            Some(2_000_000_001),Some(2_000_000_002),true,Some(Programme{frames:48000,sample_rate:48000}));
        let sample=control.device_clock.fresh_sample(2_100_000_000).unwrap();
        assert_eq!(sample.source,Source::CoreAudioFrames);assert_eq!(sample.timestamp_domain,Domain::MachAbsolute);
        assert_eq!(sample.programme.unwrap().frames,48000);
        assert!(control.device_clock.fresh_sample(2_600_000_003).is_none());
    }
    #[test]
    fn bridge_clock_feedback_expires_without_retimestamping_the_callback(){
        let control=Arc::new(PlaybackControl::default());let session=Session::new(control.clone());
        session.record(Source::WasapiTicks,4800,48000,1_000_000_000,Domain::Qpc,Some(1_000_000_001),Some(1_000_000_002),true);
        assert!(control.device_clock.fresh_sample(1_100_000_000).is_some());
        assert!(control.device_clock.fresh_sample(1_600_000_000).is_none());
        assert!(control.device_clock.fresh_sample(1_000_000_001).is_none());
    }
    #[test]
    fn android_timestamp_keeps_monotonic_domain_and_exact_programme_cursor() {
        let control=Arc::new(PlaybackControl::default());let clock=Session::new(control.clone());
        clock.record_programme(Source::AndroidFrames,48000,48000,1_000_000_000,Domain::Monotonic,Some(1_000_000_010),Some(1_000_000_020),true,Some(Programme{frames:47520,sample_rate:48000}));
        let sample=control.device_clock.sample().unwrap();assert_eq!(sample.source,Source::AndroidFrames);assert_eq!(sample.counter,48000);assert_eq!(sample.programme.unwrap().frames,47520);
        control.device_clock.invalidate(clock.epoch());assert!(control.device_clock.sample().is_none());
    }
    #[test]
    fn device_clock_sessions_reject_retired_writers_and_keep_counter_units() {
        let c=Arc::new(PlaybackControl::default());let old=Session::new(c.clone());old.record(Source::WasapiTicks,4,10_000_000,100,Domain::Qpc,Some(99),Some(101),true);
        assert_eq!(c.device_clock.sample().unwrap().counter,4);let new=Session::new(c.clone());assert!(c.device_clock.sample().is_none());
        old.record(Source::AlsaFrames,999,48000,200,Domain::Monotonic,Some(199),Some(201),true);drop(old);assert!(c.device_clock.sample().is_none());
        new.record(Source::AlsaNativeWords,7,705600,300,Domain::Monotonic,Some(299),Some(301),true);assert_eq!(c.device_clock.sample().unwrap().nominal_frequency_hz,705600);
        new.record(Source::AlsaFrames,0,0,0,Domain::Monotonic,Some(2),Some(1),false);assert!(c.device_clock.sample().is_none());
        new.record(Source::AsioFrames,8,768000,400,Domain::DriverOpaque,Some(399),Some(401),true);assert_eq!(c.device_clock.sample().unwrap().timestamp_domain,Domain::DriverOpaque);drop(new);assert!(c.device_clock.sample().is_none());
    }
    #[test]
    fn programme_clock_snapshot_is_coherent_during_callback_publication() {
        let c=Arc::new(PlaybackControl::default());let session=Session::new(c.clone());let reader=c.clone();
        let worker=std::thread::spawn(move||{for n in 1..20000 {session.record_programme(Source::AsioFrames,n,48000,n*1000,Domain::DriverOpaque,Some(n*1000+1),Some(n*1000+9),true,Some(Programme{frames:n*3,sample_rate:768000}));}std::thread::sleep(std::time::Duration::from_millis(2));});
        while !worker.is_finished(){if let Some(s)=reader.device_clock.sample(){assert_eq!(s.programme,Some(Programme{frames:s.counter*3,sample_rate:768000}));assert_eq!(s.device_time_ns,s.counter*1000);assert_eq!(s.host_read_before_ns,s.device_time_ns+1);assert_eq!(s.host_read_after_ns,s.device_time_ns+9);}}
        worker.join().unwrap();assert!(reader.device_clock.sample().is_none());assert!(host_ns().is_some());
    }
}

#[cfg(test)]
#[test]
fn programme_clock_epoch_and_legacy_samples_never_reuse_a_prior_pair() {
    let control=Arc::new(PlaybackControl::default());let old=Session::new(control.clone());
    old.record_programme(Source::WasapiTicks,1<<60,10_000_000,100,Domain::Qpc,Some(99),Some(101),true,Some(Programme{frames:(1<<60)-1,sample_rate:768000}));
    let sample=control.device_clock.sample().unwrap();assert_eq!(sample.programme.unwrap().frames,(1<<60)-1);
    let mut encoded=serde_json::to_value(&sample).unwrap();encoded.as_object_mut().unwrap().remove("programme");assert!(serde_json::from_value::<Sample>(encoded).unwrap().programme.is_none());
    old.record(Source::AlsaNativeWords,7,705600,200,Domain::Monotonic,Some(199),Some(201),true);assert!(control.device_clock.sample().unwrap().programme.is_none());
    let new=Session::new(control.clone());old.record_programme(Source::AlsaFrames,9,48000,300,Domain::Monotonic,Some(299),Some(301),true,Some(Programme{frames:9,sample_rate:48000}));assert!(control.device_clock.sample().is_none());
    new.record_programme(Source::AlsaFrames,10,48000,400,Domain::Monotonic,Some(399),Some(401),true,Some(Programme{frames:10,sample_rate:48000}));assert_eq!(control.device_clock.sample().unwrap().programme.unwrap().frames,10);drop(old);assert!(control.device_clock.sample().is_some());drop(new);assert!(control.device_clock.sample().is_none());
}
