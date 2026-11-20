use crate::{
    auth::{authenticate, Auth},
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use futures_util::{stream::BoxStream, StreamExt};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use std::{collections::HashMap, sync::Mutex};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
#[derive(Deserialize, Default)]
pub struct TicketQuery {
    pub ticket: Option<String>,
}
pub async fn authorize(s: &AppState, h: &HeaderMap, q: &TicketQuery, scope: &str) -> ApiResult<Option<Auth>> {
    let mut principal=None;let mut admitted=false;
    if let Some(t)=q.ticket.as_deref(){if s.check_ticket(t,scope).await{admitted=true;let owner=s.tickets.lock().await.get(&crate::auth::hash(t)).and_then(|t|t.owner.clone());if let Some(owner)=owner{principal=Some(crate::auth::authenticate_hash(s,owner).await?);}}}
    if !admitted {let token=h.get(header::AUTHORIZATION).and_then(|v|v.to_str().ok()).and_then(|v|v.strip_prefix("Bearer ")).ok_or_else(ApiError::auth)?;let a=authenticate(s,token).await?;a.require("read")?;principal=Some(a);}
    if let Some(a)=principal.as_ref(){if let Some(p)=crate::accounts::policy(s,a).await?{if let Some(id)=scope.strip_prefix("media:"){crate::accounts::scope::track(s,&p,id).await?;}if let Some(id)=scope.strip_prefix("cover:"){crate::accounts::scope::artwork(s,&p,id,true).await?;}}}
    Ok(principal)
}

#[derive(Deserialize, Default)]
pub struct IssueTicket { zone: Option<String> }

pub async fn ticket(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Query(query): Query<IssueTicket>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let track=crate::library::get_track(&s, id.clone()).await?;
    let api_path=if let Some(zone_id)=query.zone {
        let zone=s.zones.read().await.get(&zone_id).cloned().ok_or_else(ApiError::missing)?;
        if !s.bridge_owners.read().await.get(&zone.endpoint_id).is_some_and(|owner|owner==&a.session_hash)
            || zone.track.as_ref().is_none_or(|current|current.id!=id) {
            return Err(ApiError::forbidden());
        }
        crate::render_media::path(&s,&zone,&track,Some(a.session_hash)).await?
    } else {
        let token=s.ticket_for(format!("media:{id}"),86400,Some(a.session_hash)).await;
        let encoded:String=url::form_urlencoded::byte_serialize(id.as_bytes()).collect();
        format!("api/v1/media/{encoded}?ticket={token}")
    };
    Ok(Json(
        json!({"url":format!("{}/{api_path}",s.config.base_path),"api_path":api_path,"expires_at":chrono::Utc::now().timestamp()+86400}),
    ))
}
pub fn range(value: Option<&str>, total: u64) -> Result<(u64, u64, bool), ()> {
    if total == 0 {
        return Err(());
    }
    let Some(value) = value else {
        return Ok((0, total - 1, false));
    };
    let Some(v) = value.strip_prefix("bytes=") else {
        return Err(());
    };
    if v.contains(',') {
        return Err(());
    }
    let Some((start, end)) = v.split_once('-') else {
        return Err(());
    };
    if start.is_empty() {
        let length = end.parse::<u64>().map_err(|_| ())?;
        if length == 0 {
            return Err(());
        }
        return Ok((total.saturating_sub(length), total - 1, true));
    }
    let start = start.parse::<u64>().map_err(|_| ())?;
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(total - 1)
    };
    if start >= total || end < start {
        return Err(());
    }
    Ok((start, end, true))
}
fn range_error(total: u64) -> Response {
    (
        StatusCode::RANGE_NOT_SATISFIABLE,
        [(header::CONTENT_RANGE, format!("bytes */{total}"))],
    )
        .into_response()
}
pub fn mime(format: &str) -> &'static str {
    match format.to_lowercase().as_str() {
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "aiff" | "aif" => "audio/aiff",
        _ => "application/octet-stream",
    }
}
pub async fn media(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TicketQuery>,
    h: HeaderMap,
    method: Method,
) -> ApiResult<Response> {
    let principal=authorize(&s, &h, &q, &format!("media:{id}")).await?;
    let track = crate::library::get_track(&s, id.clone()).await?;
    let requested = h
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let cached_only=if let Some(a)=principal.as_ref(){crate::accounts::policy(&s,a).await?.is_some_and(|p|!p.providers.contains(&track.source))}else{false};
    let quality=if cached_only{"cached".into()}else{crate::catalog::quality(&s,&track.source).await};
    if track.source != "local" { if let Some(response)=cached_source(&s,&id,requested.as_deref(),&h,&method,&quality).await? { return Ok(response); } }
    if track.source!="local"{if let Some(a)=principal.as_ref(){if crate::accounts::policy(&s,a).await?.is_some_and(|p|!p.providers.contains(&track.source)){return Err(ApiError(StatusCode::GONE,"authorized_cache_missing".into(),"授权范围内的完整缓存已不可用，请联系管理员补齐缓存".into()));}}}
    if track.source == "local" {
        let tid = id.clone();
        let path=crate::library::local_path(&s,tid).await?;
        let mut f = tokio::fs::File::open(&path)
            .await
            .map_err(|_| ApiError::missing())?;
        let m = f.metadata().await.map_err(anyhow::Error::from)?;
        let total = m.len();
        let etag = format!(
            "\"{}-{}\"",
            total,
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|t| t.as_nanos())
                .unwrap_or(0)
        );
        if h.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(&etag)
            && requested.is_none()
        {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
        let request = if h
            .get(header::IF_RANGE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v != etag)
        {
            None
        } else {
            requested.as_deref()
        };
        let Ok((start, end, partial)) = range(request, total) else {
            return Ok(range_error(total));
        };
        f.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(anyhow::Error::from)?;
        let guard=retain_path(&s,&path).await?;
        let input=tokio_util::io::ReaderStream::with_capacity(f.take(end-start+1),64*1024);
        let stream=futures_util::stream::unfold((input,guard),|(mut input,guard)|async move{input.next().await.map(|chunk|(chunk,(input,guard)))}).boxed();
        return serve(
            s,
            id,
            track.format,
            total,
            start,
            end,
            partial,
            etag,
            stream,
            method,
            false,
        )
        .await;
    }
    remote(s,track,quality,requested,method).await
}
async fn remote(s:AppState,track:crate::models::Track,quality:String,requested:Option<String>,method:Method)->ApiResult<Response> {
    let id=track.id.clone();
    let key = format!("{id}:{quality}");
    let background = crate::traffic::background();
    let (resolved, upstream) = {
        let _permit = s.traffic.song(&key, background).await;
        crate::source_stream::open(&s,&track,&quality,requested.as_deref()).await?
    };
    if upstream.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        return Ok(StatusCode::RANGE_NOT_SATISFIABLE.into_response());
    }
    if !upstream.status().is_success() {
        return Err(ApiError(
            StatusCode::BAD_GATEWAY,
            "upstream_denied".into(),
            "音源未授权或播放地址已过期".into(),
        ));
    }
    let contentrange = upstream
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let total = contentrange
        .as_deref()
        .and_then(|v| v.rsplit('/').next()?.parse::<u64>().ok())
        .or(upstream.content_length())
        .or(resolved["expected_size"].as_u64())
        .ok_or_else(|| ApiError::bad("音源没有返回长度，无法验证缓存完整性"))?;
    let partial = upstream.status() == StatusCode::PARTIAL_CONTENT;
    let (start, end) = if partial {
        let (start,end,range_total) = contentrange.as_deref().and_then(crate::source_stream::content_range)
            .ok_or_else(||ApiError::bad("音源 Range 响应无效"))?;
        if range_total!=total||upstream.content_length().is_some_and(|n|n!=end-start+1){return Err(ApiError::bad("音源 Range 长度不一致"));}
        (start,end)
    } else {
        if requested
            .as_deref()
            .is_some_and(|r| !r.starts_with("bytes=0-"))
        {
            return Err(ApiError::bad("该音源暂不支持定位"));
        }
        if total==0{return Err(ApiError::bad("音源返回空文件"));}
        (0, total-1)
    };
    let format = resolved["format"]
        .as_str()
        .unwrap_or(&track.format)
        .to_string();
    // Distinct source entities must never share partial cache ranges. Without a strong
    // validator only this response may complete the file; separate reads stay separate.
    let entity=crate::source_stream::strong_etag(&upstream).map(|v|crate::auth::hash(&v))
        .unwrap_or_else(||uuid::Uuid::new_v4().to_string());
    let etag = format!("\"{}-{}-{}-e{}-q{}\"", id, total, format,entity,quality);
    let cache_allowed=resolved["cache_allowed"].as_bool().unwrap_or(false);
    let resumable = crate::source_stream::strong_etag(&upstream).is_some();
    let input = crate::source_stream::stream(s.clone(),track,quality,resolved,upstream,start,end,total);
    // A paused or backpressured response holds no source slot between reads.
    let stream = futures_util::stream::unfold(
        (input, s.traffic.clone(), key),
        move |(mut input, traffic, key)| async move {
            let permit = traffic.song(&key, background).await;
            let chunk = input.next().await?;
            drop(permit);
            Some((chunk, (input, traffic, key)))
        },
    ).boxed();
    let full = cache_allowed && s.setting("playback_cache_mode", json!("full")).await == "full";
    let stream = if full && method != Method::HEAD {
        preview_stream(&s, &id, &format, &etag, total, start, resumable, stream).await
    } else {
        stream
    };
    serve(
        s,
        id,
        format,
        total,
        start,
        end,
        partial,
        etag,
        stream,
        method,
        cache_allowed && !full,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn serve(
    s: AppState,
    id: String,
    format: String,
    total: u64,
    start: u64,
    end: u64,
    partial: bool,
    etag: String,
    stream: BoxStream<'static, Result<Bytes, std::io::Error>>,
    method: Method,
    cache: bool,
) -> ApiResult<Response> {
    let body = if method == Method::HEAD {
        Body::empty()
    } else if cache {
        Body::from_stream(
            caching_stream(
                s,
                id.clone(),
                format.clone(),
                etag.clone(),
                total,
                start,
                stream,
            )
            .await?,
        )
    } else {
        Body::from_stream(stream)
    };
    let mut r = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_TYPE, mime(&format))
        .header(header::CONTENT_LENGTH, end - start + 1)
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("X-Content-Type-Options", "nosniff");
    if partial {
        r = r.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total}"),
        );
    }
    Ok(r.body(body).map_err(anyhow::Error::from)?)
}
pub(crate) struct Active {
    s: AppState,
    id: String,
}
const PREVIEW_TRACK_BYTES: usize = 32 * 1024 * 1024;
const PREVIEW_TOTAL_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct PreviewCache(Mutex<HashMap<String, Preview>>);

