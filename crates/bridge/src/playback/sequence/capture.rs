//! Keep persistence and report work off the output deadline. Progress can coalesce;
//! completed-track boundaries cannot. Closing drains the final sample and joins.
use super::PlaybackUpdate;
use anyhow::{anyhow,Result};
use std::{sync::mpsc::{sync_channel,SyncSender,TrySendError},thread::{self,JoinHandle}};

const MAX_COMPLETIONS:usize=256;
pub(super) struct Relay<S> {
    sender:Option<SyncSender<PlaybackUpdate>>,
    worker:Option<JoinHandle<S>>,
    pending:Option<PlaybackUpdate>,
}
impl<S:Send+'static> Relay<S> {
    pub fn new(mut state:S,mut capture:impl FnMut(&mut S,PlaybackUpdate)+Send+'static)->Self {
        let (sender,receiver)=sync_channel(2);
        let worker=thread::Builder::new().name("rmusic-capture".into()).spawn(move||{while let Ok(update)=receiver.recv(){capture(&mut state,update);}state}).expect("start capture worker");
        Self{sender:Some(sender),worker:Some(worker),pending:None}
    }
    pub fn push(&mut self,mut update:PlaybackUpdate)->Result<()> {
        if let Some(mut previous)=self.pending.take(){
            previous.ended.append(&mut update.ended);update.ended=previous.ended;
            update.continuity_lost|=previous.continuity_lost;
        }
        if update.ended.len()>MAX_COMPLETIONS {return Err(anyhow!("播放记录积压，已停止输出以保留已确认进度"));}
        match self.sender.as_ref().ok_or_else(||anyhow!("播放记录线程已关闭"))?.try_send(update){
            Ok(())=>Ok(()),
            Err(TrySendError::Full(update))=>{self.pending=Some(update);Ok(())},
            Err(TrySendError::Disconnected(_))=>Err(anyhow!("播放记录线程意外退出")),
        }
    }
    // Only after the output driver has stopped: blocking final flush is safe here.
    pub fn finish(mut self)->Result<S> {
        if let Some(update)=self.pending.take(){if let Some(sender)=&self.sender{let _=sender.send(update);}}
        self.sender.take();
        self.worker.take().expect("capture worker exists").join().map_err(|_|anyhow!("播放记录线程意外退出"))
    }
}
impl<S> Drop for Relay<S> {
    fn drop(&mut self){self.sender.take();if let Some(worker)=self.worker.take(){let _=worker.join();}}
}

#[cfg(test)] mod tests {
    use super::*;
    use rmusic_audio::output::{CompletedTrack,SignalPath};
    use std::{sync::{Arc,Mutex,mpsc},time::Duration};
    fn sample(n:u64)->PlaybackUpdate {PlaybackUpdate{playback_id:"current".into(),position:n as f64,output_frames:n,ended:vec![CompletedTrack{playback_id:format!("end-{n}"),position:n as f64,frames:n}],signal_path:SignalPath{output:"capture fixture".into(),sample_rate:48000,channels:2,exclusive:false,bit_depth:24,sample_type:"integer".into(),mixer_sample_rate:None,bit_perfect_eligible:false,processing:vec![]},meter:Default::default(),continuity_lost:false}}
    #[test] fn blocked_persistence_does_not_block_feed_and_keeps_every_boundary_and_final_position(){
        let (started,ready)=mpsc::channel();let (release,blocked)=mpsc::channel();
        let mut relay=Relay::new(Vec::<PlaybackUpdate>::new(),move|seen,update|{if seen.is_empty(){started.send(()).unwrap();let _=blocked.recv_timeout(Duration::from_secs(3));}seen.push(update);});
        relay.push(sample(0)).unwrap();ready.recv_timeout(Duration::from_secs(2)).unwrap();
        // The consumer is deliberately blocked. A blocking send would deadlock this producer.
        for n in 1..100 {relay.push(sample(n)).unwrap();}
        release.send(()).unwrap();let seen=relay.finish().unwrap();
        assert!(seen.len()<=4);assert_eq!(seen.last().unwrap().output_frames,99);
        assert_eq!(seen.iter().flat_map(|u|u.ended.iter().map(|e|e.frames)).collect::<Vec<_>>(),(0..100).collect::<Vec<_>>());
    }
    #[test] fn drop_joins_and_releases_capture_owners(){
        let owner=Arc::new(Mutex::new(0));let held=owner.clone();
        let relay=Relay::new(held,|state,update|{*state.lock().unwrap()+=update.output_frames;});
        drop(relay);assert_eq!(Arc::strong_count(&owner),1);
    }
    #[test] fn worker_failure_is_reported_and_joined(){
        let mut relay=Relay::new((),|_,_|panic!("synthetic persistence failure"));relay.push(sample(0)).unwrap();assert!(relay.finish().is_err());
    }
}
