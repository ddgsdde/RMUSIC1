use crate::state::AppState;
use rmusic_remote::{config::IngressMode, runtime::Secrets};
use serde_json::json;
use std::{sync::Arc, time::Duration};

fn providers(s: &AppState) -> Secrets {
    s.remote.vault.get("providers").unwrap_or_default().unwrap_or_default()
}
pub(crate) async fn verify_url(s:&AppState,url:&str,tls:Option<Arc<rustls::ClientConfig>>)->anyhow::Result<()> {
    let gateway = rmusic_remote::native::open_core(url,s.core_id.to_string(),s.remote.identity.public_key(),tls,true).await?;
    let mut response = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(6)).build()?
        .get(format!("{}/api/v1/status",gateway.address)).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(bytes.len()+chunk.len()<=16384,"入口检查响应过大"); bytes.extend(chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(value["core_id"]==s.core_id.as_str() && value["core_public_key"]==s.remote.identity.public_key(),"外部入口指向另一 Core");
    anyhow::ensure!(value["relayed"]!=true,"请使用 Core 的直连入口");
    Ok(())
}

async fn check_direct_urls(s:AppState) {
    use futures_util::{stream, StreamExt};
    let mut changed=s.remote.changed.subscribe();
    loop {
        let config=s.remote.configuration().await;
        let urls=config.direct_urls.unwrap_or_default();
        let checks=stream::iter(urls).map(|url| {
            let state=s.clone();
            async move {
                let verified=matches!(tokio::time::timeout(Duration::from_secs(10),verify_url(&state,&url,None)).await, Ok(Ok(())));
                (url,verified)
            }
        }).buffered(3).collect::<Vec<_>>();
        let results=tokio::select!{results=checks=>results,_=changed.changed()=>continue,_=s.remote.cancel.cancelled()=>return};
        let verified=results.iter().filter(|(_,ok)|*ok).map(|(url,_)|url.clone()).collect();
        if s.remote.publish_direct_checks(config.revision,verified).await.is_ok() {
            s.remote.status.write().await["direct_urls"]=json!(results.into_iter().map(|(url,verified)|json!({"url":url,"verified":verified})).collect::<Vec<_>>());
        }
        tokio::select!{_=tokio::time::sleep(Duration::from_secs(60))=>{},_=changed.changed()=>{},_=s.remote.cancel.cancelled()=>return}
    }
}
const HEALTHY_FALLBACK:Duration=Duration::from_secs(15*60);

fn probe_input(mode:&IngressMode, network:&serde_json::Value)->Option<serde_json::Value>{
    match mode {
        IngressMode::LocalOnly=>None,
        IngressMode::ReverseProxy=>Some(json!("reverse_proxy")),
        IngressMode::ManagedHttps if network["https"]["state"]=="listening"=>Some(json!([
            network["https"]["certificate_sha256"],network["local_ipv6"],network["published_dns"],network["resolved_ips"]
        ])),
        _=>None,
    }
}

