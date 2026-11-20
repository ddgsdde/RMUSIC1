use crate::{
    cache::{entry_id, CacheEntry, CacheFile, CacheLease, CacheStore},
    profiles::ProfileStore,
};
use anyhow::{Context, Result};
use base64::Engine;
use futures_util::StreamExt;
use rmusic_audio::{
    output::{self, OutputConfig, PlaybackControl, PlaybackRequest},
    DspConfig,
};
use rmusic_bridge::{
    connection::ConnectionProfile,
    media_proxy::{MediaObserver, MediaProxy},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{atomic::{AtomicU64,Ordering}, Arc, Mutex},
};
use tauri::{Emitter, Manager, State};
mod upgrade;
mod lan;
mod requests;
mod account_archives;
mod listen_sync;
mod travel;
mod sequence;
mod preparation;
#[cfg(target_os="android")] mod downloads;
#[cfg(target_os="android")] mod storage;
mod group;
#[cfg(target_os="android")]
mod android;
const GROUP_OWNED:&str="本机输出正由整组控制";
use crate::offline_stats::{Owner as ListenOwner, Recorder, Store as ListenStore};

type CommandResult<T> = std::result::Result<T, String>;
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn source_position(request:&PlaybackRequest,control:&PlaybackControl)->f64 {
    let start=if request.is_dsd(){(request.position*request.sample_rate as f64).round()/request.sample_rate as f64}else{request.position};
    let position=start+control.seconds(request.sample_rate);
    if request.is_dsd(){request.duration.map_or(position,|duration|position.min(duration))}else{position}
}
struct NativePlayer {
    retired: bool,
    track_id: String,
    playback_id: String,
    control: Arc<PlaybackControl>,
    request: PlaybackRequest,
    output: OutputConfig,
    proxy: Option<Arc<MediaProxy>>,
    local_file: Option<CacheFile>,
    thread: Option<std::thread::JoinHandle<()>>,
    cache_id: Option<String>,
    report: Arc<Mutex<Value>>,
    listen: Option<Arc<Mutex<Recorder>>>,
    sequence: Option<sequence::Player>,
}
impl NativePlayer {
    fn position(&self) -> f64 {
        if let Some(sequence)=&self.sequence {return sequence.position();}
        source_position(&self.request,&self.control)
    }
    fn status(&self) -> Value {
        if self.retired{return json!({"status":"stopped","position":0,"paused":true});}
        if let Some(sequence)=&self.sequence {return sequence.status();}
        let mut status=self.report.lock().map(|report|report.clone()).unwrap_or_else(|_|json!({"status":"error","error":"播放状态锁不可用"}));
        status["output_config"]=json!(self.output);
        status["track_id"]=json!(self.track_id);status["playback_id"]=json!(self.playback_id);
        status["position"]=json!(self.position());status["paused"]=json!(self.control.paused.load(Ordering::Relaxed));
        status["frames_played"]=json!(self.control.frames_played.load(Ordering::Relaxed));status["exclusive"]=json!(self.output.exclusive);
        status["offline"]=json!(self.proxy.is_none()&&self.cache_id.is_some());status["cache_id"]=json!(self.cache_id);
        status["duration"]=json!(self.request.duration);status["volume"]=json!(self.control.volume_setting());status["muted"]=json!(self.control.muted.load(Ordering::Relaxed));
        status
    }
    async fn stop(mut self, reason: Option<&str>) {
        self.control.stop();
        if let Some(sequence)=&self.sequence {if let Some(reason)=reason {sequence.finish_as(reason);}sequence.cancel();}
        let thread=self.sequence.as_mut().and_then(|sequence|sequence.take_thread()).or_else(||self.thread.take());
        if let Some(thread) = thread {
            let _ = tokio::task::spawn_blocking(move || thread.join()).await;
        }
        if let (Some(listen),Some(reason))=(&self.listen,reason) {if let Ok(mut listen)=listen.lock(){let _=listen.finish(reason);}}
    }
}
pub struct DesktopState {
    profiles: Mutex<ProfileStore>,
    cache: Arc<Mutex<CacheStore>>,
    #[cfg(target_os="android")] downloads: downloads::Control,
    #[cfg(target_os="android")] storage:storage::Control,
    /// Android sessions also use the Keystore vault; other platforms keep sessions in memory.
    tokens: Mutex<HashMap<String, String>>,
    connections: tokio::sync::Mutex<HashMap<String,(ConnectionProfile,std::time::Instant)>>,
    connection_sync: Mutex<HashMap<String,rmusic_bridge::connection::ConnectionSync>>,
    zone_caches:Mutex<HashMap<String,(String,Arc<rmusic_bridge::zone_cache::ZoneCache>)>>,
    requests: requests::Requests,
    discovery: Option<Arc<rmusic_discovery::Discovery>>,
    player: Mutex<Option<NativePlayer>>,
    travel:travel::Control,
    client_prepared:Mutex<Option<(String,String,OutputConfig,Arc<sequence::Source>)>>,
    metadata_tasks:crate::cache_tail::MetadataTasks,
    playback_controls: Mutex<PlaybackControls>,
    playback_generation: AtomicU64,
    playback_gate: tokio::sync::Mutex<()>,
    recovery_gate: tokio::sync::Mutex<()>,
    recovery_cursor: Mutex<Option<rmusic_bridge::listens::cursor::Handle>>,
    recovery_request: Mutex<Option<(String,Option<u64>,String)>>,
    registrations:Mutex<HashMap<String,rmusic_bridge::sequence::Config>>,
    group_output:crate::group_output::Manager,
    upgrades: upgrade::Control,
    listens: Arc<ListenStore>,
}
#[derive(Default)]
struct PlaybackControls {
    control: Arc<PlaybackControl>,
    position: f64,
    playback_id: String,
    profile_id: Option<String>,
    revisions: [u64;3],
}
impl DesktopState {
    fn remember_listener(&self, profile:&ConnectionProfile, user:&Value)->CommandResult<()> {
        let owner=ListenOwner{core_id:profile.core_id.clone().ok_or("核心未返回身份")?,user_id:user["id"].as_str().ok_or("核心未返回账号身份")?.into()};
        self.listeners_changed(&profile.id,Some(&owner))?;
        self.listens.remember(&profile.id,&owner).map_err(err)?;
        self.listens.changes.send_modify(|revision|*revision=revision.wrapping_add(1));
        #[cfg(target_os="android")] self.storage.wake();
        Ok(())
    }
    fn listeners_changed(&self,profile:&str,next:Option<&ListenOwner>)->CommandResult<()> {
        let old=self.listens.owner(profile).map_err(err)?;
        if old.as_ref()!=next {
            account_archives::discard_profile(profile)?;
            crate::cache_tail::cancel(&self.metadata_tasks);
            self.upgrades.cancel.notify_waiters();self.upgrades.error.lock().map_err(err)?.take();
            self.recovery_cursor.lock().map_err(err)?.take();
            let recovering=self.recovery_request.lock().map_err(err)?.clone().filter(|(_,_,active)| {
                active==profile||self.listens.owner(active).ok().flatten().is_some_and(|owner|
                    next.is_some_and(|next|next.core_id==owner.core_id&&next!=&owner)||(next.is_none()&&old.as_ref().is_some_and(|old|old.core_id==owner.core_id)))
            });
            if let Some((id,_,_))=&recovering {let _=self.requests.cancel(id.clone());}
            let pending=recovering.is_some()||self.playback_controls.lock().map_err(err)?.profile_id.as_deref()==Some(profile);
            let player=self.player.lock().map_err(err)?;
            let sequence=player.as_ref().and_then(|player|player.sequence.as_ref()).filter(|sequence|sequence.core.profile.id==profile||sequence.owner.as_ref().is_some_and(|owner|
                next.is_some_and(|next|next.core_id==owner.core_id&&next!=owner)||(next.is_none()&&old.as_ref().is_some_and(|old|old.core_id==owner.core_id))));
            if pending||sequence.is_some(){self.playback_generation.fetch_add(1,Ordering::AcqRel);}
            if let Some(sequence)=sequence {sequence.cancel();}
        }
        let mut player=self.player.lock().map_err(err)?;
        if let Some(player)=player.as_mut(){
            let owner=player.sequence.as_ref().and_then(|s|s.owner.clone()).or_else(||player.listen.as_ref().and_then(|l|l.lock().ok().map(|l|l.row.owner.clone())));
            let changed=owner.as_ref().is_some_and(|owner|next.is_some_and(|next|next.core_id==owner.core_id&&next!=owner)||(next.is_none()&&old.as_ref().is_some_and(|old|old.core_id==owner.core_id)));
            if changed{if let Some(listen)=&player.listen{listen.lock().map_err(err)?.finish("interrupted").map_err(err)?;}player.control.stop();player.retired=true;}
        }Ok(())
    }
    fn pause(&self, paused: bool) -> CommandResult<()> {
        if self.group_output.owned(){return Ok(());}
        {let mut controls=self.playback_controls.lock().map_err(err)?;controls.control.set_paused(paused);controls.revisions[0]+=1;}
        if let Some(player)=self.player.lock().map_err(err)?.as_ref() {if let Some(listen)=&player.listen {
            // Local positions advance only with heard frames, including the final pause boundary.
            let mut listen=listen.lock().map_err(err)?;listen.sample(player.position(),!paused||player.proxy.is_none(),false).map_err(err)?;listen.checkpoint().map_err(err)?;
        }}
        Ok(())
    }
    fn volume(&self, volume: Option<f64>, muted: Option<bool>) -> CommandResult<()> {
        if self.group_output.owned(){return Ok(());}
        let mut controls=self.playback_controls.lock().map_err(err)?;
        if let Some(volume)=volume {controls.control.set_volume(volume).map_err(|_|"音量必须为 0..1".to_string())?;controls.revisions[1]+=1;}
        if let Some(muted)=muted {controls.control.muted.store(muted,Ordering::Relaxed);controls.revisions[2]+=1;}
        Ok(())
    }
    fn prepare_seek(&self, position:f64, playback_id:Option<String>) -> CommandResult<Option<(NativePlayer,Arc<PlaybackControl>)>> {
        if self.player.lock().map_err(err)?.as_ref().is_some_and(|p|p.retired){return Ok(None);}

        if !position.is_finite() || position<0. {return Err("无效播放位置".into());}
        let playback_id=checked_playback_id(playback_id)?;
        let mut player=self.player.lock().map_err(err)?;
        if player.as_ref().is_some_and(|player|player.sequence.is_some()) {return Err("在线连续播放请通过音区定位".into());}
        if player.is_none() {
            let mut controls=self.playback_controls.lock().map_err(err)?;
            controls.position=position;controls.playback_id=playback_id;
            return Ok(None);
        }
        let control=reset_controls(self,position,None,None,None,Some(playback_id))?;
        self.playback_generation.fetch_add(1,Ordering::AcqRel);
        Ok(player.take().map(|player|(player,control)))
    }
}
fn checked_playback_id(id:Option<String>)->CommandResult<String> {
    let id=id.unwrap_or_else(||uuid::Uuid::new_v4().to_string());
    if id.is_empty()||id.len()>128 {return Err("无效播放实例标识".into());}
    Ok(id)
}
fn reset_controls(state:&DesktopState,position:f64,volume:Option<f64>,muted:Option<bool>,paused:Option<bool>,playback_id:Option<String>)->CommandResult<Arc<PlaybackControl>> {
    if !position.is_finite() || position<0. {return Err("无效播放位置".into());}
    let playback_id=checked_playback_id(playback_id)?;
    let mut previous=state.playback_controls.lock().map_err(err)?;
    let control=Arc::new(PlaybackControl::default());
    control.set_volume(volume.unwrap_or_else(||previous.control.volume_setting())).map_err(|_|"音量必须为 0..1".to_string())?;
    control.muted.store(muted.unwrap_or_else(||previous.control.muted.load(Ordering::Relaxed)),Ordering::Relaxed);
    control.paused.store(paused.unwrap_or_else(||previous.control.paused.load(Ordering::Relaxed)),Ordering::Relaxed);
    #[cfg(windows)] output::power::watch(&control).map_err(err)?;
    *previous=PlaybackControls{control:control.clone(),position,playback_id,profile_id:None,revisions:previous.revisions};
    Ok(control)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeTrack {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    pub recording_id: String,
    pub format: String,
    pub sample_rate: u32,
    pub channels: u16,
    #[serde(default)]
    pub quality_rank: u32,
    pub integrated_lufs: Option<f64>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub true_peak_dbfs: Option<f64>,
    #[serde(default)]
    pub bit_depth: u16,
    #[serde(default)]
    pub bitrate: u32,
    #[serde(default)]
    pub source_etag: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NativeOutput {
    #[serde(flatten)]
    pub limits:output::OutputLimits,
    pub device_id: Option<String>,
    #[serde(default)]
    pub exclusive: bool,
    #[serde(default = "default_buffer")]
    pub buffer_ms: u32,
    #[serde(default)]
    pub dsd_output:output::DsdOutput,
    #[serde(default)]
    pub android_mode:Option<output::AndroidMode>,
}
impl Default for NativeOutput {
    fn default() -> Self {
        Self {
            limits:Default::default(),
            device_id: None,
            exclusive: false,
            buffer_ms: 100,
            dsd_output:output::DsdOutput::Disabled,
            android_mode:None,
        }
    }
}
fn default_buffer() -> u32 {
    100
}
impl From<NativeOutput> for OutputConfig {
    fn from(value: NativeOutput) -> Self {
        Self {
            limits:value.limits,
            device_id: value.device_id,
            exclusive: value.exclusive,
            buffer_ms: value.buffer_ms,
            dsd_output:value.dsd_output,
            android_mode:value.android_mode,
        }.normalized()
    }
}
fn profile(state: &DesktopState, id: &str) -> CommandResult<ConnectionProfile> {
    state.profiles.lock().map_err(err)?.get(id).map_err(err)
}
#[cfg(not(target_os="android"))]
async fn session_addresses(saved:&ConnectionProfile, selected:&ConnectionProfile, session:&str) -> anyhow::Result<ConnectionProfile> {
    match saved.synchronize(selected,session).await {
        Ok(updated) => Ok(updated),
        Err(e) if e.downcast_ref::<reqwest::Error>().is_some_and(|e|
            e.is_connect() || e.is_timeout() || e.status().is_some_and(|s| matches!(s.as_u16(),408|429|502|503|504))
        ) => Ok(saved.clone()),
        Err(e) => Err(e),
    }
}
async fn connected_profile(state: &DesktopState, id: &str) -> CommandResult<ConnectionProfile> {
    resolve_connection(state,id).await.map_err(err)
}
async fn resolve_connection(state:&DesktopState,id:&str)->anyhow::Result<ConnectionProfile> {
    let mut connections=state.connections.lock().await;
    #[cfg(target_os="android")]
    if let Some((_, checked)) = connections.get(id) {
        let ready = state.connection_sync.lock().map_err(|_| anyhow::anyhow!("连接状态不可用"))?
            .get(id).and_then(|sync| sync.selected_route_after(*checked));
        if let Some((selected, at)) = ready.filter(|(route, at)|
            !android::network_changed_since(*at) && route.route_fresh(*at)) {
            connections.insert(id.into(), (selected.clone(), at));
            return Ok(selected);
        }
    }
    if let Some((selected,time))=connections.get(id) {
        #[cfg(target_os="android")]
        let network_matches = !android::network_changed_since(*time);
        #[cfg(not(target_os="android"))]
        let network_matches = true;
        if network_matches && selected.route_fresh(*time) {return Ok(selected.clone());}
    }
    let started=std::time::Instant::now();
    let saved=profile(state,id).map_err(anyhow::Error::msg)?;
    let selected = saved.resolve().await?;
    #[cfg(not(target_os="android"))]
    let selected = if let Ok(session) = token(state, id) {
        let selected = if selected.core_public_key.is_some() {
            let synchronized = session_addresses(&saved, &selected, &session).await?;
            if synchronized != saved {
                let selected = synchronized.resolve().await?;
                state.profiles.lock().map_err(|_| anyhow::anyhow!("连接状态不可用"))?.save(synchronized)?;
                selected
            } else { selected }
        } else { selected };
        state.connection_sync.lock().map_err(|_| anyhow::anyhow!("连接状态不可用"))?
            .entry(id.into()).or_insert_with(|| saved.follow_connection_info(&selected, &session));
        selected
    } else { selected };
    #[cfg(target_os="android")]
    if let Ok(session) = token(state, id) {
        let mut watchers = state.connection_sync.lock().map_err(|_| anyhow::anyhow!("连接状态不可用"))?;
        // Network changes can leave the previous long poll waiting on a dead interface.
        if connections.get(id).is_none_or(|(old, checked)|
            old.address != selected.address || android::network_changed_since(*checked)) {
            watchers.remove(id);
        }
        watchers.entry(id.into()).or_insert_with(|| saved.follow_connection_info(&selected, &session));
    }
    {
        let mut profiles=state.profiles.lock().map_err(|_|anyhow::anyhow!("连接状态不可用"))?;
        let mut saved=profiles.get(id)?;
        if saved.core_id.is_none() {saved.core_id=selected.core_id.clone();saved.core_public_key=selected.core_public_key.clone();profiles.save(saved)?;}
    }
    connections.insert(id.into(),(selected.clone(),started));
    Ok(selected)
}
fn token(state: &DesktopState, id: &str) -> CommandResult<String> {
    #[cfg(target_os="android")] {
        if !state.tokens.lock().map_err(err)?.contains_key(id) {
            let saved=profile(state,id)?;
            let value=android::session("load",id,saved.core_id.as_deref(),None).map_err(err)?;
            if let Some(session)=value["token"].as_str().filter(|s|!s.is_empty()) {state.tokens.lock().map_err(err)?.insert(id.into(),session.into());}
        }
    }
    state
        .tokens
        .lock()
        .map_err(err)?
        .get(id)
        .cloned()
        .ok_or_else(|| "HTTP 401: 请先登录核心".into())
}

#[tauri::command]
fn profiles_list(state: State<'_, DesktopState>) -> CommandResult<Vec<ConnectionProfile>> {
    Ok(state.profiles.lock().map_err(err)?.list())
}
#[tauri::command]
async fn profiles_save(mut profile: ConnectionProfile, state: State<'_, DesktopState>) -> CommandResult<()> {
    state.connection_sync.lock().map_err(err)?.remove(&profile.id);
    state.group_output.stop(Some(&profile.id),None).await;
    let mut connections=state.connections.lock().await;
    connections.remove(&profile.id);
    state.tokens.lock().map_err(err)?.remove(&profile.id);
    let mut profiles=state.profiles.lock().map_err(err)?;
    if let Ok(old)=profiles.get(&profile.id) {profile.core_id=old.core_id;profile.core_public_key=old.core_public_key;}
    profiles.save(profile).map_err(err)
}
#[tauri::command]
fn native_discovery(state: State<'_,DesktopState>) -> Value {
    #[cfg(target_os="android")]
    {let _=state;return android::discovery().unwrap_or_else(|error|json!({"available":false,"items":[],"error":error.to_string()}));}
    #[cfg(not(target_os="android"))]
    json!({"available":state.discovery.is_some(),"items":state.discovery.as_ref().map(|d|d.snapshot()).unwrap_or_default()})
}
#[tauri::command]
async fn native_connection_metrics(request_id:Option<String>, profile_id:String, speed:bool, state:State<'_,DesktopState>)->CommandResult<Value>{
    state.requests.run(request_id,async {
        let profile=connected_profile(&state,&profile_id).await?;
        profile.connection_metrics(&token(&state,&profile_id)?,speed).await.map_err(err)
    }).await
}
#[tauri::command]
async fn native_connection(profile_id:String,state:State<'_,DesktopState>) -> CommandResult<Value> {
    let selected=connected_profile(&state,&profile_id).await?;
    Ok(json!({"address":rmusic_bridge::connection::logical_address(&selected.address),"core_id":selected.core_id,"core_public_key":selected.core_public_key,"transport":rmusic_bridge::connection::remote_status(&selected.address)}))
}
#[tauri::command]
async fn profiles_remove(profile_id: String, state: State<'_, DesktopState>) -> CommandResult<()> {
    state.connection_sync.lock().map_err(err)?.remove(&profile_id);
    #[cfg(target_os="android")] {state.downloads.forget(&profile_id);android::session("forget",&profile_id,None,None).map_err(err)?;}
    state.group_output.stop(Some(&profile_id),None).await;
    state.connections.lock().await.remove(&profile_id);
    state.tokens.lock().map_err(err)?.remove(&profile_id);
    state
        .profiles
        .lock()
        .map_err(err)?
        .remove(&profile_id)
        .map_err(err)
}
#[tauri::command]
async fn native_login(
    profile_id: String,
    username: String,
    password: String,
    totp: Option<String>,
    client: Option<Value>,
    state: State<'_, DesktopState>,
) -> CommandResult<Value> {
    let saved=self::profile(&state,&profile_id)?;
    let started = std::time::Instant::now();
    let profile = saved.resolve().await.map_err(err)?;
    profile.require_encrypted_credentials().map_err(err)?;
    let response = profile
        .http()
        .map_err(err)?
        .post(profile.endpoint("api/v1/auth/login").map_err(err)?)
        .json(&json!({"username":username,"password":password,"totp":totp,"client":client}))
        .send()
        .await
        .map_err(err)?;
    let status = response.status();
    let body: Value = response.json().await.map_err(err)?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {}: {}",
            status.as_u16(),
            body["error"]["message"].as_str().unwrap_or("登录失败")
        ));
    }
    let session = body["token"].as_str().ok_or("核心未返回会话")?.to_owned();
    let mut bound = saved;
    bound.core_id = profile.core_id.clone();
    bound.core_public_key = profile.core_public_key.clone();
    #[cfg(not(target_os="android"))]
    let (bound, profile) = if profile.core_public_key.is_some() {
        let updated = session_addresses(&bound, &profile, &session).await.map_err(err)?;
        let selected = if updated.alternatives != bound.alternatives {
            updated.resolve().await.map_err(err)?
        } else { profile };
        (updated, selected)
    } else { (bound, profile) };
    state.profiles.lock().map_err(err)?.save(bound.clone()).map_err(err)?;
    state.connections.lock().await.insert(profile_id.clone(), (profile.clone(), started));
    state.connection_sync.lock().map_err(err)?.insert(
        profile_id.clone(), bound.follow_connection_info(&profile, &session),
    );
    state.group_output.stop(Some(&profile_id),None).await;
    state.remember_listener(&profile,&body["user"])?;
    #[cfg(target_os="android")] android::session("save",&profile_id,profile.core_id.as_deref(),Some(&session)).map_err(err)?;
    state
        .tokens
        .lock()
        .map_err(err)?
        .insert(profile_id.clone(), session);
    #[cfg(target_os="android")] state.downloads.authorize(&profile_id);
    Ok(body)
}
#[tauri::command]
async fn native_request(
    request_id: Option<String>,
    profile_id: String,
    method: String,
    path: String,
    body: Option<Value>,
    state: State<'_, DesktopState>,
) -> CommandResult<Value> {
    state.requests.run(request_id,core_request(&state,profile_id,method,path,body)).await
}
#[tauri::command]
async fn native_request_cancel(request_id:String,state:State<'_,DesktopState>)->CommandResult<()> {
    cancel_native_request(&state,request_id).await
}
#[tauri::command]
async fn native_impulse_import(request_id:Option<String>,profile_id:String,name:String,data:String,state:State<'_,DesktopState>)->CommandResult<Value> {
    state.requests.run(request_id,import_impulse(&state,profile_id,name,data)).await
}
async fn import_impulse(state:&DesktopState,profile_id:String,name:String,data:String)->CommandResult<Value> {
    let limit=rmusic_audio::impulse::MAX_WAV_BYTES;
    if data.len()>limit.div_ceil(3)*4{return Err("响应文件不能超过 4 MiB".into());}
    let bytes=base64::engine::general_purpose::STANDARD.decode(data).map_err(err)?;
    if bytes.len()>limit{return Err("响应文件不能超过 4 MiB".into());}
    core_api_request(state,profile_id,"POST".into(),"/api/v1/dsp/impulses".into(),None,Some((name,bytes))).await
}
#[cfg(target_os="android")]
#[tauri::command]
fn native_session_available(profile_id:String,state:State<'_,DesktopState>)->bool {
    token(&state,&profile_id).is_ok()
}
#[cfg(target_os="android")]
#[tauri::command]
fn native_select_media_zone(profile_id:String,zone_id:Option<String>,legacy_groups:bool,state:State<'_,DesktopState>) {
    state.travel.select_media(profile_id,zone_id,legacy_groups);
}
async fn core_request(state:&DesktopState,profile_id:String,method:String,path:String,body:Option<Value>)->CommandResult<Value> {
    #[cfg(target_os="android")] {
        let path=path.trim_start_matches('/').to_owned();
        if state.travel.owns(&profile_id)&&(path=="api/v1/zones"||path=="api/v1/endpoints"||path=="api/v1/devices"||path.starts_with("api/v1/zones/client-zone-")){
            return state.travel.request(profile_id,method,path,body.unwrap_or(Value::Null)).await;
        }
    }
    core_api_request(state,profile_id,method,path,body,None).await
}
async fn core_api_request(state:&DesktopState,profile_id:String,method:String,path:String,body:Option<Value>,impulse:Option<(String,Vec<u8>)>)->CommandResult<Value> {
    let path = path.trim_start_matches('/');
    if !path.starts_with("api/v1/")
        || path.contains("..")
        || !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE")
    {
        return Err("无效的核心 API 请求".into());
    }
    if method=="POST"&&path=="api/v1/endpoints/browser" {
        if let Some(endpoint)=body.as_ref().and_then(|body|body["id"].as_str()).and_then(|id|state.group_output.endpoint(&profile_id,id)){return Ok(endpoint);}
    }
    let logout=method=="POST"&&path=="api/v1/auth/logout";
    if logout{state.travel.forget(&profile_id);state.connection_sync.lock().map_err(err)?.remove(&profile_id);state.group_output.stop(Some(&profile_id),None).await;}
    #[cfg(target_os="android")] if logout {state.downloads.forget(&profile_id);android::session("forget",&profile_id,None,None).map_err(err)?;}
    // Clear the local listener before a possibly unreachable core; retain only this request's token.
    let logout_session=if logout {
        state.zone_caches.lock().map_err(err)?.remove(&profile_id);
        state.listeners_changed(&profile_id,None)?;state.listens.forget(&profile_id).map_err(err)?;
        state.tokens.lock().map_err(err)?.remove(&profile_id)
    }else{None};
    let profile = connected_profile(&state, &profile_id).await?;
    let public = (method == "GET" && path == "api/v1/status")
        || (method == "POST" && path == "api/v1/setup");
    let personal = matches!(path, "api/v1/recommendations/daily" | "api/v1/recommendations/similar");
    let timeout = std::time::Duration::from_secs(300);
    let http = if personal { profile.http_with_read_timeout(Some(timeout)) } else { profile.http() }.map_err(err)?;
    let mut request = http.request(method.parse::<reqwest::Method>().map_err(err)?, profile.endpoint(path).map_err(err)?);
    if personal { request = request.timeout(timeout); }
    if let Some(origin) = profile.admin_write_origin(&method, path, body.as_ref()).map_err(err)? {
        request = request.header(reqwest::header::ORIGIN, origin);
    }
    let session=if public {None}else if logout {logout_session}else{Some(token(state,&profile_id)?)};
    if method=="GET" && path=="api/v1/zones" {
        if let Some(session)=&session {
            let cache={
                let mut caches=state.zone_caches.lock().map_err(err)?;
                let cached=caches.entry(profile_id.clone()).or_insert_with(||(session.clone(),Default::default()));
                if cached.0!=*session {*cached=(session.clone(),Default::default());}
                cached.1.clone()
            };
            return cache.load(&profile,&profile.http().map_err(err)?,session).await.map_err(|error| {
                if let Some(status)=error.downcast_ref::<reqwest::Error>().and_then(|e|e.status()) {format!("HTTP {}: 音区读取失败",status.as_u16())}else{err(error)}
            });
        }
    }
    if let Some(session)=&session {request=request.bearer_auth(session);}
    if let Some((name,bytes))=impulse {
        request=request.query(&[("name",name)]).header(reqwest::header::CONTENT_TYPE,"audio/wav").body(bytes);
    } else if let Some(body) = body {
        request = request.json(&body);
    }
    let result=if let Some(session)=session.as_ref().filter(|_|method=="GET"&&rmusic_bridge::connection::latency_sensitive_read(path)) {
        let saved=state.profiles.lock().map_err(err)?.get(&profile_id).map_err(err)?;
        saved.interactive_read(&profile,path,session,request).await
    }else{request.send().await.map(|response|(response,None)).map_err(anyhow::Error::from)};
    let response = match result {
        Ok((response,alternative))=>{
            if let Some(alternative)=alternative {
                let mut connections=state.connections.lock().await;
                if connections.get(&profile_id).is_some_and(|(current,_)|current.address==profile.address) {
                    connections.insert(profile_id.clone(),(alternative,std::time::Instant::now()));
                }
            }
            response
        }
        Err(error) => {
            let mut connections = state.connections.lock().await;
            if connections.get(&profile_id).is_some_and(|(selected, _)| selected.address == profile.address) {
                connections.remove(&profile_id);
            }
            return Err(err(error));
        }
    };
    let status = response.status();
    if status == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    let value: Value = match response.json().await {
        Ok(value)=>value,
        Err(_) if !status.is_success()=>Value::Null,
        Err(error)=>return Err(err(error)),
    };
    if !status.is_success() {
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let cleared={let mut tokens=state.tokens.lock().map_err(err)?;
                if tokens.get(&profile_id)==session.as_ref(){tokens.remove(&profile_id);true}else{false}};
            if cleared {state.connection_sync.lock().map_err(err)?.remove(&profile_id);}
            #[cfg(target_os="android")] if cleared {state.downloads.forget(&profile_id);android::session("forget",&profile_id,None,None).map_err(err)?;}
            #[cfg(not(target_os="android"))] let _=cleared;
            state.group_output.stop(Some(&profile_id),None).await;
        }
        return Err(format!(
            "HTTP {}: {}",
            status.as_u16(),
            value["error"]["message"].as_str().unwrap_or("核心请求失败")
        ));
    }
    if method=="POST"&&path=="api/v1/endpoints/browser" {
        if let (Some(session),Some(endpoint))=(&session,value["id"].as_str()) {
            if state.tokens.lock().map_err(err)?.get(&profile_id)==Some(session) {
                let config=rmusic_bridge::sequence::Config::new(profile.clone(),session.clone(),endpoint.into()).map_err(err)?;
                state.registrations.lock().map_err(err)?.insert(format!("{profile_id}:{endpoint}"),config);
            }
        }
    }
    if method == "POST" && path == "api/v1/setup" {
        let session = value["token"].as_str().ok_or("核心未返回会话")?.to_owned();
        state.remember_listener(&profile,&value["user"])?;
        #[cfg(target_os="android")] android::session("save",&profile_id,profile.core_id.as_deref(),Some(&session)).map_err(err)?;
        state
            .tokens
            .lock()
            .map_err(err)?
            .insert(profile_id, session);
    } else if method == "GET" && path == "api/v1/auth/me" {
        state.remember_listener(&profile,&value)?;
        #[cfg(target_os="android")] state.downloads.authorize(&profile_id);
    }
    Ok(value)
}
#[tauri::command]
async fn native_cover(
    request_id: Option<String>,
    profile_id: String,
    path: String,
    state: State<'_, DesktopState>,
) -> CommandResult<String> {
    state.requests.run(request_id, core_cover(&state, &profile_id, &path)).await
}
async fn core_cover(state: &DesktopState, profile_id: &str, path: &str) -> CommandResult<String> {
    let profile = connected_profile(state, profile_id).await?;
    let base = profile.base_url().map_err(err)?;
    let saved=crate::app::profile(state, profile_id)?;
    let original=saved.base_url().map_err(err)?;
    let prefix = if path.starts_with(&format!("{}/",original.path().trim_end_matches('/'))){original.path().trim_end_matches('/')}else{base.path().trim_end_matches('/')};
    let path = if !prefix.is_empty() && path.starts_with(&format!("{prefix}/")) {
        &path[prefix.len()..]
    } else {
        path
    };
    let path = crate::core_artwork::image_path(path).ok_or("无效的封面地址")?;
    let response = profile
        .http()
        .map_err(err)?
        .get(profile.endpoint(path).map_err(err)?)
        .bearer_auth(token(state, profile_id)?)
        .send()
        .await
        .map_err(err)?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}: 封面请求失败", status.as_u16()));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .to_owned();
    if !matches!(
        content_type.as_str(),
        "image/jpeg" | "image/png" | "image/webp" | "image/gif" | "image/avif"
    ) {
        return Err("不支持的封面格式".into());
    }
    if response.content_length().unwrap_or(0) > 20 * 1024 * 1024 {
        return Err("封面超过 20MiB".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.map_err(err)?);
        if bytes.len() > 20 * 1024 * 1024 {
            return Err("封面超过 20MiB".into());
        }
    }
    Ok(format!(
        "data:{content_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}
#[tauri::command]
fn native_cache_status(profile_id:Option<String>,address:Option<String>,state: State<'_, DesktopState>) -> CommandResult<Value> {
    let cache = state.cache.lock().map_err(err)?;
    let profile_id=if let Some(address)=address{state.profiles.lock().map_err(err)?.list().into_iter().find(|p|p.address.trim_end_matches('/')==address.trim_end_matches('/')).map(|p|p.id)}else{profile_id};
    let entries=cache.manifest.entries.values().filter(|e|profile_id.as_ref().is_some_and(|id|e.profile_id==*id)&&cache_owned(&state,e)).collect::<Vec<_>>();
    let bytes=|entry:&&CacheEntry|cache.entry_bytes(entry);
    Ok(json!({"limit_bytes":cache.manifest.limit_bytes,"used_bytes":entries.iter().filter(|e|!e.manual_download).map(bytes).sum::<u64>(),"download_bytes":entries.iter().filter(|e|e.manual_download).map(bytes).sum::<u64>(),"entries":entries}))
}
#[tauri::command]
fn native_cache_configure(limit_bytes: u64, state: State<'_, DesktopState>) -> CommandResult<()> {
    state
        .cache
        .lock()
        .map_err(err)?
        .set_limit(limit_bytes)
        .map_err(err)
}
#[tauri::command]
fn native_cache_remove(id: String, state: State<'_, DesktopState>) -> CommandResult<()> {
    let mut cache=state.cache.lock().map_err(err)?;if !cache.manifest.entries.get(&id).is_some_and(|e|cache_owned(&state,e)){return Err("此缓存不属于当前账号".into());}cache.remove(&id).map_err(err)
}

async fn media_ticket(profile: &ConnectionProfile, token: &str, track_id: &str) -> Result<String> {
    if track_id.contains('/') || track_id.contains('?') || track_id.contains('#') {
        anyhow::bail!("invalid track ID")
    }
    let response: Value = profile
        .http()?
        .get(profile.endpoint(&format!("api/v1/library/tracks/{track_id}/ticket"))?)
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if let Some(path) = response["api_path"].as_str() {
        return Ok(profile.media_url(path)?.to_string());
    }
    let url = response["url"].as_str().context("missing media URL")?;
    // Core may return an absolute URL or a configured relative URL.
    let absolute = if url.starts_with("http:") || url.starts_with("https:") {
        url.to_owned()
    } else {
        profile.base_url()?.join(url)?.to_string()
    };
    profile.validate_media_url(&absolute)?;
    Ok(absolute)
}
fn new_entry(profile_id: &str, track: &NativeTrack, total: u64, manual: bool) -> CacheEntry {
    let quality = track.quality_rank.max(crate::probe::quality_rank(
        &track.format.to_lowercase(),
        track.sample_rate,
        track.bit_depth,
        track.bitrate,
    ));
    CacheEntry {
        id: entry_id(
            profile_id,
            &track.id,
            &format!(
                "{}:{}",
                track.format,
                track
                    .source_etag
                    .clone()
                    .unwrap_or_else(|| format!("{}:{quality}", track.sample_rate))
            ),
        ),
        profile_id: profile_id.into(),
        owner: None,
        track_id: track.id.clone(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        recording_id: track.recording_id.clone(),
        format: track.format.to_lowercase(),
        total_bytes: total,
        ranges: vec![],
        complete: false,
        pinned: false,
        manual_download: manual,
        last_access: 0,
        quality_rank: quality,
        sha256: None,
        sample_rate: track.sample_rate,
        channels: track.channels,
        bit_depth: track.bit_depth,
        bitrate: track.bitrate,
        verified: false,
        source_etag: track.source_etag.clone(),document_uri:None,document_tree:None,
    }
}
fn owned_entry(profile_id:&str,track:&NativeTrack,total:u64,manual:bool,owner:Option<&ListenOwner>)->CacheEntry {
 let mut entry=new_entry(profile_id,track,total,manual);if let Some(owner)=owner{entry.id=entry_id(&entry.id,&owner.core_id,&owner.user_id);entry.owner=Some(owner.clone());}entry
}
fn cache_owned(state:&DesktopState,entry:&CacheEntry)->bool{entry.owner.as_ref().is_some_and(|owner|state.listens.owner(&entry.profile_id).ok().flatten().as_ref()==Some(owner))}
fn response_metadata(track: &mut NativeTrack, headers: &reqwest::header::HeaderMap) {
    track.source_etag = headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .filter(|v|v.starts_with('"')&&v.ends_with('"'))
        .map(str::to_owned);
    if let Some(format) = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .and_then(|mime| match mime.split(';').next().unwrap_or("") {
            "audio/flac" => Some("flac"),
            "audio/mpeg" => Some("mp3"),
            "audio/mp4" => Some("m4a"),
            "audio/wav" => Some("wav"),
            "audio/aiff" => Some("aiff"),
            "audio/ogg" => Some(if track.format.eq_ignore_ascii_case("opus") {
                "opus"
            } else {
                "ogg"
            }),
            _ => None,
        })
    {
        track.format = format.into();
    }
}

#[tauri::command]
async fn native_export_skin(app:tauri::AppHandle, filename:String, content:String)->CommandResult<String>{
    if !filename.ends_with(".rmusic-skin.json") || filename.len()>80 ||
        !filename.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c,b'.'|b'-'|b'_')) || content.len()>32768 {
        return Err("皮肤文件格式无效".into());
    }
    let value:Value=serde_json::from_str(&content).map_err(err)?;
    if value["kind"]!="rmusic-skin" || value["version"]!=1 {return Err("不支持的皮肤版本".into());}
    #[cfg(target_os="android")] {
        let dir=app.path().app_cache_dir().map_err(err)?;
        tokio::fs::create_dir_all(&dir).await.map_err(err)?;
        let file=dir.join(format!("rmusic-export-{}.json",uuid::Uuid::new_v4()));
        tokio::fs::write(&file,content).await.map_err(err)?;
        let result=android::save_download(&app,&file,&filename,"application/json").await.map_err(err);
        let _=tokio::fs::remove_file(file).await;
        result
    }
    #[cfg(not(target_os="android"))] {let _=app;Err("请从浏览器导出皮肤".into())}
}
#[tauri::command]
async fn native_save_file(profile_id:String,path:String,filename:String,app:tauri::AppHandle,state:State<'_,DesktopState>)->CommandResult<String>{
 let relative=path.trim_start_matches('/');let route=relative.split('?').next().unwrap_or("");let parts=route.split('/').collect::<Vec<_>>();
 let account_export=parts.len()==6&&parts[..4]==["api","v1","auth","external-accounts"]&&parts[5]=="export"&&uuid::Uuid::parse_str(parts[4]).is_ok();
 let allowed=account_export||parts.len()==6&&parts[0]=="api"&&parts[1]=="v1"&&parts[2]=="maintenance"&&parts[3]=="backups"&&parts[5]=="file"||parts.len()==5&&parts[0]=="api"&&parts[1]=="v1"&&matches!(parts[2],"cache"|"downloads")&&parts[4]=="file"||parts.len()==4&&parts[0]=="api"&&parts[1]=="v1"&&parts[2]=="media";
 if !allowed||relative.contains("..")||relative.contains('#')||!account_export&&relative.contains('?'){return Err("无效的音乐文件导出地址".into());}
 let profile=connected_profile(&state,&profile_id).await?;let session=token(&state,&profile_id)?;
 let response=profile.http().map_err(err)?.get(profile.endpoint(relative).map_err(err)?).bearer_auth(session).send().await.map_err(err)?;
 if !response.status().is_success(){return Err(format!("HTTP {}：音乐文件尚未完成或无法读取",response.status().as_u16()));}
 let mime=response.headers().get("content-type").and_then(|v|v.to_str().ok()).unwrap_or("application/octet-stream").to_owned();
 let mut name=filename.chars().map(|c|if c.is_control()||"\\/:*?\"<>|".contains(c){'_'}else{c}).take(160).collect::<String>();
 if name.trim_matches([' ','.']).is_empty(){name="音乐文件".into();}
 let extension=match mime.as_str(){"audio/flac"=>"flac","audio/mpeg"=>"mp3","audio/mp4"=>"m4a","audio/wav"=>"wav","audio/aiff"=>"aiff","audio/ogg"=>"ogg",_=>std::path::Path::new(&filename).extension().and_then(|v|v.to_str()).filter(|v|v.len()<=8&&v.chars().all(|c|c.is_ascii_alphanumeric())).unwrap_or("bin")};
 if !name.to_lowercase().ends_with(&format!(".{extension}")){name.push('.');name.push_str(extension);}
 #[cfg(not(target_os="android"))] let directory=app.path().download_dir().map_err(err)?.join("R-Music");
 #[cfg(target_os="android")] let directory=app.path().app_cache_dir().map_err(err)?;
 tokio::fs::create_dir_all(&directory).await.map_err(err)?;let temporary=directory.join(format!("rmusic-export-{}.partial",uuid::Uuid::new_v4()));
 let mut owned=false;let result=async{use tokio::io::AsyncWriteExt;let expected=response.content_length();let mut stream=response.bytes_stream();let mut file=tokio::fs::OpenOptions::new().write(true).create_new(true).open(&temporary).await.map_err(err)?;owned=true;let mut size=0u64;
  while let Some(bytes)=stream.next().await{let bytes=bytes.map_err(err)?;file.write_all(&bytes).await.map_err(err)?;size+=bytes.len()as u64;}file.sync_all().await.map_err(err)?;drop(file);
  if size==0||expected.is_some_and(|v|v!=size){return Err("音乐文件未完整接收，未发布不完整文件".into());}
  #[cfg(target_os="android")] {android::save_download(&app,&temporary,&name,&mime).await.map_err(err)}
  #[cfg(not(target_os="android"))] {let stem=std::path::Path::new(&name).file_stem().and_then(|v|v.to_str()).unwrap_or("音乐文件");for index in 0..1000{let destination=directory.join(if index==0{name.clone()}else{format!("{stem} ({index}).{extension}")});match tokio::fs::hard_link(&temporary,&destination).await{Ok(())=>return Ok(destination.to_string_lossy().into_owned()),Err(e)if e.kind()==std::io::ErrorKind::AlreadyExists=>continue,Err(e)=>return Err(err(e))}}Err("同名文件过多，请整理下载目录".into())}
 }.await;
 // This command owns exactly this generated temporary file; never recurse or touch existing songs.
 if owned{let _=tokio::fs::remove_file(&temporary).await;}result
}

