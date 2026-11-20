//! Forwarded headers are data unless the socket peer is explicitly trusted.
use crate::state::AppState;
use axum::{
    extract::{ConnectInfo, Request, State},
    middleware::Next,
    response::Response,
};
use std::net::{IpAddr, SocketAddr};
#[derive(Clone, Copy, Debug)]
pub struct ClientAddress(pub IpAddr);
#[derive(Clone,Copy)]pub struct ProxiedIngress;
#[derive(Clone,Copy)] pub struct HttpsProxy;
pub async fn boundary(State(s): State<AppState>, mut request: Request, next: Next) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|v| v.0.ip());
    let config = s.remote.configuration().await;
    let trusted = peer.is_some_and(|ip| config.core.trusted_proxy_addresses.contains(&ip));
    let mut client = peer;
    let forwarding_headers = request.headers().keys().any(|name| {
        matches!(name.as_str(),"via"|"forwarded"|"x-real-ip"|"x-rmusic-transport") || name.as_str().starts_with("x-forwarded-")
    });
    let capture = request.extensions().get::<super::local_proxy::Peer>();
    let trusted = if trusted && forwarding_headers {
        match capture {
            Some(flow) => flow.owner().await != super::local_proxy::Owner::Unverified,
            None => trusted,
        }
    } else { trusted };
    if trusted || forwarding_headers {
        request.extensions_mut().insert(ProxiedIngress);
    }
    if trusted {
        if request
            .headers()
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            == Some("https")
        {
            request.extensions_mut().insert(super::SecureIngress);
            request.extensions_mut().insert(HttpsProxy);
        }
        if let Some(chain) = request
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .filter(|s| s.len() <= 1024)
        {
            if let Ok(ips) = chain
                .split(',')
                .map(str::trim)
                .map(str::parse::<IpAddr>)
                .collect::<Result<Vec<_>, _>>()
            {
                if ips.len() <= 16 {
                    for ip in ips.into_iter().rev() {
                        client = Some(ip);
                        if !config.core.trusted_proxy_addresses.contains(&ip) {
                            break;
                        }
                    }
                }
            }
        }
    }
    // Real plain-HTTP listeners must never accept raw account credentials.
    // TLS listeners and the decrypted inner router already carry SecureIngress.
    let route = request.uri().path().strip_prefix(s.config.base_path.trim_end_matches('/'))
        .and_then(|path| path.strip_prefix("/api/v1/")).unwrap_or("");
    let account_write = request.method() != axum::http::Method::GET &&
        (route == "setup" || route.starts_with("auth/") || route == "users" || route.starts_with("users/"));
    if account_write && request.extensions().get::<super::local_proxy::Peer>().is_some()
        && request.extensions().get::<super::SecureIngress>().is_none() {
        use axum::response::IntoResponse;
        return (axum::http::StatusCode::UPGRADE_REQUIRED,axum::Json(serde_json::json!({
            "error":{"code":"encrypted_login_required","message":"账号操作需要加密连接，请刷新页面或更新客户端。"}
        }))).into_response();
    }
    for name in [
        "forwarded",
        "x-forwarded-for",
        "x-real-ip",
        "x-forwarded-proto",
        "x-rmusic-transport",
    ] {
        request.headers_mut().remove(name);
    }
    if let Some(client) = client {
        request.extensions_mut().insert(ClientAddress(client));
    }
    next.run(request).await
}
