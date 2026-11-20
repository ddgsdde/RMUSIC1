//! Shared LAN discovery for cores and host bridges. Records never carry credentials.
use anyhow::{bail, Result};
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, net::{IpAddr, Ipv4Addr}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::Duration};

pub const CORE_SERVICE: &str = "_rmusic-core._tcp.local.";
pub const BRIDGE_SERVICE: &str = "_rmusic-bridge._tcp.local.";

#[derive(Clone, Debug, Serialize)]
pub struct DiscoveredService {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub addresses: Vec<String>,
    pub core_id: Option<String>,
}
pub struct Discovery {
    daemon: ServiceDaemon,
    entries: Arc<Mutex<HashMap<String, DiscoveredService>>>,
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_control)
}
fn valid_path(path: &str) -> bool {
    (path.is_empty() || path.starts_with('/')) && !path.starts_with("//") && path.len() <= 512
        && !path.contains(['?', '#', '\\']) && !path.contains("..") && !path.chars().any(char::is_control)
        && !["%2e", "%2f", "%5c"].iter().any(|part|path.to_lowercase().contains(part))
}
fn resolved(info: &ResolvedService) -> Option<DiscoveredService> {
    let kind=match info.ty_domain.as_str() {CORE_SERVICE=>"core",BRIDGE_SERVICE=>"bridge",_=>return None};
    if info.get_property_val_str("v") != Some("1") || info.port == 0 {return None;}
    let id=info.get_property_val_str("id")?;
    let path=info.get_property_val_str("path").unwrap_or("");
    if !valid_id(id)||!valid_path(path) {return None;}
    let scheme=if info.get_property_val_str("tls")==Some("1"){"https"}else{"http"};
    let mut addresses:Vec<_>=info.get_addresses_v4().into_iter().filter(|ip|!ip.is_unspecified()&&!ip.is_multicast()&&!ip.is_loopback())
        .map(|ip|format!("{scheme}://{ip}:{}{path}",info.port)).collect();
    addresses.extend(info.get_addresses().iter().filter_map(|scoped|match scoped.to_ip_addr(){IpAddr::V6(ip) if !ip.is_unspecified()&&!ip.is_loopback()&&!ip.is_multicast()&&!ip.is_unicast_link_local()=>Some(format!("{scheme}://[{ip}]:{}{path}",info.port)),_=>None}));
    addresses.sort();addresses.dedup();
    if addresses.is_empty() {return None;}
    Some(DiscoveredService {id:id.to_owned(),name:info.get_property_val_str("name").filter(|name|valid_id(name)).unwrap_or("R-Music").to_owned(),
        kind:kind.into(),addresses,core_id:info.get_property_val_str("core_id").filter(|id|valid_id(id)).map(str::to_owned)})
}
impl Discovery {
    pub fn new() -> Result<Self> {
        let daemon=ServiceDaemon::new()?;
        let cores=daemon.browse(CORE_SERVICE)?;let bridges=daemon.browse(BRIDGE_SERVICE)?;
        let entries=Arc::new(Mutex::new(HashMap::new()));let records=entries.clone();
        let stopping=Arc::new(AtomicBool::new(false));let stop=stopping.clone();
        let worker=thread::Builder::new().name("rmusic-discovery".into()).spawn(move||{
            while !stop.load(Ordering::Relaxed) {
                for receiver in [&cores,&bridges] {
                    for event in receiver.try_iter().take(64) {
                        let mut entries=records.lock().unwrap();
                        match event {
                            ServiceEvent::ServiceResolved(info)=>if let Some(value)=resolved(&info) {
                                if entries.len()<256||entries.contains_key(&info.fullname) {entries.insert(info.fullname.clone(),value);}
                            },
                            ServiceEvent::ServiceRemoved(_,name)=>{entries.remove(&name);},
                            _=>{},
                        }
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
        })?;
        Ok(Self {daemon,entries,stopping,worker:Some(worker)})
    }
    pub fn snapshot(&self) -> Vec<DiscoveredService> {
        let mut grouped:HashMap<(String,String),DiscoveredService>=HashMap::new();
        for item in self.entries.lock().unwrap().values() {
            let key=(item.kind.clone(),item.id.clone());
            let value=grouped.entry(key).or_insert_with(||item.clone());
            value.addresses.extend(item.addresses.clone());value.addresses.sort();value.addresses.dedup();
        }
        let mut values:Vec<_>=grouped.into_values().collect();values.sort_by(|a,b|(&a.kind,&a.name,&a.id).cmp(&(&b.kind,&b.name,&b.id)));values
    }
    pub fn advertise(&self, kind: &str, id: &str, name: &str, port: u16, path: &str, tls: bool, core_id: Option<&str>, addresses: &[Ipv4Addr]) -> Result<String> {
        if !valid_id(id)||!valid_id(name)||!valid_path(path)||port==0 {bail!("invalid discovery record");}
        let service=match kind {"core"=>CORE_SERVICE,"bridge"=>BRIDGE_SERVICE,_=>bail!("invalid service kind")};
        let host=std::env::var("COMPUTERNAME").or_else(|_|std::env::var("HOSTNAME")).unwrap_or_default();
        let digest=format!("{:x}",Sha256::digest(format!("{kind}/{id}/{host}/{port}").as_bytes()));
        let instance=format!("rmusic-{}",&digest[..24]);
        let hostname=format!("{instance}.local.");
        let properties=[("v","1"),("id",id),("name",name),("path",path),("tls",if tls{"1"}else{"0"}),("core_id",core_id.unwrap_or(""))];
        let ips:Vec<IpAddr>=addresses.iter().copied().map(IpAddr::V4).collect();
        let info=ServiceInfo::new(service,&instance,&hostname,ips.as_slice(),port,&properties[..])?;
        let info=if addresses.is_empty(){info.enable_addr_auto()}else{info};
        let fullname=info.get_fullname().to_owned();self.daemon.register(info)?;Ok(fullname)
    }
    pub fn withdraw(&self, name: &str) {let _=self.daemon.unregister(name);}
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.stopping.store(true,Ordering::Relaxed);let _=self.daemon.shutdown();
        if let Some(worker)=self.worker.take(){let _=worker.join();}
    }
}
pub fn local_ipv4() -> Vec<Ipv4Addr> {
    let mut addresses:Vec<_>=if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter_map(|interface|match interface.ip() {
        IpAddr::V4(ip) if ip.is_private()&&!ip.is_loopback()=>Some(ip),_=>None,
    }).collect();addresses.sort();addresses.dedup();addresses
}
/// Physical on-link interfaces only; overlay membership is not LAN enrollment.
pub fn on_link(ip:IpAddr)->bool{
    if ip.is_loopback(){return true;}
    if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter(|i|!["utun","tun","tailscale","wg","zt"].iter().any(|prefix|i.name.starts_with(prefix))).any(|i|match(i.addr,ip){
        (if_addrs::IfAddr::V4(a),IpAddr::V4(ip))=>u32::from(a.netmask)!=0&&u32::from(a.ip)&u32::from(a.netmask)==u32::from(ip)&u32::from(a.netmask),
        (if_addrs::IfAddr::V6(a),IpAddr::V6(ip))=>u128::from(a.netmask)!=0&&u128::from(a.ip)&u128::from(a.netmask)==u128::from(ip)&u128::from(a.netmask),_=>false,
    })
}
/// Inspect the route actually chosen for a private subnet behind a VPN.
pub fn via_overlay(ip:IpAddr)->bool {
    let any=if ip.is_ipv4() {"0.0.0.0:0"} else {"[::]:0"};
    let Ok(local)=std::net::UdpSocket::bind(any).and_then(|s|{s.connect(std::net::SocketAddr::new(ip,9))?;s.local_addr()}) else{return false};
    if_addrs::get_if_addrs().unwrap_or_default().iter().any(|i|i.ip()==local.ip() && (i.is_p2p || ["utun","tun","tailscale","wg","zt","ppp","wireguard"].iter().any(|p|i.name.to_lowercase().starts_with(p))))
}
/// Require the OS route and the peer subnet to agree on an attached interface.
pub fn bridge_on_link(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    if ip.is_loopback() { return true; }
    if !on_link(ip) { return false; }
    let any = if ip.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let Ok(local) = std::net::UdpSocket::bind(any).and_then(|socket| {
        socket.connect(std::net::SocketAddr::new(ip, 9))?;
        socket.local_addr()
    }) else { return false; };
    if_addrs::get_if_addrs().unwrap_or_default().iter().any(|iface| {
        if iface.is_p2p || iface.ip() != local.ip() { return false; }
        #[cfg(any(target_os="linux",target_os="macos"))]
        if !physical_interface(&iface.name) { return false; }
        match (&iface.addr, ip) {
            (if_addrs::IfAddr::V4(a), IpAddr::V4(p)) => u32::from(a.netmask)!=0 && u32::from(a.ip)&u32::from(a.netmask)==u32::from(p)&u32::from(a.netmask),
            (if_addrs::IfAddr::V6(a), IpAddr::V6(p)) => u128::from(a.netmask)!=0 && u128::from(a.ip)&u128::from(a.netmask)==u128::from(p)&u128::from(a.netmask),
            _ => false,
        }
    })
}
/// A narrower boundary than discovery: the HTTP Host must be this physical interface's
/// LAN IP, and the socket peer must be another host on that same subnet.
pub fn direct_lan_admin_peer(host: IpAddr, peer: IpAddr) -> bool {
    let host = host.to_canonical();
    let peer = peer.to_canonical();
    let private = |ip: IpAddr| match ip {
        IpAddr::V4(v) => v.is_private() || v.is_link_local(),
        IpAddr::V6(v) => v.is_unique_local() || v.is_unicast_link_local(),
    };
    if !private(host) || !private(peer) { return false; }
    let interfaces = if_addrs::get_if_addrs().unwrap_or_default();
    if interfaces.iter().any(|interface|interface.ip()==peer) { return false; }
    let any=match peer {IpAddr::V4(_)=>"0.0.0.0:0",IpAddr::V6(_)=>"[::]:0"};
    let routed=std::net::UdpSocket::bind(any).and_then(|socket|{
        socket.connect(std::net::SocketAddr::new(peer,9))?;
        socket.local_addr()
    });
    let Ok(routed)=routed else {return false;};
    interfaces.iter().any(|interface| {
        if !interfaces.iter().any(|i|i.name==interface.name && i.ip()==routed.ip()) {return false;}
        if interface.ip()!=host || interface.is_p2p || !physical_interface(&interface.name) {
            return false;
        }
        match (&interface.addr,peer) {
            (if_addrs::IfAddr::V4(a),IpAddr::V4(p)) => {
                let mask=u32::from(a.netmask);
                mask!=0 && u32::from(a.ip)&mask==u32::from(p)&mask && !default_ipv4_gateway(p)
            }
            (if_addrs::IfAddr::V6(a),IpAddr::V6(p)) => {
                let mask=u128::from(a.netmask);
                mask!=0 && u128::from(a.ip)&mask==u128::from(p)&mask
            }
            _ => false,
        }
    })
}

#[cfg(target_os="linux")]
fn physical_interface(name:&str)->bool {
    let root=std::path::Path::new("/sys/class/net");
    // Synology places the L3 address on ovs_ethN above the real ethN device.
    root.join(name.strip_prefix("ovs_").unwrap_or(name)).join("device").exists()
}
#[cfg(target_os="macos")]
fn physical_interface(name:&str)->bool {
    name.strip_prefix("en").is_some_and(|n|!n.is_empty() && n.bytes().all(|c|c.is_ascii_digit()))
}
#[cfg(not(any(target_os="macos",target_os="linux")))]
fn physical_interface(_: &str)->bool { false }

#[cfg(target_os="linux")]
fn default_ipv4_gateway(peer:Ipv4Addr)->bool {
    let Ok(routes)=std::fs::read_to_string("/proc/net/route") else {return true;};
    routes.lines().skip(1).any(|line| {
        let mut fields=line.split_whitespace().skip(1);
        fields.next()==Some("00000000") && fields.next().and_then(|v|u32::from_str_radix(v,16).ok())
            .is_some_and(|value|value.to_ne_bytes()==peer.octets())
    })
}
#[cfg(not(target_os="linux"))]
fn default_ipv4_gateway(_:Ipv4Addr)->bool { false }

pub fn dual_stack_listener(port:u16)->Result<std::net::TcpListener>{
    let socket=socket2::Socket::new(socket2::Domain::IPV6,socket2::Type::STREAM,Some(socket2::Protocol::TCP))?;socket.set_only_v6(false)?;socket.bind(&std::net::SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED,port)).into())?;socket.listen(128)?;socket.set_nonblocking(true)?;Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]fn rejects_unsafe_discovery_paths_and_identifiers(){
        for path in ["//evil","/../admin","/%2e%2e/a","/music?token=x","/music#x","/a\\b"] {assert!(!valid_path(path));}
        assert!(valid_path("/音乐/music"));assert!(!valid_id("core\nname"));
    }
    #[test]fn parses_only_versioned_records_and_safe_paths(){
        let info=ServiceInfo::new(CORE_SERVICE,"fixture","fixture.local.","192.168.1.2",7800,&[("v","1"),("id","fixture-core"),("name","验收核心"),("path","/music"),("tls","1")][..]).unwrap();
        let service=resolved(&info.as_resolved_service()).unwrap();
        assert_eq!(service.addresses,vec!["https://192.168.1.2:7800/music"]);
        for (version,path,ip) in [("2","/music","192.168.1.2"),("1","//other","192.168.1.2"),("1","/music","127.0.0.1")] {
            let info=ServiceInfo::new(CORE_SERVICE,"fixture","fixture.local.",ip,7800,&[("v",version),("id","core"),("path",path)][..]).unwrap();
            assert!(resolved(&info.as_resolved_service()).is_none());
        }
    }
    #[test]
    #[ignore="requires a multicast-capable LAN interface"]
    fn real_mdns_announcement_and_withdrawal(){
        let addresses=local_ipv4();assert!(!addresses.is_empty(),"no private LAN IPv4 interface");
        let listener=Discovery::new().unwrap();let publisher=Discovery::new().unwrap();
        let id=format!("qa-discovery-{}",std::process::id());
        let record=publisher.advertise("core",&id,"QA 发现验收",17800,"/music",false,None,&addresses).unwrap();
        let deadline=std::time::Instant::now()+Duration::from_secs(15);
        while !listener.snapshot().iter().any(|item|item.id==id)&&std::time::Instant::now()<deadline {thread::sleep(Duration::from_millis(100));}
        assert!(listener.snapshot().iter().any(|item|item.id==id&&item.addresses.iter().all(|a|a.ends_with(":17800/music"))));
        publisher.withdraw(&record);
        let deadline=std::time::Instant::now()+Duration::from_secs(10);
        while listener.snapshot().iter().any(|item|item.id==id)&&std::time::Instant::now()<deadline {thread::sleep(Duration::from_millis(100));}
        assert!(!listener.snapshot().iter().any(|item|item.id==id));
    }
}