#[tauri::command]
async fn native_download(profile_id:String,track:NativeTrack,app:tauri::AppHandle,state:State<'_,DesktopState>)->CommandResult<Value> {
    #[cfg(target_os="android")] {return downloads::enqueue(app,&state,profile_id,track);}
    #[cfg(not(target_os="android"))] {let _=app;download_file(profile_id,track,&state).await.and_then(|entry|serde_json::to_value(entry).map_err(err))}
}
#[cfg(not(target_os="android"))]
async fn download_file(profile_id:String,mut track:NativeTrack,state:&DesktopState)->CommandResult<CacheEntry> {
    let profile = connected_profile(&state, &profile_id).await?;
    let token = token(&state, &profile_id)?;
    let url = media_ticket(&profile, &token, &track.id)
        .await
        .map_err(err)?;
    let response = profile
        .http()
        .map_err(err)?
        .get(url)
        .send()
        .await
        .map_err(err)?
        .error_for_status()
        .map_err(err)?;
    let total = response
        .content_length()
        .ok_or("音源未提供完整长度，无法验证离线下载")?;
    response_metadata(&mut track, response.headers());
    let owner=state.listens.owner(&profile_id).map_err(err)?;let entry = owned_entry(&profile_id, &track, total, true,owner.as_ref());
    let id = entry.id.clone();
    {
        let mut cache = state.cache.lock().map_err(err)?;
        cache.ensure_entry(entry).map_err(err)?;
        cache.mark_manual(&id).map_err(err)?;
    }
    let _lease = CacheLease::acquire(state.cache.clone(), id.clone()).map_err(err)?;
    let mut stream = response.bytes_stream();
    let mut offset = 0;
    let result: CommandResult<()> = async {
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(err)?;
            let cache = state.cache.clone();
            let current_id = id.clone();
            let n = bytes.len() as u64;
            tokio::task::spawn_blocking(move || {
                cache
                    .lock()
                    .map_err(err)?
                    .write_range(&current_id, offset, &bytes)
                    .map_err(err)
            })
            .await
            .map_err(err)??;
            offset += n;
        }
        if offset != total {
            return Err("下载中断，保留为部分文件".into());
        }
        Ok(())
    }
    .await;
    result?;
    let cache = state.cache.clone();
    tokio::task::spawn_blocking(move || upgrade::verify(&cache, &id).map_err(err))
        .await
        .map_err(err)?
}

