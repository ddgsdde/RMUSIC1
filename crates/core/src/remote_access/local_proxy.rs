//! Distinguish a co-hosted reverse proxy from other processes sharing its source IP.
use axum::Extension;
use std::{
    future::{ready, Ready},
    io,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::{net::TcpStream, sync::OnceCell};
use tower::Layer;

#[derive(Clone, Copy, Default)]
pub struct Capture(Option<(IpAddr, u32)>);
impl Capture {
    pub fn configured() -> anyhow::Result<Self> {
        let raw = std::env::var("RMUSIC_LOCAL_HTTPS_PROXY").unwrap_or_default();
        if raw.trim().is_empty() {
            return Ok(Self(None));
        }
        let (host, uid) = raw
            .split_once('@')
            .ok_or_else(|| anyhow::anyhow!("同机 HTTPS 反代格式为 本机IP@反代UID"))?;
        let (host, uid): (IpAddr, u32) = (host.parse()?, uid.parse()?);
        anyhow::ensure!(
            match host {
                IpAddr::V4(v) => v.is_private() || v.is_loopback(),
                IpAddr::V6(v) => v.is_unique_local() || v.is_loopback(),
            },
            "反代需要指定本机地址"
        );
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            anyhow::ensure!(
                uid > 0 && uid != 65534 && uid != std::fs::metadata("/proc/self")?.uid(),
                "请指定 HTTPS 反代的独立非 root UID"
            );
        }
        anyhow::ensure!(cfg!(target_os = "linux"), "本机反代进程校验需要 Linux");
        Ok(Self(Some((host, uid))))
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    VerifiedProxy,
    SeparateHost,
    Unverified,
}
#[derive(Clone)]
pub struct Peer {
    flow: Option<Arc<Flow>>,
    shared_address: bool,
}
struct Flow {
    local: SocketAddr,
    peer: SocketAddr,
    expected: u32,
    owner: OnceCell<Owner>,
}
impl Peer {
    pub async fn owner(&self) -> Owner {
        let Some(f) = &self.flow else {
            return if self.shared_address {
                Owner::Unverified
            } else {
                Owner::SeparateHost
            };
        };
        *f.owner
            .get_or_init(|| async {
                let (local, peer, uid) = (f.local, f.peer, f.expected);
                tokio::task::spawn_blocking(move || match socket_uid(peer, local) {
                    Some(value) if value == uid => Owner::VerifiedProxy,
                    _ => Owner::Unverified,
                })
                .await
                .unwrap_or(Owner::Unverified)
            })
            .await
    }
}
impl<S> axum_server::accept::Accept<TcpStream, S> for Capture {
    type Stream = TcpStream;
    type Service = <Extension<Peer> as Layer<S>>::Service;
    type Future = Ready<io::Result<(TcpStream, Self::Service)>>;
    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let result = (|| {
            let local = stream.local_addr()?;
            let peer = stream.peer_addr()?;
            let local = SocketAddr::new(local.ip().to_canonical(), local.port());
            let peer = SocketAddr::new(peer.ip().to_canonical(), peer.port());
            let shared_address = local.ip() == peer.ip() || peer.ip().is_loopback();
            let flow = self
                .0
                .filter(|(host, _)| *host == peer.ip())
                .map(|(_, expected)| {
                    Arc::new(Flow {
                        local,
                        peer,
                        expected,
                        owner: OnceCell::new(),
                    })
                });
            Ok((
                stream,
                Extension(Peer {
                    flow,
                    shared_address,
                })
                .layer(service),
            ))
        })();
        ready(result)
    }
}
fn proc_address(addr: SocketAddr) -> String {
    let ip = match addr.ip() {
        IpAddr::V4(v) => format!("{:08X}", u32::from_ne_bytes(v.octets())),
        IpAddr::V6(v) => v
            .octets()
            .chunks_exact(4)
            .map(|c| format!("{:08X}", u32::from_ne_bytes(c.try_into().unwrap())))
            .collect(),
    };
    format!("{ip}:{:04X}", addr.port())
}
fn lookup(reader: impl io::BufRead, source: &str, dest: &str) -> Option<u32> {
    for line in reader.lines() {
        let line = line.ok()?;
        let mut cols = line.split_ascii_whitespace().skip(1);
        if cols.next() == Some(source) && cols.next() == Some(dest) && cols.next() == Some("01") {
            return cols.nth(3)?.parse().ok();
        }
    }
    None
}
fn socket_uid(source: SocketAddr, dest: SocketAddr) -> Option<u32> {
    let check = |file, src, dst| {
        lookup(
            io::BufReader::new(std::fs::File::open(file).ok()?),
            &proc_address(src),
            &proc_address(dst),
        )
    };
    if let (IpAddr::V4(a), IpAddr::V4(b)) = (source.ip(), dest.ip()) {
        check("/proc/net/tcp", source, dest).or_else(|| {
            check(
                "/proc/net/tcp6",
                SocketAddr::new(a.to_ipv6_mapped().into(), source.port()),
                SocketAddr::new(b.to_ipv6_mapped().into(), dest.port()),
            )
        })
    } else {
        check("/proc/net/tcp6", source, dest)
    }
}
