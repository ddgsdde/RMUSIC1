//! Durable EBU R128 analysis of local files and complete Core cache entries.
use crate::{
    auth::Auth,
    error::ApiResult,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::{
    path::Path as FsPath,
    process::Stdio,
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command, sync::{broadcast, Notify}};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Measurement {
    pub integrated: f64,
    pub true_peak: Option<f64>,
}

pub(crate) fn qq_lufs(value: &Value) -> Option<f64> {
    let gain = value["gain_db"].as_f64().filter(|gain| (-40. ..=30.).contains(gain))?;
    value["sample_peak"].as_f64().filter(|peak| *peak > 0. && *peak <= 2.)?;
    (value["reference_lufs"] == -18).then_some(-18. - gain)
}

pub(crate) async fn publish_qq_gain(state: AppState, id: String) {
    let Ok(track) = crate::library::get_track(&state, id).await else { return; };
    if track.provider_data["integrated_lufs"].is_null() {
        if let Some(integrated) = track.integrated_lufs {
            publish(&state, &track, Measurement { integrated, true_peak: None }).await;
        }
    }
}

pub(crate) struct Runtime {
    pub wake: Notify,
    pub updates: broadcast::Sender<(String, Measurement)>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self { wake: Notify::new(), updates: broadcast::channel(64).0 }
    }
}

pub async fn analyze(
    State(state): State<AppState>,
    auth: Auth,
    Path(track_id): Path<String>,
) -> ApiResult<Json<Value>> {
    auth.require("library")?;
    crate::library::get_track(&state,track_id.clone()).await?;
    let id=crate::background::enqueue(&state,"loudness",&track_id,json!({"track_id":track_id,"manual":true}),50).await?;
    let key = id.clone();
    state.db.run(move |c| {
        c.execute("UPDATE jobs SET payload=json_set(payload,'$.manual',json('true')) WHERE id=?1",[key])?;
        Ok(())
    }).await?;
    let result=(id,"pending".to_owned());
    state.emit("jobs", json!({}));
    Ok(Json(json!({"id":result.0,"status":result.1})))
}

#[derive(Clone)]
struct Job {
    id: String,
    track_id: String,
}
enum Analysis {
    Measured { integrated: f64, true_peak: f64 },
    Unable,
    Deferred,
    Paused,
    Cancelled,
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut discover_at=tokio::time::Instant::now();
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = state.loudness.wake.notified() => {}
            }
            let (busy, targets) = playback_targets(&state).await;
            if !busy && tokio::time::Instant::now()>=discover_at {
                let _=enqueue_missing(&state).await;
                discover_at=tokio::time::Instant::now()+Duration::from_secs(60);
            }
            let next = next_job(&state, busy, targets).await;
            match next {
                Ok(Some(job)) => {
                    let _permit=match state.resources.background(&state,16*1024*1024,true).await {Ok(p)=>p,Err(reason)=>{let _=crate::background::waiting(&state,&job.id,reason).await;continue;}};
                    if let Err(error) = process(&state, &job).await {
                        let _ = finish(
                            &state,
                            &job.id,
                            "failed",
                            Some("响度分析失败，请检查文件和 FFmpeg".into()),
                        )
                        .await;
                        tracing::warn!(error=%error,"loudness analysis failed");
                    }
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(error=%error,"cannot query loudness queue"),
            }
        }
    });
}

fn add_targets(zone: &crate::models::Zone, reserved: Option<&crate::models::Track>, targets: &mut Vec<(String, i64)>) {
    if zone.dsp["normalize"] != true || zone.dsp["enabled"] != true { return; }
    if let Some(track) = &zone.track {
        if track.integrated_lufs.is_none() { targets.push((track.id.clone(), 100)); }
    }
    if zone.repeat == "one" { return; }
    let next = reserved.or_else(|| zone.queue.get(zone.queue_index.saturating_add(1)))
        .or_else(|| (zone.repeat == "all" && !zone.shuffle).then(||zone.queue.first()).flatten());
    if let Some(track) = next.filter(|track| track.integrated_lufs.is_none()) {
        targets.push((track.id.clone(), 90));
    }
}

