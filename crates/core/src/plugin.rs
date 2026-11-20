use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::{AppState, Config},
};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::Mutex,
};
fn background_deferred() -> ApiError {
    ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE,"background_deferred".into(),"前台优先，后台资料补全已延后".into())
}
struct Process {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    dirty:bool,
    last_used:std::time::Instant,
}
struct RpcFlight<'a>{process:&'a mut Process,complete:bool}
impl std::ops::Deref for RpcFlight<'_>{type Target=Process;fn deref(&self)->&Process{self.process}}
impl std::ops::DerefMut for RpcFlight<'_>{fn deref_mut(&mut self)->&mut Process{self.process}}
impl Drop for RpcFlight<'_>{fn drop(&mut self){if !self.complete{self.process.dirty=true;let _=self.process.child.start_kill();}}}
#[derive(Clone)]
pub struct PluginManager {
    processes: Arc<Mutex<HashMap<String, Arc<Mutex<Process>>>>>,
    dir: PathBuf,
    data: PathBuf,
    python: String,
    background_yield: Arc<tokio::sync::Notify>,
    foreground_requests: Arc<std::sync::atomic::AtomicUsize>,
    foreground_last_activity: Arc<std::sync::atomic::AtomicI64>,
    traffic: Arc<crate::traffic::Traffic>,
}
impl PluginManager {
    pub fn new(c: &Config, foreground_requests: Arc<std::sync::atomic::AtomicUsize>, foreground_last_activity: Arc<std::sync::atomic::AtomicI64>, traffic:Arc<crate::traffic::Traffic>) -> Self {
        Self {
            processes: Default::default(),
            dir: c.plugin_dir.clone(),
            data: c.data.join("providers"),
            python: c.python.clone(),
            background_yield: Default::default(),
            foreground_requests,
            foreground_last_activity,
            traffic,
        }
    }
    pub async fn call(&self, id: &str, method: &str, params: Value) -> ApiResult<Value> {
        self.call_inner(id, method, params, false, None).await
    }
    pub async fn call_declared(&self,id:&str,params:Value,manifest:Value)->ApiResult<Value>{
        self.call_inner(id,"extension_invoke",params,false,Some(manifest)).await
    }
    pub async fn background_metadata(&self, id:&str, params: Value) -> ApiResult<Value> {
        self.call_inner(id, "metadata", params, true, None).await
    }
    pub fn yield_background(&self) { self.background_yield.notify_waiters(); }
    pub fn frontend_busy(&self) -> bool {
        self.foreground_requests.load(std::sync::atomic::Ordering::Acquire)>0
            || chrono::Utc::now().timestamp()-self.foreground_last_activity.load(std::sync::atomic::Ordering::Relaxed)<8
    }
    async fn call_inner(&self, id: &str, method: &str, params: Value, background: bool, expected_manifest:Option<Value>) -> ApiResult<Value> {
        let yielded = self.background_yield.notified();
        tokio::pin!(yielded);
        if background { yielded.as_mut().enable(); }
        if background && self.frontend_busy() {
            return Err(background_deferred());
        }
        let runtime=crate::plugin_lifecycle::RUNTIME.get_or_init(|| tokio::sync::RwLock::new(()));
        let _runtime=tokio::select! {
            biased;
            _ = yielded.as_mut(), if background => return Err(background_deferred()),
            guard = runtime.read() => guard,
        };
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(ApiError::bad("插件 ID 无效"));
        }
        // Authorization was checked against this exact descriptor before entering the lifecycle lock.
        if let Some(expected)=expected_manifest {
            let current:Value=serde_json::from_slice(&tokio::fs::read(self.dir.join(id).join("manifest.json")).await.map_err(anyhow::Error::from)?).map_err(anyhow::Error::from)?;
            if current!=expected{return Err(ApiError(axum::http::StatusCode::CONFLICT,"plugin_manifest_changed".into(),"插件已更新，请重新读取扩展声明".into()));}
        }
        let anonymous = background || matches!(method,"public_recommendations"|"metadata"|"comments"|"related_catalog"|"artist_albums"|"album_catalog"|"public_metadata"|"artwork_probe");
        let isolated=(crate::traffic::background()||method=="lyrics")&&!anonymous&&tokio::fs::read(self.dir.join(id).join("manifest.json")).await.ok().and_then(|bytes|serde_json::from_slice::<Value>(&bytes).ok()).is_some_and(|manifest|manifest["capabilities"].as_array().is_some_and(|items|items.iter().any(|v|v==if method=="lyrics"{"isolated_lyrics_v1"}else{"isolated_background_v1"})));
        let key = if background {format!("{id}:metadata")} else if anonymous {format!("{id}:public")} else if isolated {format!("{id}:{}",if method=="lyrics"{"lyrics"}else{"background"})}else{id.to_owned()};
        let _admission=tokio::select! {
            biased;
            _ = yielded.as_mut(), if background => return Err(background_deferred()),
            permit=self.traffic.request(id,&key,method,background||crate::traffic::background())=>permit,
        };
        let prepare=async {
            let previous={self.processes.lock().await.get(&key).cloned()};
            if let Some(previous)=previous{let mut process=previous.lock().await;if process.dirty{let _=process.child.kill().await;drop(process);let mut processes=self.processes.lock().await;if processes.get(&key).is_some_and(|p|Arc::ptr_eq(p,&previous)){processes.remove(&key);}}}
            let mut processes = self.processes.lock().await;
            if !processes.contains_key(&key) {
                let dir = self.dir.join(id);
                let entry = dir.join("provider.py");
                if !entry.is_file() {
                    return Err(ApiError::missing());
                }
                let data = self.data.join(if anonymous {format!(".anonymous-{id}")} else {id.to_owned()});
                tokio::fs::create_dir_all(&data)
                    .await
                    .map_err(anyhow::Error::from)?;
                let entry=tokio::fs::canonicalize(entry).await.map_err(anyhow::Error::from)?;
                let local_python=if cfg!(windows){dir.join(".venv/Scripts/python.exe")}else{dir.join(".venv/bin/python")};
                let python=if local_python.is_file(){tokio::fs::canonicalize(local_python.parent().unwrap()).await.map_err(anyhow::Error::from)?.join(local_python.file_name().unwrap()).to_string_lossy().to_string()}else{self.python.clone()};
                let mut command = tokio::process::Command::new(python);
                command
                    .arg("-u")
                    .arg(entry)
                    .env("PYTHONUTF8","1")
                    .env("PYTHONIOENCODING","utf-8")
                    .env("RMUSIC_PROVIDER_DATA_DIR", &data)
                    .env("RMUSIC_PROVIDER_ANONYMOUS", if anonymous {"1"} else {"0"})
                    .env("RMUSIC_PROVIDER_BACKGROUND",if isolated{"1"}else{"0"})
                    .current_dir(&dir)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::inherit())
                    .kill_on_drop(true);
                #[cfg(windows)] command.creation_flags(0x08000000);
                let mut child=command.spawn()
                    .map_err(|e| {
                        ApiError(
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                            "plugin_start_failed".into(),
                            format!("插件无法启动：{e}"),
                        )
                    })?;
                let input = child.stdin.take().unwrap();
                let output = BufReader::new(child.stdout.take().unwrap());
                processes.insert(
                    key.clone(),
                    Arc::new(Mutex::new(Process {
                        child,
                        input,
                        output,
                        dirty:false,
                        last_used:std::time::Instant::now(),
                    })),
                );
            }
            let process = processes.get(&key).unwrap().clone();
            drop(processes);
            Ok::<_,ApiError>(process)
        };
        let process=tokio::select! {
            biased;
            _ = yielded.as_mut(), if background => return Err(background_deferred()),
            result = prepare => result?,
        };
        let mut p=tokio::select! {
            biased;
            _ = yielded.as_mut(), if background => return Err(background_deferred()),
            guard = process.lock() => guard,
        };
        if background && self.frontend_busy() {
            return Err(background_deferred());
        }
        let mut p=RpcFlight{process:&mut p,complete:false};
        let request_id = uuid::Uuid::new_v4().to_string();
        let limits=self.traffic.policy.read().unwrap().clone();
        let data = serde_json::to_vec(&json!({"id":request_id,"method":method,"params":params,"request_limits":limits}))
            .map_err(anyhow::Error::from)?;
        let seconds=if method=="extension_invoke" {25}else if method=="search" {15}else if matches!(method,"playlist_snapshot"|"playlist_apply"){300+(limits.playlists_ms/1000+1)*102}else{50};
        let request = tokio::time::timeout(std::time::Duration::from_secs(seconds), async {
            p.input.write_all(&data).await?;
            p.input.write_all(b"\n").await?;
            p.input.flush().await?;
            let mut line = String::new();
            let limit=if method=="extension_invoke"{128*1024}else{16*1024*1024};
            let n = (&mut p.output).take(limit+1).read_line(&mut line).await?;
            if n == 0 || line.len() > limit as usize {
                anyhow::bail!("插件意外退出或响应过大");
            }
            let v: Value = serde_json::from_str(&line)?;
            if v["id"].as_str() != Some(&request_id) {
                anyhow::bail!("插件响应 ID 不匹配");
            }
            Ok::<_, anyhow::Error>(v)
        });
        let (result,deferred) = tokio::select! {
            biased;
            _ = yielded.as_mut(), if background => (Err(anyhow::anyhow!("前台优先，后台资料补全已延后")),true),
            result = request => (result.map_err(|_| anyhow::anyhow!("请求超时")),false),
        };
        let v = match result.and_then(|value| value) {
            Ok(v) => v,
            Err(error) => {
                let _ = p.child.kill().await;
                let mut processes=self.processes.lock().await;
                if processes.get(&key).is_some_and(|existing|Arc::ptr_eq(existing,&process)){processes.remove(&key);}
                if deferred {return Err(background_deferred());}
                return Err(ApiError(
                    axum::http::StatusCode::BAD_GATEWAY,
                    "plugin_unavailable".into(),
                    format!("插件请求失败：{error}"),
                ));
            }
        };
        p.complete=true;
        p.last_used=std::time::Instant::now();
        if let Some(e) = v.get("error") {
            if e["code"].as_str().is_some_and(|code|code.contains("rate_limit")){self.traffic.cooldown(id);}
            return Err(ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                e["code"].as_str().unwrap_or("provider_error").into(),
                e["message"].as_str().unwrap_or("音源暂时不可用").into(),
            ));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
    /// Idle workers hold no in-memory source of truth. Persisted login/config is untouched.
    pub async fn release_idle(&self,idle:std::time::Duration)->usize {
        let snapshot=self.processes.lock().await.iter().map(|(key,p)|(key.clone(),p.clone())).collect::<Vec<_>>();let mut released=0;
        for(key,process)in snapshot {let Ok(mut guard)=process.try_lock()else{continue;};if guard.last_used.elapsed()<idle&&!guard.dirty{continue;}
            let mut entries=self.processes.lock().await;
            // A caller may already have acquired this Arc while waiting for the process mutex.
            // Mark dirty before releasing it; that caller will replace it on next invocation.
            if entries.get(&key).is_some_and(|p|Arc::ptr_eq(p,&process))&&Arc::strong_count(&process)==2 {
                guard.dirty=true;entries.remove(&key);drop(entries);let _=guard.child.kill().await;released+=1;
            }
        }released
    }
    pub async fn process_count(&self)->usize {self.processes.lock().await.len()}
    pub async fn stop_all(&self) {
        self.yield_background();
        let processes=std::mem::take(&mut *self.processes.lock().await);
        for (_,process) in processes {let _=process.lock().await.child.kill().await;}
    }
    pub async fn stop(&self, id: &str) {
        self.yield_background();
        for key in [id.to_owned(),format!("{id}:metadata"),format!("{id}:public"),format!("{id}:background"),format!("{id}:lyrics")] {
            let process=self.processes.lock().await.remove(&key);
            if let Some(p) = process {
                let _ = p.lock().await.child.kill().await;
            }
        }
    }
}
pub async fn list(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let policy=crate::accounts::policy(&s,&a).await?;
    let mut items = vec![];
    let mut dirs = tokio::fs::read_dir(&s.config.plugin_dir)
        .await
        .map_err(anyhow::Error::from)?;
    while let Some(d) = dirs.next_entry().await.map_err(anyhow::Error::from)? {
        let path = d.path().join("manifest.json");
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = tokio::fs::read(path).await else {
            continue;
        };
        let Ok(mut manifest) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let id = manifest["id"].as_str().unwrap_or("").to_string();
        if policy.as_ref().is_some_and(|p|!p.providers.contains(&id)){continue;}
        let pid = id.clone();
        let enabled =
            s.db.run(move |c| {
                use rusqlite::OptionalExtension;
                Ok(
                    c.query_row("SELECT enabled FROM plugins WHERE id=?1", [pid], |r| {
                        r.get::<_, bool>(0)
                    })
                    .optional()?
                    .unwrap_or(true),
                )
            })
            .await?;
        if !manifest["capabilities"].is_array(){manifest["capabilities"]=json!([]);}
        manifest["enabled"] = json!(enabled);
        manifest["status"] = json!(if enabled { "installed" } else { "disabled" });
        manifest["verified"] = json!(false);
        let key=id.clone();manifest["account_context"]=s.db.read(move|c|{use rusqlite::OptionalExtension;Ok(c.query_row("SELECT context FROM provider_accounts WHERE provider=?1",[key],|r|r.get::<_,String>(0)).optional()?)}).await?.map_or(Value::Null,Value::String);
        if policy.is_some(){manifest["account_context"]=Value::Null;}
        items.push(manifest);
    }
    Ok(Json(json!({"items":items})))
}
pub async fn check_enabled(s: &AppState, id: &str) -> ApiResult<()> {
    let pid = id.to_string();
    let enabled =
        s.db.read(move |c| {
            use rusqlite::OptionalExtension;
            Ok(
                c.query_row("SELECT enabled FROM plugins WHERE id=?1", [pid], |r| {
                    r.get::<_, bool>(0)
                })
                .optional()?
                .unwrap_or(true),
            )
        })
        .await?;
    if !enabled {
        return Err(ApiError::bad("音源插件已停用"));
    }
    Ok(())
}
#[derive(Deserialize)]
pub struct Toggle {
    enabled: bool,
}
pub async fn toggle(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(v): Json<Toggle>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let pid = id.clone();
    s.db.run(move|c|{crate::provider_session::bump(c,&pid)?;c.execute("INSERT INTO plugins(id,enabled,version) VALUES(?1,?2,'0.1.0') ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled",rusqlite::params![pid,v.enabled])?;Ok(())}).await?;
    if !v.enabled {
        s.plugin.stop(&id).await;
    }
    s.emit("plugins", json!({}));
    Ok(Json(json!({"id":id,"enabled":v.enabled})))
}
#[derive(Deserialize)]
pub struct Search {
    q: String,
}
pub async fn search(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Query(v): Query<Search>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    check_enabled(&s, &id).await?;
    if v.q.len() > 500 {
        return Err(ApiError::bad("搜索词过长"));
    }
    let epoch=crate::provider_session::start(&s,&id).await?;
    let result = s
        .plugin
        .call(&id, "search", json!({"query":v.q,"q":v.q}))
        .await?;
    crate::provider_session::validate(&s,&id,epoch).await?;
    import_mode_epoch(&s, &id, &result,false,Some(epoch)).await?;
    Ok(Json(result))
}
pub async fn qualities(State(s):State<AppState>,a:Auth,Path((id,track_id)):Path<(String,String)>)->ApiResult<Json<Value>>{
    a.require("read")?;check_enabled(&s,&id).await?;
    if !track_id.starts_with(&format!("{id}:"))||track_id.len()>256{return Err(ApiError::bad("歌曲与音源不匹配"));}
    let descriptor=crate::connectors::manifest(&s,&id).await?;
    if !descriptor["capabilities"].as_array().is_some_and(|values|values.iter().any(|v|v=="qualities")){return Err(ApiError::bad("此音源未提供音质查询"));}
    Ok(Json(s.plugin.call(&id,"qualities",json!({"track_id":track_id})).await?))
}
pub(crate) fn validated_tracks(id: &str, v: &Value) -> ApiResult<Vec<crate::models::Track>> {
    let mut tracks: Vec<crate::models::Track> =
        serde_json::from_value(v.get("items").cloned().unwrap_or(json!([])))
            .map_err(|_| provider_invalid("插件返回曲目格式错误"))?;
    let prefix = format!("{id}:");
    if tracks.len() > 10000 || tracks.iter().any(|t| {
        let suffix = t.id.strip_prefix(&prefix).unwrap_or("");
        t.source != id || suffix.is_empty() || suffix.len() > 256
            || suffix.chars().any(|c| c.is_whitespace() || c.is_control())
            || !t.duration.is_finite() || t.duration < 0.
    }) {
        return Err(provider_invalid("插件曲目 ID 或 source 不属于当前音源"));
    }
    for track in &mut tracks {
        if let Some(data)=track.provider_data.as_object_mut(){data.remove("source_audio");data.remove("artwork_album");}
    }
    Ok(tracks)
}
fn provider_invalid(message: &str) -> ApiError {
    ApiError(axum::http::StatusCode::BAD_GATEWAY, "invalid_provider_response".into(), message.into())
}
pub(crate) async fn import(s: &AppState, id: &str, v: &Value) -> ApiResult<()> { import_mode(s,id,v,false).await }
pub(crate) async fn import_catalog(s: &AppState, id: &str, v: &Value) -> ApiResult<()> { import_mode(s,id,v,true).await }
async fn import_mode(s: &AppState, id: &str, v: &Value, browse_only:bool) -> ApiResult<()> {import_mode_epoch(s,id,v,browse_only,None).await}
pub(crate) async fn import_mode_epoch(s: &AppState,id:&str,v:&Value,browse_only:bool,epoch:Option<i64>)->ApiResult<()> {
    let tracks = validated_tracks(id, v)?;
    let provider=id.to_owned();
    s.db.run(move |c| {
        let tx = c.transaction()?;
        if let Some(epoch)=epoch{crate::provider_session::check(&tx,&provider,epoch)?;}
        for mut t in tracks {
            use rusqlite::OptionalExtension;
            let old=tx.query_row("SELECT * FROM tracks WHERE id=?1",[&t.id],crate::models::Track::from_row).optional()?;
            let is_new=old.is_none();
            if let Some(old)=old {
                if old.provider_data["manual_metadata"]==true {continue;}
                if let (Some(new),Some(previous))=(t.provider_data.as_object_mut(),old.provider_data.as_object()){for(k,v)in previous {new.entry(k.clone()).or_insert(v.clone());}}
                if let Some(observed)=old.provider_data.get("source_audio") {t.provider_data["source_audio"]=observed.clone();}
                t.cover_url=t.cover_url.or(old.cover_url);
            }
            if !t.provider_data.is_object(){t.provider_data=json!({});}
            if browse_only&&is_new {t.provider_data["catalog_browse_only"]=json!(true);}
            if !browse_only {t.provider_data.as_object_mut().unwrap().remove("catalog_browse_only");}
            crate::library::upsert(&tx, &t, None, None, 0, 0, None, None)?;
        }
        tx.commit()?;
        Ok(())
    })
    .await?;
    s.emit("library", json!({}));
    Ok(())
}
#[derive(Deserialize)]
pub struct PlaylistQuery {
    collection: Option<String>,
    page: Option<u32>,
}
#[derive(Deserialize)]
pub struct PlaylistTracksQuery {
    page: Option<u32>,
}
fn playlist_page(page: Option<u32>) -> ApiResult<u32> {
    let page = page.unwrap_or(1);
    if !(1..=100000).contains(&page) {
        return Err(ApiError::bad("页码须为 1–100000"));
    }
    Ok(page)
}
pub async fn playlists(
    State(s): State<AppState>, a: Auth, Path(id): Path<String>, Query(v): Query<PlaylistQuery>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    check_enabled(&s, &id).await?;
    let page = playlist_page(v.page)?;
    let collection = v.collection.as_deref().unwrap_or("all");
    if !matches!(collection, "all" | "created" | "collected") {
        return Err(ApiError::bad("歌单类型须为 all、created 或 collected"));
    }
    let result = s.plugin.call(&id, "playlists", json!({"collection":collection,"page":page})).await?;
    let entries = result["items"].as_array().ok_or_else(|| provider_invalid("插件歌单列表格式错误"))?;
    if entries.len() > 1000 || entries.iter().any(|item| {
        item["source"].as_str() != Some(&id) || item["id"].as_str().is_none_or(|v|v.is_empty() || v.len()>256)
            || item["name"].as_str().is_none() || item["track_count"].as_u64().is_none()
            || !matches!(item["kind"].as_str(), Some("created" | "collected" | "liked"))
            || !(item["cover_url"].is_null() || item["cover_url"].is_string())
    }) {
        return Err(provider_invalid("插件返回了无效歌单或其他音源的数据"));
    }
    validate_next_page(&result, page)?;
    let result=s.db.run(move|c|{let mut result=result;crate::media_assets::normalize_entries(c,&mut result["items"]);Ok(result)}).await?;
    Ok(Json(result))
}
fn validate_next_page(result: &Value, page: u32) -> ApiResult<()> {
    if !result["next_page"].is_null() && result["next_page"].as_u64() != Some(u64::from(page)+1) {
        return Err(provider_invalid("插件返回了无效分页游标"));
    }
    Ok(())
}
pub async fn playlist_tracks(
    State(s): State<AppState>, a: Auth, Path((id, playlist_id)): Path<(String,String)>, Query(v): Query<PlaylistTracksQuery>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    check_enabled(&s, &id).await?;
    let page = playlist_page(v.page)?;
    if playlist_id.is_empty() || playlist_id.len()>256 || playlist_id.chars().any(|c|c.is_control()) {
        return Err(ApiError::bad("歌单 ID 无效"));
    }
    let epoch=crate::provider_session::start(&s,&id).await?;
    let cached=crate::playlist_pages::get(&s,&a.id,&id,&playlist_id,page).await?;
    let cache_hit=cached.is_some();
    let account=crate::playlist_pages::context(&s,&id).await?;
    let result=if let Some(value)=cached{value}else{s.plugin.call(&id,"playlist_tracks",json!({"playlist_id":playlist_id,"page":page})).await?};
    if result["playlist_id"].as_str() != Some(&playlist_id) || result["total"].as_u64().is_none() || !result["items"].is_array() {
        return Err(provider_invalid("插件返回的歌单曲目分页无效"));
    }
    validate_next_page(&result, page)?;
    crate::provider_session::validate(&s,&id,epoch).await?;
    import_mode_epoch(&s, &id, &result,false,Some(epoch)).await?;
    if !cache_hit{crate::playlist_pages::save(&s,&a.id,&id,&playlist_id,page,account.as_deref(),&result).await?;}
    let mut result=result;
    let tracks=validated_tracks(&id,&result)?;
    result["items"]=serde_json::to_value(crate::catalog::prefer_local(&s,tracks).await?).map_err(anyhow::Error::from)?;
    Ok(Json(result))
}
pub async fn login_start(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    payload: Option<Json<Value>>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let params = payload.map(|Json(v)|v).unwrap_or_else(||json!({}));
    let fields = params.as_object().ok_or_else(||ApiError::bad("登录参数须为 JSON 对象"))?;
    let descriptor=crate::connectors::manifest(&s,&id).await?;
    if fields.get("type").is_some_and(|value|!descriptor["login_types"].as_array().is_some_and(|types|types.contains(value))){return Err(ApiError::bad("此音源未声明该登录方式"));}
    check_enabled(&s,&id).await?;
    let mut result=s.plugin.call(&id,"login_start",params).await?;
    result["provider_name"]=descriptor["name"].clone();let kind=result["login_type"].as_str().unwrap_or("");result["login_copy"]=descriptor["login_copy"][kind].clone();
    crate::playlist_sync::switching(&s,&id).await?;
    Ok(Json(result))
}
pub async fn login_poll(
    State(s): State<AppState>,
    a: Auth,
    Path((id, session)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let result=s.plugin.call(&id,"login_poll",json!({"session_id":session})).await?;
    if matches!(result["status"].as_str(),Some("authenticated"|"error"|"expired"|"refused")){let health=s.plugin.call(&id,"health",json!({})).await?;crate::playlist_sync::record_account(&s,&id,&health,true).await?;}
    Ok(Json(result))
}
pub async fn health(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    check_enabled(&s, &id).await?;
    let result=s.plugin.call(&id,"health",json!({})).await?;crate::playlist_sync::record_account(&s,&id,&result,false).await?;Ok(Json(result))
}
pub async fn logout(State(s):State<AppState>,a:Auth,Path(id):Path<String>)->ApiResult<Json<Value>> {
    a.require("admin")?;
    let manifest=crate::connectors::manifest(&s,&id).await?;
    if !manifest["capabilities"].as_array().is_some_and(|caps|caps.iter().any(|cap|cap=="logout")){return Err(ApiError::bad("此音源未提供退出登录接口"));}
    crate::playlist_sync::switching(&s,&id).await?;
    let result=s.plugin.call(&id,"logout",json!({})).await;
    let health=s.plugin.call(&id,"health",json!({})).await?;
    crate::playlist_sync::record_account(&s,&id,&health,true).await?;
    result?;
    s.plugin.stop(&id).await;
    Ok(Json(health))
}
pub async fn sync(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(v): Json<Value>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    check_enabled(&s, &id).await?;
    let result = s.plugin.call(&id, "sync", v).await?;
    import(&s, &id, &result).await?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test] async fn idle_processes_are_reaped_and_restart_from_saved_data(){
        let dir=tempfile::tempdir().unwrap();let state=fixture(dir.path()).await;
        std::fs::write(state.config.plugin_dir.join("example/response.json"),"{\"ok\":true}").unwrap();
        state.plugin.call("example","health",json!({})).await.unwrap();assert_eq!(state.plugin.process_count().await,1);
        assert_eq!(state.plugin.release_idle(std::time::Duration::ZERO).await,1);assert_eq!(state.plugin.process_count().await,0);
        assert_eq!(state.plugin.call("example","health",json!({})).await.unwrap()["ok"],true);state.plugin.stop_all().await;assert_eq!(state.plugin.process_count().await,0);
    }
    fn track(id: &str, source: &str) -> Value {
        json!({"id":id,"title":"真实歌单夹具","artist":"艺术家","album":"专辑","album_artist":"艺术家",
            "duration":30.0,"track_number":1,"disc_number":1,"source":source,"format":"unknown",
            "sample_rate":0,"bit_depth":0,"channels":0,"bitrate":0,"cover_url":null,
            "recording_id":id,"available":true,"created_at":0,"provider_data":{}})
    }
    fn auth(scopes: &[&str]) -> Auth {
        Auth { id:"test".into(),username:"test".into(),role:"member".into(),totp_enabled:false,
            interactive:false,scopes:scopes.iter().map(|v|v.to_string()).collect(),session_hash:String::new() }
    }
    async fn fixture(dir: &std::path::Path) -> AppState {
        let config = Config { data:dir.join("data"),web:dir.join("web"),base_path:String::new(),public_url:None,
            plugin_dir:dir.join("plugins"),python:std::env::var("RMUSIC_TEST_PYTHON").unwrap_or_else(|_|if cfg!(windows){"python".into()}else{"python3".into()}) };
        std::fs::create_dir_all(&config.data).unwrap();
        let plugin = config.plugin_dir.join("example");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("provider.py"),
            "import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line)\n v=json.loads(pathlib.Path(__file__).with_name('response.json').read_text(encoding='utf-8'))\n print(json.dumps({'id':r['id'],'result':v}),flush=True)\n").unwrap();
        let db = crate::db::Db::open(&dir.join("db.sqlite")).unwrap();
        AppState::new(db, config, "test".into()).await.unwrap()
    }
    #[tokio::test]
    async fn closure_cancelled_rpc_cannot_deliver_an_old_response_to_the_next_request(){
        let temp=tempfile::tempdir().unwrap();let s=fixture(temp.path()).await;let dir=s.config.plugin_dir.join("example");
        std::fs::write(dir.join("provider.py"),"import json,sys,time,pathlib\nfor line in sys.stdin:\n r=json.loads(line)\n if r['method']=='blocked': pathlib.Path(__file__).with_name('started').write_text('yes');time.sleep(10)\n print(json.dumps({'id':r['id'],'result':{'method':r['method']}}),flush=True)\n").unwrap();
        let plugin=s.plugin.clone();let task=tokio::spawn(async move{plugin.call("example","blocked",json!({})).await});
        tokio::time::timeout(std::time::Duration::from_secs(3),async{while !dir.join("started").exists(){tokio::time::sleep(std::time::Duration::from_millis(10)).await;}}).await.unwrap();task.abort();let _=task.await;
        assert_eq!(s.plugin.call("example","health",json!({})).await.unwrap()["method"],"health");s.plugin.stop("example").await;
    }
    #[test]
    fn imported_tracks_must_belong_to_provider_before_any_write() {
        assert!(validated_tracks("qqmusic", &json!({"items":[track("qqmusic:Valid123", "qqmusic")]})).is_ok());
        for (id, source) in [("local:file", "qqmusic"), ("qqmusic:Valid123", "local"), ("qqmusic:", "qqmusic"), ("qqmusic:bad id", "qqmusic")] {
            let result=validated_tracks("qqmusic", &json!({"items":[track("qqmusic:Valid123", "qqmusic"),track(id,source)]}));
            assert_eq!(result.unwrap_err().0, axum::http::StatusCode::BAD_GATEWAY);
        }
    }
    #[tokio::test]
    async fn foreground_preemption_is_not_lost_during_background_startup() {
        let temp=tempfile::tempdir().unwrap();let s=fixture(temp.path()).await;
        let dir=s.config.plugin_dir.join("qqmusic");std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);pathlib.Path('request-sent').write_text('yes')\n print(json.dumps({'id':r['id'],'result':{}}),flush=True)\n").unwrap();
        let gate=s.plugin.processes.lock().await;
        let mut background=Box::pin(s.plugin.background_metadata("qqmusic",json!({})));
        assert!(futures_util::poll!(background.as_mut()).is_pending());
        s.plugin.yield_background();
        let deferred=tokio::time::timeout(std::time::Duration::from_millis(500),background.as_mut()).await;
        drop(gate);
        assert_eq!(deferred.expect("后台等待启动锁时也必须让路").unwrap_err().1,"background_deferred");
        assert!(!dir.join("request-sent").exists());
        s.plugin.stop("qqmusic").await;
    }
    #[tokio::test]
    async fn read_browse_uses_real_rpc_imports_tracks_and_rejects_foreign_data() {
        let temp=tempfile::tempdir().unwrap();
        let state=fixture(temp.path()).await;
        let file=state.config.plugin_dir.join("example/response.json");
        std::fs::write(&file,json!({"items":[track("example:track1","example")],"total":1,"playlist_id":"liked","next_page":null}).to_string()).unwrap();
        let response=playlist_tracks(State(state.clone()),auth(&["read"]),Path(("example".into(),"liked".into())),Query(PlaylistTracksQuery{page:Some(1)})).await.unwrap();
        assert_eq!(response.0["items"][0]["id"],"example:track1");
        let imported=state.db.run(|c|Ok(c.query_row("SELECT source FROM tracks WHERE id='example:track1'",[],|r|r.get::<_,String>(0))?)).await.unwrap();
        assert_eq!(imported,"example");
        std::fs::write(&file,json!({"items":[track("local:foreign","local")],"total":1,"playlist_id":"liked","next_page":null}).to_string()).unwrap();
        let err=playlist_tracks(State(state.clone()),auth(&["read"]),Path(("example".into(),"liked".into())),Query(PlaylistTracksQuery{page:None})).await.unwrap_err();
        assert_eq!(err.1,"invalid_provider_response");
        let count=state.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM tracks",[],|r|r.get::<_,u32>(0))?)).await.unwrap();
        assert_eq!(count,1);
        state.plugin.stop("example").await;
    }
    #[tokio::test]
    async fn playlist_scope_collection_and_page_are_checked_before_rpc() {
        let temp=tempfile::tempdir().unwrap();
        let state=fixture(temp.path()).await;
        let forbidden=playlists(State(state.clone()),auth(&["control"]),Path("example".into()),Query(PlaylistQuery{collection:None,page:None})).await.unwrap_err();
        assert_eq!(forbidden.0,axum::http::StatusCode::FORBIDDEN);
        for query in [PlaylistQuery{collection:Some("favorites".into()),page:None},PlaylistQuery{collection:None,page:Some(0)}] {
            assert_eq!(playlists(State(state.clone()),auth(&["read"]),Path("example".into()),Query(query)).await.unwrap_err().0,axum::http::StatusCode::BAD_REQUEST);
        }
        assert!(state.plugin.processes.lock().await.is_empty());
        std::fs::write(state.config.plugin_dir.join("example/response.json"),json!({"items":[{"id":"liked","name":"我喜欢","track_count":3,"cover_url":null,"source":"example","kind":"liked"}],"next_page":2}).to_string()).unwrap();
        let response=playlists(State(state.clone()),auth(&["read"]),Path("example".into()),Query(PlaylistQuery{collection:None,page:None})).await.unwrap();
        assert_eq!(response.0["items"][0]["kind"],"liked");
        assert!(validate_next_page(&json!({"next_page":1}),1).is_err());
        state.plugin.stop("example").await;
    }
}
