use anyhow::{bail, Context, Result};
use futures_util::{stream::FuturesUnordered, StreamExt};
mod tls;
mod sync;
mod diagnostics;
mod events;
pub use events::ConnectionEvents;
pub use sync::initialize as initialize_connection_cache;
pub use sync::ConnectionSync;
#[cfg(test)]
mod tls_tests;
#[cfg(test)]
use rustls::{
    client::danger::ServerCertVerifier,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use url::Url;
static STANDALONE_BRIDGE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
pub fn enable_standalone_bridge() { let _ = STANDALONE_BRIDGE.set(()); }
pub fn standalone_bridge() -> bool { STANDALONE_BRIDGE.get().is_some() }

pub fn remote_status(address: &str) -> serde_json::Value {
    rmusic_remote::native::status(address)
}
pub fn remote_gateway(address: &str) -> bool {
    rmusic_remote::native::is_remote(address)
}
pub fn logical_address(address: &str) -> String {
    rmusic_remote::native::logical_address(address)
}
async fn physical_lan_route(address: &str) -> bool {
    let Ok(url) = Url::parse(&logical_address(address)) else { return false };
    match url.host() {
        Some(url::Host::Ipv4(ip)) => rmusic_discovery::bridge_on_link(ip.into()),
        Some(url::Host::Ipv6(ip)) => rmusic_discovery::bridge_on_link(ip.into()),
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Domain(host)) if host.ends_with(".local") || !host.contains('.') => {
            let port = url.port_or_known_default().unwrap_or(0);
            let Ok(Ok(ips)) = tokio::time::timeout(Duration::from_millis(500), tokio::net::lookup_host((host, port))).await else { return false };
            let mut ips = ips.peekable();
            ips.peek().is_some() && ips.all(|a| rmusic_discovery::bridge_on_link(a.ip()))
        }
        _ => false,
    }
}

pub fn latency_sensitive_read(path:&str)->bool {
    let (route,query)=path.split_once('?').unwrap_or((path,""));
    matches!(route,"api/v1/library/home"|"api/v1/library/search"|"api/v1/search")
        || route=="api/v1/recommendations/discover"&&query.split('&').any(|part|part=="quick=true")
}

#[cfg(test)]
mod interactive_tests {
    use super::*;
    use axum::{Router,Json,routing::get};

    async fn serve(router:Router)->(String,tokio::task::JoinHandle<()>) {
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address=format!("http://{}",listener.local_addr().unwrap());
        (address,tokio::spawn(async move{axum::serve(listener,router).await.unwrap();}))
    }

    #[tokio::test]
    async fn slow_interactive_read_uses_only_an_alternative_with_the_same_identity() {
        let (primary,first)=serve(Router::new().route("/api/v1/library/home",get(||async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            "slow"
        }))).await;
        let (fallback,second)=serve(Router::new()
            .route("/api/v1/status",get(||async{Json(serde_json::json!({"core_id":"same-core"}))}))
            .route("/api/v1/library/home",get(|headers:axum::http::HeaderMap|async move {
                assert_eq!(headers["authorization"],"Bearer fixture");
                "ready"
            }))).await;
        let profile=ConnectionProfile{
            id:"fixture".into(),name:"fixture".into(),address:primary,certificate_sha256:None,
            skip_certificate_verification:false,core_id:Some("same-core".into()),core_public_key:None,
            alternatives:vec![CoreAddress{from_core:None,address:fallback.clone(),certificate_sha256:None,skip_certificate_verification:false}],
        };
        let path="api/v1/library/home";
        let request=profile.http().unwrap().get(profile.endpoint(path).unwrap()).bearer_auth("fixture");
        let (response,route)=tokio::time::timeout(Duration::from_secs(4),profile.interactive_read(&profile,path,"fixture",request)).await.unwrap().unwrap();
        assert_eq!(route.unwrap().address,fallback);
        assert_eq!(response.text().await.unwrap(),"ready");
        first.abort();
        second.abort();
    }

    #[tokio::test]
    async fn a_transient_http_error_switches_routes_only_after_a_successful_read() {
        use axum::http::StatusCode;
        let path = "api/v1/library/home";
        let (primary, first) = serve(Router::new().route("/api/v1/library/home",
            get(|| async { StatusCode::BAD_GATEWAY }))).await;
        for status in [StatusCode::OK, StatusCode::SERVICE_UNAVAILABLE] {
            let (address, server) = serve(Router::new()
                .route("/api/v1/status", get(|| async { Json(serde_json::json!({"core_id":"same-core"})) }))
                .route("/api/v1/library/home", get(move || async move { status }))).await;
            let profile: ConnectionProfile = serde_json::from_value(serde_json::json!({
                "id":"retry-fixture","name":"QA","address":primary,"core_id":"same-core",
                "alternatives":[{"address":address}]
            })).unwrap();
            let request = profile.http().unwrap().get(profile.endpoint(path).unwrap()).bearer_auth("fixture");
            let (response, route) = profile.interactive_read(&profile, path, "fixture", request).await.unwrap();
            if status.is_success() {
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(route.unwrap().address, address);
            } else {
                assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
                assert!(route.is_none());
            }
            server.abort();
        }
        first.abort();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreAddress {
    /// None is an unlabelled address saved by a client predating signed/source-separated fallbacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_core: Option<bool>,
    pub address: String,
    #[serde(default)]
    pub certificate_sha256: Option<String>,
    /// Explicit, default-off trust exception for this exact configured address.
    #[serde(default)]
    pub skip_certificate_verification: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    /// http(s)://host:port/optional-prefix, with no embedded password or query.
    pub address: String,
    /// Existing leaf pins remain readable; new clients may explicitly opt out of PKI checks.
    #[serde(default)]
    pub certificate_sha256: Option<String>,
    /// Explicit, default-off trust exception for this exact configured address.
    #[serde(default)]
    pub skip_certificate_verification: bool,
    #[serde(default)]
    pub alternatives: Vec<CoreAddress>,
    #[serde(default)]
    pub core_id: Option<String>,
    /// Stable Ed25519 Core identity, independent of HTTPS certificate renewal.
    #[serde(default)]
    pub core_public_key: Option<String>,
}
#[derive(Debug)]
struct MissingTrustedKey;
impl std::fmt::Display for MissingTrustedKey {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
        f.write_str("此 HTTP 入口不在物理局域网，当前配置尚未绑定 Core 公钥。请先通过可信 HTTPS 或在同一局域网登录一次。")
    }
}
impl std::error::Error for MissingTrustedKey {}

