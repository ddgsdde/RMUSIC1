//! Persistent listening statistics from actual endpoint progress, never wall time alone.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::Zone,
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{Days, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::OnceLock;
pub mod offline;
pub mod measured;
pub(crate) mod output;
pub(crate) mod group_output;
#[cfg(test)]
#[path = "stats/offline_tests.rs"]
mod offline_tests;

fn runtime_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

#[derive(Debug)]
struct Session {
    id: String,
    track_id: String,
    title: String,
    artist: String,
    cover_url: Option<String>,
    duration: f64,
    last_report_at: i64,
    last_position: f64,
    last_status: String,
    runtime: String,
    seconds: f64,
    counted: bool,
}
fn active(c: &Connection, zone: &str) -> anyhow::Result<Option<Session>> {
    Ok(c.query_row("SELECT id,track_id,title,artist,cover_url,duration,last_report_at,last_position,last_status,runtime_id,play_seconds,counted FROM play_sessions WHERE zone_id=?1 AND finished_at IS NULL",[zone],|r|Ok(Session {
        id:r.get(0)?,track_id:r.get(1)?,title:r.get(2)?,artist:r.get(3)?,cover_url:r.get(4)?,duration:r.get(5)?,last_report_at:r.get(6)?,last_position:r.get(7)?,last_status:r.get(8)?,runtime:r.get(9)?,seconds:r.get(10)?,counted:r.get(11)?,
    })).optional()?)
}
fn threshold(duration: f64) -> f64 {
    if duration.is_finite() && duration > 0. {
        (duration / 2.).clamp(5., 30.)
    } else {
        30.
    }
}
fn day(time_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time_ms)
        .unwrap_or_default()
        .format("%Y-%m-%d")
        .to_string()
}
fn remember_track(tx:&Transaction<'_>,track:&crate::models::Track)->anyhow::Result<()> {
    let artist=if track.album_artist.trim().is_empty(){&track.artist}else{&track.album_artist};
    let source=track.provider_data.get("cached_from").and_then(|v|v.get("source")).and_then(Value::as_str).unwrap_or(&track.source);
    tx.execute("INSERT INTO listening_metadata(track_id,album,album_artist,source) VALUES(?1,?2,?3,?4) ON CONFLICT(track_id) DO UPDATE SET album=excluded.album,album_artist=excluded.album_artist,source=excluded.source",params![track.id,track.album.trim(),artist.trim(),source])?;
    Ok(())
}
fn aggregate(
    tx: &Transaction<'_>,
    s: &Session,
    at_ms: i64,
    seconds: f64,
    plays: i64,
    skips: i64,
) -> anyhow::Result<()> {
    aggregate_impl(tx,s,at_ms,seconds,plays,skips,true)
}
fn aggregate_utc_only(tx:&Transaction<'_>,s:&Session,at_ms:i64,seconds:f64,plays:i64,skips:i64)->anyhow::Result<()>{aggregate_impl(tx,s,at_ms,seconds,plays,skips,false)}
fn aggregate_impl(tx:&Transaction<'_>,s:&Session,at_ms:i64,seconds:f64,plays:i64,skips:i64,timestamped:bool)->anyhow::Result<()> {
    if crate::accounts::listening::aggregate(tx,&s.id,&s.track_id,&s.title,&s.artist,&s.cover_url,at_ms,seconds,plays,skips)?{return Ok(());}
    let last = if seconds > 0. {
        Some(at_ms / 1000)
    } else {
        None
    };
    tx.execute("INSERT INTO track_stats(track_id,play_count,skip_count,play_seconds,last_played_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(track_id) DO UPDATE SET play_count=play_count+excluded.play_count,skip_count=skip_count+excluded.skip_count,play_seconds=play_seconds+excluded.play_seconds,last_played_at=MAX(COALESCE(last_played_at,0),COALESCE(excluded.last_played_at,0))",params![s.track_id,plays,skips,seconds,last])?;
    tx.execute("INSERT INTO daily_stats(day,track_id,title,artist,cover_url,play_count,skip_count,play_seconds,last_played_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(day,track_id) DO UPDATE SET title=excluded.title,artist=excluded.artist,cover_url=excluded.cover_url,play_count=play_count+excluded.play_count,skip_count=skip_count+excluded.skip_count,play_seconds=play_seconds+excluded.play_seconds,last_played_at=MAX(COALESCE(last_played_at,0),COALESCE(excluded.last_played_at,0))",params![day(at_ms),s.track_id,s.title,s.artist,s.cover_url,plays,skips,seconds,last])?;
    if timestamped {tx.execute("INSERT INTO listening_buckets(minute,track_id,title,artist,cover_url,play_count,skip_count,play_seconds,last_played_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(minute,track_id) DO UPDATE SET title=excluded.title,artist=excluded.artist,cover_url=excluded.cover_url,play_count=play_count+excluded.play_count,skip_count=skip_count+excluded.skip_count,play_seconds=play_seconds+excluded.play_seconds,last_played_at=MAX(COALESCE(last_played_at,0),COALESCE(excluded.last_played_at,0))",params![at_ms.div_euclid(60000),s.track_id,s.title,s.artist,s.cover_url,plays,skips,seconds,last])?;}
    Ok(())
}
/// A short, deliberate navigation is a negative signal. Stopping is neutral.
fn early_skip(reason: &str, heard: f64, position: f64, duration: f64) -> bool {
    let cutoff = if duration > 0. { (duration * 0.25).min(30.) } else { 15. };
    matches!(reason, "next" | "previous" | "replaced")
        && heard > 0. && heard < cutoff && position < cutoff
}
fn close(tx: &Transaction<'_>, s: &Session, now: i64, reason: &str) -> anyhow::Result<()> {
    let ended:Option<(f64,i64)>=tx.query_row("SELECT e.position,e.updated_at FROM listen_session_sources p JOIN listen_source_ends e ON e.playback_id=p.playback_id WHERE p.session_id=?1",[&s.id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let reason=if ended.is_some(){"ended"}else{reason};
    let finished_at=ended.map(|(_,at)|at.min(now)).unwrap_or(now);
    let position=ended.map(|(position,_)|position.max(s.last_position)).unwrap_or(s.last_position);
    let skipped = early_skip(reason, s.seconds, s.last_position, s.duration);
    if !measured::close(tx,s,now,reason)?&&skipped {
        aggregate(tx, s, now, 0., 0, 1)?;
    }
    tx.execute("UPDATE play_sessions SET finished_at=?2,finish_reason=?3,last_status='stopped',last_position=?4 WHERE id=?1 AND finished_at IS NULL",params![s.id,finished_at,reason,position])?;
    Ok(())
}

/// Call for measured endpoint progress and pause/resume transitions. A seek must use reposition.
pub async fn record_progress(state: &AppState, zone: &Zone) -> anyhow::Result<()> {
    let z = zone.clone();
    let now = Utc::now().timestamp_millis();
    let runtime = runtime_id().to_owned();
    let counted = state
        .db
        .run(move |c| progress(c, &z, now, &runtime, false))
        .await?;
    if counted {
        state.emit(
            "stats",
            json!({"zone_id":zone.id,"track_id":zone.track.as_ref().map(|t|&t.id)}),
        );
    }
    Ok(())
}
/// Call after every explicit seek, including a small seek. It changes only the progress baseline.
pub async fn reposition(state: &AppState, zone: &Zone) -> anyhow::Result<()> {
    let z = zone.clone();
    let now = Utc::now().timestamp_millis();
    let runtime = runtime_id().to_owned();
    state
        .db
        .run(move |c| progress(c, &z, now, &runtime, true))
        .await?;
    Ok(())
}
/// Control changes carry no device samples. Keep the last confirmed position and its timestamp.
pub async fn control_status(state:&AppState,zone:&Zone)->anyhow::Result<()> {
    let z=zone.clone();let now=Utc::now().timestamp_millis();let runtime=runtime_id().to_owned();
    state.db.run(move |c| {
        if let Some(session)=active(c,&z.id)?.filter(|s|s.runtime==runtime&&z.track.as_ref().is_some_and(|t|t.id==s.track_id)) {
            c.execute("UPDATE play_sessions SET last_status=?2 WHERE id=?1",params![session.id,z.status])?;
        }else{progress(c,&z,now,&runtime,false)?;}
        Ok(())
    }).await
}
/// Pass the OLD zone before resetting its position/track. Idempotent if already finished.
/// ended/replay/disconnect/delete do not count as skips; next/previous/stop/replaced can.
pub async fn finish(state: &AppState, zone: &Zone, reason: &str) -> anyhow::Result<()> {
    let z = zone.clone();
    let reason = reason.to_owned();
    let now = Utc::now().timestamp_millis();
    let runtime = runtime_id().to_owned();
    let changed = state
        .db
        .run(move |c| finish_at(c, &z, now, &runtime, &reason))
        .await?;
    if changed {
        state.emit(
            "stats",
            json!({"zone_id":zone.id,"track_id":zone.track.as_ref().map(|t|&t.id)}),
        );
    }
    Ok(())
}
/// Continuous output's terminal source: use its actual device frames, not rounded catalog duration.
pub async fn finish_device(state:&AppState,zone:&Zone,frames:u64,rate:u32)->anyhow::Result<bool> {
    let zone=zone.clone();let id=zone.id.clone();let now=Utc::now().timestamp_millis();
    let applied=state.db.run(move |c|finish_device_at(c,&zone,frames,rate,now,runtime_id())).await?;
    if applied {state.emit("stats",json!({"zone_id":id}));}
    Ok(applied)
}
fn finish_device_at(c:&mut Connection,z:&Zone,frames:u64,rate:u32,now:i64,runtime:&str)->anyhow::Result<bool> {
    Ok(device_progress_at(c,z,frames,rate,now,runtime,true)?.is_some())
}
pub async fn record_device_progress(state:&AppState,zone:&Zone,frames:u64,rate:u32)->anyhow::Result<bool> {
    let z=zone.clone();let now=Utc::now().timestamp_millis();
    let result=state.db.run(move |c|device_progress_at(c,&z,frames,rate,now,runtime_id(),false)).await?;
    if result==Some(true){state.emit("stats",json!({"zone_id":zone.id}));}
    Ok(result.is_some())
}
/// One logical listening session for a cohort; caller derives seconds from confirmed device frames.
pub(crate) async fn record_group_progress(state:&AppState,zone:&Zone,owner:&str,seconds:f64,reason:Option<&str>)->anyhow::Result<bool> {
    if !seconds.is_finite()||!(0. ..=86400.).contains(&seconds){return Ok(false);}
    let z=zone.clone();let owner=owner.to_owned();let reason=reason.map(str::to_owned);let now=Utc::now().timestamp_millis();let runtime=runtime_id().to_owned();
    let applied=state.db.run(move|c|{let tx=c.transaction()?;
        if active(&tx,&z.id)?.is_none(){progress_tx(&tx,&z,now,&runtime,false,Some(0.))?;measured::issue_current(&tx,&owner,&z)?;}
        let changed=progress_tx(&tx,&z,now,&runtime,false,Some(seconds))?;
        if let Some(reason)=&reason{if let Some(session)=active(&tx,&z.id)?{close(&tx,&session,now,reason)?;}if reason=="ended"{if let Some(id)=&z.playback_id{measured::mark_ended(&tx,id,Some(z.position),now)?;}}}
        tx.commit()?;Ok(changed||reason.is_some())
    }).await?;
    if applied{state.emit("stats",json!({"zone_id":zone.id,"track_id":zone.track.as_ref().map(|t|&t.id)}));}Ok(applied)
}
/// A joined group output may confirm a tail after cancellation closed its original session.
pub(crate) async fn correct_group_progress(state:&AppState,zone:&Zone,seconds:f64)->anyhow::Result<bool> {
    if !seconds.is_finite()||!(0. ..=86400.).contains(&seconds){return Ok(false);}
    let z=zone.clone();let now=Utc::now().timestamp_millis();let runtime=runtime_id().to_owned();
    let applied=state.db.run(move|c|{let tx=c.transaction()?;let applied=correct_group_progress_tx(&tx,&z,seconds,now,&runtime)?;if applied{tx.commit()?;}Ok(applied)
    }).await?;
    if applied{state.emit("stats",json!({"zone_id":zone.id,"track_id":zone.track.as_ref().map(|t|&t.id)}));}Ok(applied)
}
fn correct_group_progress_tx(tx:&Transaction<'_>,z:&Zone,seconds:f64,now:i64,runtime:&str)->anyhow::Result<bool> {
    let Some(playback)=z.playback_id.as_deref()else{return Ok(false);};
        let session=tx.query_row("SELECT p.id,p.track_id,p.title,p.artist,p.cover_url,p.duration,p.last_report_at,p.last_position,p.last_status,p.runtime_id,p.play_seconds,p.counted FROM play_sessions p JOIN listen_session_sources src ON src.session_id=p.id WHERE p.zone_id=?1 AND src.playback_id=?2 AND p.finished_at IS NOT NULL",params![z.id,playback],|r|Ok(Session{id:r.get(0)?,track_id:r.get(1)?,title:r.get(2)?,artist:r.get(3)?,cover_url:r.get(4)?,duration:r.get(5)?,last_report_at:r.get(6)?,last_position:r.get(7)?,last_status:r.get(8)?,runtime:r.get(9)?,seconds:r.get(10)?,counted:r.get(11)?})).optional()?;
        let Some(mut session)=session else{return Ok(false);};
        if session.runtime!=runtime||z.track.as_ref().is_none_or(|t|t.id!=session.track_id)||now<session.last_report_at||z.position<session.last_position||z.position-session.last_position>(now-session.last_report_at) as f64/1000.+1.{return Ok(false);}
        let Some((credited,counted))=measured::credit(tx,playback,z.position-seconds,z.position,now)?else{return Ok(false);};
        tx.execute("UPDATE play_sessions SET play_seconds=play_seconds+?2,counted=MAX(counted,?3),last_position=?4,last_report_at=?5 WHERE id=?1",params![session.id,credited,counted,z.position,now])?;
        session.last_position=z.position;session.seconds+=credited;session.counted=counted;measured::close(tx,&session,now,"stop")?;
    Ok(true)
}
/// One virtual zone/source per ordered group occurrence, updated in the handoff transaction.
pub(crate) fn group_queue_tx(tx:&Transaction<'_>,z:&Zone,owner:&str,seconds:f64,now:i64,reason:Option<&str>)->anyhow::Result<()> {
    if !seconds.is_finite()||!(0. ..=86400.).contains(&seconds){anyhow::bail!("组队列实际帧无效");}
    let closed=tx.query_row("SELECT p.last_position FROM play_sessions p JOIN listen_session_sources src ON src.session_id=p.id WHERE p.zone_id=?1 AND src.playback_id=?2 AND p.finished_at IS NOT NULL",params![z.id,z.playback_id],|r|r.get::<_,f64>(0)).optional()?;
    if let Some(position)=closed {
        if z.position>position&&!correct_group_progress_tx(tx,z,seconds,now,runtime_id())?{anyhow::bail!("组队列最终帧超出计时范围");}
    }else{
        if active(tx,&z.id)?.is_none(){
            let mut baseline=z.clone();baseline.position-=seconds;baseline.status="playing".into();
            progress_tx(tx,&baseline,now-(seconds*1000.).round() as i64,runtime_id(),false,Some(0.))?;
            measured::issue_current(tx,owner,&baseline)?;
        }
        let session=active(tx,&z.id)?.ok_or_else(||anyhow::anyhow!("组队列听音会话缺失"))?;
        if now<session.last_report_at||z.position<session.last_position||z.position-session.last_position>(now-session.last_report_at) as f64/1000.+1.{anyhow::bail!("组队列实际帧超出计时范围");}
        progress_tx(tx,z,now,runtime_id(),false,Some(seconds))?;
        if let Some(reason)=reason{if let Some(session)=active(tx,&z.id)?{close(tx,&session,now,reason)?;}}
    }
    if reason==Some("ended"){if let Some(id)=&z.playback_id{measured::mark_ended(tx,id,Some(z.position),now)?;}}
    Ok(())
}
fn device_progress_at(c:&mut Connection,z:&Zone,frames:u64,rate:u32,now:i64,runtime:&str,finish:bool)->anyhow::Result<Option<bool>> {
    if !(8000..=rmusic_audio::MAX_CLOCK_RATE).contains(&rate) || !z.position.is_finite() || z.position<0. {return Ok(None);}
    let seconds=frames as f64/rate as f64;
    if seconds>86400. || seconds>z.position+0.001 {return Ok(None);}
    let tx=c.transaction()?;
    let Some(session)=active(&tx,&z.id)? else {return Ok(None)};
    if let Some(id)=z.playback_id.as_deref(){if tx.query_row("SELECT timing_version FROM listen_claims WHERE playback_id=?1",[id],|r|r.get::<_,u32>(0)).optional()?==Some(2){return Ok(None);}}
    if session.runtime!=runtime || z.track.as_ref().map(|t|t.id.as_str())!=Some(session.track_id.as_str())
        || now<session.last_report_at || z.position<session.last_position
        || (z.position-session.last_position).max(0.).min(seconds)>(now-session.last_report_at) as f64/1000.+1. {return Ok(None);}
    let changed=progress_tx(&tx,z,now,runtime,false,Some(seconds))?;
    if finish {if let Some(session)=active(&tx,&z.id)? {close(&tx,&session,now,"ended")?;}
        if let Some(playback)=z.playback_id.as_deref(){measured::mark_ended(&tx,playback,Some(z.position),now)?;}}
    tx.commit()?;Ok(Some(changed||finish))
}
/// Rebind a recovered cursor without crediting device frames or elapsed downtime.
pub(crate) fn restore_sequence_tx(tx:&Transaction<'_>,owner:&str,previous:&str,old:Option<&str>,z:&Zone)->anyhow::Result<()> {
    let now=z.updated_at;
    let ended=tx.query_row("SELECT EXISTS(SELECT 1 FROM listen_source_ends WHERE playback_id=?1)",[previous],|r|r.get::<_,bool>(0))?;
    if let Some(session)=active(tx,&z.id)? {close(tx,&session,now,if ended{"ended"}else{"disconnect"})?;}
    if z.playback_id.is_none(){return Ok(());}
    if let Some(old)=old {measured::restore(tx,owner,old,z,now)?;}
    else {
        let mut baseline=z.clone();baseline.status="playing".into();
        progress_tx(tx,&baseline,now,runtime_id(),true,Some(0.))?;
        progress_tx(tx,z,now,runtime_id(),true,Some(0.))?;
        measured::issue_current(tx,owner,z)?;
    }
    Ok(())
}
/// Retain counted status and accumulated seconds across restart, but discard timing baselines.
pub async fn recover(state: &AppState) -> anyhow::Result<()> {
    state.db.run(|c| { c.execute("UPDATE play_sessions SET runtime_id='',last_status='stopped' WHERE finished_at IS NULL",[])?; Ok(()) }).await
}

fn progress(
    c: &mut Connection,
    z: &Zone,
    now: i64,
    runtime: &str,
    baseline: bool,
) -> anyhow::Result<bool> {
    let tx = c.transaction()?;
    let counted = progress_tx(&tx, z, now, runtime, baseline, None)?;
    tx.commit()?;
    Ok(counted)
}
fn progress_tx(
    tx: &Transaction<'_>,
    z: &Zone,
    now: i64,
    runtime: &str,
    baseline: bool,
    measured: Option<f64>,
) -> anyhow::Result<bool> {
    let Some(track) = &z.track else {
        return Ok(false);
    };
    if !z.position.is_finite()
        || z.position < 0.
        || !matches!(z.status.as_str(), "playing" | "paused" | "stopped")
    {
        return Ok(false);
    }
    let position = if measured.is_none() && track.duration.is_finite() && track.duration > 0. {
        z.position.min(track.duration)
    } else {
        z.position
    };
    let mut session = active(tx, &z.id)?;
    if session.as_ref().is_some_and(|s| s.track_id != track.id) {
        close(tx, session.as_ref().unwrap(), now, "replaced")?;
        session = None;
    }
    let Some(s) = session else {
        if z.status == "playing" {
            remember_track(tx,track)?;
            if let Some(user)=tx.query_row("SELECT user_id FROM external_resource_owners WHERE kind='zone' AND id=?1",[&z.id],|r|r.get::<_,Option<String>>(0)).optional()?.flatten(){crate::accounts::listening::remember(tx,&user,track)?;}
            let duration = if track.duration.is_finite() && track.duration > 0. {
                track.duration
            } else {
                0.
            };
            let issued=measured::for_playback(tx,z.playback_id.as_deref())?;
            let (id,seconds,counted)=issued.unwrap_or_else(||(uuid::Uuid::new_v4().to_string(),0.,false));
            tx.execute("INSERT INTO play_sessions(id,zone_id,track_id,title,artist,cover_url,duration,started_at,last_report_at,last_position,last_status,runtime_id,play_seconds,counted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8,?9,?10,?11,?12,?13) ON CONFLICT(id) DO UPDATE SET zone_id=excluded.zone_id,last_report_at=excluded.last_report_at,last_position=excluded.last_position,last_status=excluded.last_status,runtime_id=excluded.runtime_id,finished_at=NULL,finish_reason=NULL,play_seconds=excluded.play_seconds,counted=excluded.counted",params![id,z.id,track.id,track.title,track.artist,track.cover_url,duration,now,position,z.status,runtime,seconds,counted])?;
            crate::accounts::listening::own_zone(tx,&id,&z.id)?;
            measured::bind_source(tx,&id,z.playback_id.as_deref())?;
        }
        return Ok(false);
    };
    // Server timestamps bound the position delta. Long gaps, backwards moves, and large jumps
    // establish a fresh baseline. Explicit seek resets even when its jump is below this bound.
    if now < s.last_report_at {
        return Ok(false);
    }
    measured::bind_source(tx,&s.id,z.playback_id.as_deref())?;
    let elapsed = (now - s.last_report_at) as f64 / 1000.;
    let delta = position - s.last_position;
    let seconds = if !baseline
        && s.runtime == runtime
        && (s.last_status == "playing" || (measured.is_some() && s.last_status == "paused"))
        && (elapsed > 0. || (measured.is_some() && elapsed == 0.))
        && (elapsed <= 30. || measured.is_some())
        && delta > 0.
        && delta <= elapsed + 1.
    {
        delta.min(measured.unwrap_or(delta))
    } else {
        0.
    };
    let mut counted = !s.counted && s.seconds + seconds >= threshold(s.duration);
    let mut credited=seconds;
    if let Some(playback)=z.playback_id.as_deref().filter(|_|seconds>0.) {
        if measured::for_playback(tx,Some(playback))?.is_some() {
            credited=0.;let mut from=position-seconds;let end_day=day(now);let start_ms=now-(seconds*1000.).round() as i64;
            let mut parts=Vec::new();
            if day(start_ms)!=end_day {
                let boundary=chrono::DateTime::from_timestamp_millis(now).unwrap_or_default().date_naive().and_hms_opt(0,0,0).unwrap().and_utc().timestamp_millis();
                let previous=(boundary-start_ms) as f64/1000.;parts.push((previous,boundary-1));parts.push((seconds-previous,now));
            }else{parts.push((seconds,now));}
            for (amount,at) in parts {if let Some((new,group_counted))=measured::credit(tx,playback,from,from+amount,at)? {credited+=new;counted=!s.counted&&group_counted;}from+=amount;}
        }else if seconds>0. {aggregate_interval(tx,&s,now,seconds,counted)?;}
    }else if seconds>0. {aggregate_interval(tx,&s,now,seconds,counted)?;}
    tx.execute("UPDATE play_sessions SET last_report_at=?2,last_position=?3,last_status=?4,runtime_id=?5,play_seconds=play_seconds+?6,counted=MAX(counted,?7) WHERE id=?1",params![s.id,now,position,z.status,runtime,credited,counted])?;
    Ok(counted)
}
fn aggregate_interval(tx:&Transaction<'_>,s:&Session,now:i64,seconds:f64,counted:bool)->anyhow::Result<()> {
    if seconds > 0. {
        crate::cache_policy::heard(tx, &s.track_id, s.duration, s.seconds, seconds)?;
        // Attribute a midnight-crossing interval to both UTC dates instead of the report date only.
        let end_day = day(now);
        let start_ms = now - (seconds * 1000.).round() as i64;
        if day(start_ms) != end_day {
            let end_date = chrono::DateTime::from_timestamp_millis(now)
                .unwrap_or_default()
                .date_naive();
            let boundary = end_date
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis();
            let previous = (boundary - start_ms) as f64 / 1000.;
            aggregate(tx, &s, boundary - 1, previous, 0, 0)?;
            aggregate(tx, &s, now, seconds - previous, i64::from(counted), 0)?;
        } else {
            aggregate(tx, &s, now, seconds, i64::from(counted), 0)?;
        }
    }
    Ok(())
}

/// Measured continuous-output boundaries, inside the same transaction as the new zone and receipt.
/// The first source may start at a seek offset. Later sources always start at zero. Device frames
/// cap every credit, and the server interval bounds the total unreported audio across the batch.
pub(crate) fn sequence_tx(
    tx: &Transaction<'_>,
    ended: &[(Zone, f64)],
    current: &Zone,
    current_seconds: f64,
    now: i64,
) -> anyhow::Result<bool> {
    let Some((first, first_seconds)) = ended.first() else { return Ok(false); };
    let Some(session) = active(tx, &first.id)? else { return Ok(false); };
    if session.runtime != runtime_id()
        || first.track.as_ref().map(|t| t.id.as_str()) != Some(session.track_id.as_str())
        || now < session.last_report_at
        || first.position + 0.001 < session.last_position
    {
        return Ok(false);
    }
    let tail = ended.iter().skip(1).map(|(_, seconds)| seconds).sum::<f64>() + current_seconds;
    let unreported = (first.position - session.last_position).max(0.).min(*first_seconds) + tail;
    if unreported > (now - session.last_report_at) as f64 / 1000. + 1. {
        return Ok(false);
    }
    let mut at = now - (tail * 1000.).round() as i64;
    progress_tx(tx, first, at.max(session.last_report_at), runtime_id(), false, Some(*first_seconds))?;
    if let Some(session) = active(tx, &first.id)? {
        close(tx, &session, at.max(session.last_report_at), "ended")?;
    }
    if let Some(playback)=first.playback_id.as_deref(){measured::mark_ended(tx,playback,Some(first.position),at)?;}
    for (zone, seconds) in ended.iter().skip(1) {
        let mut baseline = zone.clone();
        baseline.status = "playing".into();
        baseline.position = 0.;
        progress_tx(tx, &baseline, at, runtime_id(), true, Some(0.))?;
        at += (seconds * 1000.).round() as i64;
        progress_tx(tx, zone, at, runtime_id(), false, Some(*seconds))?;
        if let Some(session) = active(tx, &zone.id)? { close(tx, &session, at, "ended")?; }
        if let Some(playback)=zone.playback_id.as_deref(){measured::mark_ended(tx,playback,Some(zone.position),at)?;}
    }
    let mut baseline = current.clone();
    baseline.status = "playing".into();
    baseline.position = 0.;
    progress_tx(tx, &baseline, at, runtime_id(), true, Some(0.))?;
    progress_tx(tx, current, now.max(at), runtime_id(), false, Some(current_seconds))?;
    Ok(true)
}
fn finish_at(
    c: &mut Connection,
    z: &Zone,
    now: i64,
    runtime: &str,
    reason: &str,
) -> anyhow::Result<bool> {
    let Some(s) = active(c, &z.id)? else {
        return Ok(false);
    };
    if z.track.as_ref().is_some_and(|t| t.id != s.track_id) {
        return Ok(false);
    };
    progress(c, z, now, runtime, false)?;
    let tx = c.transaction()?;
    if let Some(s) = active(&tx, &z.id)? {
        close(&tx, &s, now, reason)?;
    }
    tx.commit()?;
    Ok(true)
}

pub(crate) const DEFAULT_TIMEZONE: &str = "Asia/Shanghai";

#[derive(Deserialize)]
pub struct StatsQuery {
    pub period_days: Option<u32>,
    pub timezone: Option<String>,
}
pub async fn overview(
    State(state): State<AppState>,
    auth: Auth,
    Query(q): Query<StatsQuery>,
) -> ApiResult<Json<Value>> {
    auth.require("read")?;
    let period = q.period_days.unwrap_or(30);
    if period>3650 {
        return Err(ApiError::bad("period_days 须为 0（历史所有）或 1 到 3650"));
    }
    let configured=state.setting("statistics_timezone",json!(DEFAULT_TIMEZONE)).await;
    let timezone=q.timezone.unwrap_or_else(||configured.as_str().unwrap_or(DEFAULT_TIMEZONE).into()).parse::<chrono_tz::Tz>().map_err(|_|ApiError::bad("timezone 须为有效 IANA 时区，例如 Asia/Shanghai"))?;
    Ok(Json(
        state
            .db
            .read(move |c| crate::accounts::listening::with_stats(c,if auth.role=="external"{Some(&auth.id)}else{None},||overview_in_timezone(c, period, Utc::now().timestamp_millis(),timezone)))
            .await?,
    ))
}
#[derive(Deserialize)]
pub struct HistoryQuery {limit:Option<usize>,cursor:Option<String>}
/// Bounded chronological history for details and future recommendation consumers.
/// Uses the existing session index; no event generation, background scan or AI upload.
pub async fn history(State(state):State<AppState>,auth:Auth,Query(q):Query<HistoryQuery>)->ApiResult<Json<Value>> {
    auth.require("read")?;
    let before=if let Some(cursor)=q.cursor {let Some((at,id))=cursor.split_once(':')else{return Err(ApiError::bad("历史游标无效"));};let at=at.parse::<i64>().map_err(|_|ApiError::bad("历史游标无效"))?;if at<0||id.is_empty()||id.len()>180{return Err(ApiError::bad("历史游标无效"));}Some((at,id.to_owned()))}else{None};
    let limit=q.limit.unwrap_or(50).clamp(1,100);Ok(Json(state.db.read(move|c|if auth.role=="external"{crate::accounts::listening::history(c,&auth.id,limit,before)}else{history_at(c,limit,before)}).await?))
}
fn history_at(c:&Connection,limit:usize,before:Option<(i64,String)>)->anyhow::Result<Value>{
    let (at,id)=before.unwrap_or((i64::MAX,"".into()));
    let mut stmt=c.prepare("SELECT p.id,p.track_id,p.title,p.artist,COALESCE(NULLIF(t.album,''),m.album,''),COALESCE(NULLIF(t.album_artist,''),m.album_artist,''),COALESCE(t.cover_url,p.cover_url),COALESCE(m.source,t.source,'unknown'),p.started_at,p.finished_at,p.finish_reason,p.play_seconds,p.counted,p.duration FROM play_sessions p LEFT JOIN tracks t ON t.id=p.track_id LEFT JOIN listening_metadata m ON m.track_id=p.track_id WHERE p.play_seconds>0 AND NOT EXISTS(SELECT 1 FROM account_session_owners o WHERE o.event_id=p.id AND o.external=1) AND (p.started_at,p.id)<(?1,?2) ORDER BY p.started_at DESC,p.id DESC LIMIT ?3")?;
    let mut rows=stmt.query_map(params![at,id,(limit+1)as i64],|r|Ok(json!({"session_id":r.get::<_,String>(0)?,"track_id":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"artist":r.get::<_,String>(3)?,"album":r.get::<_,String>(4)?,"album_artist":r.get::<_,String>(5)?,"cover_url":r.get::<_,Option<String>>(6)?,"source":r.get::<_,String>(7)?,"started_at_ms":r.get::<_,i64>(8)?,"finished_at_ms":r.get::<_,Option<i64>>(9)?,"finish_reason":r.get::<_,Option<String>>(10)?,"listened_seconds":r.get::<_,f64>(11)?,"counted":r.get::<_,bool>(12)?,"duration":r.get::<_,f64>(13)?})))?.collect::<Result<Vec<_>,_>>()?;
    let more=rows.len()>limit;rows.truncate(limit);let next=if more{rows.last().map(|r|format!("{}:{}",r["started_at_ms"].as_i64().unwrap(),r["session_id"].as_str().unwrap()))}else{None};
    Ok(json!({"version":1,"items":rows,"next_cursor":next,"limit":limit,"time_basis":"confirmed_playback","day_timezone":"UTC","scope":"core"}))
}
pub async fn track(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    auth.require("read")?;
    let result = state.db.read(move |c| crate::accounts::listening::with_stats(c,if auth.role=="external"{Some(&auth.id)}else{None},||track_at(c, &id))).await?;
    Ok(Json(result.ok_or_else(ApiError::missing)?))
}
fn track_at(c: &Connection, id: &str) -> anyhow::Result<Option<Value>> {
    let result=c.query_row("SELECT play_count,skip_count,play_seconds,last_played_at FROM track_stats WHERE track_id=?1",[id],|r|Ok(json!({"track_id":id,"play_count":r.get::<_,i64>(0)?,"skip_count":r.get::<_,i64>(1)?,"listened_seconds":r.get::<_,f64>(2)?,"last_played_at":r.get::<_,Option<i64>>(3)?}))).optional()?;
    if result.is_some() {
        return Ok(result);
    }
    let exists = c
        .query_row("SELECT 1 FROM tracks WHERE id=?1", [id], |_| Ok(()))
        .optional()?
        .is_some();
    Ok(exists.then(||json!({"track_id":id,"play_count":0,"skip_count":0,"listened_seconds":0.,"last_played_at":null})))
}
/// Old daily-only rows cannot be repartitioned truthfully. Keep those on their UTC date
/// and report the legacy duration; every newly confirmed delta uses timestamped minute buckets.
fn overview_in_timezone(c:&Connection,period:u32,now:i64,timezone:chrono_tz::Tz)->anyhow::Result<Value>{
    if period==0 {
        let end=chrono::DateTime::from_timestamp_millis(now).unwrap_or_default().with_timezone(&timezone).date_naive();
        let day:Option<String>=c.query_row("SELECT MIN(day) FROM daily_stats WHERE day<=?1",[end.to_string()],|r|r.get(0))?;
        let minute:Option<i64>=c.query_row("SELECT MIN(minute) FROM listening_buckets WHERE minute<=?1",[now.div_euclid(60000)],|r|r.get(0))?;
        let start=day.and_then(|d|chrono::NaiveDate::parse_from_str(&d,"%Y-%m-%d").ok()).into_iter().chain(minute.and_then(|v|chrono::DateTime::from_timestamp(v.saturating_mul(60),0)).map(|v|v.with_timezone(&timezone).date_naive())).min().unwrap_or(end).min(end);
        let period=(end-start).num_days().saturating_add(1).max(1) as u32;
        let mut value=overview_in_timezone(c,period,now,timezone)?;value["period_days"]=json!(0);value["all_time"]=json!(true);value["calendar_days"]=json!(period);return Ok(value);
    }
    if timezone==chrono_tz::UTC {let mut value=overview_daily(c,period,now,"daily_stats")?;value["timezone"]=json!("UTC");value["legacy_utc_seconds"]=json!(0);return Ok(value);}
    let local=chrono::DateTime::from_timestamp_millis(now).unwrap_or_default().with_timezone(&timezone);
    let end=local.date_naive();let start=end.checked_sub_days(Days::new(u64::from(period-1))).unwrap();
    // +/- 2 UTC days covers even skipped civil days and UTC+14. Source scans stay index-bounded.
    let from=start.checked_sub_days(Days::new(2)).unwrap().and_hms_opt(0,0,0).unwrap().and_utc().timestamp()/60;
    let to=end.checked_add_days(Days::new(2)).unwrap().and_hms_opt(0,0,0).unwrap().and_utc().timestamp()/60;
    c.execute_batch("DROP TABLE IF EXISTS temp.zoned_daily; CREATE TEMP TABLE zoned_daily(day TEXT,track_id TEXT,title TEXT,artist TEXT,cover_url TEXT,play_count INTEGER,skip_count INTEGER,play_seconds REAL,last_played_at INTEGER,PRIMARY KEY(day,track_id));")?;
    let mut insert=c.prepare("INSERT INTO temp.zoned_daily VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(day,track_id) DO UPDATE SET play_count=play_count+excluded.play_count,skip_count=skip_count+excluded.skip_count,play_seconds=play_seconds+excluded.play_seconds,last_played_at=MAX(last_played_at,excluded.last_played_at)")?;
    let mut query=c.prepare("SELECT minute,track_id,title,artist,cover_url,play_count,skip_count,play_seconds,last_played_at FROM listening_buckets WHERE minute>=?1 AND minute<?2")?;let mut rows=query.query(params![from,to])?;
    while let Some(row)=rows.next()? {let minute:i64=row.get(0)?;let date=chrono::DateTime::from_timestamp(minute*60,0).unwrap_or_default().with_timezone(&timezone).date_naive();if date<start||date>end{continue;}insert.execute(params![date.to_string(),row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?,row.get::<_,f64>(7)?,row.get::<_,Option<i64>>(8)?])?;}
    drop(rows);drop(query);
    let mut legacy=0.;
    let mut query=c.prepare("SELECT d.day,d.track_id,d.title,d.artist,d.cover_url,d.play_count-COALESCE(b.plays,0),d.skip_count-COALESCE(b.skips,0),MAX(0,d.play_seconds-COALESCE(b.seconds,0)),d.last_played_at FROM daily_stats d LEFT JOIN (SELECT date(minute*60,'unixepoch') day,track_id,SUM(play_count) plays,SUM(skip_count) skips,SUM(play_seconds) seconds FROM listening_buckets WHERE minute>=?3 AND minute<?4 GROUP BY day,track_id) b ON b.day=d.day AND b.track_id=d.track_id WHERE d.day>=?1 AND d.day<=?2")?;
    let mut rows=query.query(params![start.to_string(),end.to_string(),from,to])?;
    while let Some(row)=rows.next()? {let seconds:f64=row.get(7)?;let plays:i64=row.get(5)?;let skips:i64=row.get(6)?;if seconds<=0.&&plays==0&&skips==0{continue;}legacy+=seconds;insert.execute(params![row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<String>>(4)?,plays,skips,seconds,row.get::<_,Option<i64>>(8)?])?;}
    drop(rows);drop(query);drop(insert);
    // Reuse all ranking projections against the local-day snapshot.
    let synthetic=end.and_hms_opt(12,0,0).unwrap().and_utc().timestamp_millis();
    let mut value=overview_daily(c,period,synthetic,"temp.zoned_daily")?;value["timezone"]=json!(timezone.name());value["legacy_utc_seconds"]=json!(legacy);c.execute_batch("DROP TABLE temp.zoned_daily")?;Ok(value)
}
#[cfg(test)]
fn overview_at(c: &Connection, period: u32, now: i64) -> anyhow::Result<Value> {overview_daily(c,period,now,"daily_stats")}
fn overview_daily(c: &Connection, period: u32, now: i64, table:&str) -> anyhow::Result<Value> {
    let end = chrono::DateTime::from_timestamp_millis(now)
        .unwrap_or_default()
        .date_naive();
    let start = end
        .checked_sub_days(Days::new(u64::from(period - 1)))
        .unwrap();
    let start_day = start.to_string();
    let end_day = end.to_string();
    let mut summary=c.query_row(&"SELECT COALESCE(SUM(d.play_count),0),COALESCE(SUM(d.skip_count),0),COALESCE(SUM(d.play_seconds),0),COUNT(DISTINCT CASE WHEN d.play_seconds>0 THEN COALESCE({identity},d.track_id) END),MAX(d.last_played_at) FROM daily_stats d LEFT JOIN tracks t ON t.id=d.track_id WHERE day>=?1 AND day<=?2".replace("{identity}",crate::cache_identity::ORIGINAL).replace("daily_stats",table),params![start_day,end_day],|r|Ok(json!({"play_count":r.get::<_,i64>(0)?,"skip_count":r.get::<_,i64>(1)?,"listened_seconds":r.get::<_,f64>(2)?,"unique_tracks":r.get::<_,i64>(3)?,"last_played_at":r.get::<_,Option<i64>>(4)?})))?;
    let mut stmt=c.prepare(&"SELECT day,SUM(play_count),SUM(skip_count),SUM(play_seconds) FROM daily_stats WHERE day>=?1 AND day<=?2 GROUP BY day".replace("{identity}",crate::cache_identity::ORIGINAL).replace("daily_stats",table))?;
    let totals=stmt.query_map(params![start_day,end_day],|r|Ok((r.get::<_,String>(0)?,json!({"day":r.get::<_,String>(0)?,"play_count":r.get::<_,i64>(1)?,"skip_count":r.get::<_,i64>(2)?,"listened_seconds":r.get::<_,f64>(3)?}))))?.collect::<Result<std::collections::HashMap<_,_>,_>>()?;
    // A lifetime view keeps its full totals while chart output stays bounded.
    let bucket_days=period.div_ceil(366).max(1);
    let mut days=(0..period.div_ceil(bucket_days)).map(|n|json!({"day":start.checked_add_days(Days::new(u64::from(n*bucket_days))).unwrap().to_string(),"play_count":0,"skip_count":0,"listened_seconds":0.})).collect::<Vec<_>>();
    for (day,total) in &totals {
        if let Ok(day)=chrono::NaiveDate::parse_from_str(day,"%Y-%m-%d") {
            let index=(day-start).num_days().max(0) as usize/bucket_days as usize;
            if let Some(bin)=days.get_mut(index){for key in ["play_count","skip_count"]{bin[key]=json!(bin[key].as_i64().unwrap_or(0)+total[key].as_i64().unwrap_or(0));}bin["listened_seconds"]=json!(bin["listened_seconds"].as_f64().unwrap_or(0.)+total["listened_seconds"].as_f64().unwrap_or(0.));}
        }
    }
    let mut stmt=c.prepare(&"SELECT COALESCE({identity},d.track_id),COALESCE(t.title,d.title),COALESCE(t.artist,d.artist),COALESCE(t.cover_url,d.cover_url),SUM(d.play_count),SUM(d.skip_count),SUM(d.play_seconds),MAX(d.last_played_at) FROM daily_stats d LEFT JOIN tracks t ON t.id=d.track_id WHERE day>=?1 AND day<=?2 GROUP BY COALESCE({identity},d.track_id) ORDER BY SUM(d.play_seconds) DESC,COALESCE({identity},d.track_id) LIMIT 100".replace("{identity}",crate::cache_identity::ORIGINAL).replace("daily_stats",table))?;
    let tracks=stmt.query_map(params![start_day,end_day],|r|Ok(json!({"track_id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"artist":r.get::<_,String>(2)?,"cover_url":r.get::<_,Option<String>>(3)?,"play_count":r.get::<_,i64>(4)?,"skip_count":r.get::<_,i64>(5)?,"listened_seconds":r.get::<_,f64>(6)?,"last_played_at":r.get::<_,Option<i64>>(7)?})))?.collect::<Result<Vec<_>,_>>()?;
    let mut stmt=c.prepare(&"SELECT COALESCE(t.artist,d.artist) AS performer,SUM(d.play_count),SUM(d.skip_count),SUM(d.play_seconds),COUNT(DISTINCT CASE WHEN d.play_seconds>0 THEN COALESCE({identity},d.track_id) END),COUNT(*) OVER() FROM daily_stats d LEFT JOIN tracks t ON t.id=d.track_id WHERE day>=?1 AND day<=?2 GROUP BY performer HAVING SUM(d.play_seconds)>0 ORDER BY SUM(d.play_seconds) DESC,performer LIMIT 100".replace("{identity}",crate::cache_identity::ORIGINAL).replace("daily_stats",table))?;
    let artists=stmt.query_map(params![start_day,end_day],|r|Ok(json!({"artist":r.get::<_,String>(0)?,"play_count":r.get::<_,i64>(1)?,"skip_count":r.get::<_,i64>(2)?,"listened_seconds":r.get::<_,f64>(3)?,"unique_tracks":r.get::<_,i64>(4)?,"total_artists":r.get::<_,i64>(5)?})))?.collect::<Result<Vec<_>,_>>()?;
    // The primary (day,track_id) index bounds all reads to the requested period.
    // Group album + album artist together, so unrelated releases with the same name stay separate.
    let mut stmt=c.prepare(&"WITH history AS (SELECT d.*,COALESCE({identity},d.track_id) logical_id,trim(COALESCE(NULLIF(t.album,''),m.album,'')) album_name,trim(COALESCE(NULLIF(t.album_artist,''),NULLIF(m.album_artist,''),t.artist,d.artist)) performer,COALESCE(t.cover_url,d.cover_url) art FROM daily_stats d LEFT JOIN tracks t ON t.id=d.track_id LEFT JOIN listening_metadata m ON m.track_id=d.track_id WHERE d.day>=?1 AND d.day<=?2  ) SELECT album_name,performer,MAX(art),SUM(play_count),SUM(skip_count),SUM(play_seconds),COUNT(DISTINCT CASE WHEN play_seconds>0 THEN logical_id END),MAX(last_played_at),COUNT(*) OVER() FROM history WHERE album_name<>'' GROUP BY album_name COLLATE NOCASE,performer COLLATE NOCASE HAVING SUM(play_seconds)>0 ORDER BY SUM(play_seconds) DESC,album_name,performer LIMIT 100".replace("{identity}",crate::cache_identity::ORIGINAL).replace("daily_stats",table))?;
    let albums=stmt.query_map(params![start_day,end_day],|r|Ok(json!({"album":r.get::<_,String>(0)?,"artist":r.get::<_,String>(1)?,"cover_url":r.get::<_,Option<String>>(2)?,"play_count":r.get::<_,i64>(3)?,"skip_count":r.get::<_,i64>(4)?,"listened_seconds":r.get::<_,f64>(5)?,"unique_tracks":r.get::<_,i64>(6)?,"last_played_at":r.get::<_,Option<i64>>(7)?,"total_albums":r.get::<_,i64>(8)?})))?.collect::<Result<Vec<_>,_>>()?;
    let mut stmt=c.prepare(&"SELECT COALESCE(NULLIF(m.source,''),NULLIF(t.source,''),'unknown') provider,SUM(d.play_seconds),SUM(d.play_count) FROM daily_stats d LEFT JOIN tracks t ON t.id=d.track_id LEFT JOIN listening_metadata m ON m.track_id=d.track_id WHERE d.day>=?1 AND d.day<=?2 GROUP BY provider HAVING SUM(d.play_seconds)>0 ORDER BY SUM(d.play_seconds) DESC,provider LIMIT 32".replace("{identity}",crate::cache_identity::ORIGINAL).replace("daily_stats",table))?;
    let sources=stmt.query_map(params![start_day,end_day],|r|Ok(json!({"source":r.get::<_,String>(0)?,"listened_seconds":r.get::<_,f64>(1)?,"play_count":r.get::<_,i64>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
    let mut streak=0;let mut longest=0;let mut active=0;let mut previous=None;
    let mut active_dates=totals.iter().filter(|(_,v)|v["listened_seconds"].as_f64().unwrap_or(0.)>0.).filter_map(|(day,_)|chrono::NaiveDate::parse_from_str(day,"%Y-%m-%d").ok()).collect::<Vec<_>>();active_dates.sort();
    for day in active_dates {active+=1;streak=if previous.and_then(|v:chrono::NaiveDate|v.succ_opt())==Some(day){streak+1}else{1};longest=longest.max(streak);previous=Some(day);}
    summary["unique_albums"]=albums.first().map(|v|v["total_albums"].clone()).unwrap_or(json!(0));
    summary["unique_artists"]=artists.first().map(|v|v["total_artists"].clone()).unwrap_or(json!(0));
    summary["active_days"]=json!(active);summary["longest_streak"]=json!(longest);
    Ok(json!({"chart_bucket_days":bucket_days,"period_days":period,"start_day":start_day,"end_day":end_day,"summary":summary,"days":days,"tracks":tracks,"artists":artists,"albums":albums,"sources":sources,"ranking_limit":100}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Track;
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(include_str!("schema.sql")).unwrap();
        c
    }
    fn zone(id: &str, duration: f64) -> Zone {
        let mut z = Zone::new(id.into(), id.into(), "test".into());
        z.status = "playing".into();
        z.track=Some(serde_json::from_value::<Track>(json!({"id":"track","title":"真实曲目","artist":"艺术家","album":"专辑","album_artist":"艺术家","duration":duration,"track_number":1,"disc_number":1,"source":"local","format":"flac","sample_rate":48000,"bit_depth":24,"channels":2,"bitrate":0,"cover_url":null,"recording_id":"recording","available":true,"created_at":0})).unwrap());
        z
    }
    fn report(c: &mut Connection, z: &mut Zone, time: i64, position: f64) -> bool {
        z.position = position;
        progress(c, z, time, "process", false).unwrap()
    }
    #[test]
    fn actual_progress_counts_once_and_duplicate_reports_do_not_inflate() {
        let mut c = db();
        let mut z = zone("a", 120.);
        assert!(!report(&mut c, &mut z, 1000, 0.));
        assert!(!report(&mut c, &mut z, 16000, 15.));
        assert!(report(&mut c, &mut z, 31000, 30.));
        assert!(!report(&mut c, &mut z, 31000, 30.));
        assert!(!report(&mut c, &mut z, 32000, 30.));
        report(&mut c, &mut z, 42000, 40.);
        let s = track_at(&c, "track").unwrap().unwrap();
        assert_eq!(s["play_count"], 1);
        assert_eq!(s["listened_seconds"], 40.);
    }
    #[test]
    fn seeks_backwards_jumps_and_long_gaps_establish_only_a_baseline() {
        let mut c = db();
        let mut z = zone("a", 300.);
        report(&mut c, &mut z, 1000, 0.);
        report(&mut c, &mut z, 11000, 10.);
        report(&mut c, &mut z, 12000, 100.);
        report(&mut c, &mut z, 13000, 20.);
        z.position = 20.5;
        progress(&mut c, &z, 13500, "process", true).unwrap();
        report(&mut c, &mut z, 14500, 21.5);
        report(&mut c, &mut z, 114500, 121.5);
        report(&mut c, &mut z, 115500, 122.5);
        assert_eq!(
            track_at(&c, "track").unwrap().unwrap()["listened_seconds"],
            12.
        );
        z.position = f64::NAN;
        progress(&mut c, &z, 116500, "process", false).unwrap();
        assert_eq!(active(&c, "a").unwrap().unwrap().last_position, 122.5);
    }
    #[test]
    fn pause_resume_does_not_count_paused_time_or_stalled_reports() {
        let mut c = db();
        let mut z = zone("a", 120.);
        report(&mut c, &mut z, 1000, 0.);
        z.status = "paused".into();
        report(&mut c, &mut z, 11000, 10.);
        report(&mut c, &mut z, 21000, 10.);
        z.status = "playing".into();
        report(&mut c, &mut z, 31000, 10.);
        report(&mut c, &mut z, 41000, 20.);
        report(&mut c, &mut z, 51000, 20.);
        assert_eq!(
            track_at(&c, "track").unwrap().unwrap()["listened_seconds"],
            20.
        );
    }
    #[test]
    fn restart_preserves_session_count_but_excludes_downtime_and_stale_reports() {
        let mut c = db();
        let mut z = zone("a", 120.);
        report(&mut c, &mut z, 1000, 0.);
        report(&mut c, &mut z, 31000, 30.);
        let id = active(&c, "a").unwrap().unwrap().id;
        z.position = 90.;
        progress(&mut c, &z, 40000, "restarted", false).unwrap();
        z.position = 100.;
        progress(&mut c, &z, 50000, "restarted", false).unwrap();
        z.position = 101.;
        progress(&mut c, &z, 49000, "restarted", false).unwrap();
        assert_eq!(active(&c, "a").unwrap().unwrap().id, id);
        let s = track_at(&c, "track").unwrap().unwrap();
        assert_eq!(s["play_count"], 1);
        assert_eq!(s["listened_seconds"], 40.);
    }
    #[test]
    fn finish_skip_is_idempotent_and_ended_disconnect_are_not_skips() {
        let mut c = db();
        let mut z = zone("a", 120.);
        report(&mut c, &mut z, 1000, 0.);
        report(&mut c, &mut z, 11000, 10.);
        assert!(finish_at(&mut c, &z, 11000, "process", "next").unwrap());
        assert!(!finish_at(&mut c, &z, 11000, "process", "next").unwrap());
        report(&mut c, &mut z, 12000, 0.);
        report(&mut c, &mut z, 22000, 10.);
        finish_at(&mut c, &z, 22000, "process", "disconnect").unwrap();
        report(&mut c, &mut z, 23000, 110.);
        report(&mut c, &mut z, 33000, 120.);
        finish_at(&mut c, &z, 33000, "process", "ended").unwrap();
        report(&mut c, &mut z, 34000, 0.);
        report(&mut c, &mut z, 44000, 10.);
        finish_at(&mut c, &z, 44000, "process", "stop").unwrap();
        let s = track_at(&c, "track").unwrap().unwrap();
        assert_eq!(s["skip_count"], 1);
        assert_eq!(s["listened_seconds"], 40.);
        assert!(!early_skip("next",90.,90.,120.));
        assert!(!early_skip("next",5.,100.,120.));
    }
    #[test]
    fn independent_zones_and_short_song_thresholds() {
        let mut c = db();
        let mut a = zone("a", 6.);
        let mut b = zone("b", 10.);
        report(&mut c, &mut a, 1000, 0.);
        report(&mut c, &mut b, 1000, 0.);
        assert!(!report(&mut c, &mut a, 4000, 3.));
        assert!(report(&mut c, &mut a, 6000, 5.));
        assert!(report(&mut c, &mut b, 6000, 5.));
        let s = track_at(&c, "track").unwrap().unwrap();
        assert_eq!(s["play_count"], 2);
        assert_eq!(s["listened_seconds"], 10.);
        assert_eq!(threshold(0.), 30.);
    }
    #[test]
    fn daily_split_period_aggregates_and_deleted_track_metadata_remain_available() {
        let mut c = db();
        let mut z = zone("a", 120.);
        let midnight = chrono::DateTime::parse_from_rfc3339("2026-10-13T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        report(&mut c, &mut z, midnight - 10000, 0.);
        report(&mut c, &mut z, midnight + 10000, 20.);
        let today = overview_at(&c, 1, midnight + 10000).unwrap();
        assert_eq!(today["summary"]["listened_seconds"], 10.);
        assert_eq!(today["days"].as_array().unwrap().len(), 1);
        assert_eq!(today["tracks"][0]["title"], "真实曲目");
        let both = overview_at(&c, 3, midnight + 10000).unwrap();
        assert_eq!(both["summary"]["listened_seconds"], 20.);
        assert_eq!(both["days"][0]["listened_seconds"], 0.);
        assert_eq!(both["artists"][0]["unique_tracks"], 1);
        assert_eq!(both["summary"]["unique_tracks"], 1);
        assert!(track_at(&c, "missing").unwrap().is_none());
    }
    #[test]
    fn track_replacement_finishes_old_session_without_mixing_progress() {
        let mut c = db();
        let mut z = zone("a", 120.);
        report(&mut c, &mut z, 1000, 0.);
        report(&mut c, &mut z, 11000, 10.);
        let old = z.clone();
        z.track.as_mut().unwrap().id = "other".into();
        report(&mut c, &mut z, 12000, 0.);
        report(&mut c, &mut z, 22000, 10.);
        assert!(!finish_at(&mut c, &old, 22000, "process", "stop").unwrap());
        assert_eq!(track_at(&c, "track").unwrap().unwrap()["skip_count"], 1);
        assert_eq!(
            track_at(&c, "other").unwrap().unwrap()["listened_seconds"],
            10.
        );
    }
    #[test]
    fn persisted_checkpoint_reopens_without_recounting_or_losing_listened_time() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("stats.sqlite");
        let mut c = Connection::open(&path).unwrap();
        c.execute_batch(include_str!("schema.sql")).unwrap();
        let mut z = zone("a", 120.);
        report(&mut c, &mut z, 1000, 0.);
        report(&mut c, &mut z, 31000, 30.);
        drop(c);
        let mut c = Connection::open(&path).unwrap();
        z.position = 40.;
        progress(&mut c, &z, 131000, "new-process", false).unwrap();
        z.position = 50.;
        progress(&mut c, &z, 141000, "new-process", false).unwrap();
        let s = track_at(&c, "track").unwrap().unwrap();
        assert_eq!(s["play_count"], 1);
        assert_eq!(s["listened_seconds"], 40.);
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM play_sessions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn measured_same_millisecond_reports_do_not_clamp_or_recredit_submillisecond_positions() {
        let mut c=db();let mut z=zone("a",0.002);
        progress(&mut c,&z,1000,"process",false).unwrap();z.position=113./48000.;
        assert!(device_progress_at(&mut c,&z,113,48000,1001,"process",false).unwrap().is_some());
        let original=z.position;z.position-=0.0001;
        assert!(device_progress_at(&mut c,&z,108,48000,1001,"process",false).unwrap().is_none());
        z.position=original;
        assert!(finish_device_at(&mut c,&z,113,48000,1001,"process").unwrap());
        assert_eq!(track_at(&c,"track").unwrap().unwrap()["listened_seconds"],113./48000.);
        let mut z=zone("b",0.007);
        progress(&mut c,&z,2000,"process",false).unwrap();z.position=0.005;
        device_progress_at(&mut c,&z,240,48000,2001,"process",false).unwrap();z.position=333./48000.;
        device_progress_at(&mut c,&z,333,48000,2001,"process",false).unwrap();
        assert!(finish_device_at(&mut c,&z,333,48000,2001,"process").unwrap());
        let total=track_at(&c,"track").unwrap().unwrap()["listened_seconds"].as_f64().unwrap();
        assert!((total-446./48000.).abs()<1e-9);
    }
    #[test]
    fn device_eof_uses_actual_frames_even_when_catalog_duration_is_rounded_down() {
        let mut c=db();let mut z=zone("a",0.1);
        progress(&mut c,&z,1000,"process",false).unwrap();
        z.position=0.125;
        assert!(finish_device_at(&mut c,&z,6000,48000,1200,"process").unwrap());
        assert_eq!(track_at(&c,"track").unwrap().unwrap()["listened_seconds"],0.125);
        assert!(!finish_device_at(&mut c,&z,6000,48000,1300,"process").unwrap());
        let mut z=zone("b",120.);z.position=50.;
        progress(&mut c,&z,2000,"process",true).unwrap();z.position=52.3;
        assert!(!finish_device_at(&mut c,&z,110400,48000,2100,"process").unwrap());
        assert!(finish_device_at(&mut c,&z,110400,48000,5000,"process").unwrap());
        let seconds=track_at(&c,"track").unwrap().unwrap()["listened_seconds"].as_f64().unwrap();
        assert!((seconds-2.425).abs()<0.000001);
    }
    #[test]
    fn continuous_frames_cap_seek_tail_and_allow_measured_next_progress_over_thirty_seconds() {
        let mut c = db();
        let mut old = zone("a", 120.);
        old.position = 95.;
        progress(&mut c, &old, 1000, runtime_id(), true).unwrap();
        old.position = 97.;
        progress(&mut c, &old, 3000, runtime_id(), false).unwrap();
        old.position = 100.;
        let mut next = old.clone();
        next.track.as_mut().unwrap().id = "next".into();
        next.position = 90.;
        let tx = c.transaction().unwrap();
        assert!(sequence_tx(&tx, &[(old,3.)], &next, 90., 96000).unwrap());
        tx.commit().unwrap();
        assert_eq!(track_at(&c,"track").unwrap().unwrap()["listened_seconds"],5.);
        assert_eq!(track_at(&c,"next").unwrap().unwrap()["listened_seconds"],90.);
        next.position = 96.;
        progress(&mut c,&next,102000,runtime_id(),false).unwrap();
        assert_eq!(track_at(&c,"next").unwrap().unwrap()["listened_seconds"],96.);
        assert_eq!(track_at(&c,"next").unwrap().unwrap()["play_count"],1);
    }
    #[test]
    fn continuous_short_tracks_split_midnight_without_counting_reserved_audio() {
        let mut c = db();
        let midnight = chrono::DateTime::parse_from_rfc3339("2026-10-13T00:00:00Z").unwrap().timestamp_millis();
        let mut first = zone("a",30.);
        progress(&mut c,&first,midnight-32000,runtime_id(),false).unwrap();
        first.position = 30.;
        let mut second = first.clone();
        second.track.as_mut().unwrap().id = "short".into();
        second.position = 2.;
        let mut current = first.clone();
        current.position = 0.125;
        let tx = c.transaction().unwrap();
        assert!(sequence_tx(&tx,&[(first,30.),(second,2.)],&current,0.125,midnight+125).unwrap());
        tx.commit().unwrap();
        let today = overview_at(&c,1,midnight+125).unwrap();
        assert_eq!(today["summary"]["listened_seconds"],0.125);
        let both = overview_at(&c,2,midnight+125).unwrap();
        assert_eq!(both["summary"]["listened_seconds"],32.125);
        assert_eq!(both["summary"]["play_count"],1);
        assert_eq!(both["summary"]["skip_count"],0);
    }
    #[test]
    fn album_stats_snapshot_groups_release_artist_and_keeps_unknowns_out() {
        let mut c=db();
        for (n,artist,album,album_artist) in [(0,"Singer A","Shared","Release A"),(1,"Singer B","Shared","Release A"),(2,"Singer A","Shared","Release B"),(3,"Singer C","","")] {
            let mut z=zone(&format!("room-{n}"),120.);let t=z.track.as_mut().unwrap();t.id=format!("song-{n}");t.artist=artist.into();t.album=album.into();t.album_artist=album_artist.into();
            if n==0 {t.provider_data=json!({"cached_from":{"source":"example"}});}
            report(&mut c,&mut z,1000,0.);report(&mut c,&mut z,31000,30.);
        }
        let v=overview_at(&c,1,40000).unwrap();assert_eq!(v["summary"]["unique_albums"],2);assert_eq!(v["summary"]["unique_artists"],3);assert_eq!(v["summary"]["active_days"],1);assert_eq!(v["summary"]["listened_seconds"],120.);
        assert_eq!(v["albums"][0]["artist"],"Release A");assert_eq!(v["albums"][0]["unique_tracks"],2);assert_eq!(v["albums"][0]["listened_seconds"],60.);assert_eq!(v["sources"].as_array().unwrap().len(),2);
        // No catalog rows are needed after the actual playback metadata has been snapshotted.
        assert_eq!(c.query_row("SELECT COUNT(*) FROM tracks",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        let plan:String=c.query_row("EXPLAIN QUERY PLAN SELECT SUM(play_seconds) FROM daily_stats WHERE day>=?1 AND day<=?2",params!["1970-01-01","1970-01-02"],|r|r.get(3)).unwrap();assert!(plan.contains("SEARCH"),"{plan}");
    }
    #[test]
    fn album_stats_migration_snapshots_existing_history_only_once() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("library.db");
        let c=Connection::open(&path).unwrap();c.execute_batch(include_str!("schema.sql")).unwrap();
        c.execute("INSERT INTO tracks(id,title,artist,album,album_artist,source,format,recording_id,created_at) VALUES('old','Song','Artist','Album','Release Artist','local','flac','r',0)",[]).unwrap();
        c.execute("INSERT INTO daily_stats(day,track_id,title,artist,play_seconds) VALUES('1970-01-01','old','Song','Artist',12)",[]).unwrap();drop(c);
        let db=crate::db::Db::open(&path).unwrap();let c=db.0.lock().unwrap();c.execute("DELETE FROM tracks",[]).unwrap();let v=overview_at(&c,1,20000).unwrap();assert_eq!(v["albums"][0]["album"],"Album");assert_eq!(v["albums"][0]["artist"],"Release Artist");assert_eq!(v["albums"][0]["listened_seconds"],12.);
    }

    #[test]
    fn album_stats_history_cursor_handles_equal_times_without_duplicates() {
        let mut c=db();for n in 0..3 {let mut z=zone(&format!("room{n}"),120.);z.track.as_mut().unwrap().id=format!("song{n}");report(&mut c,&mut z,1000,0.);report(&mut c,&mut z,16000,15.);}
        let first=history_at(&c,2,None).unwrap();assert_eq!(first["items"].as_array().unwrap().len(),2);let cursor=first["next_cursor"].as_str().unwrap().split_once(':').unwrap();let second=history_at(&c,2,Some((cursor.0.parse().unwrap(),cursor.1.into()))).unwrap();assert_eq!(second["items"].as_array().unwrap().len(),1);assert!(second["next_cursor"].is_null());assert_ne!(first["items"][1]["session_id"],second["items"][0]["session_id"]);assert_eq!(second["items"][0]["album"],"专辑");assert_eq!(second["items"][0]["listened_seconds"],15.);
    }

    #[test]
    fn album_stats_keeps_zero_second_skip_rows_on_the_following_day() {
        let mut c=db();let mut z=zone("room",120.);report(&mut c,&mut z,1000,0.);report(&mut c,&mut z,31000,30.);
        c.execute("INSERT INTO daily_stats(day,track_id,title,artist,skip_count,play_seconds) VALUES('1970-01-02','track','真实曲目','艺术家',1,0)",[]).unwrap();
        let v=overview_at(&c,2,86401000).unwrap();assert_eq!(v["albums"][0]["skip_count"],1);assert_eq!(v["albums"][0]["listened_seconds"],30.);assert_eq!(v["summary"]["active_days"],1);assert_eq!(v["summary"]["unique_artists"],1);
    }

}

#[cfg(test)] mod timezone_tests {
 use super::*;
 fn at(date:&str)->i64{chrono::DateTime::parse_from_rfc3339(date).unwrap().timestamp_millis()}
 #[tokio::test] async fn local_midnight_dst_and_legacy_are_reported_without_double_counting(){
  let dir=tempfile::tempdir().unwrap();let db=crate::db::Db::open(&dir.path().join("db")).unwrap();
  db.run(|c|{let tx=c.transaction()?;let s=Session{id:"s".into(),track_id:"t".into(),title:"Test".into(),artist:"Artist".into(),cover_url:None,duration:200.,last_report_at:0,last_position:0.,last_status:"playing".into(),runtime:"test".into(),seconds:0.,counted:false};
   for (stamp,seconds) in [("2026-10-21T18:29:00Z",10.),("2026-10-21T18:30:00Z",20.),("2026-10-22T01:00:00Z",30.)]{aggregate(&tx,&s,at(stamp),seconds,1,0)?;}
   tx.commit()?;Ok(())}).await.unwrap();
  let value=db.read(|c|overview_in_timezone(c,1,at("2026-10-22T12:00:00Z"),"Asia/Kolkata".parse().unwrap())).await.unwrap();assert_eq!(value["summary"]["listened_seconds"],50.);assert_eq!(value["summary"]["play_count"],2);assert_eq!(value["legacy_utc_seconds"],0.);assert_eq!(value["end_day"],"2026-10-22");
  let utc=db.read(|c|overview_in_timezone(c,2,at("2026-10-22T12:00:00Z"),chrono_tz::UTC)).await.unwrap();assert_eq!(utc["summary"]["listened_seconds"],60.);
  db.run(|c|{let tx=c.transaction()?;let s=Session{id:"dst".into(),track_id:"dst".into(),title:"DST".into(),artist:"A".into(),cover_url:None,duration:10.,last_report_at:0,last_position:0.,last_status:"playing".into(),runtime:"test".into(),seconds:0.,counted:false};for time in ["2026-03-08T04:59:00Z","2026-03-08T06:59:00Z","2026-03-08T07:01:00Z"]{aggregate(&tx,&s,at(time),5.,1,0)?;}tx.commit()?;Ok(())}).await.unwrap();
  let dst=db.read(|c|overview_in_timezone(c,1,at("2026-03-08T12:00:00Z"),"America/New_York".parse().unwrap())).await.unwrap();assert_eq!(dst["summary"]["listened_seconds"],10.);
  db.run(|c|{c.execute("INSERT INTO daily_stats(day,track_id,title,artist,play_seconds) VALUES('2026-10-22','old','Old','A',7)",[])?;Ok(())}).await.unwrap();
  let old=db.read(|c|overview_in_timezone(c,1,at("2026-10-22T12:00:00Z"),"Asia/Kolkata".parse().unwrap())).await.unwrap();assert_eq!(old["summary"]["listened_seconds"],57.);assert_eq!(old["legacy_utc_seconds"],7.);
 }
}
