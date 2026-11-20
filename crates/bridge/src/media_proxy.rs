//! Loopback-only bounded streaming proxy keeps FFmpeg behind Rust TLS verification.
use crate::connection::ConnectionProfile;
use anyhow::Result;
use axum::{
    body::{Body,Bytes},
    extract::{State,Query},
    http::{header, HeaderMap, Method, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use futures_util::{StreamExt,stream::BoxStream};
use std::sync::{Arc,atomic::{AtomicBool,AtomicU64,Ordering}};

/// Optional application media cache. Ownership stays with this endpoint session, never global.
pub trait MediaResolver: Send + Sync {
    fn resolve<'a>(&'a self, profile: &'a ConnectionProfile, track: &'a str, source: &'a str)
        -> futures_util::future::BoxFuture<'a, Result<Arc<MediaProxy>>>;
}
mod conversion;
mod remote;
mod retry;
mod cache_writer;
mod warm;
mod spool;
mod ogg_progress;
pub use cache_writer::buffered as buffered_cache_observer;
pub use remote::{StreamingPolicy,NetworkBuffer};
pub type MediaObserverFactory=Arc<dyn Fn(&reqwest::header::HeaderMap)->Option<MediaObserver>+Send+Sync>;
pub type MediaObserver = Arc<dyn Fn(u64, Option<u64>, &[u8]) + Send + Sync>;
#[derive(Clone)]
struct ProxyState {
    warm:Arc<tokio::sync::Mutex<Option<(conversion::Request,Response)>>>,
    network_buffer:Arc<std::sync::RwLock<Option<NetworkBuffer>>>,
    network_client:reqwest::Client,
    downloaded:Arc<std::sync::Mutex<Option<(u64,u64)>>>,
    fill:Arc<std::sync::OnceLock<StagedBuffer>>,
    converted_to_end:Arc<AtomicBool>,
    converted_ms:Arc<AtomicU64>,
    remote: Option<remote::RemoteSpec>,
    client: reqwest::Client,
    source: String,
    local: Option<std::path::PathBuf>,
    conversion: Arc<std::sync::Mutex<Option<conversion::Confirmed>>>,
    observer: Option<MediaObserver>,
    observer_factory: Option<MediaObserverFactory>,
    entity: Option<String>,
    cancelled: tokio::sync::watch::Receiver<bool>,
    source_failed: Arc<AtomicBool>,
    network_interrupted: Arc<AtomicBool>,
}
struct StagedBuffer {
    gate: tokio::sync::watch::Sender<f64>,
    directory: Option<std::path::PathBuf>,
}
pub struct MediaProxy {
    preparation:Option<ProxyState>,
    network_buffer:Arc<std::sync::RwLock<Option<NetworkBuffer>>>,
    remote: Option<remote::RemoteSpec>,
    _lease: Option<Box<dyn Send + Sync>>,
    pub url: String,
    conversion: Arc<std::sync::Mutex<Option<conversion::Confirmed>>>,
    task: tokio::task::JoinHandle<()>,
    cancelled: tokio::sync::watch::Sender<bool>,
    source_failed: Arc<AtomicBool>,
}
impl MediaProxy {
    /// Original-byte coverage from the beginning of the active stream. A fully
    /// verified cache file is available in its entirety.
    pub fn buffered_fraction(&self) -> Option<f64> {
        let Some(state) = &self.preparation else { return Some(1.); };
        if state.local.is_some() || state.converted_to_end.load(Ordering::Acquire) { return Some(1.); }
        state.downloaded.lock().ok().and_then(|value| *value)
            .map(|(end,total)| end.min(total) as f64 / total as f64)
    }
    pub fn buffered_audio_seconds(&self) -> Option<f64> {
        let milliseconds = self.preparation.as_ref()?.converted_ms.load(Ordering::Acquire);
        (milliseconds > 0).then(|| milliseconds as f64 / 1000.)
    }
    pub fn fill_after_startup(&self) {
        self.fill_after_startup_in(None);
    }
    pub fn fill_after_startup_in(&self, directory: Option<std::path::PathBuf>) {
        if let Some(state) = &self.preparation {
            state.fill.get_or_init(|| StagedBuffer {
                gate: tokio::sync::watch::channel(0.).0,
                directory,
            });
        }
    }
    pub fn staged_buffer(&self) -> bool {
        self.preparation.as_ref().is_some_and(|state| state.fill.get().is_some())
    }
    pub fn filling_current(&self) -> bool {
        self.staged_buffer() && self.buffered_fraction() != Some(1.)
    }
    pub fn advance_playback(&self, position: f64) {
        if !position.is_finite() || position <= 0. { return; }
        if let Some(fill) = self.preparation.as_ref().and_then(|state| state.fill.get()) {
            fill.gate.send_if_modified(|played| {
                if *played >= 15. || position <= *played { return false; }
                *played = position.min(15.);
                true
            });
        }
    }
    pub async fn start(
        profile: &ConnectionProfile,
        source: &str,
        observer: Option<MediaObserver>,
    ) -> Result<Self> {
        Self::start_entity(profile, source, observer, None).await
    }
    /// Cache observers may only combine bytes of this exact strong HTTP entity.
    pub async fn start_entity(profile: &ConnectionProfile, source: &str, observer: Option<MediaObserver>, entity: Option<String>) -> Result<Self> {
        Self::start_inner(profile,source,observer,entity,None,None,None).await
    }
    pub async fn cached_file(profile:&ConnectionProfile,source:&str,path:std::path::PathBuf,lease:Box<dyn Send+Sync>)->Result<Self>{
        anyhow::ensure!(path.is_absolute()&&path.is_file(),"invalid complete cache file");
        Ok(Self::start_inner(profile,source,None,None,Some(path),None,None).await?.with_lease(lease))
    }
    pub async fn streaming_cache(profile:&ConnectionProfile,source:&str,factory:MediaObserverFactory)->Result<Self>{
        Self::start_inner(profile,source,None,None,None,Some(factory),None).await
    }
    pub fn core_pcm(source: &str) -> Option<bool> {
        reqwest::Url::parse(source).ok()?.query_pairs()
            .find(|(key, _)| key=="core_pcm").and_then(|(_, value)| match value.as_ref() {
                "render"=>Some(true), "compatibility"=>Some(false), _=>None,
            })
    }
    pub fn core_sample_rate(source: &str) -> Option<u32> {
        reqwest::Url::parse(source).ok()?.query_pairs()
            .find(|(key, _)| key=="dsp_rate")?.1.parse().ok()
            .filter(|rate| rmusic_audio::output::PCM_RATES.contains(rate))
    }
    async fn start_inner(profile:&ConnectionProfile,source:&str,observer:Option<MediaObserver>,entity:Option<String>,local:Option<std::path::PathBuf>,observer_factory:Option<MediaObserverFactory>,remote:Option<remote::RemoteSpec>)->Result<Self>{
        profile.validate_media_url(source)?;
        let client = if remote.is_some(){profile.streaming_http()?}else{profile.http()?};
        let network_client=profile.streaming_http()?;
        let network_buffer=Arc::new(std::sync::RwLock::new(None));
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let path = format!("/audio/{secret}");
        let (cancelled, cancellation) = tokio::sync::watch::channel(false);
        let source_failed=Arc::new(AtomicBool::new(false));
        let conversion=Arc::new(std::sync::Mutex::new(None));
        let state=ProxyState {
                warm:Default::default(),
                network_buffer:network_buffer.clone(),network_client,downloaded:Default::default(),fill:Default::default(),converted_to_end:Default::default(),converted_ms:Default::default(),
                remote:remote.clone(),client,
                source: source.to_owned(),local,conversion:conversion.clone(),
                observer, observer_factory, entity,
                cancelled: cancellation,
                network_interrupted:Default::default(),
                source_failed:source_failed.clone(),
            };
        let router = Router::new()
            .route(&path, get(proxy).head(proxy))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let url = format!("http://127.0.0.1:{}{}", listener.local_addr()?.port(), path);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let mut url=reqwest::Url::parse(&url)?;
        if remote.is_some() {
            url.query_pairs_mut().append_pair("roaming","1");
        } else if let Some(required)=Self::core_pcm(source) {
            url.query_pairs_mut().append_pair("core_pcm",if required {"render"} else {"compatibility"});
            if let Some(rate)=Self::core_sample_rate(source) { url.query_pairs_mut().append_pair("dsp_rate",&rate.to_string()); }
        }
        let url=url.to_string();
        Ok(Self { preparation:Some(state),network_buffer,remote,url, task, cancelled,source_failed,conversion, _lease: None })
    }
    /// Caller must verify this complete local file and hold its cache lease for our lifetime.
    pub fn local_file(path: &std::path::Path, lease: Box<dyn Send + Sync>) -> Result<Self> {
        anyhow::ensure!(path.is_absolute() && rmusic_audio::file::open(path)?.metadata()?.is_file(), "invalid complete cache file");
        let (cancelled, _) = tokio::sync::watch::channel(false);
        Ok(Self { preparation:None,network_buffer:Arc::new(std::sync::RwLock::new(None)),remote:None,url: path.to_str().ok_or_else(||anyhow::anyhow!("non-UTF8 cache path"))?.into(),
            conversion:Arc::new(std::sync::Mutex::new(None)),task: tokio::spawn(async {}), cancelled, source_failed: Arc::new(AtomicBool::new(false)), _lease: Some(lease) })
    }
    pub fn with_lease(mut self, lease: Box<dyn Send + Sync>) -> Self { self._lease=Some(lease); self }
    pub fn with_network_buffer(self,buffer:NetworkBuffer)->Self {
        self.configure_network_buffer(buffer);self
    }
    pub fn configure_network_buffer(&self,buffer:NetworkBuffer){if let Ok(mut current)=self.network_buffer.write(){*current=Some(buffer);}}
    pub fn annotate(&self,path:&mut rmusic_audio::output::SignalPath){
        let staged = self.staged_buffer();
        if staged {
            path.processing.push("Remote prefetch: adaptive 5–10 s, then buffer to the end from 15 s of playback".into());
        } else if let Some(buffer)=self.network_buffer.read().ok().and_then(|v|v.clone()) {
            path.processing.push(format!("Compressed-byte network prefetch: up to {} KiB",buffer.max_bytes/1024));
        }
        if let Some(remote)=&self.remote {
            if let Ok(report)=self.conversion.lock(){if let Some(report)=report.as_ref(){
                let mut transport = format!("Remote FLAC transport: {} Hz / {} bit / {} ch; Ogg", report.rate, report.bits, report.channels);
                if !staged { transport.push_str(&format!("; buffer up to {} s / 32 MiB",remote.policy.buffer_seconds)); }
                path.processing.push(transport);
                if report.rate==path.sample_rate&&report.channels==path.channels {path.processing.retain(|p|!p.starts_with("FFmpeg rate conversion")&&!p.starts_with("FFmpeg channel conversion"));}
                let source=&remote.source;
                if source["sample_rate"].as_u64()!=Some(report.rate as u64)||source["channels"].as_u64()!=Some(report.channels as u64)||source["bit_depth"].as_u64().is_some_and(|n|n>report.bits as u64)||matches!(source["format"].as_str(),Some("dsf"|"dff")) {
                    path.processing.push(format!("Core stream conversion: {} Hz / {} bit / {} ch → {} Hz / {} bit / {} ch",source["sample_rate"],source["bit_depth"],source["channels"],report.rate,report.bits,report.channels));path.bit_perfect_eligible=false;
                }
            }}return;
        }
        if let Ok(report)=self.conversion.lock(){if let Some(report)=report.as_ref(){
            if Self::core_pcm(&self.url)==Some(true) {
                path.processing.push("Core DSP · 按音区设置处理".into());
                match report.loudness {
                    Some(true) => path.processing.push("Core loudness applied".into()),
                    Some(false) => path.processing.push("Normalization pending measured LUFS".into()),
                    None => {}
                }
                path.bit_perfect_eligible=false;
            }
            if report.codec.as_deref()==Some("opus") {
                path.processing.push(format!("Core Opus transport: {} kbit/s; lossy; decoder clock 48000 Hz",report.bitrate));path.bit_perfect_eligible=false;
            }else if report.codec.as_deref()==Some("flac") {
                path.processing.push(format!("Core FLAC transport: {} Hz / {} bit / {} ch",report.rate,report.bits,report.channels));path.bit_perfect_eligible=false;
            }
            if report.rate==path.sample_rate&&report.channels==path.channels {
                path.processing.retain(|p|!p.starts_with("FFmpeg rate conversion")&&!p.starts_with("FFmpeg channel conversion"));
                if report.codec.is_none(){path.processing.push(format!("Core PCM conversion: {} Hz / {} bit / {} ch",report.rate,report.bits,report.channels));}
                path.bit_perfect_eligible=false;
            }
        }}
    }
    pub async fn remote_stream(profile:&ConnectionProfile,source:&str,policy:StreamingPolicy,info:serde_json::Value)->Result<Self> {
        let metadata=policy.metadata(&info)?;
        Self::start_inner(profile,source,None,None,None,None,Some(remote::RemoteSpec{policy,metadata,source:info})).await
    }
    pub fn prepared_metadata(&self)->Option<rmusic_audio::probe::AudioMetadata>{self.remote.as_ref().map(|r|r.metadata.clone())}
    pub fn source_failed(&self)->bool {self.source_failed.load(Ordering::Acquire)}
    pub(crate) fn serves_local_file(&self) -> bool {
        self.preparation.as_ref().is_none_or(|state| state.local.is_some())
    }
    pub fn finish_playback(&self,result:Result<()>,position:f64,duration:Option<f64>)->Result<()> {
        let interrupted=self.preparation.as_ref().is_some_and(|state|state.network_interrupted.load(Ordering::Acquire));
        if interrupted&&duration.is_none_or(|duration|position+0.000001<duration)
            &&(result.is_ok()||result.as_ref().is_err_and(|error|error.is::<rmusic_audio::decoder::DecodeFailure>())) {
            return Err(rmusic_audio::output::SourceUnavailable("音源网络中断，已保留播放位置".into()).into());
        }
        if result.is_ok()&&self.source_failed()&&duration.is_none_or(|duration|position+0.000001<duration){
            return Err(rmusic_audio::decoder::DecodeFailure("音源传输中断，尚未到达曲目结尾".into()).into());
        }
        result
    }
}
impl Drop for MediaProxy {
    fn drop(&mut self) {
        if self.remote.is_some()&&std::env::var_os("RMUSIC_AUDIO_DIAGNOSTICS").is_some(){eprintln!("Remote FLAC proxy released");}
        self.cancelled.send_replace(true);
        self.task.abort();
    }
}

async fn cancelled(mut receiver: tokio::sync::watch::Receiver<bool>) {
    if !*receiver.borrow_and_update() { let _ = receiver.changed().await; }
}

async fn proxy(State(state): State<ProxyState>,Query(query):Query<conversion::Request>,method:Method,headers:HeaderMap)->Response {
    if method==Method::GET&&(!headers.contains_key(header::RANGE)||headers.get(header::RANGE).is_some_and(|v|v=="bytes=0-"))&&!headers.contains_key(header::IF_RANGE) {
        let mut warm=state.warm.lock().await;
        if warm.as_ref().is_some_and(|(prepared,_)|prepared==&query) {
            return warm.take().unwrap().1;
        }
    }
    fetch(state,query,method,headers).await
}
async fn fetch(state:ProxyState,query:conversion::Request,method:Method,headers:HeaderMap)->Response {
    let network_buffer=state.network_buffer.read().ok().and_then(|value|value.clone());
    let conversion=match state.remote.as_ref().map(|r|r.target(&state.source,&query).map(Some)).unwrap_or_else(||query.target(&state.source)){Ok(value)=>value,Err(message)=>return Response::builder().status(StatusCode::BAD_REQUEST).body(Body::from(message)).unwrap()};
    if conversion.is_none(){if let Some(path)=state.local.as_ref(){return conversion::file(path,&headers,&method,state.cancelled.clone()).await;}}
    let mut request = if network_buffer.is_some(){&state.network_client}else{&state.client}.request(method.clone(), conversion.as_ref().map_or(state.source.as_str(),|(url,_)|url.as_str()));
    for name in [header::RANGE, header::IF_RANGE, header::ACCEPT_ENCODING] {
        if conversion.is_none(){if let Some(value) = headers.get(&name) {
            request = request.header(name, value);
        }}
    }
    let result = tokio::select! {
        biased;
        _ = cancelled(state.cancelled.clone()) => return Response::builder().status(StatusCode::SERVICE_UNAVAILABLE).body(Body::empty()).unwrap(),
        _ = tokio::time::sleep(std::time::Duration::from_secs(30)),if state.remote.is_some()||network_buffer.is_some()=>{
            state.source_failed.store(true,Ordering::Release);
            state.network_interrupted.store(true,Ordering::Release);
            return Response::builder().status(StatusCode::GATEWAY_TIMEOUT).body(Body::from("Core media headers timed out")).unwrap();
        },
        result = request.send() => result,
    };
    let upstream = match result {
        Ok(response) => response,
        Err(error) => {
            if error.is_connect()||error.is_timeout() {state.network_interrupted.store(true,Ordering::Release);}
            eprintln!("Core media connection failed: {:?}",error.without_url());
            state.source_failed.store(true,Ordering::Release);
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from("core media connection failed"))
                .unwrap()
        }
    };
    if upstream.status().is_server_error()||matches!(upstream.status().as_u16(),408|429){state.source_failed.store(true,Ordering::Release);}
    if matches!(upstream.status().as_u16(),408|502|503|504){state.network_interrupted.store(true,Ordering::Release);}
    if conversion.is_none() && upstream.status().is_success() && state.entity.as_deref().is_some_and(|expected| upstream.headers().get(header::ETAG).and_then(|v|v.to_str().ok())!=Some(expected) || upstream.headers().get(header::CONTENT_ENCODING).is_some_and(|v|v!="identity")) {
        state.source_failed.store(true,Ordering::Release);
        return Response::builder().status(StatusCode::BAD_GATEWAY).body(Body::from("media entity changed; retry playback")).unwrap();
    }
    if let Some((_,expected))=conversion.as_ref(){
        if upstream.status().is_success(){
            let confirmation=if state.remote.is_some(){format!("{};container=ogg",expected.header())}else{expected.header()};
            if upstream.headers().get(if state.remote.is_some(){"X-RMusic-Native-FLAC"}else{expected.header_name()}).and_then(|v|v.to_str().ok())!=Some(confirmation.as_str()) {
                state.source_failed.store(true,Ordering::Release);
                return Response::builder().status(StatusCode::BAD_GATEWAY).body(Body::from("Core did not confirm the requested PCM conversion")).unwrap();
            }
            let core_pcm = MediaProxy::core_pcm(&state.source).filter(|_| expected.codec.is_none());
            if core_pcm.is_some() && upstream.headers().get(header::CONTENT_TYPE)
                .is_none_or(|value| value != "application/octet-stream") {
                state.source_failed.store(true,Ordering::Release);
                return Response::builder().status(StatusCode::BAD_GATEWAY).body(Body::from("Core did not return raw PCM")).unwrap();
            }
            if core_pcm==Some(true) && upstream.headers().get("X-RMusic-Core-DSP").is_none_or(|value|value!="applied") {
                state.source_failed.store(true,Ordering::Release);
                return Response::builder().status(StatusCode::BAD_GATEWAY).body(Body::from("Core DSP was not applied")).unwrap();
            }
            let mut confirmed=expected.clone();
            confirmed.loudness=match upstream.headers().get("X-RMusic-Core-Loudness").and_then(|value|value.to_str().ok()) {
                Some("applied") => Some(true),
                Some("pending") => Some(false),
                _ => None,
            };
            *state.conversion.lock().unwrap()=Some(confirmed);
        }
    }
    if state.remote.is_some()&&std::env::var_os("RMUSIC_AUDIO_DIAGNOSTICS").is_some(){eprintln!("Remote FLAC upstream: {} length {:?}",upstream.status(),upstream.headers().get(header::CONTENT_LENGTH));}
    let converted=conversion.is_some();
    let mut response = Response::builder().status(upstream.status());
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_ENCODING,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::ETAG,
    ] {
        if let Some(value) = upstream.headers().get(&name) {
            response = response.header(name, value);
        }
    }
    if method == Method::HEAD {
        return response.body(Body::empty()).unwrap();
    }
    let range = upstream
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes "))
        .and_then(|v| v.split_once('/'));
    let (mut cursor, total) = match range {
        Some((segment, total)) => (
            segment
                .split('-')
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0),
            total.parse::<u64>().ok(),
        ),
        None => (0, upstream.content_length()),
    };
    let response_start = cursor;
    let successful = upstream.status().is_success();
    let observer=if successful&&!converted {state.observer.clone().or_else(||state.observer_factory.as_ref().and_then(|factory|factory(upstream.headers())))}else{None};
    let network_buffer=network_buffer.map(|buffer|{
        if let Some((_,target))=conversion.as_ref(){NetworkBuffer::for_audio(&serde_json::json!({"sample_rate":target.rate,"bit_depth":target.bits,"channels":target.channels,"bitrate":target.bitrate*1000}),buffer.seconds,buffer.max_bytes).with_preroll(buffer.preroll_seconds())}else{buffer.entity(total)}
    });
    let fill = state.fill.get().filter(|_| successful && open_ended_request(&headers));
    let capacity=state.remote.as_ref().map(|r|if fill.is_some(){r.capacity_for(5)}else{r.capacity()}).or_else(||network_buffer.as_ref().map(|b|b.capacity));
    let producer_cancel=state.cancelled.clone();
    let source_failed=state.source_failed.clone();
    let prime_bytes=state.remote.as_ref().map(|r|r.priming_bytes()).or_else(||network_buffer.as_ref().map(|b|b.prime_bytes)).unwrap_or(0);
    let original=conversion.is_none()&&network_buffer.is_some();
    let from_start=successful&&original&&cursor==0&&total.is_some_and(|n|n>0)
        && upstream.headers().get(header::CONTENT_ENCODING).is_none_or(|v|v=="identity");
    let downloaded=state.downloaded.clone();
    let body:BoxStream<'static,Result<Bytes,std::io::Error>>=if successful&&conversion.is_none() {retry::body(upstream,state.network_client.clone(),state.source.clone(),state.cancelled.clone())}
        else{upstream.bytes_stream().map(|v|v.map_err(|e|std::io::Error::other(e.without_url()))).boxed()};
    let transfer_failed=state.source_failed.clone();
    let network_interrupted=state.network_interrupted.clone();
    let converted_from_start=fill.is_some()&&converted&&query.start.unwrap_or_default()==0.;
    let converted_to_end=state.converted_to_end.clone();
    let converted_ms=state.converted_ms.clone();
    let start = query.start.unwrap_or_default();
    let mut ogg = conversion.as_ref().filter(|(_,target)| {
        converted_from_start && target.rate > 0 && matches!(target.codec.as_deref(),Some("flac"|"opus"))
    }).map(|(_,target)|(ogg_progress::Progress::default(),u64::from(target.rate)));
    let granule_clock = ogg.as_ref().map(|_| converted_ms.clone());
    let finished_transfer=transfer_failed.clone();
    let finish_cancel=state.cancelled.clone();
    let body = body.map(move |chunk| {
        if let Err(error)=&chunk {
            transfer_failed.store(true,Ordering::Release);
            if matches!(error.kind(),std::io::ErrorKind::TimedOut|std::io::ErrorKind::ConnectionReset|std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::BrokenPipe)
                ||error.get_ref().and_then(|e|e.downcast_ref::<reqwest::Error>()).is_some_and(|e|e.is_body()||e.is_timeout()) {
                network_interrupted.store(true,Ordering::Release);
            }
        }
        if successful {
            if let Ok(bytes) = &chunk {
                if let Some((progress, rate)) = ogg.as_mut() {
                    let milliseconds = progress.push(bytes).saturating_mul(1000) / *rate;
                    if milliseconds > 0 { converted_ms.fetch_max(milliseconds, Ordering::Release); }
                }
                if let Some(observer) = observer.as_ref() {
                    observer(cursor, total, bytes);
                }
                cursor += bytes.len() as u64;
                if let Some(total)=total.filter(|_|from_start) {
                    if let Ok(mut progress)=downloaded.lock() {
                        if progress.is_none_or(|(end,old_total)| old_total!=total||cursor>end) {
                            *progress=Some((cursor,total));
                        }
                    }
                }
            }
        }
        chunk
    }).chain(futures_util::stream::once(async move {
        if converted_from_start && !*finish_cancel.borrow() && !finished_transfer.load(Ordering::Acquire) {
            converted_to_end.store(true,Ordering::Release);
        }
    }).filter_map(|_|std::future::ready(None))).take_until(cancelled(state.cancelled.clone())).boxed();
    let body=if let Some(fill)=fill {
        let bytes_per_second=network_buffer.as_ref().map_or((prime_bytes as u64 / 5).max(1),NetworkBuffer::bytes_per_second);
        let start = if converted { start } else { response_start as f64 / bytes_per_second as f64 };
        let window=spool::Window::new(fill.gate.subscribe(),bytes_per_second,start,granule_clock);
        spool::fill(body,window,state.cancelled.clone(),fill.directory.clone(),spool::MAX_BYTES)
    }else{body};
    if let Some(capacity)=capacity {
        let wait=if state.remote.is_some()||network_buffer.as_ref().is_some_and(|buffer|buffer.preroll_seconds()>2){5}else{3};
        let buffered=remote::buffered(body,capacity,prime_bytes,std::time::Duration::from_secs(wait),producer_cancel,source_failed,fill.is_some()).await;
        return response.header(header::CACHE_CONTROL,"no-store").body(buffered).unwrap();
    }
    response.body(Body::from_stream(body)).unwrap()
}

