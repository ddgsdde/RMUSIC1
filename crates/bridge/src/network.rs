//! Address hints choose buffering, never TLS trust or audio quality.
use std::net::IpAddr;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum AddressKind {Loopback,Private,Overlay,Public}
pub fn address_kind(address:&str)->AddressKind {
 let Ok(url)=url::Url::parse(address) else{return AddressKind::Public;};
 match url.host() {
  Some(url::Host::Ipv4(ip)) if ip.is_loopback()=>AddressKind::Loopback,
  Some(url::Host::Ipv4(ip)) if ip.octets()[0]==100&&(64..=127).contains(&ip.octets()[1])=>AddressKind::Overlay,
  Some(url::Host::Ipv4(ip)) if ip.is_private()||ip.is_link_local()=>AddressKind::Private,
  Some(url::Host::Ipv6(ip)) if ip.is_loopback()=>AddressKind::Loopback,
  Some(url::Host::Ipv6(ip)) if ip.segments()[..3]==[0xfd7a,0x115c,0xa1e0]=>AddressKind::Overlay,
  Some(url::Host::Ipv6(ip)) if ip.is_unique_local()||ip.is_unicast_link_local()=>AddressKind::Private,
  Some(url::Host::Domain(host)) if host=="localhost"=>AddressKind::Loopback,
  Some(url::Host::Domain(host)) if host.ends_with(".ts.net")=>AddressKind::Overlay,
  Some(url::Host::Domain(host)) if host.ends_with(".local")||!host.contains('.')=>AddressKind::Private,
  _=>AddressKind::Public,
 }
}
pub fn on_link(ip:IpAddr,subnets:&[String])->bool {
 subnets.iter().any(|subnet|{
  let Some((base,bits))=subnet.split_once('/') else{return false;};
  match (ip,base.parse::<IpAddr>(),bits.parse::<u32>()) {
   (IpAddr::V4(ip),Ok(IpAddr::V4(base)),Ok(bits)) if (1..=32).contains(&bits)=>u32::from(ip)>>(32-bits)==u32::from(base)>>(32-bits),
   (IpAddr::V6(ip),Ok(IpAddr::V6(base)),Ok(bits)) if (1..=128).contains(&bits)=>u128::from(ip)>>(128-bits)==u128::from(base)>>(128-bits),
   _=>false,
  }
 })
}
#[cfg(test)] mod tests {
 use super::*;
 #[test] fn tailscale_is_not_a_fast_lan_assumption(){
  for address in ["http://100.64.1.2:18854","https://100.127.255.254","http://[fd7a:115c:a1e0::1]","https://nas.tail123.ts.net"]{assert_eq!(address_kind(address),AddressKind::Overlay);}
  assert_eq!(address_kind("http://192.168.50.10"),AddressKind::Private);
  assert_eq!(address_kind("http://100.128.1.2"),AddressKind::Public);
  assert!(on_link("192.168.50.10".parse().unwrap(),&["192.168.50.2/24".into()]));
  assert!(!on_link("192.168.50.10".parse().unwrap(),&["10.22.108.2/24".into()]));
 }
}
