//! logind notifications on a private connection; never run D-Bus on the audio thread.
use super::*;
use dbus::{arg::OwnedFd,blocking::{Connection,stdintf::org_freedesktop_dbus::Properties},message::MatchRule};
use std::time::{Duration,Instant};

const SERVICE:&str="org.freedesktop.login1";
const PATH:&str="/org/freedesktop/login1";
const INTERFACE:&str="org.freedesktop.login1.Manager";
static AVAILABLE:AtomicBool=AtomicBool::new(false);
pub(super) fn available()->bool {AVAILABLE.load(Ordering::Acquire)}
enum Notice {Sleep(bool),OwnerChanged}

fn inhibit(connection:&Connection,owner:&str)->Result<OwnedFd> {
    let (fd,):(OwnedFd,)=connection.with_proxy(owner,PATH,Duration::from_secs(2)).method_call(INTERFACE,"Inhibit",("sleep","R-Music","保存播放检查点","delay"))?;Ok(fd)
}
fn quiesce(registry:&Registry,limit:Duration) {
    let deadline=Instant::now()+limit.saturating_sub(Duration::from_millis(100));
    while Instant::now()<deadline {
        let pending=registry.controls.lock().map(|controls|controls.iter().filter_map(Weak::upgrade).any(|control|control.sleep_checkpoint_pending.load(Ordering::Acquire))).unwrap_or(true);
        if !pending {break;}
        std::thread::sleep(Duration::from_millis(5));
    }
    // The adapter's writer drains final cursor/listening commands before saving this state.
    let _=registry.cancel.send(Work::State(registry.gate.generation.load(Ordering::Acquire),registry.gate.held.load(Ordering::Acquire)));
    if let Err(error)=registry.flush_for(deadline.saturating_duration_since(Instant::now())) {eprintln!("系统挂起检查点尚未完成：{error:#}");}
}
fn monitor(registry:&Registry,initial:&mut Option<mpsc::Sender<()>>,connected:&mut bool,warned:&mut bool)->Result<()> {
    let connection=Connection::new_system()?;
    let (owner,):(String,)=connection.with_proxy("org.freedesktop.DBus","/org/freedesktop/DBus",Duration::from_secs(2)).method_call("org.freedesktop.DBus","GetNameOwner",(SERVICE,))?;
    let (send,receive)=mpsc::channel();let sleeping=send.clone();
    connection.add_match(MatchRule::new_signal(INTERFACE,"PrepareForSleep").with_path(PATH).with_strict_sender(owner.clone()),move |(value,):(bool,),_,_|{let _=sleeping.send(Notice::Sleep(value));true})?;
    connection.add_match(MatchRule::new_signal("org.freedesktop.DBus","NameOwnerChanged").with_strict_sender("org.freedesktop.DBus"),move |(name,_,_):(String,String,String),_,_|{if name==SERVICE{let _=send.send(Notice::OwnerChanged);}true})?;
    *connected=true;
    let proxy=connection.with_proxy(owner.as_str(),PATH,Duration::from_secs(2));
    let max_delay:u64=proxy.get(INTERFACE,"InhibitDelayMaxUSec")?;
    let mut lock=match inhibit(&connection,&owner){Ok(fd)=>Some(fd),Err(error)=>{eprintln!("logind 延时锁不可用，无法保证挂起前保存：{error:#}");None}};
    AVAILABLE.store(lock.is_some(),Ordering::Release);
    let mut preparing:bool=proxy.get(INTERFACE,"PreparingForSleep")?;
    if preparing {registry.event(4);quiesce(registry,if lock.is_some(){Duration::from_micros(max_delay)}else{Duration::ZERO});lock.take();AVAILABLE.store(false,Ordering::Release);}
    else {registry.event(18);}
    if *warned {eprintln!("logind 挂起监听已恢复；已有播放等待状态保留");}*warned=false;
    if let Some(initial)=initial.take(){let _=initial.send(());}
    loop {
        connection.process(Duration::from_millis(100))?;
        while let Ok(notice)=receive.try_recv(){match notice {
            Notice::OwnerChanged=>anyhow::bail!("logind 服务所有者已变化"),
            Notice::Sleep(true)=>{
                if preparing{continue;}preparing=true;
                registry.event(4);
                quiesce(registry,if lock.is_some(){Duration::from_micros(max_delay)}else{Duration::ZERO});
                lock.take();AVAILABLE.store(false,Ordering::Release);
            },
            Notice::Sleep(false)=>{
                if !preparing{continue;}preparing=false;
                lock=match inhibit(&connection,&owner){Ok(fd)=>Some(fd),Err(error)=>{eprintln!("logind 唤醒后延时锁不可用：{error:#}");None}};
                registry.event(18);AVAILABLE.store(lock.is_some(),Ordering::Release);
            }
        }}
    }
}
pub(super) fn register()->std::result::Result<usize,String> {
    let (initial,ready)=mpsc::channel();
    std::thread::Builder::new().name("audio-logind".into()).spawn(move||{
        let Some(Ok(registry))=REGISTRY.get() else{return};let mut initial=Some(initial);let mut delay=1;let mut warned=false;
        loop {
            let mut connected=false;
            if let Err(error)=monitor(registry,&mut initial,&mut connected,&mut warned) {
                AVAILABLE.store(false,Ordering::Release);
                if connected {registry.event(4);let _=registry.flush();}
                if !warned {if connected{eprintln!("logind 挂起监听中断，输出已暂停；连接恢复后请手动播放：{error:#}");}else{eprintln!("logind 不可用，缺少系统挂起保护；普通播放仍可使用：{error:#}");}warned=true;}
                if let Some(initial)=initial.take(){let _=initial.send(());}
            }
            std::thread::sleep(Duration::from_secs(delay));delay=(delay*2).min(30);
        }
    }).map_err(|e|e.to_string())?;
    let _=ready.recv_timeout(Duration::from_secs(3));Ok(1)
}