impl ConnectionProfile {
    /// Only verified Core signatures can add endpoints. TLS trust exceptions are
    /// never copied to newly advertised addresses.
    pub async fn synchronize(&self, selected: &Self, token: &str) -> Result<Self> {
        anyhow::ensure!(self.core_id.as_deref().is_none_or(|id|Some(id)==selected.core_id.as_deref())
            && self.core_public_key.as_deref().is_none_or(|key|Some(key)==selected.core_public_key.as_deref()),"Core 身份发生异常变化，已拒绝同步");
        if standalone_bridge() { return Ok(self.clone()); }
        if let Some(cached)=sync::recent(self)? { return Ok(cached); }
        Ok(self.fetch_connection_info(selected, token, None).await?.0)
    }
    async fn fetch_connection_info(
        &self, selected: &Self, token: &str, after: Option<u64>,
    ) -> Result<(Self, bool)> {
        let mut url = selected.endpoint("api/v1/connection-info")?;
        if let Some(revision) = after { url.query_pairs_mut().append_pair("after", &revision.to_string()); }
        let http = if after.is_some() {
            selected.http_with_read_timeout(Some(Duration::from_secs(55)))?
        } else { selected.http()? };
        let response = http.get(url)
            .bearer_auth(token)
            .timeout(Duration::from_secs(if after.is_some() { 255 } else { 5 }))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok((self.clone(), false));
        }
        let supports_watch = response.headers().get("x-rmusic-connection-watch").is_some_and(|v| v == "1");
        if supports_watch && response.status() == reqwest::StatusCode::NO_CONTENT {
            sync::acknowledge(self);
            return Ok((sync::apply(self)?, true));
        }
        let mut response = response.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(bytes.len() + chunk.len() <= 32768, "连接清单过大");
            bytes.extend(chunk);
        }
        if supports_watch && after.is_some() && bytes.iter().all(u8::is_ascii_whitespace) {
            sync::acknowledge(self);
            return Ok((sync::apply(self)?, true));
        }
        let manifest: rmusic_remote::identity::Signed<rmusic_remote::runtime::Manifest> =
            serde_json::from_slice(&bytes)?;
        Ok((sync::accept(self, selected, manifest)?, supports_watch))
    }
    pub fn route_fresh(&self, checked: std::time::Instant) -> bool {
        checked.elapsed() < Duration::from_secs(60) && !sync::changed_since(self, checked)
    }
    pub fn follow_connection_info(&self, selected: &Self, token: &str) -> ConnectionSync {
        sync::follow(self.clone(),selected.clone(),token.to_owned())
    }
    pub fn is_lan(&self) -> bool {
        if rmusic_remote::native::is_remote(&self.address) {
            return false;
        }
        self.base_url().ok().is_some_and(|url| match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
            Some(url::Host::Ipv6(ip)) => {
                ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
            }
            Some(url::Host::Domain(host)) => {
                host == "localhost" || host.ends_with(".local") || !host.contains('.')
            }
            None => false,
        })
    }
    pub async fn require_clock(&self, rate: u32) -> Result<()> {
        if rate <= 768000 {
            return Ok(());
        }
        let mut response = self
            .http()?
            .get(self.endpoint("api/v1/status")?)
            .timeout(Duration::from_secs(3))
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 16384 {
                bail!("invalid core status");
            }
            bytes.extend_from_slice(&chunk);
        }
        let status: serde_json::Value = serde_json::from_slice(&bytes)?;
        if self.core_id.as_deref() != status["core_id"].as_str() || self.core_id.is_none() {
            bail!("地址指向其他核心，已停止连接");
        }
        if status["capabilities"]["frame_clock_max_rate"]
            .as_u64()
            .unwrap_or(768000)
            < u64::from(rate)
        {
            bail!("当前 Core 不支持原生 DSD 字节帧计时，请更新到匹配的开发版 Core");
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        if self.alternatives.len() > 8 {
            bail!("最多保存 8 个备用地址");
        }
        if self
            .core_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 200 || id.chars().any(char::is_control))
        {
            bail!("invalid core identity");
        }
        for candidate in self.candidates() {
            candidate.base_url()?;
            candidate.tls_config()?;
        }
        Ok(())
    }
    fn candidates(&self) -> Vec<Self> {
        let mut primary = self.clone();
        primary.alternatives.clear();
        let mut candidates = vec![primary];
        candidates.extend(self.alternatives.iter().map(|address| Self {
            address: address.address.clone(),
            certificate_sha256: address.certificate_sha256.clone(),
            skip_certificate_verification: address.skip_certificate_verification,
            alternatives: vec![],
            ..self.clone()
        }));
        candidates
    }
    /// Public status only, at this exact address. Never sends account credentials,
    /// follows alternatives, or changes the profile's existing identity binding.
    pub async fn inspect_primary(&self) -> Result<serde_json::Value> {
        let budget = if self.bound_overlay() { 8 } else { 3 };
        self.inspect_with_timeout(Duration::from_secs(budget)).await
    }
    fn bound_overlay(&self) -> bool {
        if self.core_id.is_none() || self.core_public_key.is_none() || standalone_bridge() { return false; }
        self.base_url().ok().and_then(|base| match base.host()? {
            url::Host::Ipv4(ip) => Some(std::net::IpAddr::V4(ip)),
            url::Host::Ipv6(ip) => Some(std::net::IpAddr::V6(ip)),
            _ => None,
        }).is_some_and(rmusic_discovery::via_overlay)
    }
    async fn inspect_with_timeout(&self, timeout: Duration) -> Result<serde_json::Value> {
        self.validate()?;
        let mut response = self
            .http()?
            .get(self.endpoint("api/v1/status")?)
            .timeout(timeout)
            .send()
            .await?
            .error_for_status()?;
        if response.content_length().unwrap_or(0) > 16384 {
            bail!("invalid core status");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 16384 {
                bail!("invalid core status");
            }
            bytes.extend_from_slice(&chunk);
        }
        let status: serde_json::Value = serde_json::from_slice(&bytes)?;
        let id = status["core_id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control))
            .context("核心未返回身份，请先更新核心")?;
        let label = |key: &str, fallback: &str| {
            status[key]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= 200 && !v.chars().any(char::is_control))
                .unwrap_or(fallback)
                .to_owned()
        };
        Ok(
            serde_json::json!({"address":self.address,"relayed":status["relayed"],"core_id":id,"core_public_key":status["core_public_key"],"native_e2e":status["capabilities"]["native_e2e_v1"]==true,"name":label("name","Core"),"version":label("version",""),"initialized":status["initialized"].as_bool()}),
        )
    }
    /// Probe without credentials. A new profile must first establish identity at its primary address.
    pub async fn resolve(&self) -> Result<Self> {
        if standalone_bridge() { self.resolve_lan_only().await } else { sync::apply(self)?.probe_candidates(false,None).await }
    }

    /// Standalone bridges never import client manifests.
    pub async fn resolve_lan_only(&self) -> Result<Self> {
        self.probe_candidates(true,None).await
    }

    /// A slow idempotent read can race a fresh, identity-verified alternative.
    pub async fn interactive_read(
        &self, selected:&Self, path:&str, token:&str, request:reqwest::RequestBuilder,
    ) -> Result<(reqwest::Response,Option<Self>)> {
        let primary=request.send();
        tokio::pin!(primary);
        if standalone_bridge()||self.core_id.is_none()||self.core_id!=selected.core_id {
            return Ok((primary.await?,None));
        }
        let current=logical_address(&selected.address);
        let available=sync::apply(self)?;
        if !available.candidates().iter().any(|candidate|candidate.address!=current) {
            return Ok((primary.await?,None));
        }
        let mut hedge = true;
        if cfg!(target_os = "macos") && physical_lan_route(&current).await {
            hedge = false;
            for candidate in available.candidates() {
                if candidate.address != current && physical_lan_route(&candidate.address).await {
                    hedge = true;
                    break;
                }
            }
        }
        let alternative=async {
            let route=available.probe_candidates(false,Some(&current)).await?;
            let result=route.http()?.get(route.endpoint(path)?).bearer_auth(token)
                .timeout(Duration::from_secs(6)).send().await?.error_for_status()?;
            Ok::<_,anyhow::Error>((result,Some(route)))
        };
        let alternative = tokio::time::timeout(Duration::from_secs(7), alternative);
        tokio::pin!(alternative);
        tokio::select! {
            biased;
            result=&mut primary=>match result {
                Ok(response) if transient_read_status(response.status()) =>
                    return Ok(alternative.await.ok().and_then(Result::ok).unwrap_or((response,None))),
                Ok(response)=>return Ok((response,None)),
                Err(error) if error.is_connect()||error.is_timeout()=>
                    return alternative.await
                        .ok().and_then(Result::ok).ok_or_else(||error.into()),
                Err(error)=>return Err(error.into()),
            },
            _=tokio::time::sleep(Duration::from_millis(1100)), if hedge=>{}
        }
        tokio::select! {
            biased;
            result=&mut primary=>match result {
                Ok(response) if transient_read_status(response.status()) =>
                    Ok(alternative.await.ok().and_then(Result::ok).unwrap_or((response,None))),
                Ok(response)=>Ok((response,None)),
                Err(error)=>alternative.await
                    .ok().and_then(Result::ok).ok_or_else(||error.into()),
            },
            result=&mut alternative=>match result {
                Ok(Ok(response))=>Ok(response),
                _=>Ok((primary.await?,None)),
            },
        }
    }

    async fn probe_candidates(&self, lan_only: bool, excluded: Option<&str>) -> Result<Self> {
        let profile = self;
        profile.validate()?;
        let mut last_error = anyhow::anyhow!("核心不可达");
        let mut candidates = profile.candidates();
        if let Some(excluded)=excluded {candidates.retain(|candidate|candidate.address!=excluded);}
        if lan_only {
            for candidate in &mut candidates {
                let mut url = candidate.base_url()?;
                let ip = match url.host() {
                    Some(url::Host::Ipv4(ip)) => ip.into(),
                    Some(url::Host::Ipv6(ip)) => ip.into(),
                    Some(url::Host::Domain("localhost")) => std::net::Ipv4Addr::LOCALHOST.into(),
                    Some(url::Host::Domain(host)) if url.scheme()=="http" && (!host.contains('.') || host.ends_with(".local")) => {
                        let port=url.port_or_known_default().context("缺少局域网端口")?;
                        let mut ips=tokio::time::timeout(Duration::from_secs(3),tokio::net::lookup_host((host,port))).await??;
                        ips.find(|a|rmusic_discovery::bridge_on_link(a.ip())).context("主机名没有同网段地址")?.ip()
                    }
                    _ => bail!("独立网桥仅支持本机或物理局域网 IP；HTTP 也可使用本地名称"),
                };
                anyhow::ensure!(rmusic_discovery::bridge_on_link(ip),"独立网桥不支持公网或 VPN 入口");
                // Pin local-name resolution before status probing, authentication and WebSocket setup.
                url.set_ip_host(ip).map_err(|_|anyhow::anyhow!("无效局域网地址"))?;
                candidate.address=url.to_string().trim_end_matches('/').to_owned();
            }
        }
        if self.core_id.is_some() {
            let preferred = sync::preferred_addresses(self);
            candidates.sort_by_key(|candidate| {
                let rank = preferred.iter().position(|entry| entry.trim_end_matches('/') == candidate.address.trim_end_matches('/'));
                if let Some(rank) = rank {
                    rank + 1
                } else if candidate.base_url().ok().and_then(|url| match url.host()? {
                    url::Host::Ipv4(ip) => Some(ip.into()),
                    url::Host::Ipv6(ip) => Some(ip.into()),
                    url::Host::Domain("localhost") => Some(std::net::Ipv4Addr::LOCALHOST.into()),
                    _ => None,
                }).is_some_and(rmusic_discovery::bridge_on_link) {
                    0
                } else {
                    usize::MAX
                }
            });
        }
        let probe = |mut candidate: Self, timeout: Duration| async move {
                // A localhost facade still performs the remote WSS and pinned
                // Core TLS handshakes. Give that path its end-to-end budget.
                let timeout = if rmusic_remote::native::is_gateway(&candidate.address) {
                    timeout.max(Duration::from_secs(8))
                } else { timeout };
                let status = candidate.inspect_with_timeout(timeout).await?;
                anyhow::ensure!(status["relayed"] != true, "请使用 Core 的直连入口");
                let id = status["core_id"]
                    .as_str()
                    .context("核心未返回身份，请先更新核心")?;
                if self
                    .core_id
                    .as_deref()
                    .is_some_and(|expected| expected != id)
                {
                    bail!("地址指向其他核心，已停止连接");
                }
                let key = status["core_public_key"].as_str();
                if self
                    .core_public_key
                    .as_deref()
                    .is_some_and(|expected| key != Some(expected))
                {
                    bail!("Core 公钥发生异常变化，已停止连接");
                }
                if let Some(key) = key {
                    candidate.core_public_key = Some(key.into());
                }
                candidate.core_id = Some(id.into());
                if !rmusic_remote::native::is_gateway(&candidate.address) {
                    if status["native_e2e"] == true {
                        let base = candidate.base_url()?;
                        let local = match base.host() {
                            Some(url::Host::Ipv4(ip))=>rmusic_discovery::bridge_on_link(ip.into()),
                            Some(url::Host::Ipv6(ip))=>rmusic_discovery::bridge_on_link(ip.into()),
                            Some(url::Host::Domain(host))=>rmusic_remote::dns::resolve(host,base.port_or_known_default().unwrap_or(0)).await
                                .is_ok_and(|addresses|!addresses.is_empty()&&addresses.iter().all(|a|rmusic_discovery::bridge_on_link(a.ip()))),
                            None=>false,
                        };
                        if !local && base.scheme() == "http" && self.core_public_key.is_none() {
                            return Err(anyhow::Error::new(MissingTrustedKey));
                        }
                        let gateway = rmusic_remote::native::open_core(
                            &candidate.address,
                            id.into(),
                            candidate
                                .core_public_key
                                .clone()
                                .context("Core 缺少加密身份")?,
                            if base.scheme() == "https" {
                                Some(candidate.tls_config()?)
                            } else {
                                None
                            },
                            !local,
                        )
                        .await?;
                        candidate.address = gateway.address.clone();
                        candidate.skip_certificate_verification = false;
                        candidate.certificate_sha256 = None;
                        let confirmed = candidate.inspect_with_timeout(timeout).await?;
                        anyhow::ensure!(confirmed["core_id"].as_str() == Some(id) && confirmed["relayed"] != true,
                            "入口未通过 Core 加密通道验证");
                    } else if self.core_public_key.is_some() {
                        bail!("此入口未提供已绑定 Core 的加密通道，已停止发送登录凭据");
                    }
                }
                Ok::<_, anyhow::Error>(candidate)
        };
        let mut retry_overlay = None;
        if self.core_id.is_none() { candidates.truncate(1); }
        let mut local = Vec::new();
        let mut remaining = Vec::new();
        for candidate in candidates {
            if cfg!(target_os = "macos") && !lan_only && physical_lan_route(&candidate.address).await {
                local.push(candidate);
            } else {
                remaining.push(candidate);
            }
        }
        // Preserve the signed order within each group. Probe external routes only if LAN verification fails.
        for group in [local, remaining] {
            let mut probes = FuturesUnordered::new();
            let mut ready: Option<(usize, Self)> = None;
            let mut prefer_until = None;
            for (index, candidate) in group.into_iter().enumerate() {
                let run = probe(candidate.clone(), Duration::from_secs(3));
                probes.push(async move { (index, candidate, run.await) });
            }
            loop {
                let next = if let Some(deadline) = prefer_until {
                    match tokio::time::timeout_at(deadline, probes.next()).await {
                        Ok(next) => next,
                        Err(_) => break,
                    }
                } else { probes.next().await };
                let Some((index, candidate, result)) = next else { break; };
                match result {
                    Ok(selected) => {
                        if index == 0 { return Ok(selected); }
                        if ready.as_ref().is_none_or(|(rank,_)|index<*rank) { ready=Some((index,selected)); }
                        prefer_until.get_or_insert(tokio::time::Instant::now()+Duration::from_millis(150));
                    },
                    Err(error) => {
                        // All ordinary alternatives get their fast pass first. Allow one longer,
                        // credential-free probe of a bound VPN primary after a transient timeout.
                        if !lan_only && candidate.address == self.address &&
                            error.downcast_ref::<reqwest::Error>().is_some_and(|e| e.is_timeout()) && candidate.bound_overlay() {
                            retry_overlay = Some(candidate);
                        }
                        if !last_error.is::<MissingTrustedKey>() {last_error=error;}
                    },
                }
            }
            if let Some((_,selected)) = ready { return Ok(selected); }
        }
        if let Some(candidate) = retry_overlay {
            match probe(candidate, Duration::from_secs(8)).await {
                Ok(selected) => return Ok(selected),
                Err(e) => last_error = e,
            }
        }
        Err(last_error.context("无法连接核心"))
    }
    pub fn base_url(&self) -> Result<Url> {
        let url = Url::parse(&self.address).context("invalid core address")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("core address must contain only http(s), host, port and optional path")
        }
        if (self.certificate_sha256.is_some() || self.skip_certificate_verification)
            && url.scheme() != "https"
        {
            bail!("certificate trust exception requires HTTPS")
        }
        Ok(url)
    }
    pub fn require_encrypted_credentials(&self) -> Result<()> {
        anyhow::ensure!(self.base_url()?.scheme() == "https" || rmusic_remote::native::is_gateway(&self.address),
            "此 Core 没有提供加密通道，请更新 Core 或使用 HTTPS 登录。");
        Ok(())
    }
    /// Apply the policy to the route actually selected for this write, which can change after the settings page loads.
    pub fn admin_write_origin(&self, method: &str, path: &str, _body: Option<&serde_json::Value>) -> Result<Option<String>> {
        let path = path.split('?').next().unwrap_or(path).trim_start_matches('/');
        let path = path.strip_prefix("api/v1/").unwrap_or(path);
        let core_save = method == "PUT" && path == "remote-access";
        let account_write = method != "GET" && (path == "setup" || path.starts_with("auth/"));
        if !core_save && !account_write { return Ok(None); }

        let base = self.base_url()?;
        if base.scheme() == "https" || rmusic_remote::native::is_gateway(&self.address) { return Ok(None); }
        bail!("请先建立 Core 加密连接，再提交管理凭据。")
    }

    pub fn endpoint(&self, path: &str) -> Result<Url> {
        let base = self.base_url()?;
        Ok(Url::parse(&format!(
            "{}/{}",
            base.as_str().trim_end_matches('/'),
            path.trim_start_matches('/')
        ))?)
    }
    pub fn tls_config(&self) -> Result<Arc<rustls::ClientConfig>> {
        tls::config(
            self.certificate_sha256.as_deref(),
            self.skip_certificate_verification,
        )
    }

    pub fn http(&self) -> Result<reqwest::Client> {
        self.http_with_read_timeout(Some(Duration::from_secs(30)))
    }
    /// The bounded media consumer owns read deadlines so backpressure/pause cannot
    /// expire reqwest's body timer while it deliberately stops polling the network.
    pub fn streaming_http(&self) -> Result<reqwest::Client> {
        self.http_with_read_timeout(None)
    }
    pub fn http_with_read_timeout(&self, read_timeout: Option<Duration>) -> Result<reqwest::Client> {
        let base = self.base_url()?;
        // No auth headers/cookies are stored here. Reuse TLS/HTTP connections across
        // metadata, current audio and prepared songs without sharing account state.
        type Clients = std::collections::HashMap<String, (std::time::Instant, reqwest::Client)>;
        static CLIENTS: std::sync::OnceLock<std::sync::Mutex<Clients>> = std::sync::OnceLock::new();
        let clients = CLIENTS.get_or_init(Default::default);
        let key = format!(
            "{}:{:?}:{}:{read_timeout:?}",
            base, self.certificate_sha256, self.skip_certificate_verification
        );
        {
            let mut cache = clients.lock().unwrap();
            if let Some((used, client)) = cache.get_mut(&key).filter(|(used, _)| used.elapsed() < Duration::from_secs(60)) {
                *used = std::time::Instant::now();
                return Ok(client.clone());
            }
        }
        let origin = base.origin();
        let prefix = format!("{}/", base.path().trim_end_matches('/'));
        let mut client = reqwest::Client::builder()
            .dns_resolver(Arc::new(rmusic_remote::dns::Resolver))
            .connect_timeout(Duration::from_secs(15));
        // Private Core traffic must not inherit an HTTP(S)/ALL_PROXY setting.
        // The exact configured Core identity and TLS checks remain mandatory.
        if matches!(
            crate::network::address_kind(&self.address),
            crate::network::AddressKind::Loopback
                | crate::network::AddressKind::Private
                | crate::network::AddressKind::Overlay
        ) {
            client = client.no_proxy();
        }
        if let Some(timeout) = read_timeout {
            client = client.read_timeout(timeout);
        }
        let client = client
            .pool_idle_timeout(Duration::from_secs(20))
            .pool_max_idle_per_host(2)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 5
                    || attempt.url().origin() != origin
                    || !attempt.url().path().starts_with(&prefix)
                {
                    attempt.error("redirect outside selected core or too many redirects")
                } else {
                    attempt.follow()
                }
            }))
            .use_preconfigured_tls((*self.tls_config()?).clone())
            .build()?;
        let mut cache = clients.lock().unwrap();
        cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
        if cache.len() >= 32 {
            cache.clear();
        }
        cache.insert(key, (std::time::Instant::now(), client.clone()));
        Ok(client)
    }
    /// Media URLs may contain a ticket, but must stay on this core origin and configured prefix.
    pub fn validate_media_url(&self, input: &str) -> Result<Url> {
        let base = self.base_url()?;
        let url = Url::parse(input)?;
        let prefix = format!("{}/", base.path().trim_end_matches('/'));
        if url.origin() != base.origin()
            || !url.path().starts_with(&prefix)
            || !url.username().is_empty()
            || url.password().is_some()
        {
            bail!("media URL is outside the selected core")
        }
        Ok(url)
    }
    /// New core media references are relative to the selected API entry point.
    pub fn media_url(&self, input: &str) -> Result<Url> {
        if input.starts_with("api/v1/media/") {
            let url = self.endpoint(input)?;
            if !url
                .path()
                .starts_with(self.endpoint("api/v1/media/")?.path())
                || url.fragment().is_some()
            {
                bail!("invalid API-relative media URL");
            }
            self.validate_media_url(url.as_str())
        } else {
            self.validate_media_url(input)
        }
    }
}

