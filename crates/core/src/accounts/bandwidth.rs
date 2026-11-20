//! Response-side aggregate pacing; transcoder loopback is charged only once.
use super::*;
use axum::{body::{Body,Bytes},response::Response};
use futures_util::StreamExt;
use std::{collections::HashMap,sync::{Arc,OnceLock,Weak},time::Duration};
use tokio::sync::Mutex;
type Gate=Mutex<tokio::time::Instant>;
fn gate(key:String)->Arc<Gate>{
 static GATES:OnceLock<std::sync::Mutex<HashMap<String,Weak<Gate>>>>=OnceLock::new();
 let mut gates=GATES.get_or_init(Default::default).lock().unwrap();gates.retain(|_,v|v.strong_count()>0);
 let value=gates.get(&key).and_then(Weak::upgrade).unwrap_or_else(||Arc::new(Mutex::new(tokio::time::Instant::now())));gates.insert(key,Arc::downgrade(&value));value
}
pub fn wrap(s:AppState,a:Auth,p:Policy,response:Response)->Response {
 let (parts,body)=response.into_parts();
 let pacing=gate(format!("{}:{}",s.core_id,a.id));
 let input=body.into_data_stream();
 let stream=futures_util::stream::try_unfold((input,Bytes::new(),s,a,p,pacing,tokio::time::Instant::now()),|(mut input,mut pending,s,a,mut p,pacing,mut checked)|async move{
  if pending.is_empty(){match input.next().await{Some(Ok(bytes))=>pending=bytes,Some(Err(e))=>return Err(std::io::Error::other(e)),None=>return Ok(None)}}
  if checked.elapsed()>=Duration::from_secs(1){
   let auth=crate::auth::authenticate_hash(&s,a.session_hash.clone()).await.map_err(|_|std::io::Error::new(std::io::ErrorKind::PermissionDenied,"会话已失效"))?;
   p=super::policy(&s,&auth).await.map_err(|_|std::io::Error::new(std::io::ErrorKind::PermissionDenied,"账号已停用"))?.ok_or_else(||std::io::Error::other("账号权限已变更"))?;checked=tokio::time::Instant::now();
  }
  let rate=p.bandwidth_bytes_per_second;
  let count=pending.len().min(if rate==0{16*1024}else{(rate/10).clamp(1,16*1024) as usize});
  let bytes=pending.split_to(count);
  if rate>0 {
   // Keep the lock through the wait: dropped responses release the reservation,
   // and parallel devices cannot each spend a separate initial burst.
   let mut next=pacing.lock().await;let deadline=(*next).max(tokio::time::Instant::now())+Duration::from_secs_f64(count as f64/rate as f64);
   tokio::time::sleep_until(deadline).await;*next=deadline;
  }
  Ok::<_,std::io::Error>(Some((bytes,(input,pending,s,a,p,pacing,checked))))
 });
 Response::from_parts(parts,Body::from_stream(stream))
}
