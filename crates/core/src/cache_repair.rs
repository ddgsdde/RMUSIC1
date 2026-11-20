//! Bounded, resumable missing-cache scan. Child transfers use the normal pacing/resource gates.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
pub struct Request {
    pub concurrency: usize,
}
fn validate(value: usize) -> ApiResult<()> {
    if (1..=16).contains(&value) {
        Ok(())
    } else {
        Err(ApiError::bad(
            "补齐并发应为 1–16，同时受后台总并发、歌曲拉取和平台限流约束",
        ))
    }
}
pub async fn status(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let job=s.db.run(|c|Ok(c.query_row("SELECT id,status,phase,processed,total,error,payload FROM jobs WHERE kind='cache_repair' ORDER BY created_at DESC,rowid DESC LIMIT 1",[],|r|Ok(json!({"id":r.get::<_,String>(0)?,"status":r.get::<_,String>(1)?,"phase":r.get::<_,String>(2)?,"processed":r.get::<_,i64>(3)?,"total":r.get::<_,i64>(4)?,"error":r.get::<_,Option<String>>(5)?,"options":serde_json::from_str::<Value>(&r.get::<_,String>(6)?).unwrap_or_default()}))).optional()?)).await?;
    Ok(Json(
        json!({"enabled":crate::cache_policy::enabled(&s).await,"job":job}),
    ))
}
pub async fn start(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<Request>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    validate(v.concurrency)?;
    if !crate::cache_policy::enabled(&s).await {
        return Err(ApiError::bad(
            "Core 音频缓存已关闭，未启动补齐，也不会下载文件",
        ));
    }
    let job=crate::background::enqueue(&s,"cache_repair","all-missing",json!({"concurrency":v.concurrency,"after":"","scan_complete":false,"checked":0,"skipped":0}),5).await?;
    set_concurrency(&s, &job, v.concurrency).await?;
    let key = job.clone();
    let status =
        s.db.run(move |c| {
            Ok(
                c.query_row("SELECT status FROM jobs WHERE id=?1", [key], |r| {
                    r.get::<_, String>(0)
                })?,
            )
        })
        .await?;
    Ok(Json(json!({"id":job,"job_id":job,"status":status})))
}
pub async fn configure(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(v): Json<Request>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    validate(v.concurrency)?;
    set_concurrency(&s, &id, v.concurrency).await?;
    Ok(Json(json!({"saved":true})))
}
async fn set_concurrency(s: &AppState, id: &str, concurrency: usize) -> anyhow::Result<()> {
    let id = id.to_owned();
    s.db.run(move|c|{anyhow::ensure!(c.execute("UPDATE jobs SET payload=json_set(payload,'$.concurrency',?2),updated_at=unixepoch() WHERE id=?1 AND kind='cache_repair'",params![id,concurrency as i64])?>0,"补齐任务不存在");Ok(())}).await
}
pub async fn admitted(s: &AppState, payload: &Value) -> anyhow::Result<bool> {
    let Some(parent) = payload["repair_job"].as_str() else {
        return Ok(true);
    };
    let parent = parent.to_owned();
    s.db.run(move|c|{
        let policy:Option<(String,String)>=c.query_row("SELECT status,payload FROM jobs WHERE id=?1 AND kind='cache_repair'",[&parent],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((status,policy))=policy else{return Ok(false)};
        if matches!(status.as_str(),"paused"|"cancelled"|"failed"){return Ok(false);}
        let limit=serde_json::from_str::<Value>(&policy)?["concurrency"].as_u64().unwrap_or(1).clamp(1,16);
        let active:i64=c.query_row("SELECT COUNT(*) FROM jobs WHERE kind='cache_fill' AND status='running' AND json_extract(payload,'$.repair_job')=?1",[parent],|r|r.get(0))?;
        Ok((active as u64)<limit)
    }).await
}
pub async fn run(s: &AppState, id: &str, payload: &Value) -> anyhow::Result<bool> {
    if !crate::cache_policy::enabled(s).await {
        crate::cache_policy::pause(s, id, "Core 音频缓存已关闭，补齐任务已暂停").await?;
        return Ok(false);
    }
    let mut next = payload.clone();
    if next["scan_complete"] != true {
        let after = next["after"].as_str().unwrap_or("").to_owned();
        let rows=s.db.run(move|c|Ok(c.prepare("SELECT e.id,e.track_id,e.path,e.total_size FROM cache_entries e WHERE e.id>?1 AND (e.complete=1 OR EXISTS(SELECT 1 FROM cache_exports x WHERE x.cache_id=e.id AND x.status='missing') OR EXISTS(SELECT 1 FROM tracks t WHERE t.source='local' AND t.path=e.path AND json_type(t.provider_data,'$.cached_from.track_id')='text')) AND NOT EXISTS(SELECT 1 FROM file_bin b WHERE b.status IN ('planned','trashed','restoring') AND (b.original_path=e.path OR b.track_id=e.track_id)) ORDER BY e.id LIMIT 100")?.query_map([after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?.max(0) as u64)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
        let roots = [s.cache_dir().await, s.portable_dir().await];
        for (cache, track, path, size) in &rows {
            if !crate::background::disposition(s, id).await? {
                return Ok(false);
            }
            let path = std::path::Path::new(path);
            let offline = roots
                .iter()
                .any(|root| path.starts_with(root) && !root.is_dir());
            if !offline {
                let missing = match tokio::fs::metadata(path).await {
                    Ok(meta) => meta.len() != *size || !meta.is_file(),
                    Err(e) => e.kind() == std::io::ErrorKind::NotFound,
                };
                if missing {
                    crate::library_files::validate_cached(s, track).await?;
                    match crate::library::get_track(s, track.clone()).await {
                        Ok(source) if source.source != "local" => {
                            let quality = crate::catalog::quality(s, &source.source).await;
                            crate::cache_policy::schedule(
                                s,
                                track,
                                &quality,
                                "repair",
                                Some(id),
                                0,
                            )
                            .await?;
                        }
                        _ => next["skipped"] = json!(next["skipped"].as_u64().unwrap_or(0) + 1),
                    }
                }
            } else {
                next["skipped"] = json!(next["skipped"].as_u64().unwrap_or(0) + 1);
            }
            next["after"] = json!(cache);
            next["checked"] = json!(next["checked"].as_u64().unwrap_or(0) + 1);
        }
        next["scan_complete"] = json!(rows.len() < 100);
        let key = id.to_owned();
        let after = next["after"].clone();
        let scanned = next["checked"].clone();
        let skipped = next["skipped"].clone();
        let complete = next["scan_complete"].clone();
        s.db.run(move|c|{c.execute("UPDATE jobs SET payload=json_set(payload,'$.after',json(?2),'$.checked',json(?3),'$.skipped',json(?4),'$.scan_complete',json(?5)) WHERE id=?1",params![key,after.to_string(),scanned.to_string(),skipped.to_string(),complete.to_string()])?;Ok(())}).await?;
        if next["scan_complete"] != true {
            crate::background::progress(
                s,
                id,
                &format!("已检查 {} 项，继续扫描缺失文件", next["checked"]),
                0,
                0,
            )
            .await?;
            return Ok(false);
        }
    }
    let key = id.to_owned();
    let (total,done,failed)=s.db.run(move|c|Ok(c.query_row("SELECT COUNT(*),COALESCE(SUM(status='completed'),0),COALESCE(SUM(status IN ('failed','unable','cancelled')),0) FROM jobs WHERE kind='cache_fill' AND json_extract(payload,'$.repair_job')=?1",[key],|r|Ok((r.get::<_,i64>(0)?.max(0) as u64,r.get::<_,i64>(1)?.max(0) as u64,r.get::<_,i64>(2)?.max(0) as u64)))?)).await?;
    crate::background::progress(
        s,
        id,
        &format!(
            "补齐 {done}/{total}，失败 {failed}，跳过 {}；平台限流与前台播放优先",
            next["skipped"]
        ),
        done,
        total,
    )
    .await?;
    if done + failed == total {
        anyhow::ensure!(failed == 0, "{failed} 项补齐失败；请检查子任务并重试补齐");
        return Ok(true);
    }
    Ok(false)
}
