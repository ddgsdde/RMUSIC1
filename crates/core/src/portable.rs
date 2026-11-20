//! Remote control projections for locally rendered native playback.
use crate::{accounts::scope, auth::Auth, error::{ApiError, ApiResult}, models::Zone, state::AppState};
use axum::{extract::{Path, State}, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};
use tokio::sync::{oneshot, Mutex, Semaphore};

const PREFIX: &str = "portable-";
const LEASE: Duration = Duration::from_secs(65);
const COMMAND_BUDGET: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(crate) struct Registry {
    players: Mutex<HashMap<String, Player>>,
    pending: Mutex<HashMap<String, Pending>>,
}
struct Player {
    user: String,
    realm: String,
    session: String,
    platform: String,
    last_seen: Instant,
    stamp: i64,
    zone: Zone,
    gate: Arc<Semaphore>,
}
struct Pending {
    session: String,
    id: String,
    response: oneshot::Sender<Result<Zone,String>>,
}
#[derive(Deserialize)]
pub struct Snapshot {
    platform: String,
    zone: Value,
    stamp: i64,
}
#[derive(Deserialize)]
pub struct Ack {
    request_id: String,
    zone: Option<Value>,
    stamp: i64,
    error: Option<String>,
}
pub(crate) fn is_portable(id: &str) -> bool { id.starts_with(PREFIX) }
fn valid_id(id: &str) -> bool {
    is_portable(id)&&id.len()>PREFIX.len()&&id.len()<=150&&id.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-')
}
fn allowed(a:&Auth,p:&Player) -> bool {
    crate::lan_bridges::endpoint(a).is_none()&&scope::realm(a)==p.realm
}
fn unavailable() -> ApiError {
    ApiError(StatusCode::SERVICE_UNAVAILABLE,"portable_offline".into(),"外出播放器未连接或未确认命令".into())
}
fn normalize(id:&str,mut raw:Value,old:Option<&Zone>) -> ApiResult<Zone> {
    fn track(value:&mut Value) {
        if let Some(map)=value.as_object_mut(){
            for (key,default) in [("album_artist",json!("")),("source",json!("local")),("track_number",json!(0)),("disc_number",json!(0)),
                ("channels",json!(0)),("bit_depth",json!(0)),("bitrate",json!(0)),("recording_id",json!("")),("available",json!(true)),("created_at",json!(0))] {
                let value=map.entry(key).or_insert(Value::Null);
                if value.is_null(){*value=default;}
            }
        }
    }
    let buffering=raw["buffering"]==true;
    if let Some(map)=raw.as_object_mut() {
        map.insert("updated_at".into(),json!(0));
        map.entry("repeat").or_insert(json!("off"));
        map.entry("shuffle").or_insert(json!(false));
        if !map.contains_key("queue") {
            let old=old.ok_or_else(unavailable)?;
            map.insert("queue".into(),json!(old.queue));
        }
        for key in ["autoplay","transitions","queue_revision"] {if map.get(key).is_some_and(Value::is_null){map.remove(key);}}
        if let Some(value)=map.get_mut("track"){track(value);}
        if let Some(queue)=map.get_mut("queue").and_then(Value::as_array_mut){for value in queue{track(value);}}
    }
    let mut zone:Zone=serde_json::from_value(raw).map_err(|_|ApiError::bad("外出播放器状态格式无效"))?;
    if buffering&&zone.status=="playing"{zone.status="paused".into();}

    if zone.queue.len()>10_000 || zone.name.trim().is_empty() || zone.name.len()>256 || !zone.position.is_finite() || zone.position<0.
        || !zone.volume.is_finite() || !(0. ..=1.).contains(&zone.volume)
        || !matches!(zone.status.as_str(),"playing"|"paused"|"stopped") || (!zone.queue.is_empty()&&zone.queue_index>=zone.queue.len())
    {return Err(ApiError::bad("外出播放器状态无效"));}
    if zone.status=="playing"&&zone.track.is_none(){return Err(ApiError::bad("外出播放器没有当前歌曲"));}
    zone.id=id.into();
    zone.endpoint_id=id.into();
    zone.updated_at=chrono::Utc::now().timestamp_millis();
    zone.queue_revision=old.map_or(0,|z|z.queue_revision);
    AppState::revise_queue(&mut zone,old);
    Ok(zone)
}
// A Mac publishes its CoreAudio endpoint in the snapshot. Bind only outputs registered
// to the host of this login session, within the caller's normal zone permissions.
async fn home_output(s: &AppState, a: &Auth, snapshot: &Snapshot) -> ApiResult<Option<(String, String, String)>> {
    let Some(endpoint) = snapshot.zone["endpoint_id"].as_str().filter(|id| !id.is_empty() && id.len() <= 200) else { return Ok(None) };
    if snapshot.platform != "macos" { return Ok(None); }
    let home = s.zones.read().await.values().filter(|z| z.endpoint_id == endpoint)
        .min_by(|a,b| a.id.cmp(&b.id)).map(|z| (z.id.clone(), z.endpoint_id.clone(), z.name.clone()));
    let Some(home) = home else { return Ok(None) };
    let (session, output) = (a.session_hash.clone(), endpoint.to_owned());
    let registered: bool = s.db.read(move |db| Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM login_clients l JOIN devices d ON d.id=l.client_id, json_each(d.outputs) o
         WHERE l.session_hash=?1 AND l.device_type='macos' AND d.platform='macos' AND json_extract(o.value,'$.id')=?2)",
        rusqlite::params![session,output], |r| r.get(0))?)).await?;
    if !registered { return Ok(None); }
    scope::check(s,a,"zone",&home.0).await?;
    scope::check(s,a,"endpoint",&home.1).await?;
    Ok(Some(home))
}

