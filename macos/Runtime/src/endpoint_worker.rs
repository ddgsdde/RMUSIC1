//! One joined control/output worker per physical UID. No device opens before a Core play command.
use anyhow::{Context, Result};
use rmusic_audio::output::{DsdOutput, OutputConfig};
use rmusic_bridge::{
    connection::ConnectionProfile, listens::Store,
    native::SessionOptions,
};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};

#[derive(Clone)]
pub struct ContextData {
    pub profile: ConnectionProfile,
    pub token: String,
    pub host: String,
    pub name: String,
    pub store: Arc<Store>,
    pub media: Arc<crate::streaming::ClientMedia>,
    pub replies: tokio::sync::mpsc::Sender<Value>,
}
pub struct Worker {
    pub observation: Arc<rmusic_bridge::observation::PlaybackObservation>,
    pub status: Arc<Mutex<Value>>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Worker {
    pub fn start(context: ContextData, mut routes: watch::Receiver<ConnectionProfile>, id: String, uid: String, name: String) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let status = Arc::new(Mutex::new(json!({})));
        let reports = status.clone();
        let observation=Arc::new(rmusic_bridge::observation::PlaybackObservation::default());let visual=observation.clone();
        let task = tokio::spawn(async move {
            loop {
                if *stopped.borrow() {
                    break;
                }
                reports.lock().unwrap()["registered"] = json!(false);
                let operation = async {
                    let profile = routes.borrow_and_update().clone();
                    let inventory =
                        core_request(&profile, &context.token, "api/v1/devices", "GET", None)
                            .await?;
                    let device = inventory["items"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|v| v["id"] == context.host)
                        .context("本机设备清单不存在或已被移除")?;
                    let output = device["outputs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|v| v["id"] == id)
                        .context("输出已从 Core 清单移除")?;
                    if output["config"]["enabled"] != true {
                        return Ok(());
                    }
                    let config: OutputConfig = serde_json::from_value(output["config"].clone())?;
                    config.limits.validate()?;
                    anyhow::ensure!(
                        config.device_id.as_deref() == Some(&uid),
                        "输出 UID 与设备清单不一致，拒绝切换设备"
                    );
                    anyhow::ensure!(
                        config.dsd_output == DsdOutput::Disabled,
                        "Mac 尚未开放 DSD 输出"
                    );
                    rmusic_bridge::native::session_with_options(
                        &profile,
                        &context.token,
                        &id,
                        &name,
                        config,
                        Some(context.store.clone()),
                        async {
                            if *stopped.borrow() { return; }
                            tokio::select! {
                                _ = stopped.changed() => {},
                                _ = async {
                                    loop {
                                        if routes.changed().await.is_err() { return; }
                                        while routes.borrow().address != profile.address {
                                            // Keep the active HAL stream until it reaches a safe handoff.
                                            let playing = reports.lock().unwrap()["playback"]["status"] == "playing";
                                            if !playing { return; }
                                            tokio::time::sleep(Duration::from_millis(350)).await;
                                        }
                                    }
                                } => {},
                            }
                        },
                        SessionOptions {
                            observation: Some(visual.clone()),
                            media: Some(context.media.clone()),
                            device_volume:Some(Arc::new({let uid=uid.clone();move|value|{
                                let state=match value{Some(value)=>rmusic_coreaudio::set_device_volume(&uid,value)?,None=>rmusic_coreaudio::device_volume(&uid)?};
                                Ok(crate::outputs::volume_json(state))
                            }})),
                            preserve_cursor: true,
                            configure_output: false,
                            inventory_managed: true,
                            report: Some(Arc::new({
                                let reports = reports.clone();
                                move |report: &Value| {
                                    let mut state = reports.lock().unwrap();
                                    if report["type"] == "error" {
                                        state["error"] = report.clone();
                                        let mut event=json!({"at":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),"position":state["playback"]["position"]});
                                        for key in ["zone_id","track_id","core_playback_id","source_failed","device_failed","recoverable"] {event[key]=report[key].clone();}
                                        let message=report["error"].as_str().unwrap_or("播放异常").split_whitespace().map(|word|if word.contains("://"){"<地址已隐藏>"}else{word}).collect::<Vec<_>>().join(" ");
                                        event["error"]=json!(message.chars().take(500).collect::<String>());
                                        if !state["recent_errors"].is_array(){state["recent_errors"]=json!([]);}
                                        let events=state["recent_errors"].as_array_mut().unwrap();events.push(event);if events.len()>16{events.remove(0);}

                                    }
                                    if report["type"] == "registered" {
                                        state["registered"] = json!(true);
                                        state.as_object_mut().unwrap().remove("connection_error");
                                    }
                                    if report["type"] == "state" || report["type"] == "ended" {
                                        state["playback"] = report.clone();
                                        // Only reports already accepted by the shared playback owner arrive here.
                                        // A fresh healthy run must not retain a previous worker's error.
                                        if report["type"] == "state" && matches!(report["status"].as_str(),Some("playing"|"paused")) {
                                            state.as_object_mut().unwrap().remove("error");
                                        }
                                    }
                                }
                            })),
                        },
                    )
                    .await
                };
                let result = operation.await;
                visual.clear();
                {
                    let mut state = reports.lock().unwrap();
                    state["registered"] = json!(false);
                    if let Err(error) = result {
                        eprintln!("Mac output connection interrupted: {error:#}");
                        state["connection_error"] = json!("与 Core 的音频控制连接已中断，正在重连。播放位置会保留；设备错误需手动恢复。");
                    }
                }
                if *stopped.borrow() {
                    break;
                }
                tokio::select! {_=stopped.changed()=>break,_=tokio::time::sleep(Duration::from_secs(2))=>{}}
            }
        });
        Self { status, stop, task, observation }
    }
    pub fn clear_error(&self) {
        let mut status = self.status.lock().unwrap();
        if let Some(object) = status.as_object_mut() {
            object.remove("error");
            object.remove("connection_error");
        }
    }
    pub async fn close(self) {
        self.stop.send_replace(true);
        let _ = self.task.await;
    }
}
pub async fn core_request(
    profile: &ConnectionProfile,
    token: &str,
    path: &str,
    method: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let mut request = profile
        .http()?
        .request(method.parse()?, profile.endpoint(path)?)
        .bearer_auth(token)
        .timeout(Duration::from_secs(10));
    if let Some(body) = body {
        request = request.json(body);
    }
    let mut response = request.send().await?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
            "Core 清单响应过大"
        );
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        return Err(crate::session::HttpFailure {
            status: status.as_u16(),
            message: serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().or_else(||v["message"].as_str()).map(str::to_owned))
                .unwrap_or_else(|| status.to_string()),
        }
        .into());
    }
    Ok(if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)?
    })
}