fn start_player(
    app: tauri::AppHandle,
    cache: Arc<Mutex<CacheStore>>,
    track_id: String,
    request: PlaybackRequest,
    config: OutputConfig,
    proxy: Option<Arc<MediaProxy>>,
    cache_id: Option<String>,
    control: Arc<PlaybackControl>,
    playback_id: String,
    listen: Option<Arc<Mutex<Recorder>>>,
    local_file: Option<CacheFile>,
) -> NativePlayer {
    let report=Arc::new(Mutex::new(json!({"status":"preparing"})));
    let worker_report=report.clone();
    let worker_lease = cache_id
        .as_ref()
        .and_then(|id| CacheLease::acquire(cache.clone(), id.clone()).ok());
    let worker_control = control.clone();
    let worker_request = request.clone();
    let worker_config = config.clone();
    let worker_cache_id = cache_id.clone();
    let worker_track_id = track_id.clone();
    let worker_playback_id = playback_id.clone();
    let worker_file=local_file.clone();
    let worker_proxy=proxy.clone();
    let worker_listen=listen.clone();
    let duration=request.duration;
    let thread = std::thread::spawn(move || {
        let _lease = worker_lease;
        let _file = worker_file;
        let clock_request=worker_request.clone();
        let publish=|event:Value| {
            if let Ok(mut report)=worker_report.lock() {*report=event.clone();}
            #[cfg(target_os="android")] app.state::<DesktopState>().travel.report(worker_control.clone(),event.clone());
            // Native queue progression continues while the WebView is suspended.
            #[cfg(not(target_os="android"))]
            let _=app.emit("native-playback",event);
        };
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let sample=|position,reset| {if let Some(listen)=&worker_listen {if let Ok(mut listen)=listen.lock(){if let Err(error)=listen.sample(position,!worker_control.paused.load(Ordering::Relaxed)||worker_proxy.is_none(),reset){let _=app.emit("native-cache-error",error.to_string());}}}};
        sample(source_position(&clock_request,&worker_control),true);
        let result = output::play(
            worker_request,
            worker_config,
            worker_control.clone(),
            |mut path, meter| {
                if let Some(proxy)=worker_proxy.as_ref(){proxy.annotate(&mut path);}
                #[cfg(target_os="android")] let report_interval=if android::foreground(){250}else{1000};
                #[cfg(not(target_os="android"))] let report_interval=250;
                if last.elapsed() > std::time::Duration::from_millis(report_interval) {
                    let position=source_position(&clock_request,&worker_control);
                    let paused=worker_control.paused.load(Ordering::Relaxed);
                    sample(position,false);
                    let event=json!({"status":"playing","track_id":worker_track_id,"playback_id":worker_playback_id,
                        "position":position,"paused":paused,"signal_path":path,"meter":meter});
                    #[cfg(target_os="android")]
                    let event={
                        let mut event=event;
                        if let Some(proxy)=worker_proxy.as_ref() {
                            if !paused&&!worker_control.stopped.load(Ordering::Acquire) {proxy.advance_playback(position);}
                            let rate=worker_control.output_sample_rate.load(Ordering::Acquire);
                            let ahead=worker_control.frames_written.load(Ordering::Acquire)
                                .saturating_sub(worker_control.frames_played.load(Ordering::Acquire));
                            let fraction=proxy.buffered_fraction();
                            let decoded=(rate>0&&ahead>0).then(||position+ahead as f64/rate as f64);
                            let until=decoded.into_iter().chain(proxy.buffered_audio_seconds()).reduce(f64::max);
                            event["buffered_fraction"]=json!(fraction);
                            event["buffered_until"]=json!(until);
                            event["filling_current"]=json!(proxy.filling_current());
                        }
                        event
                    };
                    publish(event);
                    last = std::time::Instant::now();
                }
            },
        );
        // Capture this worker's clock before any cache validation or replacement can run.
        let final_position = source_position(&clock_request,&worker_control);
        sample(final_position,false);
        let result=if let Some(proxy)=&worker_proxy {proxy.finish_playback(result,final_position,duration)}else{result};
        if let Some(listen)=&worker_listen {if let Ok(mut listen)=listen.lock(){let saved=if worker_control.stopped.load(Ordering::Relaxed)||result.is_err(){listen.checkpoint()}else{listen.finish("ended")};if let Err(error)=saved {let _=app.emit("native-cache-error",error.to_string());}}}
        if let Some(id) = worker_cache_id {
            let complete = cache.lock().ok().is_some_and(|cache| {
                cache
                    .manifest
                    .entries
                    .get(&id)
                    .is_some_and(|entry| entry.complete && !entry.verified)
            });
            // Android revalidation invokes the Activity plugin; defer it until a foreground action.
            if complete&&!cfg!(target_os="android") {
                if let Err(error) = upgrade::verify(&cache, &id) {
                    let _ = app.emit("native-cache-error", error.to_string());
                }
            }
        }
        match result {
            Ok(()) => {
                publish(json!({"status":if worker_control.stopped.load(Ordering::Relaxed) {"stopped"} else {"ended"},
                    "track_id":worker_track_id,"playback_id":worker_playback_id,"position":final_position}));
            }
            Err(error) => {
                publish(
                    json!({"status":"error","track_id":worker_track_id,"playback_id":worker_playback_id,
                        "position":final_position,"error":error.to_string(),"device_failed":output::is_device_unavailable(&error),"source_failed":error.is::<rmusic_audio::decoder::DecodeFailure>()||error.is::<output::SourceUnavailable>()||worker_proxy.as_ref().is_some_and(|proxy|proxy.source_failed())}),
                );
            }
        }
    });
    NativePlayer {
        retired: false,
        track_id,
        playback_id,
        control,
        request,
        output: config,
        proxy,
        local_file,
        thread: Some(thread),
        cache_id,
        report,
        listen,
        sequence: None,
    }
}

