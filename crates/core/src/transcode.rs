//! Bounded browser audio delivery. An ephemeral loopback proxy reuses the original media handler,
//! retaining Range support, source cache policy and authorization without bypassing remote TLS.
mod native;
use crate::{
    error::{ApiError, ApiResult},
    media::{self, TicketQuery},
    state::AppState,
};
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::Response,
    routing::get,
    Router,
};
use futures_util::{Future, Stream, StreamExt};
use serde::Deserialize;
use std::{
    io,
    pin::Pin,
    process::Stdio,
    sync::{Arc, OnceLock},
    task::{Context, Poll},
};
use tokio::{
    process::{Child, Command},
    sync::{OwnedSemaphorePermit, Semaphore},
};
use tokio_util::io::ReaderStream;

#[derive(Deserialize)]
pub struct TranscodeQuery {
    pub ticket: Option<String>,
    #[serde(default = "default_bitrate")]
    pub bitrate: u32,
    #[serde(default)]
    pub start: f64,
    pub pcm_rate: Option<u32>,
}
fn default_bitrate() -> u32 {
    128
}
#[derive(Deserialize)]
pub struct PcmQuery {
    pub ticket: Option<String>,
    pub rate: u32,
    #[serde(default)]
    pub start: f64,
}
#[derive(Clone, Copy)]
enum Encoding {
    Aac(u32),
    AacPcm {bitrate:u32,clock:u32},
    Pcm(u32),
    NativePcm {rate:u32,bits:u16,channels:u16,core_render:bool},
    NativeFlac {rate:u32,bits:u16,channels:u16},
    MobileOpus {bitrate:u32,channels:u16},
}
pub async fn pcm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PcmQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    if !(8000..=192000).contains(&query.rate) || !query.start.is_finite() || query.start < 0. {
        return Err(ApiError::bad(
            "PCM 采样率须为 8000..192000 Hz，起始位置须为非负秒数",
        ));
    }
    deliver(
        state,
        id,
        query.ticket,
        None,
        query.start,
        Encoding::Pcm(query.rate),
        headers,
    )
    .await
}
#[derive(Deserialize)]
pub struct NativePcmQuery {ticket:Option<String>,download_id:Option<String>,rate:u32,bits:u16,channels:u16,#[serde(default)]start:f64,#[serde(default)]core_render:bool}
pub async fn native_pcm(State(state):State<AppState>,Path(id):Path<String>,Query(q):Query<NativePcmQuery>,headers:HeaderMap)->ApiResult<Response>{
    if !(8000..=768000).contains(&q.rate)||![16,24,32].contains(&q.bits)||!(1..=2).contains(&q.channels)||!q.start.is_finite()||q.start<0. {return Err(ApiError::bad("原生 PCM 转换格式或起点无效"));}
    deliver(state,id,q.ticket,q.download_id,q.start,Encoding::NativePcm{rate:q.rate,bits:q.bits,channels:q.channels,core_render:q.core_render},headers).await
}
/// Native-client lossless codec transport. Converted bytes are never written to disk.
pub async fn native_flac(State(state):State<AppState>,Path(id):Path<String>,Query(q):Query<NativePcmQuery>,headers:HeaderMap)->ApiResult<Response>{
    if !(8000..=768000).contains(&q.rate)||![16,24,32].contains(&q.bits)||!(1..=2).contains(&q.channels)||!q.start.is_finite()||q.start<0. {return Err(ApiError::bad("外出 FLAC 流格式或起点无效"));}
    deliver(state,id,q.ticket,q.download_id,q.start,Encoding::NativeFlac{rate:q.rate,bits:q.bits,channels:q.channels},headers).await
}
#[derive(Deserialize)]
pub struct MobileOpusQuery {ticket:Option<String>,download_id:Option<String>,#[serde(default="default_mobile_bitrate")]bitrate:u32,#[serde(default="default_mobile_channels")]channels:u16,#[serde(default)]start:f64,#[serde(default)]accept_lossy:bool}
fn default_mobile_bitrate()->u32{160}
fn default_mobile_channels()->u16{2}
fn validate_mobile_opus(q:&MobileOpusQuery)->ApiResult<()> {
    if !q.accept_lossy || !(48..=320).contains(&q.bitrate) || !(1..=2).contains(&q.channels) || !q.start.is_finite() || q.start<0. {
        return Err(ApiError::bad("Opus为有损编码，须显式接受；码率48–320kbps，声道1–2，起点非负"));
    }Ok(())
}
pub async fn mobile_opus(State(state):State<AppState>,Path(id):Path<String>,Query(q):Query<MobileOpusQuery>,headers:HeaderMap)->ApiResult<Response>{
    validate_mobile_opus(&q)?;
    deliver(state,id,q.ticket,q.download_id,q.start,Encoding::MobileOpus{bitrate:q.bitrate,channels:q.channels},headers).await
}
fn validate(query: &TranscodeQuery) -> ApiResult<()> {
    if query.pcm_rate.is_some_and(|rate|!(8000..=192000).contains(&rate)){return Err(ApiError::bad("连续 AAC 输出时钟须为 8000..192000 Hz"));}
    if !(128..=256).contains(&query.bitrate) || !query.start.is_finite() || query.start < 0. {
        return Err(ApiError::bad(
            "AAC 码率须为 128..256 kbps，起始位置须为非负秒数",
        ));
    }
    Ok(())
}

pub async fn transcode(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<TranscodeQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    validate(&query)?;
    deliver(
        state,
        id,
        query.ticket,
        None,
        query.start,
        query.pcm_rate.map(|clock|Encoding::AacPcm{bitrate:query.bitrate,clock}).unwrap_or(Encoding::Aac(query.bitrate)),
        headers,
    )
    .await
}
async fn deliver(
    state: AppState,
    id: String,
    ticket: Option<String>,
    download_id: Option<String>,
    start: f64,
    encoding: Encoding,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let scope=download_id.as_ref().map(|id|format!("download:{id}")).unwrap_or_else(||format!("media:{id}"));
    let principal=media::authorize(
        &state,
        &headers,
        &TicketQuery { ticket: ticket.clone() },
        &scope,
    )
    .await?;
    let recipe=if matches!(encoding,Encoding::NativePcm{core_render:true,..}) {
        Some(crate::render_media::recipe(&state, ticket.as_deref(), &scope).await?)
    } else { None };
    let applied=recipe.as_ref().is_some_and(crate::render_media::Recipe::required);
    if matches!(encoding,Encoding::MobileOpus{..}) && state.setting("mobile_opus_enabled",serde_json::json!(false)).await!=true {return Err(ApiError(StatusCode::FORBIDDEN,"mobile_opus_disabled".into(),"Core尚未允许移动端Opus省流，请在Core设置中启用".into()));}
    if !matches!(encoding,Encoding::Pcm(_)|Encoding::NativePcm{..}|Encoding::NativeFlac{..}|Encoding::MobileOpus{..})&&state.setting("lossless_transport_only",serde_json::json!(true)).await!=serde_json::json!(false){return Err(ApiError(StatusCode::BAD_REQUEST,"lossy_transcode_disabled".into(),"无损传输策略已禁用 AAC 编码，请使用 PCM 或原生客户端".into()));}
    let normalization = recipe.as_ref().is_some_and(|recipe| recipe.dsp.enabled && recipe.dsp.normalize);
    let live_loudness = normalization.then(|| state.loudness.updates.subscribe());
    let mut track = crate::library::get_track(&state, id.clone()).await?;
    let loudness_status = normalization.then_some(())
        .map(|_| if track.integrated_lufs.is_some() { "applied" } else { "pending" });
    // Download consent belongs to its approved file, never to a higher-quality playback source.
    if let Some(download)=download_id.as_deref() {
        let path=crate::downloads::approved_path(&state,download,&id).await?;
        let audio=rmusic_audio::probe::probe_async(&path).await.map_err(|_|ApiError::bad("已批准下载文件无法读取，请重新下载"))?;
        track.format=audio.format;track.sample_rate=audio.sample_rate;track.bit_depth=audio.bit_depth as u32;
        track.channels=audio.channels as u32;track.duration=audio.duration.unwrap_or(track.duration);
    }
    if track.duration > 0. && start >= track.duration {
        return Err(ApiError::bad("起始位置超过歌曲长度"));
    }
    static LIMIT: OnceLock<Arc<Semaphore>> = OnceLock::new();
    static PCM_LIMIT: OnceLock<Arc<Semaphore>> = OnceLock::new();
    // Four browser zones can each decode the current source and one preloaded source.
    let (limit, count) = match encoding {
        Encoding::Aac(_) => (&LIMIT, 4),
        Encoding::Pcm(_) | Encoding::AacPcm{..} | Encoding::NativePcm{..} | Encoding::NativeFlac{..} | Encoding::MobileOpus{..} => (&PCM_LIMIT, 8),
    };
    let permit = limit
        .get_or_init(|| Arc::new(Semaphore::new(count)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "transcode_busy".into(),
                format!("已有 {count} 路音频转换，请稍后重试或使用原格式"),
            )
        })?;
    let internal_ticket = state.ticket_for(download_id.as_ref().map(|id|format!("download:{id}")).unwrap_or_else(||format!("media:{id}")), 86400,principal.map(|a|a.session_hash)).await;
    let path = format!("/source/{}", uuid::Uuid::new_v4().simple());
    let media_state=state.clone();
    let route = Router::new().route(
        &path,
        get(move |headers: HeaderMap, method: Method| {
            let state = media_state.clone();
            let id = id.clone();
            let ticket = internal_ticket.clone();
            let download = download_id.clone();
            async move {
                if let Some(download)=download {
                    return crate::downloads::file(State(state),Path(download),Query(TicketQuery{ticket:Some(ticket)}),headers,method).await;
                }
                media::media(
                    State(state),
                    Path(id),
                    Query(TicketQuery {
                        ticket: Some(ticket),
                    }),
                    headers,
                    method,
                )
                .await
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(anyhow::Error::from)?;
    let input = format!(
        "http://127.0.0.1:{}{path}",
        listener.local_addr().map_err(anyhow::Error::from)?.port()
    );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, route).await;
    });
    let exe = std::env::var_os("RMUSIC_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    let mut command = Command::new(exe);
    command.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-rw_timeout",
        "15000000",
    ]);
    let native_input=if let (Some(recipe),Encoding::NativePcm{rate,channels,..})=(recipe.filter(|recipe|recipe.required()),encoding) {
        match native::Input::render(&state,&input,&track,start,rate,channels,recipe,live_loudness).await {
            Ok(value)=>Some(value),
            Err(error)=>{server.abort();return Err(error);}
        }
    } else if matches!(encoding,Encoding::NativeFlac{..}|Encoding::MobileOpus{..})&&(track.format=="flac"||track.source!="local") {
        let (rate,channels)=match encoding{Encoding::NativeFlac{rate,channels,..}=>(rate,channels),Encoding::MobileOpus{channels,..}=>(48000,channels),_=>unreachable!()};
        match native::Input::open(&input,start,rate,channels).await {
            Ok(value)=>value,Err(_)=>{server.abort();return Err(ApiError::bad("FLAC 原生边界校验失败，未开始转换"));}
        }
    }else{None};
    let native_rate=match encoding{Encoding::NativePcm{rate,..}|Encoding::NativeFlac{rate,..}=>Some(rate),Encoding::MobileOpus{..}=>Some(48000),_=>None};
    let native_seek=if let Some(rate)=native_rate {
        let frame=(start*rate as f64).round();let seconds=(frame/rate as f64).floor();
        if seconds>0.{command.args(["-ss",&seconds.to_string()]);}
        Some(format!("aresample={rate}:filter_size=64:phase_shift=10:exact_rational=1,atrim=start_sample={}",(frame-seconds*rate as f64) as u64))
    }else{if start>0.{command.args(["-ss",&start.to_string()]);}None};
    if matches!(encoding, Encoding::Pcm(_) | Encoding::AacPcm{..} | Encoding::NativePcm{..} | Encoding::NativeFlac{..} | Encoding::MobileOpus{..}) {
        command.arg("-xerror");
    }
    let rate = if let Encoding::Pcm(rate) | Encoding::NativePcm{rate,..} | Encoding::NativeFlac{rate,..} = encoding {
        rate
    } else if matches!(encoding,Encoding::MobileOpus{..}) {48000
    } else if let Encoding::AacPcm{clock,..}=encoding {
        if clock>=44100&&clock%44100==0 {44100} else {[8000,11025,12000,16000,22050,24000,32000,44100,48000].into_iter().rev().find(|rate|*rate<=clock).unwrap()}
    } else if track.sample_rate != 0 && track.sample_rate % 44100 == 0 {
        44100
    } else {
        48000
    };
    if let Some(native_input)=&native_input {
        // The strict decoder already performed the exact seek, SRC and any Core effects.
        command=Command::new(std::env::var_os("RMUSIC_FFMPEG").unwrap_or_else(||"ffmpeg".into()));
        let channels=match encoding{Encoding::NativePcm{channels,..}|Encoding::NativeFlac{channels,..}|Encoding::MobileOpus{channels,..}=>channels,_=>unreachable!()};
        command.args(["-nostdin","-hide_banner","-loglevel","error","-xerror","-f",native_input.sample_format(),"-ar",&rate.to_string(),"-ac",&channels.to_string(),"-i","pipe:0"]).stdin(Stdio::piped());
    }else{command.args(["-i", &input]).stdin(Stdio::null());}
    command.args(["-map", "0:a:0", "-vn", "-sn", "-dn"]);
    let (content_type, description) = match encoding {
        Encoding::MobileOpus{bitrate,channels} => {
            if native_input.is_none(){if let Some(filter)=native_seek.as_deref(){command.args(["-af",filter]);}}
            command.args(["-c:a","libopus","-b:a",&format!("{bitrate}k"),"-vbr","on","-application","audio","-compression_level","8","-frame_duration","20","-ac",&channels.to_string(),"-ar","48000","-threads","1","-page_duration","200000","-f","ogg","pipe:1"]);
            ("audio/ogg",format!("opus; container=ogg; rate=48000; channels={channels}; target_bitrate={bitrate}000; lossless=false"))
        }
        Encoding::Aac(bitrate) | Encoding::AacPcm{bitrate,..} => {
            command.args([
                "-c:a",
                "aac",
                "-profile:a",
                "aac_low",
                "-b:a",
                &format!("{bitrate}k"),
                "-ac",
                "2",
                "-ar",
                &rate.to_string(),
                "-threads",
                "1",
                "-movflags",
                if matches!(encoding,Encoding::AacPcm{..}){"frag_keyframe+delay_moov+default_base_moof+skip_trailer"}else{"frag_keyframe+empty_moov+default_base_moof"},
                "-frag_duration",
                "500000",
                "-f",
                "mp4",
                "pipe:1",
            ]);
            (
                "audio/mp4",
                format!("aac-lc; bitrate={bitrate}; rate={rate}; channels=2"),
            )
        }
        Encoding::NativeFlac{bits,channels,..} => {
            if native_input.is_none(){if let Some(filter)=native_seek.as_deref(){command.args(["-af",filter]);}}
            command.args(["-c:a","flac","-compression_level","3","-strict",if bits==32{"-2"}else{"0"},"-sample_fmt",if bits==16{"s16"}else{"s32"},"-bits_per_raw_sample",&bits.to_string(),"-ac",&channels.to_string(),"-ar",&rate.to_string(),"-threads","1","-page_duration","200000","-f","ogg","pipe:1"]);
            ("audio/ogg",format!("flac; container=ogg; rate={rate}; bits={bits}; channels={channels}"))
        }
        Encoding::NativePcm{bits,channels,core_render,..} => {
            if native_input.is_none(){if let Some(filter)=native_seek.as_deref(){command.args(["-af",filter]);}}
            let (codec,pcm)=match bits {16=>("pcm_s16le","s16le"),24=>("pcm_s24le","s24le"),_=>("pcm_s32le","s32le")};
            let (content_type,muxer)=if core_render {("application/octet-stream",pcm)} else {("audio/wav","wav")};
            command.args(["-c:a",codec,"-ac",&channels.to_string(),"-ar",&rate.to_string(),"-threads","1","-f",muxer,"pipe:1"]);
            (content_type,format!("{codec}; rate={rate}; bits={bits}; channels={channels}"))
        }
        Encoding::Pcm(_) => {
            command.args([
                "-c:a",
                "pcm_f32le",
                "-ac",
                "2",
                "-ar",
                &rate.to_string(),
                "-threads",
                "1",
                "-f",
                "f32le",
                "pipe:1",
            ]);
            (
                "application/octet-stream",
                format!("f32le; rate={rate}; channels=2"),
            )
        }
    };
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            server.abort();
            return Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "ffmpeg_unavailable".into(),
                "未找到 FFmpeg，暂不能提供音频转换".into(),
            ));
        }
    };
    let decode_abort=native_input.as_ref().map(|input|input.abort.clone());
    let feeder=native_input.map(|input|input.feed(child.stdin.take().expect("native encoder stdin")));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::bad("FFmpeg 没有输出管道"))?;
    let mut stream = TranscodedStream {
        feeder,decode_abort,
        reader: ReaderStream::with_capacity(stdout, 16 * 1024),
        child: Some(child),
        server,
        _permit: permit,
        ended: false,
        waiting: None,
    };
    let first=tokio::time::timeout(std::time::Duration::from_secs(20),async {
        let mut preface=Vec::new();
        loop {
            let bytes=stream.next().await.ok_or_else(||io::Error::other("converter ended before its identification"))??;
            if let Encoding::NativeFlac{rate,bits,channels}=encoding {
                if preface.len()+bytes.len()>65536{return Err(io::Error::other("FLAC identification exceeds limit"));}
                preface.extend_from_slice(&bytes);
                if !native::confirmed_header(&preface,rate,bits,channels).map_err(io::Error::other)?{continue;}
                return Ok(Bytes::from(preface));
            }
            if let Encoding::MobileOpus{channels,..}=encoding {
                if preface.len()+bytes.len()>65536{return Err(io::Error::other("Opus identification exceeds limit"));}
                preface.extend_from_slice(&bytes);
                if !native::confirmed_opus_header(&preface,channels).map_err(io::Error::other)? {continue;}
                return Ok(Bytes::from(preface));
            }
            return Ok(bytes);
        }
    }).await;
    let first=match first {
        Ok(Ok(bytes))=>bytes,
        _=>return Err(ApiError(StatusCode::BAD_GATEWAY,"transcode_failed".into(),"音源转换失败，或编码器不支持请求的实际规格；请降低位深或更新 FFmpeg".into()))
    };
    let stream = futures_util::stream::once(async move { Ok::<_, io::Error>(first) }).chain(stream);
    let mut response=Response::builder();
    if let Encoding::NativePcm{rate,bits,channels,..}=encoding {response=response.header("X-RMusic-Native-PCM",format!("v1;rate={rate};bits={bits};channels={channels}"));}
    if applied { response=response.header("X-RMusic-Core-DSP","applied"); }
    if let Some(status)=loudness_status { response=response.header("X-RMusic-Core-Loudness",status); }
    if let Encoding::NativeFlac{rate,bits,channels}=encoding {response=response.header("X-RMusic-Native-FLAC",format!("v1;rate={rate};bits={bits};channels={channels};container=ogg")).header("X-Accel-Buffering","no");}
    if let Encoding::MobileOpus{bitrate,channels}=encoding {response=response.header("X-RMusic-Mobile-Opus",format!("v1;rate=48000;channels={channels};target_bitrate={bitrate}000;lossless=false")).header("X-Accel-Buffering","no");}
    if let Encoding::AacPcm{clock,..}=encoding {response=response.header("X-RMusic-Continuous",format!("aac-mp4-v1; output_rate={clock}"));}
    response
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header(
            if matches!(encoding, Encoding::Pcm(_)) {
                "X-RMusic-PCM"
            } else {
                "X-RMusic-Transcoded"
            },
            description,
        )
        .header("X-RMusic-Source", track.format)
        .header("X-RMusic-Start", start.to_string())
        .body(Body::from_stream(stream))
        .map_err(anyhow::Error::from)
        .map_err(ApiError::from)
}

