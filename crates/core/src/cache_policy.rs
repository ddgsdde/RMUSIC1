//! Persistent audio caching policy. Missing files wait for playback or an explicit repair.
use crate::state::AppState;
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::{json, Value};

pub async fn enabled(s: &AppState) -> bool {
    s.cache_enabled.load(std::sync::atomic::Ordering::Acquire)
}
pub(crate) fn heard(tx: &Transaction<'_>, track: &str, duration: f64, before: f64, added: f64) -> anyhow::Result<()> {
    let cutoff = duration * 0.30;
    if duration <= 0. || !duration.is_finite() || before > cutoff || before + added <= cutoff {
        return Ok(());
    }
    let eligible: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1 AND source<>'local' AND available=1)
          AND NOT EXISTS(SELECT 1 FROM cache_entries WHERE track_id=?1 AND complete=1)
          AND NOT EXISTS(SELECT 1 FROM settings WHERE (key='cache_enabled' AND value='false') OR (key='playback_cache_mode' AND value<>'\"full\"'))",
        [track], |row| row.get(0),
    )?;
    if eligible {
        crate::background::enqueue_tx(tx, "cache_fill", &format!("{track}:listened"), &json!({"track_id":track,"origin":"listened"}), 0)?;
    }
    Ok(())
}
pub async fn missing(s: &AppState, track: &str) -> anyhow::Result<bool> {
    let track = track.to_owned();
    s.db.run(move |c| Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM cache_entries e LEFT JOIN cache_exports x ON x.cache_id=e.id WHERE e.track_id=?1 AND (x.status='missing' OR EXISTS(SELECT 1 FROM tracks t WHERE t.source='local' AND t.path=e.path AND t.available=0 AND json_type(t.provider_data,'$.cached_from.track_id')='text')))",
        [track], |r| r.get(0))?)).await
}
pub async fn pause(s: &AppState, job: &str, reason: &str) -> anyhow::Result<()> {
    let job = job.to_owned();
    let reason = reason.to_owned();
    s.db.run(move |c| { c.execute("UPDATE jobs SET status='paused',phase=?2,updated_at=unixepoch() WHERE id=?1 AND status NOT IN ('cancelled','completed')", params![job,reason])?; Ok(()) }).await
}
pub async fn schedule(
    s: &AppState,
    track: &str,
    quality: &str,
    origin: &str,
    repair_job: Option<&str>,
    priority: i64,
) -> anyhow::Result<String> {
    anyhow::ensure!(
        enabled(s).await,
        "Core 音频缓存已关闭；播放可直接拉流，补齐任务不会启动"
    );
    crate::library_files::validate_cached(s, track).await?;
    let mut payload = json!({"track_id":track,"quality":quality,"origin":origin});
    if let Some(parent) = repair_job {
        payload["repair_job"] = json!(parent);
    }
    let id = crate::background::enqueue(
        s,
        "cache_fill",
        &format!("{track}:{quality}"),
        payload.clone(),
        priority,
    )
    .await?;
    let key = id.clone();
    let manual = origin != "playback";
    s.db.run(move |c| {
        // A playback read may wake a policy-paused job, but never overrides the user's pause.
        c.execute("UPDATE jobs SET payload=json_patch(payload,?2),status=CASE WHEN status='paused' AND (?3 OR COALESCE(json_extract(payload,'$.manual_pause'),0)=0) THEN 'pending' ELSE status END,next_try=0,updated_at=unixepoch() WHERE id=?1",params![key,payload.to_string(),manual])?;
        Ok(())
    }).await?;
    Ok(id)
}
pub async fn guard_fill(s: &AppState, job: &str, payload: &Value) -> anyhow::Result<bool> {
    if !enabled(s).await {
        pause(s, job, "Core 音频缓存已关闭，任务保留且不拉取音源").await?;
        return Ok(false);
    }
    // get_track may have discovered the loss and revoked the old request in SQLite.
    // The worker's captured payload can predate that transaction.
    let key = job.to_owned();
    let explicit=s.db.run(move|c|Ok(c.query_row("SELECT COALESCE(json_extract(payload,'$.origin') IN ('playback','listened','manual','repair'),0) FROM jobs WHERE id=?1",[key],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false))).await?;
    if !explicit && missing(s, payload["track_id"].as_str().unwrap_or("")).await? {
        pause(s, job, "文件缺失，等待后续播放或手动补齐；未自动拉取").await?;
        return Ok(false);
    }
    Ok(true)
}
