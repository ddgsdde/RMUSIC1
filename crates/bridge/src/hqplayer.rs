//! HQPlayer XML-over-TCP control; wire names follow Signalyst hqp-control 6.0.1.
use anyhow::{bail, Context, Result};
use quick_xml::{events::{BytesDecl,BytesEnd,BytesStart,Event},Reader,Writer};
use std::{collections::BTreeMap,time::Duration};
use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::TcpStream};

pub mod bridge;
const MAX_RESPONSE:usize=128*1024;
pub const TIMEOUT:Duration=Duration::from_secs(3);

#[derive(Clone,Debug)]
pub struct Target {pub host:String,pub port:u16}
impl Target {
    pub fn new(host:&str,port:u16)->Result<Self>{
        let host=host.trim().trim_start_matches('[').trim_end_matches(']');
        if host.is_empty()||host.len()>253||port==0||host.chars().any(|c|c.is_whitespace()||c.is_control()||matches!(c,'/'|'\\'|'@'|'?'|'#')){bail!("HQPlayer requires a hostname or IP address and a valid TCP port");}
        Ok(Self{host:host.into(),port})
    }
    pub fn local(&self)->bool {self.host.eq_ignore_ascii_case("localhost")||self.host.parse::<std::net::IpAddr>().is_ok_and(|address|address.is_loopback())}
    pub fn id(&self)->String {format!("hqplayer:{}:{}",self.host,self.port)}
}

#[derive(Clone,Debug,Default)]
pub struct Response {pub command:String,pub attributes:BTreeMap<String,String>,pub metadata:BTreeMap<String,String>}
impl Response {
    pub fn number(&self,key:&str)->Result<f64>{
        let value=self.attributes.get(key).with_context(||format!("HQPlayer omitted {key}"))?.parse::<f64>().with_context(||format!("HQPlayer returned invalid {key}"))?;
        if !value.is_finite(){bail!("HQPlayer returned nonfinite {key}");}Ok(value)
    }
}
fn attributes(element:&BytesStart<'_>)->Result<BTreeMap<String,String>>{
    let mut result=BTreeMap::new();
    for value in element.attributes(){
        let value=value?;
        if result.len()>=128{bail!("HQPlayer response has too many attributes");}
        result.insert(value.key.as_ref().into(),value.normalized_value(quick_xml::XmlVersion::Implicit1_0)?.into());
    }Ok(result)
}
fn document(bytes:&[u8])->Result<Option<(Response,usize)>>{
    let mut reader=Reader::from_reader(bytes);let mut depth=0;let mut result=Response::default();
    loop {
        let event=match reader.read_event(){Ok(event)=>event,Err(quick_xml::Error::Syntax(_))=>return Ok(None),Err(error)=>return Err(error.into())};
        match event {
            Event::Start(element)|Event::Empty(element)=>{
                let empty=bytes.get(reader.buffer_position().saturating_sub(2) as usize)==Some(&b'/');
                if depth==0 {result.command=element.name().as_ref().into();result.attributes=attributes(&element)?;}
                else if depth==1&&element.name().as_ref()=="metadata"{result.metadata=attributes(&element)?;}
                if !empty{depth+=1;if depth>8{bail!("HQPlayer XML nesting exceeds limit");}}
                else if depth==0{return Ok(Some((result,reader.buffer_position() as usize)));}
            },
            Event::End(_)=>{depth-=1;if depth==0{return Ok(Some((result,reader.buffer_position() as usize)));}},
            Event::DocType(_)=>bail!("HQPlayer XML DTD is not supported"),
            Event::Eof=>return Ok(None),
            _=>{},
        }
    }
}
pub fn command(name:&str,attrs:&[(&str,&str)],metadata:&[(&str,&str)])->Result<Vec<u8>>{
    let mut writer=Writer::new(Vec::new());writer.write_event(Event::Decl(BytesDecl::new("1.0",Some("UTF-8"),None)))?;
    let mut root=BytesStart::new(name);for &(key,value) in attrs {root.push_attribute((key,value));}
    if metadata.is_empty(){writer.write_event(Event::Empty(root))?;}else{
        writer.write_event(Event::Start(root))?;let mut item=BytesStart::new("metadata");for &(key,value) in metadata{item.push_attribute((key,value));}
        writer.write_event(Event::Empty(item))?;writer.write_event(Event::End(BytesEnd::new(name)))?;
    }
    let mut bytes=writer.into_inner();bytes.push(b'\n');Ok(bytes)
}

