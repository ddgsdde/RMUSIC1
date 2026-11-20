//! Cumulative client snapshots: one atomic delta per version, scoped to a core and user.
use super::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Day {
    pub seconds: f64,
    pub last_at: i64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub id: String,
    pub version: u64,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub duration: f64,
    pub started_at: i64,
    pub updated_at: i64,
    pub position: f64,
    pub days: BTreeMap<String, Day>,
    pub reason: String,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub measured:Option<super::measured::Measured>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub core_id: String,
    pub user_id: String,
    pub client_id: String,
    pub sessions: Vec<Snapshot>,
}
fn uuid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
}
pub(super) fn owner(batch: &Batch, core: &str, user: &str) -> ApiResult<()> {
    if batch.core_id != core || batch.user_id != user {
        return Err(ApiError::forbidden());
    }
    if !uuid(&batch.client_id) || batch.sessions.len() > 32 || batch.sessions.is_empty() {
        return Err(ApiError::bad("离线统计每批需要 1..32 条记录与有效设备 ID"));
    }
    Ok(())
}
pub async fn sync(
    State(state): State<AppState>,
    auth: Auth,
    Json(batch): Json<Batch>,
) -> ApiResult<Json<Value>> {
    auth.require("control")?;
    owner(&batch, &state.core_id, &auth.id)?;
    let result = state
        .db
        .run(move |c| import(c, &batch, Utc::now().timestamp_millis()))
        .await?;
    if result["changed"] == true {
        state.emit("stats", json!({"source":"offline","account_realm":crate::accounts::scope::realm(&auth)}));
    }
    Ok(Json(result))
}
pub(super) fn valid(s: &Snapshot, now: i64) -> bool {
    if !super::measured::valid(s)||!uuid(&s.id)
        || s.version == 0
        || s.version > i64::MAX as u64
        || s.track_id.is_empty()
        || s.track_id.len() > 512
        || s.title.len() > 2048
        || s.artist.len() > 2048
        || !s.duration.is_finite()
        || !(0. ..=604800.).contains(&s.duration)
        || !s.position.is_finite()
        || s.position < 0.
        || (s.duration > 0. && s.position > s.duration + 2.)
        || s.started_at < 946684800000
        || s.updated_at < s.started_at
        || s.updated_at > now + 300000
        || s.days.len() > 64
        || !matches!(
            s.reason.as_str(),
            "active" | "ended" | "stop" | "replaced" | "replay" | "interrupted"
        )
    {
        return false;
    }
    let mut total = 0.;
    for (date, d) in &s.days {
        let Ok(date_value) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            return false;
        };
        if date_value.to_string() != *date
            || day(d.last_at) != *date
            || !d.seconds.is_finite()
            || d.seconds <= 0.
            || d.last_at < s.started_at
            || d.last_at > s.updated_at
        {
            return false;
        }
        let midnight = date_value
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let available = (d.last_at - midnight.max(s.started_at)) as f64 / 1000.;
        if d.seconds > available + 1. {
            return false;
        }
        total += d.seconds;
    }
    total <= (s.updated_at - s.started_at) as f64 / 1000. + 1.
}
fn successor(old: &Snapshot, s: &Snapshot) -> bool {
    old.track_id == s.track_id
        && old.title == s.title
        && old.artist == s.artist
        && old.duration == s.duration
        && old.started_at == s.started_at
        && s.updated_at >= old.updated_at
        && old.reason == "active"
        && match (&old.measured,&s.measured) {(None,None)=>true,(Some(old),Some(next))=>super::measured::successor(old,next),_=>false}
        && old.days.iter().all(|(day, prior)| {
            s.days
                .get(day)
                .is_some_and(|next| next.seconds >= prior.seconds && next.last_at >= prior.last_at)
        })
}
pub(super) fn import(c: &mut Connection, batch: &Batch, now: i64) -> anyhow::Result<Value> {
    let tx = c.transaction()?;
    let policy=tx.query_row("SELECT policy FROM external_accounts WHERE user_id=?1",[&batch.user_id],|r|r.get::<_,String>(0)).optional()?.map(|v|serde_json::from_str::<crate::accounts::Policy>(&v)).transpose()?;
    let permission=policy.as_ref().map(|p|p.sql("t")).unwrap_or_else(||"1".into());
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut changed = false;
    for s in &batch.sessions {
        if !valid(s, now) {
            rejected.push(json!({"id":s.id,"version":s.version,"code":"invalid_snapshot"}));
            continue;
        }
        let prior:Option<(String,bool,bool)> = tx.query_row("SELECT payload,counted,skipped FROM offline_listens WHERE user_id=?1 AND client_id=?2 AND session_id=?3",params![batch.user_id,batch.client_id,s.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (old, mut counted, mut skipped) = if let Some((payload, counted, skipped)) = prior {
            (
                Some(serde_json::from_str::<Snapshot>(&payload)?),
                counted,
                skipped,
            )
        } else {
            (None, false, false)
        };
        if let Some(old) = &old {
            if s.version < old.version || s == old {
                accepted.push(json!({"id":s.id,"version":s.version}));
                continue;
            }
            if s.version == old.version || !successor(old, s) {
                rejected.push(json!({"id":s.id,"version":s.version,"code":"conflicting_snapshot"}));
                continue;
            }
        }
        if s.measured.is_some() {
            let Some((counted,skipped))=super::measured::import(&tx,&batch.user_id,s)? else{rejected.push(json!({"id":s.id,"version":s.version,"code":"invalid_snapshot"}));continue;};
            tx.execute("INSERT INTO offline_listens(user_id,client_id,session_id,payload,counted,skipped) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,client_id,session_id) DO UPDATE SET payload=excluded.payload,counted=excluded.counted,skipped=excluded.skipped",params![batch.user_id,batch.client_id,s.id,serde_json::to_string(s)?,counted,skipped])?;
            accepted.push(json!({"id":s.id,"version":s.version}));changed=true;continue;
        }
        let metadata: Option<(String, String, Option<String>)> = tx
            .query_row(
                &format!("SELECT t.title,t.artist,t.cover_url FROM tracks t WHERE t.id=?1 AND {permission}"),
                [&s.track_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (title, artist, cover_url) =
            metadata.unwrap_or_else(|| (s.title.clone(), s.artist.clone(), None));
        let id = format!(
            "offline-{}",
            crate::auth::hash(&format!("{}\n{}\n{}", batch.user_id, batch.client_id, s.id))
        );
        crate::accounts::listening::own(&tx,&id,&batch.user_id)?;
        let total: f64 = s.days.values().map(|d| d.seconds).sum();
        let session = Session {
            id: id.clone(),
            track_id: s.track_id.clone(),
            title,
            artist,
            cover_url,
            duration: s.duration,
            last_report_at: s.updated_at,
            last_position: s.position,
            last_status: "stopped".into(),
            runtime: "offline".into(),
            seconds: total,
            counted,
        };
        let mut cumulative = 0.;
        for (day, d) in &s.days {
            let delta = d.seconds
                - old
                    .as_ref()
                    .and_then(|old| old.days.get(day))
                    .map_or(0., |d| d.seconds);
            cumulative += d.seconds;
            let plays = i64::from(!counted && cumulative >= threshold(s.duration));
            counted |= plays > 0;
            if delta > 0. || plays > 0 {
                aggregate_utc_only(&tx, &session, d.last_at, delta, plays, 0)?;
            }
        }
        if !skipped && super::early_skip(&s.reason, total, s.position, s.duration) {
            aggregate_utc_only(&tx, &session, s.updated_at, 0., 0, 1)?;
            skipped = true;
        }
        // Imported snapshots are history; they never own a live core zone.
        tx.execute("INSERT INTO play_sessions(id,zone_id,track_id,title,artist,cover_url,duration,started_at,last_report_at,last_position,last_status,runtime_id,play_seconds,counted,finished_at,finish_reason) VALUES(?1,'offline',?2,?3,?4,?5,?6,?7,?8,?9,'stopped','offline',?10,?11,?8,?12) ON CONFLICT(id) DO UPDATE SET last_report_at=excluded.last_report_at,last_position=excluded.last_position,play_seconds=excluded.play_seconds,counted=excluded.counted,finished_at=excluded.finished_at,finish_reason=excluded.finish_reason",params![id,s.track_id,session.title,session.artist,session.cover_url,s.duration,s.started_at,s.updated_at,s.position,total,counted,s.reason])?;
        tx.execute("INSERT INTO offline_listens(user_id,client_id,session_id,payload,counted,skipped) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,client_id,session_id) DO UPDATE SET payload=excluded.payload,counted=excluded.counted,skipped=excluded.skipped",params![batch.user_id,batch.client_id,s.id,serde_json::to_string(s)?,counted,skipped])?;
        accepted.push(json!({"id":s.id,"version":s.version}));
        changed = true;
    }
    tx.commit()?;
    Ok(json!({"accepted":accepted,"rejected":rejected,"changed":changed}))
}
