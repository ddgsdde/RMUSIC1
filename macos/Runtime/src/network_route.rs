//! A private/VPN address is not evidence of a same-link LAN.
use rmusic_bridge::network::{address_kind,AddressKind};
use std::{net::IpAddr,time::{Duration,Instant},collections::HashMap,sync::{Mutex,OnceLock}};

pub async fn remote(address:&str)->bool {
 if rmusic_bridge::connection::remote_gateway(address){return true;}
 if address_kind(address)==AddressKind::Loopback{return false;}
 if address_kind(address)==AddressKind::Overlay{return true;}
 static CACHE:OnceLock<Mutex<HashMap<String,(Instant,bool)>>>=OnceLock::new();
 let cache=CACHE.get_or_init(Default::default);
 let (subnets,tunnels)=interfaces();
 let key=format!("{address}:{subnets:?}:{tunnels:?}");
 if let Some(value)=cache.lock().unwrap().get(&key).filter(|v|v.0.elapsed()<Duration::from_secs(5)){return value.1;}
 let value=if let Ok(url)=url_address(address){
  let host=url.0;let port=url.1;
  let ips=if let Ok(ip)=host.parse::<IpAddr>(){vec![ip]}else{
   tokio::time::timeout(Duration::from_secs(1),tokio::net::lookup_host((host.as_str(),port))).await.ok().and_then(Result::ok).map(|rows|rows.map(|row|row.ip()).collect()).unwrap_or_default()
  };
  !ips.into_iter().any(rmusic_discovery::bridge_on_link)
 }else{true};
 let mut rows=cache.lock().unwrap();rows.retain(|_,v|v.0.elapsed()<Duration::from_secs(5));if rows.len()>=32{rows.clear();}rows.insert(key,(Instant::now(),value));value
}
fn url_address(address:&str)->anyhow::Result<(String,u16)>{let url=reqwest::Url::parse(address)?;Ok((url.host_str().ok_or_else(||anyhow::anyhow!("missing host"))?.trim_matches(['[',']']).into(),url.port_or_known_default().unwrap_or(443)))}
#[cfg(target_os="macos")]
pub(crate) fn interfaces()->(Vec<String>,Vec<String>){
 let mut head=std::ptr::null_mut();let mut physical=Vec::new();let mut tunnels=Vec::new();
 unsafe {
  if libc::getifaddrs(&mut head)!=0{return (physical,tunnels);}
  let mut row=head;
  while !row.is_null(){let value=&*row;row=value.ifa_next;
   if value.ifa_addr.is_null()||value.ifa_netmask.is_null()||value.ifa_name.is_null()||value.ifa_flags&libc::IFF_UP as u32==0{continue;}
   let name=std::ffi::CStr::from_ptr(value.ifa_name).to_string_lossy();
   let result=if name.starts_with("en")||name.starts_with("bridge"){&mut physical}
     else if ["utun","tun","tailscale","wg","zt","ppp"].iter().any(|prefix|name.starts_with(prefix)){&mut tunnels}
     else {continue;};
   match (*value.ifa_addr).sa_family as i32 {
    libc::AF_INET=>{let addr=&*(value.ifa_addr as *const libc::sockaddr_in);let mask=&*(value.ifa_netmask as *const libc::sockaddr_in);result.push(format!("{}/{}",std::net::Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr)),u32::from_be(mask.sin_addr.s_addr).leading_ones()));},
    libc::AF_INET6=>{let addr=&*(value.ifa_addr as *const libc::sockaddr_in6);let mask=&*(value.ifa_netmask as *const libc::sockaddr_in6);result.push(format!("{}/{}",std::net::Ipv6Addr::from(addr.sin6_addr.s6_addr),u128::from_be_bytes(mask.sin6_addr.s6_addr).leading_ones()));},_=>{}
   }
  }
  libc::freeifaddrs(head);
 }
 physical.sort_unstable();physical.dedup();tunnels.sort_unstable();tunnels.dedup();
 (physical,tunnels)
}
#[cfg(not(target_os="macos"))]pub(crate) fn interfaces()->(Vec<String>,Vec<String>){Default::default()}

pub(crate) async fn selected_is_remote(address:&str)->bool {
 remote(&rmusic_bridge::connection::logical_address(address)).await
}
