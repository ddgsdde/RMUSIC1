//! Preserve system/split DNS. Failed public-name lookups receive a bounded,
//! authenticated HTTPS DNS fallback; credentials are never included in queries.
use anyhow::{ensure, Result};
use serde::Deserialize;
use std::{collections::HashMap,net::{IpAddr,SocketAddr},sync::{Mutex,OnceLock},time::{Duration,Instant}};
type Cache=HashMap<String,(Instant,Vec<IpAddr>)>;
fn cache()->&'static Mutex<Cache>{static VALUE:OnceLock<Mutex<Cache>>=OnceLock::new();VALUE.get_or_init(Default::default)}
fn public_name(host:&str)->bool{
    let normalized=host.trim_end_matches('.').to_ascii_lowercase();let host=normalized.as_str();
    host.len()<=253 && host.contains('.') && host.parse::<IpAddr>().is_err()
        && host.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'.'|b'-'))
        && ![".local",".localhost",".invalid",".test",".example",".arpa",".ts.net",".internal",".lan",".home",".onion"].iter().any(|s|host.ends_with(s))
}
pub struct Resolver;
impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self,name:reqwest::dns::Name)->reqwest::dns::Resolving{
        let host=name.as_str().to_owned();
        Box::pin(async move{let values=resolve(&host,0).await?;Ok(Box::new(values.into_iter()) as reqwest::dns::Addrs)})
    }
}
#[derive(Deserialize)]struct Answer{#[serde(rename="type")]kind:u32,data:String}
#[derive(Deserialize)]struct Response{#[serde(rename="Status")]status:u32,#[serde(default,rename="Answer")]answers:Vec<Answer>}
async fn query(host:&str,kind:&str)->Result<Vec<IpAddr>>{
    // Fixed bootstrap addresses avoid depending on the DNS service being repaired.
    // TLS still verifies cloudflare-dns.com through normal public CA validation.
    let http=reqwest::Client::builder().no_proxy().https_only(true).redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs("cloudflare-dns.com",&["[2606:4700:4700::1111]:443".parse()?,"1.1.1.1:443".parse()?])
        .timeout(Duration::from_secs(3)).build()?;
    let mut response=http.get("https://cloudflare-dns.com/dns-query").query(&[("name",host),("type",kind)])
        .header("accept","application/dns-json").send().await?.error_for_status()?;
    let mut bytes=Vec::new();while let Some(chunk)=response.chunk().await?{ensure!(bytes.len()+chunk.len()<=16384,"DNS 响应过大");bytes.extend(chunk);}
    let response:Response=serde_json::from_slice(&bytes)?;ensure!(response.status==0,"公共 DNS 未返回可用地址");
    Ok(response.answers.into_iter().filter(|a|matches!(a.kind,1|28)).filter_map(|a|a.data.parse::<IpAddr>().ok())
        .filter(|ip|crate::config::public_ip(*ip)&&!matches!(ip,IpAddr::V6(v) if v.to_ipv4_mapped().is_some_and(|ip|!crate::config::public_ip(ip.into())))).take(32).collect())
}
pub async fn resolve(host:&str,port:u16)->Result<Vec<SocketAddr>>{Ok(resolve_with_source(host,port).await?.0)}
pub async fn resolve_with_source(host:&str,port:u16)->Result<(Vec<SocketAddr>,bool)>{
    if let Ok(Ok(values))=tokio::time::timeout(Duration::from_secs(2),tokio::net::lookup_host((host,port))).await {
        let values:Vec<_>=values.take(32).collect();
        if !values.is_empty(){return Ok((values,false));}
    }
    ensure!(public_name(host),"系统无法解析此地址；局域网／VPN 专用名称不向公共 DNS 查询");
    public_lookup(host,port).await
}
async fn public_lookup(host:&str,port:u16)->Result<(Vec<SocketAddr>,bool)>{
    if let Some((_,values))=cache().lock().unwrap().get(host).filter(|(at,_)|at.elapsed()<Duration::from_secs(60)){
        return Ok((values.iter().map(|ip|SocketAddr::new(*ip,port)).collect(),true));
    }
    let (a,aaaa)=tokio::join!(query(host,"A"),query(host,"AAAA"));
    let mut values=a.unwrap_or_default();values.extend(aaaa.unwrap_or_default());values.sort();values.dedup();
    ensure!(!values.is_empty(),"系统与安全 DNS 均未返回可达地址");
    let mut all=cache().lock().unwrap();all.retain(|_,(at,_)|at.elapsed()<Duration::from_secs(60));
    if all.len()>=128 {if let Some(key)=all.iter().min_by_key(|(_,v)|v.0).map(|(k,_)|k.clone()){all.remove(&key);}}
    all.insert(host.into(),(Instant::now(),values.clone()));
    Ok((values.into_iter().map(|ip|SocketAddr::new(ip,port)).collect(),true))
}
#[cfg(test)]mod tests{use super::*;
#[tokio::test]async fn private_names_never_use_public_dns(){for name in ["localhost","nas.local","NAS.LOCAL.","nas.home.arpa","nas.tail123.ts.net","core.invalid"]{assert!(!public_name(name));}let result=resolve("127.0.0.1",19864).await.unwrap();assert_eq!(result,["127.0.0.1:19864".parse::<SocketAddr>().unwrap()]);}}
