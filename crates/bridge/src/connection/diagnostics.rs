use super::ConnectionProfile;
use anyhow::{ensure,Result};
use serde_json::{json,Value};
use std::{net::IpAddr,time::{Duration,Instant}};

const SIZE:usize=1024*1024;
fn mode(underlay:&str,ingress:&str,scheme:&str)->&'static str {
    match (ingress,underlay,scheme) {
        ("reverse_proxy",_,_)=>"reverse_proxy",(_,"vpn",_)=>"vpn",(_,"local",_)=>"local",
        (_,"lan",_)=>"lan",(_,"private",_)=>"private",(_,_,"https")=>"https",_=>"direct",
    }
}
impl ConnectionProfile {
    /// Measure the same selected authenticated transport used by ordinary API requests.
    pub async fn connection_metrics(&self,token:&str,speed:bool)->Result<Value>{
        let http=self.http()?;let ping=self.endpoint("api/v1/connection-probe")?;
        let mut elapsed=Vec::with_capacity(3);let mut ingress=String::new();
        for _ in 0..3 {
            let start=Instant::now();
            let data:Value=http.get(ping.clone()).bearer_auth(token).timeout(Duration::from_secs(6)).send().await?.error_for_status()?.json().await?;
            ingress=data["ingress"].as_str().ok_or_else(||anyhow::anyhow!("Core 不支持连接诊断"))?.to_owned();
            elapsed.push(start.elapsed().as_secs_f64()*1000.0);
        }
        elapsed.sort_by(f64::total_cmp);
        let address=super::logical_address(&self.address);let base=url::Url::parse(&address)?;
        let host=base.host_str().unwrap_or("").trim_matches(['[',']']);
        let ip=host.parse::<IpAddr>().ok();
        let underlay=if ip.is_some_and(|i|i.is_loopback()) || host=="localhost" {"local"}
            else if ip.is_some_and(rmusic_discovery::via_overlay) || crate::network::address_kind(&address)==crate::network::AddressKind::Overlay {"vpn"}
            else if ip.is_some_and(rmusic_discovery::bridge_on_link) {"lan"}
            else if crate::network::address_kind(&address)==crate::network::AddressKind::Private {"private"}
            else {"unknown"};
        let transport=super::remote_status(&self.address);
        let label=mode(underlay,&ingress,base.scheme());
        let mut result=json!({"mode":label,"underlay":underlay,"address":address,"latency_ms":elapsed[1],"end_to_end":transport["end_to_end"]==true});
        if speed {
            let url=self.endpoint("api/v1/connection-probe/payload")?;
            let start=Instant::now();
            let response=http.get(url.clone()).bearer_auth(token).timeout(Duration::from_secs(15)).send().await?.error_for_status()?;
            let mut response=response;let mut size=0;
            while let Some(chunk)=response.chunk().await?{size+=chunk.len();ensure!(size<=SIZE,"测速响应过大");}
            ensure!(size==SIZE,"测速下载长度不符");
            result["down_mbps"]=json!(SIZE as f64*8e-6/start.elapsed().as_secs_f64());
            ensure!(super::remote_status(&self.address)["path"]==transport["path"],"连接路径已切换，请重新测速");
        }
        Ok(result)
    }
}
