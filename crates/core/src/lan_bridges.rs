//! LAN membership removes the pairing dialog, not the device permission boundary.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    body::{to_bytes, Body},
    extract::{ConnectInfo, Request, State},
    http::Method,
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::net::SocketAddr;

fn local(
    peer: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    proxy: Option<axum::Extension<crate::remote_access::proxy::ProxiedIngress>>,
) -> ApiResult<String> {
    if proxy.is_some() {
        return Err(ApiError::forbidden());
    }
    let peer = peer.ok_or_else(ApiError::forbidden)?.0 .0.ip();
    if !rmusic_discovery::on_link(peer) {
        return Err(ApiError::forbidden());
    }
    Ok(peer.to_string())
}
pub async fn challenge(
    State(s): State<AppState>,
    peer: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    proxy: Option<axum::Extension<crate::remote_access::proxy::ProxiedIngress>>,
) -> ApiResult<Json<Value>> {
    let ip = local(peer, proxy)?;
    if s.setting("lan_bridge_auto_join", json!(true)).await == json!(false) {
        return Err(ApiError::forbidden());
    }
    let challenge = s.ticket_for(format!("lan-bridge:{ip}"), 60, None).await;
    Ok(Json(
        json!({"core_id":s.core_id.as_str(),"core_public_key":s.remote.identity.public_key(),"challenge":challenge,"expires":chrono::Utc::now().timestamp()+60}),
    ))
}
pub async fn enroll(
    State(s): State<AppState>,
    peer: Option<axum::Extension<ConnectInfo<SocketAddr>>>,
    proxy: Option<axum::Extension<crate::remote_access::proxy::ProxiedIngress>>,
    Json(v): Json<rmusic_remote::identity::Signed<rmusic_remote::lan::Enrollment>>,
) -> ApiResult<Json<Value>> {
    let ip = local(peer, proxy)?;
    if s.setting("lan_bridge_auto_join", json!(true)).await == json!(false) {
        return Err(ApiError::forbidden());
    }
    v.verify(&v.public_key)
        .map_err(|_| ApiError::bad("网桥身份签名无效"))?;
    let public = v.public_key;
    let v = v.payload;
    let now = chrono::Utc::now().timestamp();
    if v.schema != 1
        || v.core_id != s.core_id.as_str()
        || v.expires < now
        || v.expires > now + 90
        || v.endpoint_id.is_empty()
        || v.endpoint_id.len() > 200
        || v.endpoint_id.chars().any(char::is_control)
        || v.endpoint_id!=format!("lan-bridge:{}",crate::auth::hash(&public))
    {
        return Err(ApiError::bad("网桥注册信息无效"));
    }
    let challenge = s
        .tickets
        .lock()
        .await
        .remove(&crate::auth::hash(&v.challenge))
        .ok_or_else(ApiError::forbidden)?;
    if challenge.scope != format!("lan-bridge:{ip}") || challenge.expires < now {
        return Err(ApiError::forbidden());
    }
    if crate::devices::is_removed(&s, &v.endpoint_id).await? {
        return Err(ApiError::bad("网桥已被管理员移除，请先在设备设置中恢复"));
    }
    let id = v.endpoint_id.clone();
    let token = crate::auth::random_token();
    let hash = crate::auth::hash(&token);
    let expires = now + 86400;
    s.db.run(move|c|{let tx=c.transaction()?;let key=format!("lan_bridge_key:{}",crate::auth::hash(&id));
        let previous:Option<String>=tx.query_row("SELECT value FROM settings WHERE key=?1",[&key],|r|r.get(0)).optional()?;anyhow::ensure!(previous.as_ref().is_none_or(|s|s==&public),"网桥身份发生变化，已拒绝自动接管");
        let owner:String=tx.query_row("SELECT id FROM users WHERE role='admin' ORDER BY id LIMIT 1",[],|r|r.get(0))?;
        tx.execute("INSERT OR IGNORE INTO settings(key,value) VALUES(?1,?2)",params![key,public])?;
        let name=format!("LAN bridge:{id}");tx.execute("DELETE FROM sessions WHERE name=?1",[&name])?;
        tx.execute("INSERT INTO sessions(hash,user_id,expires,name,scopes,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![hash,owner,expires,name,json!(["read","control",format!("lan-bridge:{id}")]).to_string(),now])?;tx.commit()?;Ok(())
    }).await?;
    let context = format!("{}/{}", s.core_id, v.endpoint_id);
    let encrypted = rmusic_remote::lan::encrypt(
        &v.encryption_key,
        &json!({"token":token,"core_id":s.core_id.as_str(),"expires":expires}),
        context.as_bytes(),
    )?;
    Ok(Json(
        serde_json::to_value(s.remote.identity.sign(encrypted)?).map_err(anyhow::Error::from)?,
    ))
}
pub fn endpoint(a: &Auth) -> Option<&str> {
    a.scopes.iter().find_map(|s| s.strip_prefix("lan-bridge:"))
}
pub async fn access(s: &AppState, a: &Auth, request: &mut Request) -> ApiResult<()> {
    let Some(endpoint) = endpoint(a) else {
        return Ok(());
    };
    let endpoint = endpoint.to_owned();
    let path = request.uri().path();
    let parts = path
        .trim_matches('/')
        .split('/')
        .map(|s| {
            url::form_urlencoded::parse(format!("id={}", s.replace('+', "%2B")).as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let get = matches!(*request.method(), Method::GET | Method::HEAD);
    let zones = s
        .zones
        .read()
        .await
        .values()
        .filter(|z| z.endpoint_id == endpoint)
        .cloned()
        .collect::<Vec<_>>();
    let track = |id: &str| {
        zones.iter().any(|z| {
            z.track.as_ref().is_some_and(|t| t.id == id) || z.queue.iter().any(|t| t.id == id)
        })
    };
    if get && matches!(path, "/status" | "/auth/me" | "/bridge/events" | "/zones") {
        return Ok(());
    }
    if parts.len() >= 2 && parts[0] == "zones" && zones.iter().any(|z| z.id == parts[1]) {
        if get
            || parts.get(2).is_some_and(|s| {
                matches!(
                    s.as_str(),
                    "prepare" | "queue" | "progress" | "ready" | "recover" | "visualization"
                )
            })
        {
            return Ok(());
        }
    }
    if get
        && parts.len() >= 3
        && parts[..2] == ["library", "tracks"]
        && track(&parts[2])
        && parts
            .get(3)
            .is_none_or(|s| matches!(s.as_str(), "audio-info" | "ticket"))
    {
        return Ok(());
    }
    if get
        && parts.len() == 2
        && matches!(
            parts[0].as_str(),
            "media" | "pcm" | "native-pcm" | "native-flac" | "transcode"
        )
        && track(&parts[1])
    {
        return Ok(());
    }
    if get && path.starts_with("/dsp/impulses/") && path.ends_with("/file") {
        return Ok(());
    }
    if parts.len() == 3 && parts[0] == "sync" && parts[1] == "starts" && get {
        return Ok(());
    }
    if parts.len() == 4 && parts[..2] == ["sync", "starts"] && parts[3] == "final-receipts" {
        return Ok(());
    }
    if parts.len() == 3
        && parts[0] == "outputs"
        && parts[1] == endpoint
        && parts[2] == "config"
        && request.method() == Method::PUT
    {
        return Ok(());
    }
    if path == "/stats/offline" && request.method() == Method::POST {
        return Ok(());
    }
    if path == "/devices/register" && request.method() == Method::POST {
        let bytes = to_bytes(std::mem::replace(request.body_mut(), Body::empty()), 65536)
            .await
            .map_err(|_| ApiError::bad("网桥设备声明过大"))?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("网桥设备声明无效"))?;
        let expected = format!("bridge-{}", crate::auth::hash(&endpoint));
        let valid = value["id"] == expected
            && value["outputs"]
                .as_array()
                .is_some_and(|items| items.len() == 1 && items[0]["id"] == endpoint);
        *request.body_mut() = Body::from(bytes);
        if valid {
            return Ok(());
        }
    }
    Err(ApiError::forbidden())
}