async fn playback_targets(state: &AppState) -> (bool, Vec<(String, i64)>) {
    let reserved = crate::sequence::loudness_successors(state).await;
    let mut targets = Vec::new();
    let mut busy = false;
    {
        let zones = state.zones.read().await;
        for zone in zones.values().filter(|zone| zone.status == "playing") {
            busy = true;
            let next = reserved.get(&zone.id).filter(|(id, _)| zone.playback_id.as_ref() == Some(id)).map(|(_, track)| track);
            add_targets(zone, next, &mut targets);
        }
    }
    crate::portable::for_playing(state, |zone| {
        busy = true;
        add_targets(zone, None, &mut targets);
    }).await;
    targets.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut seen = std::collections::HashSet::new();
    targets.retain(|(id, _)| seen.insert(id.clone()));
    (busy, targets)
}

async fn next_job(state: &AppState, busy: bool, targets: Vec<(String, i64)>) -> anyhow::Result<Option<Job>> {
    if !targets.is_empty() {
        let status_filter = "kind='loudness' AND status IN ('pending','running','interrupted','waiting') AND next_try<=unixepoch()";
        for (track_id, priority) in targets {
            let id = track_id.clone();
            let status = state.db.read(move |c| {
                let found = c.query_row(&format!("SELECT id FROM jobs WHERE cursor=?1 AND {status_filter} LIMIT 1"), [&id], |row| row.get::<_, String>(0)).optional()?;
                let eligible = found.is_none() && c.query_row(&format!(
                    "SELECT EXISTS(SELECT 1 FROM tracks t WHERE t.id=?1 AND {ELIGIBLE} AND NOT EXISTS(SELECT 1 FROM jobs WHERE kind='loudness' AND cursor=t.id))"),
                    [&id], |row| row.get::<_, bool>(0))?;
                Ok((found, eligible))
            }).await?;
            if let Some(id) = status.0 { return Ok(Some(Job { id, track_id })); }
            if status.1 {
                let id = crate::background::enqueue(state, "loudness", &track_id, json!({"track_id":track_id}), priority).await?;
                return Ok(Some(Job { id, track_id }));
            }
        }
    }
    if busy { return Ok(None); }
    state.db.read(|c| Ok(c.query_row(
        "SELECT id,cursor FROM jobs WHERE kind='loudness' AND status IN ('pending','running','interrupted','waiting') AND next_try<=unixepoch() ORDER BY priority DESC,created_at,id LIMIT 1", [],
        |row| Ok(Job { id: row.get(0)?, track_id: row.get(1)? })).optional()?)).await
}

async fn defer_for_playback(state:&AppState,track:&str)->bool {
    let (busy, targets) = playback_targets(state).await;
    busy && !targets.iter().any(|(id, _)| id == track)
}
const ELIGIBLE: &str = "t.available=1 AND json_extract(t.provider_data,'$.integrated_lufs') IS NULL
  AND (t.source<>'qqmusic' OR json_extract(t.provider_data,'$.qq_replay_gain.reference_lufs') IS NULL OR json_extract(t.provider_data,'$.source_audio.quality')='HIRES')
  AND (t.source='local' AND t.path IS NOT NULL OR t.source!='local' AND EXISTS(SELECT 1 FROM cache_entries e WHERE e.track_id=t.id AND e.complete=1))";
