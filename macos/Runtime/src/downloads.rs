//! Original-byte cache and cancellable downloads. Reuses the desktop's durable range/SHA store.
use anyhow::{Context, Result};
use rmusic_bridge::{
    connection::ConnectionProfile,
    media_proxy::{MediaProxy, MediaResolver},
};
use rmusic_desktop::cache::{entry_id, CacheEntry, CacheLease, CacheStore};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{watch, Semaphore},
    task::JoinHandle,
};

struct Job {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
pub struct Downloads {
    verified_files: Arc<Mutex<HashMap<String,(u64,i64,i64,u64)>>>,
    hits: std::sync::atomic::AtomicU64,
    directory: std::path::PathBuf,
    clock:std::time::Instant,
    cache: Arc<Mutex<CacheStore>>,
    scope: String,
    token: String,
    profile: ConnectionProfile,
    jobs: Mutex<HashMap<String, Job>>,
    progress: Arc<Mutex<HashMap<String, Value>>>,
    slots: Arc<Semaphore>,
}
impl Downloads {
    pub fn open(
        root: &Path,
        core: &str,
        user: &str,
        profile: ConnectionProfile,
        token: String,
    ) -> Result<Arc<Self>> {
        let scope = format!(
            "{:x}",
            Sha256::digest(format!("{}:{core}{}:{user}", core.len(), user.len()))
        );
        let cache = CacheStore::open(root.join("media").join(&scope))?;
        Ok(Arc::new(Self {
            hits: Default::default(), verified_files:Default::default(),
            directory: root.join("media").join(&scope),
            clock:std::time::Instant::now(),
            cache: Arc::new(Mutex::new(cache)),
            scope,
            profile,
            token,
            jobs: Mutex::new(HashMap::new()),
            progress: Default::default(),
            slots: Arc::new(Semaphore::new(2)),
        }))
    }
    async fn track(&self, profile: &ConnectionProfile, id: &str) -> Result<Value> {
        anyhow::ensure!(
            !id.is_empty()
                && id.len() <= 256
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)),
            "invalid track identity"
        );
        Ok(profile
            .http()?
            .get(profile.endpoint(&format!("api/v1/library/tracks/{id}"))?)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(15))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?)
    }
    pub(crate) async fn source(&self, profile: &ConnectionProfile, id: &str) -> Result<String> {
        let response = profile
            .http()?
            .get(profile.endpoint(&format!("api/v1/library/tracks/{id}/ticket"))?)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(15))
            .send()
            .await?;
        let value = rmusic_bridge::sequence::media_json(response).await?;
        if let Some(path) = value["api_path"].as_str() {
            return Ok(profile.media_url(path)?.to_string());
        }
        let url = value["url"].as_str().context("缺少音源地址")?;
        let url = profile.base_url()?.join(url)?.to_string();
        profile.validate_media_url(&url)?;
        Ok(url)
    }
    fn entry(&self, track: &Value, total: u64, etag: Option<String>, manual: bool) -> CacheEntry {
        let format = track["format"].as_str().unwrap_or("wav").to_lowercase();
        // No ETag means a new entity per fetch; never combine unversioned partial files.
        let variant = etag
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        CacheEntry {
            id: entry_id(&self.scope, track["id"].as_str().unwrap_or(""), &variant),
            profile_id: self.scope.clone(),
            owner: None,
            track_id: track["id"].as_str().unwrap_or("").into(),
            title: track["title"].as_str().map(str::to_owned),
            artist: track["artist"].as_str().map(str::to_owned),
            recording_id: track["recording_id"].as_str().unwrap_or("").into(),
            format,
            total_bytes: total,
            ranges: vec![],
            complete: false,
            pinned: false,
            manual_download: manual,
            last_access: 0,
            quality_rank: 0,
            sha256: None,
            sample_rate: 0,
            channels: 0,
            bit_depth: 0,
            bitrate: 0,
            verified: false,
            source_etag: etag,document_uri:None,document_tree:None,
        }
    }
    pub fn complete_page(&self,offset:usize)->Result<Value> {
        let cache=self.cache.lock().map_err(|_|anyhow::anyhow!("缓存不可用"))?;
        let mut entries=cache.manifest.entries.values().filter(|e|e.complete&&e.sha256.is_some()).collect::<Vec<_>>();
        entries.sort_by(|a,b|a.title.cmp(&b.title).then(a.track_id.cmp(&b.track_id)));
        let mut seen=std::collections::HashSet::new();
        entries.retain(|entry|seen.insert(&entry.track_id));
        Ok(json!({"total":entries.len(),"items":entries.into_iter().skip(offset).take(100).collect::<Vec<_>>() }))
    }
    pub fn list(&self,offset:usize) -> Result<Value> {
        self.jobs.lock().unwrap().retain(|_, job| !job.task.is_finished());
        let cache = self
            .cache
            .lock()
            .map_err(|_| anyhow::anyhow!("cache lock"))?;
        let progress = self
            .progress
            .lock()
            .map_err(|_| anyhow::anyhow!("download lock"))?;
        let mut entries=cache.manifest.entries.values().collect::<Vec<_>>();
        entries.sort_by(|a,b|a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        let total=entries.len();
        let jobs:HashMap<_,_>=progress.iter().filter(|(_,value)|matches!(value["state"].as_str(),Some("queued"|"downloading"))||value["completed_tick"].as_u64().is_some_and(|at|self.clock.elapsed().as_secs().saturating_sub(at)<60)).take(128).collect();
        Ok(
            json!({"items":entries.into_iter().skip(offset).take(50).collect::<Vec<_>>(),"total":total,"offset":offset,"jobs":jobs,"used_bytes":cache.used_bytes(),"manual_bytes":cache.manual_bytes(),"limit_bytes":cache.manifest.limit_bytes,"cache_hits":self.hits.load(std::sync::atomic::Ordering::Relaxed)}),
        )
    }
    /// Select a complete original only inside this authenticated Core/account cache.
    pub async fn preferred_original(&self, track: &str) -> Result<Option<Arc<MediaProxy>>> {
        let track=track.to_owned();let cache=self.cache.clone();let root=self.directory.clone();let stamps=self.verified_files.clone();
        let local=tokio::task::spawn_blocking(move || -> Result<_> {
            use std::os::unix::fs::MetadataExt;
            let entry=cache.lock().unwrap().manifest.entries.values()
                .filter(|e|e.track_id==track&&e.complete&&e.sha256.is_some())
                .max_by_key(|e|(e.manual_download,e.quality_rank,e.last_access)).cloned();
            let Some(entry)=entry else{return Ok(None)};
            let lease=CacheLease::acquire(cache.clone(),entry.id.clone())?;
            let path=root.join(format!("{}.{}",entry.id,entry.format));let file=path.metadata()?;
            let stamp=(file.ino(),file.ctime(),file.ctime_nsec(),file.len());
            if stamps.lock().unwrap().get(&entry.id)!=Some(&stamp) {
                cache.lock().unwrap().complete_file(&entry.id)?;
                stamps.lock().unwrap().insert(entry.id,stamp);
            }
            Ok(Some((path,lease)))
        }).await??;
        match local {Some((path,lease))=>{self.hits.fetch_add(1,std::sync::atomic::Ordering::Relaxed);Ok(Some(Arc::new(MediaProxy::local_file(&path,Box::new(lease))?)))},None=>Ok(None)}
    }
    pub fn scope(&self)->&str{&self.scope}
    pub fn job_state(&self,id:&str)->Value{self.progress.lock().unwrap().get(id).cloned().unwrap_or(Value::Null)}
    pub fn export_original(&self,track:&str,directory:&Path,cancel:&std::sync::atomic::AtomicBool)->Result<std::path::PathBuf>{
        let directory=directory.canonicalize()?;
        anyhow::ensure!(!directory.starts_with(self.directory.canonicalize()?),"下载目标不能放进应用的内部缓存目录");
        let id=self.job_state(track)["cache_id"].as_str().context("原文件还未完成校验")?.to_owned();
        let entry=self.cache.lock().unwrap().manifest.entries.get(&id).cloned().context("原文件缓存已失效，请重试下载")?;
        anyhow::ensure!(entry.track_id==track&&entry.complete&&entry.verified,"缓存尚未完整校验");
        let _lease=CacheLease::acquire(self.cache.clone(),id.clone())?;
        let path=self.directory.join(format!("{id}.{}",entry.format));
        let name=crate::export_file::filename(entry.artist.as_deref().unwrap_or(""),entry.title.as_deref().unwrap_or(track),&id,&entry.format);
        crate::export_file::publish(&path,&directory,&name,entry.total_bytes,entry.sha256.as_deref().context("缺少完整校验摘要")?,cancel)
    }
    pub fn start(self: &Arc<Self>, id: String) -> Result<Value> {
        {let mut progress=self.progress.lock().unwrap();let tick=self.clock.elapsed().as_secs();
            if progress.len()>=4096{progress.retain(|_,v|v["completed_tick"].as_u64().is_none_or(|at|tick.saturating_sub(at)<60));}
            anyhow::ensure!(progress.len()<4096||progress.contains_key(&id),"下载记录较多，请稍后重试");
        }
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("download lock"))?;
        jobs.retain(|_,job|!job.task.is_finished());
        if jobs.get(&id).is_some_and(|j| !j.task.is_finished()) {
            return Ok(json!({"started":false}));
        }
        anyhow::ensure!(jobs.values().filter(|j|!j.task.is_finished()).count()<64,"下载任务已满，请等待当前任务完成");
        let (stop, mut stopped) = watch::channel(false);
        let owner = self.clone();
        let track = id.clone();
        self.progress
            .lock()
            .unwrap()
            .insert(id.clone(), json!({"state":"queued"}));
        let task = tokio::spawn(async move {
            let result = tokio::select! {biased; _=stopped.changed()=>Err(anyhow::anyhow!("下载已暂停，可继续")), result=async {
                let _permit=owner.slots.acquire().await?;
                owner.download(&track).await
            }=>result};
            owner.progress.lock().unwrap().insert(track,json!({"state":if result.is_ok(){"complete"}else{"paused"},"cache_id":result.as_ref().ok(),"completed_tick":owner.clock.elapsed().as_secs(),"error":result.err().map(|e|format!("{e:#}"))}));
        });
        jobs.insert(id, Job { stop, task });
        Ok(json!({"started":true}))
    }
    pub async fn cancel(&self, id: &str) -> Result<()> {
        let job = self.jobs.lock().unwrap().remove(id);
        if let Some(job) = job {
            job.stop.send_replace(true);
            job.task.await?;
        }
        Ok(())
    }
    pub async fn close(&self) {
        let jobs = std::mem::take(&mut *self.jobs.lock().unwrap());
        for job in jobs.values() {
            job.stop.send_replace(true);
        }
        for (_, job) in jobs {
            let _ = job.task.await;
        }
    }
    pub fn remove(&self, id: &str) -> Result<()> {
        self.cache.lock().unwrap().remove(id)
    }
    pub fn limit(&self, bytes: u64) -> Result<()> {
        anyhow::ensure!(bytes >= 64 * 1024 * 1024, "缓存上限至少 64 MiB");
        self.cache.lock().unwrap().set_limit(bytes)
    }
    async fn download(&self, id: &str) -> Result<String> {
        let selected = self.profile.resolve().await?;
        let track = self.track(&selected, id).await?;
        let source = self.source(&selected, id).await?;
        let client = selected.http()?;
        let head = client
            .head(&source)
            .header("Accept-Encoding", "identity")
            .timeout(Duration::from_secs(15))
            .send()
            .await?;
        let etag = strong_etag(head.headers());
        let total = entity_length(head.headers()).filter(|_| head.status().is_success());
        let existing = if let (Some(etag), Some(total)) = (&etag, total) {
            let cache = self.cache.lock().unwrap();
            cache
                .manifest
                .entries
                .values()
                .find(|e| {
                    e.track_id == id
                        && e.source_etag.as_ref() == Some(etag)
                        && e.total_bytes == total
                })
                .cloned()
        } else {
            None
        };
        if let Some(e) = &existing {
            if e.complete {
                self.cache.lock().unwrap().mark_manual(&e.id)?;
                self.verify(&e.id).await?;return Ok(e.id.clone());
            }
        }
        let offset = existing
            .as_ref()
            .and_then(|e| e.ranges.first())
            .filter(|r| r.start == 0)
            .map_or(0, |r| r.end);
        let mut request = client
            .get(&source)
            .timeout(Duration::from_secs(3600))
            .header("Accept-Encoding", "identity");
        if offset > 0 {
            request = request
                .header("Range", format!("bytes={offset}-"))
                .header("If-Range", etag.as_ref().unwrap());
        }
        let mut response = request.send().await?.error_for_status()?;
        let mut start = 0;
        let total = if offset > 0 && response.status() == 206 {
            let value = response
                .headers()
                .get("Content-Range")
                .and_then(|v| v.to_str().ok())
                .context("下载缺少范围校验")?;
            let expected = total.context("missing entity length")?;
            let expected_range = format!("bytes {offset}-{}/{expected}", expected - 1);
            anyhow::ensure!(
                value == expected_range && strong_etag(response.headers()) == etag,
                "音源在续传时改变，请重新下载"
            );
            start = offset;
            expected
        } else {
            anyhow::ensure!(response.status() == 200, "下载响应不支持原文件传输");
            response
                .content_length()
                .context("音源未提供完整长度，无法校验下载")?
        };
        let mut entry = self.entry(&track, total, strong_etag(response.headers()), true);
        if start > 0 {
            entry = existing.unwrap();
        }
        let key = entry.id.clone();
        {
            let mut cache = self.cache.lock().unwrap();
            cache.ensure_entry(entry)?;
            cache.mark_manual(&key)?;
        }
        let _lease = CacheLease::acquire(self.cache.clone(), key.clone())?;
        self.progress.lock().unwrap().insert(
            id.into(),
            json!({"state":"downloading","received":start,"total":total}),
        );
        while let Some(bytes) = response.chunk().await? {
            let cache = self.cache.clone();
            let key = key.clone();
            let size = bytes.len() as u64;
            let at = start;
            tokio::task::spawn_blocking(move || {
                cache.lock().unwrap().write_range(&key, at, &bytes)
            })
            .await??;
            start += size;
            self.progress.lock().unwrap().insert(
                id.into(),
                json!({"state":"downloading","received":start,"total":total}),
            );
        }
        anyhow::ensure!(start == total, "下载中断，已保留实际收到的数据");
        self.verify(&key).await?;Ok(key)
    }
    async fn verify(&self, id: &str) -> Result<()> {
        let cache = self.cache.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let mut cache = cache.lock().unwrap();
            let path = cache.complete_file(&id)?;
            let m = rmusic_desktop::probe::probe(&path)?;
            cache.update_audio_format(
                &id,
                &m.format,
                m.sample_rate,
                m.channels,
                m.bit_depth,
                m.bitrate,
                m.quality_rank,
            )
        })
        .await?
    }
}
fn entity_length(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    if let Some(range)=headers.get("Content-Range") {
        return range.to_str().ok().and_then(|v|v.strip_prefix("bytes ")).and_then(|v|v.split_once('/')).and_then(|(range,total)|{
            let (start,end)=range.split_once('-')?;let(start,end,total)=(start.parse::<u64>().ok()?,end.parse::<u64>().ok()?,total.parse::<u64>().ok()?);
            (start<=end&&end<total).then_some(total)
        });
    }
    headers
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
}
fn strong_etag(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.starts_with('"') && v.ends_with('"'))
        .map(str::to_owned)
}
impl Downloads {
    async fn cached_source(
        &self,
        profile: &ConnectionProfile,
        track: &str,
        source: &str,
    ) -> Result<Arc<MediaProxy>> {
        let candidate=self.cache.lock().unwrap().manifest.entries.values().any(|entry|entry.track_id==track&&entry.complete);
        if !candidate {
            // A cache miss must not make an extra upstream request. Create the observer from actual GET headers.
            let metadata=self.track(profile,track).await?;let template=self.entry(&metadata,0,None,false);let cache=self.cache.clone();
            let factory=Arc::new(move|headers:&reqwest::header::HeaderMap|->Option<rmusic_bridge::media_proxy::MediaObserver>{
                if headers.get("Content-Encoding").is_some_and(|v|v!="identity"){return None;}
                let etag=strong_etag(headers)?;let total=entity_length(headers)?;let mut entry=template.clone();
                entry.id=entry_id(&entry.profile_id,&entry.track_id,&etag);entry.source_etag=Some(etag);entry.total_bytes=total;
                let id=entry.id.clone();{
                    let mut store=cache.lock().unwrap();
                    // Older builds recorded a range response's segment length before any
                    // bytes could be claimed. Correct only empty, same-entity records.
                    if let Some(old)=store.manifest.entries.get_mut(&id).filter(|old|!old.complete&&old.ranges.is_empty()&&old.source_etag==entry.source_etag){old.total_bytes=entry.total_bytes;}
                    if store.ensure_entry(entry).is_err(){return None;}
                }
                let lease=CacheLease::acquire(cache.clone(),id.clone()).ok()?;let cache=cache.clone();
                Some(rmusic_bridge::media_proxy::buffered_cache_observer(Arc::new(move|offset,reported_total,bytes|{let _lease=&lease;if reported_total==Some(total){if let Err(error)=cache.lock().unwrap().write_range(&id,offset,bytes){eprintln!("Mac streaming cache: {error:#}");}}})))
            });
            return Ok(Arc::new(MediaProxy::streaming_cache(profile,source,factory).await?));
        }
        // Match the current source entity, so an old download cannot silently replace new audio.
        let head = profile
            .http()?
            .head(source)
            .header("Accept-Encoding", "identity")
            .timeout(Duration::from_secs(5))
            .send()
            .await;
        if let Ok(head) = head {
            if head.status().is_success() {
                if let Some(total) =
                    entity_length(head.headers()).filter(|_| strong_etag(head.headers()).is_some())
                {
                    let metadata = self.track(profile, track).await?;
                    let entry = self.entry(&metadata, total, strong_etag(head.headers()), false);
                    let id = entry.id.clone();
                    let cache = self.cache.clone();
                    let local = tokio::task::spawn_blocking(move || {
                        let mut store = cache.lock().unwrap();
                        store.ensure_entry(entry)?;
                        if store.manifest.entries[&id].complete {
                            let path = store.complete_file(&id)?;
                            drop(store);
                            let lease = CacheLease::acquire(cache, id.clone())?;
                            Ok::<_, anyhow::Error>(Some((path, lease)))
                        } else {
                            Ok(None)
                        }
                    })
                    .await??;
                    if let Some((path, lease)) = local {
                        self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        return Ok(Arc::new(MediaProxy::cached_file(profile,source,path.to_path_buf(),Box::new((lease,path))).await?));
                    }
                    // Persist only delivered byte ranges of a strongly identified entity.
                    if strong_etag(head.headers()).is_some() {
                        let entry =
                            self.entry(&metadata, total, strong_etag(head.headers()), false);
                        let id = entry.id;
                        let cache = self.cache.clone();
                        let lease = CacheLease::acquire(cache.clone(), id.clone())?;
                        let observer = Arc::new(move |offset, total: Option<u64>, bytes: &[u8]| {
                            let mut store = cache.lock().unwrap();
                            if store
                                .manifest
                                .entries
                                .get(&id)
                                .is_some_and(|entry| total == Some(entry.total_bytes))
                            {
                                if let Err(error) = store.write_range(&id, offset, bytes) {
                                    eprintln!("Mac streaming cache: {error:#}");
                                }
                            }
                        });
                        return Ok(Arc::new(
                            MediaProxy::start_entity(
                                profile,
                                source,
                                Some(rmusic_bridge::media_proxy::buffered_cache_observer(observer)),
                                strong_etag(head.headers()),
                            )
                            .await?
                            .with_lease(Box::new(lease)),
                        ));
                    }
                }
            }
        }
        Ok(Arc::new(MediaProxy::start(profile, source, None).await?))
    }
}

impl MediaResolver for Downloads {
    fn resolve<'a>(
        &'a self,
        profile: &'a ConnectionProfile,
        track: &'a str,
        source: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<Arc<MediaProxy>>> {
        Box::pin(async move {
            match self.cached_source(profile, track, source).await {
                Ok(proxy) => Ok(proxy),
                Err(error) => {
                    self.progress.lock().unwrap().insert(track.into(),json!({"state":"cache_error","error":format!("缓存暂不可用，继续网络播放：{error:#}")}));
                    Ok(Arc::new(MediaProxy::start(profile, source, None).await?))
                }
            }
        })
    }
}
#[cfg(test)]mod network_tests {
 use super::*;
 #[test]fn partial_response_length_does_not_shrink_the_original_cache_entity(){
  let mut headers=reqwest::header::HeaderMap::new();headers.insert("Content-Length","1024".parse().unwrap());headers.insert("Content-Range","bytes 2048-3071/10000".parse().unwrap());assert_eq!(entity_length(&headers),Some(10000));
  headers.insert("Content-Range","bytes 2048-10000/10000".parse().unwrap());assert_eq!(entity_length(&headers),None);
 }
}
