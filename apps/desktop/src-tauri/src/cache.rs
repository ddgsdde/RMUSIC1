use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

mod documents;
pub use documents::{CacheFile,DocumentStorage};
pub const DEFAULT_LIMIT: u64 = 5 * 1024 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}
pub(crate) struct MetadataTail {
    pub total:u64,
    pub etag:String,
    pub gaps:Vec<ByteRange>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub id: String,
    pub profile_id: String,
    #[serde(default)]
    pub owner: Option<crate::offline_stats::Owner>,
    pub track_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    pub recording_id: String,
    pub format: String,
    pub total_bytes: u64,
    pub ranges: Vec<ByteRange>,
    pub complete: bool,
    pub pinned: bool,
    pub manual_download: bool,
    pub last_access: u64,
    pub quality_rank: u32,
    pub sha256: Option<String>,
    #[serde(default)]
    pub sample_rate: u32,
    #[serde(default)]
    pub channels: u16,
    #[serde(default)]
    pub bit_depth: u16,
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub source_etag: Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub document_uri:Option<String>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub document_tree:Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheManifest {
    pub version: u32,
    pub limit_bytes: u64,
    pub entries: BTreeMap<String, CacheEntry>,
}
pub struct CacheStore {
    root: PathBuf,
    pub manifest: CacheManifest,
    active: BTreeMap<String, u32>,
    documents:Option<std::sync::Arc<dyn DocumentStorage>>,
    writers:BTreeMap<String,fs::File>,
    pub changes:std::sync::Arc<tokio::sync::Notify>,
}
pub struct CacheLease {
    cache: std::sync::Arc<std::sync::Mutex<CacheStore>>,
    id: String,
}
impl CacheLease {
    pub fn acquire(
        cache: std::sync::Arc<std::sync::Mutex<CacheStore>>,
        id: String,
    ) -> Result<Self> {
        cache
            .lock()
            .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?
            .protect(&id);
        Ok(Self { cache, id })
    }
}
impl Drop for CacheLease {
    fn drop(&mut self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.release(&self.id);
        }
    }
}