async fn prepare_source(app:tauri::AppHandle,cache_store:Arc<Mutex<CacheStore>>,profile:ConnectionProfile,token:String,profile_id:String,owner:Option<ListenOwner>,mut track:NativeTrack,output:&OutputConfig,preloading:bool,portable:bool)->CommandResult<sequence::Source> {
    let complete=cache_store.lock().map_err(err)?.manifest.entries.values()
        .filter(|e|e.profile_id==profile_id&&e.owner==owner&&e.track_id==track.id&&e.complete&&(!cfg!(target_os="android")||(1..=8).contains(&e.channels)))
        .max_by_key(|e|e.quality_rank).map(|e|e.id.clone());
    if let Some(id)=complete {
        let store=cache_store.clone();let checked=id.clone();
        let verified=tokio::task::spawn_blocking(move||upgrade::verify(&store,&checked)).await.map_err(err)?.map_err(err)?;
        let lease=Arc::new(CacheLease::acquire(cache_store.clone(),id.clone()).map_err(err)?);
        let path=cache_store.lock().map_err(err)?.complete_file(&id).map_err(err)?;
        let probe=path.clone();let metadata=tokio::task::spawn_blocking(move||crate::probe::probe(&probe)).await.map_err(err)?.map_err(err)?;
        track.title=verified.title;track.artist=verified.artist;
        return Ok(sequence::Source {track,metadata,proxy:Arc::new(MediaProxy::local_file(&path,Box::new((lease.clone(),path.clone()))).map_err(err)?),cache_id:id,_lease:lease,metadata_started:std::sync::atomic::AtomicBool::new(false),
            #[cfg(target_os="android")] quality:android::source_quality(output.device_id.as_deref()).map_err(err)?});
    }
    #[cfg(target_os="android")] let settings=android::source_quality(output.device_id.as_deref()).map_err(err)?;
    #[cfg(target_os="android")] let remote_network=crate::mobile_quality::remote_network(&profile.address,&settings);
    #[cfg(target_os="android")] if portable&&preloading&&remote_network&&settings["metered"]==true {
        return Err("计量网络仅拉取当前播放歌曲".into());
    }
    let url = media_ticket(&profile, &token, &track.id)
        .await
        .map_err(err)?;
    let head = profile
        .http()
        .map_err(err)?
        .head(&url)
        .send()
        .await
        .map_err(err)?;
    if !head.status().is_success(){return Err(format!("HTTP {}: 音源暂时不可用",head.status().as_u16()));}
    response_metadata(&mut track, head.headers());
    let cache = cache_store.clone();
    let cache_id = owned_entry(&profile_id, &track, 1, false,owner.as_ref()).id;
    let lease = Arc::new(CacheLease::acquire(cache_store.clone(), cache_id.clone()).map_err(err)?);
    let observer_id = cache_id.clone();
    let observer_app = app.clone();
    let observer_profile = profile_id.clone();
    let observer_track = track.clone();
    let cacheable=track.source_etag.is_some();
    let cache_failed = std::sync::atomic::AtomicBool::new(false);
    let observer: MediaObserver = Arc::new(move |offset, total, bytes| {
        if !cacheable {return;}
        if cache_failed.load(Ordering::Relaxed) {
            return;
        }
        let Some(total) = total else { return };
        let result: Result<()> = (|| {
            let mut store = cache
                .lock()
                .map_err(|_| anyhow::anyhow!("cache lock poisoned"))?;
            store.ensure_entry(owned_entry(&observer_profile, &observer_track, total, false,owner.as_ref()))?;
            store.write_range(&observer_id, offset, bytes)
        })();
        if let Err(error) = result {
            cache_failed.store(true, Ordering::Relaxed);
            let _ = observer_app.emit("native-cache-error", error.to_string());
        }
    });
    let mut prepared_proxy = MediaProxy::start_entity(&profile, &url, Some(rmusic_bridge::media_proxy::buffered_cache_observer(observer)),track.source_etag.clone())
            .await
            .map_err(err)?;
    let info=profile.http().map_err(err)?.get(profile.endpoint(&format!("api/v1/library/tracks/{}/audio-info",track.id)).map_err(err)?).bearer_auth(&token).timeout(std::time::Duration::from_secs(8)).send().await;
    let info=match info {Ok(response) if response.status().is_success()=>response.json::<Value>().await.ok(),_=>None};
    #[cfg(target_os="android")]
    if let Some(info)=info.as_ref().filter(|_|remote_network) {
        let cap=if preloading&&settings["metered"]==true{2*1024*1024}else{8*1024*1024};
        let seconds=if portable{5}else{30};
        let buffer=rmusic_bridge::media_proxy::NetworkBuffer::for_audio(info,seconds,cap);
        prepared_proxy=prepared_proxy.with_network_buffer(if portable{buffer.with_preroll(5)}else{buffer});
        if portable {
            let directory=cache_store.lock().map_err(err)?.private_directory().to_owned();
            prepared_proxy.fill_after_startup_in(Some(directory));
        }
    }
    #[cfg(target_os="android")]
    if let Some(info)=info.as_ref() {
        if let Some(metadata)=rmusic_audio::probe::verified_pcm(info) {
            // A complete compatible original is better than transmitting another lossy copy.
            let fits=metadata.sample_rate<=output.limits.max_sample_rate.unwrap_or(768000)
                &&metadata.bit_depth<=output.limits.max_bit_depth.unwrap_or(32)
                &&metadata.channels<=output.limits.channels.unwrap_or(2)
                &&metadata.sample_rate as u64<=settings["max_sample_rate"].as_u64().unwrap_or(48000)
                &&metadata.bit_depth as u64<=settings["max_bit_depth"].as_u64().unwrap_or(24);
            let path=if fits&&track.source_etag.is_some() {cache_store.lock().map_err(err)?.complete_file(&cache_id).ok()}else{None};
            if let Some(path)=path {
                let local=Arc::new(MediaProxy::local_file(&path,Box::new((lease.clone(),path.clone()))).map_err(err)?);
                return Ok(sequence::Source{track,metadata,proxy:local,cache_id,_lease:lease,metadata_started:std::sync::atomic::AtomicBool::new(false),quality:settings});
            }
        }
    }
    #[cfg(target_os="android")]
    let converted=if let Some(info)=info.as_ref(){
        let mut transport=settings.clone();transport["remote_lossless"]=json!(remote_network);
        let plan=crate::mobile_quality::conversion(info,&transport,&output.limits).map_err(err)?;
        if let Some((query,metadata))=plan {
            if output.android_mode==Some(output::AndroidMode::UsbBitperfect)&&query.contains("mobile_opus"){return Err("USB 原样输出不能使用 Opus，请调整当前网络的音质选择".into());}
            if output.android_mode==Some(output::AndroidMode::UsbBitperfect)&&(info["channels"].as_u64()!=Some(metadata.channels as u64)||info["sample_rate"].as_u64()!=Some(metadata.sample_rate as u64)||info["bit_depth"].as_u64().is_some_and(|bits|bits>metadata.bit_depth as u64)) {return Err("USB 原样输出不允许改变源采样率、位深或声道，请明确选择系统输出或调整音质".into());}
            prepared_proxy.url=format!("{}{}{}",prepared_proxy.url,if prepared_proxy.url.contains('?'){"&"}else{"?"},query);
            Some(metadata)
        }else{None}
    }else{return Err("Core 尚未确认音源格式，无法应用网络音质设置".into())};
    #[cfg(not(target_os="android"))] let converted:Option<rmusic_audio::probe::AudioMetadata>=None;
    #[cfg(not(target_os="android"))] let _=(output,preloading,portable);
    let proxy=Arc::new(prepared_proxy);
    let verified=converted.or_else(||info.as_ref().and_then(rmusic_audio::probe::verified_pcm));
    let audio=match verified {Some(audio)=>audio,None=>rmusic_audio::probe::probe_async(std::path::Path::new(&proxy.url)).await.map_err(|error|if proxy.source_failed(){format!("MEDIA_SOURCE_UNAVAILABLE: {error}")}else{err(error)})?};
    Ok(sequence::Source {track,metadata:audio,proxy,cache_id,_lease:lease,metadata_started:std::sync::atomic::AtomicBool::new(false),#[cfg(target_os="android")] quality:settings})
}

fn reuse_sequence(state:&DesktopState,generation:u64,profile:&ConnectionProfile,token:&str,context:&sequence::Context,output:&OutputConfig,plan_id:&str,playback_id:&str)->CommandResult<bool> {
    if generation!=state.playback_generation.load(Ordering::Acquire){return Ok(false);}
    let mut player=state.player.lock().map_err(err)?;
    let Some(player)=player.as_mut() else{return Ok(false)};
    let Some(sequence)=player.sequence.as_mut().filter(|sequence|sequence.matches(&profile.id,context,output)&&sequence.core.same_listener(profile,token,&context.endpoint_id)&&sequence.plan_id==plan_id&&sequence.alive()) else{return Ok(false)};
    sequence.alias(playback_id.into())?;sequence.epoch=generation;
    player.playback_id=playback_id.into();
    let mut controls=state.playback_controls.lock().map_err(err)?;controls.playback_id=playback_id.into();controls.position=sequence.position();
    Ok(true)
}

#[tauri::command]
fn native_sequence_cancel(plan_id:String,zone_id:String,state:State<'_,DesktopState>)->CommandResult<()> {
    if let Some(sequence)=state.player.lock().map_err(err)?.as_ref().and_then(|player|player.sequence.as_ref()).filter(|sequence|sequence.plan_id==plan_id&&sequence.context.zone_id==zone_id) {
        if state.playback_generation.load(Ordering::Acquire)==sequence.epoch {state.playback_generation.fetch_add(1,Ordering::AcqRel);}
        sequence.cancel();
    }Ok(())
}
#[tauri::command]
fn native_capabilities()->Value {
    let available=output::native_available();
    json!({"native_audio":available,"continuous_pcm":available,"continuous_recovery":available,"recovery_controls":available,"measured_listens_v2":available,"measured_listens_recovery_v2":available,"native_group_v1":available&&cfg!(windows)})
}

#[tauri::command]
fn native_identity(app:tauri::AppHandle,name:Option<String>)->Value {
    let alias=name.filter(|value|!value.trim().is_empty()&&value.chars().count()<=80);
    #[cfg(target_os="android")]
    let name=android::device_name().unwrap_or_else(|_|"Android 设备".into());
    #[cfg(not(target_os="android"))]
    let name=std::env::var("COMPUTERNAME").or_else(|_|std::env::var("HOSTNAME")).unwrap_or_else(|_|std::env::consts::OS.into());
    let display=alias.as_deref().unwrap_or(&name);
    if let Some(window)=app.get_webview_window("main"){let _=window.set_title(&format!("R-Music · {display}"));}
    json!({"name":name,"platform":std::env::consts::OS})
}

#[tauri::command]
async fn native_sequence_restore(request_id:String,profile_id:String,endpoint_id:String,output:Option<NativeOutput>,app:tauri::AppHandle,state:State<'_,DesktopState>)->CommandResult<Value> {
    state.requests.run(Some(request_id.clone()),sequence::persistent::restore(request_id,profile_id,endpoint_id,output,app,&state)).await
}
#[tauri::command]
async fn native_sequence_restore_cancel(request_id:String,state:State<'_,DesktopState>)->CommandResult<()> {
    cancel_native_request(&state,request_id).await
}
async fn cancel_native_request(state:&DesktopState,request_id:String)->CommandResult<()> {
    let generation=state.recovery_request.lock().map_err(err)?.as_ref().filter(|(id,_,_)|id==&request_id).and_then(|(_,generation,_)|*generation);
    state.requests.cancel(request_id)?;
    if let Some(generation)=generation {
        let _gate=state.playback_gate.lock().await;
        let _=state.playback_generation.compare_exchange(generation,generation+1,Ordering::AcqRel,Ordering::Acquire);
        if let Some(sequence)=state.player.lock().map_err(err)?.as_ref().and_then(|player|player.sequence.as_ref()).filter(|sequence|sequence.epoch==generation){sequence.cancel();}
    }Ok(())
}

#[tauri::command]
async fn native_play(
    request_id:Option<String>,
    playback_id: Option<String>,
    sequence: Option<sequence::Context>,
    profile_id: String,
    track: NativeTrack,
    position: Option<f64>,
    dsp: Option<DspConfig>,
    output: Option<NativeOutput>,
    volume: Option<f64>,
    muted: Option<bool>,
    paused: Option<bool>,
    transitions: Option<rmusic_audio::fade::FadeConfig>,
    android_context:Option<sequence::Context>,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> CommandResult<Value> {
    if state.group_output.owned(){return Ok(json!({"group_managed":true}));}
    let result=state.requests.run(request_id.clone(),async {
    let previous_generation=state.playback_generation.load(Ordering::Acquire);
    let revisions=state.playback_controls.lock().map_err(err)?.revisions;
    let (mut paused,mut volume,mut muted)=(paused,volume,muted);
    if let Some(context)=&sequence {
        // A known in-process format boundary is not a process recovery. In particular,
        // don't re-register the endpoint and recover an already acknowledged EOF.
        let known=state.player.lock().map_err(err)?.as_ref().and_then(|player|player.sequence.as_ref())
            .is_some_and(|player|player.matches(&profile_id,context,&output.clone().unwrap_or_default().into())&&(player.contains(&context.playback_id)||player.prepared_source(&context.playback_id,&track.id,&output.clone().unwrap_or_default().into()).is_some()));
        if !known {
            let restored=sequence::persistent::restore(request_id.unwrap_or_else(||uuid::Uuid::new_v4().to_string()),profile_id.clone(),context.endpoint_id.clone(),output.clone(),app.clone(),&state).await?;
            if !restored.is_null(){return Ok(restored);}
        }
        if !known {
            let selected=connected_profile(&state,&profile_id).await?;
            let core=rmusic_bridge::sequence::Config::new(selected,token(&state,&profile_id)?,context.endpoint_id.clone()).map_err(sequence::command_error)?;
            let latest=core.zone(&context.zone_id).await.map_err(sequence::command_error)?;
            if latest["playback_id"]!=context.playback_id||latest["track"]["id"]!=track.id||latest["endpoint_id"]!=context.endpoint_id {return Ok(json!({"zone":latest}));}
        }
    }
    if previous_generation!=state.playback_generation.load(Ordering::Acquire){return Ok(json!({"superseded":true}));}
    {let controls=state.playback_controls.lock().map_err(err)?;
        if controls.revisions[0]!=revisions[0]{paused=Some(controls.control.paused.load(Ordering::Relaxed));}
        if controls.revisions[1]!=revisions[1]{volume=Some(controls.control.volume_setting());}
        if controls.revisions[2]!=revisions[2]{muted=Some(controls.control.muted.load(Ordering::Relaxed));}
    }
    native_play_inner(playback_id,sequence,profile_id,track,position,dsp,output,volume,muted,paused,transitions,app,&state,None,android_context).await
    }).await;
    if result.as_ref().is_err_and(|error|error==GROUP_OWNED){Ok(json!({"group_managed":true}))}else{result}
}
async fn native_play_inner(
    playback_id: Option<String>,
    sequence: Option<sequence::Context>,
    profile_id: String,
    track: NativeTrack,
    position: Option<f64>,
    dsp: Option<DspConfig>,
    output: Option<NativeOutput>,
    volume: Option<f64>,
    muted: Option<bool>,
    paused: Option<bool>,
    transitions: Option<rmusic_audio::fade::FadeConfig>,
    app: tauri::AppHandle,
    state: &DesktopState,
    admission:Option<sequence::persistent::Admission>,
    android_context:Option<sequence::Context>,
) -> CommandResult<Value> {
    let output:OutputConfig=output.unwrap_or_default().into();
    let portable=cfg!(target_os="android")&&state.travel.owns(&profile_id)&&sequence.is_none()&&android_context.is_none();
    let dsp=dsp.unwrap_or_default();
    let dsp=if portable {dsp.loudness_only()} else {dsp};
    if let Some(selection)=&dsp.convolution{selection.validate().map_err(err)?;}
    let impulse_zone=sequence.as_ref().map(|context|context.zone_id.clone());
    if dsp.enabled&&dsp.convolution.is_some()&&impulse_zone.is_none(){return Err("卷积播放需要在线音区配置".into());}
    let mut position=position;
    let playback_id=checked_playback_id(playback_id)?;
    // /ended assigns a fresh Core instance ID. Preserve the exact prepared source
    // before retiring the old device session; new-instance admission is still negotiated below.
    let boundary_prepared=sequence.as_ref().and_then(|context|state.player.lock().ok().and_then(|player|player.as_ref().and_then(|player|player.sequence.as_ref())
        .filter(|player|player.matches(&profile_id,context,&output))
        .and_then(|player|player.prepared_source(&context.playback_id,&track.id,&output).map(|source|(player.core.clone(),source)))));
    let keep=sequence.as_ref().is_some_and(|context|state.player.lock().ok().and_then(|player|player.as_ref().and_then(|player|player.sequence.as_ref()).map(|player|
        player.matches(&profile_id,context,&output)&&player.contains(&context.playback_id))).unwrap_or(false));
    #[cfg(target_os="android")] let fresh_timing=!keep&&admission.is_none();
    let measured_version=if let Some(admission)=&admission {admission.measured_version}else if keep {state.player.lock().map_err(err)?.as_ref().and_then(|player|player.sequence.as_ref()).map_or(1,|player|player.measured_version)}else{1};
    let (generation,mut control)=if let Some(admission)=admission {
        if admission.generation!=state.playback_generation.load(Ordering::Acquire){return Ok(json!({"superseded":true}));}
        (admission.generation,admission.control)
    }else if keep {
        let generation=state.playback_generation.fetch_add(1,Ordering::AcqRel)+1;
        state.volume(volume,muted)?;state.pause(paused.unwrap_or(false))?;
        (generation,state.playback_controls.lock().map_err(err)?.control.clone())
    }else{begin_playback_change(&state,position.unwrap_or(0.),volume,muted,Some(paused.unwrap_or(false)),Some(playback_id.clone())).await?};
    if generation==state.playback_generation.load(Ordering::Acquire){state.playback_controls.lock().map_err(err)?.profile_id=Some(profile_id.clone());}
    let cached=if portable {let owner=state.listens.owner(&profile_id).map_err(err)?;state.cache.lock().map_err(err)?.manifest.entries.values().any(|e|e.profile_id==profile_id&&e.owner==owner&&e.track_id==track.id&&e.complete&&(1..=8).contains(&e.channels))}else{false};
    let profile = if cached {profile(state,&profile_id)?}else{connected_profile(&state, &profile_id).await?};
    let token = token(&state, &profile_id)?;
    let mut prepared=None;
    let local_prepared=if sequence.is_none()&&android_context.is_none(){
        state.client_prepared.lock().map_err(err)?.take().filter(|(id,session,selected,source)|id==&profile_id&&session==&token&&selected==&output&&source.track.id==track.id).map(|(_,_,_,source)|source)
    }else{None};
    let mut boundary_source=boundary_prepared.filter(|(core,_)|sequence.as_ref().is_some_and(|context|core.same_connection(&profile,&token,&context.endpoint_id))).map(|(_,source)|source).or(local_prepared);
    if let Some(context)=sequence {
        let core=rmusic_bridge::sequence::Config::new(profile.clone(),token.clone(),context.endpoint_id.clone()).map_err(err)?;
        #[cfg(target_os="android")] let measured_version=if fresh_timing {
            let status=android::validate_core(&core).await.map_err(err)?;
            if ["continuous_recovery","measured_listens","measured_listens_v2","measured_listens_online_v2","measured_listens_socket_v2","measured_listens_recovery_v2"]
                .iter().all(|key|status["capabilities"][*key]==true) {2}else{measured_version}
        }else{measured_version};
        let recovery={
            let _gate=state.playback_gate.lock().await;
            if generation!=state.playback_generation.load(Ordering::Acquire){return Ok(json!({"superseded":true}));}
            let mut player=state.player.lock().map_err(err)?;
            player.as_mut().and_then(|player|player.sequence.as_mut()).filter(|sequence|keep&&!sequence.alive()&&!sequence.acknowledged_end()&&sequence.core.same_listener(&profile,&token,&context.endpoint_id))
                .map(|sequence|{sequence.cancel();(sequence.recovery(),sequence.take_thread())})
        };
        if let Some((recovery,thread))=recovery {
            if let Some(thread)=thread {let _=tokio::task::spawn_blocking(move ||thread.join()).await;}
            let zone=recovery.reconcile(&core).await.map_err(sequence::command_error)?;
            if generation!=state.playback_generation.load(Ordering::Acquire){return Ok(json!({"superseded":true}));}
            if zone["playback_id"]!=context.playback_id||zone["track"]["id"]!=track.id||zone["status"]=="stopped"||zone["endpoint_id"]!=context.endpoint_id {
                return Ok(json!({"zone":zone}));
            }
            position=zone["position"].as_f64();
        }
        match core.negotiate_timed(&context.zone_id,&track.id,&context.playback_id,measured_version).await.map_err(sequence::command_error)? {
            rmusic_bridge::sequence::Preparation::Stale(zone)=>return Ok(json!({"zone":zone})),
            rmusic_bridge::sequence::Preparation::Unsupported=>{},
            rmusic_bridge::sequence::Preparation::Ready(initial)=>{
                let _gate=state.playback_gate.lock().await;
                if !preparation::current(state,generation,&profile_id,&token){return Ok(json!({"superseded":true}));}
                if reuse_sequence(&state,generation,&profile,&token,&context,&output,&initial.plan_id,&playback_id)? {
                    return Ok(json!({"core_managed":true,"adopted":true}));
                }
                if keep {
                    boundary_source=state.player.lock().map_err(err)?.as_ref().and_then(|player|player.sequence.as_ref())
                        .filter(|sequence|sequence.core.same_connection(&profile,&token,&context.endpoint_id))
                        .and_then(|sequence|sequence.prepared_source(&context.playback_id,&track.id,&output));
                    let target=state.player.lock().map_err(err)?.as_ref().and_then(|player|player.sequence.as_ref().filter(|sequence|sequence.current_id().as_deref()==Some(context.playback_id.as_str())).map(|sequence|sequence.position())).unwrap_or(position.unwrap_or(0.));
                    control=reset_controls(&state,target,None,None,None,Some(playback_id.clone()))?;
                    state.playback_controls.lock().map_err(err)?.profile_id=Some(profile_id.clone());
                    let old=state.player.lock().map_err(err)?.take();if let Some(old)=old {old.stop(Some("replaced")).await;}
                }
                prepared=Some((core,context,initial));
            }
        }
    }
    if keep&&prepared.is_none(){let _gate=state.playback_gate.lock().await;if generation==state.playback_generation.load(Ordering::Acquire){let old=state.player.lock().map_err(err)?.take();if let Some(old)=old{old.stop(Some("interrupted")).await;}}return Err("核心已不支持当前连续播放会话".into());}
    let track_id=track.id.clone();
    let queued_loudness=track.integrated_lufs.map(|lufs|(lufs,track.true_peak_dbfs));
    let (source,impulse)=match preparation::run(state,generation,&profile_id,&token,async {
        let (source,impulse)=tokio::try_join!(
            async {if let Some(source)=boundary_source {Ok(source)}else{prepare_source(app.clone(),state.cache.clone(),profile.clone(),token.clone(),profile_id.clone(),state.listens.owner(&profile_id).map_err(err)?,track,&output,false,portable).await.map(Arc::new)}},
            preparation::load_impulse(&profile,&token,impulse_zone.as_deref(),&dsp)
        )?;
        if source.metadata.is_dsd()&&output.dsd_output==output::DsdOutput::Native {profile.require_clock(source.metadata.output_rate_for(output.dsd_output)).await.map_err(err)?;}
        Ok((source,impulse))
    }).await {
        Ok(Some(ready))=>ready,
        Ok(None)=>return Ok(json!({"superseded":true})),
        Err(error)=>{
            if preparation::current(state,generation,&profile_id,&token){if let Some((core,context,_))=&prepared {
                let _=core.post(&context.zone_id,"progress",&json!({"track_id":track_id,"playback_id":context.playback_id,"status":"paused"})).await;
            }}return Err(error);
        }
    };
    let audio=&source.metadata;let proxy=source.proxy.clone();let cache_id=source.cache_id.clone();let track=&source.track;
    let (lufs, peak)=queued_loudness.map_or((track.integrated_lufs,track.true_peak_dbfs),|(lufs,peak)|(Some(lufs),peak.or(track.true_peak_dbfs)));
    let measured_lufs=dsp.peak_limited_measurement(lufs,peak);
    let mut request = PlaybackRequest {flac_samples: None, dsd_pcm: None,
        input: proxy.url.clone(),
        position: position.unwrap_or(0.),
        sample_rate: audio.output_rate_for(output.dsd_output),
        channels: audio.channels,
        source_integer_lossless: audio.source_integer_lossless,
        source_bit_depth: audio.bit_depth,
        dsp,
        impulse,
        measured_lufs,
        volume: control.volume_setting(),
        duration: audio.duration,
        transitions: transitions.unwrap_or_default(),
    };
    let _gate=state.playback_gate.lock().await;
    if !preparation::current(state,generation,&profile_id,&token) {return Ok(json!({"superseded":true}));}
    let playback_id={let controls=state.playback_controls.lock().map_err(err)?;request.position=controls.position;controls.playback_id.clone()};
    #[cfg(target_os="android")] {
        let core=android_context.map(|context|rmusic_bridge::sequence::Config::new(profile.clone(),token.clone(),context.endpoint_id.clone())
            .map(|core|(core,context,track.id.clone()))).transpose().map_err(err)?;
        android::bind(&app,&control,core,track.id.clone(),track.title.clone().unwrap_or_else(||"R-Music".into()),track.artist.clone().unwrap_or_default()).map_err(err)?;
    }
    #[cfg(not(target_os="android"))] let _=android_context;
    let actual_metadata=json!({"duration":audio.duration,"title":track.title,"artist":track.artist});
    let player=if let Some((core,context,initial))=prepared {
        #[cfg(target_os="android")] android::validate_core(&core).await.map_err(err)?;
        let owner=state.listens.owner(&profile_id).map_err(err)?.filter(|owner|core.profile.core_id.as_deref()==Some(owner.core_id.as_str()));
        sequence::start(sequence::Start {app,cache:state.cache.clone(),profile_id,token,core,context,initial,source,request,output,control,playback_id,epoch:generation,listens:state.listens.clone(),owner})?
    }else{
        let listen=if portable {state.listens.owner(&profile_id).map_err(err)?
            .map(|owner|state.travel.listen_for(state.listens.clone(),owner,playback_id.clone(),track,audio.duration.unwrap_or(0.))).transpose()?}else{None};
        start_player(app,state.cache.clone(),track.id.clone(),request,output,Some(proxy),Some(cache_id),control,playback_id,listen,None)
    };
    let managed=player.sequence.is_some();
    *state.player.lock().map_err(err)? = Some(player);
    Ok(json!({"core_managed":managed,"metadata":actual_metadata}))
}
#[tauri::command]
async fn native_offline_play(
    playback_id: Option<String>,
    id: String,
    position: Option<f64>,
    sample_rate: u32,
    channels: u16,
    output: Option<NativeOutput>,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> CommandResult<()> {
    {let cache=state.cache.lock().map_err(err)?;if !cache.manifest.entries.get(&id).is_some_and(|e|cache_owned(&state,e)){return Err("请以原账号登录后使用此离线文件".into());}}
    state.group_output.cancel_playback().await.map_err(err)?;
    let output:OutputConfig=output.unwrap_or_default().into();
    let position=position.unwrap_or(0.);
    if !position.is_finite()||position<0. {return Err("无效播放位置".into());}
    #[cfg(windows)] let system_suspend=tokio::task::spawn_blocking(output::power::held_stamp).await.map_err(err)?.map_err(err)?;
    let (generation,control)=begin_playback_change(&state,position,None,None,Some(false),playback_id).await?;
    let _preflight = CacheLease::acquire(state.cache.clone(), id.clone()).map_err(err)?;
    let verify_cache = state.cache.clone();
    let verify_id = id.clone();
    let verified = tokio::task::spawn_blocking(move || {
        upgrade::verify(&verify_cache, &verify_id).map_err(err)
    })
    .await
    .map_err(err)??;
    if verified.sample_rate != sample_rate || verified.channels != channels {
        return Err("缓存真实音频格式与请求不符，请刷新缓存列表".into());
    }
    let path = {
        let mut cache = state.cache.lock().map_err(err)?;
        let path = cache.complete_file(&id).map_err(err)?;
        path
    };
    let probe_path = path.clone();
    let audio = tokio::task::spawn_blocking(move || crate::probe::probe(&probe_path))
        .await
        .map_err(err)?
        .map_err(err)?;
    if audio.duration.is_some_and(|duration|position>duration){return Err("播放位置超出缓存音频时长".into());}
    let mut request = PlaybackRequest {flac_samples: None, dsd_pcm: None,
        input: path.to_string_lossy().into(),
        position: 0.,
        sample_rate:audio.output_rate_for(output.dsd_output),
        channels,
        source_integer_lossless: audio.source_integer_lossless,
        source_bit_depth: audio.bit_depth,
        dsp: DspConfig::default(),
        impulse:None,
        measured_lufs: None,
        volume: control.volume_setting(),
        duration: audio.duration,
        transitions: Default::default(),
    };
    let _gate=state.playback_gate.lock().await;
    if generation!=state.playback_generation.load(Ordering::Acquire) {return Ok(());}
    let playback_id={let controls=state.playback_controls.lock().map_err(err)?;request.position=controls.position;controls.playback_id.clone()};
    let listener=state.listens.owner(&verified.profile_id).map_err(err)?.filter(|owner|profile(&state,&verified.profile_id).ok().and_then(|p|p.core_id).as_deref()==Some(owner.core_id.as_str()));
    let listen=listener.map(|owner|Arc::new(Mutex::new(Recorder::new(state.listens.clone(),owner,verified.track_id.clone(),verified.title.clone().unwrap_or_else(||verified.track_id.clone()),verified.artist.clone().unwrap_or_default(),audio.duration.unwrap_or(0.)))));
    #[cfg(target_os="android")] android::bind(&app,&control,None,verified.id.clone(),verified.title.clone().unwrap_or_else(||"R-Music".into()),verified.artist.clone().unwrap_or_default()).map_err(err)?;
    // This command is an explicit local play action; validate the cache before releasing its hold.
    #[cfg(windows)] if let Some(stamp)=system_suspend {tokio::task::spawn_blocking(move||output::power::acknowledge(&stamp)).await.map_err(err)?.map_err(err)?;}
    *state.player.lock().map_err(err)? = Some(start_player(
        app,
        state.cache.clone(),
        verified.track_id,
        request,
        output,
        None,
        Some(id),
        control,
        playback_id,
        listen,
        Some(path),
    ));
    Ok(())
}
#[tauri::command]
fn native_pause(paused: bool, state: State<'_, DesktopState>) -> CommandResult<()> {
    state.pause(paused)
}
#[tauri::command]
fn native_volume(
    volume: Option<f64>,
    muted: Option<bool>,
    state: State<'_, DesktopState>,
) -> CommandResult<()> {
    state.volume(volume,muted)
}
#[tauri::command]
fn native_loudness(native_playback_id:String,playback_id:String,measured_lufs:f64,true_peak_dbfs:Option<f64>,state:State<'_,DesktopState>)->CommandResult<()> {
    let player=state.player.lock().map_err(err)?;
    if let Some(player)=player.as_ref().filter(|p|p.playback_id==native_playback_id){player.control.set_loudness(if player.sequence.is_some(){&playback_id}else{"single"},measured_lufs,true_peak_dbfs).map_err(err)?;}Ok(())
}
#[tauri::command]
async fn native_stop(preserve_recovery:Option<bool>,state: State<'_, DesktopState>) -> CommandResult<()> {
    #[cfg(target_os="android")] if preserve_recovery!=Some(true)&&state.travel.owned(){
        state.playback_controls.lock().map_err(err)?.control.stop();state.travel.system("stop".into(),None);return Ok(());
    }
    if preserve_recovery==Some(true)&&state.group_output.owned(){return Ok(());}
    if preserve_recovery!=Some(true){state.group_output.cancel_playback().await.map_err(err)?;}
    if preserve_recovery==Some(true) {
        let _gate=state.playback_gate.lock().await;
        state.playback_generation.fetch_add(1,Ordering::AcqRel);
        let old=state.player.lock().map_err(err)?.take();if let Some(old)=old {old.stop(None).await;}
        return Ok(());
    }
    begin_playback_change(&state,0.,None,None,Some(true),None).await?;
    state.travel.finish_listen("stopped");
    Ok(())
}
async fn begin_playback_change(state:&DesktopState,position:f64,volume:Option<f64>,muted:Option<bool>,paused:Option<bool>,playback_id:Option<String>)->CommandResult<(u64,Arc<PlaybackControl>)> {
    crate::cache_tail::cancel(&state.metadata_tasks);
    let _gate=state.playback_gate.lock().await;
    if state.group_output.owned(){return Err(GROUP_OWNED.into());}
    let control=reset_controls(state,position,volume,muted,paused,playback_id)?;
    let generation=state.playback_generation.fetch_add(1,Ordering::AcqRel)+1;
    if let Some(cursor)=state.recovery_cursor.lock().map_err(err)?.take(){cursor.retire().map_err(err)?;}
    let old = state.player.lock().map_err(err)?.take();
    if let Some(old) = old {
        let same_owner=old.sequence.as_ref().is_none_or(|sequence|sequence.owner==state.listens.owner(&sequence.core.profile.id).ok().flatten());
        let same_instance=state.playback_controls.lock().map_err(err)?.playback_id==old.playback_id;
        old.stop(if same_owner&&!same_instance {Some(if paused==Some(true){"stop"}else{"replaced"})}else{None}).await;
    }
    Ok((generation,control))
}
#[tauri::command]
async fn native_seek(
    playback_id: Option<String>,
    position: f64,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> CommandResult<()> {
    let _gate=state.playback_gate.lock().await;
    if state.group_output.owned(){return Err("整组播放请使用整组定位".into());}
    if let Some((old,control)) = state.prepare_seek(position,playback_id)? {
        crate::cache_tail::cancel(&state.metadata_tasks);
        let mut request = old.request.clone();
        request.position = position;
        request.volume = control.volume_setting();
        let config = old.output.clone();
        let proxy = old.proxy.clone();
        let local_file = old.local_file.clone();
        let cache_id = old.cache_id.clone();
        let track_id = old.track_id.clone();
        let listen=old.listen.clone();
        let _preflight = cache_id
            .as_ref()
            .map(|id| CacheLease::acquire(state.cache.clone(), id.clone()))
            .transpose()
            .map_err(err)?;
        old.stop(None).await;
        #[cfg(target_os="android")] if proxy.is_none() {
            let entry=cache_id.as_ref().and_then(|id|state.cache.lock().ok()?.manifest.entries.get(id).cloned());
            if let Some(entry)=entry {android::bind(&app,&control,None,entry.track_id.clone(),entry.title.unwrap_or_else(||"R-Music".into()),entry.artist.unwrap_or_default()).map_err(err)?;}
        }
        let player = start_player(
            app,
            state.cache.clone(),
            track_id,
            request,
            config,
            proxy,
            cache_id,
            control,
            state.playback_controls.lock().map_err(err)?.playback_id.clone(),
            listen,
            local_file,
        );
        *state.player.lock().map_err(err)? = Some(player);
    }
    Ok(())
}
#[tauri::command]
fn native_status(state: State<'_, DesktopState>) -> CommandResult<Value> {
    if let Some(status)=state.group_output.status(){return Ok(status);}
    Ok(match state.player.lock().map_err(err)?.as_ref() {
        Some(player) => player.status(),
        None => {
            let controls=state.playback_controls.lock().map_err(err)?;
            json!({"status":"stopped","position":controls.position,"volume":controls.control.volume_setting(),"muted":controls.control.muted.load(Ordering::Relaxed),"paused":controls.control.paused.load(Ordering::Relaxed)})
        },
    })
}
#[tauri::command]
async fn native_devices() -> CommandResult<Value> {
    tokio::task::spawn_blocking(|| output::devices().map(|v| json!(v)).map_err(err))
        .await
        .map_err(err)?
}
#[tauri::command]
async fn native_android_quality(values:Option<Value>)->CommandResult<Value> {
    #[cfg(target_os="android")] {return tokio::task::spawn_blocking(move||android::quality(values).map_err(err)).await.map_err(err)?;}
    #[cfg(not(target_os="android"))] {let _=values;Err("此设置只适用于 Android".into())}
}
#[tauri::command]
async fn native_network_policy() -> CommandResult<Value> {
    #[cfg(target_os="android")] {return native_android_quality(None).await;}
    #[cfg(not(target_os="android"))]
    Ok(json!(tokio::task::spawn_blocking(crate::network::policy)
        .await
        .map_err(err)?))
}
#[tauri::command]
fn native_upgrades_status(state: State<'_, DesktopState>) -> CommandResult<Value> {
    Ok(
        json!({"enabled":state.upgrades.enabled.load(Ordering::Relaxed),"active":state.upgrades.active.load(Ordering::Relaxed),"error":state.upgrades.error.lock().map_err(err)?.clone()}),
    )
}
#[tauri::command]
fn native_upgrades_configure(enabled: bool, state: State<'_, DesktopState>) -> CommandResult<()> {
    state.upgrades.configure(enabled).map_err(err)
}
#[tauri::command]
async fn native_upgrade_cached(
    profile_id: String,
    id: String,
    track: NativeTrack,
    app: tauri::AppHandle,
) -> CommandResult<CacheEntry> {
    upgrade::one(&app, profile_id, id, track).await
}

#[cfg(windows)]
pub fn ui_build_info()->Value {
    use sha2::{Digest,Sha256};
    let context:tauri::Context<tauri::Wry>=tauri::generate_context!();
    let assets:Vec<Value>=context.assets().iter().map(|(path,_)|{
        let bytes=context.assets().get(&tauri::utils::assets::AssetKey::from(path.as_ref())).unwrap_or_default();
        json!({"path":path,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))})
    }).collect();
    json!({"assets":assets,"dev_mode":cfg!(dev),"dev_url":context.config().build.dev_url})
}
pub fn run() {
    let mut context=tauri::generate_context!();
    if let Some(root)=std::env::var_os("RMUSIC_CLIENT_DATA_DIR") {
        context.config_mut().app.app_directories_override=Some(tauri::utils::config::AppDirectoriesOverride::Root(root.into()));
    }
    let builder=tauri::Builder::default();
    #[cfg(target_os="android")]
    let builder=builder.plugin(android::init());
    builder
        .setup(|app| {
            let root = app.path().app_local_data_dir()?;
            rmusic_bridge::connection::initialize_connection_cache(&root)?;
            let profiles = ProfileStore::open(&root)?;
            let mut cache = CacheStore::open(root.join("cache"))?;
            #[cfg(target_os="android")] cache.document_storage(Arc::new(storage::Documents));
            #[cfg(target_os="android")] let storage=storage::Control::open(&root,cache.changes.clone());
            #[cfg(target_os="android")] let downloads=downloads::Control::open(&root)?;
            #[cfg(target_os="android")] downloads.bind_legacy_cache(&mut cache)?;
            let listens=ListenStore::open(&root)?;
            #[cfg(windows)] {let saved=listens.clone();let (generation,held)=listens.system_suspend_state()?;output::power::configure(listens.client_id.clone(),generation,held,move|generation,held|saved.save_system_suspend(generation,held))?;}
            app.manage(DesktopState {
                profiles: Mutex::new(profiles),
                cache: Arc::new(Mutex::new(cache)),
                #[cfg(target_os="android")] downloads,
                #[cfg(target_os="android")] storage,
                tokens: Mutex::new(HashMap::new()),
                connections: tokio::sync::Mutex::new(HashMap::new()),
                connection_sync: Default::default(),
                zone_caches:Default::default(),
                requests: Default::default(),travel:Default::default(),client_prepared:Default::default(),
                discovery: {
                    #[cfg(target_os="android")] {None}
                    #[cfg(not(target_os="android"))] {rmusic_discovery::Discovery::new().ok().map(Arc::new)}
                },
                player: Mutex::new(None),
                metadata_tasks:Default::default(),
                playback_controls: Mutex::new(PlaybackControls::default()),
                playback_generation: AtomicU64::new(0),
                playback_gate: tokio::sync::Mutex::new(()),
                recovery_gate: Default::default(),recovery_cursor: Default::default(),recovery_request:Default::default(),
                registrations:Default::default(),group_output:Default::default(),
                upgrades: upgrade::Control::open(&root)?,
                listens,
            });
            #[cfg(target_os="android")] downloads::schedule(app.handle().clone());
            #[cfg(target_os="android")] storage::schedule(app.handle().clone());
            upgrade::schedule(app.handle().clone());
            lan::schedule(app.handle().clone());
            listen_sync::schedule(app.handle().clone());
            #[cfg(target_os="android")] travel::install(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            profiles_list,
            profiles_save,
            profiles_remove,
            native_discovery,
            native_connection,
            native_connection_metrics,
            native_login,
            native_request,
            native_request_cancel,
            #[cfg(target_os="android")] native_select_media_zone,
            #[cfg(target_os="android")] native_session_available,
            native_impulse_import,
            native_cover,
            native_cache_status,
            native_cache_configure,
            native_cache_remove,
            native_download,
            #[cfg(target_os="android")] storage::native_storage_status,
            #[cfg(target_os="android")] storage::native_storage_choose,
            #[cfg(target_os="android")] storage::native_storage_action,
            #[cfg(all(target_os="android",debug_assertions))] storage::native_storage_audit,
            #[cfg(target_os="android")] downloads::native_download_tasks,
            #[cfg(target_os="android")] downloads::native_download_action,
            native_save_file,
            native_export_skin,
            account_archives::native_account_upload_begin,
            account_archives::native_account_upload_chunk,
            account_archives::native_account_upload_finish,
            account_archives::native_account_upload_abort,
            native_play,
            native_sequence_cancel,
            native_sequence_restore,
            native_sequence_restore_cancel,
            native_capabilities,
            native_identity,
            group::native_endpoint_register,
            group::native_endpoint_disconnect,
            native_offline_play,
            native_pause,
            native_volume,
            native_loudness,
            native_stop,
            native_seek,
            native_status,
            native_devices,
            native_network_policy,
            native_android_quality,
            native_upgrades_status,
            native_upgrades_configure,
            native_upgrade_cached
        ])
        .build(context)
        .expect("failed to run R-Music desktop")
        .run(|app,event| {
            #[cfg(target_os="android")]
            if let tauri::RunEvent::ExitRequested{code:None,api,..}=&event {api.prevent_exit();}
            if matches!(event,tauri::RunEvent::Exit) {
            let state=app.state::<DesktopState>();
            tauri::async_runtime::block_on(state.group_output.stop(None,None));
            state.playback_generation.fetch_add(1,Ordering::AcqRel);
            crate::cache_tail::cancel(&state.metadata_tasks);
            if let Ok(mut cursor)=state.recovery_cursor.lock(){if let Some(cursor)=cursor.take(){let _=cursor.retire();}}
            if let Ok(mut player)=state.player.lock() {if let Some(mut player)=player.take(){player.control.stop();if let Some(sequence)=&mut player.sequence{if sequence.owner==state.listens.owner(&sequence.core.profile.id).ok().flatten(){sequence.finish_as("stop");}sequence.cancel();if let Some(thread)=sequence.take_thread(){let _=thread.join();}}if let Some(thread)=player.thread.take(){let _=thread.join();}if let Some(listen)=player.listen {if let Ok(mut listen)=listen.lock(){let _=listen.finish("interrupted");}}}}
            state.travel.finish_listen("interrupted");
            let _=state.listens.drain();
            let registrations=state.registrations.lock().map(|registrations|registrations.values().cloned().collect::<Vec<_>>()).unwrap_or_default();
            tauri::async_runtime::block_on(async {let _=tokio::time::timeout(std::time::Duration::from_secs(1),futures_util::future::join_all(registrations.iter().map(|core|core.disconnect_browser()))).await;});
        }});
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn state(root:&std::path::Path)->Arc<DesktopState> {
        Arc::new(DesktopState {
            profiles:Mutex::new(ProfileStore::open(root).unwrap()),
            cache:Arc::new(Mutex::new(CacheStore::open(root.join("cache")).unwrap())),
            tokens:Default::default(),connections:Default::default(),connection_sync:Default::default(),discovery:None,
            zone_caches:Default::default(),
            requests:Default::default(),travel:Default::default(),client_prepared:Default::default(),
            player:Mutex::new(None),metadata_tasks:Default::default(),playback_controls:Mutex::new(PlaybackControls::default()),
            playback_generation:AtomicU64::new(0),playback_gate:Default::default(),
            recovery_gate:Default::default(),recovery_cursor:Default::default(),recovery_request:Default::default(),
            registrations:Default::default(),group_output:Default::default(),
            upgrades:upgrade::Control::open(root).unwrap(),
            listens:ListenStore::open(root).unwrap(),
        })
    }

    fn http_profile(address:std::net::SocketAddr)->ConnectionProfile {
        ConnectionProfile{core_public_key:None,id:"test".into(),name:"QA".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None}
    }
    async fn request_headers(socket:&mut tokio::net::TcpStream)->String {
        use tokio::io::AsyncReadExt;
        let mut headers=Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(2),async{while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}}).await.unwrap();
        String::from_utf8(headers).unwrap()
    }
    async fn json_response(socket:&mut tokio::net::TcpStream,status:&str,value:Value){
        use tokio::io::AsyncWriteExt;
        let body=value.to_string();socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    }
    #[tokio::test]
    async fn impulse_upload_keeps_binary_bytes_auth_and_reverse_proxy_prefix(){
        use tokio::io::AsyncReadExt;
        let root=tempfile::tempdir().unwrap();let state=state(root.path());let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut profile=http_profile(server.local_addr().unwrap());profile.address.push_str("/external/music");state.connections.lock().await.insert("test".into(),(profile,std::time::Instant::now()));state.tokens.lock().unwrap().insert("test".into(),"qa-session".into());
        let bytes=(0..2*1024*1024+17).map(|i|(i%256) as u8).collect::<Vec<_>>();let expected=bytes.clone();let data=base64::engine::general_purpose::STANDARD.encode(bytes);let pending=state.clone();
        let upload=tokio::spawn(async move{import_impulse(&pending,"test".into(),"房间 A&右.wav".into(),data).await});
        let(mut socket,_)=server.accept().await.unwrap();let headers=request_headers(&mut socket).await;let path=headers.lines().next().unwrap().split_whitespace().nth(1).unwrap();
        assert!(path.starts_with("/external/music/api/v1/dsp/impulses?"));assert_eq!(reqwest::Url::parse(&format!("http://test{path}")).unwrap().query_pairs().find(|(key,_)|key=="name").unwrap().1,"房间 A&右.wav");
        let headers=headers.to_ascii_lowercase();assert!(headers.contains("authorization: bearer qa-session\r\n"));assert!(headers.contains("content-type: audio/wav\r\n"));assert!(headers.contains(&format!("content-length: {}\r\n",expected.len())));
        let mut actual=vec![0;expected.len()];socket.read_exact(&mut actual).await.unwrap();assert_eq!(actual,expected);json_response(&mut socket,"200 OK",json!({"id":"original"})).await;assert_eq!(upload.await.unwrap().unwrap()["id"],"original");
    }
    #[tokio::test]
    async fn cancelled_impulse_upload_closes_header_or_body_waits_without_clearing_a_new_session(){
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        for body_started in [false,true]{
            let root=tempfile::tempdir().unwrap();let state=state(root.path());let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();state.connections.lock().await.insert("test".into(),(http_profile(server.local_addr().unwrap()),std::time::Instant::now()));state.tokens.lock().unwrap().insert("test".into(),"old-session".into());
            let pending=state.clone();let upload=tokio::spawn(async move{pending.requests.run(Some("impulse-cancel".into()),import_impulse(&pending,"test".into(),"x.wav".into(),"AP+A/w==".into())).await});
            let(mut socket,_)=server.accept().await.unwrap();request_headers(&mut socket).await;let mut bytes=[0;4];socket.read_exact(&mut bytes).await.unwrap();assert_eq!(bytes,[0,255,128,255]);
            if body_started{socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nContent-Type: application/json\r\n\r\n{\"id\":\"").await.unwrap();}
            state.tokens.lock().unwrap().insert("test".into(),"new-session".into());state.requests.cancel("impulse-cancel".into()).unwrap();assert_eq!(upload.await.unwrap().unwrap_err(),"请求已取消");
            let closed=tokio::time::timeout(std::time::Duration::from_secs(2),socket.read_u8()).await.unwrap().unwrap_err().kind();assert!(matches!(closed,std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::ConnectionReset),"{closed:?}");assert_eq!(state.tokens.lock().unwrap().get("test").unwrap(),"new-session");
        }
    }
    #[tokio::test]
    async fn impulse_upload_rejects_base64_and_both_encoded_and_decoded_size_overflows_before_connection(){
        let root=tempfile::tempdir().unwrap();let state=state(root.path());let limit=rmusic_audio::impulse::MAX_WAV_BYTES;
        for data in ["invalid%%".into(),"A".repeat(limit.div_ceil(3)*4+4),base64::engine::general_purpose::STANDARD.encode(vec![0;limit+1])]{
            let error=import_impulse(&state,"no-profile".into(),"x.wav".into(),data).await.unwrap_err();assert!(!error.contains("profile"));assert!(state.connections.lock().await.is_empty());
        }
    }
    #[tokio::test]
    async fn cancelling_durable_restore_closes_its_tcp_request_and_keeps_a_newer_generation() {
        use tokio::io::AsyncReadExt;
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let started=Arc::new(tokio::sync::Notify::new());let ready=started.clone();
        let server=tokio::spawn(async move {let(mut socket,_)=listener.accept().await.unwrap();request_headers(&mut socket).await;ready.notify_one();assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(2),socket.read_u8()).await.unwrap().unwrap_err().kind(),std::io::ErrorKind::UnexpectedEof);});
        state.playback_generation.store(17,Ordering::Release);*state.recovery_request.lock().unwrap()=Some(("restore-job".into(),Some(17),"test".into()));
        let task_state=state.clone();let task=tokio::spawn(async move {task_state.requests.run(Some("restore-job".into()),async {reqwest::get(format!("http://{address}/recover")).await.map_err(err)?;Ok(())}).await});
        started.notified().await;cancel_native_request(&state,"restore-job".into()).await.unwrap();
        assert_eq!(task.await.unwrap().unwrap_err(),"请求已取消");server.await.unwrap();assert_eq!(state.playback_generation.load(Ordering::Acquire),18);
        state.playback_generation.store(25,Ordering::Release);cancel_native_request(&state,"restore-job".into()).await.unwrap();assert_eq!(state.playback_generation.load(Ordering::Acquire),25);
    }
    #[tokio::test]
    async fn media_ticket_uses_selected_prefix_and_keeps_legacy_compatibility(){
        for relative in [true,false]{
            let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut profile=http_profile(server.local_addr().unwrap());profile.address.push_str("/external/music");
            let task=tokio::spawn(async move{
                let(mut socket,_)=server.accept().await.unwrap();let headers=request_headers(&mut socket).await;
                assert!(headers.starts_with("GET /external/music/api/v1/library/tracks/qqmusic:abc/ticket "));assert!(headers.contains("Bearer qa-session"));
                let mut ticket=json!({"url":"/external/music/api/v1/media/qqmusic%3Aabc?ticket=t%2B1"});
                if relative {ticket["url"]=json!("/internal/api/v1/media/qqmusic%3Aabc?ticket=t%2B1");ticket["api_path"]=json!("api/v1/media/qqmusic%3Aabc?ticket=t%2B1");}
                json_response(&mut socket,"200 OK",ticket).await;
            });
            assert_eq!(media_ticket(&profile,"qa-session","qqmusic:abc").await.unwrap(),format!("{}/api/v1/media/qqmusic%3Aabc?ticket=t%2B1",profile.address));task.await.unwrap();
        }
    }
    #[tokio::test]
    async fn native_offline_upload_uses_authenticated_selected_core_and_keeps_unacked_rows(){
        use tokio::io::AsyncReadExt;
        let root=tempfile::tempdir().unwrap();let state=state(root.path());let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut profile=http_profile(server.local_addr().unwrap());profile.address.push_str("/external/music");profile.core_id=Some("core-a".into());
        state.connections.lock().await.insert("test".into(),(profile,std::time::Instant::now()));state.tokens.lock().unwrap().insert("test".into(),"qa-session".into());
        let owner=ListenOwner{core_id:"core-a".into(),user_id:"user-a".into()};state.listens.remember("test",&owner).unwrap();
        let mut listen=Recorder::new(state.listens.clone(),owner.clone(),"cached-track".into(),"缓存验收".into(),"QA".into(),120.);listen.finish("ended").unwrap();state.listens.drain().unwrap();
        let snapshot=listen.row.snapshot.clone();let id=snapshot.id.clone();let version=snapshot.version;
        let task=tokio::spawn(async move{
            let(mut socket,_)=server.accept().await.unwrap();let headers=request_headers(&mut socket).await;assert!(headers.starts_with("POST /external/music/api/v1/stats/offline "));assert!(headers.contains("Bearer qa-session"));
            let length=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|value|value.trim().parse::<usize>().unwrap())).unwrap();
            let mut bytes=vec![0;length];socket.read_exact(&mut bytes).await.unwrap();let body:Value=serde_json::from_slice(&bytes).unwrap();assert_eq!(body["core_id"],"core-a");assert_eq!(body["user_id"],"user-a");assert_eq!(body["sessions"][0]["id"],id);
            json_response(&mut socket,"200 OK",json!({"accepted":[{"id":id,"version":version}],"rejected":[],"changed":true})).await;
        });
        let (accepted_owner,result)=listen_sync::upload(&state,"test".into()).await.unwrap().unwrap();task.await.unwrap();assert_eq!(state.listens.pending(&owner).unwrap().len(),1);
        state.listens.acknowledge(&accepted_owner,&result).unwrap();assert!(state.listens.pending(&owner).unwrap().is_empty());assert!(listen_sync::upload(&state,"test".into()).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn measured_upload_negotiates_support_and_never_strips_source_identity_for_old_cores(){
        use tokio::io::AsyncReadExt;
        for supported in [false,true] {
            let root=tempfile::tempdir().unwrap();let state=state(root.path());let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut profile=http_profile(server.local_addr().unwrap());profile.address.push_str("/external/music");profile.core_id=Some("core-a".into());
            state.connections.lock().await.insert("test".into(),(profile,std::time::Instant::now()));state.tokens.lock().unwrap().insert("test".into(),"qa-session".into());
            let owner=ListenOwner {core_id:"core-a".into(),user_id:"user-a".into()};state.listens.remember("test",&owner).unwrap();
            let mut measured=Recorder::new(state.listens.clone(),owner.clone(),"track".into(),"曲目".into(),"QA".into(),120.).with_measured(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),std::time::Instant::now());
            measured.sample_device(0.25,12000,48000).unwrap();measured.finish("ended").unwrap();
            let mut legacy=Recorder::new(state.listens.clone(),owner.clone(),"legacy".into(),"离线".into(),"QA".into(),120.);legacy.finish("ended").unwrap();state.listens.drain().unwrap();
            let task=tokio::spawn(async move {
                let(mut socket,_)=server.accept().await.unwrap();let headers=request_headers(&mut socket).await;assert!(headers.starts_with("GET /external/music/api/v1/status "));assert!(!headers.contains("Bearer "));
                json_response(&mut socket,"200 OK",if supported {json!({"capabilities":{"measured_listens":true}})}else{json!({"version":"old"})}).await;
                let(mut socket,_)=server.accept().await.unwrap();let headers=request_headers(&mut socket).await;assert!(headers.starts_with("POST /external/music/api/v1/stats/offline "));assert!(headers.contains("Bearer qa-session"));
                let length=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|value|value.trim().parse::<usize>().unwrap())).unwrap();
                let mut bytes=vec![0;length];socket.read_exact(&mut bytes).await.unwrap();let body:Value=serde_json::from_slice(&bytes).unwrap();let rows=body["sessions"].as_array().unwrap();
                assert_eq!(rows.len(),if supported {2}else{1});assert_eq!(rows.iter().filter(|s|s.get("measured").is_some()).count(),usize::from(supported));
                if !supported {assert_eq!(rows[0]["track_id"],"legacy");}
                json_response(&mut socket,"200 OK",json!({"accepted":[],"rejected":[],"changed":false})).await;
            });
            listen_sync::upload(&state,"test".into()).await.unwrap().unwrap();task.await.unwrap();assert_eq!(state.listens.pending(&owner).unwrap().len(),2);
        }
    }
    #[tokio::test]
    async fn logout_clears_local_account_even_when_secure_network_submission_is_unavailable(){
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        state.connections.lock().await.insert("test".into(),(http_profile(server.local_addr().unwrap()),std::time::Instant::now()));
        state.tokens.lock().unwrap().insert("test".into(),"old-session".into());
        state.listens.remember("test",&ListenOwner{core_id:"core-a".into(),user_id:"user-a".into()}).unwrap();
        assert!(core_request(&state,"test".into(),"POST".into(),"api/v1/auth/logout".into(),None).await.unwrap_err().contains("加密"));
        assert!(state.listens.owner("test").unwrap().is_none());
        assert!(!state.tokens.lock().unwrap().contains_key("test"));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30),server.accept()).await.is_err());
    }
    #[tokio::test]
    async fn cancelling_core_json_closes_header_or_body_reads_and_allows_the_next_request(){
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        for body_started in [false,true]{
            let root=tempfile::tempdir().unwrap();let state=state(root.path());
            let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            state.connections.lock().await.insert("test".into(),(http_profile(server.local_addr().unwrap()),std::time::Instant::now()));
            state.tokens.lock().unwrap().insert("test".into(),"qa-session".into());
            let pending=state.clone();let task=tokio::spawn(async move{pending.requests.run(Some("cancel-me".into()),core_request(&pending,"test".into(),"GET".into(),"/api/v1/library/tracks".into(),None)).await});
            let (mut socket,_)=server.accept().await.unwrap();assert!(request_headers(&mut socket).await.contains("/api/v1/library/tracks"));
            if body_started {socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nContent-Type: application/json\r\n\r\n{\"items\":[").await.unwrap();}
            state.requests.cancel("cancel-me".into()).unwrap();
            assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(1),task).await.unwrap().unwrap().unwrap_err(),"请求已取消");
            let mut byte=[0];assert!(matches!(tokio::time::timeout(std::time::Duration::from_secs(1),socket.read(&mut byte)).await,Ok(Ok(0)|Err(_))));
            let next=state.clone();let task=tokio::spawn(async move{next.requests.run(Some("next".into()),core_request(&next,"test".into(),"GET".into(),"/api/v1/library/tracks".into(),None)).await});
            let (mut socket,_)=server.accept().await.unwrap();request_headers(&mut socket).await;json_response(&mut socket,"200 OK",json!({"items":[],"total":0})).await;
            assert_eq!(task.await.unwrap().unwrap()["total"],0);
        }
    }
    #[tokio::test]
    async fn cancelling_address_probe_releases_connection_selection_lock(){
        use tokio::io::AsyncReadExt;
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut profile=http_profile(server.local_addr().unwrap());profile.core_id=Some("family".into());
        profile.alternatives=(1..9).map(|index|rmusic_bridge::connection::CoreAddress{from_core: Some(false), address:format!("http://{}/fallback{index}",server.local_addr().unwrap()),skip_certificate_verification: false, certificate_sha256: None}).collect();
        state.profiles.lock().unwrap().save(profile).unwrap();
        let pending=state.clone();let task=tokio::spawn(async move{pending.requests.run(Some("probe".into()),core_request(&pending,"test".into(),"GET".into(),"/api/v1/status".into(),None)).await});
        let mut sockets=Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(2),async{for _ in 0..9{let (mut socket,_)=server.accept().await.unwrap();assert!(request_headers(&mut socket).await.contains("/api/v1/status"));sockets.push(socket);}}).await.unwrap();
        assert!(state.connections.try_lock().is_err());
        state.requests.cancel("probe".into()).unwrap();assert_eq!(task.await.unwrap().unwrap_err(),"请求已取消");
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1),state.connections.lock()).await.is_ok());
        tokio::time::timeout(std::time::Duration::from_secs(1),async{for mut socket in sockets{let mut byte=[0];assert!(matches!(socket.read(&mut byte).await,Ok(0)|Err(_)));}}).await.unwrap();
    }
    #[tokio::test]
    async fn old_unauthorized_reply_cannot_remove_a_new_native_session(){
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        state.connections.lock().await.insert("test".into(),(http_profile(server.local_addr().unwrap()),std::time::Instant::now()));
        state.tokens.lock().unwrap().insert("test".into(),"qa-old-session".into());
        for replaced in [true,false]{
            let pending=state.clone();let task=tokio::spawn(async move{core_request(&pending,"test".into(),"GET".into(),"/api/v1/library/tracks".into(),None).await});
            let (mut socket,_)=server.accept().await.unwrap();request_headers(&mut socket).await;
            if replaced {state.tokens.lock().unwrap().insert("test".into(),"qa-new-session".into());}
            json_response(&mut socket,"401 Unauthorized",json!({"error":{"message":"expired"}})).await;assert!(task.await.unwrap().unwrap_err().contains("HTTP 401"));
            assert_eq!(state.tokens.lock().unwrap().contains_key("test"),replaced);
        }
    }
    #[tokio::test]
    async fn non_json_http_errors_keep_status_and_only_current_401_clears_session(){
        use tokio::io::AsyncWriteExt;
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let server=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        state.connections.lock().await.insert("test".into(),(http_profile(server.local_addr().unwrap()),std::time::Instant::now()));
        state.tokens.lock().unwrap().insert("test".into(),"qa-session".into());
        for status in [503,401]{
            let pending=state.clone();let task=tokio::spawn(async move{core_request(&pending,"test".into(),"GET".into(),"/api/v1/library/tracks".into(),None).await});
            let (mut socket,_)=server.accept().await.unwrap();request_headers(&mut socket).await;
            socket.write_all(format!("HTTP/1.1 {status} Error\r\nContent-Length: 18\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n<html>Error</html>").as_bytes()).await.unwrap();
            assert!(task.await.unwrap().unwrap_err().starts_with(&format!("HTTP {status}:")));
            assert_eq!(state.tokens.lock().unwrap().contains_key("test"),status!=401);
        }
    }

    #[tokio::test]
    async fn controls_received_while_joining_the_old_worker_reach_the_new_player() {
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let mut old=player("old-track",0.,0);
        let old_control=old.control.clone();
        state.playback_controls.lock().unwrap().control=old_control.clone();
        let stopping=Arc::new(tokio::sync::Notify::new());let worker_stopping=stopping.clone();
        let (release,waiting)=std::sync::mpsc::channel();
        old.thread=Some(std::thread::spawn(move||{
            while !old_control.stopped.load(Ordering::Acquire) {std::thread::sleep(std::time::Duration::from_millis(1));}
            worker_stopping.notify_one();waiting.recv().unwrap();
        }));
        *state.player.lock().unwrap()=Some(old);
        let task_state=state.clone();
        let changing=tokio::spawn(async move{begin_playback_change(&task_state,0.,Some(0.8),Some(false),Some(false),Some("replacement".into())).await});
        tokio::time::timeout(std::time::Duration::from_secs(2),stopping.notified()).await.unwrap();
        state.volume(Some(0.2),Some(true)).unwrap();state.pause(true).unwrap();
        release.send(()).unwrap();
        let (_,control)=changing.await.unwrap().unwrap();
        control.initialize_volume(0.8).unwrap();
        assert_eq!(control.volume_setting(),0.2);assert_eq!(control.volume(),0.);
        assert!(control.paused.load(Ordering::Relaxed));
        assert!(Arc::ptr_eq(&control,&state.playback_controls.lock().unwrap().control));
        assert_eq!(state.playback_controls.lock().unwrap().playback_id,"replacement");
    }

    #[tokio::test]
    async fn loading_seek_preserves_generation_and_current_controls() {
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let (generation,control)=begin_playback_change(&state,0.,Some(0.8),Some(false),Some(false),Some("initial".into())).await.unwrap();
        state.volume(Some(0.25),Some(true)).unwrap();state.pause(true).unwrap();
        assert!(state.prepare_seek(42.,Some("loading-seek".into())).unwrap().is_none());
        assert_eq!(state.playback_generation.load(Ordering::Acquire),generation);
        assert_eq!(state.playback_controls.lock().unwrap().position,42.);
        assert_eq!(state.playback_controls.lock().unwrap().playback_id,"loading-seek");
        assert_eq!(control.volume_setting(),0.25);assert!(control.paused.load(Ordering::Relaxed));
        // Starting an offline track retains gain and mute, and explicitly resumes playback.
        let (_,offline)=begin_playback_change(&state,0.,None,None,Some(false),Some("offline".into())).await.unwrap();
        assert_eq!(offline.volume_setting(),0.25);assert_eq!(offline.volume(),0.);
        assert!(!offline.paused.load(Ordering::Relaxed));
        assert_eq!(state.playback_controls.lock().unwrap().playback_id,"offline");
    }

    #[tokio::test]
    async fn invalid_start_does_not_stop_or_replace_the_current_player() {
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let old=player("old-track",0.,0);let control=old.control.clone();
        control.set_volume(0.4).unwrap();control.muted.store(true,Ordering::Relaxed);
        state.playback_controls.lock().unwrap().control=control.clone();
        *state.player.lock().unwrap()=Some(old);
        assert!(begin_playback_change(&state,0.,Some(f64::NAN),Some(false),Some(false),None).await.is_err());
        assert!(begin_playback_change(&state,f64::INFINITY,None,None,None,None).await.is_err());
        assert!(begin_playback_change(&state,0.,None,None,None,Some(String::new())).await.is_err());
        assert!(state.prepare_seek(5.,Some("x".repeat(129))).is_err());
        assert_eq!(state.volume(Some(f64::NAN),Some(false)).unwrap_err(),"音量必须为 0..1");
        assert_eq!(state.playback_generation.load(Ordering::Acquire),0);
        assert!(!control.stopped.load(Ordering::Acquire));assert_eq!(control.volume(),0.);
        assert_eq!(control.volume_setting(),0.4);
        assert!(Arc::ptr_eq(&control,&state.player.lock().unwrap().as_ref().unwrap().control));
    }

    #[tokio::test]
    async fn seek_replacement_keeps_latest_controls_instead_of_the_request_snapshot() {
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        let old=player("old-track",0.,0);let control=old.control.clone();
        state.playback_controls.lock().unwrap().control=control;
        *state.player.lock().unwrap()=Some(old);
        state.volume(Some(0.3),Some(true)).unwrap();state.pause(true).unwrap();
        let (old,replacement)=state.prepare_seek(12.,Some("seek-instance".into())).unwrap().unwrap();
        assert_ne!(old.playback_id,"seek-instance");
        assert_eq!(state.playback_controls.lock().unwrap().playback_id,"seek-instance");
        assert_eq!(old.request.volume,1.);
        assert_eq!(replacement.volume_setting(),0.3);assert_eq!(replacement.volume(),0.);
        assert!(replacement.paused.load(Ordering::Relaxed));
        state.volume(Some(0.1),Some(false)).unwrap();
        assert_eq!(replacement.volume(),0.1);
    }

    #[test]
    fn volume_and_mute_updates_preserve_the_other_control() {
        let root=tempfile::tempdir().unwrap();let state=state(root.path());
        state.volume(Some(0.2),Some(true)).unwrap();
        state.volume(None,Some(false)).unwrap();
        let control=state.playback_controls.lock().unwrap().control.clone();
        assert_eq!(control.volume_setting(),0.2);assert_eq!(control.volume(),0.2);
        state.volume(None,Some(true)).unwrap();state.volume(Some(0.3),None).unwrap();
        assert_eq!(control.volume_setting(),0.3);assert_eq!(control.volume(),0.);
    }

    fn player(track_id: &str, start: f64, frames: u64) -> NativePlayer {
        let control = Arc::new(PlaybackControl::default());
        control.frames_played.store(frames, Ordering::Relaxed);
        control.frames_written.store(frames, Ordering::Relaxed);
        NativePlayer {
        retired: false,
            track_id: track_id.into(),
            playback_id: uuid::Uuid::new_v4().to_string(),
            control,
            request: PlaybackRequest {flac_samples: None, dsd_pcm: None,
                input: String::new(),
                position: start,
                sample_rate: 48_000,
                channels: 2,
                source_integer_lossless: true,
                source_bit_depth: 24,
                dsp: DspConfig::default(),
                impulse:None,
                measured_lufs: None,
                volume: 1.,
                duration: None,
                transitions: Default::default(),
            },
            output: OutputConfig::default(),
            proxy: None,
            local_file: None,
            thread: None,
            cache_id: None,
            report:Arc::new(Mutex::new(json!({"status":"playing"}))),
            listen:None,
            sequence:None,
        }
    }

    #[test]
    fn offline_pause_keeps_frames_confirmed_after_the_pause_command() {
        let root=tempfile::tempdir().unwrap();let state=state(root.path());let mut player=player("offline",0.,12000);
        let owner=ListenOwner{core_id:"core".into(),user_id:"user".into()};
        let mut recorder=Recorder::new(state.listens.clone(),owner,"offline".into(),"offline".into(),"QA".into(),20.);
        recorder.sample(0.25,true,true).unwrap();let listen=Arc::new(Mutex::new(recorder));
        player.listen=Some(listen.clone());player.cache_id=Some("cached".into());let control=player.control.clone();
        state.playback_controls.lock().unwrap().control=control.clone();*state.player.lock().unwrap()=Some(player);
        state.pause(true).unwrap();std::thread::sleep(std::time::Duration::from_millis(2));
        control.frames_written.store(12048,Ordering::Relaxed);control.frames_played.store(12048,Ordering::Release);
        state.pause(false).unwrap();let before=listen.lock().unwrap().row.snapshot.days.values().map(|day|day.seconds).sum::<f64>();
        assert!((before-0.001).abs()<1e-9);
        state.pause(true).unwrap();std::thread::sleep(std::time::Duration::from_millis(2));state.pause(false).unwrap();
        assert_eq!(listen.lock().unwrap().row.snapshot.days.values().map(|day|day.seconds).sum::<f64>(),before);
    }
    #[test]
    fn playback_status_keeps_worker_identity_and_source_clock() {
        let old = player("old-track", 30., 96_000);
        let repeated = player("old-track", 0., 24_000);
        let next = player("new-track", 0., 48_000);
        assert_ne!(old.playback_id, repeated.playback_id);
        assert_eq!(old.status()["track_id"], "old-track");
        assert_eq!(old.status()["position"], 32.);
        assert_eq!(repeated.status()["position"], 0.5);
        assert_eq!(next.status()["track_id"], "new-track");
        assert_eq!(next.status()["position"], 1.);
        assert_eq!(old.status()["playback_id"], old.playback_id);
        old.control.frames_played.store(100_000, Ordering::Relaxed);
        assert_eq!(old.status()["position"], 32.); // Device tail padding is not source audio.
        *old.report.lock().unwrap()=json!({"status":"ended","position":99.});
        assert_eq!(old.status()["status"],"ended");
        assert_eq!(old.status()["position"],32.);
        assert_eq!(repeated.status()["status"],"playing");
        *old.report.lock().unwrap()=json!({"status":"error","error":"device unplugged"});
        assert_eq!(old.status()["error"],"device unplugged");
        assert_eq!(old.status()["playback_id"],old.playback_id);
        let mut offline=player("cached",0.,0);offline.cache_id=Some("complete-file".into());offline.request.duration=Some(30.);offline.control.set_volume(0.3).unwrap();offline.control.muted.store(true,Ordering::Relaxed);
        assert_eq!(offline.status()["offline"],true);assert_eq!(offline.status()["cache_id"],"complete-file");assert_eq!(offline.status()["duration"],30.);assert_eq!(offline.status()["volume"],0.3);assert_eq!(offline.status()["muted"],true);
    }
    #[test]
    fn dsd_single_offline_clock_aligns_seek_and_excludes_final_carrier_padding() {
        let mut offline=player("cached-dsd",0.,0);offline.cache_id=Some("complete-dsf".into());
        offline.request.sample_rate=176400;offline.request.source_bit_depth=1;offline.request.source_integer_lossless=false;
        offline.request.duration=Some(4105./2822400.);
        offline.control.output_sample_rate.store(176400,Ordering::Relaxed);
        offline.request.position=0.6/176400.;
        assert_eq!(offline.position(),1./176400.);
        offline.control.frames_written.store(1,Ordering::Relaxed);offline.control.frames_played.store(1,Ordering::Relaxed);
        assert_eq!(offline.position(),2./176400.);
        offline.control.paused.store(true,Ordering::Relaxed);
        assert_eq!(offline.position(),2./176400.);
        offline.control.frames_written.store(256,Ordering::Relaxed);offline.control.frames_played.store(2048,Ordering::Relaxed);
        assert_eq!(offline.position(),4105./2822400.);
        assert_eq!(offline.status()["position"],4105./2822400.);
        offline.request.position=1.;offline.control.frames_written.store(0,Ordering::Relaxed);
        assert_eq!(offline.position(),4105./2822400.);
    }
    #[test]
    fn switching_listener_on_another_address_finalizes_only_the_matching_core(){
        let root=tempfile::tempdir().unwrap();let state=state(root.path());let owner=ListenOwner{core_id:"core-a".into(),user_id:"user-a".into()};
        state.listens.remember("lan",&owner).unwrap();let mut current=player("cached-track",0.,0);
        let listen=Arc::new(Mutex::new(Recorder::new(state.listens.clone(),owner,"cached-track".into(),"缓存".into(),"QA".into(),120.)));current.listen=Some(listen.clone());*state.player.lock().unwrap()=Some(current);
        state.listeners_changed("remote",Some(&ListenOwner{core_id:"another-core".into(),user_id:"user-b".into()})).unwrap();assert_eq!(listen.lock().unwrap().row.snapshot.reason,"active");
        state.listeners_changed("remote",Some(&ListenOwner{core_id:"core-a".into(),user_id:"user-b".into()})).unwrap();assert_eq!(listen.lock().unwrap().row.snapshot.reason,"interrupted");
        state.listens.drain().unwrap();
    }
}
