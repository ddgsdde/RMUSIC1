//! No pairing dialog: discovery + on-link challenge + per-device signed identity.
use crate::connection::ConnectionProfile;
use anyhow::{ensure, Context, Result};
use rmusic_remote::{
    identity::{Identity, Signed},
    lan::{Capsule, Enrollment},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, time::Duration};
#[derive(Serialize, Deserialize)]
struct Saved {
    profile: ConnectionProfile,
    token: String,
    expires: i64,
}
pub fn endpoint_id(root:&Path)->Result<String>{use sha2::{Digest,Sha256};rmusic_remote::vault::private_directory(root)?;let identity=Identity::open(root)?;Ok(format!("lan-bridge:{:x}",Sha256::digest(identity.public_key().as_bytes())))}
pub async fn connect(
    root: &Path,
    endpoint: &str,
    name: &str,
    discovery: &rmusic_discovery::Discovery,
    preferred: Option<&ConnectionProfile>,
) -> Result<(ConnectionProfile, String)> {
    rmusic_remote::vault::private_directory(root)?;
    let identity = Identity::open(root)?;
    let vault = rmusic_remote::vault::Vault::open(&root.join("secrets"))?;
    let saved = vault.get::<Saved>("core")?;
    let profile = if let Some(saved) = saved.as_ref() {
        saved.profile.clone()
    } else if let Some(profile) = preferred {
        profile.clone()
    } else {
        let found = discovery
            .snapshot()
            .into_iter()
            .filter(|s| s.kind == "core")
            .find(|s| s.addresses.iter().any(|url| local_url(url)))
            .context("等待局域网 Core…")?;
        let address = found
            .addresses
            .into_iter()
            .find(|url| local_url(url))
            .context("Core 没有可用局域网地址")?;
        serde_json::from_value(
            serde_json::json!({"id":"lan-bridge","name":name,"address":address,"core_id":found.id}),
        )?
    };
    ensure!(
        local_url(&profile.address),
        "独立网桥仅支持当前物理局域网的 Core"
    );
    if let Some(saved) = saved {
        if saved.expires > unix() + 300 {
            let selected = saved.profile.resolve_lan_only().await?;
            return Ok((selected, saved.token));
        }
    }
    let http = profile.http()?;
    let challenge: Value = http
        .get(profile.endpoint("api/v1/lan/bridge-challenge")?)
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let core = challenge["core_id"].as_str().context("Core 身份缺失")?;
    let key = challenge["core_public_key"]
        .as_str()
        .context("Core 公钥缺失")?;
    ensure!(
        profile.core_id.as_deref().is_none_or(|id| id == core)
            && profile.core_public_key.as_deref().is_none_or(|v| v == key),
        "Core 身份变化，已停止自动接入"
    );
    let secret = rmusic_remote::lan::secret();
    let request = identity.sign(Enrollment {
        schema: 1,
        endpoint_id: endpoint.into(),
        core_id: core.into(),
        challenge: challenge["challenge"].as_str().context("挑战缺失")?.into(),
        expires: challenge["expires"].as_i64().context("挑战时效缺失")?,
        encryption_key: rmusic_remote::lan::public_key(&secret)?,
    })?;
    let response: Signed<Capsule> = http
        .post(profile.endpoint("api/v1/lan/bridges")?)
        .json(&request)
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    response.verify(key)?;
    let context = format!("{core}/{endpoint}");
    let value: Value = rmusic_remote::lan::decrypt(&secret, response.payload, context.as_bytes())?;
    let mut profile = profile;
    profile.core_id = Some(core.into());
    profile.core_public_key = Some(key.into());
    let saved = Saved {
        profile: profile.clone(),
        token: value["token"].as_str().context("设备凭据缺失")?.into(),
        expires: value["expires"].as_i64().context("凭据时效缺失")?,
    };
    vault.put("core", &saved)?;
    Ok((profile, saved.token))
}
fn unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn local_url(address: &str) -> bool {
    url::Url::parse(address).ok().is_some_and(|url| {
        url.host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(rmusic_discovery::bridge_on_link)
    })
}
