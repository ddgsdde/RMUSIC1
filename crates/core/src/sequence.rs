//! Reserve queue instances without starting playback; commit only boundaries actually heard.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::{Track, Zone},
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
use std::collections::VecDeque;

const LIMIT: usize = 8;
fn legacy_timing()->u32{1}
fn is_legacy_timing(version:&u32)->bool{*version==1}
pub mod recovery;

#[derive(Clone, Deserialize, Serialize)]
struct Slot {
    playback_id: String,
    index: usize,
    track: Track,
    // A shuffled wrap or first recommended source carries its future queue projection.
    queue: Option<Vec<Track>>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Plan {
    #[serde(default="legacy_timing",skip_serializing_if="is_legacy_timing")]
    measured_version:u32,
    #[serde(default,skip_serializing_if="Option::is_none")]
    group_start_id: Option<String>,
    id: String,
    owner: String,
    user: String,
    endpoint: String,
    current: String,
    signature: String,
    past: VecDeque<Slot>,
    slots: VecDeque<Slot>,
}

fn signature(zone: &Zone) -> String {
    signature_with_loudness(zone, false)
}

pub(crate) fn signature_with_loudness(zone: &Zone, legacy: bool) -> String {
    let tracks: Vec<_> = zone
        .queue
        .iter()
        .map(|t| {
            let mut track = json!({
                "id":t.id,"source":t.source,"format":t.format,"sample_rate":t.sample_rate,
                "bit_depth":t.bit_depth,"channels":t.channels,"duration":t.duration,
                "available":t.available
            });
            if legacy { track["integrated_lufs"] = json!(t.integrated_lufs); }
            track
        })
        .collect();
    crate::auth::hash(
        &json!({
            "endpoint":zone.endpoint_id,"playback_id":zone.playback_id,
            "track_id":zone.track.as_ref().map(|t| &t.id),"queue":tracks,
            "index":zone.queue_index,"repeat":zone.repeat,"shuffle":zone.shuffle,
            "autoplay":zone.autoplay,"dsp":zone.dsp,"transitions":zone.transitions
        })
        .to_string(),
    )
}

pub(crate) async fn loudness_successors(s: &AppState) -> std::collections::HashMap<String, (String, Track)> {
    s.sequences.lock().await.iter().filter_map(|(zone, plan)| {
        let next = plan.slots.front()?;
        Some((zone.clone(), (plan.current.clone(), next.track.clone())))
    }).collect()
}

pub(crate) async fn update_loudness(s: &AppState, measured: &Track) -> Vec<(String, String)> {
    let mut reserved = Vec::new();
    for plan in s.sequences.lock().await.values_mut() {
        for slot in plan.slots.iter_mut().chain(plan.past.iter_mut()) {
            if crate::loudness::update_track(&mut slot.track, measured) {
                reserved.push((plan.endpoint.clone(), slot.playback_id.clone()));
            }
            if let Some(queue) = &mut slot.queue {
                for track in queue { crate::loudness::update_track(track, measured); }
            }
        }
    }
    reserved
}

/// A Ready acknowledgement requires the still-owned reservations for this cohort epoch.
pub(crate) async fn group_prepared(s:&AppState,zone:&str,playback:&str,group:&str)->bool {s.sequences.lock().await.get(zone).is_some_and(|p|p.current==playback&&p.group_start_id.as_deref()==Some(group))}
/// Group cancellation owns the output stop; discard only its reservations without another command.
pub(crate) async fn discard_group_preparation(s:&AppState,group:&str) {
    s.sequences.lock().await.retain(|_,plan|plan.group_start_id.as_deref()!=Some(group));
}
/// Caller serializes zone mutations. Gain, presentation metadata and pause preserve reservations.
pub async fn invalidate_if_changed(s: &AppState, zone: &Zone) {
    let changed = s.sequences.lock().await.get(&zone.id).is_some_and(|plan| {
        plan.signature != signature(zone) || !matches!(zone.status.as_str(), "playing" | "paused")
    });
    if changed {
        invalidate_zone(s, &zone.id, Some(zone)).await;
    }
}
/// Queue policy changes retain the current device run and rebase its admitted boundaries.
pub(crate) async fn refresh_queue(s:&AppState,zone:&Zone,action:Option<&str>)->Option<Plan> {
    if !matches!(zone.status.as_str(),"playing"|"paused") {return None;}
    if !s.endpoints.read().await.get(&zone.endpoint_id).is_some_and(|endpoint|endpoint["online"]==true&&endpoint["capabilities"]["continuous_queue_refresh"]==true) {return None;}
    let old=s.zones.read().await.get(&zone.id).cloned()?;
    let (a,b)=(old.track.as_ref()?,zone.track.as_ref()?);
    if old.endpoint_id!=zone.endpoint_id||old.playback_id!=zone.playback_id||old.dsp!=zone.dsp
        ||old.transitions.fade_in_ms!=zone.transitions.fade_in_ms||old.transitions.fade_out_ms!=zone.transitions.fade_out_ms
        ||a.id!=b.id {return None;}
    let mut plan=s.sequences.lock().await.get(&zone.id)?.clone();
    if plan.group_start_id.is_some()||zone.playback_id.as_deref()!=Some(&plan.current)||plan.signature!=signature(&old) {return None;}
    let key=signature(zone);if key==plan.signature&&action!=Some("queue") {return None;}
    for slot in &mut plan.past {slot.queue=None;slot.index=zone.queue.iter().position(|t|t.id==slot.track.id).unwrap_or(zone.queue_index);}
    let mut index=zone.queue_index;
    for slot in &mut plan.slots {
        slot.queue=None;
        index=zone.queue.iter().enumerate().skip(index.saturating_add(1)).find(|(_,track)|track.id==slot.track.id)
            .or_else(||zone.queue.iter().enumerate().find(|(_,track)|track.id==slot.track.id)).map(|(i,_)|i).unwrap_or(index);
        slot.index=index;
    }
    plan.signature=key;Some(plan)
}
pub(crate) async fn publish_queue_refresh(s:&AppState,zone:&Zone,plan:Plan) {
    let message=json!({"type":"cancel_preload","zone_id":zone.id,"plan_id":plan.id,"preserve_current":true,
        "zone":{"endpoint_id":zone.endpoint_id,"status":zone.status,"playback_id":zone.playback_id}});
    s.sequences.lock().await.insert(zone.id.clone(),plan);
    if let Some(tx)=s.bridges.read().await.get(&zone.endpoint_id) {if let Err(error)=tx.try_send(message.clone()) {tracing::warn!(zone_id=zone.id,%error,"queue refresh was not delivered");}}
    s.emit("sequence_invalidated",message);
}
/// Tail additions retain the playing cursor and its already-reserved boundaries.
pub(crate) async fn append(s:&AppState,z:&mut Zone,tracks:&[Track])->anyhow::Result<bool>{
 if z.shuffle{return Ok(false);}
 let mut plans=s.sequences.lock().await;
 let Some(plan)=plans.get_mut(&z.id).filter(|p|p.group_start_id.is_none()&&z.playback_id.as_deref()==Some(&p.current)&&p.signature==signature(z))else{return Ok(false);};
 // Recommendations already admitted to the output remain before newly appended music.
 if let Some(projected)=plan.slots.iter().rev().find_map(|slot|slot.queue.as_ref()){z.queue=projected.clone();}
 if z.queue.len()+tracks.len()>10000{anyhow::bail!("队列上限为一万首");}
 z.queue.extend_from_slice(tracks);
 for slot in &mut plan.slots{if let Some(queue)=&mut slot.queue{queue.extend_from_slice(tracks);}}
 plan.signature=signature(z);let saved=plan.clone();let zone=z.clone();
 s.db.run(move|c|{let tx=c.transaction()?;recovery::advance_plan(&tx,&zone,&saved)?;tx.commit()?;Ok(())}).await?;Ok(true)
}
pub async fn invalidate(s: &AppState, id: &str) -> anyhow::Result<()> {
    let zid = id.to_owned();
    s.db.run(move |c| {c.execute("DELETE FROM sequence_recovery WHERE zone_id=?1",[zid])?;Ok(())}).await?;
    invalidate_zone(s, id, None).await;
    Ok(())
}
pub(crate) async fn invalidate_zone(s: &AppState, id: &str, zone: Option<&Zone>) {
    let Some(plan) = s.sequences.lock().await.remove(id) else {
        return;
    };
    let zone=zone.map(|z|json!({"endpoint_id":z.endpoint_id,"status":z.status,"playback_id":z.playback_id,
        "position":z.position,"dsp":z.dsp,"transitions":z.transitions,"integrated_lufs":z.track.as_ref().and_then(|t|t.integrated_lufs)}));
    let message = json!({"type":"cancel_preload","zone_id":id,"plan_id":plan.id,"zone":zone});
    if let Some(tx) = s.bridges.read().await.get(&plan.endpoint) {
        if let Err(error) = tx.try_send(message.clone()) {
            tracing::warn!(zone_id=id, %error, "continuous preload cancellation was not delivered");
        }
    }
    s.emit("sequence_invalidated", message);
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    #[serde(default="legacy_timing")]
    measured_version:u32,
    endpoint_id: String,
    track_id: String,
    playback_id: String,
    plan_id:Option<String>,
    committed_playback_ids:Option<Vec<String>>,
}
pub(crate) async fn authorize(s: &AppState, owner: &str, endpoint: &str) -> ApiResult<()> {
    let endpoints = s.endpoints.read().await;
    let e = endpoints.get(endpoint).ok_or_else(ApiError::missing)?;
    if e["online"] != true
        || e["capabilities"]["continuous_pcm"] != true
        || e["capabilities"]["playback_instance"] != true
    {
        return Err(ApiError::bad("端点未声明连续 PCM 播放能力"));
    }
    let browser = e["kind"] == "browser";
    drop(endpoints);
    let registered = if browser {
        s.browser_owners.read().await.get(endpoint).cloned()
    } else {
        s.bridge_owners.read().await.get(endpoint).cloned()
    };
    if registered.as_deref() != Some(owner) {
        return Err(ApiError::forbidden());
    }
    Ok(())
}
fn matches(zone: &Zone, track: &str, playback: &str) -> bool {
    zone.playback_id.as_deref() == Some(playback)
        && zone.track.as_ref().is_some_and(|t| t.id == track)
}
fn apply(zone: &mut Zone, slot: &Slot) {
    if let Some(queue) = &slot.queue {
        let latest: std::collections::HashMap<_, _> =
            zone.queue.iter().map(|t| (&t.id, t)).collect();
        zone.queue = queue
            .iter()
            .map(|t| latest.get(&t.id).copied().unwrap_or(t).clone())
            .collect();
    }
    zone.queue_index = slot.index;
    // Keep refreshed display metadata if the queue has not been reordered since reservation.
    zone.track = zone
        .queue
        .get(slot.index)
        .filter(|t| t.id == slot.track.id)
        .cloned()
        .or_else(|| Some(slot.track.clone()));
    zone.playback_id = Some(slot.playback_id.clone());
    zone.position = 0.;
}
async fn fill(s:&AppState,plan: &mut Plan, zone: &Zone)->bool {
    let mut tail = zone.clone();
    for slot in &plan.slots {
        apply(&mut tail, slot);
    }
    while plan.slots.len() < LIMIT && !tail.queue.is_empty() {
        let mut reordered = false;
        if plan.group_start_id.is_some() {
            let Some(next)=tail.queue.get(tail.queue_index+1)else{break;};
            if tail.track.as_ref().is_none_or(|track|!crate::sync_start::queue::same_format(track,next)){break;}
            tail.queue_index+=1;
        } else if tail.repeat == "one" {
        } else if tail.queue_index + 1 < tail.queue.len() {
            tail.queue_index += 1;
        } else if tail.repeat == "all" {
            tail.queue_index = 0;
            if tail.shuffle {
                crate::zones::shuffle_tail(&mut tail);
                reordered = true;
            }
        } else {
            let (items,pending)=crate::autoplay::reserve(s,&tail).await;
            let Some(items)=items else {return pending;};
            tail.queue_index=crate::autoplay::extend_queue(&mut tail,items);reordered=true;
        }
        let Some(track) = tail.queue.get(tail.queue_index).cloned() else {
            break;
        };
        if plan.measured_version==2&&track.bit_depth==1 {break;}
        let slot = Slot {
            playback_id: uuid::Uuid::new_v4().to_string(),
            index: tail.queue_index,
            track,
            queue: reordered.then(|| tail.queue.clone()),
        };
        apply(&mut tail, &slot);
        plan.slots.push_back(slot);
    }
    if plan.group_start_id.is_none(){crate::autoplay::reserve(s,&tail).await;}
    false
}
pub async fn prepare(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(v): Json<Prepare>,
) -> ApiResult<Json<Value>> {
    a.require("control")?;
    let _lock = s.zone_lock.lock().await;
    if ![1,2].contains(&v.measured_version){return Err(ApiError::bad("听音记录协议版本无效"));}
    let version=v.measured_version;
    authorize(&s, &a.session_hash, &v.endpoint_id).await?;
    let group=crate::sync_start::queue::preparation(&s,&id,&v.endpoint_id,&v.track_id,&v.playback_id).await;
    if group.is_none()&&crate::sync_start::active_zone(&s,&id).await{let zone=s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?;return Ok(Json(json!({"accepted":false,"zone":zone})));}
    if let Some((group,_))=&group{if crate::sync_start::queue::timing(&s,group).await!=Some(version){return Err(ApiError::bad("分组准备计时版本不符"));}}
    let zone = if let Some((_,zone))=&group{zone.clone()}else{s.zones.read().await.get(&id).cloned().ok_or_else(ApiError::missing)?};
    if zone.endpoint_id != v.endpoint_id
        || !matches(&zone, &v.track_id, &v.playback_id)
        || !matches!(zone.status.as_str(), "playing" | "paused")
    {
        return Ok(Json(json!({"accepted":false,"zone":zone})));
    }
    if version==2 {
        if zone.track.as_ref().is_some_and(|track|track.bit_depth==1){return Err(ApiError::bad("双时间变速协议仅支持 PCM"));}
        if !s.endpoints.read().await.get(&v.endpoint_id).is_some_and(|e|e["capabilities"]["measured_listens_v2"]==true){return Err(ApiError::bad("端点未声明双时间听音记录能力"));}
    }
    let playback=v.playback_id.clone();
    let enrolled=s.db.run(move|c|Ok(c.query_row("SELECT timing_version FROM listen_claims WHERE playback_id=?1",[playback],|r|r.get::<_,u32>(0)).optional()?)).await?;
    if enrolled.is_some_and(|stored|stored!=version){return Err(ApiError::bad("本播放实例已绑定另一听音记录版本"));}
    let key = signature(&zone);
    if v.plan_id.is_some()!=v.committed_playback_ids.is_some()||v.committed_playback_ids.as_ref().is_some_and(|ids|ids.len()>32) {return Err(ApiError::bad("队列刷新参数无效"));}
    let existing = s
        .sequences
        .lock()
        .await
        .get(&id)
        .cloned()
        .filter(|p| p.owner == a.session_hash && p.signature == key && p.group_start_id.as_deref()==group.as_ref().map(|(id,_)|id.as_str()));
    if let Some(plan_id)=&v.plan_id {
        if group.is_some()||!s.endpoints.read().await.get(&v.endpoint_id).is_some_and(|e|e["capabilities"]["continuous_queue_refresh"]==true) {return Err(ApiError::bad("端点未声明无中断队列刷新能力"));}
        if existing.as_ref().is_none_or(|plan|&plan.id!=plan_id) {return Ok(Json(json!({"accepted":false,"zone":zone})));}
    }
    let mut plan=existing.unwrap_or_else(|| Plan {
            measured_version:version,
            group_start_id:group.as_ref().map(|(id,_)|id.clone()),
            id: uuid::Uuid::new_v4().to_string(),
            owner: a.session_hash.clone(),
            user: a.id.clone(),
            endpoint: v.endpoint_id,
            current: v.playback_id,
            signature: key,
            past: VecDeque::new(),
            slots: VecDeque::new(),
        });
    if let Some(committed)=v.committed_playback_ids {
        let keep=plan.slots.iter().take_while(|slot|committed.contains(&slot.playback_id)).count();
        if plan.slots.iter().skip(keep).any(|slot|committed.contains(&slot.playback_id)) {return Err(ApiError::bad("已提交音频必须按预加载顺序保留"));}
        plan.slots.truncate(keep);
    }
    let refresh=fill(&s,&mut plan, &zone).await;
    let mut tail = zone.clone();
    for slot in &plan.slots {
        apply(&mut tail, slot);
    }
    let has_more =
        if plan.group_start_id.is_some(){tail.track.as_ref().zip(tail.queue.get(tail.queue_index+1)).is_some_and(|(track,next)|crate::sync_start::queue::same_format(track,next))}
        else{tail.repeat == "one" || tail.repeat == "all" || tail.queue_index + 1 < tail.queue.len()};
    let mut items = Vec::with_capacity(plan.slots.len());
    let latest: std::collections::HashMap<_, _> = zone.queue.iter().map(|t| (&t.id, t)).collect();
    for slot in &plan.slots {
        let track = latest.get(&slot.track.id).copied().unwrap_or(&slot.track);
        let media_path=crate::render_media::path(&s,&zone,track,Some(a.session_hash.clone())).await?;
        items.push(json!({"playback_id":slot.playback_id,"track":track,"media_path":media_path}));
        if let Some(queue)=&slot.queue {let item=items.last_mut().unwrap();item["queue"]=json!(queue);item["queue_index"]=json!(slot.index);}
    }
    if let Some(group_start_id)=&plan.group_start_id {
        crate::sync_start::queue::register(&s,group_start_id,&id,plan.slots.iter().map(|slot|(slot.playback_id.clone(),slot.index)).collect()).await?;
        let mut response=json!({"accepted":true,"plan_id":plan.id,"group_start_id":group_start_id,"from_playback_id":zone.playback_id,"items":items,"has_more":has_more,"refresh":false,"continuous_recovery":false});
        if version==2{response["measured_version"]=json!(2);}
        s.sequences.lock().await.insert(id,plan);
        s.loudness.wake.notify_one();
        return Ok(Json(response));
    }
    let durable=s.endpoints.read().await.get(&plan.endpoint).is_some_and(|e|e["capabilities"]["continuous_recovery"]==true&&(version==1||e["capabilities"]["measured_listens_recovery_v2"]==true));
    let issued_zone=zone.clone();let issued_owner=a.id.clone();let issued:Vec<_>=plan.slots.iter().map(|slot|(slot.playback_id.clone(),slot.track.clone())).collect();let saved_plan=plan.clone();
    let listen_session_id=s.db.run(move |c|{let tx=c.transaction()?;let current=crate::stats::measured::issue_current_timed(&tx,&issued_owner,&issued_zone,version)?;
        let queue_owner=crate::radio::owner_at(&tx,&issued_zone.id)?;
        for (playback,track) in issued {
            if let Some(owner)=&queue_owner {crate::accounts::listening::own(&tx,&playback,owner)?;}
            crate::stats::measured::issue_next_timed(&tx,&issued_owner,&playback,&track,version)?;
        }
        if durable {recovery::save_plan(&tx,&issued_zone,&saved_plan)?;}tx.commit()?;Ok(current)}).await?;
    for item in &mut items {item["listen_session_id"]=item["playback_id"].clone();}
    let mut response = json!({"accepted":true,"plan_id":plan.id,"from_playback_id":zone.playback_id,"listen_session_id":listen_session_id,"items":items,"has_more":has_more,"refresh":refresh,"continuous_recovery":durable});
    if version==2 {response["measured_version"]=json!(2);}
    s.sequences.lock().await.insert(id, plan);
    s.loudness.wake.notify_one();
    Ok(Json(response))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Boundary {
    playback_id: String,
    track_id: String,
    position: f64,
    frames: u64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    #[serde(default="legacy_timing",skip_serializing_if="is_legacy_timing")]
    measured_version:u32,
    endpoint_id: String,
    request_id: String,
    plan_id: String,
    from_playback_id: String,
    sample_rate: u32,
    ended: Vec<Boundary>,
    current: Boundary,
}
fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
fn seconds(report: &Boundary, rate: u32, first: bool, version:u32) -> ApiResult<f64> {
    let seconds = report.frames as f64 / rate as f64;
    if !report.position.is_finite()
        || report.position < 0.
        || seconds > 86400.
        || (version==1 && !first && (report.position - seconds).abs() > 0.001)
        || (version==1 && first && seconds > report.position + 0.001)
    {
        return Err(ApiError::bad("连续播放位置与实际帧数不符"));
    }
    Ok(seconds)
}
fn conflict() -> ApiError {
    ApiError(
        StatusCode::CONFLICT,
        "request_conflict".into(),
        "交接请求 ID 已用于其他内容".into(),
    )
}
pub async fn handoff(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(v): Json<Handoff>,
) -> ApiResult<Json<Value>> {
    a.require("control")?;
    if ![1,2].contains(&v.measured_version)||!uuid(&v.request_id)
        || !uuid(&v.plan_id)
        || !uuid(&v.from_playback_id)
        || !(8000..=rmusic_audio::MAX_CLOCK_RATE).contains(&v.sample_rate)
        || v.ended.is_empty()
        || v.ended.len() > LIMIT
    {
        return Err(ApiError::bad("连续播放交接参数无效"));
    }
    let mut heard = Vec::with_capacity(v.ended.len());
    for (i, report) in v.ended.iter().enumerate() {
        heard.push(seconds(report, v.sample_rate, i == 0,v.measured_version)?);
    }
    let current_seconds = seconds(&v.current, v.sample_rate, false,v.measured_version)?;
    if heard.iter().sum::<f64>() + current_seconds > 86400. {
        return Err(ApiError::bad("一次交接最多报告二十四小时的实际帧"));
    }
    let hash = crate::auth::hash(&serde_json::to_string(&v).map_err(anyhow::Error::from)?);
    let _lock = s.zone_lock.lock().await;
    authorize(&s, &a.session_hash, &v.endpoint_id).await?;
    let zone = s
        .zones
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(ApiError::missing)?;
    let receipt_id = id.clone();
    let request_id = v.request_id.clone();
    let receipt =
        s.db.run(move |c| {
            Ok(c.query_row(
        "SELECT owner,payload_hash FROM sequence_receipts WHERE zone_id=?1 AND request_id=?2",
        params![receipt_id,request_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))
    ).optional()?)
        })
        .await?;
    if let Some((owner, previous)) = receipt {
        if owner != a.session_hash {
            return Err(ApiError::forbidden());
        }
        if previous != hash {
            return Err(conflict());
        }
        return Ok(Json(
            json!({"accepted":true,"applied":false,"request_id":v.request_id,"zone":zone}),
        ));
    }
    let plan = s.sequences.lock().await.get(&id).cloned();
    let Some(mut plan) = plan.filter(|p| {
        p.id == v.plan_id
            && p.owner == a.session_hash
            && p.endpoint == v.endpoint_id
            && p.signature == signature(&zone)
    }) else {
        return Ok(Json(json!({"accepted":false,"applied":false,"zone":zone})));
    };
    if plan.measured_version!=v.measured_version{return Err(ApiError::bad("交接听音记录版本与准备实例不符"));}
    let count = v.ended.len();
    if zone.endpoint_id != v.endpoint_id
        || zone.playback_id.as_deref() != Some(&v.from_playback_id)
        || !matches!(zone.status.as_str(), "playing" | "paused")
        || count > plan.slots.len()
    {
        return Ok(Json(json!({"accepted":false,"applied":false,"zone":zone})));
    }
    let mut next = zone.clone();
    let mut ended = Vec::with_capacity(count);
    for (i, report) in v.ended.iter().enumerate() {
        if !matches(&next, &report.track_id, &report.playback_id) {
            return Err(ApiError::bad("交接未按预载队列顺序报告"));
        }
        next.position = report.position;
        ended.push((next.clone(), heard[i]));
        apply(&mut next, &plan.slots[i]);
    }
    if !matches(&next, &v.current.track_id, &v.current.playback_id) {
        return Err(ApiError::bad("交接中的当前播放实例未预载"));
    }
    next.position = v.current.position;
    let now = chrono::Utc::now().timestamp_millis();
    next.updated_at = now;
    s.revise_queues(std::slice::from_mut(&mut next)).await;
    let committed = next.clone();
    plan.past.push_back(recovery::current_slot(&zone));
    plan.past.extend(plan.slots.drain(..count));
    plan.past.pop_back(); // The last drained slot is now current, rather than historical.
    while plan.past.len()>LIMIT {plan.past.pop_front();}
    plan.current = next.playback_id.clone().unwrap();
    plan.signature = signature(&next);
    if let Some(group)=&plan.group_start_id {
        let ended=ended.into_iter().zip(&v.ended).map(|((zone,_),report)|(zone,report.frames)).collect::<Vec<_>>();
        let applied=crate::sync_start::queue::handoff(&s,group,&zone,&ended,&next,v.sample_rate,v.current.frames,v.measured_version,&v.request_id,&a.session_hash,&hash).await?;
        if !applied{return Ok(Json(json!({"accepted":false,"applied":false,"zone":zone})));}
        s.sequences.lock().await.insert(id,plan);
        return Ok(Json(json!({"accepted":true,"applied":true,"request_id":v.request_id,"zone":next})));
    }
    let saved_plan=plan.clone();
    let owner = a.session_hash;
    let request_id = v.request_id.clone();
    let zid = id.clone();
    let version=v.measured_version;let rate=v.sample_rate;let frames=v.ended.iter().map(|report|report.frames).collect::<Vec<_>>();let current_frames=v.current.frames;
    let applied = s.db.run(move |c| {
        let tx = c.transaction()?;
        let accepted=if version==2 {let reports=ended.iter().zip(&frames).map(|((zone,_),frames)|(zone,*frames)).collect::<Vec<_>>();crate::stats::output::sequence_tx(&tx,&reports,&committed,current_frames,rate,now)?}else{crate::stats::sequence_tx(&tx,&ended,&committed,current_seconds,now)?};
        if !accepted{return Ok(false);}
        tx.execute("UPDATE zones SET data=?2 WHERE id=?1", params![zid,serde_json::to_string(&committed)?])?;
        recovery::advance_plan(&tx,&committed,&saved_plan)?;
        tx.execute("INSERT INTO sequence_receipts(zone_id,request_id,owner,payload_hash,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![zid,request_id,owner,hash,now])?;
        tx.execute("DELETE FROM sequence_receipts WHERE zone_id=?1 AND rowid NOT IN (SELECT rowid FROM sequence_receipts WHERE zone_id=?1 ORDER BY rowid DESC LIMIT 128)",[&zid])?;
        tx.commit()?;
        Ok(true)
    }).await?;
    if !applied {
        let message = "交接的实际进度超出核心计时范围";
        if version == 1 {
            return Err(ApiError(StatusCode::BAD_REQUEST, "handoff_timing_conflict".into(), message.into()));
        }
        return Err(ApiError::bad(message));
    }
    s.sequences.lock().await.insert(id.clone(), plan);
    s.zones.write().await.insert(id.clone(), next.clone());
    s.emit("zones_changed", json!({}));
    s.emit("stats", json!({"zone_id":id}));
    crate::autoplay::prepare(&s, &next).await;
    Ok(Json(
        json!({"accepted":true,"applied":true,"request_id":v.request_id,"zone":next}),
    ))
}
