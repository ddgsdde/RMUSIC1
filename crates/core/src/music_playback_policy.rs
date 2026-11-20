//! Opt-in selection for a new play command. Active streams and existing queues are never rewritten.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::Track,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub preferred_track_id: Option<String>,
    pub apply_to_new_playback: bool,
    pub automatic_fallback: bool,
    pub same_release: bool,
    pub lossless_only: bool,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            preferred_track_id: None,
            apply_to_new_playback: false,
            automatic_fallback: false,
            same_release: true,
            lossless_only: true,
        }
    }
}
fn stored(c: &rusqlite::Connection, user: &str, key: &str) -> anyhow::Result<(u64, Policy)> {
    Ok(c.query_row(
        "SELECT revision,data FROM music_playback_policies WHERE user_id=?1 AND entity_id=?2",
        params![user, key],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?)),
    )
    .optional()?
    .map(|(r, v)| Ok::<_, anyhow::Error>((r, serde_json::from_str(&v)?)))
    .transpose()?
    .unwrap_or_default())
}
pub async fn get(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    Ok(Json(s.db.read(move|c|{let key=crate::music_versions::anchor(c,&id)?;let(revision,policy)=match &key{Some(key)=>stored(c,&a.id,key)?,None=>(0,Policy::default())};Ok(json!({"revision":revision,"policy":policy,"identity_resolved":key.is_some(),"preferred_track":policy.preferred_track_id.as_ref().and_then(|id|c.query_row("SELECT * FROM tracks WHERE id=?1",[id],Track::from_row).ok()),"applies_to":"new_explicit_play_commands","active_streams_unchanged":true}))}).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    expected_revision: u64,
    policy: Policy,
}
pub async fn put(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(q): Json<Edit>,
) -> ApiResult<Json<Value>> {
    a.require("control")?;
    if q.expected_revision >= i64::MAX as u64 {
        return Err(ApiError::bad("策略修订无效"));
    }
    let value=s.db.run(move|c|{let tx=c.transaction()?;let Some(key)=crate::music_versions::anchor(&tx,&id)?else{return Ok(Err(ApiError::bad("请先唯一确认录音身份")))};
 if let Some(target)=&q.policy.preferred_track_id{if crate::music_versions::anchor(&tx,target)?!=Some(key.clone()){return Ok(Err(ApiError::bad("首选版本不属于同一已确认录音")));}}
 if stored(&tx,&a.id,&key)?.0!=q.expected_revision{return Ok(Err(ApiError(StatusCode::CONFLICT,"version_policy_conflict".into(),"播放策略已变化，请重新读取".into())));}
 tx.execute("INSERT INTO music_playback_policies(user_id,entity_id,revision,data)VALUES(?1,?2,1,?3) ON CONFLICT(user_id,entity_id)DO UPDATE SET revision=revision+1,data=excluded.data",params![a.id,key,serde_json::to_string(&q.policy).unwrap()])?;let revision=stored(&tx,&a.id,&key)?.0;tx.commit()?;Ok(Ok(json!({"revision":revision,"policy":q.policy,"active_streams_unchanged":true})))
 }).await??;
    Ok(Json(value))
}
fn release(c: &rusqlite::Connection, t: &Track) -> anyhow::Result<Option<String>> {
    let graph = crate::music_identity::graph(c, &t.id)?;
    let resolved = crate::identity_workflow::resolution(&graph);
    Ok(resolved["release"]["selected"]["id"]
        .as_str()
        .map(String::from))
}
fn compatible(
    c: &rusqlite::Connection,
    origin: &Track,
    next: &Track,
    p: &Policy,
) -> anyhow::Result<bool> {
    if p.same_release && (release(c, origin)?.is_none() || release(c, origin)? != release(c, next)?)
    {
        return Ok(false);
    }
    // User labels do not certify fidelity, but an explicit conflicting edition must not be ignored.
    let edition = |id: &str| -> rusqlite::Result<Option<String>> {
        c.query_row(
            "SELECT kind FROM music_editions WHERE track_id=?1 AND kind!='unspecified'",
            [id],
            |r| r.get(0),
        )
        .optional()
    };
    if let (Some(a), Some(b)) = (edition(&origin.id)?, edition(&next.id)?) {
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
fn lossless(format: &str) -> bool {
    matches!(
        format.to_ascii_lowercase().as_str(),
        "flac" | "alac" | "wav" | "aiff" | "aif" | "pcm" | "dsf" | "dff" | "dsd"
    )
}
async fn probe(s: &AppState, t: &Track, origin: &Track, strict: bool) -> ApiResult<()> {
    if !t.available {
        return Err(ApiError::bad("版本不可用"));
    }
    if t.source != "local" {
        crate::plugin::check_enabled(s, &t.source).await?;
    }
    let quality = crate::catalog::quality(s, &t.source).await;
    let cache_id = t.id.clone();
    let q = quality.clone();
    let suffix = format!("%-q{q}\"");
    let cached = if t.source != "local" {
        s.db.read(move |c| {
            Ok(c.query_row(
                crate::media::COMPLETE_CACHE_QUERY,
                params![cache_id, suffix, q],
                |r| {
                    Ok((
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(0)?,
                    ))
                },
            )
            .optional()?)
        })
        .await?
    } else {
        None
    };
    let cached = match cached {
        Some((path, size, cache_id))
            if size > 0
                && tokio::fs::metadata(&path)
                    .await
                    .ok()
                    .is_some_and(|m| m.is_file() && m.len() == size as u64) =>
        {
            Some((path, cache_id))
        }
        _ => None,
    };
    let cached_enhanced = if let Some((_, id)) = &cached {
        let id = id.clone();
        s.db.read(move |c| {
            Ok(c.query_row(
                "SELECT json_extract(data,'$.enhanced')=1 FROM cache_quality WHERE cache_id=?1",
                [id],
                |r| r.get::<_, Option<bool>>(0),
            )
            .optional()?
            .flatten()
            .unwrap_or(false))
        })
        .await?
    } else {
        false
    };
    let local_path = if t.source == "local" {
        Some(crate::library::local_path(s, t.id.clone()).await?)
    } else {
        cached.map(|(path, _)| path)
    };
    let (format, rate, bits, channels) = if let Some(path) = local_path {
        let _active = crate::media::retain_path(s, &path).await?;
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|_| ApiError::missing())?;
        if !meta.is_file() || meta.len() == 0 {
            return Err(ApiError::missing());
        }
        if strict {
            if cached_enhanced
                || t.provider_data["cache_quality"]["enhanced"] == true
                || t.provider_data["enhanced"] == true
                || t.provider_data["upscaled"] == true
            {
                return Err(ApiError::bad("候选经过音质增强，不能作为无损回退"));
            }
            let info = rmusic_audio::probe::probe_async(std::path::Path::new(&path))
                .await
                .map_err(|_| ApiError::bad("候选文件音质探测失败"))?;
            (
                info.format,
                info.sample_rate,
                u32::from(info.bit_depth),
                u32::from(info.channels),
            )
        } else {
            (t.format.clone(), t.sample_rate, t.bit_depth, t.channels)
        }
    } else {
        let quality = crate::catalog::quality(s, &t.source).await;
        let (v, response) = crate::source_stream::open(s, t, &quality, Some("bytes=0-0")).await?;
        if !response.status().is_success() {
            return Err(ApiError::bad("版本音源不可用"));
        }
        drop(response);
        if strict && (v["verified"] != true || v["enhanced"] == true || v["upscaled"] == true) {
            return Err(ApiError::bad("候选音质尚未实测确认"));
        }
        (
            v["format"].as_str().unwrap_or("").to_owned(),
            v["sample_rate"].as_u64().unwrap_or(0) as u32,
            v["bit_depth"].as_u64().unwrap_or(0) as u32,
            v["channels"].as_u64().unwrap_or(0) as u32,
        )
    };
    if strict
        && (!lossless(&format)
            || rate == 0
            || bits == 0
            || channels == 0
            || rate < origin.sample_rate
            || bits < origin.bit_depth
            || (origin.channels > 0 && channels != origin.channels)
            || (matches!(origin.format.as_str(), "dsf" | "dff" | "dsd")
                && !matches!(format.as_str(), "dsf" | "dff" | "dsd")))
    {
        return Err(ApiError::bad("候选不满足无损或原音质要求"));
    }
    Ok(())
}
pub async fn select(s: &AppState, user: &str, original: Track) -> ApiResult<(Track, bool)> {
    let id = original.id.clone();
    let user = user.to_owned();
    let lookup = user.clone();
    let snapshot=s.db.read(move|c|{let Some(key)=crate::music_versions::anchor(c,&id)?else{return Ok(None)};let(rev,p)=stored(c,&lookup,&key)?;if !p.apply_to_new_playback&&!p.automatic_fallback{return Ok(None)};
 let preferred=p.preferred_track_id.as_ref().filter(|_|p.apply_to_new_playback).filter(|id|crate::music_versions::anchor(c,id).ok().flatten()==Some(key.clone())).and_then(|id|c.query_row("SELECT * FROM tracks WHERE id=?1",[id],Track::from_row).ok());
 let origin=c.query_row("SELECT * FROM tracks WHERE id=?1",[&id],Track::from_row)?;
 let mut candidates=vec![];if p.automatic_fallback{let cte=crate::music_version_query::resolved_for_key("recording","?1");let sql=format!("{cte} SELECT t.* FROM tracks t JOIN resolved r ON r.track_id=t.id WHERE r.entity_id=?1 AND t.available=1 ORDER BY (t.source='local') DESC,t.id LIMIT 32");for t in c.prepare(&sql)?.query_map([&key],Track::from_row)?{let t=t?;if compatible(c,&origin,&t,&p)?{candidates.push(t);}}}
 Ok(Some((key,rev,p,preferred,candidates)))
 }).await?;
    let Some((key, revision, policy, preferred, candidates)) = snapshot else {
        return Ok((original, false));
    };
    if policy.apply_to_new_playback && policy.preferred_track_id.is_some() && preferred.is_none() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "version_preference_invalid".into(),
            "播放首选已删除或录音身份已变化，请重新选择或清除首选".into(),
        ));
    }
    let mut chosen = preferred.unwrap_or_else(|| original.clone());
    if !policy.automatic_fallback && !chosen.available {
        return Err(ApiError::bad(
            "播放首选当前不可用，请手动选择版本或启用自动回退",
        ));
    }
    let mut reason = if chosen.id == original.id {
        "requested"
    } else {
        "preferred"
    };
    if policy.automatic_fallback {
        let first = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            probe(s, &chosen, &original, false),
        )
        .await;
        if !matches!(first, Ok(Ok(()))) {
            let mut replacement = None;
            for candidate in candidates.into_iter().filter(|t| t.id != chosen.id).take(3) {
                if matches!(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(8),
                        probe(s, &candidate, &original, policy.lossless_only)
                    )
                    .await,
                    Ok(Ok(()))
                ) {
                    replacement = Some(candidate);
                    break;
                }
            }
            chosen = replacement.ok_or_else(|| {
                ApiError(
                    StatusCode::BAD_GATEWAY,
                    "version_fallback_unavailable".into(),
                    "原版本不可用，未找到符合规则的替代版本；请手动选择".into(),
                )
            })?;
            reason = "verified_fallback";
        }
    }
    let requested = original.id.clone();
    let selected = chosen.id.clone();
    let recheck_original = original.clone();
    let recheck_chosen = chosen.clone();
    let recheck_policy = policy.clone();
    let is_fallback = reason == "verified_fallback";
    let valid =
        s.db.read(move |c| {
            Ok(stored(c, &user, &key)?.0 == revision
                && crate::music_versions::anchor(c, &requested)? == Some(key.clone())
                && crate::music_versions::anchor(c, &selected)? == Some(key)
                && (!is_fallback
                    || compatible(c, &recheck_original, &recheck_chosen, &recheck_policy)?))
        })
        .await?;
    if !valid {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "version_selection_changed".into(),
            "录音身份或播放策略已变化，请重新播放".into(),
        ));
    }
    if chosen.source != "local" {
        crate::plugin::check_enabled(s, &chosen.source).await?;
    }
    if chosen.provider_data.as_object().is_none() {
        chosen.provider_data = json!({});
    }
    chosen.provider_data["playback_selection"] = json!({"requested_track_id":original.id,"selected_track_id":chosen.id,"reason":reason,"policy_revision":revision,"same_recording_confirmed":true,"scope":"new_playback_only"});
    Ok((chosen, true))
}
