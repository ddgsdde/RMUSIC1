//! Play a pull stream on one local output without registering a Core endpoint or reserving a Core queue.
use crate::{connection::ConnectionProfile, media_proxy::MediaResolver, playback::Playback};
use anyhow::{Context, Result};
use rmusic_audio::output::OutputConfig;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
mod journal;
pub use journal::Journal;

#[derive(Clone)]
pub struct ControlPort(mpsc::WeakSender<(Value, oneshot::Sender<Result<()>>)>);
impl ControlPort {
    pub async fn command(&self, cmd: Value) -> Result<()> {
        let sender = self.0.upgrade().context("本机播放器已关闭")?;
        let (reply, result) = oneshot::channel();
        sender
            .send((cmd, reply))
            .await
            .map_err(|_| anyhow::anyhow!("本机播放器已关闭"))?;
        result.await.context("本机播放器已关闭")?
    }
}
pub struct ClientStream {
    pub output: OutputConfig,
    commands: mpsc::Sender<(Value, oneshot::Sender<Result<()>>)>,
    pub status: Arc<Mutex<Value>>,
    pub observation: Arc<crate::observation::PlaybackObservation>,
    task: tokio::task::JoinHandle<()>,
}
impl ClientStream {
    pub fn start(
        profile: ConnectionProfile,
        token: String,
        id: String,
        output: OutputConfig,
        media: Arc<dyn MediaResolver>,
        terminal: Option<Box<dyn Fn(&Value) + Send + Sync>>,
        mut journal: Option<Journal>,
    ) -> Result<Self> {
        let sync=if token.is_empty() {None} else {journal.as_ref().map(|j|j.sync(profile.clone(),token.clone(),id.clone())).transpose()?};
        let mut player = Playback::connected(profile.clone(), token, id)?;
        player.attach_media(Some(media));
        let (tx, mut rx) = mpsc::channel::<(Value, oneshot::Sender<Result<()>>)>(8);
        let (reports, mut events) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(json!({})));
        let status = state.clone();
        let observation = Arc::new(crate::observation::PlaybackObservation::default());
        let visual = observation.clone();
        let selected_output = output.clone();
        let task = tokio::spawn(async move {
            let _sync=sync;
            let mut clock = tokio::time::interval(std::time::Duration::from_millis(80));
            clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let report = tokio::select! {
                    cmd=rx.recv()=>{let Some((cmd,reply))=cmd else{break};let kind=cmd["type"].as_str().unwrap_or("");
                        let result=player.client_command(kind,&cmd,&profile).and_then(|()| {
                            if let Some(journal)=&mut journal { journal.command(&cmd,&state.lock().unwrap())?; }
                            Ok(())
                        });
                        if result.is_ok() && kind=="play" {*state.lock().unwrap()=json!({"status":"buffering","core_playback_id":cmd["playback_id"]});}
                        if result.is_ok() && matches!(kind,"stop"|"pause"|"resume"|"seek") {
                            let mut s=state.lock().unwrap();match kind {
                                "stop"=>{s["status"]=json!("stopped");s["position"]=json!(0);s["buffered_until"]=Value::Null;},
                                "pause"=>s["status"]=json!("paused"),"resume"=>s["status"]=json!("playing"),
                                "seek"=>{s["position"]=cmd["position"].clone();s["buffered_until"]=Value::Null;},_=>{}
                            }
                        }
                        let _=reply.send(result);None
                    },
                    event=events.recv()=>event.filter(|v|player.accepts(v)),
                    event=player.update()=>event.or_else(||player.start_ready(&output,&reports)),
                    _=clock.tick(),if visual.active()=>{if let Some(run)=player.running(){visual.publish(run.observation(visual.audio()));}None},
                };
                if let Some(mut report) = report {
                    if report["type"] == "state" && matches!(report["status"].as_str(),Some("playing"|"paused")) {
                        if let Some(run)=player.running() {
                            let (fraction,until,filling)=run.buffered(&report);
                            report["buffered_fraction"]=json!(fraction);
                            report["buffered_until"]=json!(until);
                            report["filling_current"]=json!(filling);
                        }
                    }
                    if let Some(journal)=&mut journal {
                        if let Err(error)=journal.report(&report) { eprintln!("本机聆听记录暂存失败：{error}"); }
                    }
                    let notify = report["status"] == "ended" || report["type"] == "error";
                    *state.lock().unwrap() = report.clone();
                    if notify {
                        if let Some(send) = &terminal {
                            send(&report);
                        }
                    }
                }
            }
            player.shutdown().await;
            visual.clear();
        });
        Ok(Self {
            output: selected_output,
            commands: tx,
            status,
            observation,
            task,
        })
    }
    pub fn port(&self) -> ControlPort {
        ControlPort(self.commands.downgrade())
    }
    pub async fn command(&self, v: Value) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send((v, tx))
            .await
            .map_err(|_| anyhow::anyhow!("本机播放器已关闭"))?;
        rx.await.context("本机播放器已关闭")?
    }
    pub async fn close(self) {
        drop(self.commands);
        let _ = self.task.await;
    }
}
