use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn profile(address: String) -> ConnectionProfile {
    serde_json::from_value(serde_json::json!({"id":"qa","name":"QA","address":address})).unwrap()
}

async fn self_signed(websocket: bool) -> (String, tokio::task::JoinHandle<()>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate = include_bytes!("../../tests/fixtures/test-cert.der");
    let key = include_bytes!("../../tests/fixtures/test-key.der");
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certificate.to_vec())],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.to_vec()).into(),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("https://{}/music", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut clients = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                peer=listener.accept()=>{
                    let (socket, _) = peer.unwrap();let acceptor=acceptor.clone();
                    clients.spawn(async move {
                        let Ok(Ok(mut stream)) = tokio::time::timeout(Duration::from_secs(3), acceptor.accept(socket)).await else {return};
                        if websocket {
                            let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else { return };
                            if let Some(Ok(message)) = socket.next().await { let _ = socket.send(message).await; }
                        } else {
                            let mut bytes = Vec::new();let mut buffer=[0;1024];
                            while !bytes.windows(4).any(|s|s==b"\r\n\r\n") {
                                let Ok(Ok(n))=tokio::time::timeout(Duration::from_secs(3),stream.read(&mut buffer)).await else{return};
                                if n==0||bytes.len()+n>8192{return}bytes.extend_from_slice(&buffer[..n]);
                            }
                            let request=String::from_utf8_lossy(&bytes);
                            let response=if request.starts_with("GET /music/api/v1/status ") {
                                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                                let body=r#"{"core_id":"qa-core"}"#;
                                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())
                            } else if request.starts_with("GET /music/api/v1/media/id?ticket=qa ") {
                                assert!(request.to_ascii_lowercase().contains("range: bytes=2-5"));
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 2-5/8\r\nConnection: close\r\n\r\n2345".into()
                            } else {
                                "HTTP/1.1 302 Found\r\nLocation: https://else.invalid/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                            };
                            let _=stream.write_all(response.as_bytes()).await;
                        }
                    });
                },
                _=clients.join_next(),if !clients.is_empty()=>{}
            }
        }
    });
    (address, task)
}

#[test]
fn trust_exception_is_default_off_https_only_and_never_inherited_by_fallbacks() {
    let mut p = profile("https://primary.invalid/music".into());
    assert!(!p.skip_certificate_verification);
    p.skip_certificate_verification = true;
    p.alternatives =
        vec![
            serde_json::from_value(serde_json::json!({"address":"https://backup.invalid/music"}))
                .unwrap(),
        ];
    assert!(p.candidates()[0].skip_certificate_verification);
    assert!(!p.candidates()[1].skip_certificate_verification);
    p.certificate_sha256 = Some("aa".repeat(32));
    assert!(p.validate().is_err());
    p.certificate_sha256 = None;
    p.address = "http://primary.invalid/music".into();
    assert!(p.validate().is_err());
    assert!(serde_json::from_value::<ConnectionProfile>(serde_json::json!({"id":"qa","name":"QA","address":"https://qa","skip_certificate_verification":"true"})).is_err());
}

#[tokio::test]
async fn explicit_https_skip_keeps_identity_media_range_redirect_and_policy_change_checks() {
    let (address, server) = self_signed(false).await;
    let mut p = profile(address.clone());
    assert!(p.resolve().await.is_err());
    p.skip_certificate_verification = true;
    let selected = p.resolve().await.unwrap();
    assert_eq!(selected.core_id.as_deref(), Some("qa-core"));
    let response = selected
        .http()
        .unwrap()
        .get(selected.media_url("api/v1/media/id?ticket=qa").unwrap())
        .header("Range", "bytes=2-5")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.bytes().await.unwrap(), "2345");
    assert!(selected
        .http()
        .unwrap()
        .get(selected.endpoint("redirect").unwrap())
        .bearer_auth("synthetic-only")
        .send()
        .await
        .is_err());
    assert!(selected
        .media_url("https://else.invalid/music/api/v1/media/id")
        .is_err());
    p.core_id = Some("other-core".into());
    assert!(p.resolve().await.is_err());
    let config =
        crate::sequence::Config::new(selected.clone(), "synthetic".into(), "endpoint".into())
            .unwrap();
    let mut strict = selected.clone();
    strict.skip_certificate_verification = false;
    assert!(!config.same_connection(&strict, "synthetic", "endpoint"));
    assert!(strict.resolve().await.is_err());
    let mut fallback = profile("http://127.0.0.1:9/music".into());
    fallback.core_id = Some("qa-core".into());
    fallback.alternatives = vec![CoreAddress {
        from_core: Some(false), address,
        certificate_sha256: None,
        skip_certificate_verification: true,
    }];
    assert!(
        fallback
            .resolve()
            .await
            .unwrap()
            .skip_certificate_verification
    );
    fallback.alternatives[0].skip_certificate_verification = false;
    assert!(fallback.resolve().await.is_err());
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn websocket_uses_the_same_default_off_explicit_trust_policy() {
    let (address, server) = self_signed(true).await;
    let mut p = profile(address);
    let url = p.address.replacen("https:", "wss:", 1);
    let connect = |p: ConnectionProfile, url: String| async move {
        tokio_tungstenite::connect_async_tls_with_config(
            url,
            None,
            false,
            Some(tokio_tungstenite::Connector::Rustls(
                p.tls_config().unwrap(),
            )),
        )
        .await
    };
    assert!(connect(p.clone(), url.clone()).await.is_err());
    p.skip_certificate_verification = true;
    let (mut socket, _) = connect(p, url).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text("qa".into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap().into_text().unwrap(),
        "qa"
    );
    let _ = socket.close(None).await;
    server.abort();
    let _ = server.await;
}
