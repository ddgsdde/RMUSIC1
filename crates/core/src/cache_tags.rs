//! Prepare tagged audio without transcoding, then publish it as the canonical complete cache.
use crate::{
    error::{ApiError, ApiResult},
    media,
    models::Track,
    state::AppState,
};
use anyhow::{bail, Context};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::Response,
};
use futures_util::StreamExt;
use lofty::{
    config::WriteOptions,
    file::{FileType, TaggedFileExt},
    picture::{MimeType, Picture, PictureType},
    probe::Probe,
    tag::{Accessor, ItemKey, Tag, TagExt, TagType},
};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path as FsPath, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, SystemTime},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    process::Command,
    sync::Mutex,
};

const MAX_COVER: u64 = 10 * 1024 * 1024;
const MAX_LYRICS: usize = 2 * 1024 * 1024;
const MAX_EMBED_FILE: u64 = 256 * 1024 * 1024;

#[derive(Debug)]
struct Deferred;
impl std::fmt::Display for Deferred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("播放优先，便携副本稍后生成")
    }
}
impl std::error::Error for Deferred {}
struct Active {
    state: AppState,
    id: String,
}
impl Drop for Active {
    fn drop(&mut self) {
        let state = self.state.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            let mut active = state.cache_active.lock().await;
            if let Some(n) = active.get_mut(&id) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    active.remove(&id);
                }
            }
        });
    }
}
struct CancelWatcher(tokio::task::JoinHandle<()>);
impl Drop for CancelWatcher {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[derive(Clone)]
struct Entry {
    id: String,
    track_id: String,
    source: PathBuf,
    size: u64,
    format: String,
}
struct Metadata {
    track: Track,
    lyrics: Option<String>,
    cover: Option<Vec<u8>>,
}
struct CopyResult {
    audio: PathBuf,
    size: u64,
    embedded: bool,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn filename(track: &Track, format: &str) -> String {
    let raw = format!("{} - {}", track.artist, track.title);
    let mut clean = String::new();
    for ch in raw.chars() {
        let ch = if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
            '_'
        } else {
            ch
        };
        if clean.len() + ch.len_utf8() > 180 {
            break;
        }
        clean.push(ch);
    }
    let stem = clean.trim_matches(['.', ' ']);
    let stem = if stem.is_empty() { "R-Music" } else { stem };
    format!("{stem}.{format}")
}
fn format_safe(format: &str) -> bool {
    matches!(
        format,
        "flac" | "mp3" | "m4a" | "aac" | "wav" | "aiff" | "ogg" | "opus" | "dsf" | "dff" | "alac"
    )
}
fn detected_format(source: &FsPath, declared: &str) -> String {
    let kind = Probe::open(source)
        .ok()
        .and_then(|probe| probe.guess_file_type().ok())
        .and_then(|probe| probe.file_type());
    match kind {
        Some(FileType::Flac) => "flac",
        Some(FileType::Mpeg) => "mp3",
        Some(FileType::Mp4) => "m4a",
        Some(FileType::Aac) => "aac",
        Some(FileType::Wav) => "wav",
        Some(FileType::Aiff) => "aiff",
        Some(FileType::Opus) => "opus",
        Some(FileType::Vorbis) => "ogg",
        _ => declared,
    }
    .to_owned()
}
fn stamp(path: &FsPath) -> anyhow::Result<(u64, SystemTime)> {
    let m = std::fs::metadata(path)?;
    Ok((m.len(), m.modified()?))
}
fn cancelled(cancel: &AtomicBool) -> anyhow::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(Deferred.into())
    } else {
        Ok(())
    }
}
fn copy_source(
    source: &FsPath,
    destination: &FsPath,
    cancel: &AtomicBool,
) -> anyhow::Result<String> {
    let mut input = std::fs::File::open(source)?;
    let mut output = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(destination)?;
    let mut bytes = [0u8; 64 * 1024];
    let mut hash = Sha256::new();
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        output.write_all(&bytes[..n])?;
        hash.update(&bytes[..n]);
    }
    output.sync_all()?;
    Ok(format!("{:x}", hash.finalize()))
}
fn cover_format(bytes: &[u8]) -> Option<(&'static str, MimeType)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", MimeType::Png))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("jpg", MimeType::Jpeg))
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(("webp", MimeType::Unknown("image/webp".into())))
    } else {
        None
    }
}
fn tag(file:&mut lofty::file::TaggedFile, metadata:&Metadata)->anyhow::Result<Tag> {
    let tag_type = match file.file_type() {
        FileType::Flac => TagType::VorbisComments,
        FileType::Mpeg => TagType::Id3v2,
        FileType::Mp4 => TagType::Mp4Ilst,
        _ => bail!("此格式保留原字节并提供旁置元数据"),
    };
    if file.tag(tag_type).is_none() {
        file.insert_tag(Tag::new(tag_type));
    }
    let tag = file
        .tag_mut(tag_type)
        .context("missing writable audio tag")?;
    tag.set_title(metadata.track.title.clone());
    tag.set_artist(metadata.track.artist.clone());
    tag.set_album(metadata.track.album.clone());
    tag.insert_text(ItemKey::AlbumArtist, metadata.track.album_artist.clone());
    if metadata.track.track_number > 0 {
        tag.set_track(metadata.track.track_number)
    }
    if metadata.track.disc_number > 0 {
        tag.set_disk(metadata.track.disc_number)
    }
    if let Some(lyrics) = &metadata.lyrics {
        tag.insert_text(ItemKey::Lyrics, lyrics.clone());
    }
    if let Some(cover) = &metadata.cover {
        if let Some((_, mime)) = cover_format(cover) {
            tag.remove_picture_type(PictureType::CoverFront);
            tag.push_picture(Picture::new_unchecked(
                PictureType::CoverFront,
                Some(mime),
                None,
                cover.clone(),
            ));
        }
    }
    Ok(tag.clone())
}
fn write_tags(audio:&FsPath,metadata:&Metadata)->anyhow::Result<()> {
    let mut file=Probe::open(audio)?.guess_file_type()?.read()?;
    tag(&mut file,metadata)?.save_to_path(audio,WriteOptions::default())?;Ok(())
}
fn directory_size(directory: &FsPath) -> anyhow::Result<u64> {
    let mut size = 0u64;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let m = entry.metadata()?;
        if !m.is_file() {
            bail!("unexpected directory inside portable song")
        };
        size = size.saturating_add(m.len());
    }
    Ok(size)
}
fn remove_owned_directory(base: &FsPath, directory: &FsPath) -> anyhow::Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    let base = base.canonicalize()?;
    let target = directory.canonicalize()?;
    if target.parent() != Some(base.as_path())
        || !target.starts_with(&base)
        || target.file_name() != directory.file_name()
    {
        bail!("portable cleanup target escaped its directory")
    }
    std::fs::remove_dir_all(target)?;
    Ok(())
}
fn saved_export_root(path: &str, id: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(path);
    let parent = path.parent().context("missing portable parent")?;
    let name = parent
        .file_name()
        .and_then(|v| v.to_str())
        .context("invalid portable directory")?;
    if name != id && name != format!("{id}.staging") {
        bail!("portable cleanup directory does not match cache ID")
    }
    let base = parent.parent().context("missing portable root")?;
    if !base.is_absolute() {
        bail!("invalid portable root")
    }
    Ok(base.to_owned())
}
fn prepare_copy(
    entry: &Entry,
    root: &FsPath,
    metadata: &Metadata,
    budget: u64,
    cancel: &AtomicBool,
) -> anyhow::Result<(CopyResult, String, Option<String>)> {
    if !valid_id(&entry.id) || !format_safe(&entry.format) {
        bail!("invalid portable cache filename")
    }
    let before = stamp(&entry.source)?;
    if before.0 != entry.size || before.0 > budget {
        bail!("source size differs from complete cache")
    }
    std::fs::create_dir_all(root)?;
    let staging = root.join(format!("{}.staging", entry.id));
    remove_owned_directory(root, &staging)?;
    std::fs::create_dir(&staging)?;
    let actual_format = detected_format(&entry.source, &entry.format);
    let audio = staging.join(filename(&metadata.track, &actual_format));
    let (source_sha256,tag_error)=if before.0<=4*1024*1024 {
        let original=std::fs::read(&entry.source)?;let hash=format!("{:x}",Sha256::digest(&original));
        let mut memory=std::io::Cursor::new(original.clone());
        let result=(||->anyhow::Result<()>{let mut file=Probe::new(std::io::Cursor::new(&original)).guess_file_type()?.read()?;let tag=tag(&mut file,metadata)?;tag.save_to(&mut memory,WriteOptions::default())?;Ok(())})();
        let error=result.err().map(|_|"该格式使用旁置元数据".to_owned());
        let bytes=if error.is_none(){memory.into_inner()}else{original};
        if bytes.len() as u64>budget{bail!("tagged cache exceeds reserved capacity");}
        std::fs::write(&audio,bytes)?;(hash,error)
    }else{
        let hash=copy_source(&entry.source,&audio,cancel)?;
        let error=if before.0>MAX_EMBED_FILE {Some("大文件使用旁置元数据以限制标签整理内存".into())}else{write_tags(&audio,metadata).err().map(|_|"该文件不能安全内嵌标签，使用旁置元数据".into())};
        if error.is_some(){copy_source(&entry.source,&audio,cancel)?;}(hash,error)
    };
    cancelled(cancel)?;
    if stamp(&entry.source)? != before {
        bail!("cache source changed during portable copy")
    }
    let size = directory_size(&staging)?;
    if size > budget {
        bail!("portable copy exceeds reserved cache capacity")
    }
    Ok((
        CopyResult {
            audio,
            size,
            embedded: tag_error.is_none(),
        },
        source_sha256,
        tag_error,
    ))
}