async fn watch_endpoint(s:AppState, core:rmusic_remote::config::Ingress, mut updates:tokio::sync::watch::Receiver<serde_json::Value>) {
    use tokio::time::{sleep_until,Instant};
    let mut url=core.advertised_url().unwrap_or_default();
    if core.managed(){url.push_str(&s.config.base_path);}
    let mut previous=None;
    let mut due=None;
    let mut retry=Duration::from_secs(5);
    let mut checks=0u64;
    let mut reason="startup";
    loop {
        let network=updates.borrow_and_update().clone();
        s.remote.status.write().await["core_ingress"]=network.clone();
        let input=probe_input(&core.mode,&network);
        if input!=previous {
            reason=if checks==0 {"startup"} else {"changed"};
            previous=input;
            due=previous.as_ref().map(|_|Instant::now()+Duration::from_millis(250));
            retry=Duration::from_secs(5);
        }
        let Some(deadline)=due else {
            if updates.changed().await.is_err(){return;}
            continue;
        };
        tokio::select! {
            biased;
            changed=updates.changed()=>{if changed.is_err(){return;}}
            _=sleep_until(deadline)=>{
                checks+=1;
                let start=Instant::now();
                let result=verify_url(&s,&url,None).await;
                if probe_input(&core.mode,&updates.borrow())!=previous {continue;}
                let ok=match result {
                    Ok(())=>s.remote.mark_ingress_verified(&core,url.clone()).await.is_ok(),
                    Err(_)=>false,
                };
                let delay=if ok {retry=Duration::from_secs(5);HEALTHY_FALLBACK}
                    else {let delay=retry;retry=(retry*2).min(Duration::from_secs(120));delay};
                let mut status=json!({"state":if ok {"verified"} else {"unverified"},"checks":checks,
                    "trigger":reason,"elapsed_ms":start.elapsed().as_millis() as u64,"next_check_seconds":delay.as_secs()});
                if !ok {status["message"]=json!("入口未通过身份与 HTTPS 检查，正在重试。请核对 DNS、端口和 IPv6 防火墙。");}
                s.remote.status.write().await["core_endpoint"]=status;
                reason=if ok {"fallback"}else{"retry"};
                due=Some(Instant::now()+delay);
            }
        }
    }
}

pub fn spawn(s: AppState) {
    tokio::spawn(check_direct_urls(s.clone()));
    tokio::spawn(async move {
        let mut changes = s.remote.changed.subscribe();
        loop {
            let core = s.remote.configuration().await.core;
            let provider = providers(&s);
            s.remote.status.write().await["core_endpoint"]=json!({"state":if core.mode==IngressMode::LocalOnly {"disabled"} else {"checking"}});
            let status = rmusic_remote::ingress::status();
            let monitor=watch_endpoint(s.clone(),core.clone(),status.subscribe());
            tokio::pin!(monitor);
            let cancel = s.remote.cancel.child_token();
            let state = s.clone(); let c = cancel.clone(); let result = status.clone();
            let config = core.clone(); let cloudflare = provider.core_cloudflare.clone(); let eab = provider.core_eab.clone();
            let task = tokio::spawn(async move {
                if let Err(error) = rmusic_remote::ingress::run(config,
                    rmusic_remote::ingress::Providers{cloudflare,eab}, state.remote.root.join("ingress"),state.core_id.to_string(),
                    crate::app(state.clone()).layer(axum::Extension(super::SecureIngress)),result.clone(),c).await {
                    result.send_replace(json!({"state":"failed","message":error.to_string()}));
                }
            });
            loop {
                tokio::select! {
                    _=changes.changed()=>{
                        let next=s.remote.configuration().await.core; let secrets=providers(&s);
                        if next!=core || secrets.core_cloudflare!=provider.core_cloudflare || secrets.core_eab!=provider.core_eab {break;}
                    },
                    _=s.remote.cancel.cancelled()=>{cancel.cancel();let _=task.await;return;},
                    _=&mut monitor=>break,
                }
            }
            cancel.cancel(); let _=task.await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_trigger_ignores_repeated_healthy_reports_but_tracks_actual_changes(){
        let mode=IngressMode::ManagedHttps;
        let mut v=json!({"https":{"state":"listening","certificate_sha256":"first"},"local_ipv6":{"routed":"240e::1"},"ddns":{"state":"ok"}});
        assert!(probe_input(&mode,&json!({"https":{"state":"waiting_certificate"}})).is_none());
        let initial=probe_input(&mode,&v);
        v["ddns"]["records"]=json!([{"changed":false}]);assert_eq!(initial,probe_input(&mode,&v));
        for (k,val) in [("local_ipv6",json!({"routed":"240e::2"})),("published_dns",json!({"generation":2})),("resolved_ips",json!(["240e::2"]))] {
            let mut next=v.clone();next[k]=val;assert_ne!(initial,probe_input(&mode,&next));
        }
        v["https"]["certificate_sha256"]=json!("rotated");assert_ne!(initial,probe_input(&mode,&v));
        assert_eq!(HEALTHY_FALLBACK.as_secs(),900);
    }
}
