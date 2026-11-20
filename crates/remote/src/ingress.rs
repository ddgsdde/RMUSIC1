//! A single generation owns each DDNS, certificate and HTTPS task. Reconfiguration
//! cancels and joins that generation before starting its replacement.
mod changes;
use crate::{
    certificate::{self, Files},
    config::{Challenge, Ingress},
};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct Providers {
    pub cloudflare: Option<String>,
    pub eab: Option<String>,
}
pub type Status = watch::Sender<Value>;
pub fn status() -> Status { watch::channel(json!({})).0 }
async fn report(status:&Status,key:&str,value:Value) {
    status.send_if_modified(|all| {
        if all.get(key)==Some(&value) { return false; }
        all[key]=value; true
    });
}

fn ipv6_snapshot(interface:Option<&str>)->Value {
    let addresses:Vec<_>=crate::ddns::local_public_addresses().into_iter()
        .filter(|a|a.address.is_ipv6() && interface.is_none_or(|name|a.interface==name)).map(|v|v.address).collect();
    json!({"addresses":addresses,"routed":crate::ddns::routed_ipv6()})
}
/// Stops only the listeners and provider tasks owned by this feature. Existing
/// Core LAN and system reverse-proxy listeners are never changed.
pub async fn run(
    config: Ingress,
    providers: Providers,
    root: PathBuf,
    owner: String,
    router: axum::Router,
    status: Status,
    cancel: CancellationToken,
) -> Result<()> {
    config.validate()?;
    if !config.managed() {
        report(&status, "https", json!({"state":"external_or_disabled"})).await;
        cancel.cancelled().await;
        return Ok(());
    }
    crate::vault::private_directory(&root)?;
    let multiplex = config.certificate.challenge == Challenge::TlsAlpn01
        && (config.ports.https_listen == 443 || config.ports.https_public == 443);
    let alpn_port = if multiplex {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, config.ports.acme_tls_listen))?;
        Some(listener.local_addr()?.port())
    } else {
        None
    };
    let (cert_tx, cert_rx) = watch::channel::<Option<Files>>(None);
    let mut jobs = tokio::task::JoinSet::new();
    let c = cancel.child_token();
    let s = status.clone();
    let cfg = config.clone();
    let initial_ipv6=ipv6_snapshot(config.ddns.interface.as_deref());
    let (ip_tx,mut ip_rx)=watch::channel(initial_ipv6.clone());
    let _keep_ip_sender=ip_tx.clone();
    {
        let events=changes::Changes::addresses();
        let interface=config.ddns.interface.clone();let status=status.clone();let cancelled=cancel.clone();
        jobs.spawn(async move {
            // Subscribe before reading again so address changes during initialization cannot be lost.
            loop {
                let next=ipv6_snapshot(interface.as_deref());
                report(&status,"local_ipv6",next.clone()).await;
                ip_tx.send_if_modified(|old|if *old==next {false} else {*old=next;true});
                tokio::select!{_=events.wait(Duration::from_secs(60))=>{},_=cancelled.cancelled()=>break}
            }
        });
    }
    if let Some(target)=config.advertised_url().and_then(|url|url::Url::parse(&url).ok()) {
        if let Some(url::Host::Domain(host))=target.host() {
            let host=host.to_owned();let port=target.port_or_known_default().unwrap_or(443);
            let status=status.clone();let cancelled=cancel.clone();
            jobs.spawn(async move {
                loop {
                    let answer=tokio::time::timeout(Duration::from_secs(7),crate::dns::resolve(&host,port)).await;
                    if let Ok(Ok(records))=answer {
                        let mut ips:Vec<_>=records.into_iter().map(|r|r.ip()).collect();ips.sort_unstable();ips.dedup();
                        report(&status,"resolved_ips",json!(ips)).await;
                    }
                    tokio::select!{_=tokio::time::sleep(Duration::from_secs(60))=>{},_=cancelled.cancelled()=>break}
                }
            });
        }
    }
    let cert_root = root.join("certificates");
    let token = providers.cloudflare.clone();
    let eab = providers.eab.clone();
    jobs.spawn(async move{
        let lego=std::env::var_os("RMUSIC_LEGO").map(PathBuf::from).or_else(||std::env::current_exe().ok().and_then(|p|p.parent().map(|p|p.join("lego")))).unwrap_or_else(||"lego".into());
        let mut delay=Duration::from_secs(30);
        loop{
            let current:Option<Files>=std::fs::read(cert_root.join("active.json")).ok().and_then(|v|serde_json::from_slice(&v).ok());
            let external=matches!(cfg.certificate.challenge,Challenge::Imported|Challenge::External);
            let current=if external{Some(Files{cert:cfg.certificate.external_cert.clone().unwrap_or_default().into(),key:cfg.certificate.external_key.clone().unwrap_or_default().into()})}else{current};
            let active=cert_root.join("active.json");
            let mut paths=Vec::new();
            if !external {paths.push(active.as_path());}
            if let Some(files)=current.as_ref(){paths.extend([files.cert.as_path(),files.key.as_path()]);}
            let file_events=changes::Changes::files(&paths);
            let mut good=false;
            if let Some(files)=current.as_ref(){if certificate::validate(files,&cfg.domain).await.is_ok(){cert_tx.send_replace(Some(files.clone()));good=true;}}
            let due=!good||(!external&&current.as_ref().is_some_and(|f|certificate::renewal_due(f).unwrap_or(true)));
            if due {let _publication=crate::ddns::PUBLICATION_LOCK.lock().await;match certificate::obtain_with_alpn(&cfg,&cert_root,&lego,token.as_deref(),eab.as_deref(),c.clone(),alpn_port).await{
                Ok(files)=>{cert_tx.send_replace(Some(files));delay=Duration::from_secs(3600);report(&s,"certificate",json!({"state":"valid","external":external})).await;},
                Err(error)=>{report(&s,"certificate",json!({"state":if good{"renewal_failed_old_valid"}else{"unavailable"},"message":error.to_string()})).await;delay=(delay*2).min(Duration::from_secs(3600));}
            }}else{delay=if external{file_events.fallback_interval()}else{Duration::from_secs(3600)};report(&s,"certificate",json!({"state":"valid","external":external})).await;}
            let wait=file_events.wait(delay);
            tokio::select!{_=wait=>{},_=c.cancelled()=>break}
        }
    });
    if config.ddns.enabled {
        let cfg = config.clone();
        let c = cancel.child_token();
        let s = status.clone();
        let owner = owner.clone();
        let token = providers.cloudflare;
        jobs.spawn(async move{let mut delay=30;let mut generation=0u64;loop{
            if c.is_cancelled(){break;}
            let result={
                let _publication=crate::ddns::PUBLICATION_LOCK.lock().await;
                if let Some(token)=token.as_ref(){crate::ddns::update_cancellable(&cfg,token.clone(),&owner,c.clone()).await}else{Err(anyhow::anyhow!("DDNS 需要 Cloudflare Token"))}
            };
            if c.is_cancelled(){break;}
            match result{Ok(records)=>{
                if records.iter().any(|r|r.changed){generation=generation.saturating_add(1);}
                report(&s,"published_dns",json!({"generation":generation,"records":records.iter().map(|r|(&r.name,&r.kind,r.address)).collect::<Vec<_>>()})).await;
                report(&s,"ddns",json!({"state":"ok","records":records})).await;delay=60;},Err(error)=>{report(&s,"ddns",json!({"state":"retrying","message":error.to_string()})).await;delay=(delay*2).min(1800);}}
            tokio::select!{_=tokio::time::sleep(Duration::from_secs(delay))=>{},Ok(())=ip_rx.changed(), if cfg.ddns.auto_ipv6=>{},_=c.cancelled()=>break}
        }});
    } else {
        report(&status, "ddns", json!({"state":"disabled"})).await;
    }
    let listener_result = listen(
        config,
        router,
        cert_rx,
        status.clone(),
        cancel.clone(),
        alpn_port,
    )
    .await;
    cancel.cancel();
    // A bounded DNS write already accepted by the provider must settle before
    // another configuration generation can write the same record.
    if tokio::time::timeout(Duration::from_secs(25), async { while jobs.join_next().await.is_some() {} }).await.is_err() {
        jobs.abort_all(); while jobs.join_next().await.is_some() {}
    }
    listener_result
}