async fn playback_active(state: &AppState) -> bool {
    state.traffic.playback_waiting()||state.resources.over_budget()
}
async fn packet_hash(state: &AppState, path: &FsPath) -> anyhow::Result<String> {
    let mut command =
        Command::new(std::env::var_os("RMUSIC_FFMPEG").unwrap_or_else(|| "ffmpeg".into()));
    command
        .args(["-nostdin", "-hide_banner", "-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-c:a", "copy", "-f", "hash", "-hash", "sha256", "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let child = command.spawn()?;
    let output = child.wait_with_output();
    tokio::pin!(output);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {output=&mut output=>{let output=output?;if !output.status.success()||output.stdout.len()>256{bail!("encoded audio checksum failed")};return Ok(String::from_utf8(output.stdout)?.trim().to_owned())},_=tick.tick()=>{if playback_active(state).await{return Err(Deferred.into())}}}
    }
}
async fn metadata(state: &AppState, track: Track) -> Metadata {
    let id = track.id.clone();
    let mut lyrics = state
        .db
        .run(move |c| {
            Ok(
                c.query_row("SELECT lyrics FROM tracks WHERE id=?1", [id], |r| {
                    r.get::<_, Option<String>>(0)
                })?,
            )
        })
        .await
        .ok()
        .flatten();
    if lyrics.is_none() && track.source != "local" {
        if let Ok(Ok(result)) = tokio::time::timeout(
            Duration::from_secs(5),
            state.plugin.call(
                &track.source,
                "lyrics",
                json!({"track_id":track.id,"track":track,"provider_data":track.provider_data}),
            ),
        )
        .await
        {
            lyrics = result["text"].as_str().map(str::to_owned);
        }
    }
    lyrics = lyrics.filter(|text| !text.is_empty() && text.len() <= MAX_LYRICS);
    let mut cover = None;
    if let Some(url) = &track.cover_url {
        if let Some(id) = url
            .rsplit('/')
            .next()
            .filter(|id| id.len() == 64 && id.chars().all(|c| c.is_ascii_hexdigit()))
        {
            let path = state.config.data.join("covers").join(id);
            if tokio::fs::metadata(&path)
                .await
                .is_ok_and(|m| m.len() <= MAX_COVER)
            {
                cover = tokio::fs::read(path).await.ok();
            }
        } else if url.starts_with("https://") || url.starts_with("http://") {
            if let Ok(response) = state
                .http
                .get(url)
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                if response.status().is_success()
                    && response.content_length().unwrap_or(0) <= MAX_COVER
                {
                    let mut bytes = Vec::new();
                    let mut stream = response.bytes_stream();
                    let mut valid = true;
                    while let Some(chunk) = stream.next().await {
                        match chunk {
                            Ok(chunk) => {
                                if bytes.len() + chunk.len() > MAX_COVER as usize {
                                    valid = false;
                                    break;
                                }
                                bytes.extend_from_slice(&chunk)
                            }
                            Err(_) => {
                                valid = false;
                                break;
                            }
                        }
                    }
                    if valid {
                        cover = Some(bytes);
                    }
                }
            }
        }
    }
    cover = cover.filter(|bytes| cover_format(bytes).is_some());
    Metadata {
        track,
        lyrics,
        cover,
    }
}