fn open_ended_request(headers:&HeaderMap)->bool {
    headers.get(header::RANGE).is_none_or(|range| range.to_str().ok()
        .and_then(|value|value.strip_prefix("bytes="))
        .and_then(|value|value.strip_suffix('-'))
        .is_some_and(|start|!start.is_empty()&&start.bytes().all(|b|b.is_ascii_digit())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[tokio::test]
    async fn cache_observer_rejects_changed_entity() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let server=tokio::spawn(async move {let(mut socket,_)=listener.accept().await.unwrap();let mut buffer=[0;4096];socket.read(&mut buffer).await.unwrap();socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nETag: \"new\"\r\nConnection: close\r\n\r\nabc").await.unwrap();});
        let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None};
        let observed=Arc::new(std::sync::atomic::AtomicU64::new(0));let captured=observed.clone();
        let proxy=MediaProxy::start_entity(&profile,&format!("http://{address}/api/v1/media/qa"),Some(Arc::new(move|_,_,bytes|{captured.fetch_add(bytes.len() as u64,Ordering::Relaxed);})),Some("\"old\"".into())).await.unwrap();
        let response=reqwest::get(&proxy.url).await.unwrap();assert_eq!(response.status(),StatusCode::BAD_GATEWAY);
        assert_eq!(observed.load(Ordering::Relaxed),0);assert!(proxy.source_failed());server.await.unwrap();
    }
    #[tokio::test]
    async fn actual_upstream_failures_are_distinct_from_authorization_and_cancellation(){
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        for (status,length,body,failed) in [("200 OK",8,"abc",true),("503 Service Unavailable",3,"bad",true),("401 Unauthorized",3,"bad",false),("200 OK",3,"abc",false)]{
            let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=server.local_addr().unwrap();
            let task=tokio::spawn(async move{let(mut socket,_)=server.accept().await.unwrap();let mut buffer=[0;4096];socket.read(&mut buffer).await.unwrap();socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}").as_bytes()).await.unwrap();});
            let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None};
            let proxy=MediaProxy::start(&profile,&format!("http://{address}/api/v1/media/id"),None).await.unwrap();
            if let Ok(response)=reqwest::get(&proxy.url).await{let _=response.bytes().await;}
            task.await.unwrap();assert_eq!(proxy.source_failed(),failed,"{status} length {length}");
        }
    }
    #[tokio::test]
    async fn truncated_real_http_decoder_cannot_be_reported_as_natural_eof(){
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        use rmusic_audio::decoder::{DecodeFailure,FfmpegDecoder,PcmEncoding};
        let mut wav=Vec::new();wav.extend(b"RIFF");wav.extend((36u32+192000).to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(16u32.to_le_bytes());wav.extend(1u16.to_le_bytes());wav.extend(1u16.to_le_bytes());wav.extend(48000u32.to_le_bytes());wav.extend(96000u32.to_le_bytes());wav.extend(2u16.to_le_bytes());wav.extend(16u16.to_le_bytes());wav.extend(b"data");wav.extend(192000u32.to_le_bytes());wav.resize(24044,0);
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let task=tokio::spawn(async move{let(mut socket,_)=listener.accept().await.unwrap();let mut buffer=[0;4096];socket.read(&mut buffer).await.unwrap();socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 192044\r\nConnection: close\r\n\r\n").await.unwrap();socket.write_all(&wav).await.unwrap();});
        let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None};
        let proxy=MediaProxy::start(&profile,&format!("http://{address}/api/v1/media/id"),None).await.unwrap();let url=proxy.url.clone();
        let (result,frames)=tokio::task::spawn_blocking(move||{let mut decoder=FfmpegDecoder::open(&url,0.,48000,1,PcmEncoding::Signed32).unwrap();let mut frames=0;let mut bytes=[0;4096];loop{match decoder.read_frames(&mut bytes,4){Ok(0)=>return (Ok(()),frames),Ok(count)=>frames+=count,Err(error)=>return (Err(error),frames)}}}).await.unwrap();
        task.await.unwrap();assert!(proxy.source_failed());assert!(frames<96000);
        let error=proxy.finish_playback(result,frames as f64/48000.,Some(2.)).unwrap_err();assert!(error.is::<DecodeFailure>());
        assert!(proxy.finish_playback(Ok(()),2.,Some(2.)).is_ok());
        assert!(proxy.finish_playback(Ok(()),1.99995,Some(2.)).is_err());
        let device_error=proxy.finish_playback(Err(anyhow::anyhow!("DAC disconnected")),0.25,Some(2.)).unwrap_err();assert!(!device_error.is::<DecodeFailure>());
    }
    #[tokio::test]
    async fn dropping_proxy_cancels_upstream_headers_and_body_reads() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use std::time::Duration;
        for body_started in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let profile = ConnectionProfile {core_public_key:None,id:"test".into(),name:"test".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None};
            let observed = Arc::new(tokio::sync::Notify::new());let notify = observed.clone();
            let proxy = MediaProxy::start(&profile,&format!("http://{address}/api/v1/media/slow"),Some(Arc::new(move|_,_,_|notify.notify_one()))).await.unwrap();
            let url = proxy.url.clone();
            let request = tokio::spawn(async move {reqwest::Client::new().get(url).send().await?.bytes().await});
            let (mut upstream,_) = tokio::time::timeout(Duration::from_secs(5),listener.accept()).await.unwrap().unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {headers.push(upstream.read_u8().await.unwrap());}
            if body_started {
                upstream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\nabc").await.unwrap();
                tokio::time::timeout(Duration::from_secs(1),observed.notified()).await.unwrap();
            }
            drop(proxy);
            let closed = tokio::time::timeout(Duration::from_secs(1),async {let mut byte=[0];matches!(upstream.read(&mut byte).await,Ok(0)|Err(_))}).await.unwrap_or(false);
            request.abort();drop(upstream);
            assert!(closed,"dropping the proxy must close upstream while body_started={body_started}");
        }
    }
    #[tokio::test]
    async fn keeps_range_ticket_and_records_only_read_bytes() {
        async fn source(method: Method, headers: HeaderMap) -> Response {
            assert_eq!(headers.get(header::RANGE).unwrap(), "bytes=3-5");
            assert_eq!(headers.get(header::IF_RANGE).unwrap(), "\"fixture\"");
            assert_eq!(headers.get(header::ACCEPT_ENCODING).unwrap(), "identity");
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_LENGTH, "3")
                .header(header::CONTENT_RANGE, "bytes 3-5/10")
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::ETAG, "\"fixture\"")
                .header(header::CONTENT_ENCODING, "identity")
                .body(if method == Method::HEAD {
                    Body::empty()
                } else {
                    Body::from("345")
                })
                .unwrap()
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/prefix/api/v1/media/test", get(source).head(source)),
            )
            .await
            .unwrap();
        });
        let profile = ConnectionProfile {core_public_key:None,
            id: "test".into(),
            name: "test".into(),
            address: format!("http://{address}/prefix"),
            skip_certificate_verification: false, certificate_sha256: None, alternatives: vec![], core_id: None,
        };
        let writes = Arc::new(Mutex::new(Vec::new()));
        let observed = writes.clone();
        let proxy = MediaProxy::start(
            &profile,
            &format!("http://{address}/prefix/api/v1/media/test?ticket=test"),
            Some(Arc::new(move |offset, total, bytes| {
                observed
                    .lock()
                    .unwrap()
                    .push((offset, total, bytes.to_vec()))
            })),
        )
        .await
        .unwrap();
        let response = reqwest::Client::new()
            .get(&proxy.url)
            .header(header::RANGE, "bytes=3-5")
            .header(header::IF_RANGE, "\"fixture\"")
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 3-5/10");
        assert_eq!(response.headers()[header::ETAG], "\"fixture\"");
        assert_eq!(response.headers()[header::CONTENT_ENCODING], "identity");
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"345");
        assert_eq!(
            *writes.lock().unwrap(),
            vec![(3, Some(10), b"345".to_vec())]
        );
        server.abort();
    }
}
