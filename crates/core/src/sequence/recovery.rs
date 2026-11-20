//! Durable queue authority. Transport loss preserves intent; explicit navigation revokes it.
use super::*;
use rusqlite::{Connection, Transaction};
#[path="suspend.rs"]
mod fresh;

#[derive(Clone,Debug,Deserialize,Serialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SystemSuspend {pub host_id:String,pub generation:u64}
impl SystemSuspend {
    pub(crate) fn validate(&self)->ApiResult<()> {if !uuid(&self.host_id)||self.generation==0{return Err(ApiError::bad("系统挂起标记无效"));}Ok(())}
    fn newer_than(&self,old:Option<&Self>)->bool {old.is_none_or(|old|old.host_id!=self.host_id||self.generation>old.generation)}
}

#[derive(Clone, Deserialize, Serialize)]
struct Lease {
    plan: Plan,
    anchor: String,
    desired_status: String,
    suspended: bool,
    #[serde(default)]
    device_waiting: bool,
    #[serde(default)]
    last_system_suspend:Option<SystemSuspend>,
}
fn load(c: &Connection, id: &str) -> anyhow::Result<Option<Lease>> {
    c.query_row(
        "SELECT data FROM sequence_recovery WHERE zone_id=?1",
        [id],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map(|data| Ok(serde_json::from_str(&data)?))
    .transpose()
}
fn store(tx: &Transaction<'_>, id: &str, lease: &Lease) -> anyhow::Result<()> {
    tx.execute("INSERT INTO sequence_recovery(zone_id,data) VALUES(?1,?2) ON CONFLICT(zone_id) DO UPDATE SET data=excluded.data",
        params![id, serde_json::to_string(lease)?])?;
    Ok(())
}
pub(super) fn current_slot(z: &Zone) -> Slot {
    Slot {
        playback_id: z.playback_id.clone().unwrap(),
        index: z.queue_index,
        track: z.track.clone().unwrap(),
        queue: None,
    }
}
pub(super) fn save_plan(tx: &Transaction<'_>, z: &Zone, plan: &Plan) -> anyhow::Result<()> {
    tx.execute(
        "UPDATE zones SET data=?2 WHERE id=?1",
        params![z.id, serde_json::to_string(z)?],
    )?;
    store(
        tx,
        &z.id,
        &Lease {
            plan: plan.clone(),
            anchor: signature(z),
            desired_status: z.status.clone(),
            suspended: false,
            device_waiting: false,
            last_system_suspend:load(tx,&z.id)?.and_then(|lease|lease.last_system_suspend),
        },
    )
}
pub(crate) fn advance_plan(tx: &Transaction<'_>, z: &Zone, plan: &Plan) -> anyhow::Result<()> {
    if let Some(mut lease) = load(tx, &z.id)?.filter(|l| l.plan.id == plan.id && !l.suspended) {
        lease.plan = plan.clone();
        lease.anchor = signature(z);
        store(tx, &z.id, &lease)?;
    }
    Ok(())
}
/// Run in the same transaction as every ordinary zone save, including after a Core restart.
pub(crate) fn reconcile(
    tx: &Transaction<'_>,
    z: &Zone,
    action: Option<&str>,
) -> anyhow::Result<bool> {
    if action==Some("play") {fresh::grant(tx,z)?;}
    let Some(mut lease) = load(tx, &z.id)? else {
        return Ok(false);
    };
    let navigation = matches!(
        action,
        Some("stop" | "seek" | "next" | "previous" | "ended" | "queue" | "recovery_unavailable")
    );
    let valid_status = matches!(z.status.as_str(), "playing" | "paused")
        || lease.suspended && z.status == "stopped";
    if navigation || lease.anchor != signature(z) || !valid_status {
        tx.execute("DELETE FROM sequence_recovery WHERE zone_id=?1", [&z.id])?;
        return Ok(true);
    }
    if matches!(action, Some("play" | "pause")) {
        lease.desired_status = z.status.clone();
        if action==Some("play") {lease.device_waiting=false;}
        store(tx, &z.id, &lease)?;
    }
    Ok(false)
}
fn suspend_record(tx: &Transaction<'_>, old: &Zone, z: &Zone, device_waiting:bool,stamp:Option<&SystemSuspend>, restarting:bool) -> anyhow::Result<()> {
    if let Some(mut lease) = load(tx, &z.id)? {
        if lease.anchor == signature(old) || restarting && lease.anchor == super::signature_with_loudness(old, true) {
            lease.anchor = signature(z);
            lease.suspended = true;
            if device_waiting {lease.device_waiting=true;lease.desired_status="paused".into();}
            if let Some(stamp)=stamp {lease.last_system_suspend=Some(stamp.clone());}
            store(tx, &z.id, &lease)?;
        } else {
            tx.execute("DELETE FROM sequence_recovery WHERE zone_id=?1", [&z.id])?;
        }
    }
    tx.execute(
        "UPDATE zones SET data=?2 WHERE id=?1",
        params![z.id, serde_json::to_string(z)?],
    )?;
    Ok(())
}
pub(crate) fn restart(tx: &Transaction<'_>, old: Zone) -> anyhow::Result<Zone> {
    let mut z = old.clone();
    z.status = "stopped".into();
    z.playback_id = None;
    suspend_record(tx, &old, &z, false,None,true)?;
    Ok(z)
}
/// The caller holds zone_lock and has finished the old statistics baseline.
pub(crate) async fn suspend(s: &AppState, old: &Zone) -> anyhow::Result<()> {
    suspend_output(s,old,false,None).await
}
/// Keep the measured cursor, but a decoder/source failure requires an explicit retry.
/// A control-socket reconnect alone is not evidence that the source became healthy.
pub(crate) async fn suspend_source(s:&AppState,old:&Zone)->anyhow::Result<()> {
    suspend_output(s,old,true,None).await
}
pub(crate) async fn suspend_device_with_stamp(s:&AppState,old:&Zone,stamp:Option<SystemSuspend>)->anyhow::Result<()> {
    if let Some(stamp)=&stamp {stamp.validate().map_err(|e|anyhow::anyhow!(e.2))?;let id=old.id.clone();let previous=s.db.run(move|c|load(c,&id)).await?;if previous.as_ref().is_some_and(|lease|!stamp.newer_than(lease.last_system_suspend.as_ref())){return Ok(());}}
    suspend_output(s,old,true,stamp).await
}
pub(crate) async fn new_system_suspend(s:&AppState,zone:&str,stamp:&SystemSuspend)->anyhow::Result<bool> {
    let zone=zone.to_owned();let previous=s.db.run(move|c|load(c,&zone)).await?;
    Ok(previous.as_ref().is_none_or(|lease|stamp.newer_than(lease.last_system_suspend.as_ref())))
}
async fn suspend_output(s:&AppState,old:&Zone,device_waiting:bool,stamp:Option<SystemSuspend>)->anyhow::Result<()> {
    let mut z = old.clone();
    if z.status == "playing" || stamp.is_some() {
        z.status = "paused".into();
    }
    z.playback_id = None;
    z.updated_at = chrono::Utc::now().timestamp_millis();
    let before = old.clone();
    let saved = z.clone();
    s.db.run(move |c| {
        let tx = c.transaction()?;
          suspend_record(&tx, &before, &saved, device_waiting,stamp.as_ref(),false)?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    super::invalidate_zone(s, &z.id, Some(&z)).await;
    s.zones.write().await.insert(z.id.clone(), z);
    s.emit("zones_changed", json!({}));
    Ok(())
}

/// Caller holds zone_lock. A new listener cannot implicitly prepare the previous listener's cursor.
pub(crate) async fn register_listener(s:&AppState,endpoint:&str,user:&str,session:&str)->anyhow::Result<()> {
    let takeover=s.browser_owners.read().await.get(endpoint).is_some_and(|owner|owner!=session);
    let zones:Vec<_>=s.zones.read().await.values().filter(|z|z.endpoint_id==endpoint&&z.playback_id.is_some()).cloned().collect();
    for zone in zones {
        let id=zone.id.clone();let user=user.to_owned();
        let foreign=takeover||s.db.run(move |c|Ok(load(c,&id)?.is_some_and(|lease|lease.plan.user!=user))).await?;
        if foreign {crate::stats::finish(s,&zone,"disconnect").await?;suspend(s,&zone).await?;}
    }
    Ok(())
}

/// A resume without navigation waits for the endpoint's actual checkpoint. Legacy adapters
/// keep their original play behavior until they can also report a missing local checkpoint.
pub(crate) async fn pending_resume(s:&AppState,z:&Zone,user:&str)->anyhow::Result<bool> {
    if z.playback_id.is_some()||!s.endpoints.read().await.get(&z.endpoint_id).is_some_and(|e|e["capabilities"]["recovery_controls"]==true){return Ok(false);}
    let zone=z.clone();let user=user.to_owned();
    s.db.run(move |c|Ok(load(c,&zone.id)?.is_some_and(|lease|lease.suspended&&lease.plan.user==user&&lease.anchor==signature(&zone)))).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unavailable {endpoint_id:String,#[serde(default)]system_suspend:Option<SystemSuspend>,#[serde(default)]playback_id:Option<String>}
/// The registered adapter has no active local checkpoint, for example after a normal exit.
/// An explicit pending play may use the Core position; registration alone never starts audio.
pub async fn unavailable(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Unavailable>)->ApiResult<Json<Value>> {
    a.require("control")?;let _lock=s.zone_lock.lock().await;
    authorize(&s,&a.session_hash,&v.endpoint_id).await?;
    let mut zone=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
    if let Some(stamp)=v.system_suspend {stamp.validate()?;return fresh::unavailable(&s,zone,&a.id,&v.endpoint_id,stamp,v.playback_id).await;}
    if v.playback_id.is_some(){return Err(ApiError::bad("起播实例须与系统挂起标记一起提交"));}
    if zone.endpoint_id!=v.endpoint_id||!pending_resume(&s,&zone,&a.id).await? {return Ok(Json(json!({"applied":false,"zone":zone})));}
    if zone.status=="playing" {
        zone.renew_playback();crate::zones::dispatch(&s,&zone,"play").await?;
        crate::stats::record_progress(&s,&zone).await?;
    }
    zone.updated_at=chrono::Utc::now().timestamp_millis();
    s.save_zone_action(zone.clone(),Some("recovery_unavailable")).await?;
    Ok(Json(json!({"applied":true,"zone":zone})))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recover {
    #[serde(default="legacy_timing",skip_serializing_if="is_legacy_timing")]
    measured_version:u32,
    endpoint_id: String,
    request_id: String,
    plan_id: String,
    sample_rate: u32,
    heard: Vec<Boundary>,
    terminal: bool,
    // Mutable interruption notification is excluded from the immutable heard-cursor receipt hash.
    #[serde(default,skip_serializing)]
    system_suspend:Option<SystemSuspend>,
}
fn source_matches(baseline:f64,confirmed:f64,report:&Boundary,rate:u32,version:u32)->bool {
    let seconds=report.frames as f64/rate as f64;
    // A reopened/paused v1 output can reset its device frame counter while
    // retaining the source position. Already persisted output is independent
    // proof of that position; this does not credit new listening or permit an
    // offset beyond the durable ledger. v2 still requires its exact frame map.
    if version==1 {return report.position<=baseline+seconds+0.001||report.position<=confirmed+0.001;}
    ((report.position-baseline)-seconds).abs()<=seconds*0.0005+2./rate as f64
}
pub async fn recover(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(v): Json<Recover>,
) -> ApiResult<Json<Value>> {
    a.require("control")?;
    if let Some(stamp)=&v.system_suspend {stamp.validate()?;}
    let version=v.measured_version;
    let respond=|mut value:Value| {if version==2 {value["measured_version"]=json!(2);}Json(value)};
    if ![1,2].contains(&version)||!uuid(&v.request_id)
        || !uuid(&v.plan_id)
        || !(8000..=rmusic_audio::MAX_CLOCK_RATE).contains(&v.sample_rate)
        || v.heard.is_empty()
        || v.heard.len() > LIMIT + 1
    {
        return Err(ApiError::bad("队列恢复参数无效"));
    }
    let mut total = 0.;
    for (i, report) in v.heard.iter().enumerate() {
        total += seconds(report, v.sample_rate, i == 0,version)?;
    }
    if total > 86400. {
        return Err(ApiError::bad("一次恢复最多报告二十四小时的实际帧"));
    }
    let hash = crate::auth::hash(&serde_json::to_string(&v).map_err(anyhow::Error::from)?);
    let _lock = s.zone_lock.lock().await;
    authorize(&s, &a.session_hash, &v.endpoint_id).await?;
    if !s
        .endpoints
        .read()
        .await
        .get(&v.endpoint_id)
        .is_some_and(|e| e["capabilities"]["continuous_recovery"] == true&&(version==1||e["capabilities"]["measured_listens_recovery_v2"]==true&&e["capabilities"]["measured_listens_v2"]==true))
    {
        return Err(ApiError::bad("端点未声明持久队列恢复能力"));
    }
    let zone = s
        .zones
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(ApiError::missing)?;
    let zid = id.clone();
    let request = v.request_id.clone();
    let (lease,receipt)=s.db.run(move |c|Ok((load(c,&zid)?,c.query_row(
        "SELECT user_id,payload_hash,result_plan FROM sequence_recovery_receipts WHERE zone_id=?1 AND request_id=?2",
        params![zid,request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).optional()?))).await?;
    if lease.as_ref().is_some_and(|lease|lease.plan.measured_version!=version){return Err(ApiError::bad("恢复计时版本与持久计划不符"));}
    let mut resume_receipt = false;
    let mut accepted_receipt=false;
    if let Some((user, previous, result_plan)) = receipt {
        if user != a.id {
            return Err(ApiError::forbidden());
        }
        if previous != hash {
            return Err(conflict());
        }
        let accepted = if let Some(plan) = &result_plan {
            let valid = lease
                .as_ref()
                .is_some_and(|l| l.plan.id == *plan && l.anchor == signature(&zone));
            resume_receipt = valid && lease.as_ref().is_some_and(|l| l.suspended);
            valid && !resume_receipt
        } else {
            zone.status == "stopped" && zone.playback_id.is_none() && zone.position == 0.
        };
        if !resume_receipt&&(v.system_suspend.is_none()||!accepted) {
            return Ok(respond(
                json!({"accepted":accepted,"applied":false,"request_id":v.request_id,
            "plan_id":result_plan,"finished":result_plan.is_none(),"zone":zone}),
            ));
        }
        accepted_receipt=accepted;
    }
    let Some(lease) = lease.filter(|l| {
        (l.plan.id == v.plan_id || resume_receipt || accepted_receipt)
            && l.plan.endpoint == v.endpoint_id
            && l.anchor == signature(&zone)
            && zone.endpoint_id == v.endpoint_id
    }) else {
        return Ok(respond(json!({"accepted":false,"applied":false,"zone":zone})));
    };
    if lease.plan.user != a.id {
        return Err(ApiError::forbidden());
    }
    let new_suspend=v.system_suspend.as_ref().is_some_and(|stamp|stamp.newer_than(lease.last_system_suspend.as_ref()));
    if accepted_receipt&&new_suspend {
        crate::stats::finish(&s,&zone,"disconnect").await?;suspend_device_with_stamp(&s,&zone,v.system_suspend.clone()).await?;
        let held=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
        return Ok(respond(json!({"accepted":false,"applied":false,"waiting_for_play":true,"zone":held})));
    }
    if lease.device_waiting&&!new_suspend {
        return Ok(respond(json!({"accepted":false,"applied":false,"waiting_for_play":true,"zone":zone})));
    }
    if accepted_receipt {return Ok(respond(json!({"accepted":true,"applied":false,"zone":zone,"system_suspend_ack":v.system_suspend})));}
    if s.zones
        .read()
        .await
        .values()
        .any(|z| z.id != id && z.endpoint_id == v.endpoint_id && z.status == "playing")
    {
        return Ok(respond(json!({"accepted":false,"applied":false,"zone":zone})));
    }
    let mut base = zone.clone();
    base.playback_id = Some(lease.plan.current.clone());
    // A previously committed restore needs a fresh binding after another transport/Core loss.
    // Its receipt authorizes the latest canonical cursor, never the original older boundaries.
    let canonical = Boundary {
        playback_id: lease.plan.current.clone(),
        track_id: base.track.as_ref().unwrap().id.clone(),
        position: base.position,
        frames: 0,
    };
    let mut canonical_only = resume_receipt;
    let mut heard = if resume_receipt {
        vec![canonical.clone()]
    } else {
        v.heard.clone()
    };
    let mut known: Vec<_> = lease.plan.past.iter().cloned().collect();
    let current = known.len();
    known.push(current_slot(&base));
    known.extend(lease.plan.slots.iter().cloned());
    let start = match known
        .iter()
        .position(|slot| slot.playback_id == heard[0].playback_id)
    {
        Some(start) => start,
        None => {
            // A slow durable cursor can lag beyond the eight retained historical slots.
            // Only proven finished sources may be discarded; resume the canonical queue instead.
            let reports = heard.clone();
            let user = a.id.clone();
            let rate = v.sample_rate;
            let closed =
                s.db.run(move |c| {
                    for report in reports {
                        let Some((baseline, confirmed, Some(end))) =
                            crate::stats::measured::recovery_position(
                                c,
                                &user,
                                &report.playback_id,
                                &report.track_id,
                            )?
                        else {
                            return Ok(false);
                        };
                        if report.position > end + 0.001
                            || !source_matches(baseline,confirmed,&report,rate,version)
                        {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                })
                .await?;
            if !closed {
                return Ok(respond(json!({"accepted":false,"applied":false,"zone":zone})));
            }
            canonical_only = true;
            heard = vec![canonical];
            current
        }
    };
    if start + heard.len() > known.len()
        || heard
            .iter()
            .zip(&known[start..])
            .any(|(r, slot)| r.playback_id != slot.playback_id || r.track_id != slot.track.id)
    {
        return Err(ApiError::bad("恢复检查点未按授权队列顺序报告"));
    }
    let reports = if canonical_only {
        Vec::new()
    } else {
        heard.clone()
    };
    let user = a.id.clone();
    let rate = v.sample_rate;
    let sources = known.clone();
    let positions =
        s.db.run(move |c| {
            // A frame report may have started before prepare; the issued baseline still bounds any offset.
            for report in &reports {
                let Some((baseline, confirmed, _)) = crate::stats::measured::recovery_position(
                    c,
                    &user,
                    &report.playback_id,
                    &report.track_id,
                )?
                else {
                    anyhow::bail!("unissued recovery source");
                };
                if !source_matches(baseline,confirmed,report,rate,version) {
                    anyhow::bail!("recovery position exceeds source frames");
                }
            }
            sources
                .iter()
                .map(|slot| {
                    crate::stats::measured::recovery_position(
                        c,
                        &user,
                        &slot.playback_id,
                        &slot.track.id,
                    )
                })
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await
        .map_err(|_| ApiError::bad("恢复位置与已授权音源不符"))?;
    let last = start + heard.len() - 1;
    if new_suspend {
        crate::stats::finish(&s,&zone,"disconnect").await?;suspend_device_with_stamp(&s,&zone,v.system_suspend.clone()).await?;
        let held=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;
        return Ok(respond(json!({"accepted":false,"applied":false,"waiting_for_play":true,"zone":held})));
    }
    let mut target = current.max(last);
    let mut next = base.clone();
    for slot in known.iter().take(target + 1).skip(current + 1) {
        apply(&mut next, slot);
    }
    let known_position = positions[target]
        .as_ref()
        .ok_or_else(|| ApiError::bad("恢复实例没有听音授权"))?
        .1;
    next.position = next.position.max(known_position);
    if last >= current {
        next.position = next.position.max(heard.last().unwrap().position);
    }
    // Never replay a source whose EOF was already durably imported, even if the cursor lagged it.
    let mut ended: Vec<_> = known
        .iter()
        .take(target)
        .skip(current)
        .map(|slot| slot.playback_id.clone())
        .collect();
    while (v.terminal && !canonical_only && target == last)
        || positions[target].as_ref().is_some_and(|p| p.2.is_some())
    {
        ended.push(known[target].playback_id.clone());
        target += 1;
        if target >= known.len() {
            break;
        }
        apply(&mut next, &known[target]);
        next.position = positions[target].as_ref().map(|p| p.1).unwrap_or(0.);
    }
    let old_source = if target < known.len() {
        Some(known[target].playback_id.clone())
    } else {
        None
    };
    if old_source.is_none() {
        crate::zones::advance(&s, &mut next, false).await;
    }
    if next.status != "stopped" || old_source.is_some() {
        next.status = lease.desired_status;
        next.renew_playback();
    }
    crate::zones::processing_allowed(&s, &next).await?;
    next.updated_at = chrono::Utc::now().timestamp_millis();
    let mut plan = Plan {
        measured_version:version,
        group_start_id:None,
        id: uuid::Uuid::new_v4().to_string(),
        owner: a.session_hash.clone(),
        user: a.id.clone(),
        endpoint: v.endpoint_id.clone(),
        current: next.playback_id.clone().unwrap_or_default(),
        signature: signature(&next),
        past: VecDeque::new(),
        slots: VecDeque::new(),
    };
    if next.playback_id.is_some() {
        fill(&s,&mut plan, &next).await;
    }
    s.revise_queues(std::slice::from_mut(&mut next)).await;
    let saved = next.clone();
    let saved_plan = plan.clone();
    let request = v.request_id.clone();
    let user = a.id;
    let result_plan = next.playback_id.as_ref().map(|_| plan.id.clone());
    let receipt_plan = result_plan.clone();
    let reports = heard;
    let previous = lease.plan.current;
    let last_system_suspend=lease.last_system_suspend;
    let evidence = known;
    let expected = positions;
    let applied=s.db.run(move |c| {
        let tx=c.transaction()?;
        // Journal uploads do not hold zone_lock. Recheck their evidence in this transaction,
        // so a concurrently imported EOF/position cannot be replayed by a stale restore read.
        for (slot,expected) in evidence.iter().zip(&expected) {
            if crate::stats::measured::recovery_position(&tx,&user,&slot.playback_id,&slot.track.id)?!=*expected{return Ok(false);}
        }
        for playback in ended {let position=reports.iter().find(|r|r.playback_id==playback).map(|r|r.position);
            crate::stats::measured::mark_ended(&tx,&playback,position,saved.updated_at)?;}
        crate::stats::restore_sequence_tx(&tx,&user,&previous,old_source.as_deref(),&saved)?;
        tx.execute("UPDATE zones SET data=?2 WHERE id=?1",params![saved.id,serde_json::to_string(&saved)?])?;
        tx.execute("DELETE FROM sequence_recovery WHERE zone_id=?1",[&saved.id])?;
        if saved.playback_id.is_some() {
            for slot in &saved_plan.slots {crate::stats::measured::issue_next_timed(&tx,&user,&slot.playback_id,&slot.track,version)?;}
            save_plan(&tx,&saved,&saved_plan)?;
            if let Some(mut lease)=load(&tx,&saved.id)? {lease.last_system_suspend=last_system_suspend;store(&tx,&saved.id,&lease)?;}
        }
        tx.execute("INSERT INTO sequence_recovery_receipts(zone_id,request_id,user_id,payload_hash,result_plan,created_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(zone_id,request_id) DO UPDATE SET result_plan=excluded.result_plan,created_at=excluded.created_at",params![saved.id,request,user,hash,receipt_plan,saved.updated_at])?;
        tx.execute("DELETE FROM sequence_recovery_receipts WHERE zone_id=?1 AND rowid NOT IN (SELECT rowid FROM sequence_recovery_receipts WHERE zone_id=?1 ORDER BY rowid DESC LIMIT 128)",[&saved.id])?;
        tx.commit()?;Ok(true)
    }).await?;
    if !applied {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "recovery_changed".into(),
            "听音检查点已更新，请重试原恢复请求".into(),
        ));
    }
    super::invalidate_zone(&s, &id, Some(&next)).await;
    if next.playback_id.is_some() {
        s.sequences.lock().await.insert(id.clone(), plan);
    }
    s.zones.write().await.insert(id.clone(), next.clone());
    s.emit("zones_changed", json!({}));
    s.emit("stats", json!({"zone_id":id}));
    crate::autoplay::prepare(&s, &next).await;
    // The requesting adapter opens this new instance after receiving the response; no old play is dispatched.
    Ok(respond(
        json!({"accepted":true,"applied":true,"request_id":v.request_id,"plan_id":result_plan,
        "finished":next.playback_id.is_none(),"zone":next,"system_suspend_ack":v.system_suspend}),
    ))
}