pub async fn queue_export(state: &AppState, id: &str) -> ApiResult<()> {
    if !valid_id(id) {
        return Err(ApiError::missing());
    }
    let _lock = state.cache_lock.lock().await;
    let id = id.to_owned();
    let queued=state.db.run(move|c|{
        if !c.query_row("SELECT EXISTS(SELECT 1 FROM cache_entries WHERE id=?1 AND complete=1)",[&id],|r|r.get::<_,bool>(0))?{return Ok(false)}
        c.execute("INSERT INTO cache_exports(cache_id,status,size,updated_at)VALUES(?1,'pending',0,?2) ON CONFLICT(cache_id) DO UPDATE SET status=CASE WHEN status IN ('ready','writing') THEN status ELSE 'pending' END,error=NULL,updated_at=excluded.updated_at",params![id,chrono::Utc::now().timestamp()])?;Ok(true)
    }).await?;
    if !queued {
        return Err(ApiError::bad("仅完整缓存可生成便携副本"));
    }
    state.emit("cache", json!({}));
    Ok(())
}

async fn create_export(state: &AppState, entry: Entry) -> anyhow::Result<()> {
    let guard = {
        let _lock = state.cache_lock.lock().await;
        let mut active = state.cache_active.lock().await;
        if active.contains_key(&entry.id) {
            return Err(Deferred.into());
        }
        active.insert(entry.id.clone(), 1);
        Active {
            state: state.clone(),
            id: entry.id.clone(),
        }
    };
    let result = create_export_inner(state, entry.clone()).await;
    if let Err(error) = &result {
        if let Err(cleanup) =
            failed_export(state, &entry, error.downcast_ref::<Deferred>().is_some()).await
        {
            tracing::warn!(error=%cleanup,"portable export cleanup deferred; reserved quota retained")
        }
    }
    drop(guard);
    result
}

async fn create_export_inner(state: &AppState, entry: Entry) -> anyhow::Result<()> {
    if playback_active(state).await {
        return Err(Deferred.into());
    }
    let track = crate::library::get_track(state, entry.track_id.clone())
        .await
        .map_err(|error| anyhow::anyhow!(error.2))?;
    let metadata = metadata(state, track).await;
    let budget = entry
        .size
        .saturating_add(metadata.cover.as_ref().map_or(0, |b| b.len() as u64) * 2)
        .saturating_add(metadata.lyrics.as_ref().map_or(0, |t| t.len() as u64) * 2)
        .saturating_add(1024 * 1024);
    let root = state.portable_dir().await;
    let staging = root.join(format!("{}.staging", entry.id));
    let final_dir = root.join(&entry.id);
    {
        let _lock = state.cache_lock.lock().await;
        let id = entry.id.clone();
        let previous = state
            .db
            .run(move |c| {
                Ok(c.query_row(
                    "SELECT path,status FROM cache_exports WHERE cache_id=?1",
                    [id],
                    |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?)
            })
            .await?;
        if previous.as_ref().is_some_and(|row| row.1 == "ready") {
            return Ok(());
        }
        let previous_root = previous
            .as_ref()
            .and_then(|row| row.0.as_deref())
            .map(|path| saved_export_root(path, &entry.id))
            .transpose()?;
        let cleanup_root = root.clone();
        let cleanup_staging = staging.clone();
        let cleanup_final = final_dir.clone();
        let cleanup_id = entry.id.clone();
        tokio::task::spawn_blocking(move || {
            // Keep the old reservation until all files from an interrupted export are gone.
            if let Some(old) = previous_root.filter(|old| old != &cleanup_root) {
                remove_owned_directory(&old, &old.join(format!("{cleanup_id}.staging")))?;
                remove_owned_directory(&old, &old.join(&cleanup_id))?;
            }
            remove_owned_directory(&cleanup_root, &cleanup_staging)?;
            remove_owned_directory(&cleanup_root, &cleanup_final)
        })
        .await??;
        let id = entry.id.clone();
        state
            .db
            .run(move |c| {
                c.execute(
                    "UPDATE cache_exports SET size=0 WHERE cache_id=?1 AND status!='ready'",
                    [id],
                )?;
                Ok(())
            })
            .await?;
        if !media::ensure_capacity(state, budget).await? {
            bail!("cache quota cannot hold a portable copy; original retained")
        }
        let id = entry.id.clone();
        let path = staging
            .join(filename(&metadata.track, &entry.format))
            .to_string_lossy()
            .into_owned();
        state.db.run(move|c|{c.execute("INSERT INTO cache_exports(cache_id,path,size,status,updated_at)VALUES(?1,?2,?3,'writing',?4) ON CONFLICT(cache_id)DO UPDATE SET path=excluded.path,size=excluded.size,status='writing',error=NULL,updated_at=excluded.updated_at",params![id,path,budget as i64,chrono::Utc::now().timestamp()])?;Ok(())}).await?;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let check_cancel = cancel.clone();
    let check_state = state.clone();
    let check_id=entry.id.clone();
    let _watcher = CancelWatcher(tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tick.tick().await;
            let key=check_id.clone();let stopped=check_state.db.run(move|c|Ok(c.query_row("SELECT status IN ('paused','cancelled') FROM cache_exports WHERE cache_id=?1",[key],|r|r.get::<_,bool>(0))?)).await.unwrap_or(true);
            if stopped||check_state.resources.over_budget()||playback_active(&check_state).await {
                check_cancel.store(true, Ordering::Relaxed);
                break;
            }
        }
    }));
    let before = stamp(&entry.source)?;
    let prepared = {
        let entry = entry.clone();
        let root = root.clone();
        let cancel = cancel.clone();
        let metadata = metadata;
        tokio::task::spawn_blocking(move || {
            let result = prepare_copy(&entry, &root, &metadata, budget, &cancel);
            (result, metadata)
        })
        .await?
    };
    let (mut copy, source_sha256, tag_error) = prepared.0?;
    let metadata = prepared.1;
    let packet_sha256 = if copy.embedded {
        let original = packet_hash(state, &entry.source).await?;
        let tagged = packet_hash(state, &copy.audio).await?;
        if original != tagged {
            bail!("tagging changed encoded audio; original cache retained")
        }
        Some(original)
    } else {
        None
    };
    cancelled(&cancel)?;
    if stamp(&entry.source)? != before {
        bail!("cache source changed during tag verification")
    }
    let result = {
        let audio = copy.audio.clone();
        let source = entry.source.clone();
        let source_before = before;
        let cancel = cancel.clone();
        let dir = staging.clone();
        let format = copy
            .audio
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or(&entry.format)
            .to_owned();
        tokio::task::spawn_blocking(move||->anyhow::Result<u64>{
            if let Some(lyrics)=&metadata.lyrics{std::fs::write(audio.with_extension("lrc"),lyrics)?;}
            if let Some(cover)=&metadata.cover{if let Some((ext,_))=cover_format(cover){std::fs::write(dir.join(format!("cover.{ext}")),cover)?;}}
            let manifest=json!({"version":1,"title":metadata.track.title,"artist":metadata.track.artist,"album":metadata.track.album,"album_artist":metadata.track.album_artist,"track_number":metadata.track.track_number,"disc_number":metadata.track.disc_number,"recording_id":metadata.track.recording_id,"source":metadata.track.source,"format":format,"sample_rate":metadata.track.sample_rate,"bit_depth":metadata.track.bit_depth,"integrated_lufs":metadata.track.integrated_lufs,"embedded":copy.embedded,"tag_note":tag_error,"source_sha256":source_sha256,"encoded_audio_sha256":packet_sha256,"exported_at":chrono::Utc::now().timestamp()});
            let json=serde_json::to_vec_pretty(&manifest)?;if json.len()>1024*1024{bail!("portable metadata is too large")};std::fs::write(dir.join("metadata.json"),json)?;
            cancelled(&cancel)?;if stamp(&source)?!=source_before{bail!("cache source changed before export publication")}
            let size=directory_size(&dir)?;if size>budget{bail!("portable export exceeds reserved capacity")};
            for file in std::fs::read_dir(&dir)?{std::fs::OpenOptions::new().write(true).open(file?.path())?.sync_all()?;}
            Ok(size)
        }).await??
    };
    copy.size = result;
    {
        let _lock = state.cache_lock.lock().await;
        if final_dir.exists() {
            bail!("portable destination already exists")
        }
        tokio::fs::rename(&staging, &final_dir).await?;
        let id = entry.id.clone();
        let path = final_dir
            .join(copy.audio.file_name().context("missing audio filename")?)
            .to_string_lossy()
            .into_owned();
        let size = copy.size;
        state.db.run(move|c|{c.execute("UPDATE cache_exports SET path=?1,size=?2,status='ready',error=NULL,updated_at=?3,finalize_single=COALESCE((SELECT single_file FROM cache_entries WHERE id=?4),0) WHERE cache_id=?4",params![path,size as i64,chrono::Utc::now().timestamp(),id])?;Ok(())}).await?;
    }
    state.emit("cache", json!({}));
    Ok(())
}