async fn listen(
    config: Ingress,
    router: axum::Router,
    mut cert: watch::Receiver<Option<Files>>,
    status: Status,
    cancel: CancellationToken,
    alpn_port: Option<u16>,
) -> Result<()> {
    // Bind first: conflicts and permission errors fail immediately, before ACME.
    let address =
        std::net::SocketAddr::new(config.listen_address.parse()?, config.ports.https_listen);
    let listener =
        std::net::TcpListener::bind(address).context("HTTPS 监听端口不可用；不会停止其他服务")?;
    listener.set_nonblocking(true)?;
    report(
        &status,
        "https",
        json!({"state":"waiting_certificate","listen":address.to_string()}),
    )
    .await;
    if let Some(port) = alpn_port {
        let (tx, rx) = watch::channel(None);
        let cfg = config.clone();
        let c = cancel.clone();
        let s = status.clone();
        let watcher = tokio::spawn(async move {
            loop {
                let files = cert.borrow().clone();
                if let Some(files) = files {
                    match certificate::validated_config(&files, &cfg.domain).await {
                        Ok((tls, fingerprint)) => {
                            tx.send_replace(Some(tls.get_inner()));
                            report(
                                &s,
                                "https",
                                json!({"state":"listening","url":cfg.advertised_url(),"certificate_sha256":fingerprint}),
                            )
                            .await;
                        }
                        Err(_) => {}
                    }
                }
                tokio::select! {v=cert.changed()=>{if v.is_err(){break;}},_=c.cancelled()=>break}
            }
        });
        let result = crate::alpn::serve(
            tokio::net::TcpListener::from_std(listener)?,
            config.domain,
            port,
            rx,
            router,
            cancel,
        )
        .await;
        watcher.abort();
        return result;
    }
    let files = loop {
        if let Some(files) = cert.borrow().clone() {
            break files;
        }
        tokio::select! {v=cert.changed()=>{v?;},_=cancel.cancelled()=>return Ok(())}
    };
    let (tls,fingerprint) = certificate::validated_config(&files, &config.domain).await?;
    let handle = axum_server::Handle::new();
    let h = handle.clone();
    let c = cancel.clone();
    let reload = tls.clone();
    let domain = config.domain.clone();
    let s = status.clone();
    report(
        &status,
        "https",
        json!({"state":"listening","listen":address.to_string(),"url":config.advertised_url(),"certificate_sha256":fingerprint}),
    )
    .await;
    let mut active_fingerprint=fingerprint.clone();
    let watcher = tokio::spawn(async move {
        loop {
            tokio::select! {
                result=cert.changed()=>{
                    if result.is_err(){break;}
                    let files=cert.borrow_and_update().clone();
                    if let Some(files)=files {
                        if let Ok((next,key))=certificate::validated_config(&files,&domain).await {
                            if key!=active_fingerprint {
                                reload.reload_from_config(next.get_inner());
                                active_fingerprint=key.clone();
                                s.send_modify(|all|all["https"]["certificate_sha256"]=json!(key));
                            }
                        }
                    }
                },
                _=c.cancelled()=>{h.graceful_shutdown(Some(Duration::from_secs(5)));return;}
            }
        }
    });

    let result = axum_server::from_tcp_rustls(listener, tls)?
        .handle(handle)
        .serve(router.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .await;
    watcher.abort();
    result?;
    Ok(())
}