struct Preview {
    etag: String,
    format: String,
    total: u64,
    resumable: bool,
    bytes: Vec<u8>,
    _memory: crate::resources::Permit,
}

async fn preview_stream(
    s: &AppState, track: &str, format: &str, etag: &str, total: u64, start: u64,
    resumable: bool, input: BoxStream<'static, Result<Bytes, std::io::Error>>,
) -> BoxStream<'static, Result<Bytes, std::io::Error>> {
    if !crate::cache_policy::enabled(s).await || !s.zones.read().await.values().any(|zone|
        zone.status == "playing" && zone.track.as_ref().is_some_and(|current| current.id == track)) {
        return input;
    }
    let size = total.min(PREVIEW_TRACK_BYTES as u64) as usize;
    let key = track.to_owned();
    let entity = etag.to_owned();
    let limit = crate::resources::limit(s).await;
    {
        let mut previews = s.playback_cache.0.lock().unwrap();
        if !previews.contains_key(track) && start == 0
            && previews.values().map(|preview| preview.bytes.capacity()).sum::<usize>() + size <= PREVIEW_TOTAL_BYTES
        {
            if let Some(memory) = s.resources.memory(size as u64, limit) {
                previews.insert(key.clone(), Preview {
                    etag: entity.clone(), format: format.to_owned(), total, resumable,
                    bytes: Vec::with_capacity(size), _memory: memory,
                });
            }
        }
    }
    let cache = s.playback_cache.clone();
    futures_util::stream::unfold((input, start), move |(mut input, offset)| {
        let cache = cache.clone();
        let key = key.clone();
        let entity = entity.clone();
        async move {
            let chunk = input.next().await?;
            let mut next = offset;
            if let Ok(bytes) = &chunk {
                next += bytes.len() as u64;
                if let Some(preview) = cache.0.lock().unwrap().get_mut(&key).filter(|p| p.etag == entity) {
                    let end = preview.bytes.len();
                    if offset <= end as u64 && next > end as u64 && end < preview.bytes.capacity() {
                        let skip = end - offset as usize;
                        let keep = (bytes.len() - skip).min(preview.bytes.capacity() - end);
                        preview.bytes.extend_from_slice(&bytes[skip..skip + keep]);
                    }
                }
            }
            Some((chunk, (input, next)))
        }
    }).boxed()
}

