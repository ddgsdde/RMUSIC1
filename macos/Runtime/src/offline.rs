//! The last signed-in account may listen to complete originals on this Mac while Core is unreachable.
use crate::{downloads::Downloads, local_outputs::LocalOutputs};
use anyhow::{ensure, Context, Result};
use rmusic_bridge::{
    client_stream::ClientStream,
    connection::ConnectionProfile,
    media_proxy::{MediaProxy, MediaResolver},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Serialize, Deserialize)]
struct Account {
    core: String,
    user: String,
    username: String,
    host: String,
}

fn marker(root: &Path) -> PathBuf {
    root.join("offline-listener.json")
}
pub fn remember(root: &Path, info: &Value) -> Result<()> {
    let account = Account {
        core: info["core_id"].as_str().context("缺少 Core 身份")?.into(),
        user: info["user"]["id"].as_str().context("缺少账号身份")?.into(),
        username: info["user"]["username"].as_str().unwrap_or("").into(),
        host: info["host_id"].as_str().context("缺少本机身份")?.into(),
    };
    let dest = marker(root);
    let tmp = dest.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(&account)?)?;
    std::fs::rename(tmp, dest)?;
    Ok(())
}
pub fn forget(root: &Path) -> Result<()> {
    match std::fs::remove_file(marker(root)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}
struct CompleteOriginals(Arc<Downloads>);
impl MediaResolver for CompleteOriginals {
    fn resolve<'a>(
        &'a self,
        _: &'a ConnectionProfile,
        track: &'a str,
        _: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<Arc<MediaProxy>>> {
        Box::pin(async move {
            self.0
                .preferred_original(track)
                .await?
                .context("歌曲没有可用的完整缓存")
        })
    }
}
pub struct Library {
    root: PathBuf,
    account: Account,
    profile: ConnectionProfile,
    originals: Arc<CompleteOriginals>,
    player: Option<ClientStream>,
}
impl Library {
    pub fn open(root: &Path) -> Result<Option<Self>> {
        let bytes = match std::fs::read(marker(root)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let account: Account = serde_json::from_slice(&bytes)?;
        let profile = ConnectionProfile {
            id: "offline".into(),
            name: "本机缓存".into(),
            address: "http://127.0.0.1".into(),
            alternatives: vec![],
            core_id: Some(account.core.clone()),
            core_public_key: None,
            certificate_sha256: None,
            skip_certificate_verification: false,
        };
        let downloads = Downloads::open(
            root,
            &account.core,
            &account.user,
            profile.clone(),
            String::new(),
        )?;
        Ok(Some(Self {
            root: root.into(),
            account,
            profile,
            originals: Arc::new(CompleteOriginals(downloads)),
            player: None,
        }))
    }
    pub fn list(&self, offset: usize) -> Result<Value> {
        Ok(
            json!({"username":self.account.username,"page":self.originals.0.complete_page(offset)?,
            "outputs":LocalOutputs::open(&self.root)?.snapshot(&self.account.host)?}),
        )
    }
    pub async fn control(&mut self, mut cmd: Value) -> Result<Value> {
        let kind = cmd["type"].as_str().unwrap_or("").to_owned();
        if kind == "stop" {
            self.stop().await;
            return Ok(Value::Null);
        }
        if kind == "play" {
            let inventory = LocalOutputs::open(&self.root)?.snapshot(&self.account.host)?;
            let output = inventory["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| {
                    o["id"] == cmd["output"] && o["enabled"] == true && o["available"] == true
                })
                .context("请先启用本机输出")?;
            ensure!(
                cmd["dsp"]["convolution"].is_null(),
                "配置了卷积的输出需要 Core 提供响应文件"
            );
            let track = cmd["track"]["id"].as_str().context("请选择缓存歌曲")?;
            let encoded = crate::streaming::url_encode(track);
            cmd["url"] = json!(format!("api/v1/media/{encoded}"));
            self.stop().await;
            self.player = Some(ClientStream::start(
                self.profile.clone(),
                String::new(),
                output["id"].as_str().unwrap().into(),
                serde_json::from_value(output["config"].clone())?,
                self.originals.clone(),
                None,
                Some(rmusic_bridge::client_stream::Journal::new(rmusic_bridge::listens::Store::open(&self.root)?,rmusic_bridge::listens::Owner {core_id:self.account.core.clone(),user_id:self.account.user.clone()})),
            )?);
        }
        ensure!(
            matches!(
                kind.as_str(),
                "play" | "pause" | "resume" | "seek" | "volume"
            ),
            "未知的离线播放操作"
        );
        self.player
            .as_ref()
            .context("请先选择缓存歌曲")?
            .command(cmd)
            .await?;
        Ok(self.status())
    }
    pub fn status(&self) -> Value {
        self.player
            .as_ref()
            .map(|p| p.status.lock().unwrap().clone())
            .unwrap_or(Value::Null)
    }
    pub async fn stop(&mut self) {
        if let Some(p) = self.player.take() {
            p.close().await;
        }
    }
}
