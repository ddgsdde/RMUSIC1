//! A client pull policy; audio stays on HTTPS, never on the control WebSocket.
use super::*;
use anyhow::Context;
use rmusic_audio::probe::AudioMetadata;
use serde::{Deserialize, Serialize};

/// Compressed-byte prefetch, independent of decoder/HAL buffers and conversion.
#[derive(Clone,Debug)]
pub struct NetworkBuffer {pub(crate) capacity:usize,pub(crate) prime_bytes:usize,pub(crate) seconds:u32,pub(crate) max_bytes:usize,duration:Option<f64>,bytes_per_second:u64,preroll_seconds:u32}
impl NetworkBuffer {
    pub fn for_audio(info:&serde_json::Value,seconds:u32,max_bytes:usize)->Self {
        let bitrate=info["bitrate"].as_u64().filter(|n|(8000..=200_000_000).contains(n));
        let pcm=info["sample_rate"].as_u64().unwrap_or(48000).min(768000)
            .saturating_mul(info["channels"].as_u64().unwrap_or(2).clamp(1,32))
            .saturating_mul(info["bit_depth"].as_u64().unwrap_or(24).clamp(16,32));
        let bytes_per_second=bitrate.unwrap_or(pcm)/8;
        let min_bytes = if seconds == 5 { 64 * 1024 } else { 256 * 1024 };
        let bytes=bytes_per_second.saturating_mul(u64::from(seconds.clamp(5,60)))
            .clamp(min_bytes,max_bytes.clamp(256*1024,32*1024*1024) as u64);
        let capacity=bytes.div_ceil(65536) as usize;
        Self{capacity,prime_bytes:0,seconds,max_bytes,duration:info["duration"].as_f64().filter(|n|n.is_finite()&&(1. ..=86400.).contains(n)),bytes_per_second,preroll_seconds:2}.with_preroll(2)
    }
    pub fn with_preroll(mut self, seconds:u32)->Self {
        self.preroll_seconds=seconds.clamp(2,self.seconds.clamp(5,60));
        self.prime_bytes=self.bytes_per_second.saturating_mul(u64::from(self.preroll_seconds))
            .clamp(65536,self.capacity as u64*65536) as usize;
        self
    }
    pub(crate) fn preroll_seconds(&self)->u32 {self.preroll_seconds}
    pub(crate) fn bytes_per_second(&self)->u64 {self.bytes_per_second}
    pub(crate) fn entity(self,total:Option<u64>)->Self {
        if let Some((duration,total))=self.duration.zip(total.filter(|n|*n>0)) {
            return Self::for_audio(&serde_json::json!({"duration":duration,"bitrate":(total as f64*8./duration).ceil() as u64}),self.seconds,self.max_bytes).with_preroll(self.preroll_seconds);
        }self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct StreamingPolicy {
    pub quality: String,
    pub mode: String,
    pub max_sample_rate: u32,
    pub max_bit_depth: u16,
    pub buffer_seconds: u32,
}
impl Default for StreamingPolicy {
    fn default() -> Self {
        Self {
            quality: "original".into(),
            mode: "auto".into(),
            max_sample_rate: 768000,
            max_bit_depth: 32,
            buffer_seconds: 30,
        }
    }
}
impl StreamingPolicy {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            ["original", "custom"].contains(&self.quality.as_str())
                && ["auto", "lan", "remote", "vpn"].contains(&self.mode.as_str())
                && rmusic_audio::output::PCM_RATES.contains(&self.max_sample_rate)
                && [16, 24, 32].contains(&self.max_bit_depth)
                && (5..=60).contains(&self.buffer_seconds),
            "外出流格式或缓冲设置无效"
        );
        Ok(())
    }
    pub fn remote(&self, address: &str) -> bool {
        if rmusic_remote::native::is_remote(address){return true;}
        if self.mode != "auto" {
            return self.mode == "remote"||self.mode=="vpn";
        }
        matches!(crate::network::address_kind(address),crate::network::AddressKind::Overlay|crate::network::AddressKind::Public)
    }
    pub fn metadata(&self, value: &serde_json::Value) -> Result<AudioMetadata> {
        self.validate()?;
        anyhow::ensure!(
            value["verified"] == true,
            "外出流需要已验证的音源格式，请刷新音源后重试"
        );
        let source_rate = value["sample_rate"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let format = value["format"].as_str().unwrap_or("");
        let dsd = matches!(format, "dsf" | "dff");
        let rate = if dsd { source_rate / 16 } else { source_rate };
        let limits = rmusic_audio::output::OutputLimits {
            max_sample_rate: Some(self.max_sample_rate.min(if dsd{384000}else{768000})),
            ..Default::default()
        };
        let sample_rate = limits.rate(rate)?;
        anyhow::ensure!(rate >= 8000 && sample_rate <= rate, "音源采样率无效");
        let source_bits = value["bit_depth"].as_u64().unwrap_or(0).min(32) as u16;
        let bit_limit=self.max_bit_depth;
        let bit_depth = if dsd || source_bits == 0 {
            bit_limit
        } else {
            source_bits.max(16).min(bit_limit)
        };
        let channels = value["channels"]
            .as_u64()
            .filter(|n| (1..=32).contains(n))
            .context("音源声道数无效")?
             as u16;
        Ok(AudioMetadata {
            flac_samples: None,
            duration: value["duration"]
                .as_f64()
                .filter(|n| n.is_finite() && *n > 0.),
            format: "flac".into(),
            sample_rate,
            channels,
            bit_depth: [16,24,32].into_iter().find(|bits|*bits>=bit_depth).unwrap_or(32),
            bitrate: 0,
            quality_rank: 0,
            source_integer_lossless: matches!(format, "flac" | "alac")
                || matches!(format, "wav" | "aiff") && (1..=24).contains(&source_bits),
        })
    }
}
#[derive(Clone)]
pub(super) struct RemoteSpec {
    pub policy: StreamingPolicy,
    pub metadata: AudioMetadata,
    pub source: serde_json::Value,
}
impl RemoteSpec {
    pub fn capacity(&self) -> usize { self.capacity_for(self.policy.buffer_seconds) }
    pub fn capacity_for(&self, seconds:u32) -> usize {
        let m = &self.metadata;
        let min_bytes = if seconds == 5 { 64 * 1024 } else { 256 * 1024 };
        ((u64::from(m.sample_rate) * u64::from(m.channels) * u64::from(m.bit_depth) / 8
            * u64::from(seconds))
        .clamp(min_bytes, 32 * 1024 * 1024)
        .div_ceil(65536)) as usize
    }
    pub fn priming_bytes(&self) -> usize {
        let m = &self.metadata;
        (u64::from(m.sample_rate) * u64::from(m.channels) * u64::from(m.bit_depth) / 8 * 5)
            .min((self.capacity() * 65536) as u64) as usize
    }
    pub fn target(
        &self,
        source: &str,
        query: &conversion::Request,
    ) -> std::result::Result<(String, conversion::Confirmed), String> {
        let m = &self.metadata;
        let mut query = query.clone();
        query.conversion = Some("core".into());
        query.rate = Some(
            query
                .rate
                .unwrap_or(m.sample_rate)
                .min(m.sample_rate),
        );
        query.bits = Some(query.bits.unwrap_or(m.bit_depth).min(m.bit_depth));
        query.channels = Some(query.channels.unwrap_or(m.channels).min(m.channels));
        let (url, format) = query.target(source)?.ok_or("missing stream target")?;
        Ok((
            url.replacen("/api/v1/native-pcm/", "/api/v1/native-flac/", 1),
            format,
        ))
    }
}
/// Network buffering is separate from the small HAL ring. Fixed-size chunks keep
/// the compressed queue bounded even when HTTP/2 delivers many tiny frames.
async fn send_chunk(
    sender: &tokio::sync::mpsc::Sender<Result<axum::body::Bytes, std::io::Error>>,
    chunk: Result<axum::body::Bytes, std::io::Error>,
    cancellation: tokio::sync::watch::Receiver<bool>,
) -> bool {
    tokio::select! {biased;_ = cancelled(cancellation)=>false,result=sender.send(chunk)=>result.is_ok()}
}
pub(super) async fn buffered<S>(
    body: S,
    capacity: usize,
    prime_bytes: usize,
    prime_wait: std::time::Duration,
    cancellation: tokio::sync::watch::Receiver<bool>,
    source_failed: Arc<AtomicBool>,
    staged: bool,
) -> Body
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send + 'static,
{
    buffered_with_deadlines(
        body,
        capacity,
        prime_bytes,
        cancellation,
        source_failed,
        (!staged).then_some(std::time::Duration::from_secs(30)),
        std::time::Duration::from_millis(250),
        prime_wait,
    )
    .await
}
async fn buffered_with_deadlines<S>(
    body: S,
    capacity: usize,
    prime_bytes: usize,
    cancellation: tokio::sync::watch::Receiver<bool>,
    source_failed: Arc<AtomicBool>,
    stall: Option<std::time::Duration>,
    flush: std::time::Duration,
    prime_wait: std::time::Duration,
) -> Body
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send + 'static,
{
    let started = std::time::Instant::now();
    let diagnostic = std::env::var_os("RMUSIC_AUDIO_DIAGNOSTICS").is_some();
    let (sender, receiver) = tokio::sync::mpsc::channel(capacity);
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let producer_cancel = cancellation.clone();
    tokio::spawn(async move {
        let mut ready = Some(ready);
        let mut queued = 0usize;
        let mut pending = Vec::with_capacity(65536);
        futures_util::pin_mut!(body);
        loop {
            if sender.capacity() == 0 {
                let permit = tokio::select! {biased;_ = cancelled(producer_cancel.clone())=>return,permit=sender.reserve()=>permit};
                if permit.is_err() {
                    return;
                }
            }
            let chunk = tokio::select! {biased;
            _ = tokio::time::sleep(flush),if !pending.is_empty()=>{
                let count=pending.len();
                if !send_chunk(&sender,Ok(axum::body::Bytes::from(std::mem::replace(&mut pending,Vec::with_capacity(65536)))),producer_cancel.clone()).await{return;}
                queued+=count;
                // Small flushed packets can fill the queue before reaching the byte target.
                if queued>=prime_bytes || sender.capacity()==0 {if let Some(ready)=ready.take(){let _=ready.send(());}}
                continue;
            },_ = cancelled(producer_cancel.clone())=>return,_ = sender.closed()=>return,_ = tokio::time::sleep(stall.unwrap_or_default()),if stall.is_some()=>{
                    source_failed.store(true,Ordering::Release);
                    let _=send_chunk(&sender,Err(std::io::Error::new(std::io::ErrorKind::TimedOut,"Core audio stream stalled")),producer_cancel.clone()).await;return;
                },chunk=body.next()=>chunk};
            let Some(chunk) = chunk else { break };
            match chunk {
                Ok(bytes) => {
                    let mut offset = 0;
                    while offset < bytes.len() {
                        let take = (65536 - pending.len()).min(bytes.len() - offset);
                        pending.extend_from_slice(&bytes[offset..offset + take]);
                        offset += take;
                        if pending.len() == 65536 {
                            if !send_chunk(
                                &sender,
                                Ok(axum::body::Bytes::from(std::mem::replace(
                                    &mut pending,
                                    Vec::with_capacity(65536),
                                ))),
                                producer_cancel.clone(),
                            )
                            .await
                            {
                                return;
                            }
                            queued += 65536;
                            if queued >= prime_bytes || sender.capacity() == 0 {
                                if let Some(ready) = ready.take() {
                                    let _ = ready.send(());
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    source_failed.store(true,Ordering::Release);
                    if diagnostic {
                        eprintln!(
                            "Remote FLAC prefetch failed: {}",
                            if error.kind()==std::io::ErrorKind::TimedOut {
                                "timeout"
                            } else {
                                "body read"
                            }
                        );
                    }
                    if !pending.is_empty()
                        && !send_chunk(
                            &sender,
                            Ok(axum::body::Bytes::from(std::mem::replace(
                                &mut pending,
                                Vec::with_capacity(65536),
                            ))),
                            producer_cancel.clone(),
                        )
                        .await
                    {
                        return;
                    }
                    let _ = send_chunk(
                        &sender,
                        Err(error),
                        producer_cancel.clone(),
                    )
                    .await;
                    return;
                }
            }
        }
        if !pending.is_empty() {
            let _ = send_chunk(
                &sender,
                Ok(axum::body::Bytes::from(pending)),
                producer_cancel,
            )
            .await;
        }
        // EOF (including a short song) releases priming immediately.
    });
    // Avoid adding unbounded startup latency on slow WANs. After priming, fetching
    // continues independently until the full user budget reaches backpressure.
    tokio::select! {biased;_ = cancelled(cancellation)=>{},_ = tokio::time::timeout(prime_wait,waiting)=>{}}
    if diagnostic {
        eprintln!(
            "Remote FLAC network primed: {} ms, capacity {} KiB",
            started.elapsed().as_millis(),
            capacity * 64
        );
    }
    let buffered = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    });
    Body::from_stream(buffered)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_detection_caps_and_memory_are_bounded_without_upsampling() {
        let p = StreamingPolicy::default();
        assert!(!p.remote("https://192.168.1.2/music"));
        assert!(!p.remote("https://nas.local"));
        assert!(p.remote("https://music.example.org/prefix"));
        let m=p.metadata(&serde_json::json!({"verified":true,"format":"flac","sample_rate":44100,"bit_depth":16,"channels":2})).unwrap();
        assert_eq!((m.sample_rate, m.bit_depth), (44100, 16));
        let dsd=p.metadata(&serde_json::json!({"verified":true,"format":"dsf","sample_rate":11289600,"bit_depth":1,"channels":2})).unwrap();
        assert_eq!(dsd.sample_rate, 352800);
        assert_eq!(dsd.format, "flac");
        assert!(!dsd.source_integer_lossless);
        assert!(
            RemoteSpec {
                policy: p,
                metadata: dsd,
                source: serde_json::Value::Null
            }
            .capacity()
                <= 512
        );
    }
    #[test]fn prefetch_budgets_use_encoded_bitrate_and_remain_bounded(){
        let info=serde_json::json!({"sample_rate":96000,"bit_depth":24,"channels":2,"bitrate":1_000_000});
        let current=NetworkBuffer::for_audio(&info,30,8*1024*1024);assert_eq!(current.prime_bytes,250000);assert!(current.capacity*65536<=8*1024*1024);
        let measured=NetworkBuffer::for_audio(&serde_json::json!({"duration":100}),30,8*1024*1024).entity(Some(1_000_000));assert_eq!(measured.prime_bytes,65536);assert!(measured.capacity*65536<=512*1024);
        let roaming=NetworkBuffer::for_audio(&serde_json::json!({"duration":100}),30,8*1024*1024)
            .with_preroll(5).entity(Some(12_000_000));
        assert_eq!(roaming.prime_bytes,600_000);
        assert_eq!(roaming.preroll_seconds(),5);
        let next=NetworkBuffer::for_audio(&info,30,2*1024*1024);assert_eq!(next.capacity*65536,2*1024*1024);
        let original=StreamingPolicy::default().metadata(&serde_json::json!({"verified":true,"format":"flac","sample_rate":192000,"bit_depth":24,"channels":6})).unwrap();assert_eq!((original.sample_rate,original.bit_depth,original.channels),(192000,24,6));
    }
    #[tokio::test]
    async fn full_queue_of_partial_packets_releases_priming() {
        let (_cancel, cancellation) = tokio::sync::watch::channel(false);
        let source = futures_util::stream::unfold(0, |index| async move {
            if index == 2 { return None; }
            if index == 1 { tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
            Some((Ok(axum::body::Bytes::from_static(b"audio")), index + 1))
        });
        let body = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            buffered_with_deadlines(source, 1, 65536, cancellation, Arc::new(AtomicBool::new(false)),
                Some(std::time::Duration::from_secs(1)), std::time::Duration::from_millis(10),std::time::Duration::from_secs(3)),
        ).await.expect("a full queue must be available to the decoder");
        assert_eq!(&axum::body::to_bytes(body, 10).await.unwrap()[..], b"audioaudio");
    }

    #[tokio::test]
    async fn tiny_http_frames_share_bounded_blocks_and_cancel_during_priming() {
        let (_cancel, cancellation) = tokio::sync::watch::channel(false);
        let body = futures_util::stream::iter(
            (0..200).map(|_| Ok(axum::body::Bytes::from(vec![3; 1000]))),
        );
        let result = buffered(
            body,
            4,
            196608,
            std::time::Duration::from_secs(3),
            cancellation,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await;
        let bytes = axum::body::to_bytes(result, 200001).await.unwrap();
        assert_eq!(bytes.len(), 200000);
        assert!(bytes.iter().all(|v| *v == 3));
        struct Guard(Arc<AtomicBool>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Guard(dropped.clone());
        let source = futures_util::stream::once(async move {
            let _guard = guard;
            std::future::pending::<Result<axum::body::Bytes, std::io::Error>>().await
        });
        let (cancel, cancellation) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(buffered(
            source,
            4,
            196608,
            std::time::Duration::from_secs(3),
            cancellation,
            Arc::new(AtomicBool::new(false)),
            false,
        ));
        tokio::task::yield_now().await;
        cancel.send_replace(true);
        let body = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        drop(body);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !dropped.load(Ordering::Acquire) {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn backpressure_pause_stops_the_network_stall_deadline() {
        let (_cancel, cancellation) = tokio::sync::watch::channel(false);
        let failed = Arc::new(AtomicBool::new(false));
        let source =
            futures_util::stream::once(async { Ok(axum::body::Bytes::from(vec![3; 1000])) })
                .chain(futures_util::stream::pending());
        let body = buffered_with_deadlines(
            source,
            1,
            1,
            cancellation,
            failed.clone(),
            Some(std::time::Duration::from_millis(40)),
            std::time::Duration::from_millis(1),
            std::time::Duration::from_secs(3),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        let mut stream = body.into_data_stream();
        assert_eq!(stream.next().await.unwrap().unwrap().len(), 1000);
        assert!(!failed.load(Ordering::Acquire));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), stream.next())
                .await
                .is_err(),
            "resuming starts a fresh network deadline"
        );
        assert!(
            stream.next().await.unwrap().is_err(),
            "an actual continued network stall still fails"
        );
        assert!(failed.load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn ffmpeg_accepts_short_primed_flac_http_response() {
        let encoded = std::process::Command::new(rmusic_audio::tool_binary("ffmpeg"))
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=997:sample_rate=48000:duration=1",
                "-ac",
                "2",
                "-sample_fmt",
                "s16",
                "-c:a",
                "flac",
                "-f",
                "ogg",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(encoded.status.success());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let bytes = encoded.stdout;
        let router = Router::new().route(
            "/api/v1/native-flac/test",
            get(move || {
                let bytes = bytes.clone();
                async move {
                    Response::builder()
                        .header("Content-Type", "audio/ogg")
                        .header(
                            "X-RMusic-Native-FLAC",
                            "v1;rate=48000;bits=16;channels=2;container=ogg",
                        )
                        .body(Body::from_stream(futures_util::stream::once(async move {
                            Ok::<_, std::io::Error>(bytes)
                        })))
                        .unwrap()
                }
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let profile = ConnectionProfile {core_public_key:None,
            id: "short".into(),
            name: "test".into(),
            address: format!("http://{address}"),
            skip_certificate_verification: false,
            certificate_sha256: None,
            alternatives: vec![],
            core_id: None,
        };
        let proxy=MediaProxy::remote_stream(&profile,&format!("http://{address}/api/v1/media/test?ticket=test"),StreamingPolicy::default(),serde_json::json!({"verified":true,"format":"flac","sample_rate":48000,"bit_depth":16,"channels":2})).await.unwrap();
        let url = proxy.url.clone();
        let decoded = tokio::task::spawn_blocking(move || {
            let mut decoder = rmusic_audio::decoder::FfmpegDecoder::open(
                &url,
                0.,
                48000,
                2,
                rmusic_audio::decoder::PcmEncoding::Signed32,
            )?;
            let mut frames = 0;
            let mut bytes = [0; 16384];
            loop {
                let count = decoder.read_frames(&mut bytes, 8)?;
                if count == 0 {
                    break;
                }
                frames += count;
            }
            Ok::<_, anyhow::Error>(frames)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(decoded, 48000);
        drop(proxy);
        server.abort();
    }
    #[tokio::test]
    async fn remote_stream_requires_confirmed_format_keeps_prefix_and_cancels() {
        use axum::{extract::Query, http::HeaderMap};
        use std::collections::HashMap;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requested = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = requested.clone();
        let router = Router::new().route(
            "/prefix/api/v1/native-flac/test",
            get(
                move |Query(q): Query<HashMap<String, String>>, headers: HeaderMap| {
                    let count = count.clone();
                    async move {
                        assert!(!headers.contains_key(header::RANGE));
                        assert_eq!(q["ticket"], "synthetic");
                        assert_eq!(q["start"], "1.25");
                        assert_eq!(q["rate"], "44100");
                        assert_eq!(q["bits"], "16");
                        count.fetch_add(1, Ordering::Relaxed);
                        Response::builder()
                            .header(
                                "X-RMusic-Native-FLAC",
                                "v1;rate=44100;bits=16;channels=2;container=ogg",
                            )
                            .body(Body::from(vec![7u8; 200000]))
                            .unwrap()
                    }
                },
            ),
        );
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let profile = ConnectionProfile {core_public_key:None,
            id: "remote-fixture".into(),
            name: "test".into(),
            address: format!("http://{address}/prefix"),
            skip_certificate_verification: false,
            certificate_sha256: None,
            alternatives: vec![],
            core_id: None,
        };
        let proxy=MediaProxy::remote_stream(&profile,&format!("http://{address}/prefix/api/v1/media/test?ticket=synthetic"),StreamingPolicy::default(),serde_json::json!({"verified":true,"format":"flac","sample_rate":44100,"bit_depth":16,"channels":2})).await.unwrap();
        let response = reqwest::Client::new()
            .get(format!("{}&start=1.25", proxy.url))
            .header(header::RANGE, "bytes=100-")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.bytes().await.unwrap().len(), 200000);
        assert_eq!(requested.load(Ordering::Relaxed), 1);
        assert!(proxy.prepared_metadata().unwrap().flac_samples.is_none());
        drop(proxy);
        server.abort();
    }
}