async fn enqueue_missing(state:&AppState)->anyhow::Result<()> {
    let tracks=state.db.read(|c|Ok(c.prepare(&format!("SELECT t.id FROM tracks t WHERE {ELIGIBLE}
        AND NOT EXISTS(SELECT 1 FROM jobs j WHERE j.kind='loudness' AND j.cursor=t.id)
        LIMIT MIN(32,MAX(0,256-(SELECT COUNT(*) FROM jobs WHERE kind='loudness' AND status IN ('pending','running','waiting','interrupted'))))"))?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    for id in tracks{crate::background::enqueue(state,"loudness",&id,json!({"track_id":id}),0).await?;}
    Ok(())
}

async fn finish(
    state: &AppState,
    id: &str,
    status: &str,
    error: Option<String>,
) -> anyhow::Result<()> {
    let id = id.to_owned();
    let status = status.to_owned();
    state.db.run(move|c|{c.execute("UPDATE jobs SET status=?1,error=?2,processed=CASE WHEN ?1='completed' THEN 1 ELSE processed END,phase=CASE WHEN ?1='completed' THEN '完成' ELSE COALESCE(?2,phase) END,updated_at=unixepoch() WHERE id=?3",params![status,error,id])?;Ok(())}).await?;
    state.emit("jobs", json!({}));
    Ok(())
}

#[derive(PartialEq)]
struct Source {
    path: String,
    cache_id: Option<String>,
    bytes: Option<i64>,
    playback_version: bool,
}

fn source(c: &rusqlite::Connection, id: &str, local: bool, policy: &str) -> rusqlite::Result<Option<Source>> {
    if local {
        return Ok(c.query_row("SELECT path FROM tracks WHERE id=?1 AND available=1 AND path IS NOT NULL",
            [id], |row| Ok(Source {path: row.get(0)?, cache_id: None, bytes: None, playback_version:true})).optional()?);
    }
    let suffix = format!("%-q{policy}\"");
    let cached = |policy: &str, playback_version| c.query_row(crate::media::COMPLETE_CACHE_QUERY,
        params![id, suffix, policy], |row| Ok(Source {path:row.get(1)?,cache_id:Some(row.get(0)?),
            bytes:Some(row.get(3)?),playback_version})).optional();
    match cached(policy, true)? {
        Some(entry) => Ok(Some(entry)),
        None => cached("cached", false),
    }
}

async fn process(state: &AppState, job: &Job) -> anyhow::Result<()> {
    let id = job.id.clone();
    let track_id = job.track_id.clone();
    let track=crate::library::get_track(state,job.track_id.clone()).await.map_err(|e|anyhow::anyhow!(e.2))?;
    if track.source == "qqmusic" && track.integrated_lufs.is_some() && track.provider_data["integrated_lufs"].is_null() {
        let (key, manual) = (job.id.clone(), "SELECT COALESCE(json_extract(payload,'$.manual'),0) FROM jobs WHERE id=?1");
        if !state.db.read(move |c| Ok(c.query_row(manual, [key], |r| r.get::<_,bool>(0))?)).await? {
            finish(state, &job.id, "completed", None).await?;
            return Ok(());
        }
    }
    let policy=crate::catalog::quality(state,&track.source).await;
    let requested_policy=policy.clone();
    let local=track.source=="local";
    let selected=state.db.run(move|c|{
        let changed=c.execute("UPDATE jobs SET status='running',error=NULL,phase='分析响度',total=1,updated_at=unixepoch() WHERE id=?1 AND status IN ('pending','running','interrupted','waiting')",[id])?;
        if changed==0 {return Ok((false,None))}
        Ok((true, source(c,&track_id,local,&requested_policy)?))
    }).await?;
    if !selected.0 {
        return Ok(());
    }
    let Some(selected) = selected.1 else {
        crate::background::waiting(state,&job.id,"等待来源文件或完整缓存可用").await?;
        return Ok(());
    };
    let _cache_lease=crate::media::retain_path(state,&selected.path).await?;
    let source_stamp = file_stamp(&selected.path)?;
    if selected.bytes.is_some_and(|bytes| u64::try_from(bytes).ok() != Some(source_stamp.0)) {
        finish(state,&job.id,"unable",Some("缓存文件长度与完整记录不一致".into())).await?;
        return Ok(());
    }
    let result = analyze_file(state, job, &selected.path).await?;
    if !local&&crate::catalog::quality(state,&track.source).await!=policy{crate::background::waiting(state,&job.id,"音源音质已变更，等待对应缓存分析").await?;return Ok(());}
    match result {
        Analysis::Deferred => {
            finish(
                state,
                &job.id,
                "pending",
                Some("播放优先，分析将在空闲时重新开始".into()),
            )
            .await?
        }
        Analysis::Paused => finish(state, &job.id, "paused", None).await?,
        Analysis::Cancelled => finish(state, &job.id, "cancelled", None).await?,
        Analysis::Unable => {
            finish(
                state,
                &job.id,
                "unable",
                Some("静音或无法获得有限响度，未应用归一化增益".into()),
            )
            .await?
        }
        Analysis::Measured {
            integrated,
            true_peak,
        } => {
            if source_stamp != file_stamp(&selected.path)? {
                finish(
                    state,
                    &job.id,
                    "pending",
                    Some("来源文件已变化，等待重新分析".into()),
                )
                .await?;
                return Ok(());
            }
            let id = job.track_id.clone();
            let job_id = job.id.clone();
            let source_hash = crate::auth::hash(&selected.path);
            let playback_version = selected.playback_version;
            let stored=state.db.run(move|c|{
                let status:String=c.query_row("SELECT status FROM jobs WHERE id=?1",[job_id],|r|r.get(0))?;
                if status!="running" {return Ok(None)}
                if source(c,&id,local,&policy)?.as_ref()!=Some(&selected) {return Ok(Some(false));}
                let data:String=c.query_row("SELECT provider_data FROM tracks WHERE id=?1",[&id],|r|r.get(0))?;
                let mut data:Value=serde_json::from_str(&data)?;
                if !data.is_object(){data=json!({});}
                let measured=json!({"integrated_lufs":integrated,"true_peak_dbfs":true_peak,"source":"ffmpeg_loudnorm_ebu_r128","cache_id":selected.cache_id,"source_path_sha256":source_hash,"source_size":source_stamp.0,"source_mtime":chrono::DateTime::<chrono::Utc>::from(source_stamp.1).to_rfc3339(),"analyzed_at":chrono::Utc::now().timestamp()});
                if !data["loudness_versions"].is_object(){data["loudness_versions"]=json!({});}
                data["loudness_versions"][source_hash]=measured.clone();
                if playback_version {
                    if !data["analysis"].is_object(){data["analysis"]=json!({});}
                    data["analysis"]["loudness"]=measured;
                }
                let manual=data["integrated_lufs_source"]=="manual" || data["provenance"]["integrated_lufs"]["source"]=="manual"
                    || data["manual_corrections"].get("integrated_lufs").is_some();
                if playback_version && !manual {
                    data["integrated_lufs"]=json!(integrated);
                    if !data["provenance"].is_object(){data["provenance"]=json!({});}
                    data["provenance"]["integrated_lufs"]=json!({"source":"ffmpeg_loudnorm_ebu_r128","analyzed_at":chrono::Utc::now().timestamp()});
                }
                c.execute("UPDATE tracks SET provider_data=?1 WHERE id=?2",params![serde_json::to_string(&data)?,id])?;Ok(Some(true))
            }).await?;
            match stored {
                None => return Ok(()),
                Some(false) => {
                    finish(
                        state,
                        &job.id,
                        "pending",
                        Some("曲目来源已变化，等待重新分析".into()),
                    )
                    .await?;
                    return Ok(());
                }
                Some(true) => {}
            }
            if playback_version {
                let track=crate::library::get_track(state,job.track_id.clone()).await.map_err(|e|anyhow::anyhow!(e.2))?;
                if let Some(integrated)=track.integrated_lufs {
                    publish(state,&track,Measurement { integrated, true_peak: Some(true_peak) }).await;
                }
            }
            finish(state, &job.id, "completed", None).await?;
            state.emit("library", json!({}));
        }
    }
    Ok(())
}

pub(crate) fn update_track(track: &mut crate::models::Track, measured: &crate::models::Track) -> bool {
    if track.id != measured.id { return false; }
    let analysis = &measured.provider_data["analysis"]["loudness"];
    if track.integrated_lufs == measured.integrated_lufs && &track.provider_data["analysis"]["loudness"] == analysis {
        return false;
    }
    track.integrated_lufs = measured.integrated_lufs;
    track.provider_data["analysis"]["loudness"] = analysis.clone();
    true
}

async fn publish(state: &AppState, track: &crate::models::Track, measurement: Measurement) {
    let _ = state.loudness.updates.send((track.id.clone(), measurement));
    let _guard = state.zone_lock.lock().await;
    let mut recipients = crate::sequence::update_loudness(state, track).await;
    let mut changed = false;
    {
        let mut zones = state.zones.write().await;
        for zone in zones.values_mut() {
            let mut queue_changed = false;
            for entry in &mut zone.queue { queue_changed |= update_track(entry, track); }
            if queue_changed { zone.queue_revision = zone.queue_revision.saturating_add(1); }
            let current = zone.track.as_mut().is_some_and(|current| update_track(current, track));
            changed |= queue_changed || current;
            if current {
                if let Some(id) = zone.playback_id.as_ref() {
                    recipients.push((zone.endpoint_id.clone(), id.clone()));
                }
            }
        }
    }
    if changed {
        state.emit("zones_changed",json!({}));
        state.loudness.wake.notify_one();
    }
    let bridges = state.bridges.read().await;
    for (endpoint, id) in recipients {
        if let Some(tx) = bridges.get(&endpoint) {
            let _ = tx.try_send(json!({"type":"loudness","track_id":track.id,"playback_id":id,
                "integrated_lufs":measurement.integrated,"true_peak_dbfs":measurement.true_peak}));
        }
    }
    drop(bridges);
    crate::portable::publish_loudness(state, track, measurement).await;
}

fn file_stamp(source: &str) -> anyhow::Result<(u64, std::time::SystemTime)> {
    let metadata = std::fs::metadata(source)?;
    Ok((metadata.len(), metadata.modified()?))
}

async fn analyze_file(state: &AppState, job: &Job, source: &str) -> anyhow::Result<Analysis> {
    if !FsPath::new(source).is_file() {
        anyhow::bail!("local analysis source unavailable")
    }
    if defer_for_playback(state,&job.track_id).await {
        return Ok(Analysis::Deferred);
    }
    let threads=if playback_targets(state).await.0 { 1 } else {
        state.setting("analysis_threads",json!(1)).await.as_u64().unwrap_or(1).min(std::thread::available_parallelism().map_or(1,|n|n.get() as u64))
    };
    let threads=threads.to_string();
    let ffmpeg = std::env::var_os("RMUSIC_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    #[cfg(unix)]
    let mut command = { let mut cmd=Command::new("nice"); cmd.args(["-n","10"]).arg(ffmpeg); cmd };
    #[cfg(not(unix))]
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "info",
            "-threads",
            &threads,
            "-filter_threads",
            &threads,
            "-i",
            source,
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-af",
            "loudnorm=I=-18:TP=-1:LRA=11:print_format=json",
            "-threads",
            &threads,
            "-f",
            "null",
            "-",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn()?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("missing analysis stderr"))?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut poll = tokio::time::interval(Duration::from_millis(250));
    let mut stderr_done = false;
    let status = loop {
        tokio::select! {
            status=child.wait()=>break status?,
            n=stderr.read(&mut chunk),if !stderr_done=>{let n=n?;if n==0 {stderr_done=true;}else{bytes.extend_from_slice(&chunk[..n]);if bytes.len()>65536 {bytes.drain(..bytes.len()-65536);}}},
            _=poll.tick()=>{
                let disposition=if state.resources.over_budget()||defer_for_playback(state,&job.track_id).await {Some(Analysis::Deferred)} else {
                    let id=job.id.clone();let status=state.db.run(move|c|Ok(c.query_row("SELECT status FROM jobs WHERE id=?1",[id],|r|r.get::<_,String>(0))?)).await?;
                    match status.as_str(){"paused"=>Some(Analysis::Paused),"cancelled"=>Some(Analysis::Cancelled),_=>None}
                };
                if let Some(disposition)=disposition {let _=child.start_kill();let _=child.wait().await;return Ok(disposition)}
            }
        }
    };
    if !stderr_done {
        loop {
            let n = stderr.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            if bytes.len() > 65536 {
                bytes.drain(..bytes.len() - 65536);
            }
        }
    }
    if !status.success() {
        anyhow::bail!("FFmpeg loudness analysis failed")
    }
    let text = String::from_utf8_lossy(&bytes);
    let end = text
        .rfind('}')
        .ok_or_else(|| anyhow::anyhow!("missing loudness result"))?;
    let start = text[..end]
        .rfind('{')
        .ok_or_else(|| anyhow::anyhow!("invalid loudness result"))?;
    let json: Value = serde_json::from_str(&text[start..=end])?;
    let finite = |name: &str| {
        json[name]
            .as_f64()
            .or_else(|| json[name].as_str()?.parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    Ok(match (finite("input_i"), finite("input_tp")) {
        (Some(integrated), Some(true_peak)) => Analysis::Measured {
            integrated,
            true_peak,
        },
        _ => Analysis::Unable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::{body::Body, routing::post, Router};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    async fn fixture(silent: bool) -> (tempfile::TempDir, AppState) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.wav");
        let mut data = Vec::new();
        for n in 0..48000 {
            let sample = if silent {
                0
            } else {
                (0.1 * (std::f64::consts::TAU * 1000. * n as f64 / 48000.).sin() * 32767.).round()
                    as i16
            };
            for _ in 0..2 {
                data.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&192000u32.to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        tokio::fs::write(&path, wav).await.unwrap();
        let db = crate::db::Db::open(&directory.path().join("library.db")).unwrap();
        let source = path.to_string_lossy().into_owned();
        db.run(move|c|{c.execute("INSERT INTO tracks(id,path,title,artist,album,album_artist,source,format,recording_id,created_at)VALUES('track',?1,'Test','Test','Test','Test','local','wav','recording',0)",[source])?;c.execute("INSERT INTO users(id,username,password,role)VALUES('u','test','unused','member')",[])?;c.execute("INSERT INTO sessions(hash,user_id,expires,scopes,created_at)VALUES(?1,'u',?2,'[\"library\"]',0)",params![crate::auth::hash("test-token"),chrono::Utc::now().timestamp()+60])?;Ok(())}).await.unwrap();
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
        (directory, state)
    }
    async fn enqueue(state: AppState) -> Job {
        let response = Router::new()
            .route("/tracks/{id}/analyze", post(analyze))
            .with_state(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/tracks/track/analyze")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let data: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        Job {
            id: data["id"].as_str().unwrap().into(),
            track_id: "track".into(),
        }
    }
    #[tokio::test]
    async fn discovery_reads_do_not_wait_for_the_writer_queue() {
        let (_directory,state)=fixture(false).await;
        state.db.run(|c|{c.execute("UPDATE tracks SET provider_data='{\"integrated_lufs\":-20}'",[])?;Ok(())}).await.unwrap();
        let db=state.db.clone();let (entered,ready)=tokio::sync::oneshot::channel();
        let writer=tokio::spawn(async move{db.run(move|_|{let _=entered.send(());std::thread::sleep(Duration::from_millis(400));Ok(())}).await});
        ready.await.unwrap();
        let discovery=tokio::time::timeout(Duration::from_millis(150),enqueue_missing(&state)).await;
        writer.await.unwrap().unwrap();
        discovery.expect("只读扫描不应等待写队列").unwrap();
    }
    #[tokio::test]
    async fn completion_automatic_current_track_analysis_runs_without_waiting_for_playback_end(){
        let (_directory,state)=fixture(false).await;
        let track=crate::library::get_track(&state,"track".into()).await.unwrap();
        {let mut zones=state.zones.write().await;let zone=zones.get_mut("zone-local").unwrap();zone.status="playing".into();zone.track=Some(track.clone());zone.queue=vec![track];zone.dsp=json!({"enabled":true,"normalize":true});}
        let (busy, targets)=playback_targets(&state).await;
        let _=next_job(&state,busy,targets).await.unwrap().unwrap();
        let job=state.db.run(|c|Ok(c.query_row("SELECT id,cursor,priority FROM jobs WHERE kind='loudness'",[],|r|Ok((Job{id:r.get(0)?,track_id:r.get(1)?},r.get::<_,i64>(2)?)))?)).await.unwrap();
        assert_eq!(job.1,100);process(&state,&job.0).await.unwrap();
        assert!(crate::library::get_track(&state,"track".into()).await.unwrap().integrated_lufs.is_some());
        assert_eq!(state.zones.read().await["zone-local"].status,"playing");
    }
    #[tokio::test]
    async fn next_complete_cloud_track_is_prioritized_and_partial_cache_is_ignored() {
        let (directory,state)=fixture(false).await;
        let path=directory.path().join("sample.wav");
        let bytes=std::fs::metadata(&path).unwrap().len() as i64;
        let path=path.to_string_lossy().into_owned();
        state.db.run(move |c| {
            for (id,complete) in [("complete",true),("partial",false)] {
                c.execute("INSERT INTO tracks(id,title,artist,album,album_artist,source,format,recording_id,created_at) VALUES(?1,?1,'','','','qqmusic','wav',?1,0)",[id])?;
                c.execute("INSERT INTO cache_entries(id,track_id,path,format,size,total_size,complete,last_access,etag) VALUES(?1,?1,?2,'wav',?3,?3,?4,0,?5)",
                    params![id,path,bytes,complete,"\"complete-qnative\""])?;
            }
            Ok(())
        }).await.unwrap();
        enqueue_missing(&state).await.unwrap();
        let counts=state.db.read(|c|Ok(c.query_row("SELECT count(*),sum(cursor='partial') FROM jobs WHERE kind='loudness'",[],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?)))?)).await.unwrap();
        assert_eq!(counts,(2,0));
        let mut current=crate::library::get_track(&state,"track".into()).await.unwrap();
        current.integrated_lufs=Some(-24.);
        let next=crate::library::get_track(&state,"complete".into()).await.unwrap();
        {
            let mut zones=state.zones.write().await;
            let zone=zones.get_mut("zone-local").unwrap();
            zone.status="playing".into();zone.track=Some(current.clone());zone.queue=vec![current,next];
            zone.dsp=json!({"enabled":true,"normalize":true});
        }
        let (busy,targets)=playback_targets(&state).await;
        let job=next_job(&state,busy,targets).await.unwrap().unwrap();
        assert_eq!(job.track_id,"complete");
        process(&state,&job).await.unwrap();
        let zone=&state.zones.read().await["zone-local"];
        assert_eq!(zone.status,"playing");
        assert_eq!(zone.track.as_ref().unwrap().id,"track");
        assert!(zone.queue[1].integrated_lufs.is_some());
        assert_eq!(zone.queue[1].provider_data["analysis"]["loudness"]["cache_id"],"complete");
    }

    #[tokio::test]
    async fn idle_worker_backfills_missing_analysis_without_manual_requests() {
        let (_directory,state)=fixture(false).await;
        spawn(state.clone());
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                let track=crate::library::get_track(&state,"track".into()).await.unwrap();
                if track.integrated_lufs.is_some() { return; }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }).await.expect("idle Core should automatically analyze the eligible file");
    }
    #[tokio::test]
    async fn real_wav_analysis_stores_measurement_and_provenance() {
        let (_directory, state) = fixture(false).await;
        let job = enqueue(state.clone()).await;
        process(&state, &job).await.unwrap();
        let track = crate::library::get_track(&state, "track".into())
            .await
            .unwrap();
        let lufs = track.provider_data["integrated_lufs"].as_f64().unwrap();
        assert!((-22. ..-18.).contains(&lufs));
        assert!(
            track.provider_data["analysis"]["loudness"]["true_peak_dbfs"]
                .as_f64()
                .unwrap()
                < -19.
        );
        assert_eq!(
            track.provider_data["provenance"]["integrated_lufs"]["source"],
            "ffmpeg_loudnorm_ebu_r128"
        );
    }
    #[tokio::test]
    async fn silence_is_unable_and_never_fabricates_lufs() {
        let (_directory, state) = fixture(true).await;
        let job = enqueue(state.clone()).await;
        process(&state, &job).await.unwrap();
        let id = job.id;
        let status = state
            .db
            .run(move |c| {
                Ok(
                    c.query_row("SELECT status FROM jobs WHERE id=?1", [id], |r| {
                        r.get::<_, String>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(status, "unable");
        assert!(crate::library::get_track(&state, "track".into())
            .await
            .unwrap()
            .provider_data
            .get("integrated_lufs")
            .is_none());
    }
    #[tokio::test]
    async fn playback_defers_analysis_without_touching_source_metadata() {
        let (_directory, state) = fixture(false).await;
        let job = enqueue(state.clone()).await;
        state
            .zones
            .write()
            .await
            .get_mut("zone-local")
            .unwrap()
            .status = "playing".into();
        process(&state, &job).await.unwrap();
        let id = job.id;
        let status = state
            .db
            .run(move |c| {
                Ok(
                    c.query_row("SELECT status FROM jobs WHERE id=?1", [id], |r| {
                        r.get::<_, String>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(status, "pending");
    }
    #[tokio::test]
    async fn manual_lufs_correction_is_preserved() {
        let (_directory, state) = fixture(false).await;
        state.db.run(|c|{c.execute("UPDATE tracks SET provider_data=?1 WHERE id='track'",[json!({"integrated_lufs":-15.0,"provenance":{"integrated_lufs":{"source":"manual"}}}).to_string()])?;Ok(())}).await.unwrap();
        let job = enqueue(state.clone()).await;
        process(&state, &job).await.unwrap();
        let data = crate::library::get_track(&state, "track".into())
            .await
            .unwrap()
            .provider_data;
        assert_eq!(data["integrated_lufs"], -15.0);
        assert!(data["analysis"]["loudness"]["integrated_lufs"].is_number());
    }
    #[tokio::test]
    async fn unavailable_source_finishes_and_paused_job_stays_paused() {
        let (_directory, state) = fixture(false).await;
        let job = enqueue(state.clone()).await;
        state
            .db
            .run(|c| {
                c.execute("UPDATE tracks SET available=0 WHERE id='track'", [])?;
                Ok(())
            })
            .await
            .unwrap();
        process(&state, &job).await.unwrap();
        let id = job.id.clone();
        let status = state
            .db
            .run(move |c| {
                Ok(
                    c.query_row("SELECT status FROM jobs WHERE id=?1", [id], |r| {
                        r.get::<_, String>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(status, "waiting");
        let id = job.id.clone();
        state
            .db
            .run(move |c| {
                c.execute("UPDATE jobs SET status='paused' WHERE id=?1", [id])?;
                Ok(())
            })
            .await
            .unwrap();
        process(&state, &job).await.unwrap();
        let id = job.id;
        assert_eq!(
            state
                .db
                .run(move |c| Ok(c.query_row(
                    "SELECT status FROM jobs WHERE id=?1",
                    [id],
                    |r| r.get::<_, String>(0)
                )?))
                .await
                .unwrap(),
            "paused"
        );
    }
    #[test]
    fn file_stamp_detects_source_content_length_change() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.wav");
        std::fs::write(&source, b"one").unwrap();
        let source = source.to_str().unwrap();
        let old = file_stamp(source).unwrap();
        std::fs::write(source, b"changed").unwrap();
        assert_ne!(old, file_stamp(source).unwrap());
    }
}