pub(crate) async fn finalize_single(state:&AppState)->anyhow::Result<bool> {
    let entry=state.db.run(|c|Ok(c.query_row("SELECT c.id,c.path,e.path,e.size,c.track_id FROM cache_entries c JOIN cache_exports e ON e.cache_id=c.id WHERE c.complete=1 AND c.single_file=1 AND e.status='ready' AND e.finalize_single=1 LIMIT 1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?))).optional()?)).await?;
    let Some((id,source,target,total,track_id))=entry else{return Ok(false);};
    let source=PathBuf::from(source);let target=PathBuf::from(target);
    if !valid_id(&id)||!source.is_absolute()||source.file_name().and_then(|n|n.to_str())!=Some(&id)||saved_export_root(target.to_str().context("output path is not UTF-8")?,&id).is_err() {bail!("canonical cache paths do not match the managed layout");}
    if state.cache_active.lock().await.contains_key(&id){return Ok(false);}
    let cancel=Arc::new(AtomicBool::new(false));let check=cancel.clone();let check_state=state.clone();
    let _watcher=CancelWatcher(tokio::spawn(async move{let mut tick=tokio::time::interval(Duration::from_millis(250));loop{tick.tick().await;if playback_active(&check_state).await{check.store(true,Ordering::Relaxed);break;}}}));
    let prepared=target.clone();
    let (bytes,etag,after)=tokio::task::spawn_blocking(move||->anyhow::Result<_>{let before=stamp(&prepared)?;let mut file=std::fs::File::open(&prepared)?;let mut hash=Sha256::new();let mut buffer=[0u8;64*1024];loop{cancelled(&cancel)?;let n=file.read(&mut buffer)?;if n==0{break;}hash.update(&buffer[..n]);}let after=stamp(&prepared)?;if before!=after{bail!("tagged cache changed during hashing");}Ok((after.0,format!("\"cache-{:x}\"",hash.finalize()),after))}).await??;
    let _lock=state.cache_lock.lock().await;
    if state.cache_active.lock().await.contains_key(&id)||playback_active(state).await{return Ok(false);}
    if stamp(&target)?!=after||total<bytes as i64{bail!("canonical cache size changed");}
    let original=crate::library::get_track(state,track_id).await.map_err(|e|anyhow::anyhow!(e.2))?;
    let prepared=target.clone();let data=state.config.data.clone();let local_id=crate::auth::hash(&target.to_string_lossy());
    let (mut local,lyrics)=tokio::task::spawn_blocking(move||crate::library::probe_track(&prepared,&local_id,prepared.extension().and_then(|v|v.to_str()).unwrap_or(""),&data)).await??;
    local.recording_id=original.recording_id;local.provider_data=original.provider_data.clone();local.provider_data["cached_from"]=json!({"track_id":original.provider_data["cached_from"]["track_id"].as_str().unwrap_or(&original.id),"source":original.provider_data["cached_from"]["source"].as_str().unwrap_or(&original.source),"cache_id":id});
    let quality_id=id.clone();let quality=state.db.run(move|c|Ok(c.query_row("SELECT data FROM cache_quality WHERE cache_id=?1",[quality_id],|r|r.get::<_,String>(0)).optional()?)).await?;if let Some(quality)=quality{local.provider_data["cache_quality"]=serde_json::from_str(&quality)?;}
    let mtime=after.1.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let old_size=std::fs::metadata(&source)?.len();
    let old=source.to_string_lossy().into_owned();let root=source.parent().context("cache parent missing")?.to_string_lossy().into_owned();let path=target.to_string_lossy().into_owned();
    let published=local.clone();
    state.db.run(move|c|{let tx=c.transaction()?;
        let current=tx.query_row("SELECT path FROM cache_entries WHERE id=?1",[&id],|r|r.get::<_,String>(0))?;if current!=old{bail!("canonical cache source changed");}
        tx.execute("INSERT INTO cache_retired(path,root,size) VALUES(?1,?2,?3)",params![old,root,old_size as i64])?;
        tx.execute("UPDATE cache_entries SET path=?2,size=?3,total_size=?3,allocated_size=?3,ranges=?4,content_etag=?5 WHERE id=?1",params![id,path,bytes as i64,json!([[0,bytes]]).to_string(),etag])?;
        tx.execute("UPDATE cache_exports SET size=?2,finalize_single=0 WHERE cache_id=?1",params![id,total-bytes as i64])?;
        crate::library::upsert(&tx,&local,Some(&path),None,mtime,bytes,None,lyrics.as_deref())?;
        tx.commit()?;Ok(())
    }).await?;
    drop(_lock);crate::cache_versions::consider(state,published).await?;cleanup_retired(state).await?;state.emit("cache",json!({}));state.emit("library",json!({}));Ok(true)
}
pub(crate) async fn cleanup_retired(state:&AppState)->anyhow::Result<()> {
    let _lock=state.cache_lock.lock().await;
    let entries=state.db.run(|c|{let mut q=c.prepare("SELECT path,root FROM cache_retired")?;let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(rows)}).await?;
    for(path,root)in entries {let source=PathBuf::from(&path);let root=PathBuf::from(root);
        if !source.is_absolute()||source.parent()!=Some(root.as_path())||source.file_name().and_then(|n|n.to_str()).is_none_or(|n|!valid_id(n)){bail!("retired cache path escaped its recorded root");}
        match tokio::fs::symlink_metadata(&source).await {
            Ok(stat)=>{if !stat.is_file()||stat.file_type().is_symlink(){bail!("retired cache must be a regular file");}let resolved=tokio::fs::canonicalize(&source).await?;let root=tokio::fs::canonicalize(&root).await?;if resolved.parent()!=Some(root.as_path()){bail!("retired cache resolved outside its root");}if let Err(error)=tokio::fs::remove_file(&resolved).await {tracing::debug!(%error,"retired cache still in use");continue;}}
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},Err(error)=>return Err(error.into()),
        }
        state.db.run(move|c|{c.execute("DELETE FROM cache_retired WHERE path=?1",[path])?;Ok(())}).await?;
    }Ok(())
}

pub fn spawn(state: AppState) {
    static WORKER: OnceLock<Arc<Mutex<()>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| Arc::new(Mutex::new(()))).clone();
    tokio::spawn(crate::traffic::BACKGROUND.scope(true,async move {
        let _=state.db.run(|c|{c.execute("UPDATE cache_exports SET status='pending',error='核心重启，等待重新生成' WHERE status='writing'",[])?;Ok(())}).await;
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            if !crate::cache_policy::enabled(&state).await || playback_active(&state).await {
                continue;
            }
            let Ok(_resource)=state.resources.background(&state,16*1024*1024,false).await else{continue;};
            let Ok(_worker) = worker.try_lock() else {
                continue;
            };
            if let Err(error)=cleanup_retired(&state).await {tracing::warn!(%error,"retired cache cleanup deferred");}
            if let Err(error)=crate::cache_versions::cleanup(&state).await{tracing::warn!(%error,"better cache replacement deferred");}
            match finalize_single(&state).await {Ok(true)=>continue,Err(error)=>tracing::warn!(%error,"single cache publication deferred"),_=>{}}
            let entries=state.db.run(|c|{let mut q=c.prepare("SELECT c.id,c.track_id,c.path,c.total_size,c.format FROM cache_entries c LEFT JOIN cache_exports e ON e.cache_id=c.id WHERE c.complete=1 AND ((c.single_file=1 AND e.cache_id IS NULL) OR e.status='pending') ORDER BY c.last_access DESC LIMIT 32")?;let entries=q.query_map([],|r|Ok(Entry{id:r.get(0)?,track_id:r.get(1)?,source:r.get::<_,String>(2)?.into(),size:r.get::<_,i64>(3)?as u64,format:r.get(4)?}))?.collect::<Result<Vec<_>,_>>()?;Ok(entries)}).await;
            let Ok(entries) = entries else { continue };
            for entry in entries {
                if state.cache_active.lock().await.contains_key(&entry.id) {
                    continue;
                }
                let result = create_export(&state, entry.clone()).await;
                if let Err(error) = result {
                    let deferred = error.downcast_ref::<Deferred>().is_some();
                    if !deferred {
                        tracing::warn!(error=%error,"portable cache generation failed")
                    }
                }
                break;
            }
        }
    }));
}

