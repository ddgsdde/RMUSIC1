use super::*;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(super) struct Control {
    pub enabled: AtomicBool,
    pub active: AtomicBool,
    pub cancel: tokio::sync::Notify,
    checked: Mutex<HashMap<String, Instant>>,
    pub error: Mutex<Option<String>>,
    path: PathBuf,
}
impl Control {
    pub fn open(root: &Path) -> Result<Self> {
        let path = root.join("upgrades.json");
        let enabled = std::fs::read(&path)
            .ok()
            .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
            .and_then(|v| v["enabled"].as_bool())
            .unwrap_or(true);
        Ok(Self {
            enabled: AtomicBool::new(enabled),
            active: AtomicBool::new(false),
            cancel: tokio::sync::Notify::new(),
            checked: Mutex::new(HashMap::new()),
            error: Mutex::new(None),
            path,
        })
    }
    pub fn configure(&self, enabled: bool) -> Result<()> {
        let temporary = self.path.with_extension("next");
        std::fs::write(&temporary, serde_json::to_vec(&json!({"enabled":enabled}))?)?;
        std::fs::rename(temporary, &self.path)?;
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            self.cancel.notify_waiters();
        }
        Ok(())
    }
}
struct Lease {
    cache: Arc<Mutex<CacheStore>>,
    ids: Vec<String>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut cache) = self.cache.lock() {
            for id in &self.ids {
                cache.release(id);
            }
        }
    }
}
struct Active<'a>(&'a AtomicBool);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

pub(super) fn verify(cache: &Arc<Mutex<CacheStore>>, id: &str) -> Result<CacheEntry> {
    let path = cache
        .lock()
        .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?
        .complete_file(id)?;
    let audio = crate::probe::probe(&path)?;
    let mut store = cache
        .lock()
        .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
    store.update_audio_format(
        id,
        &audio.format,
        audio.sample_rate,
        audio.channels,
        audio.bit_depth,
        audio.bitrate,
        audio.quality_rank,
    )?;
    Ok(store.manifest.entries[id].clone())
}
pub(super) async fn one(
    app: &tauri::AppHandle,
    profile_id: String,
    old_id: String,
    mut track: NativeTrack,
) -> CommandResult<CacheEntry> {
    let state = app.state::<DesktopState>();
    if !state.upgrades.enabled.load(Ordering::Relaxed) {
        return Err("自动升级已暂停".into());
    }
    let network = tokio::task::spawn_blocking(crate::network::policy)
        .await
        .map_err(err)?;
    if !network.unmetered_wifi {
        return Err(network.reason);
    }
    state
        .upgrades
        .active
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "已有升级正在进行")?;
    let _active = Active(&state.upgrades.active);
    let old = {
        let store = state.cache.lock().map_err(err)?;
        store
            .manifest
            .entries
            .get(&old_id)
            .cloned()
            .ok_or("缓存不存在")?
    };
    if !old.complete
        || !old.verified
        || old.manual_download
        || old.profile_id != profile_id
        || !cache_owned(&state,&old)
        || old.recording_id != track.recording_id
    {
        return Err("仅升级同一录音的已完整自动缓存".into());
    }
    let candidate = owned_entry(&profile_id, &track, 1, false,old.owner.as_ref());
    if candidate.quality_rank <= old.quality_rank || candidate.id == old_id {
        return Err("没有已确认的更高音质版本".into());
    }
    let profile = connected_profile(&state, &profile_id).await?;
    let token = token(&state, &profile_id)?;
    state.cache.lock().map_err(err)?.protect(&old_id);
    let mut lease = Lease {
        cache: state.cache.clone(),
        ids: vec![old_id.clone()],
    };
    let url = media_ticket(&profile, &token, &track.id)
        .await
        .map_err(err)?;
    let response = profile
        .http()
        .map_err(err)?
        .get(url)
        .send()
        .await
        .map_err(err)?
        .error_for_status()
        .map_err(err)?;
    let total = response.content_length().ok_or("升级音源未提供完整长度")?;
    response_metadata(&mut track, response.headers());
    let mut replacement = owned_entry(&profile_id, &track, total, false,old.owner.as_ref());
    replacement.pinned = old.pinned;
    let id = replacement.id.clone();
    state
        .cache
        .lock()
        .map_err(err)?
        .begin(replacement)
        .map_err(err)?;
    lease.ids.push(id.clone());
    let mut stream = response.bytes_stream();
    let mut offset = 0;
    let mut check = tokio::time::interval(Duration::from_secs(5));
    loop {
        if !state.upgrades.enabled.load(Ordering::Relaxed)||!cache_owned(&state,&old) {
            return Err("自动升级已停止，旧副本保留".into());
        }
        tokio::select! {
            _=state.upgrades.cancel.notified()=>return Err("自动升级已停止，旧副本保留".into()),
            _=check.tick()=>{if !tokio::task::spawn_blocking(crate::network::policy).await.map_err(err)?.unmetered_wifi {return Err("网络条件改变，升级已停止，旧副本保留".into())}},
            chunk=stream.next()=>{let Some(chunk)=chunk else {break};let bytes=chunk.map_err(err)?;let n=bytes.len() as u64;
                let cache=state.cache.clone();let key=id.clone();
                tokio::task::spawn_blocking(move||cache.lock().map_err(err)?.write_range(&key,offset,&bytes).map_err(err)).await.map_err(err)??;offset+=n;}
        }
    }
    if offset != total {
        return Err("升级不完整，旧副本保留".into());
    }
    let cache = state.cache.clone();
    let key = id.clone();
    let verified = tokio::task::spawn_blocking(move || verify(&cache, &key).map_err(err))
        .await
        .map_err(err)??;
    if verified.quality_rank <= old.quality_rank {
        return Err("真实编码未达到更高音质，旧副本保留".into());
    }
    drop(lease);
    state
        .cache
        .lock()
        .map_err(err)?
        .promote_upgrade(&old_id, &id, true)
        .map_err(err)?;
    let _ = app.emit(
        "native-cache-upgraded",
        json!({"previous_id":old_id,"entry":verified}),
    );
    Ok(verified)
}

