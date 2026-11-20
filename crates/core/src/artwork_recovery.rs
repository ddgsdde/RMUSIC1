//! Shared source recovery, durable revisions and bounded negative caching.
use crate::{
    artwork_files::{self, ImageFile},
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::http::StatusCode;
use rusqlite::{params, OptionalExtension};
use std::sync::LazyLock;
static SLOTS: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(10));
pub fn bind(c: &rusqlite::Connection, kind: &str, entity: &str, url: &str) -> anyhow::Result<()> {
    let source = if let Some(id) = url.strip_prefix("/api/v1/artwork/") {
        Some(id.to_owned())
    } else if let Some(id) = url.strip_prefix("/api/v1/covers/") {
        c.query_row(
            "SELECT id FROM artwork_sources WHERE cover_id=?1 ORDER BY fetched_at DESC LIMIT 1",
            [id],
            |r| r.get(0),
        )
        .optional()?
    } else {
        None
    };
    if let Some(id) = source {
        c.execute("INSERT INTO artwork_entities(kind,entity_id,source_id)VALUES(?1,?2,?3) ON CONFLICT(kind,entity_id)DO UPDATE SET source_id=excluded.source_id",params![kind,entity,id])?;
    }
    Ok(())
}
async fn cached(s: &AppState, id: Option<&str>) -> ApiResult<Option<ImageFile>> {
    if let Some(id) = id {
        match artwork_files::open(s, id).await {
            Ok(file) => return Ok(Some(file)),
            Err(e) if e.1 == "not_found" || e.1 == "artwork_corrupt" => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}
type Loader = fn(String) -> futures_util::future::BoxFuture<'static, ApiResult<Vec<u8>>>;
fn remote(url: String) -> futures_util::future::BoxFuture<'static, ApiResult<Vec<u8>>> {
    Box::pin(async move { crate::artwork_fetch::fetch(&url).await })
}
pub async fn source(s: &AppState, id: &str, refresh: bool) -> ApiResult<ImageFile> {
    source_with(s, id, refresh, remote).await
}
struct Job {
    value:
        futures_util::future::Shared<futures_util::future::BoxFuture<'static, ApiResult<String>>>,
}
static JOBS: LazyLock<std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<Job>>>> =
    LazyLock::new(Default::default);
async fn source_with(s: &AppState, id: &str, refresh: bool, load: Loader) -> ApiResult<ImageFile> {
    use futures_util::FutureExt;
    let key = format!("{}:{id}:{refresh}", s.config.data.display());
    let job = {
        let mut jobs = JOBS.lock().unwrap();
        jobs.retain(|_, v| v.strong_count() > 0);
        if let Some(job) = jobs.get(&key).and_then(std::sync::Weak::upgrade) {
            job
        } else {
            if jobs.len() >= 128 {
                return Err(ApiError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "artwork_busy".into(),
                    "图片请求繁忙".into(),
                ));
            }
            let (s, id) = (s.clone(), id.to_owned());
            let job = std::sync::Arc::new(Job {
                value: async move { resolve_source(&s, &id, refresh, load).await }
                    .boxed()
                    .shared(),
            });
            jobs.insert(key, std::sync::Arc::downgrade(&job));
            job
        }
    };
    let result = job.value.clone().await?;
    artwork_files::open(s, &result).await
}
async fn resolve_source(s: &AppState, id: &str, refresh: bool, load: Loader) -> ApiResult<String> {
    if !artwork_files::valid_id(id) {
        return Err(ApiError::missing());
    }
    let gate = crate::media_assets::gate(s, &format!("artwork:{id}")).await;
    let _guard = gate.lock().await;
    let key = id.to_owned();
    let (url,cover,retry)=s.db.read(move|c|{
  let row=c.query_row("SELECT a.url,a.cover_id,f.retry_at,f.code FROM artwork_sources a LEFT JOIN artwork_failures f ON f.source_id=a.id WHERE a.id=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<i64>>(2)?.zip(r.get::<_,Option<String>>(3)?)))).optional()?;Ok(row)
 }).await?.ok_or_else(ApiError::missing)?;
    if !refresh {
        if let Some(file) = cached(s, cover.as_deref()).await? {
            return Ok(file.id);
        }
        if crate::artwork_probe::known_absent(s, id).await? {
            return Err(crate::artwork_probe::unavailable());
        }
    }
    if let Some((at, code)) = retry {
        if at > chrono::Utc::now().timestamp() && (!refresh || code == "artwork_rate_limited") {
            if let Some(file) = cached(s, cover.as_deref()).await? {
                return Ok(file.id);
            }
            return Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                code,
                "图片暂不可用，已安排冷却后重试".into(),
            ));
        }
    }
    let result = download(s, id, &url, refresh, load).await;
    match result {
        Ok(id) => Ok(id),
        Err(e) => {
            if e.1 == "artwork_not_provided" {
                if let Some(file) = cached(s, cover.as_deref()).await? { return Ok(file.id); }
                return Err(e);
            }
            let seconds = match e.1.as_str() {
                "artwork_rate_limited" => crate::artwork_fetch::retry_seconds(&url).max(300) as i64,
                "artwork_source_expired" => 60,
                _ => 15,
            };
            let key = id.to_owned();
            let code = e.1.clone();
            s.db.run(move|c|{c.execute("INSERT INTO artwork_failures(source_id,retry_at,code)VALUES(?1,unixepoch()+?2,?3) ON CONFLICT(source_id)DO UPDATE SET retry_at=excluded.retry_at,code=excluded.code",params![key,seconds,code])?;Ok(())}).await?;
            if let Some(file) = cached(s, cover.as_deref()).await? {
                return Ok(file.id);
            }
            Err(e)
        }
    }
}
async fn download(
    s: &AppState,
    id: &str,
    url: &str,
    refresh: bool,
    load: Loader,
) -> ApiResult<String> {
    // All registered entities sharing this exact source URL share one upstream task.
    let gate =
        crate::media_assets::gate(s, &format!("artwork-url:{}", crate::auth::hash(url))).await;
    let _guard = gate.lock().await;
    if !refresh {
        let url = url.to_owned();
        let covers=s.db.read(move|c|Ok(c.prepare("SELECT DISTINCT cover_id FROM artwork_sources WHERE url=?1 AND cover_id IS NOT NULL LIMIT 4")?.query_map([url],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
        for cover in covers {
            if cached(s, Some(&cover)).await?.is_some() {
                publish(s, id, &cover).await?;
                return Ok(cover);
            }
        }
    }
    // JOBS already bounds admission to 128 sources. Waiting for one of the ten
    // upstream slots is cancellation-safe and must not turn into a false network failure.
    let _slot = SLOTS
        .acquire()
        .await
        .map_err(|_| ApiError::bad("图片服务停止"))?;
    let started = std::time::Instant::now();
    let fetched_result: ApiResult<Vec<u8>> = async {
        Ok(match load(url.into()).await {
            Err(e) if e.1 == "artwork_source_expired" => {
                if let Some(next) = refresh_url(s, id, url).await? {
                    load(next).await?
                } else {
                    return Err(e);
                }
            }
            value => value?,
        })
    }
    .await;
    let fetched = match fetched_result {
        Ok(v) => v,
        Err(e) => {
            if e.1 != "artwork_not_provided" { crate::artwork_diagnostics::record(s, id, "upstream", &e, started).await; }
            return Err(e);
        }
    };
    let metrics = crate::artwork_maintenance::counters(&s.config.data.join("covers"));
    metrics
        .upstream_reads
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    metrics
        .upstream_bytes
        .fetch_add(fetched.len() as u64, std::sync::atomic::Ordering::Relaxed);
    let started = std::time::Instant::now();
    let bytes = match crate::artwork_images::prepare(fetched).await {
        Ok(v) => v,
        Err(e) => {
            crate::artwork_diagnostics::record(s, id, "decode", &e, started).await;
            return Err(e);
        }
    };
    let started = std::time::Instant::now();
    let cover = match artwork_files::save(s, &bytes).await {
        Ok(v) => v,
        Err(e) => {
            crate::artwork_diagnostics::record(s, id, "storage", &e, started).await;
            return Err(e);
        }
    };
    drop(bytes);
    publish(s, id, &cover).await?;
    let _ = crate::artwork_maintenance::health(s, id, "ready", None).await;
    Ok(cover)
}
pub(crate) async fn publish(s: &AppState, id: &str, cover: &str) -> ApiResult<()> {
    let (id, cover) = (id.to_owned(), cover.to_owned());
    s.db.run(move|c|{let tx=c.transaction()?;
 tx.execute("INSERT OR IGNORE INTO artwork_versions SELECT cover_id,id FROM artwork_sources WHERE id=?1 AND cover_id IS NOT NULL",[&id])?;
 tx.execute("INSERT OR IGNORE INTO artwork_versions(cover_id,source_id)VALUES(?1,?2)",params![cover,id])?;
 tx.execute("UPDATE artwork_sources SET cover_id=?1,fetched_at=unixepoch() WHERE id=?2",params![cover,id])?;
 tx.execute("DELETE FROM artwork_failures WHERE source_id=?1",[&id])?;
 tx.execute("DELETE FROM artwork_absences WHERE source_id=?1",[id])?;tx.commit()?;Ok(())}).await?;
    Ok(())
}
pub async fn cover(s: &AppState, id: &str) -> ApiResult<ImageFile> {
    if let Some(file) = cached(s, Some(id)).await? {
        return Ok(file);
    }
    let key = id.to_owned();
    let source_id=s.db.read(move|c|Ok(c.query_row("SELECT source_id FROM artwork_versions WHERE cover_id=?1 UNION SELECT id FROM artwork_sources WHERE cover_id=?1 LIMIT 1",[key],|r|r.get::<_,String>(0)).optional()?)).await?.ok_or_else(||ApiError(StatusCode::NOT_FOUND,"artwork_origin_missing".into(),"原图片文件已丢失，尚无可确认的来源；请刷新该媒体资料或重新指定图片".into()))?;
    source(s, &source_id, false).await
}

// Provider catalog refresh is a cold path. Keep its large async state on the heap;
// embedding it in download/resolve_source multiplies poll-frame stack copies in
// the background select loop and overflows Tokio's default worker stack in debug.
fn refresh_url<'a>(
    s: &'a AppState,
    id: &'a str,
    url: &'a str,
) -> futures_util::future::BoxFuture<'a, ApiResult<Option<String>>> {
    Box::pin(async move {
        if let Some(next) = crate::artwork_probe::resolve(s, id, url).await? { return Ok(Some(next)); }
        let key = id.to_owned();
        let tracks=s.db.read(move|c|Ok(c.prepare("SELECT t.* FROM artwork_entities e JOIN tracks t ON e.kind='track' AND e.entity_id=t.id WHERE e.source_id=?1 AND t.source!='local' AND NOT EXISTS(SELECT 1 FROM catalog_profiles p WHERE p.kind='track' AND p.entity_id=t.id AND json_type(p.manual,'$.image_url') IS NOT NULL) ORDER BY t.id LIMIT 2")?.query_map([key],crate::models::Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
        // Ambiguous ownership never guesses which entity a replacement image belongs to.
        if tracks.len() != 1 {
            return Box::pin(crate::catalog_artwork::refresh(s, id)).await;
        }
        let t = &tracks[0];
        let value = match tokio::time::timeout(
            std::time::Duration::from_secs(12),
            crate::catalog::metadata_for_track_refresh(s, t, true),
        )
        .await
        {
            Ok(Ok(v)) => v,
            _ => return Ok(None),
        };
        if value["matched"] != true || value["track"]["id"] != t.id {
            return Ok(None);
        }
        let Some(url) = value["track"]["cover_url"]
            .as_str()
            .filter(|u| u.starts_with("https://"))
        else {
            return Ok(None);
        };
        let (key, url) = (id.to_owned(), url.to_owned());
        let updated = url.clone();
        s.db.run(move |c| {
            c.execute(
                "UPDATE artwork_sources SET url=?1 WHERE id=?2",
                params![url, key],
            )?;
            Ok(())
        })
        .await?;
        Ok(Some(updated))
    })
}

#[cfg(test)]
#[path = "artwork_recovery_tests.rs"]
mod tests;