struct TranscodedStream {
    feeder:Option<tokio::task::JoinHandle<io::Result<()>>>,
    decode_abort:Option<rmusic_audio::decoder::DecoderAbort>,
    reader: ReaderStream<tokio::process::ChildStdout>,
    child: Option<Child>,
    server: tokio::task::JoinHandle<()>,
    _permit: OwnedSemaphorePermit,
    ended: bool,
    waiting: Option<Pin<Box<dyn Future<Output = io::Result<std::process::ExitStatus>> + Send>>>,
}
impl Stream for TranscodedStream {
    type Item = io::Result<Bytes>;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.ended {
            return Poll::Ready(None);
        }
        if self.waiting.is_none() {
            match Pin::new(&mut self.reader).poll_next(context) {
                Poll::Ready(None) => {
                    let mut child = self
                        .child
                        .take()
                        .expect("transcoder child exists until stdout EOF");
                    let feeder=self.feeder.take();
                    self.waiting = Some(Box::pin(async move {
                        let status=child.wait().await?;
                        if let Some(feeder)=feeder {feeder.await.map_err(io::Error::other)??;}
                        Ok(status)
                    }));
                }
                result => return result,
            }
        }
        match self.waiting.as_mut().unwrap().as_mut().poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.ended = true;
                match result {
                    Ok(status) if status.success() => Poll::Ready(None),
                    Ok(_) => Poll::Ready(Some(Err(io::Error::other(
                        "audio converter exited unsuccessfully",
                    )))),
                    Err(error) => Poll::Ready(Some(Err(error))),
                }
            }
        }
    }
}
impl Drop for TranscodedStream {
    fn drop(&mut self) {
        if let Some(decoder)=&self.decode_abort {decoder.abort();}
        if let Some(feeder)=self.feeder.take(){tokio::spawn(async move{let _=feeder.await;});}
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
        self.server.abort();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn mobile_opus_requires_explicit_lossy_consent_and_bounded_parameters(){
        use super::*;
        let mut q=MobileOpusQuery{ticket:None,download_id:None,bitrate:160,channels:2,start:0.,accept_lossy:false};
        assert!(validate_mobile_opus(&q).is_err());q.accept_lossy=true;assert!(validate_mobile_opus(&q).is_ok());
        for rate in [0,47,321,u32::MAX]{q.bitrate=rate;assert!(validate_mobile_opus(&q).is_err());}
        q.bitrate=160;q.start=f64::NAN;assert!(validate_mobile_opus(&q).is_err());q.start=0.;q.channels=3;assert!(validate_mobile_opus(&q).is_err());
        let mut head=vec![0u8;47];head[..4].copy_from_slice(b"OggS");head[26]=1;head[27]=19;head[28..36].copy_from_slice(b"OpusHead");head[36]=1;head[37]=2;
        assert!(native::confirmed_opus_header(&head,2).unwrap());assert!(native::confirmed_opus_header(&head,1).is_err());
        assert!(!native::confirmed_opus_header(&head[..12],2).unwrap());
    }
    #[tokio::test]
    async fn mobile_opus_real_stream_obeys_permission_and_exact_seek_eof(){
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let(dir,state,ticket)=fixture(true).await;
        let router=Router::new().route("/mobile-opus/{id}",get(mobile_opus)).with_state(state.clone());
        let request=||axum::http::Request::builder().uri(format!("/mobile-opus/test?ticket={ticket}&accept_lossy=true&bitrate=160&start=0.2")).body(Body::empty()).unwrap();
        assert_eq!(router.clone().oneshot(request()).await.unwrap().status(),StatusCode::FORBIDDEN);
        state.db.run(|c|{c.execute("INSERT INTO settings(key,value)VALUES('mobile_opus_enabled','true')",[])?;Ok(())}).await.unwrap();
        let response=router.oneshot(request()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
        assert!(response.headers()["X-RMusic-Mobile-Opus"].to_str().unwrap().contains("lossless=false"));
        assert_eq!(response.headers()["cache-control"],"private, no-store");
        let bytes=response.into_body().collect().await.unwrap().to_bytes();
        let path=dir.path().join("mobile.ogg");std::fs::write(&path,bytes).unwrap();
        let decoded=std::process::Command::new(std::env::var_os("RMUSIC_FFMPEG").unwrap_or_else(||"ffmpeg".into())).args(["-v","error","-xerror","-i"]).arg(path).args(["-f","s32le","pipe:1"]).output().unwrap();
        assert!(decoded.status.success(),"{}",String::from_utf8_lossy(&decoded.stderr));assert_eq!(decoded.stdout.len(),38400*2*4);
    }
    #[tokio::test]
    async fn download_conversion_uses_only_approved_file_and_its_own_ticket() {
        use http_body_util::BodyExt;use tower::ServiceExt;
        let(dir,state,media_ticket)=fixture(true).await;
        let mut approved=std::fs::read(dir.path().join("sample.wav")).unwrap();
        for chunk in approved[44..].chunks_exact_mut(2){chunk.copy_from_slice(&1234i16.to_le_bytes());}
        let path=dir.path().join("downloads/approved.wav");std::fs::write(&path,&approved).unwrap();
        let dbpath=path.to_string_lossy().to_string();
        state.db.run(move|c|{c.execute("INSERT INTO downloads(id,track_id,status,path,size,created_at)VALUES('approved','test','complete',?1,192044,0)",[dbpath])?;Ok(())}).await.unwrap();
        let download_ticket=state.ticket("download:approved".into(),60).await;
        let router=Router::new().route("/native-flac/{id}",get(native_flac)).with_state(state.clone());
        let req=|ticket:&str,id:&str|axum::http::Request::builder().uri(format!("/native-flac/test?ticket={ticket}&download_id={id}&rate=48000&bits=16&channels=2")).body(Body::empty()).unwrap();
        assert_eq!(router.clone().oneshot(req(&media_ticket,"approved")).await.unwrap().status(),StatusCode::UNAUTHORIZED);
        assert!(crate::downloads::approved_path(&state,"approved","another-track").await.is_err());
        let response=router.clone().oneshot(req(&download_ticket,"approved")).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
        let bytes=response.into_body().collect().await.unwrap().to_bytes();let encoded=dir.path().join("approved.oga");std::fs::write(&encoded,bytes).unwrap();
        let decoded=std::process::Command::new("ffmpeg").args(["-v","error","-i"]).arg(encoded).args(["-f","s16le","pipe:1"]).output().unwrap();
        assert!(decoded.status.success());assert_eq!(decoded.stdout,&approved[44..],"must not export the different playback source");
        // A stale approval must fail, even though the original playback file remains available.
        std::fs::remove_file(path).unwrap();
        assert_eq!(router.oneshot(req(&download_ticket,"approved")).await.unwrap().status(),StatusCode::NOT_FOUND);
    }
    #[tokio::test]
    async fn native_flac_exact_boundary_survives_post_roll_but_rejects_corrupt_programme() {
        use super::*;use http_body_util::BodyExt;use tower::ServiceExt;
        for damaged in [false,true] {
            let(dir,state,ticket)=fixture(true).await;let original=dir.path().join("sample.wav");let flac=dir.path().join("native.flac");
            assert!(std::process::Command::new("ffmpeg").args(["-v","error","-i"]).arg(&original).args(["-c:a","flac"]).arg(&flac).status().unwrap().success());
            let mut bytes=std::fs::read(flac).unwrap();
            if damaged {bytes.truncate(bytes.len()/2);}else{bytes.extend(vec![0;2048]);}
            std::fs::write(&original,&bytes).unwrap();let size=bytes.len() as i64;
            state.db.run(move|c|{c.execute("UPDATE tracks SET format='flac',size=?1 WHERE id='test'",[size])?;Ok(())}).await.unwrap();
            let response=Router::new().route("/native-flac/{id}",get(native_flac)).with_state(state).oneshot(axum::http::Request::builder().uri(format!("/native-flac/test?ticket={ticket}&rate=48000&bits=16&channels=2")).body(Body::empty()).unwrap()).await.unwrap();
            if damaged {assert!(response.status()!=StatusCode::OK||response.into_body().collect().await.is_err());continue;}
            assert_eq!(response.status(),StatusCode::OK);let result=response.into_body().collect().await.unwrap().to_bytes();
            let encoded=dir.path().join("output.flac");std::fs::write(&encoded,result).unwrap();
            let decoded=std::process::Command::new("ffmpeg").args(["-v","error","-i"]).arg(encoded).args(["-f","s32le","pipe:1"]).output().unwrap();
            assert!(decoded.status.success());assert_eq!(decoded.stdout.len(),48000*2*4);
        }
    }
    #[tokio::test]
    async fn native_flac_32bit_preserves_nonzero_low_order_samples() {
        use super::*;use http_body_util::BodyExt;use tower::ServiceExt;
        let(dir,state,ticket)=fixture(true).await;
        let samples:Vec<u8>=(0..96000).flat_map(|i|(i as i32).wrapping_mul(123457).to_le_bytes()).collect();
        let size=samples.len() as u32;let mut wav=Vec::new();
        wav.extend(b"RIFF");wav.extend((size+36).to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(16u32.to_le_bytes());wav.extend(1u16.to_le_bytes());wav.extend(2u16.to_le_bytes());wav.extend(48000u32.to_le_bytes());wav.extend(384000u32.to_le_bytes());wav.extend(8u16.to_le_bytes());wav.extend(32u16.to_le_bytes());wav.extend(b"data");wav.extend(size.to_le_bytes());wav.extend(&samples);
        std::fs::write(dir.path().join("sample.wav"),&wav).unwrap();
        state.db.run(move|c|{c.execute("UPDATE tracks SET bit_depth=32,size=?1 WHERE id='test'",[size as i64+44])?;Ok(())}).await.unwrap();
        let response=Router::new().route("/native-flac/{id}",get(native_flac)).with_state(state).oneshot(axum::http::Request::builder().uri(format!("/native-flac/test?ticket={ticket}&rate=48000&bits=32&channels=2")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);let bytes=response.into_body().collect().await.unwrap().to_bytes();
        let encoded=dir.path().join("actual-32.oga");std::fs::write(&encoded,&bytes).unwrap();
        let decoded=std::process::Command::new("ffmpeg").args(["-v","error","-i"]).arg(encoded).args(["-f","s32le","pipe:1"]).output().unwrap();
        assert!(decoded.status.success());assert_eq!(decoded.stdout,samples,"32-bit stream must preserve its low eight bits too");
    }
    #[tokio::test]
    async fn native_flac_stream_preserves_pcm_and_does_not_create_transcode_files() {
        use super::*;use http_body_util::BodyExt;use tower::ServiceExt;
        let(dir,state,ticket)=fixture(true).await;
        let original=dir.path().join("sample.wav");let mut wav=std::fs::read(&original).unwrap();
        for (i,bytes) in wav[44..].chunks_exact_mut(2).enumerate(){bytes.copy_from_slice(&((i as i16).wrapping_mul(31)).to_le_bytes());}
        std::fs::write(&original,wav).unwrap();
        let reference=std::process::Command::new("ffmpeg").args(["-v","error","-ss","0.25","-i"]).arg(&original).args(["-f","s32le","pipe:1"]).output().unwrap();assert!(reference.status.success());
        let router=Router::new().route("/native-flac/{id}",get(native_flac)).with_state(state);
        for bits in [16,24,32] {
            let response=router.clone().oneshot(axum::http::Request::builder().uri(format!("/native-flac/test?ticket={ticket}&rate=48000&bits={bits}&channels=2&start=0.25")).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(),StatusCode::OK);
            assert_eq!(response.headers()["X-RMusic-Native-FLAC"],format!("v1;rate=48000;bits={bits};channels=2;container=ogg"));
            assert_eq!(response.headers()["Cache-Control"],"private, no-store");
            let bytes=response.into_body().collect().await.unwrap().to_bytes();assert!(bytes.starts_with(b"OggS"));
            assert!(native::confirmed_header(&bytes[..bytes.len().min(32768)],48000,bits,2).unwrap());
            assert!(native::confirmed_header(&bytes[..bytes.len().min(32768)],48000,if bits==16{24}else{16},2).is_err());
            let path=dir.path().join(format!("verified-{bits}.flac"));std::fs::write(&path,bytes).unwrap();
            let decoded=std::process::Command::new("ffmpeg").args(["-v","error","-i"]).arg(path).args(["-f","s32le","pipe:1"]).output().unwrap();
            assert!(decoded.status.success());assert_eq!(decoded.stdout.len(),36000*2*4);assert_eq!(decoded.stdout,reference.stdout,"FLAC transport must preserve every nonzero PCM sample");
        }
        assert_eq!(std::fs::read_dir(dir.path().join("cache")).unwrap().count(),0,"converted stream must not create cache files");
    }
    use super::*;
    #[tokio::test]
    async fn bridge_core_render_applies_eq_fir_leveling_fades_and_rate_at_the_requested_cursor() {
        use http_body_util::BodyExt;
        use rmusic_audio::dsp::{DspConfig, EqBand, EqFilter};
        use serde_json::json;
        use sha2::{Digest, Sha256};
        use tower::ServiceExt;

        let (dir,state,plain)=fixture(true).await;
        let path=dir.path().join("sample.wav");
        let mut wav=std::fs::read(&path).unwrap();
        for sample in wav[44..].chunks_exact_mut(2) { sample.copy_from_slice(&4096i16.to_le_bytes()); }
        std::fs::write(&path,wav).unwrap();
        state.db.run(|db| {
            db.execute("UPDATE tracks SET provider_data = ? WHERE id='test'", [
                json!({"integrated_lufs":-24.0,"analysis":{"loudness":{"true_peak_dbfs":-18.0}}}).to_string(),
            ])?;
            Ok(())
        }).await.unwrap();

        let mut impulse=b"RIFF".to_vec();
        impulse.extend_from_slice(&44u32.to_le_bytes());
        impulse.extend_from_slice(b"WAVEfmt ");
        impulse.extend_from_slice(&16u32.to_le_bytes());
        impulse.extend_from_slice(&3u16.to_le_bytes());
        impulse.extend_from_slice(&1u16.to_le_bytes());
        impulse.extend_from_slice(&96000u32.to_le_bytes());
        impulse.extend_from_slice(&768000u32.to_le_bytes());
        impulse.extend_from_slice(&8u16.to_le_bytes());
        impulse.extend_from_slice(&64u16.to_le_bytes());
        impulse.extend_from_slice(b"data");
        impulse.extend_from_slice(&8u32.to_le_bytes());
        impulse.extend_from_slice(&0.5f64.to_le_bytes());
        let id=format!("{:x}",Sha256::digest(&impulse));
        let assets=dir.path().join("dsp/impulses");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join(format!("{id}.wav")),&impulse).unwrap();
        std::fs::write(assets.join(format!("{id}.json")),json!({
            "id":id,"name":"scale.wav","bytes":impulse.len(),"sample_rate":96000,"channels":1,"frames":1,"channel_mask":0,"created_at":0,
        }).to_string()).unwrap();

        let settings=DspConfig {
            enabled:true, normalize:true, target_lufs:-18., sample_rate:Some(96000),
            bands:vec![EqBand { enabled:true, kind:EqFilter::LowShelf, frequency:400., gain_db:6., q:0.707, slope_db_oct: None }],
            convolution:Some(rmusic_audio::impulse::ConvolutionConfig { response_ids:vec![id] }),
            ..Default::default()
        };
        let track=crate::library::get_track(&state,"test".into()).await.unwrap();
        let mut zone=crate::models::Zone::new("z".into(),"QA".into(),"bridge-test".into());
        zone.dsp=serde_json::to_value(settings).unwrap();
        zone.transitions.fade_in_ms=100; zone.transitions.fade_out_ms=200;
        state.endpoints.write().await.insert(zone.endpoint_id.clone(),json!({"capabilities":{"core_pcm_dsp_v1":true}}));
        let media=crate::render_media::path(&state,&zone,&track,None).await.unwrap();
        assert!(media.contains("core_pcm=render")&&media.contains("dsp_rate=96000"));
        let uri=url::Url::parse(&format!("http://localhost/{media}")).unwrap();
        let token=uri.query_pairs().find(|(key,_)|key=="ticket").unwrap().1.into_owned();

        let router=Router::new().route("/native-pcm/{id}",get(native_pcm)).with_state(state);
        let request=|ticket:&str,rate:u32,start:f64| axum::http::Request::builder()
            .uri(format!("/native-pcm/test?ticket={ticket}&core_render=true&rate={rate}&bits=32&channels=2&start={start}"))
            .body(Body::empty()).unwrap();
        assert_eq!(router.clone().oneshot(request(&plain,96000,0.)).await.unwrap().status(),StatusCode::FORBIDDEN);
        assert_eq!(router.clone().oneshot(request(&token,48000,0.)).await.unwrap().status(),StatusCode::BAD_REQUEST);
        for (index,start) in [0.,0.25].into_iter().enumerate() {
            let response=router.clone().oneshot(request(&token,96000,start)).await.unwrap();
            assert_eq!(response.status(),StatusCode::OK);
            assert_eq!(response.headers()["X-RMusic-Core-DSP"],"applied");
            assert_eq!(response.headers()["X-RMusic-Native-PCM"],"v1;rate=96000;bits=32;channels=2");
            assert_eq!(response.headers()[header::CONTENT_TYPE],"application/octet-stream");
            let output=response.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(output.len(),((1.-start)*96000.) as usize*8);
            let sample=|time:f64| {
                let offset=((time-start)*96000.).round() as usize * 8;
                i32::from_le_bytes(output[offset..offset+4].try_into().unwrap()) as f64 / 2147483648.
            };
            let expected=0.125*0.5*10_f64.powf(12./20.);
            assert!((sample(0.5)-expected).abs()<0.00005);
            assert!((sample(0.9)-expected*std::f64::consts::FRAC_1_SQRT_2).abs()<0.00005);
            if index==0 { assert!(sample(0.).abs()<0.00001); }
            else { assert!((sample(start)-expected).abs()<0.00005, "seek must not restart fade-in"); }
        }
    }

    #[tokio::test]
    async fn native_pcm_caps_authentication_header_and_actual_frames(){
        use http_body_util::BodyExt;use tower::ServiceExt;
        let(dir,state,ticket)=fixture(true).await;
        let router=Router::new().route("/native-pcm/{id}",get(native_pcm)).with_state(state);
        for (query,status) in [("rate=48000&bits=24&channels=2".to_string(),StatusCode::UNAUTHORIZED),(format!("ticket={ticket}&rate=768001&bits=24&channels=2"),StatusCode::BAD_REQUEST),(format!("ticket={ticket}&rate=48000&bits=1&channels=2"),StatusCode::BAD_REQUEST)]{
            assert_eq!(router.clone().oneshot(axum::http::Request::builder().uri(format!("/native-pcm/test?{query}")).body(Body::empty()).unwrap()).await.unwrap().status(),status);
        }
        let response=router.oneshot(axum::http::Request::builder().uri(format!("/native-pcm/test?ticket={ticket}&rate=44100&bits=24&channels=2&start=0.2")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);assert_eq!(response.headers()["X-RMusic-Native-PCM"],"v1;rate=44100;bits=24;channels=2");
        let bytes=response.into_body().collect().await.unwrap().to_bytes();let file=dir.path().join("native.wav");std::fs::write(&file,bytes).unwrap();
        let output=std::process::Command::new("ffmpeg").args(["-v","error","-i"]).arg(file).args(["-f","s32le","pipe:1"]).output().unwrap();assert!(output.status.success());assert_eq!(output.stdout.len(),35280*2*4);
    }
    #[tokio::test]
    async fn continuous_aac_preserves_encoder_delay_and_actual_programme_end() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let (directory,state,ticket)=fixture(true).await;
        let response=Router::new().route("/transcode/{id}",get(transcode)).with_state(state)
            .oneshot(axum::http::Request::builder().uri(format!("/transcode/test?ticket={ticket}&bitrate=128&start=0.2&pcm_rate=192000")).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        assert_eq!(response.headers().get("X-RMusic-Continuous").and_then(|v|v.to_str().ok()),Some("aac-mp4-v1; output_rate=192000"));
        let bytes=response.into_body().collect().await.unwrap().to_bytes();assert!(bytes.windows(4).any(|v|v==b"elst"));
        let path=directory.path().join("continuous.m4a");tokio::fs::write(&path,bytes).await.unwrap();
        let probe=std::process::Command::new(std::env::var_os("RMUSIC_FFPROBE").unwrap_or_else(||"ffprobe".into()))
            .args(["-v","error","-select_streams","a:0","-show_entries","packet=pts:stream=duration_ts","-of","json"]).arg(path).output().unwrap();
        assert!(probe.status.success());let value:serde_json::Value=serde_json::from_slice(&probe.stdout).unwrap();let packets=value["packets"].as_array().unwrap();
        assert_eq!(packets[0]["pts"],-1024);
        // FFprobe 5.1 reports 1024 for the final AAC packet even when trun contains 512.
        // The container duration retains that final packet duration and includes priming.
        assert_eq!(value["streams"][0]["duration_ts"].as_i64().unwrap()-1024,38400);
    }
    #[tokio::test]
    async fn continuous_aac_rejects_invalid_render_clocks() {
        use tower::ServiceExt;
        let (_directory,state,ticket)=fixture(true).await;
        let router=Router::new().route("/transcode/{id}",get(transcode)).with_state(state);
        for clock in [0,192001] {
            let response=router.clone().oneshot(axum::http::Request::builder().uri(format!("/transcode/test?ticket={ticket}&pcm_rate={clock}")).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(),StatusCode::BAD_REQUEST);
        }
    }
    #[test]
    fn refuses_invalid_bitrate_and_seek() {
        assert!(validate(&TranscodeQuery {
            ticket: None,
            bitrate: 128,
            start: 0.,
            pcm_rate:None,
        })
        .is_ok());
        for (bitrate, start) in [
            (0, 0.),
            (257, 0.),
            (128, -1.),
            (128, f64::NAN),
            (128, f64::INFINITY),
        ] {
            assert!(validate(&TranscodeQuery {
                ticket: None,
                bitrate,
                start,
                pcm_rate:None,
            })
            .is_err());
        }
    }
    async fn fixture(valid: bool) -> (tempfile::TempDir, AppState, String) {
        let directory = tempfile::tempdir().unwrap();
        for folder in ["cache", "covers", "downloads", "providers"] {
            tokio::fs::create_dir_all(directory.path().join(folder))
                .await
                .unwrap();
        }
        let audio = directory.path().join("sample.wav");
        let mut wav = Vec::new();
        let data = vec![0u8; 48000 * 2 * 2];
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&192000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        tokio::fs::write(
            &audio,
            if valid {
                wav
            } else {
                b"this is not audio".to_vec()
            },
        )
        .await
        .unwrap();
        let db = crate::db::Db::open(&directory.path().join("library.db")).unwrap();
        let path = audio.to_string_lossy().to_string();
        db.run(move |c| { c.execute("INSERT INTO tracks(id,path,title,artist,album,album_artist,source,format,sample_rate,channels,recording_id,created_at,duration,bit_depth) VALUES('test',?1,'Test','R-Music','Test','R-Music','local','wav',48000,2,'recording',0,1,16)",[path])?; Ok(()) }).await.unwrap();
        let state = AppState::new(
            db,
            crate::state::Config {
                data: directory.path().into(),
                web: directory.path().into(),
                base_path: "/music".into(),
                public_url: None,
                plugin_dir: directory.path().join("plugins"),
                python: if cfg!(windows){"python".into()}else{"python3".into()},
            },
            String::new(),
        )
        .await
        .unwrap();
        state.db.run(|c|{c.execute("INSERT INTO settings(key,value)VALUES('lossless_transport_only','false')",[])?;Ok(())}).await.unwrap();
        let ticket = state.ticket("media:test".into(), 60).await;
        (directory, state, ticket)
    }
    #[tokio::test]async fn lossless_policy_rejects_aac_before_starting_an_encoder(){use tower::ServiceExt;let(_dir,state,ticket)=fixture(true).await;state.db.run(|c|{c.execute("DELETE FROM settings WHERE key='lossless_transport_only'",[])?;Ok(())}).await.unwrap();let router=Router::new().route("/transcode/{id}",get(transcode)).with_state(state);let result=router.oneshot(axum::http::Request::builder().uri(format!("/transcode/test?ticket={ticket}")).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(result.status(),StatusCode::BAD_REQUEST);}
    #[tokio::test]
    async fn pcm_preserves_actual_stereo_samples_and_seeks_before_streaming() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let (directory, state, ticket) = fixture(true).await;
        let path = directory.path().join("sample.wav");
        let mut wav = tokio::fs::read(&path).await.unwrap();
        let pattern = [32767i16, -32768, 12345, -23456];
        let mut expected = Vec::new();
        for (i, sample) in wav[44..].chunks_exact_mut(2).enumerate() {
            let value = pattern[i % pattern.len()];
            sample.copy_from_slice(&value.to_le_bytes());
            if i >= 19200 {
                expected.extend_from_slice(&(value as f32 / 32768.).to_le_bytes());
            }
        }
        tokio::fs::write(path, wav).await.unwrap();
        let router = Router::new().route("/pcm/{id}", get(pcm)).with_state(state);
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/pcm/test?ticket={ticket}&rate=48000&start=0.2"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["X-RMusic-PCM"],
            "f32le; rate=48000; channels=2"
        );
        assert_eq!(response.headers()["X-RMusic-Source"], "wav");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            bytes.as_ref(),
            expected,
            "decode and source seek must preserve every actual PCM sample"
        );
    }
    #[tokio::test]
    async fn pcm_resamples_to_the_requested_browser_clock() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let (_directory, state, ticket) = fixture(true).await;
        let router = Router::new().route("/pcm/{id}", get(pcm)).with_state(state);
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/pcm/test?ticket={ticket}&rate=24000&start=0.2"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["X-RMusic-PCM"],
            "f32le; rate=24000; channels=2"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(bytes.len(), 19200 * 2 * 4);
        assert!(bytes.iter().all(|b| *b == 0));
    }
    #[tokio::test]
    async fn truncated_pcm_source_never_becomes_a_successful_natural_eof() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let (directory, state, ticket) = fixture(true).await;
        let path = directory.path().join("sample.wav");
        let mut wav = tokio::fs::read(&path).await.unwrap();
        wav.truncate(wav.len() - 123);
        tokio::fs::write(path, wav).await.unwrap();
        let response = Router::new()
            .route("/pcm/{id}", get(pcm))
            .with_state(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/pcm/test?rate=48000&ticket={ticket}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "fixture must reach actual streaming before the decode error"
        );
        let mut body = response.into_body();
        let mut received = 0;
        let mut failed = false;
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(frame) => {
                    if let Ok(bytes) = frame.into_data() {
                        received += bytes.len();
                    }
                }
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(received > 0);
        assert!(
            failed,
            "a decoder error after PCM must fail the body, never confirm EOF"
        );
    }
    #[tokio::test]
    async fn pcm_requires_media_authority_and_valid_bounded_clock_and_offset() {
        use tower::ServiceExt;
        let (_directory, state, ticket) = fixture(true).await;
        let wrong = state.ticket("media:another".into(), 60).await;
        let router = Router::new().route("/pcm/{id}", get(pcm)).with_state(state);
        for (query, status) in [
            ("rate=48000".into(), StatusCode::UNAUTHORIZED),
            (
                format!("rate=48000&ticket={wrong}"),
                StatusCode::UNAUTHORIZED,
            ),
            (format!("rate=0&ticket={ticket}"), StatusCode::BAD_REQUEST),
            (
                format!("rate=192001&ticket={ticket}"),
                StatusCode::BAD_REQUEST,
            ),
            (
                format!("rate=48000&start=-1&ticket={ticket}"),
                StatusCode::BAD_REQUEST,
            ),
            (
                format!("rate=48000&start=1&ticket={ticket}"),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let response = router
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri(format!("/pcm/test?{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{query}");
        }
        let (_directory, state, ticket) = fixture(false).await;
        let response = Router::new()
            .route("/pcm/{id}", get(pcm))
            .with_state(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/pcm/test?rate=48000&ticket={ticket}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }
    #[tokio::test]
    async fn real_wav_transcodes_through_media_handler_to_aac() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let (directory, state, ticket) = fixture(true).await;
        let router = Router::new()
            .route("/transcode/{id}", get(transcode))
            .with_state(state);
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/transcode/test?ticket={ticket}&bitrate=128&start=0.2"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["X-RMusic-Source"], "wav");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(bytes.windows(4).any(|v| v == b"moof"));
        let output = directory.path().join("result.m4a");
        tokio::fs::write(&output, bytes).await.unwrap();
        let probe = std::process::Command::new(
            std::env::var_os("RMUSIC_FFPROBE").unwrap_or_else(|| "ffprobe".into()),
        )
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name,sample_rate,channels",
            "-of",
            "json",
        ])
        .arg(output)
        .output()
        .unwrap();
        assert!(probe.status.success());
        let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(metadata["streams"][0]["codec_name"], "aac");
        assert_eq!(metadata["streams"][0]["sample_rate"], "48000");
        assert_eq!(metadata["streams"][0]["channels"], 2);
    }
    #[tokio::test]
    async fn invalid_audio_does_not_report_a_successful_complete_stream() {
        use tower::ServiceExt;
        let (_directory, state, ticket) = fixture(false).await;
        let router = Router::new()
            .route("/transcode/{id}", get(transcode))
            .with_state(state);
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/transcode/test?ticket={ticket}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }
    #[tokio::test]
    async fn nonzero_exit_after_audio_bytes_is_a_stream_error() {
        let (directory, _state, _ticket) = fixture(true).await;
        let playlist = directory.path().join("input.ffconcat");
        let name = |file: &str| {
            directory
                .path()
                .join(file)
                .to_string_lossy()
                .replace('\\', "/")
        };
        tokio::fs::write(
            &playlist,
            format!(
                "file '{}'\nfile '{}'\n",
                name("sample.wav"),
                name("missing.wav")
            ),
        )
        .await
        .unwrap();
        let mut child =
            Command::new(std::env::var_os("RMUSIC_FFMPEG").unwrap_or_else(|| "ffmpeg".into()))
                .args([
                    "-nostdin",
                    "-hide_banner",
                    "-v",
                    "error",
                    "-xerror",
                    "-f",
                    "concat",
                    "-safe",
                    "0",
                    "-i",
                ])
                .arg(playlist)
                .args(["-c:a", "aac", "-f", "adts", "pipe:1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
        let reader = ReaderStream::with_capacity(child.stdout.take().unwrap(), 16384);
        let mut stream = TranscodedStream {
            reader,
            feeder:None,decode_abort:None,
            child: Some(child),
            server: tokio::spawn(std::future::pending()),
            _permit: Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap(),
            ended: false,
            waiting: None,
        };
        let mut received = 0;
        let mut failed = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => received += bytes.len(),
                Err(_) => failed = true,
            }
        }
        assert!(
            received > 0,
            "fixture must emit actual encoded AAC before failing"
        );
        assert!(
            failed,
            "failed FFmpeg process must never become a complete cached stream"
        );
    }
}