pub(super) fn schedule(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let state = app.state::<DesktopState>();
            if !state.upgrades.enabled.load(Ordering::Relaxed)
                || state.upgrades.active.load(Ordering::Relaxed)
            {
                continue;
            }
            if !tokio::task::spawn_blocking(crate::network::policy)
                .await
                .is_ok_and(|p| p.unmetered_wifi)
            {
                continue;
            }
            let entries = match state.cache.lock() {
                Ok(cache) => cache
                    .manifest
                    .entries
                    .values()
                    .filter(|e| e.complete && !e.manual_download && cache_owned(&state,e))
                    .cloned()
                    .collect::<Vec<_>>(),
                Err(_) => continue,
            };
            for old in entries {
                if !state.upgrades.enabled.load(Ordering::Relaxed) {
                    break;
                }
                let recently = state
                    .upgrades
                    .checked
                    .lock()
                    .map(|m| {
                        m.get(&old.id)
                            .is_some_and(|time| time.elapsed() < Duration::from_secs(6 * 3600))
                    })
                    .unwrap_or(true);
                if recently {
                    continue;
                }
                let Ok(profile) = connected_profile(&state, &old.profile_id).await else {
                    continue;
                };
                let Ok(token) = token(&state, &old.profile_id) else {
                    continue;
                };
                if let Ok(mut checked) = state.upgrades.checked.lock() {
                    checked.insert(old.id.clone(), Instant::now());
                }
                let path = format!("api/v1/library/tracks/{}", old.track_id);
                let response = match profile.http() {
                    Ok(http) => {
                        http.get(match profile.endpoint(&path) {
                            Ok(url) => url,
                            Err(_) => continue,
                        })
                        .bearer_auth(token)
                        .send()
                        .await
                    }
                    Err(_) => continue,
                };
                let Ok(response) = response else { continue };
                if !response.status().is_success() {
                    continue;
                }
                let Ok(track) = response.json::<NativeTrack>().await else {
                    continue;
                };
                // Library metadata is only a candidate. The replacement's real codec/rate/bits is probed before promotion.
                if new_entry(&old.profile_id, &track, 1, false).quality_rank <= old.quality_rank {
                    continue;
                }
                if let Err(error) = one(&app, old.profile_id.clone(), old.id, track).await {
                    if let Ok(mut last) = state.upgrades.error.lock() {
                        *last = Some(error.clone());
                    }
                    let _ = app.emit("native-cache-upgrade-error", error);
                }
            }
        }
    });
}
