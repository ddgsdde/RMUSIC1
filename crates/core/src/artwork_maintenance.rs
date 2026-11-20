//! Protected persistent images: bounded storage, diagnostics and explicit repair jobs.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
pub const DEFAULT_LIMIT: u64 = 512 * 1024 * 1024;
#[derive(Default)]
pub struct Counters {
    pub requests: AtomicU64,
    pub not_modified: AtomicU64,
    pub response_bytes: AtomicU64,
    pub upstream_bytes: AtomicU64,
    pub upstream_reads: AtomicU64,
}
static COUNTERS: LazyLock<Mutex<HashMap<PathBuf, Arc<Counters>>>> = LazyLock::new(Default::default);
static SIZES: LazyLock<Mutex<HashMap<PathBuf, (Instant, u64, u64)>>> =
    LazyLock::new(Default::default);
pub fn counters(root: &Path) -> Arc<Counters> {
    let mut map = COUNTERS.lock().unwrap();
    if let Some(v) = map.get(root) {
        return v.clone();
    }
    if map.len() >= 64 {
        map.retain(|_, v| Arc::strong_count(v) > 1);
    }
    let c = Arc::new(Counters::default());
    map.insert(root.into(), c.clone());
    c
}
pub async fn usage(s: &AppState, force: bool) -> ApiResult<(u64, u64)> {
    let root = s.config.data.join("covers");
    if !force {
        if let Some((at, size, count)) = SIZES.lock().unwrap().get(&root) {
            if at.elapsed() < Duration::from_secs(30) {
                return Ok((*size, *count));
            }
        }
    }
    let mut size = 0u64;
    let mut count = 0u64;
    match tokio::fs::read_dir(&root).await {
        Ok(mut entries) => {
            while let Some(entry) = entries.next_entry().await.map_err(anyhow::Error::from)? {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !crate::artwork_files::valid_id(&name) {
                    continue;
                }
                let meta = entry.metadata().await.map_err(anyhow::Error::from)?;
                if meta.is_file()
                    && !entry
                        .file_type()
                        .await
                        .map_err(anyhow::Error::from)?
                        .is_symlink()
                {
                    size = size.saturating_add(meta.len());
                    count += 1;
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(anyhow::Error::from(e).into()),
    }
    let mut map = SIZES.lock().unwrap();
    if map.len() >= 64 {
        map.clear();
    }
    map.insert(root, (Instant::now(), size, count));
    Ok((size, count))
}
pub fn published(s: &AppState, old: u64, new: u64, created: bool) {
    if let Some((_, bytes, count)) = SIZES.lock().unwrap().get_mut(&s.config.data.join("covers")) {
        *bytes = bytes.saturating_sub(old).saturating_add(new);
        if created {
            *count += 1;
        }
    }
}
pub async fn reserve(s: &AppState, id: &str, size: u64) -> ApiResult<(u64, bool)> {
    let prior = tokio::fs::metadata(s.config.data.join("covers").join(id))
        .await
        .ok()
        .map(|m| m.len());
    let (used, _) = usage(s, false).await?;
    let limit = s
        .setting("artwork_limit_bytes", json!(DEFAULT_LIMIT))
        .await
        .as_u64()
        .unwrap_or(DEFAULT_LIMIT);
    if size > prior.unwrap_or(0)
        && used.saturating_sub(prior.unwrap_or(0)).saturating_add(size) > limit
    {
        return Err(ApiError(
            StatusCode::INSUFFICIENT_STORAGE,
            "artwork_budget_exceeded".into(),
            "图片预算不足；已保留现有图片，请提高独立图片预算".into(),
        ));
    }
    Ok((prior.unwrap_or(0), prior.is_none()))
}
pub async fn validate_limit(s: &AppState, limit: u64) -> ApiResult<()> {
    let (used, _) = usage(s, true).await?;
    if limit < used {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "artwork_budget_below_usage".into(),
            format!("图片预算不能低于当前占用 {used} 字节；永久资料不会自动删除"),
        ));
    }
    Ok(())
}
#[derive(Deserialize)]
pub struct Page {
    #[serde(default)]
    offset: u32,
    #[serde(default = "page_size")]
    limit: u32,
    #[serde(default)]
    failed: bool,
}
fn page_size() -> u32 {
    50
}
pub async fn status(
    State(s): State<AppState>,
    a: Auth,
    Query(q): Query<Page>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if q.limit == 0 || q.limit > 100 || q.offset > 1_000_000 {
        return Err(ApiError::bad("图片分页无效"));
    }
    let (used, count) = {
        let gate = storage_gate(&s.config.data);
        let _guard = gate.lock().await;
        usage(&s, false).await?
    };
    let limit = s.setting("artwork_limit_bytes", json!(DEFAULT_LIMIT)).await;
    let metrics = counters(&s.config.data.join("covers"));
    let mut value=s.db.read(move|c|{
  let condition=if q.failed{" WHERE h.state IN ('missing','corrupt','failed') OR f.retry_at IS NOT NULL"}else{""};
  let total=c.query_row(&format!("SELECT COUNT(*) FROM artwork_sources a LEFT JOIN artwork_health h ON h.source_id=a.id LEFT JOIN artwork_failures f ON f.source_id=a.id{condition}"),[],|r|r.get::<_,i64>(0))?;
  let items=c.prepare(&format!("SELECT a.id,a.url,a.cover_id,a.fetched_at,h.state,h.error,h.checked_at,f.retry_at,f.code,COALESCE((SELECT COALESCE(t.title,json_extract(p.automatic,'$.name'),e.entity_id) FROM artwork_entities e LEFT JOIN tracks t ON e.kind='track' AND t.id=e.entity_id LEFT JOIN catalog_profiles p ON p.kind=e.kind AND p.entity_id=e.entity_id WHERE e.source_id=a.id LIMIT 1),NULLIF(p.name,''),'图片') FROM artwork_sources a LEFT JOIN artwork_absences p ON p.source_id=a.id AND p.url=a.url LEFT JOIN artwork_health h ON h.source_id=a.id LEFT JOIN artwork_failures f ON f.source_id=a.id{condition} ORDER BY COALESCE(f.retry_at,h.checked_at,a.fetched_at,0) DESC,a.id LIMIT ?1 OFFSET ?2"))?.query_map(params![q.limit,q.offset],|r|{let url:String=r.get(1)?;let cover:Option<String>=r.get(2)?;Ok(json!({"id":r.get::<_,String>(0)?,"host":url::Url::parse(&url).ok().and_then(|u|u.host_str().map(String::from)),"cover_id":cover,"fetched_at":r.get::<_,Option<i64>>(3)?,"state":r.get::<_,Option<String>>(4)?.unwrap_or_else(||if cover.is_some(){"stored".into()}else{"not_requested".into()}),"error":r.get::<_,Option<String>>(5)?.or(r.get::<_,Option<String>>(8)?),"checked_at":r.get::<_,Option<i64>>(6)?,"retry_at":r.get::<_,Option<i64>>(7)?,"name":r.get::<_,String>(9)?}))})?.collect::<rusqlite::Result<Vec<_>>>()?;
  let jobs=c.prepare("SELECT id,kind,status,phase,processed,total,error FROM jobs WHERE kind IN ('artwork_check','artwork_refresh','catalog_scrape') ORDER BY created_at DESC LIMIT 5")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"phase":r.get::<_,String>(3)?,"processed":r.get::<_,i64>(4)?,"total":r.get::<_,i64>(5)?,"error":r.get::<_,Option<String>>(6)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
  Ok(json!({"items":items,"total":total,"limit":q.limit,"offset":q.offset,"jobs":jobs}))
 }).await?;
    value["used_bytes"] = json!(used);
    value["file_count"] = json!(count);
    value["limit_bytes"] = limit;
    value["protected_base_images"] = json!(true);
    value["derived_count"] = json!(0);
    value["metrics"] = json!({"scope":"since_start","requests":metrics.requests.load(Ordering::Relaxed),"not_modified":metrics.not_modified.load(Ordering::Relaxed),"response_bytes":metrics.response_bytes.load(Ordering::Relaxed),"upstream_bytes":metrics.upstream_bytes.load(Ordering::Relaxed),"upstream_reads":metrics.upstream_reads.load(Ordering::Relaxed)});
    Ok(Json(value))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refresh {
    ids: Vec<String>,
}
pub async fn refresh(
    State(s): State<AppState>,
    a: Auth,
    Json(q): Json<Refresh>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if q.ids.is_empty()
        || q.ids.len() > 50
        || q.ids.iter().any(|id| !crate::artwork_files::valid_id(id))
    {
        return Err(ApiError::bad("每次选择1–50张图片"));
    }
    let mut ids = q.ids;
    ids.sort();
    ids.dedup();
    let key = crate::auth::hash(&serde_json::to_string(&ids).unwrap());
    let id = crate::background::enqueue(
        &s,
        "artwork_refresh",
        &key,
        json!({"ids":ids,"after":0}),
        20,
    )
    .await?;
    Ok(Json(json!({"id":id,"status":"pending"})))
}
pub async fn check(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let id = crate::background::enqueue(&s, "artwork_check", "all", json!({"after":""}), 0).await?;
    Ok(Json(json!({"id":id,"status":"pending"})))
}
pub async fn health(
    s: &AppState,
    id: &str,
    state: &str,
    error: Option<&str>,
) -> anyhow::Result<()> {
    let (id, state, error) = (id.to_owned(), state.to_owned(), error.map(String::from));
    s.db.run(move|c|{c.execute("INSERT INTO artwork_health(source_id,state,error,checked_at)VALUES(?1,?2,?3,unixepoch()) ON CONFLICT(source_id)DO UPDATE SET state=excluded.state,error=excluded.error,checked_at=excluded.checked_at",params![id,state,error])?;Ok(())}).await
}
pub async fn run(s: &AppState, job: &str, kind: &str, payload: &Value) -> anyhow::Result<bool> {
    if kind == "artwork_refresh" {
        let ids = payload["ids"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("图片列表缺失"))?;
        let after = payload["after"].as_u64().unwrap_or(0) as usize;
        for (i, id) in ids.iter().enumerate().skip(after).take(3) {
            if !crate::background::disposition(s, job).await? {
                return Ok(false);
            }
            let id = id.as_str().unwrap_or("");
            let result = crate::artwork_recovery::source(s, id, true).await;
            match result {
                Ok(_) => health(s, id, "ready", None).await?,
                Err(e) if e.1 == "artwork_not_provided" => health(s, id, "unavailable", None).await?,
                Err(e) => health(s, id, "failed", Some(&e.1)).await?,
            };
            save_cursor(s, job, json!(i + 1), i + 1, ids.len()).await?;
        }
        return Ok(after + 3 >= ids.len());
    }
    let after = payload["after"].as_str().unwrap_or("").to_owned();
    let items =
        s.db.read(move |c| {
            Ok(c.prepare(
                "SELECT a.id,a.cover_id,p.source_id IS NOT NULL FROM artwork_sources a LEFT JOIN artwork_absences p ON p.source_id=a.id AND p.url=a.url WHERE a.id>?1 ORDER BY a.id LIMIT 128",
            )?
            .query_map([after], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_,bool>(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await?;
    for (id, cover, absent) in &items {
        if !crate::background::disposition(s, job).await? {
            return Ok(false);
        }
        let (state, error) = if let Some(cover) = cover {
            match crate::artwork_files::open(s, cover).await {
                Ok(_) => ("ready", None),
                Err(e) if e.1 == "not_found" => ("missing", Some(e.1)),
                Err(e) if e.1 == "artwork_corrupt" => ("corrupt", Some(e.1)),
                Err(e) => ("failed", Some(e.1)),
            }
        } else if *absent {
            ("unavailable", None)
        } else {
            ("not_requested", None)
        };
        health(s, id, state, error.as_deref()).await?;
        save_cursor(s, job, json!(id), 1, 0).await?;
    }
    // A forced recount must not race an atomic publication and double-count it.
    let gate = storage_gate(&s.config.data);
    let _guard = gate.lock().await;
    let _ = usage(s, true).await;
    Ok(items.len() < 128)
}
async fn save_cursor(
    s: &AppState,
    job: &str,
    after: Value,
    processed: usize,
    total: usize,
) -> anyhow::Result<()> {
    let job = job.to_owned();
    s.db.run(move|c|{c.execute("UPDATE jobs SET payload=json_set(payload,'$.after',json(?2)),processed=CASE WHEN total>0 THEN ?3 ELSE processed+?3 END,total=MAX(total,?4),phase='检查／更新图片',updated_at=unixepoch() WHERE id=?1",params![job,after.to_string(),processed as i64,total as i64])?;Ok(())}).await
}

static STORAGE_GATES: LazyLock<Mutex<HashMap<PathBuf, std::sync::Weak<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Default::default);
pub fn storage_gate(data: &Path) -> Arc<tokio::sync::Mutex<()>> {
    let mut gates = STORAGE_GATES.lock().unwrap();
    gates.retain(|_, g| g.strong_count() > 0);
    let gate = gates
        .get(data)
        .and_then(std::sync::Weak::upgrade)
        .unwrap_or_default();
    gates.insert(data.into(), Arc::downgrade(&gate));
    gate
}
/// Scanner-only synchronous boundary. A busy/full image store never makes music import fail.
pub fn embedded(data: &Path, bytes: &[u8]) -> anyhow::Result<Option<String>> {
    let key = crate::artwork_files::content_id(bytes);
    let root = data.join("covers");
    let target = root.join(&key);
    if target.is_file() {
        return Ok(Some(format!("/api/v1/covers/{key}")));
    }
    let gate = storage_gate(data);
    let Ok(_guard) = gate.try_lock_owned() else {
        return Ok(None);
    };
    let limit = rusqlite::Connection::open_with_flags(
        data.join("library.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()
    .and_then(|c| {
        c.query_row(
            "SELECT value FROM settings WHERE key='artwork_limit_bytes'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
    })
    .and_then(|v| serde_json::from_str::<u64>(&v).ok())
    .unwrap_or(DEFAULT_LIMIT);
    let known = {
        SIZES
            .lock()
            .unwrap()
            .get(&root)
            .filter(|v| v.0.elapsed() < Duration::from_secs(30))
            .map(|v| v.1)
    };
    let size = if let Some(size) = known {
        size
    } else {
        let mut size = 0;
        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries {
                let entry = entry?;
                if crate::artwork_files::valid_id(&entry.file_name().to_string_lossy()) {
                    let meta = entry.metadata()?;
                    if meta.is_file() {
                        size += meta.len();
                        count += 1;
                    }
                }
            }
        }
        SIZES
            .lock()
            .unwrap()
            .insert(root.clone(), (Instant::now(), size, count));
        size
    };
    if size.saturating_add(bytes.len() as u64) > limit {
        return Ok(None);
    }
    std::fs::create_dir_all(&root)?;
    let temp = root.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, &target)?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    if let Some((_, size, count)) = SIZES.lock().unwrap().get_mut(&root) {
        *size += bytes.len() as u64;
        *count += 1;
    }
    Ok(Some(format!("/api/v1/covers/{key}")))
}
