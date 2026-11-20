use anyhow::{bail, Context, Result};
use rmusic_bridge::{connection::ConnectionProfile, listens::Store};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    sync::Arc,
    time::Duration,
};

#[derive(Deserialize)]
struct Login {
    profile: ConnectionProfile,
    #[serde(default)] anonymous_metadata:bool,
    #[serde(default)] streaming_policy: rmusic_bridge::media_proxy::StreamingPolicy,
    #[serde(default)] defer_output_refresh: bool,
    username: String,
    password: String,
    #[serde(default)] session_token: Option<String>,
    totp: Option<String>,
    #[serde(alias = "endpoint_id")]
    host_id: String,
    name: String,
    device_name: Option<String>,
}
#[derive(Debug)]
pub struct HttpFailure {
    pub status: u16,
    pub message: String,
}
impl std::fmt::Display for HttpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Core {}: {}", self.status, self.message)
    }
}
impl std::error::Error for HttpFailure {}
pub fn transient_http(error: &anyhow::Error) -> bool {
    error.downcast_ref::<reqwest::Error>().is_some_and(|e| e.is_timeout() || e.is_connect()
        || e.status().is_some_and(|s| matches!(s.as_u16(), 408 | 429 | 502 | 503 | 504)))
}
pub struct Session {
    events: Option<rmusic_bridge::connection::ConnectionEvents>,
    event_replies: tokio::sync::mpsc::Sender<Value>,
    _connection_sync: rmusic_bridge::connection::ConnectionSync,
    _listen_sync: rmusic_bridge::listens::sync::Job,
    anonymous_metadata: std::sync::atomic::AtomicBool,
    streaming_policy: Arc<std::sync::RwLock<rmusic_bridge::media_proxy::StreamingPolicy>>,
    profile: ConnectionProfile,
    selected: ConnectionProfile,
    api: crate::api::Api,
    token: String,
    pub downloads: Arc<crate::downloads::Downloads>,
    pub exports:crate::file_exports::FileExports,
    pub outputs: crate::outputs::Outputs,
    assets: std::path::PathBuf,
    host: String,
    user: Value,
}
impl Session {
    pub async fn connect(body: Value, store: Arc<Store>, root: &std::path::Path, replies:tokio::sync::mpsc::Sender<Value>) -> Result<Self> {
        let mut input: Login = serde_json::from_value(body)?;
        if !input.host_id.starts_with("mac-")
            || input.host_id.len() > 76
            || !input
                .host_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            bail!("invalid native endpoint identity");
        }
        if input.name.trim().is_empty() || input.name.len() > 100 {
            bail!("invalid endpoint name");
        }
        input.streaming_policy.validate()?;
        let selected = input.profile.resolve().await?;
        input.profile.core_id = selected.core_id.clone();
        input.profile.core_public_key = selected.core_public_key.clone();
        selected.require_encrypted_credentials()?;
        let http = selected.http()?;
        let client=json!({"id":input.host_id,"name":input.device_name.as_deref().unwrap_or(&input.name),"type":"macos"});
        let (token,user)=if let Some(token)=input.session_token.filter(|_|!rmusic_bridge::connection::standalone_bridge()){
            let user:Value=http.get(selected.endpoint("api/v1/auth/me")?).bearer_auth(&token)
                .timeout(Duration::from_secs(8)).send().await?.error_for_status()?.json().await?;
            if user["username"].as_str().is_none_or(|name|name.to_lowercase()!=input.username.to_lowercase()){bail!("已保存会话不属于当前账号");}
            let _=http.put(selected.endpoint("api/v1/auth/sessions/current-device")?).bearer_auth(&token)
                .timeout(Duration::from_secs(3)).json(&client).send().await;
            (token,user)
        }else{
            let auth:Value=http.post(selected.endpoint("api/v1/auth/login")?)
                .timeout(Duration::from_secs(15))
                .json(&json!({"username":input.username,"password":input.password,"totp":input.totp,"client":client}))
                .send().await?.error_for_status()?.json().await?;
            (auth["token"].as_str().context("登录响应缺少会话")?.to_owned(),auth["user"].clone())
        };
        // The session watcher refreshes signed endpoints after login.
        let core = input
            .profile
            .core_id
            .as_deref()
            .context("missing Core identity")?;
        let user_id = user["id"]
            .as_str()
            .context("missing user identity")?;
        store.remember(&input.profile.id,&rmusic_bridge::listens::Owner {core_id:core.into(),user_id:user_id.into()})?;
        if user["role"]=="external"{input.host_id=format!("{}-u-{}",input.host_id,user_id);}
        let downloads = crate::downloads::Downloads::open(
            root,
            core,
            user_id,
            input.profile.clone(),
            token.clone(),
        )?;
        use sha2::{Digest, Sha256};
        let assets = root.join("artwork").join(format!(
            "{:x}",
            Sha256::digest(format!("{}:{core}{user_id}", core.len()))
        ));
        let streaming_policy=Arc::new(std::sync::RwLock::new(input.streaming_policy));
        let client_media=Arc::new(crate::streaming::ClientMedia{local:downloads.clone(),policy:streaming_policy.clone(),token:token.clone(),capability:tokio::sync::OnceCell::new()});
        let api = crate::api::Api::new(input.profile.clone(),selected.clone(),token.clone())?;
        let listen_sync=rmusic_bridge::listens::sync::Sync::new(store.clone(),
            rmusic_bridge::sequence::Config::new(input.profile.clone(),token.clone(),input.host_id.clone())?,
            rmusic_bridge::listens::Owner{core_id:core.into(),user_id:user_id.into()}).schedule();
        let outputs = crate::outputs::Outputs::new(crate::endpoint_worker::ContextData {
            profile: input.profile.clone(),
            token: token.clone(),
            host: input.host_id.clone(),
            name: input.name,
            store,
            media: client_media, replies:replies.clone(),
        }, root.join("output-volume-defaults-v1.json"), api.clone(), input.defer_output_refresh)
        .await?;
        let exports=crate::file_exports::FileExports::new(downloads.clone(),root,downloads.scope())?;
        let mut session = Self {
            events:None, event_replies:replies,
            _connection_sync: input.profile.follow_connection_info(&selected,&token),
            _listen_sync:listen_sync,
            anonymous_metadata:std::sync::atomic::AtomicBool::new(input.anonymous_metadata),
            streaming_policy,downloads,exports,
            outputs,
            assets,
            profile: input.profile.clone(),
            api,
            selected,
            token,
            host: input.host_id,
            user,
        };
        session.set_ui_visible(!rmusic_bridge::connection::standalone_bridge());
        Ok(session)
    }
    pub fn set_ui_visible(&mut self, visible: bool) -> Value {
        if rmusic_bridge::connection::standalone_bridge() { self.events=None; }
        else if self.events.is_none() {
            let replies=self.event_replies.clone();
            self.events=Some(self.profile.follow_selected_events(self.token.clone(),Arc::new(move|event| {
                if matches!(event["type"].as_str(),Some("playlists"|"connection_ready"|"portable_command"|"portable_loudness"|"connection_interrupted")) {
                    let replies=replies.clone();
                    tokio::spawn(async move { let _=replies.send(json!({"id":"core-event","result":event})).await; });
                }
            }),self.api.routes()));
        }
        json!({"visible":visible})
    }
    pub fn configure_metadata(&self,value:Value)->Result<Value>{
        let enabled=value["enabled"].as_bool().context("资料直连开关无效")?;
        self.anonymous_metadata.store(enabled,std::sync::atomic::Ordering::Relaxed);Ok(json!({"saved":true}))
    }
    pub fn configure_streaming(&self,value:Value)->Result<Value>{
        let policy:rmusic_bridge::media_proxy::StreamingPolicy=serde_json::from_value(value)?;policy.validate()?;
        *self.streaming_policy.write().map_err(|_|anyhow::anyhow!("网络播放设置不可用"))?=policy;
        Ok(json!({"saved":true,"apply":"next_playback"}))
    }
    pub fn info(&self, include_session:bool) -> Value {
        let mut value=json!({"core_id":self.profile.core_id,"core_public_key":self.profile.core_public_key,"profile":self.profile,"address":rmusic_bridge::connection::logical_address(&self.selected.address),"host_id":self.host,"user":self.user});
        if include_session&&!rmusic_bridge::connection::standalone_bridge(){value["session_token"]=json!(self.token);}
        value
    }
    pub fn install_plugin_task(&self,body:Value)->Result<futures_util::future::BoxFuture<'static,Result<Value>>>{
        let profile=self.profile.clone();let token=self.token.clone();
        Ok(Box::pin(async move {
        let path=std::path::PathBuf::from(body["path"].as_str().context("请选择插件 ZIP")?);
        anyhow::ensure!(path.is_absolute(),"插件路径必须为绝对路径");
        let file=tokio::fs::File::open(path).await?;let metadata=file.metadata().await?;
        anyhow::ensure!(metadata.is_file()&&metadata.len()<=16*1024*1024,"插件 ZIP 不能超过 16 MiB");
        use tokio::io::AsyncReadExt;let mut bytes=Vec::new();file.take(16*1024*1024+1).read_to_end(&mut bytes).await?;
        anyhow::ensure!(bytes.len()<=16*1024*1024,"插件 ZIP 超过限制");
        let profile=profile.resolve().await?;
        let response=profile.http()?.post(profile.endpoint("api/v1/plugins/install")?).bearer_auth(&token).header("Content-Type","application/zip").body(bytes).timeout(Duration::from_secs(90)).send().await?;
        let status=response.status();let result:Value=response.json().await?;
        if !status.is_success(){return Err(HttpFailure{status:status.as_u16(),message:result["error"]["message"].as_str().unwrap_or("插件安装失败").into()}.into());}Ok(result)
        }))
    }
    pub fn import_account_task(&self,body:Value)->Result<futures_util::future::BoxFuture<'static,Result<Value>>>{
        let api=self.api.clone();Ok(Box::pin(async move{api.import_account_file(body).await}))
    }
    pub fn save_file_task(&self,body:Value)->Result<futures_util::future::BoxFuture<'static,Result<Value>>> {
        let api=self.api.clone();Ok(Box::pin(crate::api_files::save(api,body)))
    }
    pub fn downloads_task(&self,body:Value)->Result<futures_util::future::BoxFuture<'static,Result<Value>>>{
        let downloads=self.downloads.clone();let offset=body["export_offset"].as_u64().unwrap_or(0).min(2000)as usize;let exports=self.exports.snapshot(offset);let cache_offset=body["cache_offset"].as_u64().unwrap_or(0)as usize;
        Ok(Box::pin(async move{tokio::task::spawn_blocking(move||{let mut value=downloads.list(cache_offset)?;value["exports"]=exports;Ok(value)}).await?}))
    }
    pub fn connection_metrics_task(&self,body:Value)->Result<futures_util::future::BoxFuture<'static,Result<Value>>> {
        let api=self.api.clone();let speed=body["speed"].as_bool().unwrap_or(false);
        Ok(Box::pin(async move {api.connection_metrics(speed).await}))
    }
    pub fn request_task(&self,body:Value)->Result<futures_util::future::BoxFuture<'static,Result<Value>>>{
        let api=self.api.clone();let direct=self.anonymous_metadata.load(std::sync::atomic::Ordering::Relaxed);Ok(Box::pin(async move{if direct{crate::public_metadata::request(api,body).await}else{api.request(body).await}}))
    }
    pub fn status(&self) -> Value {
        let mut status=self.outputs.snapshot();status["transport"]=self.api.transport_status().unwrap_or_else(||rmusic_bridge::connection::remote_status(&self.selected.address));status
    }
    pub fn clear_error(&self, id: Option<&str>) {
        self.outputs.clear_error(id);
    }
    pub fn artwork_task(
        &self,
        mut body: Value,
    ) -> Result<futures_util::future::BoxFuture<'static, Result<Value>>> {
        let options = body.as_object_mut().context("缺少封面请求")?;
        options.insert("anonymous_metadata".into(), json!(self.anonymous_metadata.load(std::sync::atomic::Ordering::Relaxed)));
        let (root, api, token) = (self.assets.clone(), self.api.clone(), self.token.clone());
        Ok(Box::pin(async move {
            let (profile, _) = api.selected_route().await?;
            if body["cover_url"].as_str().is_some_and(|url| !url.is_empty()) {
                crate::artwork::fetch(&root, &profile, &token, &body).await
            } else {
                let id = body["track_id"].as_str().context("missing track identity")?.to_owned();
                crate::artwork::fetch_track(root, profile, token, id, body).await
            }
        }))
    }
    pub async fn close(self) -> Result<()> {
        self.finish(false).await
    }
    pub async fn logout(self) -> Result<()> {
        self.finish(true).await
    }
    async fn finish(mut self, revoke: bool) -> Result<()> {
        self.events = None;
        self.outputs.close().await;
        self.exports.close().await;
        self.downloads.close().await;
        if revoke {
            let _ = tokio::time::timeout(Duration::from_secs(3), self.api.request(json!({
                "path":"/auth/logout","method":"POST","json":{}
            }))).await;
        }
        self.api.close().await;
        Ok(())
    }
}
