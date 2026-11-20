//! Two-phase start of a prepared PCM device. Times are in output::clock's host domain.
use super::{clock,PlaybackControl};
use anyhow::{bail,Context,Result};
use serde::Serialize;
use std::sync::{atomic::Ordering,Condvar,Mutex};
const MAX_WAIT:u64=10_000_000_000;
const MAX_LATE:u64=20_000_000;
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize)]
#[serde(rename_all="snake_case")]
pub enum Phase {Immediate,Armed,Ready,Committed,Released,Cancelled,Expired}
#[derive(Clone,Copy,Debug,Serialize)]
pub struct Snapshot {pub phase:Phase,pub expires_host_ns:u64,pub deadline_host_ns:u64,pub ready_host_ns:u64,pub released_host_ns:u64}
struct State {snapshot:Snapshot,sealed:bool}
impl State {
    fn arm(&mut self,expires:u64,now:u64)->Result<()> {
        if self.sealed||self.snapshot.phase!=Phase::Immediate{bail!("计划起播必须在新输出准备前设置");}
        if expires.checked_sub(now).is_none_or(|span|span==0||span>MAX_WAIT){bail!("计划起播有效期必须在未来十秒以内");}
        self.snapshot.phase=Phase::Armed;self.snapshot.expires_host_ns=expires;Ok(())
    }
    fn commit(&mut self,deadline:u64,now:u64)->Result<()> {
        let s=&mut self.snapshot;
        if s.phase==Phase::Committed&&s.deadline_host_ns==deadline&&now<s.expires_host_ns{return Ok(());}
        if s.phase!=Phase::Ready||deadline<=now||deadline>=s.expires_host_ns{bail!("设备尚未就绪或起播时刻无效");}
        s.deadline_host_ns=deadline;s.phase=Phase::Committed;Ok(())
    }
    fn poll(&mut self,now:u64,paused:bool,stopped:bool)->Result<Option<bool>> {
        let s=&mut self.snapshot;
        if s.phase==Phase::Immediate{return Ok(Some(true));}
        if stopped||paused||s.phase==Phase::Cancelled{s.phase=Phase::Cancelled;return Ok(Some(false));}
        if now>=s.expires_host_ns||s.phase==Phase::Expired||(s.phase==Phase::Committed&&now>s.deadline_host_ns.saturating_add(MAX_LATE)){s.phase=Phase::Expired;bail!("计划起播已过期，设备未启动");}
        if s.phase==Phase::Armed{s.ready_host_ns=now;s.phase=Phase::Ready;}
        if s.phase==Phase::Committed&&now>=s.deadline_host_ns{s.released_host_ns=now;s.phase=Phase::Released;return Ok(Some(true));}
        if s.phase==Phase::Released{bail!("计划起播不能重复启动设备");}Ok(None)
    }
}
pub struct Gate {state:Mutex<State>,wake:Condvar}
impl Default for Gate {
    fn default()->Self {Self{state:Mutex::new(State{snapshot:Snapshot{phase:Phase::Immediate,expires_host_ns:0,deadline_host_ns:0,ready_host_ns:0,released_host_ns:0},sealed:false}),wake:Condvar::new()}}
}
impl Gate {
    /// Arm once, before calling play_sequence. No output is started without a later commit.
    pub fn arm(&self,expires_host_ns:u64)->Result<()> {self.state.lock().map_err(|_|anyhow::anyhow!("start lock poisoned"))?.arm(expires_host_ns,clock::host_ns().context("宿主单调时钟不可用")?)}
    /// Commit only after snapshot.phase is Ready. A repeated identical commit is idempotent.
    pub fn commit(&self,deadline_host_ns:u64)->Result<()> {self.state.lock().map_err(|_|anyhow::anyhow!("start lock poisoned"))?.commit(deadline_host_ns,clock::host_ns().context("宿主单调时钟不可用")?)?;self.wake.notify_all();Ok(())}
    pub fn snapshot(&self)->Option<Snapshot> {self.state.lock().ok().map(|s|s.snapshot)}
    pub fn is_scheduled(&self)->bool {self.state.lock().map_or(true,|s|s.snapshot.phase!=Phase::Immediate)}
    pub fn cancel(&self) {if let Ok(mut state)=self.state.lock(){if matches!(state.snapshot.phase,Phase::Armed|Phase::Ready|Phase::Committed){state.snapshot.phase=Phase::Cancelled;}}self.wake.notify_all();}
    pub(super) fn setup(&self,dsd:bool)->Result<bool> {
        let mut state=self.state.lock().map_err(|_|anyhow::anyhow!("start lock poisoned"))?;state.sealed=true;
        let scheduled=state.snapshot.phase!=Phase::Immediate;
        if scheduled&&dsd{bail!("计划起播当前仅支持 PCM，DSD 需单独同步准入");}Ok(scheduled)
    }
    /// A native callback driver may warm up with silence after commit, then release PCM at the deadline.
    /// The gate remains committed until wait() accounts for its actual release; cancellation still wakes it.
    pub(super) fn wait_for_commit(&self,control:&PlaybackControl)->Result<Option<u64>> {
        let mut state=self.state.lock().map_err(|_|anyhow::anyhow!("start lock poisoned"))?;
        loop {
            let now=clock::host_ns().context("宿主单调时钟不可用")?;
            if state.snapshot.phase==Phase::Committed && now<state.snapshot.deadline_host_ns && !control.stopped.load(Ordering::Acquire) && !control.paused.load(Ordering::Acquire) {return Ok(Some(state.snapshot.deadline_host_ns));}
            if let Some(go)=state.poll(now,control.paused.load(Ordering::Acquire),control.stopped.load(Ordering::Acquire))?{if go{bail!("计划起播已错过驱动预热期限");}return Ok(None);}
            state=self.wake.wait_timeout(state,std::time::Duration::from_millis(2)).map_err(|_|anyhow::anyhow!("start lock poisoned"))?.0;
        }
    }
    /// Called on the output worker after the first buffer is prepared, never in an audio callback.
    pub(super) fn wait(&self,control:&PlaybackControl)->Result<bool> {
        let mut state=self.state.lock().map_err(|_|anyhow::anyhow!("start lock poisoned"))?;
        if state.snapshot.phase==Phase::Immediate{return Ok(true);}
        loop {
            let now=clock::host_ns().context("宿主单调时钟不可用")?;
            if let Some(start)=state.poll(now,control.paused.load(Ordering::Acquire),control.stopped.load(Ordering::Acquire))?{if !start{drop(state);control.stop();}return Ok(start);}
            let deadline=if state.snapshot.phase==Phase::Committed{state.snapshot.deadline_host_ns}else{state.snapshot.expires_host_ns};
            let duration=std::time::Duration::from_nanos(deadline.saturating_sub(now).clamp(1,2_000_000));
            state=self.wake.wait_timeout(state,duration).map_err(|_|anyhow::anyhow!("start lock poisoned"))?.0;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state()->State {State{snapshot:Gate::default().snapshot().unwrap(),sealed:false}}
    #[test]
    fn scheduled_start_requires_ready_and_releases_once_at_the_host_deadline() {
        let mut s=state();s.arm(100_000_000,1).unwrap();assert!(s.commit(50_000_000,2).is_err());
        assert_eq!(s.poll(3,false,false).unwrap(),None);assert_eq!(s.snapshot.ready_host_ns,3);
        s.commit(50_000_000,4).unwrap();s.commit(50_000_000,5).unwrap();assert!(s.commit(60_000_000,6).is_err());
        assert_eq!(s.poll(49_999_999,false,false).unwrap(),None);assert_eq!(s.poll(50_000_000,false,false).unwrap(),Some(true));assert_eq!(s.snapshot.released_host_ns,50_000_000);assert!(s.poll(50_000_001,false,false).is_err());
    }
    #[test]
    fn scheduled_start_cancellation_expiry_lateness_and_configuration_are_bounded() {
        for (paused,stopped) in [(true,false),(false,true)] {let mut s=state();s.arm(100,1).unwrap();assert_eq!(s.poll(2,paused,stopped).unwrap(),Some(false));assert_eq!(s.snapshot.phase,Phase::Cancelled);assert!(s.commit(50,3).is_err());}
        let mut s=state();s.arm(100,1).unwrap();assert!(s.poll(100,false,false).is_err());assert_eq!(s.snapshot.phase,Phase::Expired);
        let mut s=state();s.arm(100_000_000,1).unwrap();s.poll(2,false,false).unwrap();s.commit(30_000_000,3).unwrap();assert!(s.poll(50_000_001,false,false).is_err());assert_eq!(s.snapshot.released_host_ns,0);
        let g=Gate::default();g.setup(false).unwrap();assert!(g.arm(clock::host_ns().unwrap()+1_000_000).is_err());
        let g=Gate::default();g.arm(clock::host_ns().unwrap()+1_000_000_000).unwrap();assert!(g.setup(true).is_err());
        let mut s=state();assert!(s.arm(0,0).is_err());assert!(s.arm(MAX_WAIT+1,0).is_err());
    }
    #[test]
    fn callback_warmup_waits_for_commit_and_keeps_cancellation_effective() {
        let c=std::sync::Arc::new(PlaybackControl::default());
        c.start.arm(clock::host_ns().unwrap()+3_000_000_000).unwrap();c.start.setup(false).unwrap();let worker=c.clone();
        let task=std::thread::spawn(move||worker.start.wait_for_commit(&worker));
        let until=std::time::Instant::now()+std::time::Duration::from_secs(1);
        while c.start.snapshot().unwrap().phase!=Phase::Ready {assert!(std::time::Instant::now()<until);std::thread::yield_now();}
        let deadline=clock::host_ns().unwrap()+500_000_000;c.start.commit(deadline).unwrap();
        assert_eq!(task.join().unwrap().unwrap(),Some(deadline));assert_eq!(c.start.snapshot().unwrap().phase,Phase::Committed);
        c.stop();assert!(!c.start.wait(&c).unwrap());assert_eq!(c.start.snapshot().unwrap().released_host_ns,0);
        let c=PlaybackControl::default();c.start.arm(clock::host_ns().unwrap()+1_000_000_000).unwrap();c.start.setup(false).unwrap();c.stop();assert_eq!(c.start.wait_for_commit(&c).unwrap(),None);
    }
    #[test]
    fn scheduled_start_stop_wakes_a_prepared_worker_without_a_commit() {
        let c=std::sync::Arc::new(PlaybackControl::default());c.start.arm(clock::host_ns().unwrap()+5_000_000_000).unwrap();c.start.setup(false).unwrap();let worker=c.clone();
        let task=std::thread::spawn(move||worker.start.wait(&worker));let until=std::time::Instant::now()+std::time::Duration::from_secs(1);
        while c.start.snapshot().unwrap().phase!=Phase::Ready {assert!(std::time::Instant::now()<until);std::thread::yield_now();}
        c.stop();assert!(!task.join().unwrap().unwrap());assert_eq!(c.start.snapshot().unwrap().phase,Phase::Cancelled);
    }
}