pub(crate) async fn control_id(s: &AppState, a: &Auth, id: &str) -> Option<String> {
    if is_portable(id) { return Some(id.to_owned()); }
    s.portable.players.lock().await.iter().find(|(_,p)| p.zone.id == id && allowed(a,p) && p.last_seen.elapsed() < LEASE)
        .map(|(key,_)| key.clone())
}

pub(crate) fn overlay(items: &mut Vec<Value>, active: Vec<Value>) {
    for value in active {
        if let Some(stored) = items.iter_mut().find(|item| item["id"] == value["id"]) { *stored = value; }
        else { items.push(value); }
    }
}

pub async fn publish(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(snapshot):Json<Snapshot>) -> ApiResult<StatusCode> {
    a.require("control")?;
    if !a.interactive||!valid_id(&id)||!matches!(snapshot.platform.as_str(),"android"|"macos"){return Err(ApiError::forbidden());}
    let home = home_output(&s, &a, &snapshot).await?;
    let mut players=s.portable.players.lock().await;
    if let Some(current)=players.get(&id) {
        if current.user!=a.id{return Err(ApiError::missing());}
    }else if players.values().filter(|p|p.user==a.id).count()>=32 {
        return Err(ApiError::bad("外出设备数量已达上限"));
    }
    let current=players.get(&id);
    if current.is_some_and(|p|p.session==a.session_hash&&snapshot.stamp<=p.stamp){return Ok(StatusCode::NO_CONTENT);}
    let mut zone=normalize(&id,snapshot.zone,current.map(|p|&p.zone))?;
    if let Some((zone_id, endpoint, name)) = home {
        zone.id = zone_id;
        zone.endpoint_id = endpoint;
        zone.name = name;
        if current.is_none_or(|p| p.zone.id != zone.id) { zone.queue_revision = zone.updated_at as u64; }
    }
    let new_connection=current.is_none_or(|p|p.session!=a.session_hash||p.last_seen.elapsed()>LEASE);
    let structural=new_connection||current.is_some_and(|p|p.zone.id!=zone.id||p.zone.queue_revision!=zone.queue_revision||
        p.zone.status!=zone.status||p.zone.playback_id!=zone.playback_id||p.zone.volume!=zone.volume||
        p.zone.muted!=zone.muted||p.zone.name!=zone.name||p.zone.repeat!=zone.repeat||p.zone.shuffle!=zone.shuffle||
        p.zone.autoplay.enabled!=zone.autoplay.enabled);
    let gate=current.filter(|_|!new_connection).map(|p|p.gate.clone()).unwrap_or_else(||Arc::new(Semaphore::new(1)));
    let realm=scope::realm(&a);
    let moved = current.is_some_and(|p| p.zone.id != zone.id);
    if !is_portable(&zone.id) { players.retain(|key,p| key == &id || p.realm != realm || p.zone.id != zone.id); }
    players.insert(id.clone(),Player{user:a.id,realm:realm.clone(),session:a.session_hash,platform:snapshot.platform,last_seen:Instant::now(),stamp:snapshot.stamp,zone:zone.clone(),gate});
    drop(players);
    if structural { s.loudness.wake.notify_one(); }
    if new_connection||moved{s.emit("endpoints",json!({}));s.emit("devices",json!({}));}
    if structural {s.emit("zones_changed",json!({}));}
    else{s.emit("zone_progress",json!({"id":zone.id,"playback_id":zone.playback_id,"position":zone.position,"status":zone.status,"updated_at":zone.updated_at,"account_realm":realm}));}
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn for_playing(s: &AppState, mut visit: impl FnMut(&Zone)) {
    for player in s.portable.players.lock().await.values() {
        if player.last_seen.elapsed() < LEASE && player.zone.status == "playing" { visit(&player.zone); }
    }
}

pub(crate) async fn publish_loudness(s: &AppState, track: &crate::models::Track, measurement: crate::loudness::Measurement) {
    for player in s.portable.players.lock().await.values_mut().filter(|player| player.last_seen.elapsed()<LEASE) {
        let zone = &mut player.zone;
        let current = zone.track.as_mut().is_some_and(|entry| crate::loudness::update_track(entry, track));
        let mut queued = false;
        for entry in &mut zone.queue { queued |= crate::loudness::update_track(entry, track); }
        if current || queued {
            s.emit("portable_loudness",json!({"session":player.session, "track_id":track.id,
                "playback_id":if current {zone.playback_id.as_deref()} else {None},
                "integrated_lufs":measurement.integrated,"true_peak_dbfs":measurement.true_peak}));
        }
    }
}
pub async fn disconnect(State(s):State<AppState>,a:Auth,Path(id):Path<String>) -> ApiResult<StatusCode> {
    a.require("control")?;
    let mut players=s.portable.players.lock().await;
    if players.get(&id).is_some_and(|p|p.session==a.session_hash){players.remove(&id);drop(players);invalidate(&s);}
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) fn monitor(s:AppState) {
    tokio::spawn(async move {
        let mut interval=tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let expired:Vec<_>=s.portable.players.lock().await.iter().map(|(id,p)|(id.clone(),p.session.clone(),p.last_seen.elapsed()>LEASE)).collect();
            let mut changed=false;
            for (id,session,elapsed) in expired {
                if !elapsed&&s.session_active(session.clone()).await{continue;}
                let mut players=s.portable.players.lock().await;
                if players.get(&id).is_some_and(|p|p.session==session&&(!elapsed||p.last_seen.elapsed()>LEASE)){players.remove(&id);changed=true;}
            }
            if changed {invalidate(&s);}
        }
    });
}
fn invalidate(s:&AppState) {
    s.emit("endpoints",json!({}));s.emit("devices",json!({}));s.emit("zones_changed",json!({}));
}
pub(crate) async fn check(s:&AppState,a:&Auth,id:&str)->ApiResult<()> {
    if s.portable.players.lock().await.get(id).is_some_and(|p|allowed(a,p)){Ok(())}else{Err(ApiError::missing())}
}
pub(crate) async fn zone(s:&AppState,a:&Auth,id:&str)->ApiResult<Zone> {
    with_zone(s, a, id, |zone| Ok(zone.clone())).await
}
pub(crate) async fn with_zone<T>(s:&AppState,a:&Auth,id:&str,read:impl FnOnce(&Zone)->ApiResult<T>)->ApiResult<T> {
    let players = s.portable.players.lock().await;
    let player = players.get(id).filter(|p| allowed(a, p)).ok_or_else(ApiError::missing)?;
    read(&player.zone)
}
pub(crate) async fn zones(s:&AppState,a:&Auth,compact:bool)->Vec<Value> {
    s.portable.players.lock().await.values().filter(|p|allowed(a,p)&&p.last_seen.elapsed()<LEASE)
        .map(|p|if compact{crate::zones::snapshots::state(&p.zone)}else{json!(p.zone)}).collect()
}
pub(crate) async fn endpoints(s:&AppState,a:&Auth)->Vec<Value> {
    s.portable.players.lock().await.values().filter(|p|allowed(a,p)&&p.last_seen.elapsed()<LEASE)
        .map(|p|json!({"id":p.zone.endpoint_id,"name":p.zone.name,
            "kind":if p.platform=="macos"{"coreaudio"}else{"android"},"verified":true,"online":true,
            "capabilities":{"platform":p.platform,"portable_control_v1":true,"native_client_v1":true,"playback_instance":true,"dsp":false}})).collect()
}
pub(crate) async fn devices(s:&AppState,a:&Auth)->Vec<Value> {
    endpoints(s,a).await.into_iter().map(|e|json!({"id":e["id"],"name":e["name"],"platform":e["capabilities"]["platform"],"online":true,"renameable":false,
        "outputs":[{"id":e["id"],"name":"外出播放","kind":e["kind"],"online":true,"endpoint":e,"configurable":false}]})).collect()
}
pub(crate) async fn command(s:&AppState,a:&Auth,id:&str,method:&str,action:&str,body:Value)->ApiResult<Zone> {
    a.require("control")?;
    if !matches!((method,action),("POST","play"|"pause"|"stop"|"next"|"previous"|"seek"|"queue"|"queue/reorder")|
        ("PUT"|"DELETE","queue")|("PATCH","")) {return Err(ApiError::bad("此操作需要在外出播放器本机完成"));}
    if method=="PATCH"&&body.as_object().is_none_or(|v|v.keys().any(|k|!matches!(k.as_str(),"volume"|"muted"|"repeat"|"shuffle"|"autoplay"))){
        return Err(ApiError::bad("只能遥控播放设置"));
    }
    let (session,gate,output)={
        let players=s.portable.players.lock().await;
        let p=players.get(id).filter(|p|allowed(a,p)).ok_or_else(ApiError::missing)?;
        if p.last_seen.elapsed()>=LEASE{return Err(unavailable());}
        (p.session.clone(),p.gate.clone(),p.zone.endpoint_id.clone())
    };
    if !s.session_active(session.clone()).await{return Err(unavailable());}
    let guard=gate.try_acquire_owned().map_err(|_|ApiError(StatusCode::CONFLICT,"portable_busy".into(),"播放器正在处理上一条命令".into()))?;
    let s=s.clone();
    let (id,method,action)=(id.to_owned(),method.to_owned(),action.to_owned());
    tokio::spawn(async move {
        let _guard=guard;
    let request_id=uuid::Uuid::new_v4().to_string();
    let (tx,rx)=oneshot::channel();
    s.portable.pending.lock().await.insert(request_id.clone(),Pending{session:session.clone(),id:id.clone(),response:tx});
    let expires_at=chrono::Utc::now().timestamp_millis()+COMMAND_BUDGET.as_millis() as i64;
    s.emit("portable_command",json!({"session":session,"id":id,"output":output,"request_id":request_id,"expires_at":expires_at,"method":method,"action":action,"body":body}));
    let result=tokio::time::timeout(COMMAND_BUDGET,rx).await;
    s.portable.pending.lock().await.remove(&request_id);
    match result {
        Ok(Ok(Ok(zone)))=>Ok(zone),
        Ok(Ok(Err(message)))=>Err(ApiError::bad(message)),
        _=>Err(unavailable()),
    }
    }).await.map_err(anyhow::Error::from)?
}
pub async fn acknowledge(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(ack):Json<Ack>)->ApiResult<StatusCode> {
    a.require("control")?;
    let mut pending=s.portable.pending.lock().await;
    if !pending.get(&ack.request_id).is_some_and(|p|p.id==id&&p.session==a.session_hash) {return Err(ApiError::missing());}
    let pending=pending.remove(&ack.request_id).unwrap();
    let mut players=s.portable.players.lock().await;
    let Some(player)=players.get_mut(&id).filter(|p|p.session==a.session_hash) else{return Err(ApiError::missing());};
    let result=if let Some(zone)=ack.zone {
        let mut zone=normalize(&id,zone,Some(&player.zone))?;
        if !is_portable(&player.zone.id) {
            zone.id.clone_from(&player.zone.id);
            zone.endpoint_id.clone_from(&player.zone.endpoint_id);
            zone.name.clone_from(&player.zone.name);
        }
        if ack.stamp>=player.stamp{player.zone=zone;player.stamp=ack.stamp;}
        player.last_seen=Instant::now();
        Ok(player.zone.clone())
    }else{Err(ack.error.unwrap_or_else(||"播放器拒绝了这条命令".into()).chars().take(250).collect())};
    drop(players);
    let _=pending.response.send(result);
    s.emit("zones_changed",json!({}));
    Ok(StatusCode::NO_CONTENT)
}
