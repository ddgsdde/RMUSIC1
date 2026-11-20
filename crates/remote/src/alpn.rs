//! Multiplex ACME TLS-ALPN-01 and normal HTTPS on the same listener. The ACME
//! backend is a fixed loopback port; no destination comes from a client request.
use anyhow::{ensure, Result};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio_util::sync::CancellationToken;
struct Recorded {
    io: tokio::net::TcpStream,
    hello: Vec<u8>,
    record: bool,
}
impl AsyncRead for Recorded {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.io).poll_read(cx, buf);
        if this.record {
            this.hello.extend_from_slice(&buf.filled()[before..]);
            if this.hello.len() > 65536 {
                return Poll::Ready(Err(std::io::Error::other("TLS ClientHello 过大")));
            }
        }
        result
    }
}
impl AsyncWrite for Recorded {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }
}
pub async fn serve(
    listener: tokio::net::TcpListener,
    domain: String,
    validation_port: u16,
    mut certificates: tokio::sync::watch::Receiver<Option<Arc<rustls::ServerConfig>>>,
    router: axum::Router,
    cancel: CancellationToken,
) -> Result<()> {
    let capacity = Arc::new(tokio::sync::Semaphore::new(128));
    let mut jobs = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted=listener.accept()=>{let(io,peer)=accepted?;let Ok(permit)=capacity.clone().try_acquire_owned()else{continue;};let cert=certificates.borrow().clone();let domain=domain.clone();let router=router.clone().layer(axum::Extension(axum::extract::ConnectInfo(peer)));let c=cancel.child_token();jobs.spawn(async move{let _permit=permit;let result=async{
                let recorded=Recorded{io,hello:Vec::new(),record:true};let mut start=tokio::time::timeout(Duration::from_secs(10),tokio_rustls::LazyConfigAcceptor::new(rustls::server::Acceptor::default(),recorded)).await??;
                let acme=start.client_hello().alpn().is_some_and(|mut protocols|protocols.any(|v|v==b"acme-tls/1"));
                if acme{ensure!(start.client_hello().server_name()==Some(domain.as_str()),"ACME SNI 不匹配");let mut backend=tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST,validation_port)).await?;backend.write_all(&start.io.hello).await?;let mut io=start.io.io;tokio::time::timeout(Duration::from_secs(30),tokio::io::copy_bidirectional(&mut io,&mut backend)).await??;}
                else{let config=cert.ok_or_else(||anyhow::anyhow!("HTTPS 证书尚不可用"))?;start.io.record=false;start.io.hello.clear();let io=tokio::time::timeout(Duration::from_secs(10),start.into_stream(config)).await??;hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(io),hyper_util::service::TowerToHyperService::new(router)).with_upgrades().await?;}
                Ok::<_,anyhow::Error>(())};tokio::select!{_=result=>{},_=c.cancelled()=>{}}});},
            changed=certificates.changed()=>{if changed.is_err(){break;}},Some(_)=jobs.join_next()=>{},_=cancel.cancelled()=>break,
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn validation_works_before_first_certificate_and_does_not_take_over_https() {
        use tokio::io::AsyncReadExt;
        let identity = crate::identity::Identity::create();
        let core = uuid::Uuid::new_v4().to_string();
        let domain = crate::identity::core_hostname(&core);
        let normal = identity.tls(&core).unwrap();
        let mut acme = (*normal).clone();
        acme.alpn_protocols = vec![b"acme-tls/1".to_vec()];
        let validation = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = validation.local_addr().unwrap().port();
        let validation_job = tokio::spawn(async move {
            let (io, _) = validation.accept().await.unwrap();
            let mut tls = tokio_rustls::TlsAcceptor::from(Arc::new(acme))
                .accept(io)
                .await
                .unwrap();
            tls.write_all(b"validation").await.unwrap();
        });
        let public = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = public.local_addr().unwrap();
        let (tx, rx) = tokio::sync::watch::channel(None);
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let router =
            axum::Router::new().route("/status", axum::routing::get(|| async { "normal HTTPS" }));
        let sni = domain.clone();
        let task = tokio::spawn(serve(public, domain, port, rx, router, c));
        let mut client =
            (*crate::identity::core_tls_client(&core, &identity.public_key()).unwrap()).clone();
        client.alpn_protocols = vec![b"acme-tls/1".to_vec()];
        let mut io = tokio_rustls::TlsConnector::from(Arc::new(client))
            .connect(
                rustls::pki_types::ServerName::try_from(sni.clone()).unwrap(),
                tokio::net::TcpStream::connect(address).await.unwrap(),
            )
            .await
            .unwrap();
        let mut bytes = [0; 10];
        io.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"validation");
        validation_job.await.unwrap();
        tx.send_replace(Some(normal));
        let io = tokio_rustls::TlsConnector::from(
            crate::identity::core_tls_client(&core, &identity.public_key()).unwrap(),
        )
        .connect(
            rustls::pki_types::ServerName::try_from(sni).unwrap(),
            tokio::net::TcpStream::connect(address).await.unwrap(),
        )
        .await
        .unwrap();
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
                .await
                .unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let response = sender
            .send_request(
                axum::http::Request::builder()
                    .uri("/status")
                    .header("host", "localhost")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        cancel.cancel();
        task.await.unwrap().unwrap();
    }
}
