//! Keep the bounded compressed response across a native-rate output boundary.
use super::*;
impl MediaProxy {
    pub async fn prefetch_start(&self)->Result<()> {
        let Some(state)=self.preparation.as_ref() else{return Ok(())};
        if state.local.is_some()||state.remote.is_none()&&state.network_buffer.read().ok().is_none_or(|b|b.is_none()){return Ok(());}
        let uri=self.url.parse::<axum::http::Uri>()?;
        let Query(query)=Query::<conversion::Request>::try_from_uri(&uri)?;
        let mut warm=state.warm.lock().await;
        if warm.is_some(){return Ok(());}
        let mut headers=HeaderMap::new();headers.insert(header::ACCEPT_ENCODING,axum::http::HeaderValue::from_static("identity"));
        let response=fetch(state.clone(),query.clone(),Method::GET,headers).await;
        anyhow::ensure!(response.status().is_success(),"下一首预取失败：HTTP {}",response.status().as_u16());
        *warm=Some((query,response));Ok(())
    }
}
#[cfg(test)]mod tests {
 use super::*;use std::sync::atomic::AtomicUsize;
 #[tokio::test]async fn native_rate_boundary_reuses_one_bounded_response_and_seek_stays_independent(){
  let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();
  let audio=vec![73u8;768*1024];let expected=audio.clone();
  let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
  let server=tokio::spawn(async move{axum::serve(listener,Router::new().route("/api/v1/media/test",get(move|headers:HeaderMap|{let audio=audio.clone();let count=count.clone();async move{
   count.fetch_add(1,Ordering::SeqCst);let seek=headers.get(header::RANGE).is_some();let start=if seek{1024}else{0};
   let mut response=Response::builder().status(if seek{206}else{200}).header(header::CONTENT_LENGTH,audio.len()-start).header(header::ETAG,"\"fixture\"").header(header::ACCEPT_RANGES,"bytes");
   if seek{response=response.header(header::CONTENT_RANGE,format!("bytes {start}-{}/{}",audio.len()-1,audio.len()));}
   response.body(Body::from(audio[start..].to_vec())).unwrap()
  }}))).await.unwrap()});
  let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),skip_certificate_verification:false,certificate_sha256:None,alternatives:vec![],core_id:None};
  let proxy=MediaProxy::start(&profile,&format!("http://{address}/api/v1/media/test"),None).await.unwrap().with_network_buffer(NetworkBuffer::for_audio(&serde_json::json!({"bitrate":160000}),30,256*1024));
  proxy.prefetch_start().await.unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);
  let client=reqwest::Client::new();let seek=client.get(&proxy.url).header(header::RANGE,"bytes=1024-").send().await.unwrap();assert_eq!(seek.status(),StatusCode::PARTIAL_CONTENT);assert_eq!(&seek.bytes().await.unwrap()[..],&expected[1024..]);
  let response=client.get(&proxy.url).header(header::RANGE,"bytes=0-").send().await.unwrap();assert_eq!(response.status(),StatusCode::OK);assert_eq!(&response.bytes().await.unwrap()[..],expected.as_slice());assert_eq!(calls.load(Ordering::SeqCst),2,"initial playback reuses the prepared stream; a seek has its own checked range");
  server.abort();
 }

    #[tokio::test]
    async fn at_fifteen_seconds_the_same_response_fills_without_decoder_reads() {
        use std::time::Duration;

        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let bytes: Bytes = (0..2 * 1024 * 1024).map(|n| (n % 251) as u8).collect::<Vec<_>>().into();
        let expected = bytes.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let router = Router::new().route("/api/v1/media/test", get(move || {
                count.fetch_add(1, Ordering::Relaxed);
                let bytes = bytes.clone();
                async move {
                    let len = bytes.len();
                    let source = futures_util::stream::unfold((bytes, 0), |(bytes, start)| async move {
                        if start == bytes.len() { return None; }
                        tokio::time::sleep(Duration::from_millis(1)).await;
                        let end = (start + 16 * 1024).min(bytes.len());
                        Some((Ok::<_, std::io::Error>(bytes.slice(start..end)), (bytes, end)))
                    });
                    Response::builder().header(header::CONTENT_LENGTH, len).body(Body::from_stream(source)).unwrap()
                }
            }));
            axum::serve(listener, router).await.unwrap();
        });
        let profile = ConnectionProfile {
            core_public_key: None, id: "qa".into(), name: "qa".into(),
            address: format!("http://{address}"), skip_certificate_verification: false,
            certificate_sha256: None, alternatives: vec![], core_id: None,
        };
        let proxy = MediaProxy::start(&profile, &format!("http://{address}/api/v1/media/test"), None)
            .await.unwrap()
            .with_network_buffer(NetworkBuffer::for_audio(&serde_json::json!({"bitrate": 160000}), 5, 256 * 1024).with_preroll(5));
        let cache = tempfile::tempdir().unwrap();
        proxy.fill_after_startup_in(Some(cache.path().to_owned()));
        proxy.prefetch_start().await.unwrap();

        proxy.advance_playback(14.9);
        assert!(tokio::time::timeout(Duration::from_millis(80), async {
            while proxy.buffered_fraction() != Some(1.) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.is_err());
        let fraction = proxy.buffered_fraction().unwrap();
        assert!(fraction > 0. && fraction < 0.15, "early prefetch fraction: {fraction}");

        proxy.advance_playback(15.);
        tokio::time::timeout(Duration::from_secs(3), async {
            while proxy.buffered_fraction() != Some(1.) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("response must fill to the end while the decoder has not read it");

        let response = reqwest::get(&proxy.url).await.unwrap().error_for_status().unwrap();
        assert_eq!(response.bytes().await.unwrap(), expected);
        assert_eq!(requests.load(Ordering::Relaxed), 1);
        server.abort();
    }
}
