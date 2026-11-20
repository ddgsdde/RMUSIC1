//! Authenticated, cache-free application-path measurements. Never touch the music library or audio cache.
use crate::{auth::Auth,error::ApiResult,remote_access::{proxy::HttpsProxy,SecureIngress}};
use axum::{body::Bytes,extract::ConnectInfo,http::header,response::{IntoResponse,Response},Extension,Json};
use rand::RngCore;
use serde_json::{json};
use std::{net::SocketAddr,sync::OnceLock};

const SIZE:usize=1024*1024;
pub async fn ping(a:Auth, proxy:Option<Extension<HttpsProxy>>,
    secure:Option<Extension<SecureIngress>>, peer:Option<Extension<ConnectInfo<SocketAddr>>>)->ApiResult<Response>{
    a.require("read")?;
    let ingress=if proxy.is_some(){"reverse_proxy"}else if secure.is_some(){"secure"}else{"direct"};
    let on_link=peer.is_some_and(|p|rmusic_discovery::bridge_on_link(p.0.0.ip()));
    Ok(([(header::CACHE_CONTROL,"no-store")],Json(json!({"ingress":ingress,"on_link":on_link}))).into_response())
}
pub async fn download(a:Auth)->ApiResult<Response>{
    a.require("read")?;
    static DATA:OnceLock<Bytes>=OnceLock::new();
    let data=DATA.get_or_init(||{let mut v=vec![0;SIZE];rand::thread_rng().fill_bytes(&mut v);Bytes::from(v)});
    Ok(([(header::CACHE_CONTROL,"no-store, no-transform"),(header::CONTENT_TYPE,"application/octet-stream")],data.clone()).into_response())
}
