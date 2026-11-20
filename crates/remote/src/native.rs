//! Loopback HTTP facade around a pinned end-to-end Core TLS connection.
use anyhow::{ensure, Result};
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

pub struct Gateway {
    pub address: String,
    pub logical_address: String,
    pub core_id: String,
    pub core_public_key: String,
    remote: bool,
    capability: String,
    tls: Arc<rustls::ClientConfig>,
    outer_tls: Option<Arc<rustls::ClientConfig>>,
    cancel: tokio_util::sync::CancellationToken,
    last: Mutex<Instant>,
    active: std::sync::atomic::AtomicUsize,
    control_pool: crate::native_pool::Pool,
}
type Gateways = HashMap<String, Arc<Gateway>>;
fn gateways() -> &'static Mutex<Gateways> {
    static VALUE: OnceLock<Mutex<Gateways>> = OnceLock::new();
    VALUE.get_or_init(Default::default)
}
pub fn is_remote(address: &str) -> bool {
    gateways()
        .lock()
        .unwrap()
        .values()
        .any(|g| g.address == address && g.remote)
}
pub fn is_gateway(address: &str) -> bool {
    gateways()
        .lock()
        .unwrap()
        .values()
        .any(|g| g.address == address)
}
pub fn logical_address(address: &str) -> String {
    gateways()
        .lock()
        .unwrap()
        .values()
        .find(|g| g.address == address)
        .map(|g| g.logical_address.clone())
        .unwrap_or_else(|| address.into())
}
pub async fn open_core(
    base: &str,
    core_id: String,
    core_public_key: String,
    outer_tls: Option<Arc<rustls::ClientConfig>>,
    remote: bool,
) -> Result<Arc<Gateway>> {
    uuid::Uuid::parse_str(&core_id)?;
    let key = format!("{base}:{core_id}:{core_public_key}:{remote}");
    if let Some(gateway) = gateways().lock().unwrap().get(&key).cloned() {
        *gateway.last.lock().unwrap() = Instant::now();
        return Ok(gateway);
    }
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let capability = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let gateway = Arc::new(Gateway {
        address: format!("http://127.0.0.1:{port}/{capability}"),
        logical_address: base.into(),
        core_id: core_id.clone(),
        core_public_key: core_public_key.clone(),
        remote,
        capability,
        tls: crate::identity::core_tls_client(&core_id, &core_public_key)?,
        outer_tls,
        cancel: tokio_util::sync::CancellationToken::new(),
        last: Mutex::new(Instant::now()),
        active: 0.into(),
        control_pool: Default::default(),
    });
    {
        let mut all = gateways().lock().unwrap();
        ensure!(all.len() < 16, "远程连接过多，请断开不用的 Core");
        if let Some(existing) = all.get(&key) {
            return Ok(existing.clone());
        }
        all.insert(key.clone(), gateway.clone());
    }
    let router = Router::new().fallback(proxy).with_state(gateway.clone());
    let cancel = gateway.cancel.clone();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router.into_make_service())
            .with_graceful_shutdown(cancel.cancelled_owned())
            .await;
    });
    let weak = Arc::downgrade(&gateway);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let Some(g) = weak.upgrade() else {
                return;
            };
            if g.active.load(std::sync::atomic::Ordering::Acquire) == 0
                && g.last.lock().unwrap().elapsed() > std::time::Duration::from_secs(300)
            {
                g.cancel.cancel();
                gateways().lock().unwrap().remove(&key);
                return;
            }
        }
    });
    Ok(gateway)
}
struct Active(Arc<Gateway>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0
            .active
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        *self.0.last.lock().unwrap() = Instant::now();
    }
}
async fn proxy(State(g): State<Arc<Gateway>>, mut request: Request<Body>) -> Response {
    let prefix = format!("/{}", g.capability);
    let path = request.uri().path();
    let Some(stripped) = path.strip_prefix(&prefix).filter(|p| p.starts_with('/')) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let uri = format!(
        "{}{}",
        stripped,
        request
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default()
    );
    let Ok(uri) = uri.parse() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *request.uri_mut() = uri;
    g.active.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    let active = Active(g.clone());
    let result = forward(g, request, active).await;
    match result{Ok(v)=>v,Err(_)=>(StatusCode::BAD_GATEWAY,axum::Json(serde_json::json!({"error":{"code":"remote_transport_interrupted","message":"远程通道中断，请稍后重试"}}))).into_response()}
}
async fn forward(g: Arc<Gateway>, mut request: Request<Body>, active: Active) -> Result<Response> {
    let upgrade = request.headers().get("upgrade").is_some();
    let reusable = !upgrade && !is_audio_request(request.uri().path());
    if reusable {
        if let Some(mut connection) = crate::native_pool::take(&g.control_pool) {
            let retry = matches!(request.method().as_str(), "GET" | "HEAD").then(|| {
                let mut retry = Request::new(Body::empty());
                *retry.method_mut() = request.method().clone();
                *retry.uri_mut() = request.uri().clone();
                *retry.version_mut() = request.version();
                *retry.headers_mut() = request.headers().clone();
                retry
            });
            match connection.sender.send_request(request).await {
                Ok(response) => return Ok(crate::native_pool::response(g.control_pool.clone(), connection, response)),
                Err(error) => { request = match retry { Some(retry) => retry, None => return Err(error.into()) }; }
            }
        }
    }
    let (wire, app) = tokio::io::duplex(crate::tunnel::WINDOW);
    let cancel = g.cancel.child_token();
    let early_cancel = cancel.clone().drop_guard();
    let c = cancel.clone();
    let socket = crate::tunnel::connect_core(&g.logical_address, "api/v1/native/connect", g.outer_tls.clone()).await?;
    tokio::spawn(async move {
        let _ = crate::tunnel::bridge_client(socket, wire, c).await;
    });
    let name = rustls::pki_types::ServerName::try_from(crate::identity::core_hostname(&g.core_id))?;
    let tls = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio_rustls::TlsConnector::from(g.tls.clone()).connect(name, app),
    )
    .await??;
    let incoming = if upgrade {
        Some(hyper::upgrade::on(&mut request))
    } else {
        None
    };
    let (tls, closed) = crate::lifetime::track(tls);
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(tls)).await?;
    let activity = Arc::new(Mutex::new(None));
    let idle = activity.clone();
    let c = cancel.clone();
    tokio::spawn(async move {
        let _active = active;
        let _cancel_on_close = early_cancel;
        tokio::select! {_=async{let _=connection.with_upgrades().await;let _=closed.await;}=>{},_=c.cancelled()=>{},_=crate::native_pool::idle(idle),if reusable=>{}}
        c.cancel();
    });
    let mut response = sender.send_request(request).await?;
    if reusable {
        return Ok(crate::native_pool::response(g.control_pool.clone(), crate::native_pool::Connection {sender, activity}, response));
    }
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        if let Some(incoming) = incoming {
            let outgoing = hyper::upgrade::on(&mut response);
            tokio::spawn(async move {
                if let (Ok(a), Ok(b)) = tokio::join!(incoming, outgoing) {
                    let _ = tokio::io::copy_bidirectional(
                        &mut hyper_util::rt::TokioIo::new(a),
                        &mut hyper_util::rt::TokioIo::new(b),
                    )
                    .await;
                }
            });
        }
    }
    let (parts, body) = response.into_parts();
    Ok(Response::from_parts(parts, Body::new(body)))
}
pub fn status(address: &str) -> serde_json::Value {
    let all = gateways().lock().unwrap();
    if let Some(g) = all.values().find(|g| g.address == address) {
        serde_json::json!({"remote":g.remote,"path":"core","end_to_end":true,"core_id":g.core_id})
    } else {
        serde_json::json!({"remote":false})
    }
}
pub fn is_audio_request(path:&str)->bool {
    let path=path.trim_start_matches('/');
    let path=path.strip_prefix("api/v1/").unwrap_or(path);
    let mut parts=path.split('/');
    match parts.next() {
        Some("media"|"pcm"|"native-pcm"|"native-flac"|"mobile-opus"|"transcode")=>true,
        Some("cache"|"downloads")=>parts.next().is_some_and(|id|!id.is_empty())&&parts.next()==Some("file")&&parts.next().is_none(),
        _=>false,
    }
}
pub async fn serve_core(
    io: tokio::io::DuplexStream,
    router: Router,
    tls: Option<Arc<rustls::ServerConfig>>,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    if let Some(tls) = tls {
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio_rustls::TlsAcceptor::from(tls).accept(io),
        )
        .await??;
        serve_io(stream, router, cancel).await?;
    } else {
        serve_io(io, router, cancel).await?;
    }
    Ok(())
}
async fn serve_io<I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static>(
    io: I,
    router: Router,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<()> {
    let (io, closed) = crate::lifetime::track(io);
    let service = hyper_util::service::TowerToHyperService::new(router);
    tokio::select! {v=async{hyper::server::conn::http1::Builder::new().timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(std::time::Duration::from_secs(15)).serve_connection(hyper_util::rt::TokioIo::new(io),service).with_upgrades().await?;let _=closed.await;Ok::<_,anyhow::Error>(())}=>v,_=cancel.cancelled()=>Ok(())}
}

#[cfg(test)]mod route_tests {
    use super::*;
    #[tokio::test]
    async fn controls_reuse_core_tls_and_audio_does_not_occupy_the_pool() {
        use axum::{extract::WebSocketUpgrade, routing::get};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let identity=crate::identity::Identity::create();let core=uuid::Uuid::new_v4().to_string();
        let tls=identity.tls(&core).unwrap();
        let count=Arc::new(AtomicUsize::new(0));let counted=count.clone();
        let inner=Router::new().fallback(||async {axum::Json(serde_json::json!({"ok":true}))});
        let cancel=tokio_util::sync::CancellationToken::new();let stop=cancel.clone();
        let router=Router::new().route("/api/v1/native/connect",get(move |ws:WebSocketUpgrade| {
            let tls=tls.clone();let inner=inner.clone();let cancel=stop.child_token();let count=counted.clone();
            async move {ws.on_upgrade(move |socket|async move {
                count.fetch_add(1,Ordering::SeqCst);
                let (a,b)=tokio::io::duplex(crate::tunnel::WINDOW);
                let wire=crate::tunnel::bridge_server(socket,a,cancel.clone());
                let server=serve_core(b,inner,Some(tls),cancel.clone());
                let _=crate::tunnel::serve_and_drain(wire,server,cancel).await;
            })}
        }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
        let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        let gateway=open_core(&format!("http://127.0.0.1:{port}"),core,identity.public_key(),None,false).await.unwrap();
        let http=reqwest::Client::builder().no_proxy().build().unwrap();
        for method in [reqwest::Method::GET,reqwest::Method::POST,reqwest::Method::GET] {
            assert_eq!(http.request(method,format!("{}/api/v1/control",gateway.address)).send().await.unwrap().json::<serde_json::Value>().await.unwrap()["ok"],true);
        }
        assert_eq!(count.load(Ordering::SeqCst),1,"三次控制只建立一条 Core TLS 连接");
        http.get(format!("{}/api/v1/media/test",gateway.address)).send().await.unwrap().bytes().await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst),2,"音频单独建流");
        http.get(format!("{}/api/v1/control",gateway.address)).send().await.unwrap().bytes().await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst),2,"音频结束后控制继续复用");
        gateway.cancel.cancel();cancel.cancel();server.abort();
    }
    #[test]fn every_audio_export_uses_the_same_gate_and_metadata_does_not(){
        for path in ["/api/v1/media/id","/api/v1/pcm/id","native-pcm/id","native-flac/id","mobile-opus/id","transcode/id","/api/v1/cache/id/file","downloads/id/file"] {assert!(is_audio_request(path),"{path}");}
        for path in ["/api/v1/cache","cache/id/export","library/tracks/id/audio-info","covers/id","downloads/id","cache/id/file/other"] {assert!(!is_audio_request(path),"{path}");}
    }
}
