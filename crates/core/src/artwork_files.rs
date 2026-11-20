//! Immutable, validated image files and HTTP cache negotiation.
use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use futures_util::TryStreamExt;
use std::sync::atomic::Ordering;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::SystemTime,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
const MAX: usize = crate::artwork_images::INPUT_LIMIT;
#[derive(Clone, PartialEq)]
struct Stamp {
    len: u64,
    modified: SystemTime,
}
static VALID: LazyLock<Mutex<HashMap<PathBuf, (Stamp, String)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|v| v.is_ascii_hexdigit())
}
pub fn mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}
pub fn complete(bytes: &[u8]) -> bool {
    match mime(bytes) {
        Some("image/png") => bytes.len() >= 24 && bytes.ends_with(b"\0\0\0\0IEND\xaeB`\x82"),
        Some("image/jpeg") => {
            bytes.len() >= 4
                && bytes[bytes.len().saturating_sub(64)..]
                    .windows(2)
                    .any(|v| v == b"\xff\xd9")
        }
        Some("image/webp") => {
            bytes.len() >= 16
                && u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize + 8 == bytes.len()
        }
        _ => false,
    }
}
pub fn content_id(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn corrupt() -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        "artwork_corrupt".into(),
        "图片缓存损坏，正在等待重新获取".into(),
    )
}
fn io_error(e: std::io::Error) -> ApiError {
    if e.kind() == std::io::ErrorKind::NotFound {
        ApiError::missing()
    } else {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "artwork_storage_unavailable".into(),
            "图片存储不可读或不可写，请检查磁盘与权限".into(),
        )
    }
}
pub struct ImageFile {
    file: tokio::fs::File,
    pub id: String,
    size: u64,
    mime: &'static str,
    version: String,
    metrics: std::sync::Arc<crate::artwork_maintenance::Counters>,
}
impl ImageFile {
    pub async fn response(mut self, headers: &HeaderMap, immutable: bool) -> ApiResult<Response> {
        let etag = format!("\"{}\"", self.version);
        let cached = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(',').any(|v| {
                    let v = v.trim().strip_prefix("W/").unwrap_or(v.trim());
                    v == etag || v == "*"
                })
            });
        self.metrics.requests.fetch_add(1,Ordering::Relaxed);
        let policy = if immutable {
            "private, max-age=31536000, immutable"
        } else {
            "private, max-age=60, must-revalidate"
        };
        let mut response = if cached {
            self.metrics.not_modified.fetch_add(1,Ordering::Relaxed);
            StatusCode::NOT_MODIFIED.into_response()
        } else {
            self.file
                .seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(io_error)?;
            let metrics=self.metrics.clone();
            let mut r = axum::body::Body::from_stream(tokio_util::io::ReaderStream::with_capacity(
                self.file,
                64 * 1024,
            ).inspect_ok(move|chunk|{metrics.response_bytes.fetch_add(chunk.len() as u64,Ordering::Relaxed);}))
            .into_response();
            r.headers_mut()
                .insert(header::CONTENT_LENGTH, self.size.into());
            r
        };
        let h = response.headers_mut();
        h.insert(header::CONTENT_TYPE, self.mime.parse().unwrap());
        h.insert(header::ETAG, etag.parse().unwrap());
        h.insert(header::CACHE_CONTROL, policy.parse().unwrap());
        h.insert(header::VARY, "Authorization".parse().unwrap());
        h.insert("x-rmusic-artwork-version", self.version.parse().unwrap());
        Ok(response)
    }
}
pub async fn open(s: &AppState, id: &str) -> ApiResult<ImageFile> {
    open_at(&s.config.data.join("covers"), id).await
}
pub async fn open_at(root: &Path, id: &str) -> ApiResult<ImageFile> {
    if !valid_id(id) {
        return Err(ApiError::missing());
    }
    let path = root.join(id);
    let mut file = tokio::fs::File::open(&path).await.map_err(io_error)?;
    let meta = file.metadata().await.map_err(io_error)?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX as u64 {
        return Err(corrupt());
    }
    let stamp = Stamp {
        len: meta.len(),
        modified: meta.modified().map_err(io_error)?,
    };
    let mut head = [0; 16];
    let read = file.read(&mut head).await.map_err(io_error)?;
    let mime = mime(&head[..read]).ok_or_else(corrupt)?;
    let previous = VALID
        .lock()
        .unwrap()
        .get(&path)
        .filter(|v| v.0 == stamp)
        .map(|v| v.1.clone());
    let version = if let Some(version) = previous {
        version
    } else {
        file.seek(std::io::SeekFrom::Start(0))
            .await
            .map_err(io_error)?;
        let mut raw = Sha256::new();
        let mut legacy = Sha256::new();
        // This buffer lives across read().await. An inline array expands every
        // parent future and its debug poll-frame copies (refresh jobs run on a
        // 2 MiB Tokio worker stack). Keep one bounded allocation per validation.
        let mut buffer = vec![0u8; 49152];
        let mut pending = Vec::with_capacity(49154);
        loop {
            let n = file.read(&mut buffer).await.map_err(io_error)?;
            if n == 0 {
                break;
            }
            raw.update(&buffer[..n]);
            pending.extend_from_slice(&buffer[..n]);
            let end = pending.len() / 3 * 3;
            legacy.update(data_encoding::BASE64.encode(&pending[..end]).as_bytes());
            pending.drain(..end);
        }
        legacy.update(data_encoding::BASE64.encode(&pending).as_bytes());
        let digest = raw.finalize();
        let current = format!("{digest:x}");
        let old = crate::auth::hash(&format!("{digest:?}"));
        let embedded = format!("{:x}", legacy.finalize());
        if id != current && id != old && id != embedded {
            return Err(corrupt());
        }
        let mut cache = VALID.lock().unwrap();
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(path, (stamp, current.clone()));
        current
    };
    Ok(ImageFile {
        file,
        id: id.into(),
        size: meta.len(),
        mime,
        version,
        metrics: crate::artwork_maintenance::counters(root),
    })
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub async fn save(s: &AppState, bytes: &[u8]) -> ApiResult<String> {
    if bytes.len() > MAX || !complete(bytes) {
        return Err(corrupt());
    }
    let id = content_id(bytes);
    let gate=crate::artwork_maintenance::storage_gate(&s.config.data);let _guard=gate.lock().await;
    if open(s,&id).await.is_ok(){return Ok(id);}
    let(old,created)=crate::artwork_maintenance::reserve(s,&id,bytes.len() as u64).await?;
    let dir = s.config.data.join("covers");
    tokio::fs::create_dir_all(&dir).await.map_err(io_error)?;
    let temp = Temporary(dir.join(format!(".{}.tmp", uuid::Uuid::new_v4())));
    tokio::fs::write(&temp.0, bytes).await.map_err(io_error)?;
    tokio::fs::rename(&temp.0, dir.join(&id))
        .await
        .map_err(io_error)?;
    crate::artwork_maintenance::published(s,old,bytes.len() as u64,created);
    Ok(id)
}
