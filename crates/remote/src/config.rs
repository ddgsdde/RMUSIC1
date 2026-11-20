use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IngressMode {
    #[default]
    #[serde(rename = "local_relay")]
    LocalOnly,
    ManagedHttps,
    ReverseProxy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Ports {
    pub https_listen: u16,
    pub https_public: u16,
    pub acme_http_listen: u16,
    /// 0 chooses a free loopback ALPN backend, or public 443 for standalone validation.
    pub acme_tls_listen: u16,
}
impl Default for Ports {
    fn default() -> Self {
        Self {
            https_listen: 8443,
            https_public: 443,
            acme_http_listen: 80,
            acme_tls_listen: 0,
        }
    }
}
impl Ports {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.https_listen > 0 && self.https_public > 0 && self.acme_http_listen > 0,
            "HTTPS 与 HTTP-01 端口必须大于零");
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Challenge {
    #[default]
    DnsCloudflare,
    Http01,
    TlsAlpn01,
    Imported,
    External,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Certificate {
    pub challenge: Challenge,
    pub email: String,
    pub directory: String,
    pub eab_kid: Option<String>,
    pub external_cert: Option<String>,
    pub external_key: Option<String>,
    pub dns_resolvers: Vec<String>,
    /// 0 keeps authoritative/recursive propagation checks; otherwise CA still verifies DNS.
    pub dns_wait_seconds: u16,
}
impl Default for Certificate {
    fn default() -> Self {
        Self {
            challenge: Challenge::default(),
            email: String::new(),
            directory: "https://acme-v02.api.letsencrypt.org/directory".into(),
            eab_kid: None,
            external_cert: None,
            external_key: None,
            dns_resolvers: vec![],
            dns_wait_seconds: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag="kind", rename_all="snake_case", deny_unknown_fields)]
pub enum Ipv6Lookup {
    Hostname { hostname: String },
    Https { urls: Vec<String> },
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Ddns {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv6_lookup: Option<Ipv6Lookup>,
    pub enabled: bool,
    pub zone_id: String,
    pub ipv4: Option<Ipv4Addr>,
    pub ipv6: Option<Ipv6Addr>,
    /// A shared IPv4 egress address is not proof of an inbound route.
    pub ipv4_inbound_confirmed: bool,
    pub auto_ipv6: bool,
    pub auto_ipv4: bool,
    pub adopt_existing: bool,
    pub interface: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Ingress {
    pub mode: IngressMode,
    pub domain: String,
    pub public_url: Option<String>,
    pub listen_address: String,
    pub ports: Ports,
    pub ddns: Ddns,
    pub certificate: Certificate,
    pub trusted_proxy_addresses: Vec<IpAddr>,
}
impl Default for Ingress {
    fn default() -> Self {
        Self {
            mode: IngressMode::LocalOnly,
            domain: String::new(),
            public_url: None,
            listen_address: "::".into(),
            ports: Ports::default(),
            ddns: Ddns::default(),
            certificate: Certificate::default(),
            trusted_proxy_addresses: vec![],
        }
    }
}
pub fn domain(value: &str) -> Result<String> {
    let parsed = url::Url::parse(&format!("https://{value}"))?;
    let host = parsed.host_str().unwrap_or("");
    ensure!(
        matches!(parsed.host(), Some(url::Host::Domain(_)))
            && !value.is_empty()
            && value.len() <= 253
            && parsed.port().is_none()
            && parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && host == value.trim_end_matches('.').to_ascii_lowercase(),
        "请填写不含端口和路径的域名"
    );
    ensure!(
        host.split('.').all(|label| !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')),
        "域名格式无效"
    );
    Ok(host.into())
}
pub fn https_url(value: &str) -> Result<url::Url> {
    ensure!(
        !value.contains('\\')
            && !value.split('/').any(|s| {
                let s = s
                    .to_ascii_lowercase()
                    .replace("%2e", ".")
                    .replace("%5c", "\\");
                s == ".." || s == "." || s.contains('\\')
            }),
        "地址不能包含路径跳转或反斜线"
    );
    let url = url::Url::parse(value)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && !url.path_segments().is_some_and(|p| p
                .into_iter()
                .any(|s| s == ".." || s.contains(['\\', '\r', '\n']))),
        "地址须为完整 HTTPS 地址，可包含端口和反代路径，不得含密码或查询参数"
    );
    ensure!(!value.chars().any(char::is_control), "地址包含控制字符");
    Ok(url)
}
impl Ingress {
    pub fn validate(&self) -> Result<()> {
        self.ports.validate()?;
        ensure!(self.certificate.dns_resolvers.len() <= 4 && self.certificate.dns_wait_seconds <= 180, "DNS 验证参数超出范围");
        for resolver in &self.certificate.dns_resolvers {
            let address: std::net::SocketAddr = resolver.parse().context("DNS 验证服务器须为 IP:端口，IPv6 使用方括号")?;
            ensure!(address.port() > 0, "DNS 验证服务器端口无效");
        }
        self.listen_address.parse::<IpAddr>()?;
        ensure!(
            self.trusted_proxy_addresses.len() <= 16,
            "可信反代地址最多 16 个"
        );
        match self.mode {
            IngressMode::LocalOnly => {},
            IngressMode::ReverseProxy => {
                https_url(self.public_url.as_deref().unwrap_or(""))?;
            }
            IngressMode::ManagedHttps => {
                domain(&self.domain)?;
                let ca = https_url(&self.certificate.directory)?;
                ensure!(ca.port_or_known_default().is_some(), "ACME 地址无效");
                match self.certificate.challenge {
                    Challenge::Http01 => ensure!(
                        self.ports.https_listen != self.ports.acme_http_listen,
                        "HTTP-01 本机验证端口与其他监听冲突"
                    ),
                    Challenge::TlsAlpn01 => {
                        let port = self.ports.acme_tls_listen;
                        ensure!(port == 0 || port != self.ports.https_listen, "TLS-ALPN 本机验证端口与其他监听冲突");
                    },
                    Challenge::Imported | Challenge::External => ensure!(
                        self.certificate
                            .external_cert
                            .as_ref()
                            .is_some_and(|s| !s.is_empty())
                            && self
                                .certificate
                                .external_key
                                .as_ref()
                                .is_some_and(|s| !s.is_empty()),
                        "请同时配置证书和私钥文件"
                    ),
                    _ => {}
                }
            }
        }
        if self.ddns.enabled && self.mode == IngressMode::ManagedHttps {
            ensure!(
                !self.ddns.zone_id.is_empty()
                    && self.ddns.zone_id.len() <= 64
                    && self.ddns.zone_id.bytes().all(|b| b.is_ascii_hexdigit()),
                "Cloudflare Zone ID 无效"
            );
            ensure!(
                !self.ddns.auto_ipv4 || self.ddns.ipv4_inbound_confirmed,
                "Core 发布 IPv4 前须确认入站条件"
            );
            if let Some(ip) = self.ddns.ipv4 {
                ensure!(
                    public_ip(ip.into()) && self.ddns.ipv4_inbound_confirmed,
                    "不能发布私网或未经确认的共享 IPv4 出口"
                );
            }
            if let Some(source) = &self.ddns.ipv6_lookup {
                ensure!(self.ddns.auto_ipv6 && self.ddns.ipv6.is_none(), "外部 IPv6 来源只用于 Core 自动更新 AAAA");
                match source {
                    Ipv6Lookup::Hostname { hostname } => {
                        ensure!(domain(hostname)? != domain(&self.domain)?, "来源域名不能是正在更新的域名");
                    }
                    Ipv6Lookup::Https { urls } => {
                        ensure!((1..=3).contains(&urls.len()), "请填写一到三个 HTTPS 查询地址");
                        for url in urls {
                            ensure!(url.len() <= 2048, "查询地址过长");
                            https_url(url)?;
                        }
                    }
                }
            }
            if let Some(ip) = self.ddns.ipv6 {
                ensure!(public_ip(ip.into()), "不能发布本地／特殊 IPv6 地址");
            }
        }
        Ok(())
    }
    pub fn managed(&self) -> bool {
        self.mode == IngressMode::ManagedHttps
    }
    pub fn advertised_url(&self) -> Option<String> {
        match self.mode {
            IngressMode::LocalOnly => None,
            IngressMode::ReverseProxy => self.public_url.clone(),
            IngressMode::ManagedHttps => Some(format!(
                "https://{}{}",
                self.domain,
                if self.ports.https_public == 443 {
                    String::new()
                } else {
                    format!(":{}", self.ports.https_public)
                }
            )),
        }
    }
}
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let a = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || a[0] == 0
                || a[0] >= 240
                || (a[0] == 100 && (64..128).contains(&a[1]))
                || (a[0] == 198 && (a[1] == 18 || a[1] == 19)))
        }
        IpAddr::V6(ip) => {
            let a = ip.segments();
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && !ip.is_multicast()
                && a[0] & 0xe000 == 0x2000
                && a[..2] != [0x2001, 0xdb8]
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Configuration {
    /// Preferred HTTPS entries, including reverse proxies with path prefixes.
    /// None is accepted from older full-configuration editors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_urls: Option<Vec<String>>,
    pub revision: u64,
    pub core: Ingress,
}
impl Default for Configuration {
    fn default() -> Self {
        let mut core = Ingress::default();
        core.ports.https_public = 8443;
        Self {
            revision: 0,
            direct_urls: None,
            core,
        }
    }
}
impl Configuration {
    pub fn validate(&self) -> Result<()> {
        self.core.validate()?;
        let urls=self.direct_urls.as_deref().unwrap_or_default();
        ensure!(urls.len() <= 5, "最多添加五个优先直连入口");
        let mut seen=std::collections::HashSet::new();
        for address in urls {
            ensure!(address.len()<=2048 && address.trim()==address && !address.chars().any(char::is_control), "直连地址格式无效");
            let url=https_url(address)?;
            ensure!(seen.insert(url.as_str().trim_end_matches('/').to_owned()), "直连地址重复");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_ports_and_external_prefix() {
        let mut v = Ingress::default();
        v.mode = IngressMode::ManagedHttps;
        v.domain = "core.example.com".into();
        v.ports.https_public = 4443;
        v.ports.https_listen = 9443;
        v.validate().unwrap();
        assert_eq!(v.advertised_url().unwrap(), "https://core.example.com:4443");
        v.mode = IngressMode::ReverseProxy;
        v.public_url = Some("https://music.example.com:7443/music".into());
        assert!(!v.managed());
        v.validate().unwrap();
        assert_eq!(
            v.advertised_url().unwrap(),
            "https://music.example.com:7443/music"
        );
    }
    #[test]
    fn custom_acme_listeners_validate_conflicts_without_changing_public_challenge_ports() {
        let mut ingress = Ingress::default();
        ingress.mode = IngressMode::ManagedHttps;
        ingress.domain = "music.example.com".into();
        ingress.certificate.challenge = Challenge::Http01;
        ingress.ports.acme_http_listen = 18080;
        ingress.validate().unwrap();
        ingress.ports.acme_http_listen = ingress.ports.https_listen;
        assert!(ingress.validate().is_err());
        ingress.certificate.challenge = Challenge::TlsAlpn01;
        ingress.ports.acme_tls_listen = ingress.ports.https_listen;
        assert!(ingress.validate().is_err());
        let old: Ports = serde_json::from_value(serde_json::json!({"https_listen":9443})).unwrap();
        assert_eq!(old.acme_http_listen,80); assert_eq!(old.acme_tls_listen,0);
    }
    #[test]
    fn never_publish_nat_or_local_ipv6() {
        for ip in [
            "100.64.1.2",
            "192.168.50.10",
            "127.0.0.1",
            "192.0.2.1",
            "fd7a:115c:a1e0::1",
            "fe80::1",
            "::1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("2606:4700:4700::1111".parse().unwrap()));
        let mut v = Ingress::default();
        v.mode = IngressMode::ManagedHttps;
        v.domain = "core.example.com".into();
        v.ddns.enabled = true;
        v.ddns.zone_id = "abcdef".into();
        v.ddns.ipv4 = Some("1.1.1.1".parse().unwrap());
        assert!(v.validate().is_err());
    }
    #[test]
    fn reject_ports_and_url_injection() {
        let mut p = Ports::default();
        p.https_listen = 0;
        assert!(p.validate().is_err());
        for v in [
            "example.com:443",
            "x/../../a",
            "x\ncommand",
            "user@example.com",
            "example.com?x",
        ] {
            assert!(domain(v).is_err());
        }
        assert!(https_url("https://user:pass@example.com").is_err());
        assert!(https_url("https://[2606:4700::1]:9443/music").is_ok());
    }
}
