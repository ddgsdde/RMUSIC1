//! Bounded read-ahead keeps network/decoder stalls off the device clock thread.
use super::{sequence::{Chunk,PcmStream,Read},PlaybackControl,SequenceFormat};
use anyhow::{bail,Result};
use std::sync::{Arc,atomic::{AtomicBool,AtomicU64,Ordering},mpsc::{self,Receiver,TryRecvError,TrySendError}};
use std::time::Instant;
struct Diagnostics {epoch:Instant,reading_since:AtomicU64,max_read:AtomicU64,packets:AtomicU64}
#[derive(Debug)]
pub(super) struct FeedState {pub buffered_packets:u64,pub reading_us:u64,pub max_read_us:u64}
pub(super) struct ReadAhead {receiver:Receiver<Result<Read>>,cancelled:Arc<AtomicBool>,waiting:Arc<AtomicBool>,worker:Option<std::thread::JoinHandle<()>>,control:Arc<PlaybackControl>,current:Option<(Chunk,usize)>,ended:bool,diagnostics:Arc<Diagnostics>}
impl ReadAhead {
 pub fn new(mut source:PcmStream,control:Arc<PlaybackControl>,format:SequenceFormat)->Result<Self>{
  let frames=(format.sample_rate as usize/100).clamp(80,7680);
  Self::spawn(move||source.read(frames),control)
 }
 fn spawn(mut read:impl FnMut()->Result<Read>+Send+'static,control:Arc<PlaybackControl>)->Result<Self>{
  // At most 250 ms plus the producer/current packet; never an entire song in RAM.
  let(sender,receiver)=mpsc::sync_channel(25);let cancelled=Arc::new(AtomicBool::new(false));let cancel=cancelled.clone();let state=control.clone();let waiting=Arc::new(AtomicBool::new(false));let next=waiting.clone();
  let diagnostics=Arc::new(Diagnostics{epoch:Instant::now(),reading_since:AtomicU64::new(0),max_read:AtomicU64::new(0),packets:AtomicU64::new(0)});let timing=diagnostics.clone();
  let worker=std::thread::Builder::new().name("audio-read-ahead".into()).spawn(move||{
   #[cfg(target_os="macos")] if let Ok(backend)=super::external::backend(){backend.prepare_decode_thread();}
   while !cancel.load(Ordering::Acquire)&&!state.stopped.load(Ordering::Acquire){
    let start=timing.epoch.elapsed().as_nanos() as u64;timing.reading_since.store(start,Ordering::Release);
    let mut value=read();timing.reading_since.store(0,Ordering::Release);timing.max_read.fetch_max((timing.epoch.elapsed().as_nanos() as u64).saturating_sub(start),Ordering::Relaxed);next.store(matches!(value,Ok(Read::Pending)),Ordering::Release);if matches!(value,Ok(Read::Pending)){std::thread::sleep(std::time::Duration::from_millis(2));continue;}
    let terminal=!matches!(value,Ok(Read::Data(_)));
    loop{if cancel.load(Ordering::Acquire)||state.stopped.load(Ordering::Acquire){return;}timing.packets.fetch_add(1,Ordering::Relaxed);match sender.try_send(value){Ok(())=>break,Err(TrySendError::Full(saved))=>{timing.packets.fetch_sub(1,Ordering::Relaxed);value=saved;std::thread::sleep(std::time::Duration::from_millis(2));},Err(TrySendError::Disconnected(_))=>{timing.packets.fetch_sub(1,Ordering::Relaxed);return}}}
    if terminal{return;}
   }
  })?;
  Ok(Self{receiver,cancelled,waiting,worker:Some(worker),control,current:None,ended:false,diagnostics})
 }
 pub fn diagnostics(&self)->FeedState {let start=self.diagnostics.reading_since.load(Ordering::Acquire);FeedState{buffered_packets:self.diagnostics.packets.load(Ordering::Relaxed),reading_us:if start==0{0}else{(self.diagnostics.epoch.elapsed().as_nanos() as u64).saturating_sub(start)/1000},max_read_us:self.diagnostics.max_read.load(Ordering::Relaxed)/1000}}
 pub fn waiting_for_next(&self)->bool{self.waiting.load(Ordering::Acquire)}
 pub fn read(&mut self,max_frames:usize)->Result<Read>{
  if max_frames==0{return Ok(Read::Pending);}if self.ended{return Ok(Read::End);}
  if self.current.is_none(){let value=self.receiver.try_recv();if value.is_ok(){self.diagnostics.packets.fetch_sub(1,Ordering::Relaxed);}match value{Ok(Ok(Read::Data(chunk)))=>self.current=Some((chunk,0)),Ok(Ok(Read::End))=>{self.ended=true;return Ok(Read::End);},Ok(Ok(Read::Pending))|Err(TryRecvError::Empty)=>return Ok(Read::Pending),Ok(Err(error))=>return Err(error),Err(TryRecvError::Disconnected)=>{if self.control.stopped.load(Ordering::Acquire){return Ok(Read::End);}bail!("音频预取线程退出，未收到完整结束确认");}}}
  if self.current.as_ref().is_some_and(|(chunk,offset)|*offset==0&&chunk.frames>0&&chunk.frames<=max_frames){
   let (chunk,_)=self.current.take().unwrap();return Ok(Read::Data(chunk));
  }
  let(chunk,offset)=self.current.as_mut().unwrap();if chunk.frames==0||chunk.bytes.len()%chunk.frames!=0{bail!("预取音频帧长度无效");}
  let width=chunk.bytes.len()/chunk.frames;let frames=max_frames.min(chunk.frames-*offset);let bytes=chunk.bytes[*offset*width..(*offset+frames)*width].to_vec();let meter=chunk.meter.clone();*offset+=frames;if *offset==chunk.frames{self.current=None;}Ok(Read::Data(Chunk{bytes,frames,meter}))
 }
}
impl Drop for ReadAhead {fn drop(&mut self){self.cancelled.store(true,Ordering::Release);if let Some(worker)=self.worker.take(){if !worker.is_finished(){if let Ok(decoder)=self.control.decoder.lock(){if let Some(decoder)=decoder.as_ref(){decoder.abort();}}}let _=worker.join();}}}
#[cfg(test)]mod tests{
 use super::*;use crate::dsp::Meter;
 #[test]fn bridge_read_ahead_preserves_every_byte_across_stall_and_uneven_device_packets(){
  let mut n=0;let control=Arc::new(PlaybackControl::default());let mut source=ReadAhead::spawn(move||{n+=1;if n==2{std::thread::sleep(std::time::Duration::from_millis(35));}if n>4{return Ok(Read::End);}Ok(Read::Data(Chunk{bytes:vec![n;32],frames:8,meter:Meter::default()}))},control).unwrap();
  let mut bytes=Vec::new();let mut pending=false;loop{let now=std::time::Instant::now();match source.read(3).unwrap(){Read::Data(v)=>bytes.extend(v.bytes),Read::Pending=>{assert!(now.elapsed()<std::time::Duration::from_millis(20));pending=true;std::thread::sleep(std::time::Duration::from_millis(1));},Read::End=>break,}}
  assert!(pending);assert_eq!(bytes,(1..=4).flat_map(|n|vec![n;32]).collect::<Vec<_>>());
 }
 #[test]fn cancelling_a_full_read_ahead_queue_joins_and_releases_its_source_owner(){
  let control=Arc::new(PlaybackControl::default());let owner=Arc::new(());let weak=Arc::downgrade(&owner);
  let source=ReadAhead::spawn(move||{let _keep_alive=&owner;Ok(Read::Data(Chunk{bytes:vec![7;32],frames:8,meter:Meter::default()}))},control.clone()).unwrap();
  let deadline=Instant::now()+std::time::Duration::from_secs(3);
  while source.diagnostics().buffered_packets<25&&Instant::now()<deadline{std::thread::sleep(std::time::Duration::from_millis(1));}
  assert!(source.diagnostics().buffered_packets>=25,"exercise a producer blocked by the full bounded queue");
  drop(source);assert!(weak.upgrade().is_none(),"the joined producer must release its captured source");
  assert_eq!(Arc::strong_count(&control),1,"no read-ahead worker may survive teardown");
 }
}