pub fn entry_id(profile: &str, track: &str, variant: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            format!(
                "{}:{}:{}:{}:{}",
                profile.len(),
                profile,
                track.len(),
                track,
                variant
            )
            .as_bytes()
        )
    )
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn valid_format(format: &str) -> bool {
    matches!(
        format,
        "flac" | "mp3" | "aac" | "m4a" | "ogg" | "opus" | "wav" | "aiff" | "dsf" | "dff" | "alac"
    )
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn checksum(path: &Path) -> Result<String> {
    let mut input = rmusic_audio::file::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
impl CacheStore {
    #[cfg(target_os = "android")]
    pub(crate) fn private_directory(&self) -> &Path {
        &self.root
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_owned();
        fs::create_dir_all(&root)?;
        let path = root.join("manifest.json");
        let mut manifest: CacheManifest = if path.exists() {
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            CacheManifest {
                version: 1,
                limit_bytes: DEFAULT_LIMIT,
                entries: BTreeMap::new(),
            }
        };
        if manifest.version != 1 {
            bail!("unsupported cache manifest version")
        }
        // Validate persisted names before using any path from disk.
        for (key, entry) in &manifest.entries {
            if !valid_id(&entry.id) || key != &entry.id || !valid_format(&entry.format) {
                bail!("invalid cache manifest filename")
            }
            let retired = root.join(format!("{}.retired", entry.id));
            let file = root.join(format!(
                "{}.{}",
                entry.id,
                if entry.complete {
                    entry.format.as_str()
                } else {
                    "part"
                }
            ));
            if entry.document_uri.is_none() && !file.exists() && retired.exists() {
                fs::rename(retired, file)?;
            }
        }
        // An interrupted write can leave data ahead of its manifest, never the reverse.
        manifest.entries.retain(|_, e| {
            e.document_uri.is_some() || root.join(format!(
                "{}.{}",
                e.id,
                if e.complete {
                    e.format.as_str()
                } else {
                    "part"
                }
            ))
            .exists()
        });
        Ok(Self {
            root,
            manifest,
            active: Default::default(),documents:None,writers:Default::default(),changes:Default::default(),
        })
    }
    fn path(&self, entry: &CacheEntry) -> PathBuf {
        self.root.join(format!(
            "{}.{}",
            entry.id,
            if entry.complete {
                entry.format.as_str()
            } else {
                "part"
            }
        ))
    }
    fn persist(&self) -> Result<()> {
        let path = self.root.join("manifest.next");
        let mut file = fs::File::create(&path)?;
        file.write_all(&serde_json::to_vec(&self.manifest)?)?;
        file.sync_all()?;
        fs::rename(path, self.root.join("manifest.json"))?;
        Ok(())
    }
    /// Includes logical temporary file lengths, not just complete entries. Manual downloads have a separate budget.
    pub fn bind_owners(&mut self,owners:&[(String,crate::offline_stats::Owner)])->Result<()> {
        let mut changed=false;
        for(id,owner)in owners{if let Some(entry)=self.manifest.entries.get_mut(id){if entry.owner.is_none(){entry.owner=Some(owner.clone());changed=true;}}}
        if changed{self.persist()?;}Ok(())
    }
    pub fn entry_bytes(&self,entry:&CacheEntry)->u64{if entry.document_uri.is_some(){if entry.complete{entry.total_bytes}else{entry.ranges.iter().map(|range|range.end-range.start).sum()}}else{self.path(entry).metadata().map(|m|m.len()).unwrap_or(0)}}
    pub fn used_bytes(&self) -> u64 {
        let downloads = self
            .manifest
            .entries
            .values()
            .filter(|e| e.manual_download)
            .map(|e| self.path(e))
            .collect::<std::collections::BTreeSet<_>>();
        fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let path = entry.path();
                let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
                !downloads.contains(&path)
                    && (ext == "part" || ext == "retired" || valid_format(ext))
            })
            .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .sum()
    }
    pub fn manual_bytes(&self) -> u64 {
        self.manifest
            .entries
            .values()
            .filter(|e| e.manual_download)
            .map(|e|self.entry_bytes(e))
            .sum()
    }
    pub fn set_limit(&mut self, limit: u64) -> Result<()> {
        self.make_room(0, limit, None)?;
        self.manifest.limit_bytes = limit;
        self.persist()
    }
    fn make_room(&mut self, additional: u64, limit: u64, exclude: Option<&str>) -> Result<()> {
        if additional > limit {
            bail!("cache quota is smaller than incoming data")
        }
        let mut candidates: Vec<_> = self
            .manifest
            .entries
            .values()
            .filter(|e| {
                !e.pinned
                    && !e.manual_download
                    && !self.active.contains_key(&e.id)
                    && Some(e.id.as_str()) != exclude
            })
            .map(|e| (e.last_access, e.id.clone()))
            .collect();
        candidates.sort();
        for (_, id) in candidates {
            if self.used_bytes().saturating_add(additional) <= limit {
                break;
            }
            self.remove(&id)?;
        }
        if self.used_bytes().saturating_add(additional) > limit {
            bail!("cache quota occupied by active or pinned entries")
        }
        Ok(())
    }
    pub fn begin(&mut self, entry: CacheEntry) -> Result<()> {
        let id = entry.id.clone();
        self.ensure_entry(entry)?;
        self.protect(&id);
        Ok(())
    }
    pub fn protect(&mut self, id: &str) {
        *self.active.entry(id.into()).or_default() += 1;
    }
    pub fn ensure_entry(&mut self, mut entry: CacheEntry) -> Result<()> {
        if !valid_id(&entry.id) || !valid_format(&entry.format) || entry.total_bytes == 0 {
            bail!("invalid cache entry")
        }
        if let Some(existing) = self.manifest.entries.get_mut(&entry.id) {
            if existing.total_bytes != entry.total_bytes
                || existing.recording_id != entry.recording_id
                || existing.source_etag != entry.source_etag
            {
                bail!("cache identity changed")
            }
            let mut changed = false;
            if entry.title.is_some() && entry.title != existing.title {
                existing.title = entry.title;
                changed = true;
            }
            if entry.artist.is_some() && entry.artist != existing.artist {
                existing.artist = entry.artist;
                changed = true;
            }
            return if changed { self.persist() } else { Ok(()) };
        }
        entry.complete = false;
        entry.ranges.clear();
        entry.last_access = now();
        let path = self.path(&entry);
        if entry.document_uri.is_none(){OpenOptions::new().create_new(true).write(true).open(path)?;}
        self.manifest.entries.insert(entry.id.clone(), entry);
        self.persist()
    }
    /// Find only missing trailer bytes; never turn skipped programme audio into a full download.
    pub(crate) fn dsd_metadata_tail(&self,id:&str)->Result<Option<MetadataTail>> {
        let entry=self.manifest.entries.get(id).context("unknown cache entry")?;
        if entry.complete||!matches!(entry.format.as_str(),"dsf"|"dff"){return Ok(None);}
        let Some(etag)=entry.source_etag.as_ref().filter(|tag|tag.starts_with('"')&&tag.ends_with('"')) else{return Ok(None);};
        let Some(prefix)=entry.ranges.first().filter(|range|range.start==0) else{return Ok(None);};
        let source=self.read_file(entry)?;
        let mut file=rmusic_audio::file::open(&source)?.take(prefix.end);
        let Ok(layout)=rmusic_audio::dsd::DsdLayout::read(&mut file) else{return Ok(None);};
        if layout.file_end!=entry.total_bytes||layout.data_end>=layout.file_end{return Ok(None);}
        let mut cursor=layout.data_end;let mut gaps=Vec::new();
        for range in &entry.ranges {
            if range.end<=cursor{continue;}
            let start=range.start.min(layout.file_end);
            if start>cursor{gaps.push(ByteRange{start:cursor,end:start});}
            cursor=cursor.max(range.end).min(layout.file_end);
            if cursor==layout.file_end{break;}
        }
        if cursor<layout.file_end{gaps.push(ByteRange{start:cursor,end:layout.file_end});}
        Ok(Some(MetadataTail{total:entry.total_bytes,etag:etag.clone(),gaps}))
    }
    /// Only byte ranges actually delivered by the media proxy are recorded. Gaps remain partial.
    pub fn write_range(&mut self, id: &str, start: u64, bytes: &[u8]) -> Result<()> {
        let entry = self
            .manifest
            .entries
            .get(id)
            .context("unknown cache entry")?
            .clone();
        if entry.complete {
            return Ok(());
        }
        let end = start
            .checked_add(bytes.len() as u64)
            .context("range overflow")?;
        if end > entry.total_bytes {
            bail!("range exceeds declared audio length")
        }
        if bytes.is_empty() {
            return Ok(());
        }
        let path = self.path(&entry);
        let current_len = if entry.document_uri.is_some(){self.entry_bytes(&entry)}else{fs::metadata(&path)?.len()};
        if !entry.manual_download {
            self.make_room(
                end.saturating_sub(current_len),
                self.manifest.limit_bytes,
                Some(id),
            )?;
        }
        let mut file=if let Some(uri)=&entry.document_uri {
            if !self.writers.contains_key(id){let writer=self.documents.as_ref().context("离线目录尚未连接")?.open(uri,true)?;self.writers.insert(id.into(),writer);}
            self.writers[id].try_clone()?
        }else{OpenOptions::new().write(true).open(&path)?};
        file.seek(SeekFrom::Start(start))?;
        file.write_all(bytes)?;
        // Persist range claims only after their source bytes are durable.
        file.sync_data()?;
        let mut ranges = entry.ranges.clone();
        ranges.push(ByteRange { start, end });
        ranges.sort_by_key(|r| r.start);
        let mut merged: Vec<ByteRange> = Vec::new();
        for range in ranges {
            if let Some(previous) = merged.last_mut().filter(|r| r.end >= range.start) {
                previous.end = previous.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        let complete =
            merged.len() == 1 && merged[0].start == 0 && merged[0].end == entry.total_bytes;
        let item = self.manifest.entries.get_mut(id).unwrap();
        item.ranges = merged;
        item.last_access = now();
        if complete {
            file.sync_all()?;
            drop(file);
            return self.finish_download(id);
        }
        self.persist()
    }
    /// Resume final verification when all bytes arrived before a process interruption.
    pub fn finish_download(&mut self,id:&str)->Result<()> {
        let entry=self.manifest.entries.get(id).context("unknown cache entry")?.clone();
        if entry.complete{return Ok(());}
        anyhow::ensure!(entry.ranges.len()==1&&entry.ranges[0].start==0&&entry.ranges[0].end==entry.total_bytes,"下载尚未完整接收");
        if let Some(writer)=self.writers.remove(id){writer.sync_all()?;}
        let readable=self.read_file(&entry)?;
        anyhow::ensure!(rmusic_audio::file::open(&readable)?.metadata()?.len()==entry.total_bytes,"离线文件长度与下载记录不符");
        let digest=checksum(&readable)?;
        if entry.document_uri.is_none(){fs::rename(self.path(&entry),self.private_file(&entry))?;}
        let item=self.manifest.entries.get_mut(id).unwrap();item.complete=true;item.sha256=Some(digest);
        self.persist()
    }
    pub fn release(&mut self, id: &str) {
        if let Some(count) = self.active.get_mut(id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.active.remove(id);
                self.writers.remove(id);
                self.changes.notify_one();
            }
        }
    }
    pub fn mark_manual(&mut self, id: &str) -> Result<()> {
        self.manifest
            .entries
            .get_mut(id)
            .context("unknown cache entry")?
            .manual_download = true;
        self.persist()
    }
    pub fn update_audio_format(
        &mut self,
        id: &str,
        format: &str,
        rate: u32,
        channels: u16,
        bits: u16,
        bitrate: u32,
        rank: u32,
    ) -> Result<()> {
        let valid_rate=if matches!(format,"dsf"|"dff") {
            bits==1&&channels<=6&&[2822400,3072000,5644800,6144000,11289600,12288000,22579200,24576000].contains(&rate)
        }else{(8000..=1536000).contains(&rate)};
        if !valid_format(format)
            || !valid_rate
            || channels == 0
            || channels > 32
        {
            bail!("invalid verified audio format")
        }
        let old = self
            .manifest
            .entries
            .get(id)
            .context("unknown cache entry")?
            .clone();
        if !old.complete {
            bail!("cannot verify incomplete cache")
        }
        if old.format != format && old.document_uri.is_none() {
            fs::rename(self.path(&old), self.root.join(format!("{id}.{format}")))?;
        }
        let entry = self.manifest.entries.get_mut(id).unwrap();
        entry.format = format.into();
        entry.sample_rate = rate;
        entry.channels = channels;
        entry.bit_depth = bits;
        entry.bitrate = bitrate;
        entry.quality_rank = rank;
        entry.verified = true;
        self.persist()
    }
    pub fn complete_file(&mut self, id: &str) -> Result<CacheFile> {
        let entry = self
            .manifest
            .entries
            .get(id)
            .context("unknown cache entry")?
            .clone();
        if !entry.complete {
            bail!("partial audio cannot be played as an offline song")
        }
        let path = self.read_file(&entry)?;
        if rmusic_audio::file::open(&path)?.metadata()?.len() != entry.total_bytes
            || entry.sha256.as_deref() != Some(checksum(&path)?.as_str())
        {
            if entry.document_uri.is_none(){fs::rename(&path,self.root.join(format!("{id}.part")))?;}else{fs::File::create(self.root.join(format!("{id}.part")))?;}
            let item = self.manifest.entries.get_mut(id).unwrap();
            item.complete = false;
            item.document_uri=None;item.document_tree=None;
            item.verified = false;
            item.ranges.clear();
            item.sha256 = None;
            self.persist()?;
            bail!("cached audio integrity check failed; download again")
        }
        let item = self
            .manifest
            .entries
            .get_mut(id)
            .context("unknown cache entry")?;
        if !item.complete {
            bail!("partial audio cannot be played as an offline song")
        }
        item.last_access = now();
        self.persist()?;
        path.rewind()?;Ok(path)
    }
    pub fn remove(&mut self, id: &str) -> Result<()> {
        if self.active.contains_key(id) {
            bail!("entry is playing or downloading")
        }
        if let Some(entry) = self.manifest.entries.get(id) {
            if let Some(uri)=&entry.document_uri {self.documents.as_ref().context("离线目录尚未连接")?.remove(uri)?;}
            else {let path=self.path(entry);if path.exists(){fs::remove_file(path)?;}}
        }
        self.manifest.entries.remove(id);
        self.persist()
    }
    /// Promote a fully downloaded higher-quality variant, preserving the old file until verification succeeds.
    pub fn promote_upgrade(
        &mut self,
        previous_id: &str,
        replacement_id: &str,
        unmetered_wifi: bool,
    ) -> Result<()> {
        if !unmetered_wifi {
            bail!("automatic upgrade requires confirmed unmetered Wi-Fi")
        }
        let old = self
            .manifest
            .entries
            .get(previous_id)
            .context("missing previous version")?
            .clone();
        let new = self
            .manifest
            .entries
            .get(replacement_id)
            .context("missing replacement")?
            .clone();
        if !old.complete
            || !new.complete
            || old.recording_id != new.recording_id
            || old.profile_id != new.profile_id
            || new.quality_rank <= old.quality_rank
            || new.sha256.is_none()
        {
            bail!("replacement is incomplete, mismatched or not higher quality")
        }
        if self.active.contains_key(previous_id) || self.active.contains_key(replacement_id) {
            bail!("cannot upgrade active audio")
        }
        // Replacement remains a distinct source variant; do not falsify its original encoding or checksum.
        let retired = self.root.join(format!("{previous_id}.retired"));
        fs::rename(self.path(&old), &retired)?;
        self.manifest.entries.remove(previous_id);
        self.manifest
            .entries
            .get_mut(replacement_id)
            .unwrap()
            .pinned = old.pinned;
        if let Err(error) = self.persist() {
            self.manifest
                .entries
                .insert(previous_id.into(), old.clone());
            self.manifest.entries.insert(replacement_id.into(), new);
            let _ = fs::rename(&retired, self.path(&old));
            return Err(error);
        }
        let _ = fs::remove_file(retired);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> CacheStore {
        CacheStore::open(
            std::env::temp_dir().join(format!("rmusic-cache-test-{}", uuid::Uuid::new_v4())),
        )
        .unwrap()
    }
    fn entry(id: &str, total: u64) -> CacheEntry {
        CacheEntry {
            title: None,
            artist: None,
            id: id.into(),
            profile_id: "core".into(),
            owner: None,
            track_id: "song".into(),
            recording_id: "recording".into(),
            format: "flac".into(),
            total_bytes: total,
            ranges: vec![],
            complete: false,
            pinned: false,
            manual_download: false,
            last_access: 0,
            quality_rank: 1,
            sha256: None,
            sample_rate: 48000,
            channels: 2,
            bit_depth: 16,
            bitrate: 1536000,
            verified: false,
            source_etag: None,document_uri:None,document_tree:None,
        }
    }
    #[test]
    fn partial_ranges_do_not_become_offline_and_resume_merges() {
        let mut store = store();
        store.begin(entry("one", 10)).unwrap();
        store.write_range("one", 5, b"56789").unwrap();
        assert!(store.complete_file("one").is_err());
        store.write_range("one", 0, b"012").unwrap();
        assert!(!store.manifest.entries["one"].complete);
        store.write_range("one", 2, b"2345").unwrap();
        let path = store.complete_file("one").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"0123456789");
        assert!(store.manifest.entries["one"].sha256.is_some());
        fs::remove_dir_all(store.root).unwrap();
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn dsd_natural_eof_keeps_the_original_file_complete_with_large_tail_metadata() {
        use std::{io::Write,sync::{Arc,Mutex},time::Duration};
        use axum::{Router,routing::get,http::HeaderMap,body::Body,response::Response};
        use rmusic_bridge::{connection::ConnectionProfile,media_proxy::{MediaProxy,MediaObserver}};
        let syncsafe=|value:u32|[(value>>21) as u8&127,(value>>14) as u8&127,(value>>7) as u8&127,value as u8&127];
        let metadata_bytes=3*1024*1024_u32;
        let mut data=b"DSD ".to_vec();data.extend(28_u64.to_le_bytes());
        data.extend((92+8192+20+metadata_bytes as u64).to_le_bytes());data.extend(8284_u64.to_le_bytes());
        data.extend(b"fmt ");data.extend(52_u64.to_le_bytes());
        for value in [1_u32,0,2,2,2822400,1]{data.extend(value.to_le_bytes());}
        data.extend(32768_u64.to_le_bytes());data.extend(4096_u32.to_le_bytes());data.extend(0_u32.to_le_bytes());
        data.extend(b"data");data.extend(8204_u64.to_le_bytes());data.extend(vec![0x96;8192]);
        data.extend(b"ID3\x04\0\0");data.extend(syncsafe(metadata_bytes+10));
        data.extend(b"XZZZ");data.extend(syncsafe(metadata_bytes));data.extend([0,0]);data.extend(vec![1;metadata_bytes as usize]);
        let mut local=tempfile::Builder::new().suffix(".dsf").tempfile().unwrap();local.write_all(&data).unwrap();local.flush().unwrap();assert!(rmusic_audio::probe::probe(local.path()).unwrap().is_dsd());
        let expected=data.clone();let data=Arc::new(data);let total=data.len() as u64;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let served=Arc::new(Mutex::new(Vec::new()));
        let router={let served=served.clone();let data=data.clone();Router::new().route("/source.dsf",get(move|headers:HeaderMap|{
            let served=served.clone();let data=data.clone();async move {
                let range=headers.get("range").and_then(|value|value.to_str().ok()).and_then(|value|value.strip_prefix("bytes="));
                let offset=range.and_then(|range|range.split('-').next()?.parse::<usize>().ok()).unwrap_or(0);
                let end=range.and_then(|range|range.split_once('-')?.1.parse::<usize>().ok()).unwrap_or(data.len()-1).min(data.len()-1);
                served.lock().unwrap().push(offset);
                if offset>=data.len(){return Response::builder().status(416).header("content-range",format!("bytes */{}",data.len())).body(Body::empty()).unwrap();}
                let length=end+1-offset;
                let stream=futures_util::stream::unfold((data,offset,end),|(data,offset,end)|async move {
                    if offset>end{return None;}
                    tokio::time::sleep(Duration::from_millis(2)).await;
                    let next=(offset+4096).min(end+1);let bytes=data[offset..next].to_vec();
                    Some((Ok::<_,std::io::Error>(bytes),(data,next,end)))
                });
                Response::builder().status(206).header("content-type","audio/x-dsf").header("content-length",length)
                    .header("content-range",format!("bytes {offset}-{end}/{total}")).header("accept-ranges","bytes").header("etag","\"dsd-tail-v1\"").body(Body::from_stream(stream)).unwrap()
            }
        }))};
        let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        let cache=Arc::new(Mutex::new(store()));let mut cached=entry("dsd-tail",total);cached.format="dsf".into();cached.sample_rate=2822400;cached.bit_depth=1;cached.source_etag=Some("\"dsd-tail-v1\"".into());cache.lock().unwrap().begin(cached).unwrap();
        let observer:MediaObserver={let cache=cache.clone();Arc::new(move|offset,actual,bytes|{assert_eq!(actual,Some(total));cache.lock().unwrap().write_range("dsd-tail",offset,bytes).unwrap();})};
        let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"QA".into(),address:format!("http://{address}"),core_id:None,alternatives:vec![],skip_certificate_verification: false, certificate_sha256: None};
        let proxy=MediaProxy::start(&profile,&format!("http://{address}/source.dsf"),Some(observer)).await.unwrap();
        let metadata=rmusic_audio::probe::probe_async(std::path::Path::new(&proxy.url)).await.unwrap();assert!(metadata.is_dsd());
        let url=proxy.url.clone();
        let frames=tokio::task::spawn_blocking(move||{
            let mut reader=rmusic_audio::dsd::open(&url,0.,|_|Ok(())).unwrap();let mut encoder=rmusic_audio::dsd::DopEncoder::new(2).unwrap();let mut bytes=[0;129*8];let mut frames=0;
            loop{let n=reader.read_dop(&mut encoder,&mut bytes,rmusic_audio::dsd::DopContainer::LeftAligned32,false).unwrap();if n==0{break;}frames+=n;}
            frames
        }).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let snapshot=cache.lock().unwrap().manifest.entries["dsd-tail"].clone();
        drop(proxy);
        assert_eq!(frames,2048);assert!(!snapshot.complete,"fixture must leave unread metadata: {:?}",snapshot.ranges);
        let source_url=format!("http://{address}/source.dsf");
        assert!(crate::cache_tail::complete_metadata(cache.clone(),"dsd-tail".into(),profile.clone(),source_url.clone()).await.unwrap());
        let path=cache.lock().unwrap().complete_file("dsd-tail").unwrap();assert_eq!(fs::read(&path).unwrap(),expected);
        let audio=rmusic_audio::probe::probe(&path).unwrap();assert_eq!(audio.sample_rate,2822400);assert_eq!(audio.bit_depth,1);
        {
            let mut store=cache.lock().unwrap();
            assert!(store.update_audio_format("dsd-tail","flac",audio.sample_rate,2,24,0,0).is_err());
            assert!(store.update_audio_format("dsd-tail","dsf",audio.sample_rate,2,24,0,0).is_err());
            store.update_audio_format("dsd-tail",&audio.format,audio.sample_rate,audio.channels,audio.bit_depth,audio.bitrate,audio.quality_rank).unwrap();
            assert!(store.manifest.entries["dsd-tail"].verified);
        }
        // A concurrent proxy completion is also eligible for verification, without another request.
        assert!(crate::cache_tail::complete_metadata(cache.clone(),"dsd-tail".into(),profile.clone(),source_url.clone()).await.unwrap());

        let partial=|id:&str,etag:&str|{
            let mut cached=entry(id,total);cached.format="dsf".into();cached.source_etag=Some(etag.into());
            let mut store=cache.lock().unwrap();store.begin(cached).unwrap();store.write_range(id,0,&expected[..92]).unwrap();
        };
        partial("skipped-audio","\"dsd-tail-v1\"");
        let count=served.lock().unwrap().len();
        assert!(!crate::cache_tail::complete_metadata(cache.clone(),"skipped-audio".into(),profile.clone(),source_url.clone()).await.unwrap());
        assert_eq!(served.lock().unwrap()[count..],[8284]);
        assert_eq!(cache.lock().unwrap().manifest.entries["skipped-audio"].ranges,vec![ByteRange{start:0,end:92},ByteRange{start:8284,end:total}]);
        assert!(cache.lock().unwrap().complete_file("skipped-audio").is_err());

        partial("changed-file","\"another-version\"");
        assert!(crate::cache_tail::complete_metadata(cache.clone(),"changed-file".into(),profile.clone(),source_url.clone()).await.is_err());
        assert_eq!(cache.lock().unwrap().manifest.entries["changed-file"].ranges,vec![ByteRange{start:0,end:92}]);
        partial("weak-tag","W/\"dsd-tail-v1\"");
        assert!(!crate::cache_tail::complete_metadata(cache.clone(),"weak-tag".into(),profile.clone(),source_url.clone()).await.unwrap());
        assert_eq!(cache.lock().unwrap().manifest.entries["weak-tag"].ranges,vec![ByteRange{start:0,end:92}]);

        partial("cancelled-tail","\"dsd-tail-v1\"");
        let worker=tokio::spawn(crate::cache_tail::complete_metadata(cache.clone(),"cancelled-tail".into(),profile,source_url));
        tokio::time::timeout(Duration::from_secs(3),async {
            while cache.lock().unwrap().manifest.entries["cancelled-tail"].ranges.len()==1{tokio::time::sleep(Duration::from_millis(2)).await;}
        }).await.unwrap();
        worker.abort();assert!(worker.await.unwrap_err().is_cancelled());
        let cancelled=cache.lock().unwrap().manifest.entries["cancelled-tail"].clone();assert!(!cancelled.complete);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(cache.lock().unwrap().manifest.entries["cancelled-tail"].ranges,cancelled.ranges);
        server.abort();
    }
    #[test]
    fn quota_counts_temporary_files_and_protects_active_and_manual() {
        let mut store = store();
        store.set_limit(10).unwrap();
        store.begin(entry("one", 10)).unwrap();
        store.write_range("one", 0, b"123456").unwrap();
        assert_eq!(store.used_bytes(), 6);
        store.begin(entry("two", 10)).unwrap();
        assert!(store.write_range("two", 0, b"12345").is_err());
        store.release("one");
        store.write_range("two", 0, b"12345").unwrap();
        assert!(!store.manifest.entries.contains_key("one"));
        let mut manual = entry("manual", 20);
        manual.manual_download = true;
        store.begin(manual).unwrap();
        store.write_range("manual", 0, &[1; 20]).unwrap();
        assert_eq!(store.used_bytes(), 5);
        assert_eq!(store.manual_bytes(), 20);
        fs::remove_dir_all(store.root).unwrap();
    }
    #[test]
    fn failed_upgrade_keeps_complete_old_copy() {
        let mut store = store();
        store.begin(entry("old", 3)).unwrap();
        store.write_range("old", 0, b"old").unwrap();
        store.release("old");
        let mut better = entry("new", 4);
        better.quality_rank = 2;
        store.begin(better).unwrap();
        store.write_range("new", 0, b"ne").unwrap();
        store.release("new");
        assert!(store.promote_upgrade("old", "new", true).is_err());
        assert_eq!(
            fs::read(store.complete_file("old").unwrap()).unwrap(),
            b"old"
        );
        fs::remove_dir_all(store.root).unwrap();
    }
    #[test]
    fn path_traversal_is_rejected() {
        let mut store = store();
        assert!(store.begin(entry("../../file", 1)).is_err());
        fs::remove_dir_all(store.root).unwrap();
    }
    #[test]
    fn playing_and_downloading_the_same_file_have_independent_leases() {
        let mut cache = store();
        cache.begin(entry("one", 3)).unwrap();
        cache.begin(entry("one", 3)).unwrap();
        cache.write_range("one", 0, b"123").unwrap();
        cache.release("one");
        assert!(cache.remove("one").is_err());
        cache.release("one");
        cache.remove("one").unwrap();
        fs::remove_dir_all(cache.root).unwrap();
    }
    #[test]
    fn corrupted_complete_cache_is_demoted_and_can_be_repaired() {
        let mut cache = store();
        cache.begin(entry("one", 3)).unwrap();
        cache.write_range("one", 0, b"123").unwrap();
        cache.release("one");
        let path = cache.complete_file("one").unwrap();
        fs::write(path, b"456").unwrap();
        assert!(cache.complete_file("one").is_err());
        assert!(!cache.manifest.entries["one"].complete);
        cache.write_range("one", 0, b"123").unwrap();
        assert_eq!(
            fs::read(cache.complete_file("one").unwrap()).unwrap(),
            b"123"
        );
        fs::remove_dir_all(cache.root).unwrap();
    }
}