pub(crate) async fn discard_inactive_previews(s: &AppState) -> anyhow::Result<()> {
    let tracks: Vec<_> = s.playback_cache.0.lock().unwrap().keys().cloned().collect();
    if tracks.is_empty() { return Ok(()); }
    let zones = s.zones.read().await;
    let candidates = tracks.into_iter().filter(|track| !zones.values().any(|zone|
        matches!(zone.status.as_str(), "playing" | "paused") && zone.track.as_ref().is_some_and(|current| &current.id == track)
    )).collect::<Vec<_>>();
    drop(zones);
    if candidates.is_empty() { return Ok(()); }
    let keys = serde_json::to_string(&candidates)?;
    let pending = s.db.read(move |c| {
        Ok(c.prepare("SELECT CAST(wanted.value AS TEXT) FROM json_each(?1) wanted
            WHERE EXISTS(SELECT 1 FROM jobs WHERE kind='cache_fill' AND cursor=wanted.value||':listened'
              AND status IN ('pending','running','waiting','interrupted'))")?
            .query_map([keys], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await?;
    let mut previews = s.playback_cache.0.lock().unwrap();
    for track in candidates.into_iter().filter(|track| !pending.contains(track)) { previews.remove(&track); }
    Ok(())
}

async fn save_listened_preview(s: &AppState, track: &str, quality: &str) -> ApiResult<()> {
    let preview = s.playback_cache.0.lock().unwrap().remove(track);
    let Some(preview) = preview.filter(|p| !p.bytes.is_empty()
        && (p.resumable || p.bytes.len() as u64 == p.total)
        && p.etag.ends_with(&format!("-q{quality}\""))) else { return Ok(()); };
    let input = futures_util::stream::once(async { Ok(Bytes::from(preview.bytes)) }).boxed();
    let mut cached = caching_stream(s.clone(), track.to_owned(), preview.format, preview.etag, preview.total, 0, input).await?;
    while let Some(chunk) = cached.next().await {
        chunk.map_err(anyhow::Error::from)?;
    }
    Ok(())
}
pub(crate) const COMPLETE_CACHE_QUERY:&str="SELECT id,path,format,total_size,etag,content_etag FROM cache_entries WHERE track_id=?1 AND complete=1 AND (?3='cached' OR etag LIKE ?2 OR (?3='best' AND etag NOT LIKE '%-q%\"')) ORDER BY size DESC LIMIT 1";
async fn cached_source(s:&AppState,track_id:&str,requested:Option<&str>,headers:&HeaderMap,method:&Method,quality:&str)->ApiResult<Option<Response>> {
    let _lock=s.cache_lock.lock().await;let tid=track_id.to_string();let suffix=format!("%-q{quality}\"");let quality=quality.to_owned();
    let cached=s.db.run(move|c|Ok(c.query_row(COMPLETE_CACHE_QUERY,params![tid,suffix,quality],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)? as u64,r.get::<_,Option<String>>(5)?.or(r.get::<_,Option<String>>(4)?)))).optional()?)).await?;
    let Some((id,path,format,total,etag))=cached else{return Ok(None);};let Ok(mut file)=tokio::fs::File::open(&path).await else{return Ok(None);};if file.metadata().await.map_err(anyhow::Error::from)?.len()!=total{return Ok(None);}
    if requested.is_none()&&etag.as_deref().is_some_and(|etag|headers.get(header::IF_NONE_MATCH).and_then(|v|v.to_str().ok())==Some(etag)){return Ok(Some(StatusCode::NOT_MODIFIED.into_response()));}
    let requested=if headers.get(header::IF_RANGE).and_then(|v|v.to_str().ok()).is_some_and(|value|Some(value)!=etag.as_deref()){None}else{requested};
    let Ok((start,end,partial))=range(requested,total)else{return Ok(Some(range_error(total)));};file.seek(std::io::SeekFrom::Start(start)).await.map_err(anyhow::Error::from)?;
    *s.cache_active.lock().await.entry(id.clone()).or_default()+=1;let guard=Active{s:s.clone(),id:id.clone()};let input=tokio_util::io::ReaderStream::with_capacity(file.take(end-start+1),64*1024);
    let stream=futures_util::stream::unfold((input,guard),|(mut input,guard)|async move{input.next().await.map(|chunk|(chunk,(input,guard)))}).boxed();
    s.db.run(move|c|{c.execute("UPDATE cache_entries SET last_access=?1 WHERE id=?2",params![chrono::Utc::now().timestamp(),id])?;Ok(())}).await?;
    let mut response=serve(s.clone(),track_id.into(),format,total,start,end,partial,etag.unwrap_or_default(),stream,method.clone(),false).await?;response.headers_mut().insert("X-RMusic-Cache","complete".parse().unwrap());Ok(Some(response))
}
pub(crate) async fn retain_path(s:&AppState,path:&str)->anyhow::Result<Option<Active>> {
    let _lock=s.cache_lock.lock().await;let path=path.to_owned();
    let id=s.db.run(move|c|Ok(c.query_row("SELECT id FROM cache_entries WHERE path=?1 AND complete=1",[path],|r|r.get::<_,String>(0)).optional()?)).await?;
    if let Some(id)=id{*s.cache_active.lock().await.entry(id.clone()).or_default()+=1;Ok(Some(Active{s:s.clone(),id}))}else{Ok(None)}
}
impl Drop for Active {
    fn drop(&mut self) {
        let s = self.s.clone();
        let id = self.id.clone();
        tokio::spawn(async move {
            let _=flush_buffer(&s,&id).await;
            let mut a = s.cache_active.lock().await;
            if let Some(n) = a.get_mut(&id) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    a.remove(&id);drop(a);
                    let _=s.db.run(move|c|{c.execute("UPDATE cache_entries SET allocated_size=0 WHERE id=?1 AND complete=0 AND size=0",[id])?;Ok(())}).await;
                }
            }
        });
    }
}
pub(crate) async fn caching_stream(
    s: AppState,
    track_id: String,
    format: String,
    etag: String,
    total: u64,
    start: u64,
    input: BoxStream<'static, Result<Bytes, std::io::Error>>,
) -> ApiResult<BoxStream<'static, Result<Bytes, std::io::Error>>> {
    if !crate::cache_policy::enabled(&s).await { return Ok(input); }
    let _lock=s.cache_lock.lock().await;
    let id = crate::auth::hash(&format!("{track_id}:{etag}"));
    let path = s.cache_dir().await.join(&id);
    let cid = id.clone();
    let p = path.to_string_lossy().to_string();
    s.db.run(move|c|{c.execute("INSERT INTO cache_entries(id,track_id,format,total_size,last_access,path,etag,single_file) VALUES(?1,?2,?3,?4,?5,?6,?7,1) ON CONFLICT(id) DO UPDATE SET last_access=excluded.last_access",params![cid,track_id,format,total as i64,chrono::Utc::now().timestamp(),p,etag])?;Ok(())}).await?;
    let cid=id.clone();let (path,canonical)=s.db.run(move|c|Ok(c.query_row("SELECT path,content_etag IS NOT NULL FROM cache_entries WHERE id=?1",[cid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?)))?)).await?;
    // A request resolved before publication can arrive late with raw source ranges.
    // Serve that stream unchanged without writing source offsets into the tagged entity.
    if canonical{return Ok(input);}
    let path=std::path::PathBuf::from(path);
    *s.cache_active.lock().await.entry(id.clone()).or_default() += 1;
    let guard = Active {
        s: s.clone(),
        id: id.clone(),
    };
    drop(_lock);
    Ok(futures_util::stream::unfold(
        (input, s, id, path, start, guard),
        |(mut input, s, id, path, mut offset, guard)| async move {
            let Some(chunk)=input.next().await else {let _=flush_buffer(&s,&id).await;return None;};
            if let Ok(bytes) = &chunk {
                if let Err(e) = buffer_chunk(&s, &id, &path, offset, bytes).await {
                    tracing::warn!(error=%e,"cache write skipped");
                }
                offset += bytes.len() as u64;
            }
            if chunk.is_err(){let _=flush_buffer(&s,&id).await;}
            Some((chunk, (input, s, id, path, offset, guard)))
        },
    )
    .boxed())
}
// One shared allocation per source entity. Small responses are committed once;
// long responses flush bounded blocks, and dropped readers flush their committed tail.
const WRITE_BLOCK:usize=1024*1024;
pub(crate) struct WriteBuffer {path:std::path::PathBuf,offset:u64,bytes:Vec<u8>,_permit:crate::resources::Permit}
struct Temporary {path:std::path::PathBuf,root:std::path::PathBuf}
impl Drop for Temporary {fn drop(&mut self){if self.path.is_absolute()&&self.path.parent()==Some(self.root.as_path())&&self.path.file_name().and_then(|n|n.to_str()).is_some_and(|n|n.starts_with(".rmusic-incoming-")){let _=std::fs::remove_file(&self.path);}}}
async fn flush_buffer(s:&AppState,id:&str)->anyhow::Result<()> {
    let buffer=s.cache_buffers.lock().await.remove(id);
    if let Some(buffer)=buffer{if !buffer.bytes.is_empty()&&crate::cache_policy::enabled(s).await{cache_chunk(s,id,&buffer.path,buffer.offset,&Bytes::from(buffer.bytes)).await?;}}
    Ok(())
}
async fn buffer_chunk(s:&AppState,id:&str,path:&std::path::Path,offset:u64,bytes:&[u8])->anyhow::Result<()> {
    if !crate::cache_policy::enabled(s).await{s.cache_buffers.lock().await.remove(id);return Ok(());}
    let limit=crate::resources::limit(s).await;
    for (index,chunk) in bytes.chunks(WRITE_BLOCK).enumerate(){
        let start=offset.checked_add((index*WRITE_BLOCK) as u64).ok_or_else(||anyhow::anyhow!("cache offset overflow"))?;
        loop {
            let mut buffers=s.cache_buffers.lock().await;
            if let Some(buffer)=buffers.get(id){if start>=buffer.offset&&start.saturating_add(chunk.len() as u64)<=buffer.offset+buffer.bytes.len() as u64{
                let from=(start-buffer.offset) as usize;anyhow::ensure!(&buffer.bytes[from..from+chunk.len()]==chunk,"相同音源实体返回不一致的重复范围");break;
            }}
            let discontinuous=buffers.get(id).is_some_and(|b|b.offset+b.bytes.len() as u64!=start||b.bytes.len()+chunk.len()>WRITE_BLOCK);
            if discontinuous {let old=buffers.remove(id).unwrap();drop(buffers);cache_chunk(s,id,&old.path,old.offset,&Bytes::from(old.bytes)).await?;continue;}
            if !buffers.contains_key(id){let key=id.to_owned();let total=s.db.run(move|c|Ok(c.query_row("SELECT total_size FROM cache_entries WHERE id=?1",[key],|r|r.get::<_,i64>(0))?)).await?.max(0) as usize;
                let capacity=WRITE_BLOCK.min(total).max(chunk.len());let Some(permit)=s.resources.memory(capacity as u64,limit)else{return Ok(());};buffers.insert(id.into(),WriteBuffer{path:path.into(),offset:start,bytes:Vec::with_capacity(capacity),_permit:permit});}
            let buffer=buffers.get_mut(id).unwrap();buffer.bytes.extend_from_slice(chunk);
            if buffer.bytes.len()==WRITE_BLOCK {let full=buffers.remove(id).unwrap();drop(buffers);cache_chunk(s,id,&full.path,full.offset,&Bytes::from(full.bytes)).await?;}
            break;
        }
    }Ok(())
}
pub(crate) async fn flush_buffers(s:&AppState)->anyhow::Result<()> {s.cache_buffers.lock().await.clear();Ok(())}
pub(crate) async fn cleanup_incoming(s:&AppState)->anyhow::Result<()> {
    let cutoff=std::time::SystemTime::now();let mut roots=s.db.run(|c|Ok(c.prepare("SELECT DISTINCT path FROM cache_entries WHERE complete=0 AND size=0")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?.into_iter().filter_map(|p|std::path::PathBuf::from(p).parent().map(std::path::Path::to_path_buf)).collect::<std::collections::HashSet<_>>();roots.insert(s.cache_dir().await);
    for root in roots{let Ok(root)=tokio::fs::canonicalize(root).await else{continue;};let mut files=tokio::fs::read_dir(&root).await?;while let Some(file)=files.next_entry().await?{let name=file.file_name();let name=name.to_string_lossy();if !name.strip_prefix(".rmusic-incoming-").is_some_and(|id|uuid::Uuid::parse_str(id).is_ok()){continue;}let metadata=file.metadata().await?;if !file.file_type().await?.is_file()||metadata.modified().is_ok_and(|time|time>=cutoff){continue;}let resolved=tokio::fs::canonicalize(file.path()).await?;if resolved.parent()==Some(root.as_path()){tokio::fs::remove_file(resolved).await?;}}}
    let entries=s.db.run(|c|Ok(c.prepare("SELECT id,path FROM cache_entries WHERE complete=0 AND size=0")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    for(id,path)in entries{let bytes=tokio::fs::metadata(path).await.map_or(0,|m|m.len());s.db.run(move|c|{c.execute("UPDATE cache_entries SET allocated_size=?2 WHERE id=?1 AND complete=0 AND size=0",params![id,bytes as i64])?;Ok(())}).await?;}Ok(())
}
pub(crate) async fn fill_cache(s:&AppState,job:&str,payload:&Value)->anyhow::Result<bool> {
    if !crate::cache_policy::enabled(s).await {crate::cache_policy::pause(s,job,"Core 音频缓存已关闭，未拉取音源").await?;return Ok(false);}
    let track=crate::library::get_track(s,payload["track_id"].as_str().unwrap_or("").into()).await.map_err(|e|anyhow::anyhow!(e.2))?;
    if !crate::cache_policy::guard_fill(s,job,payload).await? {return Ok(false);}
    let memory_track=track.id.clone();
    if track.source=="local"{return Ok(true);}
    let listened = payload["origin"] == "listened";
    if listened && s.setting("playback_cache_mode", json!("full")).await != "full" {
        return Ok(true);
    }
    let quality=if listened { crate::catalog::quality(s, &track.source).await } else { payload["quality"].as_str().unwrap_or("best").to_owned() };
    if listened {
        save_listened_preview(s, &track.id, &quality).await.map_err(|e| anyhow::anyhow!(e.2))?;
    }
    let tid=track.id.clone();let suffix=format!("%-q{quality}\"");
    if payload["upgrade"]!=true&&s.db.run(move|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM cache_entries WHERE track_id=?1 AND complete=1 AND etag LIKE ?2)",params![tid,suffix],|r|r.get::<_,bool>(0))?)).await?{return Ok(true);}
    if payload["upgrade"]==true{
        let resolved=crate::catalog::resolve_quality(s,&track,&quality).await.map_err(|e|anyhow::anyhow!(e.2))?;
        let Some(candidate)=rmusic_audio::probe::verified_pcm(&resolved)else{return Ok(true);};if resolved["enhanced"]==true||resolved["upscaled"]==true{return Ok(true);}
        let recording=track.recording_id.clone();let channel=candidate.channels as u32;
        let best=s.db.run(move|c|{let rows=c.prepare("SELECT * FROM tracks WHERE source='local' AND available=1 AND recording_id=?1 AND channels=?2 AND json_type(provider_data,'$.cached_from.cache_id')='text'")?.query_map(params![recording,channel],crate::models::Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(rows.into_iter().map(|t|rmusic_audio::probe::quality_rank(&t.format,t.sample_rate,u16::try_from(t.bit_depth).unwrap_or(0),t.bitrate)).max().unwrap_or(0))}).await?;
        if candidate.quality_rank<=best{return Ok(true);}
    }
    let tid=track.id.clone();let readers=s.db.run(move|c|Ok(c.prepare("SELECT id FROM cache_entries WHERE track_id=?1 AND complete=0")?.query_map([tid],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    {let active=s.cache_active.lock().await;if readers.iter().any(|id|active.contains_key(id)){return Ok(false);}}
    let _song=s.traffic.song(&format!("{}:{quality}",track.id),true).await;
    let (tid,qsuffix)=(track.id.clone(),format!("%-q{quality}\""));
    let previous=s.db.run(move|c|Ok(c.query_row("SELECT id,path,ranges,etag FROM cache_entries WHERE track_id=?1 AND complete=0 AND size>0 AND etag LIKE ?2 ORDER BY last_access DESC LIMIT 1",params![tid,qsuffix],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?)).await?;
    let previous=previous.and_then(|(id,path,ranges,tag)|{let ranges:Vec<(u64,u64)>=serde_json::from_str(&ranges).ok()?;let prefix=ranges.first().filter(|(a,b)|*a==0&&*b>0)?.1;if std::fs::metadata(&path).ok()?.len()<prefix{return None;}Some((id,path,tag,prefix))});
    let mut prefix=previous.as_ref().map_or(0,|p|p.3);let range=(prefix>0).then(||format!("bytes={prefix}-"));
    let (resolved,response)=crate::source_stream::open(s,&track,&quality,range.as_deref()).await.map_err(|e|anyhow::anyhow!(e.2))?;
    if resolved["cache_allowed"]!=true{anyhow::bail!("音源不允许持久缓存，播放可继续");}
    let total=if response.status()==StatusCode::PARTIAL_CONTENT&&prefix>0 {
        let (start,end,total)=response.headers().get(header::CONTENT_RANGE).and_then(|v|v.to_str().ok()).and_then(crate::source_stream::content_range).ok_or_else(||anyhow::anyhow!("缓存续传缺少有效范围"))?;
        anyhow::ensure!(start==prefix&&end+1==total&&response.content_length().is_none_or(|n|n==total-prefix),"缓存续传范围不匹配");total
    }else{anyhow::ensure!(response.status()==StatusCode::OK,"完整缓存需要有效的文件响应");prefix=0;response.content_length().or(resolved["expected_size"].as_u64()).filter(|n|*n>0).ok_or_else(||anyhow::anyhow!("音源没有返回完整文件长度"))?};
    let format=resolved["format"].as_str().unwrap_or(&track.format).to_owned();
    let entity=crate::source_stream::strong_etag(&response).map(|v|crate::auth::hash(&v)).unwrap_or_else(||uuid::Uuid::new_v4().to_string());
    let etag=format!("\"{}-{}-{}-e{}-q{}\"",track.id,total,format,entity,quality);let id=crate::auth::hash(&format!("{}:{etag}",track.id));
    if prefix>0{anyhow::ensure!(previous.as_ref().is_some_and(|old|old.0==id&&old.2==etag),"音源实体或音质已改变，拒绝混合缓存范围");}
    // Existing entities keep their original directory when the default cache root changes.
    let key=id.clone();let existing=s.db.run(move|c|Ok(c.query_row("SELECT path FROM cache_entries WHERE id=?1",[key],|r|r.get::<_,String>(0)).optional()?)).await?;
    let path=existing.map(std::path::PathBuf::from).unwrap_or(s.cache_dir().await.join(&id));
    let root=path.parent().ok_or_else(||anyhow::anyhow!("缓存路径缺少目录"))?.to_path_buf();
    let previous_bytes=std::fs::metadata(&path).map_or(0,|m|m.len());let allocation=total.checked_add(previous_bytes).ok_or_else(||anyhow::anyhow!("缓存空间溢出"))?;
    {let _lock=s.cache_lock.lock().await;if !ensure_capacity(s,total).await?{return Ok(false);}
        let (key,tid,fmt,p,tag)=(id.clone(),track.id.clone(),format.clone(),path.to_string_lossy().into_owned(),etag.clone());
        s.db.run(move|c|{c.execute("INSERT INTO cache_entries(id,track_id,format,total_size,last_access,path,etag,single_file,allocated_size) VALUES(?1,?2,?3,?4,unixepoch(),?5,?6,1,?7) ON CONFLICT(id) DO UPDATE SET allocated_size=MAX(allocated_size,excluded.allocated_size)",params![key,tid,fmt,total as i64,p,tag,allocation as i64])?;Ok(())}).await?;
        *s.cache_active.lock().await.entry(id.clone()).or_default()+=1;
    }
    let _active=Active{s:s.clone(),id:id.clone()};
    let temporary=Temporary{path:root.join(format!(".rmusic-incoming-{}",uuid::Uuid::new_v4())),root:root.clone()};
    let file=tokio::fs::OpenOptions::new().create_new(true).write(true).open(&temporary.path).await?;
    let mut output=tokio::io::BufWriter::with_capacity(WRITE_BLOCK,file);let mut done=prefix;let mut reported=prefix;
    if prefix>0{let file=tokio::fs::File::open(&previous.as_ref().unwrap().1).await?;let copied=tokio::io::copy(&mut file.take(prefix),&mut output).await?;anyhow::ensure!(copied==prefix,"缓存前缀读取不完整");}
    let provenance=json!({"enhanced":resolved["enhanced"]==true||resolved["upscaled"]==true,"quality":resolved["quality"],"verified":resolved["verified"]});
    let mut input=crate::source_stream::stream(s.clone(),track,quality,resolved,response,prefix,total-1,total);
    while let Some(chunk)=input.next().await{let chunk=chunk?;output.write_all(&chunk).await?;done+=chunk.len() as u64;
        if done-reported>=WRITE_BLOCK as u64{output.flush().await?;reported=done;if !crate::background::progress(s,job,"保存完整歌曲",done,total).await?{return Ok(false);}}
        if !crate::background::disposition(s,job).await?||s.traffic.playback_waiting(){return Ok(false);}
    }
    if done!=total{anyhow::bail!("完整歌曲长度不匹配，未发布文件");}output.flush().await?;let file=output.into_inner();file.sync_all().await?;drop(file);
    let _lock=s.cache_lock.lock().await;
    if temporary.path.parent()!=Some(root.as_path())||path.parent()!=Some(root.as_path()){anyhow::bail!("完整歌曲输出超出缓存目录");}
    tokio::fs::rename(&temporary.path,&path).await?;
    let key=id.clone();s.db.run(move|c|{let tx=c.transaction()?;tx.execute("UPDATE cache_entries SET size=?2,total_size=?2,allocated_size=?2,ranges=?3,complete=1,last_access=unixepoch() WHERE id=?1",params![key,total as i64,json!([[0,total]]).to_string()])?;tx.execute("INSERT INTO cache_quality(cache_id,data)VALUES(?1,?2)ON CONFLICT(cache_id)DO UPDATE SET data=excluded.data",params![key,provenance.to_string()])?;invalidate_analysis(&tx,&key)?;tx.commit()?;Ok(())}).await?;
    s.loudness.wake.notify_one();
    let keys=s.db.run(move|c|Ok(c.prepare("SELECT id FROM cache_entries WHERE track_id=?1")?.query_map([memory_track],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;let mut buffers=s.cache_buffers.lock().await;for key in keys{buffers.remove(&key);}s.emit("cache",json!({}));Ok(true)
}
pub fn merge_ranges(mut ranges: Vec<(u64, u64)>, new: (u64, u64)) -> Vec<(u64, u64)> {
    ranges.push(new);
    ranges.sort_unstable();
    let mut out: Vec<(u64, u64)> = vec![];
    for (a, b) in ranges {
        if let Some(last) = out.last_mut() {
            if a <= last.1 {
                last.1 = last.1.max(b);
                continue;
            }
        }
        out.push((a, b));
    }
    out
}
fn invalidate_analysis(c:&rusqlite::Connection,cid:&str)->anyhow::Result<()> {
let changed="SELECT t.id FROM tracks t JOIN cache_entries e ON e.track_id=t.id WHERE e.id=?1 AND t.source!='local' AND json_extract(t.provider_data,'$.provenance.integrated_lufs.source')='ffmpeg_loudnorm_ebu_r128' AND COALESCE(json_extract(t.provider_data,'$.analysis.loudness.cache_id'),'')!=?1 AND COALESCE(json_extract(t.provider_data,'$.integrated_lufs_source'),'')!='manual' AND json_type(t.provider_data,'$.manual_corrections.integrated_lufs') IS NULL";
c.execute(&format!("DELETE FROM jobs WHERE kind='loudness' AND status IN ('completed','unable','failed') AND cursor IN ({changed})"),[cid])?;
c.execute(&format!("UPDATE tracks SET provider_data=json_remove(provider_data,'$.integrated_lufs','$.analysis.loudness','$.provenance.integrated_lufs') WHERE id IN ({changed})"),[cid])?;
c.execute("DELETE FROM jobs WHERE kind='loudness' AND status IN ('completed','unable','failed')
 AND cursor IN (SELECT t.id FROM tracks t JOIN cache_entries e ON e.track_id=t.id
 WHERE e.id=?1 AND t.source!='local' AND json_extract(t.provider_data,'$.integrated_lufs') IS NULL)",[cid])?;
Ok(())
}
async fn cache_chunk(
    s: &AppState,
    id: &str,
    path: &std::path::Path,
    offset: u64,
    bytes: &Bytes,
) -> anyhow::Result<()> {
    let _lock = s.cache_lock.lock().await;
    let cid = id.to_string();
    let bytes_len = bytes.len() as u64;
    let (old, total,allocated,canonical) =
        s.db.run(move |c| {
            Ok(c.query_row(
                "SELECT ranges,total_size,MAX(size,allocated_size),content_etag IS NOT NULL FROM cache_entries WHERE id=?1",
                [cid],
                |r| {
                    Ok((
                        serde_json::from_str::<Vec<(u64, u64)>>(&r.get::<_, String>(0)?)
                            .unwrap_or_default(),
                        r.get::<_, i64>(1).map(|n| n as u64)?,
                        r.get::<_,i64>(2)? as u64,r.get::<_,bool>(3)?,
                    ))
                },
            )?)
        })
        .await?;
    if canonical{return Ok(());}
    anyhow::ensure!(bytes_len>0&&offset.checked_add(bytes_len).is_some_and(|end|end<=total),"cache write outside source entity");
    let merged = merge_ranges(old, (offset, (offset + bytes_len).min(total)));
    let size = merged.iter().map(|(a, b)| b - a).sum::<u64>();
    // Count the file's high-water length as well as received bytes, including seek holes.
    let stored=allocated.max(offset.saturating_add(bytes_len));
    let added = stored.saturating_sub(allocated);
    if !ensure_capacity(s, added).await? {
        return Ok(());
    }
    // Range coverage is published only after the bytes are durably written. The raw entity
    // remains private/incomplete until every source range is present; late canonical writes stop above.
    let mut file=tokio::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;file.write_all(bytes).await?;file.sync_data().await?;
    let complete = merged == vec![(0, total)];
    let cid = id.to_string();
    s.db.run(move |c| {
        c.execute(
            "UPDATE cache_entries SET ranges=?1,size=?2,complete=?3,last_access=?4,allocated_size=?6 WHERE id=?5",
            params![
                serde_json::to_string(&merged)?,
                size as i64,
                complete,
                chrono::Utc::now().timestamp(),
                cid,
                stored as i64
            ],
        )?;
        if complete {
            invalidate_analysis(c,&cid)?;
        }
        Ok(())
    })
    .await?;
    if complete { s.loudness.wake.notify_one(); }
    Ok(())
}
pub async fn ensure_capacity(s: &AppState, additional: u64) -> anyhow::Result<bool> {
    crate::cache_budget::fits(s, additional).await
}

pub(crate) async fn remove_export_directory(id:&str,audio:&str)->anyhow::Result<()> {
    if id.is_empty()||!id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'||b==b'_'){anyhow::bail!("invalid portable cache id");}
    let path=std::path::Path::new(audio);
    let directory=path.parent().ok_or_else(||anyhow::anyhow!("portable cache parent missing"))?;
    let expected=directory.file_name().and_then(|s|s.to_str()).is_some_and(|name|name==id||name==format!("{id}.staging"));
    let root=directory.parent().ok_or_else(||anyhow::anyhow!("portable root missing"))?;
    if !directory.is_absolute()||!expected||!root.is_absolute(){anyhow::bail!("portable cache directory escaped expected layout");}
    match tokio::fs::symlink_metadata(directory).await {
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(()),
        Err(e)=>return Err(e.into()),
        Ok(m) if m.file_type().is_symlink()=>anyhow::bail!("portable cache directory cannot be a symlink"),
        _=>{}
    }
    let resolved_root=tokio::fs::canonicalize(root).await?;
    let resolved_directory=tokio::fs::canonicalize(directory).await?;
    if resolved_directory.parent()!=Some(resolved_root.as_path())||resolved_directory==resolved_root{anyhow::bail!("portable cache resolved outside expected root");}
    tokio::fs::remove_dir_all(resolved_directory).await?;
    Ok(())
}
pub async fn cache_export(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("read")?;crate::cache_tags::queue_export(&s,&id).await?;
    Ok(Json(json!({"status":"pending"})))
}
#[derive(Deserialize)]pub struct CacheList {#[serde(default)]include_partial:bool,limit:Option<usize>,#[serde(default)]offset:usize}
pub async fn cache_status(State(s): State<AppState>, a: Auth,Query(query):Query<CacheList>) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let limit=query.limit.unwrap_or(1000);if !(1..=1000).contains(&limit)||query.offset>1000000{return Err(ApiError::bad("缓存列表分页范围无效"));}let limit=limit as i64;let offset=query.offset as i64;
    let v=s.db.run(move|c|{let protected=crate::playlists::protected_tracks(c)?;let mut q=c.prepare("SELECT e.id,e.track_id,e.format,e.size,e.complete,e.pinned,e.last_access,e.total_size,t.title,t.artist,COALESCE(stats.play_count,0),x.status,x.path,x.size,x.error,e.content_etag IS NOT NULL FROM cache_entries e LEFT JOIN tracks t ON t.id=e.track_id LEFT JOIN track_stats stats ON stats.track_id=e.track_id LEFT JOIN cache_exports x ON x.cache_id=e.id WHERE (e.complete=1 OR ?1) ORDER BY e.last_access DESC,e.id LIMIT ?2 OFFSET ?3")?;let entries=q.query_map(params![query.include_partial,limit,offset],|r|Ok(json!({"id":r.get::<_,String>(0)?,"track_id":r.get::<_,String>(1)?,"playlist_protected":protected.contains(&r.get::<_,String>(1)?),"canonical":r.get::<_,bool>(15)?,"format":r.get::<_,String>(2)?,"size":r.get::<_,i64>(3).map(|n|n as u64)?,"complete":r.get::<_,bool>(4)?,"pinned":r.get::<_,bool>(5)?,"last_access":r.get::<_,i64>(6)?,"total_size":r.get::<_,i64>(7).map(|n|n as u64)?,"title":r.get::<_,Option<String>>(8)?,"artist":r.get::<_,Option<String>>(9)?,"play_count":r.get::<_,i64>(10)?,"exports":r.get::<_,Option<String>>(11)?.map(|status|json!({"status":status,"path":r.get::<_,Option<String>>(12).ok().flatten(),"size":r.get::<_,i64>(13).unwrap_or(0),"error":r.get::<_,Option<String>>(14).ok().flatten()}))})))?.collect::<Result<Vec<_>,_>>()?;let used=c.query_row("SELECT COALESCE(SUM(MAX(size,allocated_size)),0) FROM cache_entries",[],|r|r.get::<_,i64>(0).map(|n|n as u64))?;let portable=c.query_row("SELECT COALESCE(SUM(size),0) FROM cache_exports",[],|r|r.get::<_,i64>(0).map(|n|n as u64))?;let retired=c.query_row("SELECT COALESCE(SUM(size),0) FROM cache_retired",[],|r|r.get::<_,i64>(0).map(|n|n as u64))?;Ok(json!({"total":c.query_row("SELECT COUNT(*) FROM cache_entries WHERE complete=1 OR ?1",[query.include_partial],|r|r.get::<_,i64>(0))?,"entries":entries,"used_bytes":used+portable+retired+c.query_row("SELECT COALESCE(SUM(size),0) FROM file_bin WHERE storage='cache' AND status IN ('planned','trashed','restoring')",[],|r|r.get::<_,i64>(0).map(|n|n as u64))?,"portable_bytes":portable,"retired_bytes":retired}))}).await?;
    let mut v = v;
    v["limit_bytes"] = s
        .setting("cache_limit_bytes", json!(2_u64 * 1024 * 1024 * 1024))
        .await;
    v["enabled"]=json!(crate::cache_policy::enabled(&s).await);
    v["portable_directory"]=json!(s.portable_dir().await);v["temporary_storage"]=json!("bounded_memory_and_private_ranges");v["directory"]=json!(s.cache_dir().await);v["eviction"]=json!("manual_only");v["capacity_blocked"]=json!(v["used_bytes"].as_u64().unwrap_or(0)>=v["limit_bytes"].as_u64().unwrap_or(0));
    Ok(Json(v))
}
#[derive(Deserialize)]
pub struct CacheLimit {
    limit_bytes: Option<u64>,
    directory:Option<String>,
    portable_directory:Option<String>,
}
pub async fn cache_config(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<CacheLimit>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if v.limit_bytes.is_some_and(|l|l>1024_u64.pow(5)){return Err(ApiError::bad("缓存上限超过 1 PiB"));}
    let _lock = s.cache_lock.lock().await;
    if let Some(limit)=v.limit_bytes { s.db.run(move|c|Ok(crate::cache_budget::validate(c,limit))).await??; }
    if let Some(directory)=v.directory{
        let path=std::path::PathBuf::from(directory.trim());if !path.is_absolute()||path==s.config.data||path==s.config.plugin_dir{return Err(ApiError::bad("缓存目录必须是独立的绝对路径；Docker 内需先挂载可写目录"));}
        tokio::fs::create_dir_all(&path).await.map_err(|_|ApiError::bad("无法创建缓存目录，请检查挂载和写入权限"))?;let path=tokio::fs::canonicalize(path).await.map_err(anyhow::Error::from)?;
        let probe=path.join(format!(".rmusic-write-probe-{}",uuid::Uuid::new_v4()));tokio::fs::write(&probe,[]).await.map_err(|_|ApiError::bad("缓存目录不可写"))?;tokio::fs::remove_file(probe).await.map_err(anyhow::Error::from)?;
        if path==s.config.data||path==s.config.plugin_dir{return Err(ApiError::bad("请选择独立缓存目录"));}
        let value=serde_json::to_string(&path.to_string_lossy()).map_err(anyhow::Error::from)?;s.db.run(move|c|{c.execute("INSERT INTO settings(key,value) VALUES('cache_directory',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[value])?;Ok(())}).await?;*s.cache_directory.write().await=path;
    }
    if let Some(directory)=v.portable_directory{
        let path=std::path::PathBuf::from(directory.trim());if !path.is_absolute(){return Err(ApiError::bad("完整歌曲输出目录必须为绝对路径"));}
        tokio::fs::create_dir_all(&path).await.map_err(|_|ApiError::bad("无法创建完整歌曲输出目录"))?;let path=tokio::fs::canonicalize(path).await.map_err(anyhow::Error::from)?;
        if path==s.cache_dir().await||path==s.config.data||path==s.config.plugin_dir{return Err(ApiError::bad("完整歌曲输出目录应与核心数据、插件及缓存根目录分开"));}
        s.db.run(move|c|{c.execute("INSERT INTO settings(key,value) VALUES('portable_directory',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[json!(path).to_string()])?;Ok(())}).await?;
    }
    if let Some(l)=v.limit_bytes{if l>1024_u64.pow(5){return Err(ApiError::bad("缓存上限超过 1 PiB"));}s.db.run(move|c|{c.execute("INSERT INTO settings(key,value) VALUES('cache_limit_bytes',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[l.to_string()])?;Ok(())}).await?;}
    ensure_capacity(&s, 0).await?;
    s.emit("cache", json!({}));
    drop(_lock);cache_status(State(s),a,Query(CacheList{include_partial:false,limit:None,offset:0})).await
}
pub async fn cache_delete(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    a.require("admin")?;
    let _lock = s.cache_lock.lock().await;
    if s.cache_active.lock().await.contains_key(&id) {
        return Err(ApiError::bad("歌曲正在播放，暂不能删除"));
    }
    let cid = id.clone();
    let (path,track_id) =
        s.db.run(move |c| {
            Ok(
                c.query_row("SELECT path,track_id FROM cache_entries WHERE id=?1", [cid], |r| {
                    Ok((r.get::<_, String>(0)?,r.get::<_,String>(1)?))
                })
                .optional()?,
            )
        })
        .await?
        .ok_or_else(ApiError::missing)?;
    if s.zones.read().await.values().any(|z|z.status=="playing"&&z.track.as_ref().is_some_and(|t|t.id==track_id)){return Err(ApiError::bad("歌曲正在播放，暂不能删除"));}
    let cid=id.clone();let export=s.db.run(move|c|Ok(c.query_row("SELECT path FROM cache_exports WHERE cache_id=?1",[cid],|r|r.get::<_,Option<String>>(0)).optional()?.flatten())).await?;if let Some(path)=export{remove_export_directory(&id,&path).await?;}
    match tokio::fs::remove_file(path).await{Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(anyhow::Error::from(e).into())}
    s.db.run(move |c| {
        c.execute("UPDATE tracks SET available=0 WHERE source='local' AND path IN(SELECT path FROM cache_entries WHERE id=?1)",[&id])?;
        c.execute("DELETE FROM cache_entries WHERE id=?1", [id])?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn cover(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<TicketQuery>,
    h: HeaderMap,
) -> ApiResult<Response> {
    authorize(&s, &h, &q, &format!("cover:{id}")).await?;
    if !crate::artwork_files::valid_id(&id){return Err(ApiError::missing());}
    let file=crate::artwork_recovery::cover(&s,&id).await?;
    if file.id!=id {
        let mut location=format!("{}/api/v1/covers/{}",s.config.base_path,file.id);
        if let Some(ticket)=q.ticket.as_deref(){
            let previous=s.tickets.lock().await.get(&crate::auth::hash(ticket)).cloned();
            if let Some(previous)=previous{let seconds=(previous.expires-chrono::Utc::now().timestamp()).clamp(1,60);let ticket=s.ticket_for(format!("cover:{}",file.id),seconds,previous.owner).await;location.push_str(&format!("?ticket={ticket}"));}
        }
        return Ok((StatusCode::TEMPORARY_REDIRECT,[(header::LOCATION,location),(header::CACHE_CONTROL,"private, no-store".into())]).into_response());
    }
    file.response(&h,true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_cases() {
        assert_eq!(range(Some("bytes=-3"), 10), Ok((7, 9, true)));
        assert_eq!(range(Some("bytes=3-"), 10), Ok((3, 9, true)));
        assert!(range(Some("bytes=10-"), 10).is_err());
        assert!(range(Some("bytes=0-1,3-4"), 10).is_err());
    }
    #[test]
    fn ranges_never_complete_missing_middle() {
        assert_eq!(
            merge_ranges(vec![(0, 10), (20, 30)], (5, 15)),
            vec![(0, 15), (20, 30)]
        );
        assert_eq!(
            merge_ranges(vec![(0, 10), (20, 30)], (10, 20)),
            vec![(0, 30)]
        );
    }
}
