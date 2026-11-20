//! Disk durability work must not block the network/audio delivery task.
use super::MediaObserver;
use std::{sync::{Arc,mpsc},time::{Duration,Instant}};
struct Write {at:u64,total:Option<u64>,bytes:Vec<u8>}
pub fn buffered(observer:MediaObserver)->MediaObserver {
 let(sender,receiver)=mpsc::sync_channel::<Write>(8);
 let target=observer.clone();
 if std::thread::Builder::new().name("rmusic-cache-write".into()).spawn(move||{
  let mut pending:Option<Write>=None;let mut deadline=Instant::now()+Duration::from_millis(500);
  let flush=|pending:&mut Option<Write>|{if let Some(p)=pending.take(){target(p.at,p.total,&p.bytes);}};
  loop {
   match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
    Ok(write)=>{
     if pending.as_ref().is_some_and(|p|p.total!=write.total||p.at+p.bytes.len() as u64!=write.at){flush(&mut pending);}
     if let Some(p)=pending.as_mut(){p.bytes.extend_from_slice(&write.bytes);}else{pending=Some(write);}
     if pending.as_ref().is_some_and(|p|p.bytes.len()>=512*1024)||Instant::now()>=deadline {flush(&mut pending);deadline=Instant::now()+Duration::from_millis(500);}
    },
    Err(mpsc::RecvTimeoutError::Timeout)=>{flush(&mut pending);deadline=Instant::now()+Duration::from_millis(500);},
    Err(mpsc::RecvTimeoutError::Disconnected)=>{flush(&mut pending);break;}
   }
  }
 }).is_err(){return observer;}
 Arc::new(move|at,total,bytes|{
  for(offset,part)in bytes.chunks(256*1024).enumerate(){
   // Saturation skips cache-only bytes. The manifest never claims these gaps;
   // audio delivery continues and a future validated fetch may fill them.
   if sender.try_send(Write{at:at+offset as u64*256*1024,total,bytes:part.to_vec()}).is_err(){break;}
  }
 })
}
#[cfg(test)]mod tests {
 use super::*;
 #[test]fn coalesces_consecutive_ranges_flushes_gaps_and_keeps_audio_nonblocking(){
  let(sender,receiver)=mpsc::channel();let writes=buffered(Arc::new(move|at,total,bytes|{std::thread::sleep(Duration::from_millis(30));sender.send((at,total,bytes.to_vec())).unwrap();}));
  let start=Instant::now();writes(0,Some(20),b"abc");writes(3,Some(20),b"def");writes(12,Some(20),b"xy");assert!(start.elapsed()<Duration::from_millis(20));drop(writes);
  assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(),(0,Some(20),b"abcdef".to_vec()));assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(),(12,Some(20),b"xy".to_vec()));
 }
}
