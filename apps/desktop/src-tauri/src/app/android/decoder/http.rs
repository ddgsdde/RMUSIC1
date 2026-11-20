//! Cancellable, bounded loopback streams with entity-checked byte seeking.
use super::*;
use reqwest::{header,Client,Response,StatusCode,Url};
use tokio::sync::{mpsc,oneshot};

#[derive(Clone)] struct Identity {length:u64,etag:String}
struct Headers {length:Option<u64>,identity:Option<Identity>}
pub(super) struct Http {
    url:Url,client:Client,cancel:Arc<Cancel>,receive:Option<Mutex<mpsc::Receiver<io::Result<Vec<u8>>>>>,
    bytes:io::Cursor<Vec<u8>>,position:u64,length:Option<u64>,identity:Option<Identity>,
    // Retain only the initial codec probe window. This is a local replay, never a new HTTP seek.
    replay:Vec<u8>,streamed:u64,
}
fn invalid(message:&str)->io::Error {io::Error::other(message)}
fn text(response:&Response,name:header::HeaderName)->Option<&str> {response.headers().get(name)?.to_str().ok()}
fn headers(response:&Response,expected:Option<&Identity>,offset:u64)->io::Result<Headers> {
    if text(response,header::CONTENT_ENCODING).is_some_and(|value|!value.eq_ignore_ascii_case("identity")) {
        return Err(invalid("媒体响应编码变化，拒绝按字节定位"));
    }
    let length=response.content_length();
    let etag=text(response,header::ETAG);
    if let Some(expected)=expected {
        if response.status()!=StatusCode::PARTIAL_CONTENT {return Err(invalid("媒体定位需要206响应，音源可能已变化"));}
        if etag!=Some(expected.etag.as_str()) {return Err(invalid("媒体实体标记变化，拒绝拼接音源"));}
        let range=text(response,header::CONTENT_RANGE).and_then(|value|value.strip_prefix("bytes "))
            .and_then(|value|value.split_once('/')).and_then(|(segment,total)| {
                let (start,end)=segment.split_once('-')?;
                Some((start.parse::<u64>().ok()?,end.parse::<u64>().ok()?,total.parse::<u64>().ok()?))
            });
        if range!=Some((offset,expected.length-1,expected.length))||length!=Some(expected.length-offset) {
            return Err(invalid("媒体Content-Range或长度变化，拒绝拼接音源"));
        }
        return Ok(Headers{length:Some(expected.length),identity:Some(expected.clone())});
    }
    if response.status()!=StatusCode::OK||response.headers().contains_key(header::CONTENT_RANGE) {
        return Err(invalid("媒体初始响应不是完整200流"));
    }
    let identity=length.zip(etag).filter(|(_,etag)|etag.len()>=2&&etag.starts_with('"')&&etag.ends_with('"'))
        .filter(|_|text(response,header::ACCEPT_RANGES).is_some_and(|value|value.eq_ignore_ascii_case("bytes")))
        .map(|(length,etag)|Identity{length,etag:etag.into()});
    Ok(Headers{length,identity})
}
impl Http {
    pub(super) fn new(url:Url,cancel:Arc<Cancel>)->Result<Self> {
        let client=Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(5)).read_timeout(Duration::from_secs(15)).build()?;
        Ok(Self{url,client,cancel,receive:None,bytes:io::Cursor::new(Vec::new()),position:0,length:None,identity:None,replay:Vec::new(),streamed:0})
    }
    pub(super) fn prepare(&mut self)->io::Result<()> {if self.receive.is_none(){self.start(None,0)?;}Ok(())}
    fn start(&mut self,expected:Option<Identity>,offset:u64)->io::Result<()> {
        let (send,receive)=mpsc::channel(2);let (head,ready)=oneshot::channel();
        let client=self.client.clone();let url=self.url.clone();
        let task=tauri::async_runtime::spawn(async move {
            use futures_util::StreamExt;
            let response=async {
                let mut request=client.get(url).header(header::ACCEPT_ENCODING,"identity");
                if let Some(identity)=&expected {request=request.header(header::RANGE,format!("bytes={offset}-")).header(header::IF_RANGE,&identity.etag);}
                let response=request.send().await.map_err(|error|invalid(&error.without_url().to_string()))?;
                let metadata=headers(&response,expected.as_ref(),offset)?;
                Ok::<_,io::Error>((response,metadata))
            }.await;
            let (response,metadata)=match response {Ok(response)=>response,Err(error)=>{let _=head.send(Err(error));return;}};
            let expected_bytes=metadata.length.map(|length|length-offset);
            if head.send(Ok(metadata)).is_err(){return;}
            let result=async {
                let mut stream=response.bytes_stream();let mut received=0u64;
                while let Some(chunk)=stream.next().await {
                    let chunk=chunk.map_err(|error|invalid(&error.without_url().to_string()))?;
                    received=received.checked_add(chunk.len() as u64).ok_or_else(||invalid("媒体字节计数溢出"))?;
                    if expected_bytes.is_some_and(|length|received>length){return Err(invalid("媒体响应超过声明长度"));}
                    for part in chunk.chunks(64*1024) {if send.send(Ok(part.to_vec())).await.is_err(){return Ok(());}}
                }
                if expected_bytes.is_some_and(|length|received!=length){return Err(invalid("媒体传输中断，尚未读取声明长度"));}
                Ok(())
            }.await;
            if let Err(error)=result {let _=send.send(Err(error)).await;}
        });
        {
            let mut saved=match self.cancel.task.lock(){Ok(saved)=>saved,Err(_)=>{task.abort();return Err(invalid("decoder cancellation lock unavailable"));}};
            if self.cancel.stopped.load(Ordering::Acquire){task.abort();return Err(invalid("decoder cancelled"));}
            if let Some(previous)=saved.replace(task){previous.abort();}
        }
        self.receive=Some(Mutex::new(receive));self.bytes=io::Cursor::new(Vec::new());self.position=offset;self.streamed=offset;self.replay.clear();
        let metadata=ready.blocking_recv().map_err(|_|invalid("媒体读取已取消"))??;
        self.length=metadata.length;self.identity=metadata.identity;Ok(())
    }
    pub(super) fn seekable(&self)->bool {self.identity.is_some()}
    pub(super) fn length(&self)->Option<u64> {self.length}
}
impl Read for Http {
    fn read(&mut self,output:&mut[u8])->io::Result<usize> {
        if self.cancel.stopped.load(Ordering::Acquire){return Err(invalid("decoder cancelled"));}
        if output.is_empty()||self.length==Some(self.position){return Ok(0);}
        if self.position<self.streamed {
            let start=usize::try_from(self.position).map_err(|_|invalid("媒体回读位置无效"))?;
            if start>=self.replay.len(){return Err(invalid("媒体回读超过有界头部窗口"));}
            let count=output.len().min(self.replay.len()-start);output[..count].copy_from_slice(&self.replay[start..start+count]);self.position+=count as u64;return Ok(count);
        }
        if self.bytes.position()==self.bytes.get_ref().len() as u64 {
            let receive=self.receive.as_mut().ok_or_else(||invalid("媒体流尚未准备"))?.get_mut().map_err(|_|invalid("decoder receiver lock unavailable"))?;
            match receive.blocking_recv() {
                Some(chunk)=>self.bytes=io::Cursor::new(chunk?),
                None=>return if self.length.is_some_and(|length|self.position<length){Err(invalid("媒体传输中断，尚未读取声明长度"))}else{Ok(0)},
            }
        }
        let count=self.bytes.read(output)?;
        const REPLAY_LIMIT:usize=256*1024;
        if self.identity.is_none()&&self.streamed==self.replay.len() as u64&&self.replay.len()<REPLAY_LIMIT {
            self.replay.extend_from_slice(&output[..count.min(REPLAY_LIMIT-self.replay.len())]);
        }
        self.position+=count as u64;self.streamed=self.position;Ok(count)
    }
}
impl Seek for Http {
    fn seek(&mut self,from:SeekFrom)->io::Result<u64> {
        if self.cancel.stopped.load(Ordering::Acquire){return Err(invalid("decoder cancelled"));}
        if from==SeekFrom::Current(0){return Ok(self.position);}
        if self.identity.is_none() {
            let target=match from {SeekFrom::Start(offset)=>Some(offset as i128),SeekFrom::Current(offset)=>Some(self.position as i128+offset as i128),SeekFrom::End(_)=>None};
            if let Some(target)=target.filter(|target|*target>=0&&*target<=u64::MAX as i128) {
                let target=target as u64;
                if target==self.position||target==self.streamed||target<=self.replay.len() as u64 {self.position=target;return Ok(target);}
            }
            return Err(io::Error::new(io::ErrorKind::Unsupported,"媒体缺少强实体标记或Range支持，无法定位"));
        }
        let identity=self.identity.clone().unwrap();
        let target=match from {SeekFrom::Start(offset)=>offset as i128,SeekFrom::Current(offset)=>self.position as i128+offset as i128,SeekFrom::End(offset)=>identity.length as i128+offset as i128};
        if target<0||target>identity.length as i128{return Err(io::Error::new(io::ErrorKind::InvalidInput,"媒体定位超出文件范围"));}
        let target=target as u64;
        if target!=self.position {
            if target==identity.length {
                if let Ok(mut task)=self.cancel.task.lock(){if let Some(task)=task.take(){task.abort();}}
                self.bytes=io::Cursor::new(Vec::new());self.position=target;
            }else{self.start(Some(identity),target)?;}
        }
        Ok(target)
    }
}