async fn failed_export(state: &AppState, entry: &Entry, deferred: bool) -> anyhow::Result<()> {
    let _lock = state.cache_lock.lock().await;
    let id = entry.id.clone();
    let saved = state
        .db
        .run(move |c| {
            Ok(c.query_row(
                "SELECT path,size,status FROM cache_exports WHERE cache_id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, i64>(1)? as u64,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?)
        })
        .await?;
    if saved.as_ref().is_some_and(|row| row.2 == "ready") {
        return Ok(());
    }
    let root = if let Some(path) = saved.as_ref().and_then(|row| row.0.as_ref()) {
        saved_export_root(path, &entry.id)?
    } else {
        state.cache_dir().await.join("portable")
    };
    let staging = root.join(format!("{}.staging", entry.id));
    let final_dir = root.join(&entry.id);
    let cleaned = tokio::task::spawn_blocking(move || {
        remove_owned_directory(&root, &staging)?;
        remove_owned_directory(&root, &final_dir)
    })
    .await?
    .is_ok();
    let reserved = if cleaned {
        0
    } else {
        saved.as_ref().map_or(entry.size, |row| row.1)
    };
    let status = if deferred { "pending" } else { "failed" };
    let message = if deferred {
        "播放优先，等待空闲生成便携副本"
    } else {
        "便携副本生成失败，原始缓存保留"
    };
    let id = entry.id.clone();
    state.db.run(move|c|{if !c.query_row("SELECT EXISTS(SELECT 1 FROM cache_entries WHERE id=?1)",[&id],|r|r.get::<_,bool>(0))?{return Ok(())}c.execute("INSERT INTO cache_exports(cache_id,size,status,error,updated_at)VALUES(?1,?2,?3,?4,?5) ON CONFLICT(cache_id)DO UPDATE SET size=excluded.size,status=CASE WHEN status IN ('paused','cancelled') THEN status ELSE excluded.status END,error=excluded.error,updated_at=excluded.updated_at",params![id,reserved as i64,status,message,chrono::Utc::now().timestamp()])?;Ok(())}).await?;
    Ok(())
}

#[derive(Deserialize, Default)]
pub struct ExportQuery {
    pub ticket: Option<String>,
    pub asset: Option<String>,
}
pub async fn file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ExportQuery>,
    headers: HeaderMap,
    method: Method,
) -> ApiResult<Response> {
    if !valid_id(&id) {
        return Err(ApiError::missing());
    }
    media::authorize(
        &state,
        &headers,
        &media::TicketQuery {
            ticket: query.ticket,
        },
        &format!("cache:{id}"),
    )
    .await?;
    let lock = state.cache_lock.lock().await;
    let cid = id.clone();
    let path=state.db.run(move|c|Ok(c.query_row("SELECT e.path FROM cache_exports e JOIN cache_entries c ON c.id=e.cache_id WHERE e.cache_id=?1 AND e.status='ready' AND c.complete=1",[cid],|r|r.get::<_,String>(0)).optional()?)).await?;
    let Some(path) = path else {
        drop(lock);
        queue_export(&state, &id).await?;
        return Err(ApiError(
            StatusCode::CONFLICT,
            "export_pending".into(),
            "便携副本尚未生成，空闲时会自动处理".into(),
        ));
    };
    let audio = PathBuf::from(path);
    let parent = audio.parent().ok_or_else(ApiError::missing)?;
    let path = match query.asset.as_deref().unwrap_or("audio") {
        "audio" => audio.clone(),
        "metadata" => parent.join("metadata.json"),
        "lyrics" => audio.with_extension("lrc"),
        "cover" => ["jpg", "png", "webp"]
            .into_iter()
            .map(|ext| parent.join(format!("cover.{ext}")))
            .find(|p| p.is_file())
            .ok_or_else(ApiError::missing)?,
        _ => return Err(ApiError::bad("无效便携文件类型")),
    };
    let canonical = path.canonicalize().map_err(|_| ApiError::missing())?;
    let directory = parent.canonicalize().map_err(|_| ApiError::missing())?;
    if canonical.parent() != Some(directory.as_path())
        || canonical
            .parent()
            .and_then(FsPath::file_name)
            .and_then(|v| v.to_str())
            != Some(id.as_str())

    {
        return Err(ApiError::missing());
    }
    let mut input = tokio::fs::File::open(&canonical)
        .await
        .map_err(|_| ApiError::missing())?;
    let stat = input.metadata().await.map_err(anyhow::Error::from)?;
    let total = stat.len();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, partial) = media::range(range, total).map_err(|_| {
        ApiError(
            StatusCode::RANGE_NOT_SATISFIABLE,
            "invalid_range".into(),
            "文件Range超出范围".into(),
        )
    })?;
    input
        .seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(anyhow::Error::from)?;
    *state
        .cache_active
        .lock()
        .await
        .entry(id.clone())
        .or_default() += 1;
    let guard = Active {
        state: state.clone(),
        id: id.clone(),
    };
    drop(lock);
    let stream =
        tokio_util::io::ReaderStream::with_capacity(input.take(end - start + 1), 64 * 1024);
    let stream = futures_util::stream::unfold((stream, guard), |(mut stream, guard)| async move {
        stream.next().await.map(|chunk| (chunk, (stream, guard)))
    });
    let name = canonical
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("music");
    let encoded = url::form_urlencoded::byte_serialize(name.as_bytes()).collect::<String>().replace('+', "%20");
    let mime = mime_guess::from_path(&canonical)
        .first_or_octet_stream()
        .to_string();
    let mut response = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, (end - start + 1).to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename*=UTF-8''{encoded}"),
        )
        .header(header::CACHE_CONTROL, "private, no-cache")
        .header("X-RMusic-Portable", "true");
    if partial {
        response = response.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total}"),
        );
    }
    response
        .body(if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from_stream(stream)
        })
        .map_err(anyhow::Error::from)
        .map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn track(format: &str) -> Track {
        Track {
            id: "song".into(),
            title: "晚风 / Live".into(),
            artist: "测试艺术家".into(),
            album: "测试专辑".into(),
            album_artist: "测试艺术家".into(),
            duration: 0.1,
            track_number: 2,
            disc_number: 1,
            source: "local".into(),
            format: format.into(),
            sample_rate: 48000,
            bit_depth: 24,
            channels: 2,
            bitrate: 0,
            cover_url: None,
            recording_id: "recording".into(),
            available: true,
            created_at: 0,
            integrated_lufs: None,
            provider_data: json!({}),
        }
    }
    fn pcm(path: &FsPath) -> Vec<u8> {
        let result = std::process::Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:a:0", "-f", "s32le", "pipe:1"])
            .output()
            .unwrap();
        assert!(result.status.success());
        result.stdout
    }
    #[test]
    fn flac_mp3_m4a_tags_preserve_every_decoded_sample_and_original_bytes() {
        for (format, codec) in [("flac", "flac"), ("mp3", "libmp3lame"), ("m4a", "aac")] {
            let directory = tempfile::tempdir().unwrap();
            let source = directory.path().join(format!("input.{format}"));
            assert!(std::process::Command::new("ffmpeg")
                .args([
                    "-nostdin",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "aevalsrc=0.1*sin(2*PI*997*t):s=48000:d=0.1",
                    "-c:a",
                    codec
                ])
                .arg(&source)
                .status()
                .unwrap()
                .success());
            let before = std::fs::read(&source).unwrap();
            let before_pcm = pcm(&source);
            let entry = Entry {
                id: "cache-id".into(),
                track_id: "song".into(),
                size: before.len() as u64,
                source: source.clone(),
                format: format.into(),
            };
            let cover=data_encoding::BASE64.decode(b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aZwAAAABJRU5ErkJggg==").unwrap();
            let metadata = Metadata {
                track: track(format),
                lyrics: Some("[00:00.00]晚风".into()),
                cover: Some(cover.clone()),
            };
            let (copy, _, note) = prepare_copy(
                &entry,
                &directory.path().join("portable"),
                &metadata,
                1024 * 1024,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(copy.embedded, "{format}: {note:?}");
            assert_eq!(before, std::fs::read(source).unwrap());
            assert_eq!(before_pcm, pcm(&copy.audio));
            let tagged = Probe::open(&copy.audio).unwrap().read().unwrap();
            let tag = tagged.primary_tag().unwrap();
            assert_eq!(tag.title().as_deref(), Some("晚风 / Live"));
            assert_eq!(tag.get_string(&ItemKey::Lyrics), Some("[00:00.00]晚风"));
            assert_eq!(tag.pictures()[0].data(), cover);
        }
    }
    #[test]
    fn portable_filename_and_cleanup_stay_inside_cache_root() {
        let mut t = track("flac");
        t.title = "../../CON:bad?\\name".into();
        let name = filename(&t, "flac");
        assert!(
            !name.contains('/')
                && !name.contains('\\')
                && !name.contains(':')
                && !name.contains('?')
        );
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        assert!(remove_owned_directory(root.path(), outside.path()).is_err());
        assert!(outside.path().exists());
    }

    async fn fixture() -> (tempfile::TempDir, AppState, Entry) {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let source = cache.join("cache-id");
        assert!(std::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.1*sin(2*PI*997*t):s=48000:d=0.1",
                "-c:a",
                "flac",
                "-f",
                "flac"
            ])
            .arg(&source)
            .status()
            .unwrap()
            .success());
        let size = std::fs::metadata(&source).unwrap().len();
        let path = source.to_string_lossy().into_owned();
        let db = crate::db::Db::open(&directory.path().join("library.db")).unwrap();
        db.run(move|c|{c.execute("INSERT INTO tracks(id,path,title,artist,album,album_artist,source,format,recording_id,created_at,lyrics)VALUES('song',?1,'晚风','测试艺术家','测试专辑','测试艺术家','local','flac','recording',0,'[00:00.00]晚风')",[&path])?;
            c.execute("INSERT INTO cache_entries(id,track_id,format,size,total_size,complete,last_access,path,etag,ranges)VALUES('cache-id','song','flac',?1,?1,1,0,?2,'source-etag',?3)",params![size as i64,path,format!("[[0,{size}]]")])?;Ok(())}).await.unwrap();
        let state = AppState::new(
            db,
            crate::state::Config {
                data: directory.path().into(),
                web: directory.path().into(),
                base_path: String::new(),
                public_url: None,
                plugin_dir: directory.path().join("plugins"),
                python: if cfg!(windows){"python".into()}else{"python3".into()},
            },
            String::new(),
        )
        .await
        .unwrap();
        *state.cache_directory.write().await = cache;
        let entry = Entry {
            id: "cache-id".into(),
            track_id: "song".into(),
            source,
            size,
            format: "flac".into(),
        };
        (directory, state, entry)
    }
    #[tokio::test]
    async fn single_cache_publication_keeps_audio_and_updates_http_identity() {
        use axum::{http::Request,routing::get,Router};use tower::ServiceExt;use http_body_util::BodyExt;use futures_util::{stream,StreamExt};
        let (directory,state,mut entry)=fixture().await;
        let id=crate::auth::hash("song:source-etag");let source=entry.source.parent().unwrap().join(&id);std::fs::rename(&entry.source,&source).unwrap();entry.id=id.clone();entry.source=source.clone();
        let cid=id.clone();let path=source.to_string_lossy().into_owned();
        state.db.run(move|c|{c.execute("UPDATE cache_entries SET id=?1,path=?2,single_file=1 WHERE id='cache-id'",params![cid,path])?;c.execute("UPDATE tracks SET path=?1 WHERE id='song'",[path])?;Ok(())}).await.unwrap();
        let output=directory.path().join("my-music");std::fs::create_dir_all(&output).unwrap();let saved=json!(output).to_string();state.db.run(move|c|{c.execute("INSERT INTO settings(key,value) VALUES('portable_directory',?1)",[saved])?;Ok(())}).await.unwrap();
        let before_pcm=pcm(&source);create_export(&state,entry).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2),async{while state.cache_active.lock().await.contains_key(&id){tokio::task::yield_now().await;}}).await.unwrap();
        state.cache_active.lock().await.insert(id.clone(),1);assert!(!finalize_single(&state).await.unwrap());assert!(source.exists());state.cache_active.lock().await.remove(&id);
        assert!(finalize_single(&state).await.unwrap());assert!(!source.exists());
        let cid=id.clone();let (target,etag,used,retired)=state.db.run(move|c|{let (path,etag)=c.query_row("SELECT path,content_etag FROM cache_entries WHERE id=?1",[cid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;
            c.execute("UPDATE tracks SET source='cached-test-provider',path=NULL WHERE id='song'",[])?;
            let used=c.query_row("SELECT (SELECT SUM(size) FROM cache_entries)+(SELECT SUM(size) FROM cache_exports)",[],|r|r.get::<_,i64>(0))?;let retired=c.query_row("SELECT COUNT(*) FROM cache_retired",[],|r|r.get::<_,i64>(0))?;Ok((PathBuf::from(path),etag,used,retired))}).await.unwrap();
        assert!(target.starts_with(directory.path().join("my-music")));let owned=target.to_string_lossy().into_owned();let local=state.db.run(move|c|Ok(c.query_row("SELECT source,json_extract(provider_data,'$.cached_from.track_id') FROM tracks WHERE path=?1",[owned],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?)).await.unwrap();assert_eq!(local,("local".into(),"song".into()));
        let tagged=std::fs::read(&target).unwrap();assert_eq!(before_pcm,pcm(&target));assert_eq!(used as u64,directory_size(target.parent().unwrap()).unwrap());assert_eq!(retired,0);
        let input=stream::iter([Ok(axum::body::Bytes::from_static(b"raw late bytes"))]).boxed();let mut late=media::caching_stream(state.clone(),"song".into(),"flac".into(),"source-etag".into(),entry_size(&state,&id).await,0,input).await.unwrap();assert!(late.next().await.unwrap().is_ok());drop(late);assert_eq!(std::fs::read(&target).unwrap(),tagged);
        let ticket=state.ticket("media:song".into(),60).await;let app=Router::new().route("/media/{id}",get(media::media)).with_state(state.clone());
        for (range_tag,status) in [("source-etag",StatusCode::OK),(etag.as_str(),StatusCode::PARTIAL_CONTENT)] {let response=app.clone().oneshot(Request::builder().uri(format!("/media/song?ticket={ticket}")).header(header::RANGE,"bytes=2-5").header(header::IF_RANGE,range_tag).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),status);assert_eq!(response.headers()[header::ETAG],etag);let bytes=response.into_body().collect().await.unwrap().to_bytes();if status==StatusCode::OK{assert_eq!(bytes.as_ref(),tagged);}else{assert_eq!(bytes.as_ref(),&tagged[2..6]);}}
        let response=app.oneshot(Request::builder().uri(format!("/media/song?ticket={ticket}")).header(header::IF_NONE_MATCH,etag).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::NOT_MODIFIED);
    }
    async fn entry_size(state:&AppState,id:&str)->u64 {let id=id.to_owned();state.db.run(move|c|Ok(c.query_row("SELECT total_size FROM cache_entries WHERE id=?1",[id],|r|r.get::<_,i64>(0))? as u64)).await.unwrap()}
    #[tokio::test]
    async fn background_queue_publishes_tagged_file_range_and_sidecars_after_directory_switch() {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let (directory, state, entry) = fixture().await;
        let original = std::fs::read(&entry.source).unwrap();
        queue_export(&state, &entry.id).await.unwrap();
        spawn(state.clone());
        let exported = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let status = state
                    .db
                    .run(|c| {
                        Ok(c.query_row(
                            "SELECT status,path,size FROM cache_exports WHERE cache_id='cache-id'",
                            [],
                            |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, Option<String>>(1)?,
                                    r.get::<_, i64>(2)?,
                                ))
                            },
                        )
                        .optional()?)
                    })
                    .await
                    .unwrap();
                if let Some((status, Some(path), size)) = status {
                    assert_ne!(status, "failed");
                    if status == "ready" {
                        break (PathBuf::from(path), size as u64);
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(&entry.source).unwrap(), original);
        assert_eq!(
            exported.1,
            directory_size(exported.0.parent().unwrap()).unwrap()
        );
        assert_eq!(pcm(&entry.source), pcm(&exported.0));
        let new_cache = directory.path().join("new-cache");
        std::fs::create_dir(&new_cache).unwrap();
        *state.cache_directory.write().await = new_cache;
        let ticket = state.ticket("cache:cache-id".into(), 60).await;
        let router = axum::Router::new()
            .route("/cache/{id}/file", axum::routing::get(file))
            .with_state(state.clone());
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/cache/cache-id/file?ticket={ticket}"))
                    .header(header::RANGE, "bytes=0-15")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "16");
        assert_eq!(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .len(),
            16
        );
        let response = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/cache/cache-id/file?ticket={ticket}&asset=metadata"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let manifest: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(manifest["embedded"], true);
        assert!(manifest["encoded_audio_sha256"]
            .as_str()
            .unwrap()
            .starts_with("SHA256="));
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/cache/cache-id/file?ticket={ticket}&asset=lyrics"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            "[00:00.00]晚风"
        );
    }
    #[tokio::test]
    async fn insufficient_quota_keeps_original_and_releases_export_reservation() {
        let (_directory, state, entry) = fixture().await;
        let size = entry.size;
        state
            .db
            .run(move |c| {
                c.execute(
                    "INSERT INTO settings(key,value)VALUES('cache_limit_bytes',?1)",
                    [size.to_string()],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        queue_export(&state, &entry.id).await.unwrap();
        assert!(create_export(&state, entry.clone()).await.is_err());
        failed_export(&state, &entry, false).await.unwrap();
        assert!(entry.source.exists());
        let (status, size) = state
            .db
            .run(|c| {
                Ok(c.query_row(
                    "SELECT status,size FROM cache_exports WHERE cache_id='cache-id'",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(status, "failed");
        assert_eq!(size, 0);
    }
    #[tokio::test]
    async fn failed_copy_cleans_its_old_root_after_online_directory_switch() {
        let (directory, state, entry) = fixture().await;
        let old_root = state.cache_dir().await.join("portable");
        let staging = old_root.join("cache-id.staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("partial.flac"), b"partial").unwrap();
        let path = staging.join("partial.flac").to_string_lossy().into_owned();
        state.db.run(move|c|{c.execute("INSERT INTO cache_exports(cache_id,path,size,status,updated_at)VALUES('cache-id',?1,100,'writing',0)",[path])?;Ok(())}).await.unwrap();
        let new_root = directory.path().join("new-cache");
        std::fs::create_dir(&new_root).unwrap();
        std::fs::write(new_root.join("keep"), b"keep").unwrap();
        *state.cache_directory.write().await = new_root.clone();
        failed_export(&state, &entry, true).await.unwrap();
        assert!(!staging.exists());
        assert!(new_root.join("keep").exists());
        let (status, size) = state
            .db
            .run(|c| {
                Ok(c.query_row(
                    "SELECT status,size FROM cache_exports WHERE cache_id='cache-id'",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(size, 0);
    }
    #[tokio::test]
    async fn interrupted_export_recovers_old_staging_before_publishing_in_new_directory() {
        let (directory, state, entry) = fixture().await;
        let old_root = state.cache_dir().await.join("portable");
        let staging = old_root.join("cache-id.staging");
        let unfinished = old_root.join("cache-id");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::create_dir(&unfinished).unwrap();
        std::fs::write(staging.join("partial.flac"), b"partial").unwrap();
        std::fs::write(unfinished.join("unpublished.flac"), b"unpublished").unwrap();
        let path = staging.join("partial.flac").to_string_lossy().into_owned();
        state.db.run(move|c|{c.execute("INSERT INTO cache_exports(cache_id,path,size,status,updated_at)VALUES('cache-id',?1,100,'pending',0)",[path])?;Ok(())}).await.unwrap();
        let new_root = directory.path().join("new-cache");
        std::fs::create_dir(&new_root).unwrap();
        *state.cache_directory.write().await = new_root.clone();
        create_export(&state, entry.clone()).await.unwrap();
        assert!(!staging.exists());
        assert!(!unfinished.exists());
        let (path, status, size) = state
            .db
            .run(|c| {
                Ok(c.query_row(
                    "SELECT path,status,size FROM cache_exports WHERE cache_id='cache-id'",
                    [],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                        ))
                    },
                )?)
            })
            .await
            .unwrap();
        assert_eq!(status, "ready");
        let path = PathBuf::from(path);
        assert!(path.starts_with(new_root.join("portable")));
        assert_eq!(size as u64, directory_size(path.parent().unwrap()).unwrap());
        assert!(entry.source.exists());
    }
    #[tokio::test]
    async fn playback_priority_cancels_the_actual_hash_process_and_defers_export() {
        let (_directory, state, entry) = fixture().await;
        state.traffic.song_limit.store(1,Ordering::Release);let held=state.traffic.song("background",true).await;
        let traffic=state.traffic.clone();let pending=tokio::spawn(async move{traffic.song("foreground",false).await});tokio::task::yield_now().await;assert!(state.traffic.playback_waiting());
        let error = packet_hash(&state, &entry.source).await.unwrap_err();
        assert!(error.downcast_ref::<Deferred>().is_some());
        assert!(create_export(&state, entry.clone())
            .await
            .unwrap_err()
            .downcast_ref::<Deferred>()
            .is_some());
        assert!(entry.source.exists());pending.abort();let _=pending.await;drop(held);
    }
}
