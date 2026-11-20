//! A format request may only retarget the already-authorized Core media route.
use super::*;
use serde::Deserialize;
#[derive(Clone,Default,Deserialize,PartialEq)]
pub(super) struct Request {pub(super) conversion:Option<String>,pub(super) rate:Option<u32>,pub(super) bits:Option<u16>,pub(super) channels:Option<u16>,pub(super) start:Option<f64>,pub(super) bitrate:Option<u32>}
#[derive(Clone)]
pub(super) struct Confirmed {pub rate:u32,pub bits:u16,pub channels:u16,pub codec:Option<String>,pub bitrate:u32,pub loudness:Option<bool>}
impl Confirmed {
    pub fn header(&self)->String {match self.codec.as_deref(){
        Some("opus")=>format!("v1;rate=48000;channels={};target_bitrate={}000;lossless=false",self.channels,self.bitrate),
        Some("flac")=>format!("v1;rate={};bits={};channels={};container=ogg",self.rate,self.bits,self.channels),
        _=>format!("v1;rate={};bits={};channels={}",self.rate,self.bits,self.channels),
    }}
    pub fn header_name(&self)->&str {match self.codec.as_deref(){Some("opus")=>"X-RMusic-Mobile-Opus",Some("flac")=>"X-RMusic-Native-FLAC",_=>"X-RMusic-Native-PCM"}}
}
impl Request {
    pub fn target(&self,source:&str)->std::result::Result<Option<(String,Confirmed)>,String>{
        let Some(mode)=self.conversion.as_deref() else{return Ok(None);};
        let (rate,bits,channels,start)=(self.rate.unwrap_or(0),self.bits.unwrap_or(0),self.channels.unwrap_or(0),self.start.unwrap_or(0.));
        let opus=mode=="mobile_opus";let flac=mode=="mobile_flac";
        if !["core","mobile_opus","mobile_flac"].contains(&mode)||!(8000..=768000).contains(&rate)||(!opus&&![16,24,32].contains(&bits))||(opus&&(rate!=48000||!matches!(self.bitrate,Some(96|160|320))))||!(1..=2).contains(&channels)||!start.is_finite()||start<0.{return Err("invalid PCM target".into());}
        let mut url=url::Url::parse(source).map_err(|_|"invalid media route")?;
        let path=url.path().replacen("/api/v1/media/",if opus{"/api/v1/mobile-opus/"}else if flac{"/api/v1/native-flac/"}else{"/api/v1/native-pcm/"},1);
        if path==url.path(){return Err("this source has no Core PCM route".into());}
        url.set_path(&path);
        let ticket=url.query_pairs().find(|(k,_)|k=="ticket").map(|(_,v)|v.into_owned());
        let core_render=mode=="core"&&url.query_pairs().any(|(key,_)|key=="core_pcm");
        url.set_query(None);let mut query=url.query_pairs_mut();
        if core_render { query.append_pair("core_render","true"); }
        if let Some(ticket)=ticket {query.append_pair("ticket",&ticket);}
        query.append_pair("channels",&channels.to_string()).append_pair("start",&start.to_string());
        if opus{query.append_pair("accept_lossy","true").append_pair("bitrate",&self.bitrate.unwrap().to_string());}else{query.append_pair("rate",&rate.to_string()).append_pair("bits",&bits.to_string());}drop(query);
        Ok(Some((url.into(),Confirmed{rate,bits,channels,codec:if opus{Some("opus".into())}else if flac{Some("flac".into())}else{None},bitrate:self.bitrate.unwrap_or(0),loudness:None})))
    }
}
pub(super) async fn file(path:&std::path::Path,headers:&HeaderMap,method:&Method,cancellation:tokio::sync::watch::Receiver<bool>)->Response {
    use tokio::io::{AsyncSeekExt,AsyncReadExt};
    let Ok(mut file)=tokio::fs::File::open(path).await else{return Response::builder().status(404).body(Body::empty()).unwrap();};
    let Ok(metadata)=file.metadata().await else{return Response::builder().status(500).body(Body::empty()).unwrap();};
    let total=metadata.len();let range=headers.get(header::RANGE).and_then(|v|v.to_str().ok());
    let segment=range.and_then(|v|v.strip_prefix("bytes=")).and_then(|v|v.split_once('-')).filter(|(_,b)|!b.contains(','));
    let bounds=match segment {
        Some(("",suffix))=>suffix.parse::<u64>().ok().filter(|v|*v>0).map(|n|(total.saturating_sub(n),total.saturating_sub(1))),
        Some((a,b))=>a.parse::<u64>().ok().zip(if b.is_empty(){Some(total.saturating_sub(1))}else{b.parse::<u64>().ok().map(|n|n.min(total.saturating_sub(1))) }),
        None if range.is_none()=>Some((0,total.saturating_sub(1))),_=>None,
    };
    let Some((start,end))=bounds.filter(|(a,b)|a<=b&&*a<total) else{return Response::builder().status(416).header(header::CONTENT_RANGE,format!("bytes */{total}")).body(Body::empty()).unwrap();};
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err(){return Response::builder().status(500).body(Body::empty()).unwrap();}
    let mut response=Response::builder().status(if range.is_some(){206}else{200}).header(header::CONTENT_LENGTH,end-start+1).header(header::ACCEPT_RANGES,"bytes");
    if range.is_some(){response=response.header(header::CONTENT_RANGE,format!("bytes {start}-{end}/{total}"));}
    response.body(if method==Method::HEAD{Body::empty()}else{Body::from_stream(tokio_util::io::ReaderStream::with_capacity(file.take(end-start+1),65536).take_until(super::cancelled(cancellation)))}).unwrap()
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn target_retains_only_existing_media_authority_and_valid_format(){
        let query=Request{conversion:Some("core".into()),rate:Some(96000),bits:Some(24),channels:Some(2),start:Some(1.25),..Default::default()};
        let(url,target)=query.target("https://core.test/prefix/api/v1/media/qqmusic%3Atrack?ticket=synthetic-ticket&discard=1").unwrap().unwrap();
        let parsed=url::Url::parse(&url).unwrap();assert_eq!(parsed.path(),"/prefix/api/v1/native-pcm/qqmusic%3Atrack");assert!(parsed.query_pairs().any(|(k,v)|k=="ticket"&&v=="synthetic-ticket"));assert!(!parsed.query_pairs().any(|(k,_)|k=="discard"));assert_eq!(target.rate,96000);
        assert!(query.target("https://core.test/arbitrary").is_err());
        assert!(Request{rate:Some(768001),..query}.target("https://core.test/api/v1/media/a").is_err());
    }
    #[tokio::test] async fn mobile_profiles_require_exact_confirmation_and_do_not_cache_original_bytes(){
        for (mode,codec,header,value) in [("mobile_opus","opus","X-RMusic-Mobile-Opus","v1;rate=48000;channels=2;target_bitrate=160000;lossless=false"),("mobile_flac","flac","X-RMusic-Native-FLAC","v1;rate=48000;bits=24;channels=2;container=ogg")] {
            for confirmed in [true,false] {
                let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
                let route=if codec=="opus"{"/api/v1/mobile-opus/test"}else{"/api/v1/native-flac/test"};
                let router=Router::new().route(route,get(move|headers:HeaderMap,Query(query):Query<std::collections::HashMap<String,String>>|async move {
                    assert!(!headers.contains_key(header::RANGE));assert_eq!(query.get("ticket").unwrap(),"synthetic");
                    if codec=="opus"{assert_eq!(query.get("accept_lossy").unwrap(),"true");}
                    let mut response=Response::builder();if confirmed{response=response.header(header,value);}response.body(Body::from("converted-data")).unwrap()
                }));
                let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap()});
                let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),skip_certificate_verification:false,certificate_sha256:None,alternatives:vec![],core_id:None};
                let observed=Arc::new(std::sync::atomic::AtomicUsize::new(0));let count=observed.clone();
                let proxy=MediaProxy::start(&profile,&format!("http://{address}/api/v1/media/test?ticket=synthetic"),Some(Arc::new(move|_,_,bytes|{count.fetch_add(bytes.len(),Ordering::Relaxed);}))).await.unwrap();
                let response=reqwest::Client::new().get(format!("{}?conversion={mode}&rate=48000&bits=24&channels=2&bitrate=160",proxy.url)).header(header::RANGE,"bytes=0-").send().await.unwrap();
                assert_eq!(response.status(),if confirmed{StatusCode::OK}else{StatusCode::BAD_GATEWAY});let _=response.bytes().await.unwrap();
                assert_eq!(observed.load(Ordering::Relaxed),0);assert_eq!(proxy.source_failed(),!confirmed);server.abort();
            }
        }
    }
    #[tokio::test] async fn real_proxy_requires_confirmation_and_never_caches_converted_bytes(){
        for confirmed in [true,false]{
            let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
            let router=Router::new().route("/api/v1/native-pcm/test",get(move|headers:HeaderMap|async move{assert!(!headers.contains_key(header::RANGE));let mut response=Response::builder().status(200);if confirmed{response=response.header("X-RMusic-Native-PCM","v1;rate=44100;bits=24;channels=2");}response.body(Body::from("actual-converted-bytes")).unwrap()}));
            let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap()});
            let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None};
            let observed=Arc::new(std::sync::atomic::AtomicUsize::new(0));let count=observed.clone();
            let proxy=MediaProxy::start(&profile,&format!("http://{address}/api/v1/media/test?ticket=fixture"),Some(Arc::new(move|_,_,bytes|{count.fetch_add(bytes.len(),Ordering::Relaxed);}))).await.unwrap();
            let response=reqwest::Client::new().get(format!("{}?conversion=core&rate=44100&bits=24&channels=2",proxy.url)).header(header::RANGE,"bytes=0-").send().await.unwrap();
            assert_eq!(response.status(),if confirmed{StatusCode::OK}else{StatusCode::BAD_GATEWAY});let _=response.bytes().await.unwrap();assert_eq!(observed.load(Ordering::Relaxed),0);assert_eq!(proxy.conversion.lock().unwrap().is_some(),confirmed);server.abort();
        }
    }
}
