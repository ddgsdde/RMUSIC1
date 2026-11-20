//! Only explicitly selected records may be adopted. Never enumerates/deletes a zone.
use crate::config::{public_ip, Ingress};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

pub struct Cloudflare {
    http: reqwest::Client,
    base: String,
    zone: String,
    token: zeroize::Zeroizing<String>,
    cancel: tokio_util::sync::CancellationToken,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Record {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub proxied: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Update {
    pub name: String,
    pub kind: String,
    pub record_id: String,
    pub address: IpAddr,
    pub changed: bool,
}
#[derive(Deserialize)]
struct Response<T> {
    success: bool,
    result: Option<T>,
}
impl Cloudflare {
    pub fn new(token: String, zone: String) -> Result<Self> {
        ensure!(
            !token.is_empty() && token.len() <= 4096 && !token.chars().any(char::is_control),
            "Cloudflare Token 无效"
        );
        ensure!(
            !zone.is_empty() && zone.bytes().all(|b| b.is_ascii_hexdigit()),
            "Zone ID 无效"
        );
        Ok(Self {
            http: reqwest::Client::builder().dns_resolver(std::sync::Arc::new(crate::dns::Resolver))
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(20))
                .build()?,
            base: "https://api.cloudflare.com/client/v4".into(),
            zone,
            token: zeroize::Zeroizing::new(token),
            cancel: Default::default(),
        })
    }
    async fn response<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T> {
        // Do not abandon an already accepted provider write. Let it settle, but
        // never begin another request for a superseded configuration generation.
        ensure!(!self.cancel.is_cancelled(), "DNS 配置已更新，旧任务停止");
        let mut response = request.bearer_auth(self.token.as_str()).send().await?;
        let status = response.status();
        ensure!(
            status.is_success(),
            "Cloudflare 请求失败（HTTP {}）；请检查 Token 权限、配额和网络",
            status.as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(bytes.len() + chunk.len() <= 1024 * 1024, "DNS 响应过大");
            bytes.extend(chunk);
        }
        let data: Response<T> = serde_json::from_slice(&bytes)?;
        ensure!(data.success, "Cloudflare 拒绝了 DNS 操作");
        data.result.context("DNS 响应缺少结果")
    }
    pub async fn set(&self, name: &str, ip: IpAddr, owner: &str, adopt: bool) -> Result<Update> {
        crate::config::domain(name)?;
        ensure!(public_ip(ip), "不能向 DDNS 发布私网地址");
        ensure!(
            owner.len() <= 100
                && owner
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "记录归属无效"
        );
        let kind = if ip.is_ipv6() { "AAAA" } else { "A" };
        let url = format!("{}/zones/{}/dns_records", self.base, self.zone);
        let comment = format!("R-Music:{owner}");
        let records: Vec<Record> = self
            .response(self.http.get(&url).query(&[
                ("name", name),
                ("type", kind),
                ("per_page", "2"),
            ]))
            .await?;
        ensure!(
            records.len() <= 1,
            "同名记录不唯一，请先明确要管理的 DNS 记录"
        );
        let payload = serde_json::json!({"type":kind,"name":name,"content":ip.to_string(),"ttl":1,"proxied":false,"comment":comment});
        let (record, changed) = if let Some(record) = records.into_iter().next() {
            ensure!(
                record.name.eq_ignore_ascii_case(name) && record.kind == kind,
                "DNS 响应记录不匹配"
            );
            ensure!(
                record.comment.as_deref() == Some(&comment) || adopt,
                "该 DNS 记录不属于 R-Music，请明确允许接管后重试"
            );
            if record.content == ip.to_string()
                && !record.proxied
                && record.comment.as_deref() == Some(&comment)
            {
                (record, false)
            } else {
                let updated: Record = self
                    .response(
                        self.http
                            .patch(format!("{url}/{}", record.id))
                            .json(&payload),
                    )
                    .await?;
                (updated, true)
            }
        } else {
            let created: Record = self.response(self.http.post(&url).json(&payload)).await?;
            (created, true)
        };
        Ok(Update {
            name: name.into(),
            kind: kind.into(),
            record_id: record.id,
            address: ip,
            changed,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Address {
    pub interface: String,
    pub address: IpAddr,
}
pub fn local_public_addresses() -> Vec<Address> {
    let mut result = Vec::new();
    #[cfg(unix)]
    unsafe {
        let mut head = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return result;
        }
        let mut next = head;
        while !next.is_null() {
            let row = &*next;
            next = row.ifa_next;
            if row.ifa_addr.is_null()
                || row.ifa_name.is_null()
                || row.ifa_flags & libc::IFF_UP as u32 == 0
            {
                continue;
            }
            let name = std::ffi::CStr::from_ptr(row.ifa_name)
                .to_string_lossy()
                .into_owned();
            if name.starts_with("utun")
                || name.starts_with("tun")
                || name.starts_with("wg")
                || name.starts_with("tailscale")
                || name.starts_with("veth")
                || name.starts_with("docker")
                || name.starts_with("lo")
            {
                continue;
            }
            let address = match (*row.ifa_addr).sa_family as i32 {
                libc::AF_INET => {
                    let v = &*(row.ifa_addr as *const libc::sockaddr_in);
                    Some(IpAddr::V4(Ipv4Addr::from(v.sin_addr.s_addr.to_ne_bytes())))
                }
                libc::AF_INET6 => {
                    let v = &*(row.ifa_addr as *const libc::sockaddr_in6);
                    Some(IpAddr::V6(Ipv6Addr::from(v.sin6_addr.s6_addr)))
                }
                _ => None,
            };
            if let Some(address) = address.filter(|ip| public_ip(*ip)) {
                result.push(Address {
                    interface: name,
                    address,
                });
            }
        }
        libc::freeifaddrs(head);
    }
    result.sort_by(|a, b| (&a.interface, a.address).cmp(&(&b.interface, b.address)));
    result.dedup_by(|a, b| a.address == b.address && a.interface == b.interface);
    result
}
// Looking up a UDP route chooses a source address without sending application data.
pub(crate) fn routed_ipv6() -> Option<Ipv6Addr> {
    let socket = std::net::UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111), 53)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V6(ip) => Some(ip),
        _ => None,
    }
}

fn select_ipv6(candidates: &[&Address], routed: Option<Ipv6Addr>) -> Result<Option<IpAddr>> {
    let ips: Vec<_> = candidates.iter().filter_map(|a| match a.address {
        IpAddr::V6(v) => Some(v),
        _ => None,
    }).collect();
    if let Some(ip) = routed.filter(|ip| ips.contains(ip)) {
        return Ok(Some(ip.into()));
    }
    match ips.as_slice() {
        [] => Ok(None),
        [ip] => Ok(Some((*ip).into())),
        _ => anyhow::bail!("选定网卡存在多个公网 IPv6，系统出站源地址不在其中。请核对网卡或指定地址。"),
    }
}

fn observed_ipv6(text: &[u8]) -> Result<Ipv6Addr> {
    let raw=std::str::from_utf8(text)?.trim();
    let ip=if let Ok(ip)=raw.parse::<Ipv6Addr>() { ip } else {
        let value:serde_json::Value=serde_json::from_str(raw).context("响应须为 IPv6 文本或包含 ip 的 JSON 对象")?;
        value["ip"].as_str().context("响应没有 ip 字段")?.parse()?
    };
    ensure!(public_ip(ip.into()) && ip.segments()[0] & 0xe000 == 0x2000,"查询结果不是公网 IPv6");
    Ok(ip)
}

async fn https_ipv6(endpoint:&str, bind:Option<Ipv6Addr>) -> Result<Ipv6Addr> {
    let url=crate::config::https_url(endpoint)?;
    let host=url.host_str().context("缺少查询主机")?.trim_matches(['[',']']);
    let port=url.port_or_known_default().context("缺少查询端口")?;
    // Pin the validated AAAA set for this request. Never follow redirects or send provider credentials.
    let addresses:Vec<_>=crate::dns::resolve(host,port).await?.into_iter()
        .filter(|a|matches!(a.ip(),IpAddr::V6(ip) if ip.segments()[0]&0xe000==0x2000 && public_ip(ip.into()))).collect();
    ensure!(!addresses.is_empty(),"查询服务没有公网 IPv6 地址");
    let client=reqwest::Client::builder().https_only(true).no_proxy()
        .resolve_to_addrs(host,&addresses).local_address(IpAddr::V6(bind.unwrap_or(Ipv6Addr::UNSPECIFIED)))
        .redirect(reqwest::redirect::Policy::none()).timeout(Duration::from_secs(8)).build()?;
    let mut response=client.get(url).send().await?;
    ensure!(response.status().is_success(),"IPv6 查询服务返回 HTTP {}",response.status());
    let mut bytes=Vec::new();
    while let Some(chunk)=response.chunk().await? {
        ensure!(bytes.len()+chunk.len()<=4096,"IPv6 查询响应过大");
        bytes.extend_from_slice(&chunk);
    }
    observed_ipv6(&bytes)
}

async fn external_ipv6(source:&crate::config::Ipv6Lookup, selected:&[&Address], pin_interface:bool) -> Result<Ipv6Addr> {
    let routed=routed_ipv6();
    let ip=match source {
        crate::config::Ipv6Lookup::Hostname{hostname}=>{
            let mut ips:Vec<_>=crate::dns::resolve(hostname,443).await?.into_iter().filter_map(|a|match a.ip(){
                IpAddr::V6(ip) if ip.segments()[0]&0xe000==0x2000 && public_ip(ip.into())=>Some(ip), _=>None,
            }).collect();
            ips.sort_unstable();ips.dedup();
            if pin_interface {ips.retain(|ip|selected.iter().any(|a|a.address==IpAddr::V6(*ip)));}
            if let Some(current)=routed.filter(|ip|ips.contains(ip)) {current}
            else {ensure!(ips.len()==1,"来源域名没有唯一可用的公网 AAAA");ips[0]}
        }
        crate::config::Ipv6Lookup::Https{urls}=>{
            let bind=if pin_interface {
                match select_ipv6(selected,routed)? {Some(IpAddr::V6(v))=>Some(v),_=>anyhow::bail!("指定网卡没有可用的 IPv6")}
            } else { None };
            let mut agreed=None;
            for url in urls {
                let answer=https_ipv6(url,bind).await?;
                ensure!(agreed.is_none_or(|ip|ip==answer),"多个查询服务的 IPv6 结果不一致，保留原记录");
                agreed=Some(answer);
            }
            agreed.context("没有 IPv6 查询服务")?
        }
    };
    Ok(ip)
}

pub async fn addresses(config: &Ingress) -> Result<Vec<IpAddr>> {
    let mut output = Vec::new();
    let local = local_public_addresses();
    let selected: Vec<_> = local
        .iter()
        .filter(|a| {
            config
                .ddns
                .interface
                .as_ref()
                .is_none_or(|i| i == &a.interface)
        })
        .collect();
    if let Some(ip) = config.ddns.ipv6 {
        output.push(ip.into());
    } else if config.ddns.auto_ipv6 {
        if let Some(source)=&config.ddns.ipv6_lookup {
            let ip=tokio::time::timeout(Duration::from_secs(30),external_ipv6(source,&selected,config.ddns.interface.is_some())).await.context("外部 IPv6 查询超时")??;
            output.push(ip.into());
        } else if let Some(ip) = select_ipv6(&selected, routed_ipv6())? {
            output.push(ip);
        }
    }
    if let Some(ip) = config.ddns.ipv4 {
        output.push(ip.into());
    } else if config.ddns.auto_ipv4 {
        ensure!(
            config.ddns.ipv4_inbound_confirmed,
            "Core 未确认 IPv4 入站条件"
        );
        if let Some(a) = selected.iter().find(|a| a.address.is_ipv4()) {
            output.push(a.address);
        } else {
            let client = reqwest::Client::builder().dns_resolver(std::sync::Arc::new(crate::dns::Resolver))
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(8))
                .build()?;
            let mut found = None;
            for url in ["https://api4.ipify.org", "https://ipv4.icanhazip.com"] {
                let mut response = client.get(url).send().await?.error_for_status()?;
                let mut bytes=Vec::new();
                while let Some(chunk)=response.chunk().await? {
                    ensure!(bytes.len()+chunk.len()<100,"IPv4 探测结果无效"); bytes.extend(chunk);
                }
                let ip = std::str::from_utf8(&bytes)?.trim().parse::<Ipv4Addr>()?;
                ensure!(public_ip(ip.into()), "出口地址不是公网地址");
                if let Some(old) = found {
                    ensure!(old == ip, "两次 IPv4 出口探测不一致，未修改 DNS");
                }
                found = Some(ip);
            }
            if let Some(ip) = found {
                output.push(ip.into());
            }
        }
    }
    ensure!(output.iter().all(|ip| public_ip(*ip)), "DDNS 地址无效");
    Ok(output)
}
pub async fn update_cancellable(
    config: &Ingress, token: String, owner: &str,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<Vec<Update>> {
    if !config.managed() || !config.ddns.enabled {
        return Ok(vec![]);
    }
    config.validate()?;
    let addresses = tokio::select! { result=addresses(config)=>result?,_=cancel.cancelled()=>anyhow::bail!("DNS 配置已更新，旧任务停止") };
    ensure!(
        !addresses.is_empty(),
        "未找到可发布的公网地址；保留现有 DNS"
    );
    let mut cf = Cloudflare::new(token, config.ddns.zone_id.clone())?;
    cf.cancel=cancel;
    let mut results = Vec::new();
    for ip in addresses {
        results.push(cf.set(&config.domain, ip, owner, config.ddns.adopt_existing).await?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "explicit network QA with test-selected sources; does not update any DNS records"]
    async fn read_configured_external_ipv6_without_publishing() {
        for source in [
            crate::config::Ipv6Lookup::Hostname{hostname:std::env::var("RMUSIC_QA_AAAA_HOST").unwrap()},
            crate::config::Ipv6Lookup::Https{urls:vec![std::env::var("RMUSIC_QA_IPV6_URL").unwrap()]},
        ] {
            let mut ingress=Ingress::default();
            ingress.ddns.auto_ipv6=true;
            ingress.ddns.ipv6_lookup=Some(source);
            let ips=addresses(&ingress).await.unwrap();
            assert_eq!(ips.len(),1);assert!(ips[0].is_ipv6() && public_ip(ips[0]));
            println!("只读查询返回 {}",ips[0]);
        }
    }
    #[test]
    fn external_lookup_accepts_only_global_v6_and_validates_source() {
        let ip:Ipv6Addr="2606:4700:4700::1111".parse().unwrap();
        for response in [b"2606:4700:4700::1111\n".as_slice(),br#"{"ip":"2606:4700:4700::1111"}"#] {
            assert_eq!(observed_ipv6(response).unwrap(),ip);
        }
        for response in ["localhost","127.0.0.1","::1","fd00::1","::ffff:8.8.8.8","2001:db8::1"] {assert!(observed_ipv6(response.as_bytes()).is_err());}
        let mut ingress=Ingress::default();
        ingress.mode=crate::config::IngressMode::ManagedHttps;
        ingress.domain="core.example.org".into();
        ingress.certificate.email="admin@example.org".into();
        ingress.ddns.enabled=true; ingress.ddns.auto_ipv6=true; ingress.ddns.zone_id="abcd".into();
        ingress.ddns.ipv6_lookup=Some(crate::config::Ipv6Lookup::Hostname{hostname:"core.example.org".into()});
        assert!(ingress.validate().is_err());
        ingress.ddns.ipv6_lookup=Some(crate::config::Ipv6Lookup::Hostname{hostname:"nas.example.org".into()});
        ingress.validate().unwrap();
    }
    #[test]
    fn renumbered_ipv6_prefers_kernel_source_and_rejects_ambiguous_fallback() {
        let old=Address{interface:"ovs_eth0".into(),address:"240e:30b:3c03:8d01::808".parse().unwrap()};
        let new=Address{interface:"ovs_eth0".into(),address:"240e:30b:3c40:2601::808".parse().unwrap()};
        let route=Some("240e:30b:3c40:2601::808".parse().unwrap());
        assert_eq!(select_ipv6(&[&old,&new],route).unwrap(),Some(new.address));
        assert!(select_ipv6(&[&old,&new],None).is_err());
        assert_eq!(select_ipv6(&[&new],None).unwrap(),Some(new.address));
    }

    #[tokio::test]
    async fn reverse_proxy_never_calls_dns_or_needs_token() {
        let mut ingress = Ingress::default();
        ingress.mode = crate::config::IngressMode::ReverseProxy;
        ingress.public_url = Some("https://example.com/music".into());
        ingress.ddns.enabled = true;
        assert!(update_cancellable(&ingress, String::new(), "core", Default::default())
            .await
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn core_never_uses_unconfirmed_nat_egress() {
        let mut ingress = Ingress::default();
        ingress.ddns.auto_ipv4 = true;
        assert!(addresses(&ingress).await.is_err());
    }
    #[tokio::test]
    async fn cancelled_generation_does_not_start_a_write_after_its_dns_read() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        let cancel=tokio_util::sync::CancellationToken::new();let stop=cancel.clone();
        let writes=Arc::new(AtomicUsize::new(0));let count=writes.clone();
        let router=axum::Router::new().fallback(move |request:axum::extract::Request| {
            let stop=stop.clone();let count=count.clone();async move {
                if request.method()==axum::http::Method::GET {
                    stop.cancel();
                    axum::Json(serde_json::json!({"success":true,"result":[{"id":"record","name":"music.example.com","type":"A","content":"1.1.1.1","comment":"R-Music:core","proxied":false}]}))
                } else {count.fetch_add(1,Ordering::SeqCst);axum::Json(serde_json::json!({"success":false}))}
            }
        });
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let server=tokio::spawn(async move {axum::serve(listener,router).await.unwrap();});
        let cf=Cloudflare{http:reqwest::Client::builder().no_proxy().build().unwrap(),base:format!("http://{address}"),zone:"abc".into(),token:zeroize::Zeroizing::new("test-only-token".into()),cancel};
        assert!(cf.set("music.example.com","8.8.8.8".parse().unwrap(),"core",false).await.is_err());
        assert_eq!(writes.load(Ordering::SeqCst),0);server.abort();
    }
}

pub static PUBLICATION_LOCK:tokio::sync::Mutex<()>=tokio::sync::Mutex::const_new(());
