use crate::{
    config::Configuration,
    identity::{Identity, Signed},
    vault::Vault,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub core_id: String,
    pub revision: u64,
    pub endpoints: Vec<String>,
    // Keep the v1 wire field so existing clients can verify signed manifests.
    // Old snapshots may contain a node. It is never published by this Core.
    pub relay: Option<LegacyManifestNode>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyManifestNode {
    id: String,
    url: String,
    public_key: String,
    p2p_domain: String,
    tcp_port: u16,
    udp_port: u16,
}
impl Manifest {
    pub fn direct_endpoints(&self) -> impl Iterator<Item = &String> {
        let retired = self.relay.as_ref().map(|node| node.url.trim_end_matches('/'));
        self.endpoints.iter().filter(move |address| Some(address.trim_end_matches('/')) != retired)
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Secrets {
    pub core_cloudflare: Option<String>,
    pub core_eab: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct VerifiedIngress { configuration: crate::config::Ingress, url: String }
/// Secrets deliberately have no Debug implementation and are never API responses.
pub struct Runtime {
    pub root: PathBuf,
    pub identity: Arc<Identity>,
    pub vault: Arc<Vault>,
    configuration: tokio::sync::RwLock<Configuration>,
    verified_ingress: tokio::sync::RwLock<Option<VerifiedIngress>>,
    verified_direct_urls: tokio::sync::RwLock<Vec<String>>,
    manifest_state: tokio::sync::Mutex<Option<Manifest>>,
    pub changed: tokio::sync::watch::Sender<u64>,
    pub manifest_changed: tokio::sync::watch::Sender<()>,
    pub cancel: tokio_util::sync::CancellationToken,
    pub status: tokio::sync::RwLock<serde_json::Value>,
    pub update_lock: tokio::sync::Mutex<()>,
}
impl Runtime {
    pub fn open(root: &Path) -> Result<Self> {
        crate::vault::private_directory(root)?;
        let configuration: Configuration = match std::fs::read(root.join("configuration.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Configuration::default(),
            Err(e) => return Err(e.into()),
        };
        configuration.validate()?;
        let (changed, _) = tokio::sync::watch::channel(configuration.revision);
        Ok(Self {
            root: root.into(),
            identity: Arc::new(Identity::open(root)?),
            vault: Arc::new(Vault::open(&root.join("secrets"))?),
            configuration: tokio::sync::RwLock::new(configuration),
            verified_ingress: tokio::sync::RwLock::new(std::fs::read(root.join("verified-ingress.json")).ok().filter(|v|v.len()<=32768).and_then(|v|serde_json::from_slice(&v).ok())),
            verified_direct_urls: tokio::sync::RwLock::new(std::fs::read(root.join("verified-direct-urls.json")).ok()
                .filter(|v|v.len()<=32768).and_then(|v|serde_json::from_slice(&v).ok()).unwrap_or_default()),
            manifest_state: tokio::sync::Mutex::new(match std::fs::read(root.join("connection-manifest.json")) {
                Ok(bytes) => { ensure!(bytes.len() <= 32768, "持久连接清单过大"); Some(serde_json::from_slice(&bytes)?) },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            }),
            changed,
            manifest_changed: tokio::sync::watch::channel(()).0,
            cancel: tokio_util::sync::CancellationToken::new(),
            status: tokio::sync::RwLock::new(serde_json::json!({"state":"disabled"})),
            update_lock: tokio::sync::Mutex::new(()),
        })
    }
    pub async fn configuration(&self) -> Configuration {
        self.configuration.read().await.clone()
    }
    /// The caller holds update_lock across secret updates and final commit.
    pub async fn save(&self, mut next: Configuration, expected: u64) -> Result<u64> {
        next.validate()?;
        let mut current = self.configuration.write().await;
        ensure!(current.revision == expected, "连接设置已变化，请刷新后重试");
        next.revision = expected
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("配置版本溢出"))?;
        // A removed address must be probed again if an administrator later adds it back.
        {
            let mut checked=self.verified_direct_urls.write().await;
            let kept:Vec<_>=checked.iter().filter(|url| next.direct_urls.as_deref().unwrap_or_default().contains(url)).cloned().collect();
            if kept!=*checked {
                crate::vault::atomic_private(&self.root.join("verified-direct-urls.json"),&serde_json::to_vec(&kept)?)?;
                *checked=kept;
            }
        }
        crate::vault::atomic_private(
            &self.root.join("configuration.json"),
            &serde_json::to_vec_pretty(&next)?,
        )?;
        *current = next;
        let version = current.revision;
        self.changed.send_replace(version);
        self.manifest_changed.send_replace(());
        Ok(version)
    }
    pub async fn mark_ingress_verified(&self, expected: &crate::config::Ingress, url: String) -> Result<()> {
        let current = self.configuration.read().await;
        ensure!(&current.core == expected, "网络配置已变化，旧探测结果未发布");
        crate::config::https_url(&url)?;
        if self.verified_ingress.read().await.as_ref().is_some_and(|v| &v.configuration == expected && v.url == url) { return Ok(()); }
        let verified = VerifiedIngress { configuration: expected.clone(), url };
        crate::vault::atomic_private(&self.root.join("verified-ingress.json"), &serde_json::to_vec(&verified)?)?;
        *self.verified_ingress.write().await = Some(verified);
        self.manifest_changed.send_replace(());
        Ok(())
    }
    pub async fn publish_direct_checks(&self, revision:u64, verified:Vec<String>) -> Result<()> {
        let config=self.configuration.read().await;
        ensure!(revision==config.revision,"直连配置已更新");
        let configured=config.direct_urls.as_deref().unwrap_or_default();
        ensure!(verified.iter().all(|u|configured.contains(u)),"探测地址不属于当前配置");
        let mut old=self.verified_direct_urls.write().await;
        if *old!=verified {
            crate::vault::atomic_private(&self.root.join("verified-direct-urls.json"),&serde_json::to_vec(&verified)?)?;
            *old=verified;
            self.manifest_changed.send_replace(());
        }
        Ok(())
    }
    pub async fn manifest(&self, core: &str, legacy: Option<String>) -> Result<Signed<Manifest>> {
        self.identity.sign(self.manifest_snapshot(core, legacy).await?)
    }
    pub async fn manifest_snapshot(&self, core: &str, legacy: Option<String>) -> Result<Manifest> {
        // Keep the generation stable through publication. A late request cannot
        // publish an older snapshot after a newer configuration has committed.
        let config = self.configuration.read().await;
        let mut endpoints = Vec::new();
        let verified = self.verified_ingress.read().await.clone().filter(|v|v.configuration == config.core);
        let checked=self.verified_direct_urls.read().await;
        for address in config.direct_urls.as_deref().unwrap_or_default() {
            let managed = verified.as_ref().is_some_and(|v| v.url.trim_end_matches('/') == address.trim_end_matches('/'));
            if (managed || checked.contains(address)) && !endpoints.iter().any(|u: &String|u.trim_end_matches('/')==address.trim_end_matches('/')) {
                endpoints.push(address.to_owned());
            }
        }
        drop(checked);
        if let Some(value) = verified.as_ref() {
            if !endpoints.iter().any(|url| url.trim_end_matches('/') == value.url.trim_end_matches('/')) {
                endpoints.push(value.url.clone());
            }
        }
        if let Some(url) = legacy {
            if let Ok(value) = url::Url::parse(&url) {
                if matches!(value.scheme(), "https" | "http")
                    && value.username().is_empty()
                    && value.password().is_none()
                    && value.query().is_none()
                    && value.fragment().is_none()
                    && !matches!(value.host(),Some(url::Host::Ipv6(ip)) if ip.is_unicast_link_local() || ip.is_loopback())
                    && !endpoints.contains(&url)
                {
                    endpoints.push(url);
                }
            }
        }
        let floor = config.revision.checked_mul(2).and_then(|v|v.checked_add(u64::from(verified.is_some()))).context("连接配置版本溢出")?;
        let mut manifest = Manifest {
            schema: 1,
            core_id: core.into(),
            revision: floor,
            endpoints,
            relay: None,
        };
        let mut state = self.manifest_state.lock().await;
        if let Some(previous) = state.as_ref() {
            ensure!(previous.schema == 1 && previous.core_id == core, "持久连接清单身份不匹配");
            let unchanged = previous.endpoints == manifest.endpoints && previous.relay == manifest.relay;
            manifest.revision = if unchanged { previous.revision.max(floor) } else {
                previous.revision.checked_add(1).context("连接清单版本溢出")?.max(floor)
            };
        }
        if state.as_ref() != Some(&manifest) {
            crate::vault::atomic_private(&self.root.join("connection-manifest.json"), &serde_json::to_vec(&manifest)?)?;
            *state = Some(manifest.clone());
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn existing_configuration_keeps_core_ddns_and_retires_the_relay_manifest() {
        let root=tempfile::tempdir().unwrap();
        let path=root.path();
        let old_node=serde_json::json!({
            "id":"old-node","url":"https://retired.example","public_key":"old-key",
            "p2p_domain":"p2p.example","tcp_port":11010,"udp_port":11010
        });
        std::fs::write(path.join("configuration.json"),serde_json::to_vec(&serde_json::json!({
            "revision":4,
            "direct_urls":["https://primary.example/music"],
            "core":{"mode":"managed_https","domain":"music.example","ports":{"https_public":19443,"p2p_tcp_public":11010},
                "p2p_domain":"unused.example","ddns":{"enabled":true,"zone_id":"abcd","auto_ipv6":true}},
            "active":old_node
        })).unwrap()).unwrap();
        std::fs::write(path.join("connection-manifest.json"),serde_json::to_vec(&serde_json::json!({
            "schema":1,"core_id":"core","revision":27,"endpoints":["https://retired.example"],"relay":old_node
        })).unwrap()).unwrap();
        let runtime=Runtime::open(path).unwrap();
        let core=runtime.configuration().await.core;
        assert!(core.ddns.enabled && core.ddns.auto_ipv6);
        assert_eq!(core.ports.https_public,19443);
        runtime.publish_direct_checks(4,vec!["https://primary.example/music".into()]).await.unwrap();
        runtime.mark_ingress_verified(&core,"https://music.example:19443".into()).await.unwrap();
        let upgraded=runtime.manifest("core",None).await.unwrap();
        upgraded.verify(&runtime.identity.public_key()).unwrap();
        assert_eq!(upgraded.payload.endpoints,["https://primary.example/music","https://music.example:19443"]);
        assert!(upgraded.payload.relay.is_none());
        assert!(upgraded.payload.revision > 27);
    }
    #[tokio::test]
    async fn stale_edit_cannot_override_another_save() {
        let d = tempfile::tempdir().unwrap();
        let r = Runtime::open(d.path()).unwrap();
        let c = r.configuration().await;
        assert_eq!(r.save(c.clone(), 0).await.unwrap(), 1);
        assert!(r.save(c, 0).await.is_err());
        let a = r
            .manifest("core", Some("https://example.com/core".into()))
            .await
            .unwrap();
        a.verify(&r.identity.public_key()).unwrap();
        assert_eq!(a.payload.revision, 2);
        let reopened = Runtime::open(d.path()).unwrap();
        assert_eq!(reopened.configuration().await.revision, 1);
        assert_eq!(reopened.identity.public_key(), r.identity.public_key());
    }    #[tokio::test]
    async fn desired_endpoint_is_not_published_until_verified_and_old_generation_cannot_publish() {
        let temp = tempfile::tempdir().unwrap(); let runtime = Runtime::open(temp.path()).unwrap();
        let mut config = runtime.configuration().await; config.core.mode = crate::config::IngressMode::ReverseProxy;
        config.core.public_url = Some("https://new.example.com/music".into()); runtime.save(config.clone(),0).await.unwrap();
        assert!(runtime.manifest("core",None).await.unwrap().payload.endpoints.is_empty());
        runtime.mark_ingress_verified(&config.core,"https://new.example.com/music".into()).await.unwrap();
        let published = runtime.manifest("core",None).await.unwrap().payload;
        assert_eq!(published.endpoints,vec!["https://new.example.com/music"]); assert_eq!(published.revision,3);
        let old = config.core.clone(); config.core.public_url = Some("https://replacement.example.com/music".into()); runtime.save(config,1).await.unwrap();
        assert!(runtime.mark_ingress_verified(&old,"https://new.example.com/music".into()).await.is_err());
        assert!(runtime.manifest("core",None).await.unwrap().payload.endpoints.is_empty());
    }
    #[tokio::test]
    async fn multiple_direct_proxies_preserve_order_identity_checks_and_revision() {
        let root=tempfile::tempdir().unwrap();
        let runtime=Runtime::open(root.path()).unwrap();
        let mut config=runtime.configuration().await;
        let a=String::from("https://proxy.example/music");
        let b=String::from("https://[2001:db8::1]:8443/core");
        config.direct_urls=Some(vec![a.clone(),b.clone()]);
        runtime.save(config.clone(),0).await.unwrap();
        assert!(runtime.manifest("core",None).await.unwrap().payload.endpoints.is_empty());
        runtime.publish_direct_checks(1,vec![b.clone(),a.clone()]).await.unwrap();
        let both=runtime.manifest("core",None).await.unwrap().payload;
        assert_eq!(both.endpoints,vec![a.clone(),b.clone()]);
        config.direct_urls=Some(vec![b.clone(),a.clone()]);
        runtime.save(config.clone(),1).await.unwrap();
        let reordered=runtime.manifest("core",None).await.unwrap().payload;
        assert_eq!(reordered.endpoints,vec![b.clone(),a.clone()]);
        assert!(reordered.revision>both.revision);
        config.direct_urls=Some(vec![b.clone()]);runtime.save(config,2).await.unwrap();
        assert!(runtime.publish_direct_checks(1,vec![a]).await.is_err());
        drop(runtime);
        let next=Runtime::open(root.path()).unwrap().manifest("core",None).await.unwrap().payload;
        assert_eq!(next.endpoints,vec![b]);assert!(next.revision>reordered.revision);
    }
    #[tokio::test]
    async fn manifest_versions_remain_monotonic_when_endpoints_change_and_after_restart() {
        let temp=tempfile::tempdir().unwrap();let runtime=Runtime::open(temp.path()).unwrap();
        let first=runtime.manifest("core",Some("https://old.example/music".into())).await.unwrap().payload;
        let next=runtime.manifest("core",Some("https://new.example:9443/music".into())).await.unwrap().payload;
        assert!(next.revision>first.revision);
        drop(runtime);
        let runtime=Runtime::open(temp.path()).unwrap();
        assert_eq!(runtime.manifest("core",Some("https://new.example:9443/music".into())).await.unwrap().payload,next);
        let changed=runtime.manifest("core",None).await.unwrap().payload;
        assert!(changed.revision>next.revision);assert!(changed.endpoints.is_empty());
    }

}