pub struct Client {stream:Option<TcpStream>,buffer:Vec<u8>}
impl Client {
    pub async fn connect(target:&Target)->Result<Self>{
        let stream=tokio::time::timeout(TIMEOUT,TcpStream::connect((target.host.as_str(),target.port))).await.context("HQPlayer connection timed out")??;
        stream.set_nodelay(true)?;Ok(Self{stream:Some(stream),buffer:Vec::new()})
    }
    pub async fn request(&mut self,name:&str,attrs:&[(&str,&str)],metadata:&[(&str,&str)])->Result<Response>{
        let bytes=command(name,attrs,metadata)?;
        let result=tokio::time::timeout(TIMEOUT,async {
            let stream=self.stream.as_mut().context("HQPlayer connection is no longer usable")?;
            stream.write_all(&bytes).await?;
            loop {
                if let Some((response,used))=document(&self.buffer)?{
                    if used>MAX_RESPONSE{bail!("HQPlayer response exceeds limit");}
                    self.buffer.drain(..used);
                    if response.command!=name{bail!("HQPlayer response does not match {name}");}
                    if response.attributes.get("result").is_some_and(|value|value!="OK"){bail!("HQPlayer rejected {name}");}
                    return Ok(response);
                }
                if self.buffer.len()>MAX_RESPONSE{bail!("HQPlayer response exceeds limit");}
                let mut block=[0u8;4096];let size=stream.read(&mut block).await?;
                if size==0{bail!("HQPlayer closed the control connection");}self.buffer.extend_from_slice(&block[..size]);
            }
        }).await.context("HQPlayer control request timed out").and_then(|value|value);
        if result.is_err(){self.stream.take();self.buffer.clear();}result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_fragmentation_coalescing_metadata_and_escaping(){
        let bytes=command("PlaylistAdd",&[("uri","https://core/audio?x=1&y=\"二\"")],&[("song","<歌曲> & 'live'")]).unwrap();
        for end in 0..bytes.len()-1{assert!(document(&bytes[..end]).unwrap().is_none());}
        let (value,used)=document(&bytes).unwrap().unwrap();assert_eq!(value.command,"PlaylistAdd");assert_eq!(value.attributes["uri"],"https://core/audio?x=1&y=\"二\"");assert_eq!(value.metadata["song"],"<歌曲> & 'live'");
        let joined=[bytes.clone(),command("Stop",&[],&[]).unwrap()].concat();let(_,first)=document(&joined).unwrap().unwrap();assert_eq!(first,used);assert_eq!(document(&joined[first..]).unwrap().unwrap().0.command,"Stop");
        assert!(document(b"<!DOCTYPE x><Status/>").is_err());assert!(document(b"<Status></Other>").is_err());
    }
    #[test]
    fn target_and_numbers_reject_invalid_values(){
        assert!(Target::new("http://host",4321).is_err());assert!(Target::new("host",0).is_err());assert!(Target::new("[::1]",4321).unwrap().local());assert!(!Target::new("192.168.1.5",4321).unwrap().local());
        let response=document(b"<Status position='NaN'/>").unwrap().unwrap().0;assert!(response.number("position").is_err());assert!(response.number("min").is_err());
    }
    #[tokio::test]
    async fn real_tcp_handles_fragmented_documents_and_rejects_errors(){
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let target=Target::new("127.0.0.1",listener.local_addr().unwrap().port()).unwrap();
        let server=tokio::spawn(async move{let(mut socket,_)=listener.accept().await.unwrap();let mut bytes=[0;1024];assert!(socket.read(&mut bytes).await.unwrap()>0);
            for part in [b"<?xml version='1.0'?>\n<Status sta".as_slice(),b"te='2' min='1' sec='3'><metadata uri='http://test?a=1&amp;b=2'/>",b"</Status>\n"]{socket.write_all(part).await.unwrap();tokio::task::yield_now().await;}
            assert!(socket.read(&mut bytes).await.unwrap()>0);socket.write_all(b"<Stop result='Error'/>\n").await.unwrap();});
        let mut client=Client::connect(&target).await.unwrap();let status=client.request("Status",&[("subscribe","0")],&[]).await.unwrap();assert_eq!(status.number("min").unwrap(),1.);assert_eq!(status.metadata["uri"],"http://test?a=1&b=2");
        assert!(client.request("Stop",&[],&[]).await.unwrap_err().to_string().contains("rejected"));assert!(client.stream.is_none());server.await.unwrap();
    }
    #[tokio::test]
    async fn oversized_response_and_disconnect_cannot_be_reused(){
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let target=Target::new("127.0.0.1",listener.local_addr().unwrap().port()).unwrap();
        let server=tokio::spawn(async move{let(mut socket,_)=listener.accept().await.unwrap();let mut bytes=[0;1024];socket.read(&mut bytes).await.unwrap();let _=socket.write_all(&[b"<Status x='".as_slice(),&vec![b'x';MAX_RESPONSE+8192]].concat()).await;});
        let mut client=Client::connect(&target).await.unwrap();assert!(client.request("Status",&[],&[]).await.unwrap_err().to_string().contains("limit"));assert!(client.stream.is_none());server.await.unwrap();
    }
}
