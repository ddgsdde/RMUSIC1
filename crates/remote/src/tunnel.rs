//! Bounded WebSocket byte streams for the encrypted connection to a Core.
use anyhow::{ensure, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    net::TcpStream,
};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tokio_util::sync::CancellationToken;

pub const WINDOW: usize = 64 * 1024;
pub async fn connect_core(
    base: &str,
    path: &str,
    tls: Option<std::sync::Arc<rustls::ClientConfig>>,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    let mut url = url::Url::parse(base)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Core 入口无效"
    );
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("Core 通道无效"))?;
    url.set_path(&format!("{}/{}", url.path().trim_end_matches('/'), path));
    connect_url(url, tls).await
}
async fn connect_url(
    url: url::Url,
    tls: Option<std::sync::Arc<rustls::ClientConfig>>,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>> {
    let stream = dial(&url).await?;
    let request = url.as_str().into_client_request()?;
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .write_buffer_size(16 * 1024)
        .max_message_size(Some(WINDOW))
        .max_frame_size(Some(WINDOW))
        .max_write_buffer_size(WINDOW * 2);
    let (socket, _) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio_tungstenite::client_async_tls_with_config(
            request,
            stream,
            Some(config),
            tls.map(tokio_tungstenite::Connector::Rustls),
        ),
    )
    .await??;
    Ok(socket)
}
async fn dial(url: &url::Url) -> Result<TcpStream> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("缺少主机地址"))?
        .trim_matches(['[', ']']);
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("缺少端口"))?;
    let addresses = crate::dns::resolve(host,port).await?;
    let (mut six, mut four): (Vec<_>, Vec<_>) = addresses.into_iter().partition(|a| a.is_ipv6());
    six.reverse();
    four.reverse();
    let mut ordered = Vec::new();
    while !six.is_empty() || !four.is_empty() {
        if let Some(a) = six.pop() {
            ordered.push(a);
        }
        if let Some(a) = four.pop() {
            ordered.push(a);
        }
    }
    let mut jobs = tokio::task::JoinSet::new();
    for (index, address) in ordered.into_iter().take(16).enumerate() {
        jobs.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(index as u64 * 200)).await;
            #[cfg(target_os="macos")] { crate::underlay::connect(address).await }
            #[cfg(not(target_os="macos"))] { TcpStream::connect(address).await }
        });
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(result) = jobs.join_next().await {
            if let Ok(Ok(stream)) = result {
                stream.set_nodelay(true)?;
                return Ok(stream);
            }
        }
        anyhow::bail!("IPv4／IPv6 入口均不可达，请检查域名、端口和防火墙")
    })
    .await;
    jobs.abort_all();
    result.map_err(|_| anyhow::anyhow!("双栈连接超时，请检查入口和网络"))?
}
pub async fn bridge_client<S>(
    socket: WebSocketStream<S>,
    stream: DuplexStream,
    cancel: CancellationToken,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut send, mut receive) = socket.split();
    let (mut read, mut write) = tokio::io::split(stream);
    let outgoing = async {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let n = read.read(&mut buffer).await?;
            if n == 0 {
                let _ = send.send(Message::Close(None)).await;
                break;
            }
            send.send(Message::Binary(buffer[..n].to_vec().into()))
                .await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let incoming = async {
        while let Some(message) = receive.next().await {
            match message? {
                Message::Binary(bytes) => {
                    ensure!(bytes.len() <= WINDOW, "通道数据块过大");
                    write.write_all(&bytes).await?;
                }
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => {}
                _ => anyhow::bail!("字节通道不接受文本"),
            }
        }
        write.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {v=outgoing=>v,v=incoming=>v,_=cancel.cancelled()=>Ok(())}
}
pub async fn bridge_server(
    socket: axum::extract::ws::WebSocket,
    stream: DuplexStream,
    cancel: CancellationToken,
) -> Result<()> {
    use axum::extract::ws::Message as M;
    let (mut send, mut receive) = socket.split();
    let (mut read, mut write) = tokio::io::split(stream);
    let outgoing = async {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let n = read.read(&mut buffer).await?;
            if n == 0 {
                let _ = send.send(M::Close(None)).await;
                break;
            }
            send.send(M::Binary(buffer[..n].to_vec().into())).await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    let incoming = async {
        while let Some(message) = receive.next().await {
            match message? {
                M::Binary(bytes) => {
                    ensure!(bytes.len() <= WINDOW, "通道数据块过大");
                    write.write_all(&bytes).await?;
                }
                M::Close(_) => break,
                M::Ping(_) | M::Pong(_) => {}
                _ => anyhow::bail!("字节通道不接受文本"),
            }
        }
        write.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {v=outgoing=>v,v=incoming=>v,_=cancel.cancelled()=>Ok(())}
}

pub async fn serve_and_drain<C, H>(codec: C, handler: H, cancel: CancellationToken) -> Result<()>
where
    C: std::future::Future<Output = Result<()>>,
    H: std::future::Future<Output = Result<()>>,
{
    tokio::pin!(codec, handler);
    let result = tokio::select! {
        value = &mut codec => value,
        value = &mut handler => {
            if value.is_ok() {
                tokio::select! {
                    _ = tokio::time::timeout(std::time::Duration::from_secs(5), &mut codec) => {},
                    _ = cancel.cancelled() => {},
                }
            }
            value
        },
        _ = cancel.cancelled() => Ok(()),
    };
    cancel.cancel();
    result
}
