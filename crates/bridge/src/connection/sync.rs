//! Signed, monotonic connection information. This store contains no account
//! tokens or TLS trust exceptions and is separate from user-edited profiles.
use super::{ConnectionProfile, CoreAddress};
use anyhow::{ensure, Context, Result};
use rmusic_remote::{identity::Signed, runtime::Manifest};
use std::{collections::HashMap, path::{Path, PathBuf}, sync::{Mutex, OnceLock}};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Store {
    root: Option<PathBuf>,
    entries: HashMap<String, Signed<Manifest>>,
    checked: HashMap<String, std::time::Instant>,
    changed_at: HashMap<String, std::time::Instant>,
}
fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Store::default()))
}
pub struct ConnectionSync {
    cancel: tokio_util::sync::CancellationToken,
    selected: tokio::sync::watch::Receiver<Option<(ConnectionProfile, std::time::Instant)>>,
}
impl ConnectionSync {
    pub fn selected_route_after(&self, checked: std::time::Instant) -> Option<(ConnectionProfile, std::time::Instant)> {
        self.selected.borrow().as_ref().filter(|(_, at)| *at > checked).cloned()
    }
}
impl Drop for ConnectionSync {
    fn drop(&mut self) {self.cancel.cancel();}
}
pub(super) fn follow(mut profile:ConnectionProfile,mut selected:ConnectionProfile,token:String)->ConnectionSync {
    let cancel = tokio_util::sync::CancellationToken::new();
    let (routes, selection) = tokio::sync::watch::channel(None);
    let guard = ConnectionSync { cancel: cancel.clone(), selected: selection };
    if super::standalone_bridge() || selected.core_public_key.is_none(){return guard;}
    profile.core_id=selected.core_id.clone();profile.core_public_key=selected.core_public_key.clone();
    tokio::spawn(async move {
        let mut delay=std::time::Duration::ZERO;
        let mut first=true;
        loop {
            tokio::select!{_=tokio::time::sleep(delay)=>{},_=cancel.cancelled()=>break}
            let request=async {
                let after = {
                    let (core, key) = identity(&profile).context("缺少 Core 身份")?;
                    if first {None} else {store().lock().unwrap().get(core, key)?.map(|doc| doc.payload.revision)}
                };
                match profile.fetch_connection_info(&selected, &token, after).await {
                    Ok(next) => Ok(next),
                    Err(error) if error.downcast_ref::<reqwest::Error>().and_then(|e|e.status())==Some(reqwest::StatusCode::UNAUTHORIZED) => Err(error),
                    Err(_) => {
                        selected=profile.resolve().await?;
                        profile.fetch_connection_info(&selected, &token, None).await
                    }
                }
            };
            let result=tokio::select!{result=request=>result,_=cancel.cancelled()=>break};
            match result {
                Ok((next, watches))=>{
                    profile=next;first=false;
                    let resolve = async {
                        let started = std::time::Instant::now();
                        if let Ok(ready) = profile.resolve().await {
                            selected = ready;
                            routes.send_replace(Some((selected.clone(), started)));
                        }
                    };
                    tokio::select! { _ = resolve => {}, _ = cancel.cancelled() => break }
                    delay=if watches {std::time::Duration::ZERO} else {std::time::Duration::from_secs(300)};
                },
                Err(error)=>{
                    if error.downcast_ref::<reqwest::Error>().and_then(|e|e.status())==Some(reqwest::StatusCode::UNAUTHORIZED){break;}
                    delay=(delay.max(std::time::Duration::from_secs(15))*2).min(std::time::Duration::from_secs(120));
                }
            }
        }
    });
    guard
}
pub fn initialize(root: &Path) -> Result<()> {
    let root = root.join("connection-info");
    rmusic_remote::vault::private_directory(&root)?;
    let mut state = store().lock().unwrap();
    ensure!(state.root.as_ref().is_none_or(|previous| previous == &root), "连接清单目录已初始化");
    state.root = Some(root);
    Ok(())
}
fn identity(profile: &ConnectionProfile) -> Option<(&str, &str)> {
    Some((profile.core_id.as_deref()?, profile.core_public_key.as_deref()?))
}
fn cache_key(core: &str, key: &str) -> String {
    format!("{:x}", Sha256::digest(format!("{}:{core}{key}", core.len())))
}
fn validate(document: &Signed<Manifest>, core: &str, key: &str) -> Result<()> {
    document.verify(key)?;
    let manifest = &document.payload;
    ensure!(manifest.schema == 1 && manifest.core_id == core, "连接清单身份或版本不匹配");
    ensure!(manifest.endpoints.len() <= 16, "连接清单入口过多");
    for address in &manifest.endpoints {
        let url = url::Url::parse(address)?;
        ensure!(address.len() <= 2048 && matches!(url.scheme(), "http" | "https")
            && url.host().is_some() && url.username().is_empty() && url.password().is_none()
            && url.query().is_none() && url.fragment().is_none(), "连接清单入口无效");
    }
    Ok(())
}
impl Store {
    fn get(&mut self, core: &str, key: &str) -> Result<Option<Signed<Manifest>>> {
        let id = cache_key(core, key);
        if let Some(value) = self.entries.get(&id) { return Ok(Some(value.clone())); }
        let Some(root) = &self.root else { return Ok(None); };
        let path = root.join(format!("{id}.json"));
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(32769).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 32768, "连接清单缓存过大");
        let value: Signed<Manifest> = serde_json::from_slice(&bytes)?;
        validate(&value, core, key)?;
        self.remember(id, value.clone());
        Ok(Some(value))
    }
    fn remember(&mut self, id: String, value: Signed<Manifest>) {
        if self.entries.len() >= 32 && !self.entries.contains_key(&id) {
            if let Some(old) = self.entries.keys().next().cloned() { self.entries.remove(&old);self.checked.remove(&old);self.changed_at.remove(&old); }
        }
        self.entries.insert(id, value);
    }
    fn accept(&mut self, core: &str, key: &str, value: Signed<Manifest>) -> Result<Option<Signed<Manifest>>> {
        validate(&value, core, key)?;
        let old = self.get(core, key)?;
        if let Some(previous) = old.as_ref() {
            ensure!(value.payload.revision >= previous.payload.revision, "连接清单版本回退，已拒绝同步");
            ensure!(value.payload.revision != previous.payload.revision || value.payload == previous.payload,
                "相同版本的连接清单内容发生变化，已拒绝同步");
            if value.payload == previous.payload {
                self.checked.insert(cache_key(core,key), std::time::Instant::now());
                return Ok(old);
            }
        }
        if let Some(root) = &self.root {
            rmusic_remote::vault::atomic_private(&root.join(format!("{}.json", cache_key(core,key))), &serde_json::to_vec(&value)?)?;
        }
        self.changed_at.insert(cache_key(core,key), std::time::Instant::now());
        self.remember(cache_key(core, key), value);
        self.checked.insert(cache_key(core,key), std::time::Instant::now());
        Ok(old)
    }
}
pub(super) fn changed_since(profile: &ConnectionProfile, since: std::time::Instant) -> bool {
    identity(profile).is_some_and(|(core, key)| {
        store().lock().unwrap().changed_at.get(&cache_key(core, key)).is_some_and(|changed| *changed > since)
    })
}
pub(super) fn acknowledge(profile: &ConnectionProfile) {
    if let Some((core,key)) = identity(profile) {
        store().lock().unwrap().checked.insert(cache_key(core,key),std::time::Instant::now());
    }
}
pub(super) fn recent(profile: &ConnectionProfile) -> Result<Option<ConnectionProfile>> {
    let Some((core,key)) = identity(profile) else { return Ok(None); };
    let mut state=store().lock().unwrap();
    if !state.checked.get(&cache_key(core,key)).is_some_and(|time|time.elapsed()<std::time::Duration::from_secs(300)) {
        return Ok(None);
    }
    Ok(state.get(core,key)?.map(|value|merge(profile,&value.payload,None)))
}
pub(super) fn preferred_addresses(profile: &ConnectionProfile) -> Vec<String> {
    let Some((core, key)) = identity(profile) else { return vec![]; };
    store().lock().unwrap().get(core, key).ok().flatten()
        .map(|document| document.payload.direct_endpoints().cloned().collect()).unwrap_or_default()
}
fn merge(profile: &ConnectionProfile, manifest: &Manifest, previous: Option<&Manifest>) -> ConnectionProfile {
    let mut next = profile.clone();
    let previous = previous.map(|m| m.endpoints.as_slice()).unwrap_or_default();
    let mut manual: Vec<_> = next.alternatives.drain(..).filter(|a| match a.from_core { Some(managed) => !managed, None => !previous.contains(&a.address) }).collect();
    for address in &mut manual { address.from_core.get_or_insert(false); }
    for address in manifest.direct_endpoints() {
        if address != &next.address && !next.alternatives.iter().any(|a| &a.address == address) {
            next.alternatives.push(manual.iter().find(|a|&a.address==address).cloned().unwrap_or_else(||CoreAddress { from_core: Some(true), address: address.clone(), certificate_sha256: None, skip_certificate_verification: false }));
        }
    }
    for address in manual {
        if next.alternatives.len() < 8 && address.address != next.address && !next.alternatives.iter().any(|a| a.address == address.address) {
            next.alternatives.push(address);
        }
    }
    next.alternatives.truncate(8);
    next
}
pub(super) fn apply(profile: &ConnectionProfile) -> Result<ConnectionProfile> {
    let Some((core,key)) = identity(profile) else { return Ok(profile.clone()); };
    Ok(match store().lock().unwrap().get(core,key)? {
        Some(value) => merge(profile, &value.payload, None),
        None => profile.clone(),
    })
}
pub(super) fn accept(profile: &ConnectionProfile, selected: &ConnectionProfile, document: Signed<Manifest>) -> Result<ConnectionProfile> {
    let (core,key) = identity(selected).context("Core 未提供稳定身份")?;
    ensure!(profile.core_id.as_deref().is_none_or(|id|id==core)
        && profile.core_public_key.as_deref().is_none_or(|k|k==key), "Core 身份发生异常变化，已拒绝同步");
    let mut next = profile.clone(); next.core_id=Some(core.into()); next.core_public_key=Some(key.into());
    let old = store().lock().unwrap().accept(core,key,document.clone())?;
    Ok(merge(&next, &document.payload, old.as_ref().map(|v| &v.payload)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_connection_cache_survives_restart_rejects_replay_and_replaces_old_endpoints() {
        let dir=tempfile::tempdir().unwrap(); let id=rmusic_remote::identity::Identity::create(); let key=id.public_key();
        let manifest=|revision,url:&str|id.sign(Manifest{schema:1,core_id:"core".into(),revision,endpoints:vec![url.into()],relay:None}).unwrap();
        let mut state=Store{root:Some(dir.path().into()),..Default::default()};
        state.accept("core",&key,manifest(8,"https://old.example/music")).unwrap();
        let mut reopened=Store{root:Some(dir.path().into()),..Default::default()};
        assert!(reopened.accept("core",&key,manifest(7,"https://stale.example")).is_err());
        assert!(reopened.accept("core",&key,manifest(8,"https://changed.example")).is_err());
        let newer=manifest(9,"https://new.example:9443/prefix");
        let old=reopened.accept("core",&key,newer.clone()).unwrap().unwrap();
        let mut profile:ConnectionProfile=serde_json::from_value(serde_json::json!({"id":"qa","name":"QA","address":"https://core.example"})).unwrap();
        profile.alternatives=vec![CoreAddress{from_core: None, address:old.payload.endpoints[0].clone(),certificate_sha256:Some("old-pin".into()),skip_certificate_verification:true}];
        let merged=merge(&profile,&newer.payload,Some(&old.payload));
        assert_eq!(merged.alternatives.len(),1); assert_eq!(merged.alternatives[0].address,"https://new.example:9443/prefix");
        assert!(!merged.alternatives[0].skip_certificate_verification); assert!(merged.alternatives[0].certificate_sha256.is_none());
        assert!(Store{root:Some(dir.path().into()),..Default::default()}.get("core",&key).unwrap().is_some());
        assert!(validate(&newer,"other",&key).is_err());
        let mut changed=newer; changed.payload.endpoints.push("https://attacker.example".into()); assert!(validate(&changed,"core",&key).is_err());
    }
    #[test]
    fn updating_idle_cache_removes_learned_addresses_from_older_profiles() {
        let id = rmusic_remote::identity::Identity::create();
        let core = uuid::Uuid::new_v4().to_string();
        let mut seed: ConnectionProfile = serde_json::from_value(serde_json::json!({
            "id":"client","name":"Core","address":"http://192.168.50.10:7800",
            "alternatives":[{"address":"https://manual.example.org","from_core":false}]
        })).unwrap();
        seed.core_id=Some(core.clone());seed.core_public_key=Some(id.public_key());
        let doc=|revision,url:&str|id.sign(Manifest{schema:1,core_id:core.clone(),revision,endpoints:vec![url.into()],relay:None}).unwrap();
        let previously_saved=accept(&seed,&seed,doc(1,"https://old.example.org")).unwrap();
        assert_eq!(previously_saved.alternatives[0].from_core,Some(true));
        accept(&seed,&seed,doc(2,"https://new.example.org")).unwrap();
        let updated=apply(&previously_saved).unwrap();
        let urls:Vec<_>=updated.alternatives.iter().map(|a|a.address.as_str()).collect();
        assert_eq!(urls,["https://new.example.org","https://manual.example.org"]);
    }
    #[tokio::test]
    async fn signed_priority_overrides_saved_address_and_fails_over_to_the_next_endpoint() {
        use axum::{extract::{Path, WebSocketUpgrade}, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        let identity = rmusic_remote::identity::Identity::create();
        let core = uuid::Uuid::new_v4().to_string();
        let key = identity.public_key();
        let available = Arc::new(AtomicBool::new(true));
        let up = available.clone();
        let value = serde_json::json!({"core_id":core,"core_public_key":key,"capabilities":{"native_e2e_v1":true}});
        let tls = identity.tls(&core).unwrap();
        let inner = Router::new().route("/api/v1/status", get({let value=value.clone(); move || {let value=value.clone(); async move {Json(value)}}}));
        let app = Router::new().route("/{name}/api/v1/status", get(move |Path(name): Path<String>| {
            let response = Json(value.clone());
            let up = up.clone();
            async move {
                if name == "preferred" && !up.load(Ordering::Relaxed) {
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                response.into_response()
            }
        })).route("/{name}/api/v1/native/connect", get(move |ws: WebSocketUpgrade| {
            let tls = tls.clone();
            let inner = inner.clone();
            async move {
                ws.on_upgrade(move |socket| async move {
                    use rmusic_remote::{native, tunnel};
                    let cancel = tokio_util::sync::CancellationToken::new();
                    let (wire_io, server_io) = tokio::io::duplex(tunnel::WINDOW);
                    let wire = tunnel::bridge_server(socket, wire_io, cancel.clone());
                    let server = native::serve_core(server_io, inner, Some(tls), cancel.clone());
                    let _ = tunnel::serve_and_drain(wire, server, cancel).await;
                })
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener,app).await.unwrap(); });
        let backup = format!("{base}/backup");
        let preferred = format!("{base}/preferred");
        let seed: ConnectionProfile = serde_json::from_value(serde_json::json!({
            "id":"qa","name":"QA","address":backup,"core_id":core,"core_public_key":key
        })).unwrap();
        let sign = |revision, endpoints| identity.sign(Manifest {
            schema:1, core_id:core.clone(), revision, endpoints, relay:None
        }).unwrap();
        let first = sign(1, vec![preferred.clone(), backup.clone()]);
        accept(&seed,&seed,first).unwrap();
        assert_eq!(super::super::logical_address(&seed.resolve().await.unwrap().address), preferred);
        available.store(false,Ordering::Relaxed);
        assert_eq!(super::super::logical_address(&seed.resolve().await.unwrap().address), backup);
        available.store(true,Ordering::Relaxed);
        accept(&seed,&seed,sign(2, vec![backup.clone(), preferred])).unwrap();
        assert_eq!(super::super::logical_address(&seed.resolve().await.unwrap().address), backup);
        task.abort();
    }
    #[tokio::test]
    async fn standalone_bridge_rejects_remote_seed_or_fallback_before_any_connection() {
        let mut profile:ConnectionProfile=serde_json::from_value(serde_json::json!({
            "id":"bridge","name":"local","address":"http://127.0.0.1:1",
            "alternatives":[{"address":"https://foreign.example.org"}]
        })).unwrap();
        assert!(profile.resolve_lan_only().await.unwrap_err().to_string().contains("局域网"));
        profile.alternatives.clear(); profile.address="https://203.0.113.50:19443".into();
        assert!(profile.resolve_lan_only().await.unwrap_err().to_string().contains("网桥"));
    }
    #[tokio::test]
    async fn ending_a_client_session_cancels_a_pending_connection_info_read() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let (ready,received)=tokio::sync::oneshot::channel();
        let server=tokio::spawn(async move {
            let(mut socket,_)=listener.accept().await.unwrap();let mut bytes=[0;4096];let n=socket.read(&mut bytes).await.unwrap();
            assert!(String::from_utf8_lossy(&bytes[..n]).starts_with("GET /api/v1/connection-info "));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{").await.unwrap();ready.send(()).unwrap();
            assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(2),socket.read(&mut bytes)).await.unwrap().unwrap(),0);
        });
        let id=rmusic_remote::identity::Identity::create();
        let profile:ConnectionProfile=serde_json::from_value(serde_json::json!({"id":"qa","name":"QA","address":format!("http://{address}"),"core_id":uuid::Uuid::new_v4().to_string(),"core_public_key":id.public_key()})).unwrap();
        let following=profile.follow_connection_info(&profile,"test-only-session");
        tokio::time::timeout(std::time::Duration::from_secs(2),received).await.unwrap().unwrap();drop(following);server.await.unwrap();
    }
}
