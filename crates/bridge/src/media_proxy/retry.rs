//! Resume only a strongly identified original HTTP entity. Never splice conversion streams.
use super::*;
use std::io;
use futures_util::stream::BoxStream;
fn failure(message:&str)->io::Error{io::Error::new(io::ErrorKind::UnexpectedEof,message)}
struct Transfer {
 stream:BoxStream<'static,Result<Bytes,reqwest::Error>>,client:reqwest::Client,url:String,
 cursor:u64,end:u64,total:u64,etag:String,retries:u8,last_resume:u64,cancelled:tokio::sync::watch::Receiver<bool>,done:bool,
}
// A connection that carried a real block made progress. A sequence of headers or
// tiny partial bodies still shares the same retry budget.
const RESUME_PROGRESS_BYTES: u64 = 64 * 1024;
fn content_range(value:&str)->Option<(u64,u64,u64)>{let (range,total)=value.strip_prefix("bytes ")?.split_once('/')?;let(start,end)=range.split_once('-')?;Some((start.parse().ok()?,end.parse().ok()?,total.parse().ok()?))}
pub(super) fn body(response:reqwest::Response,client:reqwest::Client,url:String,cancellation:tokio::sync::watch::Receiver<bool>)->BoxStream<'static,Result<Bytes,io::Error>>{
 let headers=response.headers();
 let etag=headers.get(header::ETAG).and_then(|v|v.to_str().ok()).filter(|v|v.starts_with('"')&&v.ends_with('"'));
 let length=response.content_length();
 let range=if response.status()==StatusCode::PARTIAL_CONTENT {headers.get(header::CONTENT_RANGE).and_then(|v|v.to_str().ok()).and_then(content_range)}else if response.status()==StatusCode::OK{length.filter(|n|*n>0).map(|n|(0,n-1,n))}else{None};
 let safe=headers.get(header::CONTENT_ENCODING).is_none_or(|v|v=="identity")&&headers.get(header::ACCEPT_RANGES).is_some_and(|v|v=="bytes");
 let Some(((start,end,total),etag))=range.zip(etag.map(str::to_owned)).filter(|((start,end,total),_)|safe&&end>=start&&end<total&&length==Some(end-start+1)) else {
  return response.bytes_stream().map(|v|v.map_err(|e|io::Error::other(e.without_url()))).boxed();
 };
 let transfer=Transfer{stream:response.bytes_stream().boxed(),client,url,cursor:start,end,total,etag,retries:0,last_resume:start,cancelled:cancellation,done:false};
 futures_util::stream::unfold(transfer,|mut t|async move {
  if t.done{return None;}
  loop {
   let next=tokio::select!{biased;_=cancelled(t.cancelled.clone())=>return None,
    value=tokio::time::timeout(std::time::Duration::from_secs(8),t.stream.next())=>value};
   match next {
    Ok(Some(Ok(bytes))) if bytes.is_empty()=>continue,
    Ok(Some(Ok(bytes)))=>{
     if bytes.len() as u64>t.end.saturating_add(1).saturating_sub(t.cursor){t.done=true;return Some((Err(io::Error::new(io::ErrorKind::InvalidData,"音源实体长度发生变化")),t));}
     t.cursor+=bytes.len() as u64;
     if t.cursor - t.last_resume >= RESUME_PROGRESS_BYTES { t.retries = 0; }
     return Some((Ok(bytes),t));
    },
    Ok(None) if t.cursor==t.end+1=>return None,
    _=>{}
   }
   if t.cursor==t.end+1{return None;}
   // The old body has already failed. Do not wait on it again if a range request times out.
   let response=loop {
    if t.retries>=3 {t.done=true;return Some((Err(failure("音源连接中断，校验续传重试已用尽")),t));}
    t.retries+=1;
    tokio::select!{biased;_=cancelled(t.cancelled.clone())=>return None,_=tokio::time::sleep(std::time::Duration::from_millis(200*u64::from(t.retries)))=>{}}
    let request=t.client.get(&t.url).header(header::RANGE,format!("bytes={}-{}",t.cursor,t.end)).header(header::IF_RANGE,&t.etag).header(header::ACCEPT_ENCODING,"identity");
    let response=tokio::select!{biased;_=cancelled(t.cancelled.clone())=>return None,
     response=tokio::time::timeout(std::time::Duration::from_secs(5),request.send())=>response};
    if let Ok(Ok(response))=response {
     if !matches!(response.status().as_u16(),408|429|502|503|504) {break response;}
    }
   };
   let h=response.headers();
   if response.status()!=StatusCode::PARTIAL_CONTENT||h.get(header::CONTENT_RANGE).and_then(|v|v.to_str().ok()).and_then(content_range)!=Some((t.cursor,t.end,t.total))
    ||h.get(header::ETAG).and_then(|v|v.to_str().ok())!=Some(t.etag.as_str())||response.content_length()!=Some(t.end-t.cursor+1)||h.get(header::CONTENT_ENCODING).is_some_and(|v|v!="identity") {
     t.done=true;return Some((Err(io::Error::new(io::ErrorKind::InvalidData,"音源实体或续传范围已改变，未拼接不同音频")),t));
   }
   t.last_resume=t.cursor;
   t.stream=response.bytes_stream().boxed();
  }
 }).boxed()
}
#[cfg(test)]mod tests {
 use super::*;use tokio::io::{AsyncReadExt,AsyncWriteExt};use futures_util::TryStreamExt;
 async fn fixture(changed:bool)->(String,tokio::task::JoinHandle<String>){
  let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
  let task=tokio::spawn(async move {
   let(mut first,_)=listener.accept().await.unwrap();let mut request=vec![0;4096];first.read(&mut request).await.unwrap();
   first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nETag: \"same\"\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n0123").await.unwrap();first.shutdown().await.unwrap();drop(first);
   let(mut second,_)=listener.accept().await.unwrap();let n=second.read(&mut request).await.unwrap();let headers=String::from_utf8_lossy(&request[..n]).to_ascii_lowercase();
   if changed {second.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nETag: \"new\"\r\nConnection: close\r\n\r\nabcdefghij").await.unwrap();}
   else {second.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 6\r\nContent-Range: bytes 4-9/10\r\nETag: \"same\"\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n456789").await.unwrap();}headers
  });(format!("http://{address}/audio"),task)
 }
 #[tokio::test]async fn resumes_truncated_original_at_the_exact_confirmed_entity(){
  let(url,server)=fixture(false).await;let client=reqwest::Client::new();let response=client.get(&url).send().await.unwrap();let(_stop,cancel)=tokio::sync::watch::channel(false);
  let bytes=body(response,client,url,cancel).try_fold(Vec::new(),|mut all,chunk|async move{all.extend_from_slice(&chunk);Ok(all)}).await.unwrap();assert_eq!(bytes,b"0123456789");let request=server.await.unwrap();assert!(request.contains("range: bytes=4-9"));assert!(request.contains("if-range: \"same\""));
 }
 #[tokio::test]async fn changed_or_ignored_range_never_splices_another_entity(){
  let(url,server)=fixture(true).await;let client=reqwest::Client::new();let response=client.get(&url).send().await.unwrap();let(_stop,cancel)=tokio::sync::watch::channel(false);
  let mut stream=body(response,client,url,cancel);let mut bytes=Vec::new();let mut failed=false;while let Some(chunk)=stream.next().await{match chunk{Ok(chunk)=>bytes.extend_from_slice(&chunk),Err(_)=>{failed=true;break;}}}
  assert!(failed);assert_eq!(bytes,b"0123");server.await.unwrap();
 }

 #[tokio::test]
 async fn repeated_disconnects_can_resume_when_each_connection_delivers_a_block() {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=format!("http://{}/audio",listener.local_addr().unwrap());
    let total=5 * RESUME_PROGRESS_BYTES;
    let server=tokio::spawn(async move {
        for part in 0..5 {
            let (mut socket,_)=listener.accept().await.unwrap();
            let mut input=vec![0;4096];
            let size=socket.read(&mut input).await.unwrap();
            let start=part * RESUME_PROGRESS_BYTES;
            let headers=if start==0 {"HTTP/1.1 200 OK\r\n".to_owned()}else{
                assert!(String::from_utf8_lossy(&input[..size]).to_ascii_lowercase().contains(&format!("range: bytes={start}-{}",total-1)));
                format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{total}\r\n",total-1)
            };
            socket.write_all(format!("{headers}Content-Length: {}\r\nETag: \"same\"\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",total-start).as_bytes()).await.unwrap();
            socket.write_all(&vec![part as u8; RESUME_PROGRESS_BYTES as usize]).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    let client=reqwest::Client::new();
    let first=client.get(&url).send().await.unwrap();
    let (_stop,cancel)=tokio::sync::watch::channel(false);
    let chunks: Vec<_>=body(first,client,url,cancel).try_collect().await.unwrap();
    let bytes=chunks.concat();
    assert_eq!(bytes.len(),total as usize);
    for (part,block) in bytes.chunks_exact(RESUME_PROGRESS_BYTES as usize).enumerate() {
        assert!(block.iter().all(|&value| value==part as u8));
    }
    server.await.unwrap();
 }
}
