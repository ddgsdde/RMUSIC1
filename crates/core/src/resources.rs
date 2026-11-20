//! Admission for managed cache buffers and background work, not a process RSS limit.
use crate::state::AppState;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Resources {usage:Mutex<Usage>,jobs:Mutex<std::collections::HashSet<String>>,worker:std::sync::atomic::AtomicBool,limit:std::sync::atomic::AtomicU64}
#[derive(Default)]
struct Usage { bytes: u64, background_bytes: u64, tasks: usize, analyses: usize }
pub struct Running {resources:Arc<Resources>,id:String}
impl Drop for Running{fn drop(&mut self){self.resources.jobs.lock().unwrap().remove(&self.id);}}
pub struct Permit { resources: Arc<Resources>, bytes: u64, background: bool, analysis: bool }
impl Drop for Permit {
    fn drop(&mut self) {
        let mut usage=self.resources.usage.lock().unwrap();
        usage.bytes-=self.bytes;
        if self.background {usage.background_bytes-=self.bytes;usage.tasks-=1;}
        if self.analysis {usage.analyses-=1;}
    }
}
pub async fn limit(s:&AppState)->u64 {let cached=s.resources.limit.load(std::sync::atomic::Ordering::Acquire);if cached!=0{return cached;}let value=s.setting("managed_memory_bytes",json!(1024_u64*1024*1024)).await.as_u64().unwrap_or(1024*1024*1024);s.resources.limit.compare_exchange(0,value,std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire).map_or_else(|current|current,|_|value)}
pub fn reserve(limit:u64)->u64 {(limit/4).min(256*1024*1024)}
impl Resources {
    pub fn set_limit(&self,value:u64){self.limit.store(value,std::sync::atomic::Ordering::Release);}
    pub fn over_budget(&self)->bool{let limit=self.limit.load(std::sync::atomic::Ordering::Acquire);let usage=self.usage.lock().unwrap();limit!=0&&(usage.bytes>limit||usage.background_bytes>limit.saturating_sub(reserve(limit)))}
    pub fn start_worker(&self)->bool{!self.worker.swap(true,std::sync::atomic::Ordering::AcqRel)}
    pub fn job(self:&Arc<Self>,id:&str)->Option<Running>{if self.jobs.lock().unwrap().insert(id.into()){Some(Running{resources:self.clone(),id:id.into()})}else{None}}
    pub fn memory(self:&Arc<Self>,bytes:u64,limit:u64)->Option<Permit> {
        let mut usage=self.usage.lock().unwrap();
        if bytes>limit.saturating_sub(usage.bytes){return None;}
        usage.bytes+=bytes;Some(Permit{resources:self.clone(),bytes,background:false,analysis:false})
    }
    pub async fn background(self:&Arc<Self>,s:&AppState,bytes:u64,analysis:bool)->Result<Permit,&'static str> {
        let limit=limit(s).await;
        let cpus=std::thread::available_parallelism().map_or(1,usize::from);
        let slots=s.setting("background_concurrency",json!(2)).await.as_u64().unwrap_or(2) as usize;
        let mut usage=self.usage.lock().unwrap();
        if usage.tasks>=slots.min(if cpus==1{1}else{16})||analysis&&usage.analyses>=1{return Err("等待后台并发名额");}
        let available=limit.saturating_sub(reserve(limit));
        if bytes>available.saturating_sub(usage.background_bytes)||bytes>limit.saturating_sub(usage.bytes){return Err("等待受控内存预算");}
        usage.bytes+=bytes;usage.background_bytes+=bytes;usage.tasks+=1;if analysis{usage.analyses+=1;}
        Ok(Permit{resources:self.clone(),bytes,background:true,analysis})
    }
    pub async fn status(&self,s:&AppState)->Value {
        let limit=limit(s).await;let usage=self.usage.lock().unwrap();
        json!({"budget_bytes":limit,"used_bytes":usage.bytes,"background_bytes":usage.background_bytes,"playback_reserved_bytes":reserve(limit),"active_tasks":usage.tasks,"active_analyses":usage.analyses,"process_rss_bytes":process_rss()})
    }
}
#[cfg(windows)]
fn process_rss()->Option<u64> {
    #[repr(C)] struct Counters {size:u32,faults:u32,peak:usize,working:usize,rest:[usize;6]}
    #[link(name="psapi")] unsafe extern "system" {fn GetProcessMemoryInfo(process:isize,counters:*mut Counters,size:u32)->i32;}
    let mut value=Counters{size:std::mem::size_of::<Counters>() as u32,faults:0,peak:0,working:0,rest:[0;6]};
    unsafe{if GetProcessMemoryInfo(-1,&mut value,std::mem::size_of::<Counters>() as u32)!=0{Some(value.working as u64)}else{None}}
}
#[cfg(target_os="linux")]
fn process_rss()->Option<u64> {
    use std::io::Read;let mut bytes=String::new();std::fs::File::open("/proc/self/status").ok()?.take(16384).read_to_string(&mut bytes).ok()?;
    bytes.lines().find_map(|line|line.strip_prefix("VmRSS:").and_then(|value|value.split_whitespace().next()?.parse::<u64>().ok()?.checked_mul(1024)))
}
#[cfg(target_os="macos")]
fn process_rss()->Option<u64> {
    // proc_taskinfo layout and flavor from the installed macOS SDK sys/proc_info.h.
    #[repr(C)]struct TaskInfo {sizes:[u64;6],counters:[i32;12]}
    #[link(name="proc")]unsafe extern "C" {fn proc_pidinfo(pid:i32,flavor:i32,arg:u64,buffer:*mut std::ffi::c_void,size:i32)->i32;}
    let mut info=TaskInfo{sizes:[0;6],counters:[0;12]};let size=std::mem::size_of::<TaskInfo>() as i32;
    let result=unsafe{proc_pidinfo(std::process::id() as i32,4,0,(&mut info as *mut TaskInfo).cast(),size)};
    (result==size).then_some(info.sizes[1])
}
#[cfg(not(any(windows,target_os="linux",target_os="macos")))]
fn process_rss()->Option<u64> {None}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn allocated_capacity_is_released_and_playback_reserve_is_bounded(){let resources=Arc::new(Resources::default());let a=resources.memory(512,1024).unwrap();assert!(resources.memory(513,1024).is_none());drop(a);assert!(resources.memory(1024,1024).is_some());assert_eq!(reserve(1024*1024*1024),256*1024*1024);}
}
