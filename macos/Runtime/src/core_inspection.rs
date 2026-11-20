//! Credential-free login preview, cancellable like other bounded background requests.
use anyhow::Result;
use futures_util::future::BoxFuture;
use rmusic_bridge::connection::ConnectionProfile;
use serde_json::Value;
pub fn task(body:Value)->Result<BoxFuture<'static,Result<Value>>> {
    let mut profile:ConnectionProfile=serde_json::from_value(body)?;
    profile.alternatives.clear();
    // This probe only requests public /status and never carries login data or
    // tickets. Preserve its transport/TLS cause so the UI can distinguish a
    // certificate failure from a blocked local-network connection.
    Ok(Box::pin(async move{if rmusic_bridge::connection::standalone_bridge() { profile.resolve_lan_only().await?.inspect_primary().await } else { profile.inspect_primary().await }.map_err(|error|anyhow::anyhow!("Core 地址检查失败：{error:#}"))}))
}
