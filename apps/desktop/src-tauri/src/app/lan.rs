use super::*;
use std::net::{IpAddr,Ipv4Addr};

/// Docker Desktop does not forward mDNS. Only relay a loopback core after checking its LAN listener.
pub(super) fn schedule(app:tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state=app.state::<DesktopState>();
        let Some(discovery)=state.discovery.clone() else{return;};
        let mut published:HashMap<String,(String,Vec<Ipv4Addr>)>=HashMap::new();
        loop {
            let profiles=state.profiles.lock().map(|p|p.list()).unwrap_or_default();
            let mut live=Vec::new();
            for profile in profiles {
                let Ok(base)=profile.base_url() else{continue;};
                let local=base.host_str().is_some_and(|host|host=="localhost"||host.parse::<IpAddr>().is_ok_and(|ip|ip.is_loopback()));
                if !local||base.scheme()!="http" {continue;}
                let Ok(selected)=profile.resolve().await else{continue;};
                let Some(id)=selected.core_id else{continue;};
                let mut addresses=Vec::new();
                for ip in rmusic_discovery::local_ipv4() {
                    let mut url=base.clone();if url.set_host(Some(&ip.to_string())).is_err(){continue;}
                    let candidate=ConnectionProfile{core_public_key:None,address:url.to_string(),skip_certificate_verification: false, certificate_sha256: None,core_id:Some(id.clone()),alternatives:vec![],..profile.clone()};
                    if tokio::time::timeout(std::time::Duration::from_secs(2),candidate.resolve()).await.is_ok_and(|r|r.is_ok()){addresses.push(ip);}
                }
                if addresses.is_empty(){continue;}
                live.push(id.clone());
                if published.get(&id).is_some_and(|(_,old)|old==&addresses){continue;}
                if let Some((old,_))=published.remove(&id){discovery.withdraw(&old);}
                if let Ok(record)=discovery.advertise("core",&id,"R-Music",base.port_or_known_default().unwrap_or(7800),base.path().trim_end_matches('/'),false,None,&addresses){
                    published.insert(id,(record,addresses));
                }
            }
            published.retain(|id,(record,_)|{if live.contains(id){true}else{discovery.withdraw(record);false}});
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}