fn transient_read_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 502 | 503 | 504)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_lan_origins_bypass_forward_proxies() {
        let mut p = profile();
        for address in [
            "http://192.168.50.10:7800",
            "http://192.168.1.3",
            "http://172.20.1.2",
            "http://127.0.0.1",
            "http://[fd00::1]",
            "https://nas.local",
            "http://nas",
        ] {
            p.address = address.into();
            assert!(p.is_lan(), "{address}");
        }
        for address in [
            "https://music.example",
            "https://8.8.8.8",
            "https://[240e:30b::808]",
            "http://evil.local.example",
        ] {
            p.address = address.into();
            assert!(!p.is_lan(), "{address}");
        }
    }
    #[tokio::test]
    async fn private_core_status_uses_a_direct_connection() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer).await.unwrap();
            let body = r#"{"core_id":"direct-core","initialized":true}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let mut p = profile();
        p.address = format!("http://{address}");
        assert_eq!(p.inspect_primary().await.unwrap()["core_id"], "direct-core");
        server.await.unwrap();
    }
    use super::tls::ExplicitCertificate as PinnedCertificate;
    #[tokio::test]
    async fn native_dsd_clock_admission_checks_core_identity_and_legacy_limits() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for body in [
                serde_json::json!({"core_id":"core-a"}),
                serde_json::json!({"core_id":"core-a","capabilities":{"frame_clock_max_rate":3072000}}),
                serde_json::json!({"core_id":"other","capabilities":{"frame_clock_max_rate":3072000}}),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                let request = std::str::from_utf8(&request[..n]).unwrap();
                assert!(request.starts_with("GET /music/api/v1/status "));
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let mut profile = profile();
        profile.address = format!("http://{address}/music");
        profile.core_id = Some("core-a".into());
        profile.require_clock(768000).await.unwrap();
        assert!(profile
            .require_clock(3072000)
            .await
            .unwrap_err()
            .to_string()
            .contains("更新"));
        profile.require_clock(3072000).await.unwrap();
        assert!(profile
            .require_clock(3072000)
            .await
            .unwrap_err()
            .to_string()
            .contains("其他核心"));
        server.await.unwrap();
    }
    #[test]
    fn sensitive_admin_writes_reject_an_unapproved_plaintext_route() {
        let mut profile = profile();
        profile.address = "http://192.168.50.10:7800".into();
        assert!(profile.admin_write_origin("PUT", "/remote-access", None).is_err());
        let consent = serde_json::json!({"allow_lan_http":true});
        profile.address = "http://203.0.113.10".into();
        assert!(profile.admin_write_origin("PUT", "/remote-access", Some(&consent)).is_err());
        assert!(profile.admin_write_origin("GET", "/remote-access", None).unwrap().is_none());
        profile.address = "https://core.example.org".into();
        assert!(profile.admin_write_origin("PUT", "/remote-access", None).unwrap().is_none());
    }

    fn profile() -> ConnectionProfile {
        ConnectionProfile {
            core_public_key: None,
            id: "a".into(),
            name: "NAS".into(),
            address: "https://core.example:7800/music/".into(),
            skip_certificate_verification: false,
            certificate_sha256: None,
            alternatives: vec![],
            core_id: None,
        }
    }
    #[test]
    fn keeps_reverse_proxy_prefix_and_rejects_other_origins() {
        let p = profile();
        assert_eq!(
            p.endpoint("/api/v1/status").unwrap().as_str(),
            "https://core.example:7800/music/api/v1/status"
        );
        assert!(p
            .validate_media_url("https://core.example:7800/music/api/v1/media/1?ticket=t")
            .is_ok());
        assert!(p
            .validate_media_url("https://evil.example/music/a")
            .is_err());
        assert!(p
            .validate_media_url("https://core.example:7800/music-other/a")
            .is_err());
    }
    #[test]
    fn api_relative_media_paths_follow_the_selected_address_and_prefix() {
        let mut p = profile();
        p.address = "https://backup.example/remote/music".into();
        assert_eq!(
            p.media_url("api/v1/media/qqmusic%3AAB12?ticket=t%2B1")
                .unwrap()
                .as_str(),
            "https://backup.example/remote/music/api/v1/media/qqmusic%3AAB12?ticket=t%2B1"
        );
        for input in [
            "//other.example/api/v1/media/id",
            "https://other.example/api/v1/media/id",
            "api/v1/media/../../auth/users?ticket=t",
            "api/v1/media/%2e%2e/status?ticket=t",
        ] {
            assert!(p.media_url(input).is_err(), "{input}");
        }
        assert!(p
            .media_url("https://backup.example/remote/music/api/v1/media/id?ticket=t")
            .is_ok());
    }
    #[test]
    fn certificate_exception_is_exact_pin_only() {
        assert!(PinnedCertificate::new("abcd").is_err());
        let der = CertificateDer::from(b"test certificate".as_slice());
        let digest = Sha256::digest(der.as_ref());
        let pin = digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let verifier = PinnedCertificate::new(&pin).unwrap();
        let name = ServerName::try_from("core.example").unwrap();
        assert!(verifier
            .verify_server_cert(
                &der,
                &[],
                &name,
                &[],
                UnixTime::since_unix_epoch(Duration::ZERO)
            )
            .is_ok());
        let other = CertificateDer::from(b"changed certificate".as_slice());
        assert!(verifier
            .verify_server_cert(
                &other,
                &[],
                &name,
                &[],
                UnixTime::since_unix_epoch(Duration::ZERO)
            )
            .is_err());
    }
    async fn server(
        id: &'static str,
        available: bool,
    ) -> (
        String,
        Arc<std::sync::atomic::AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        use axum::{
            http::{HeaderMap, StatusCode},
            routing::get,
            Json, Router,
        };
        let credentials = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = credentials.clone();
        let app = Router::new().route(
            "/music/api/v1/status",
            get(move |headers: HeaderMap| async move {
                if headers.contains_key("authorization") {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                (
                    if available {
                        StatusCode::OK
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    },
                    Json(serde_json::json!({"core_id":id})),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}/music", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (address, credentials, task)
    }
    #[tokio::test]
    async fn inspection_exposes_actual_identity_but_never_changes_binding_or_follows_fallbacks() {
        let (primary, credentials, a) = server("actual", true).await;
        let (fallback, _, b) = server("saved", true).await;
        let mut p = profile();
        p.address = primary.clone();
        p.core_id = Some("saved".into());
        p.alternatives = vec![CoreAddress {
            from_core: Some(false), address: fallback.clone(),
            certificate_sha256: None,
            skip_certificate_verification: false,
        }];
        let status = p.inspect_primary().await.unwrap();
        assert_eq!(status["core_id"], "actual");
        assert_eq!(status["address"], primary);
        assert_eq!(p.core_id.as_deref(), Some("saved"));
        assert_eq!(credentials.load(std::sync::atomic::Ordering::SeqCst), 0);
        p.alternatives.clear();
        assert!(p.resolve().await.is_err());
        // If primary is unavailable, inspection must not report a reachable backup as this address.
        a.abort();
        assert!(p.inspect_primary().await.is_err());
        p.alternatives = vec![CoreAddress {
            from_core: Some(false), address: fallback,
            certificate_sha256: None,
            skip_certificate_verification: false,
        }];
        assert!(p.inspect_primary().await.is_err());
        b.abort();
    }
    #[tokio::test]
    async fn inspection_bounds_status_body_and_sanitizes_optional_labels() {
        use axum::{extract::Path, routing::get, Json, Router};
        let app=Router::new().route("/{kind}/api/v1/status",get(|Path(kind):Path<String>|async move {
            Json(match kind.as_str() {
                "huge"=>serde_json::json!({"core_id":"actual","padding":"x".repeat(20000)}),
                "invalid"=>serde_json::json!({"core_id":"bad\nidentity"}),
                _=>serde_json::json!({"core_id":"actual","name":"unsafe\nname","version":"1.2","initialized":false,"private":"omit"}),
            })
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut p = profile();
        p.address = format!("http://{address}/ready");
        let status = p.inspect_primary().await.unwrap();
        assert_eq!(status["name"], "Core");
        assert_eq!(status["version"], "1.2");
        assert_eq!(status["initialized"], false);
        assert!(status.get("private").is_none());
        for path in ["huge", "invalid"] {
            p.address = format!("http://{address}/{path}");
            assert!(p.inspect_primary().await.is_err());
        }
        task.abort();
    }
    #[tokio::test]
    async fn fallback_preserves_prefix_and_probes_identity_without_credentials() {
        let (primary, first, a) = server("family", false).await;
        let (wrong, second, b) = server("stranger", true).await;
        let (fallback, third, c) = server("family", true).await;
        let mut p = profile();
        p.address = primary;
        p.core_id = Some("family".into());
        p.alternatives = vec![
            CoreAddress {
                from_core: Some(false), address: wrong,
                skip_certificate_verification: false,
                certificate_sha256: None,
            },
            CoreAddress {
                from_core: Some(false), address: fallback.clone(),
                skip_certificate_verification: false,
                certificate_sha256: None,
            },
        ];
        let selected = p.resolve().await.unwrap();
        assert_eq!(selected.address, fallback);
        assert_eq!(selected.core_id.as_deref(), Some("family"));
        assert_eq!(
            selected.endpoint("api/v1/status").unwrap().path(),
            "/music/api/v1/status"
        );
        for count in [first, second, third] {
            assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
        a.abort();
        b.abort();
        c.abort();
    }
    #[tokio::test]
    async fn the_last_fallback_is_reached_within_one_probe_timeout() {
        use axum::{extract::Path, http::HeaderMap, routing::get, Json, Router};
        let app = Router::new().route(
            "/{prefix}/api/v1/status",
            get(
                |Path(prefix): Path<String>, headers: HeaderMap| async move {
                    assert!(!headers.contains_key("authorization"));
                    if prefix != "ready" {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                    Json(serde_json::json!({"core_id":"family"}))
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut p = profile();
        p.address = format!("http://{address}/slow0");
        p.core_id = Some("family".into());
        p.alternatives = (1..8)
            .map(|index| CoreAddress {
                from_core: Some(false), address: format!("http://{address}/slow{index}"),
                skip_certificate_verification: false,
                certificate_sha256: None,
            })
            .chain(std::iter::once(CoreAddress {
                from_core: Some(false), address: format!("http://{address}/ready"),
                skip_certificate_verification: false,
                certificate_sha256: None,
            }))
            .collect();
        let selected = tokio::time::timeout(Duration::from_millis(700), p.resolve()).await;
        task.abort();
        assert_eq!(
            selected
                .expect("a ready alternative must not wait for unreachable preferred addresses")
                .unwrap()
                .address,
            format!("http://{address}/ready")
        );
    }
    #[tokio::test]
    async fn parallel_probes_preserve_primary_priority_and_unknown_identity_is_primary_only() {
        use axum::{extract::Path, http::HeaderMap, routing::get, Json, Router};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let seen = Arc::new(AtomicUsize::new(0));
        let count = seen.clone();
        let app = Router::new().route(
            "/{prefix}/api/v1/status",
            get(move |Path(prefix): Path<String>, headers: HeaderMap| {
                let count = count.clone();
                async move {
                    assert!(!headers.contains_key("authorization"));
                    count.fetch_add(1, Ordering::Relaxed);
                    if prefix == "primary" {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    Json(serde_json::json!({"core_id":"family"}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut p = profile();
        p.address = format!("http://{address}/primary");
        p.core_id = Some("family".into());
        p.alternatives = vec![CoreAddress {
            from_core: Some(false), address: format!("http://{address}/backup"),
            skip_certificate_verification: false,
            certificate_sha256: None,
        }];
        assert_eq!(p.resolve().await.unwrap().address, p.address);
        assert_eq!(seen.load(Ordering::Relaxed), 2);
        seen.store(0, Ordering::Relaxed);
        p.core_id = None;
        assert_eq!(p.resolve().await.unwrap().address, p.address);
        assert_eq!(seen.load(Ordering::Relaxed), 1);
        task.abort();
    }
    #[tokio::test]
    async fn unknown_profile_cannot_learn_identity_from_an_alternate() {
        let (primary, _, a) = server("family", false).await;
        let (fallback, _, b) = server("stranger", true).await;
        let mut p = profile();
        p.address = primary;
        p.alternatives = vec![CoreAddress {
            from_core: Some(false), address: fallback,
            skip_certificate_verification: false,
            certificate_sha256: None,
        }];
        assert!(p.resolve().await.is_err());
        a.abort();
        b.abort();
    }
    #[test]
    fn alternate_certificates_do_not_inherit_primary_exception() {
        let mut p = profile();
        p.certificate_sha256 = Some("ab".repeat(32));
        p.alternatives = vec![CoreAddress {
            from_core: Some(false), address: "https://backup.example/music".into(),
            skip_certificate_verification: false,
            certificate_sha256: None,
        }];
        assert!(p.validate().is_ok());
        assert!(p.candidates()[1].certificate_sha256.is_none());
        let old: ConnectionProfile = serde_json::from_value(
            serde_json::json!({"id":"a","name":"NAS","address":"http://localhost:7800"}),
        )
        .unwrap();
        assert!(old.alternatives.is_empty());
        assert!(old.core_id.is_none());
    }
    #[tokio::test]
    async fn actual_https_fallback_requires_its_own_certificate_pin() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = include_bytes!("../tests/fixtures/test-cert.der");
        let key = include_bytes!("../tests/fixtures/test-key.der");
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.to_vec())],
                rustls::pki_types::PrivatePkcs8KeyDer::from(key.to_vec()).into(),
            )
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("https://{}/music", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    if let Ok(mut tls) = acceptor.accept(socket).await {
                        let mut buffer = [0; 4096];
                        let len = tls.read(&mut buffer).await.unwrap();
                        let request = String::from_utf8_lossy(&buffer[..len]);
                        assert!(!request.to_lowercase().contains("authorization:"));
                        let response = if request.starts_with("GET /music/api/v1/status ") {
                            let body = r#"{"core_id":"family"}"#;
                            format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())
                        } else {
                            assert!(request.starts_with("GET /music/api/v1/media/id?ticket=t%2B1 "));
                            assert!(request.to_lowercase().contains("range: bytes=2-5"));
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 2-5/8\r\nConnection: close\r\n\r\n2345".to_owned()
                        };
                        tls.write_all(response.as_bytes()).await.unwrap();
                    }
                });
            }
        });
        let (primary, _, first) = server("family", false).await;
        let mut p = profile();
        p.address = primary;
        p.core_id = Some("family".into());
        p.alternatives = vec![CoreAddress {
            from_core: Some(false), address: address.clone(),
            skip_certificate_verification: false,
            certificate_sha256: None,
        }];
        assert!(p.resolve().await.is_err());
        p.alternatives[0].certificate_sha256 = Some(format!("{:x}", Sha256::digest(cert)));
        let selected = p.resolve().await.unwrap();
        assert_eq!(selected.address, address);
        let response = selected
            .http()
            .unwrap()
            .get(selected.media_url("api/v1/media/id?ticket=t%2B1").unwrap())
            .header("Range", "bytes=2-5")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.bytes().await.unwrap(), "2345");
        p.alternatives[0].certificate_sha256 = Some("00".repeat(32));
        assert!(p.resolve().await.is_err());
        task.abort();
        first.abort();
    }
}
