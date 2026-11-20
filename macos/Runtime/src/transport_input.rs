//! A local media key must not queue behind Core route discovery or LAN registration.
use super::{response, Command};
use anyhow::{Context, Result};
use rmusic_bridge::client_stream::ControlPort;
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc,
};

type Selection = (String, String, ControlPort);
fn selection() -> &'static Mutex<Option<Selection>> {
    static ACTIVE: OnceLock<Mutex<Option<Selection>>> = OnceLock::new();
    ACTIVE.get_or_init(Default::default)
}
fn observations() -> &'static Mutex<std::collections::HashMap<String, Weak<rmusic_bridge::observation::PlaybackObservation>>> {
    static PORTS: OnceLock<Mutex<std::collections::HashMap<String, Weak<rmusic_bridge::observation::PlaybackObservation>>>> = OnceLock::new();
    PORTS.get_or_init(Default::default)
}
pub fn observe(id: &str, source: &Arc<rmusic_bridge::observation::PlaybackObservation>) {
    observations().lock().unwrap().insert(id.to_owned(), Arc::downgrade(source));
}
pub fn unobserve(id: &str) { observations().lock().unwrap().remove(id); }
fn observation(body: &Value) -> Value {
    let source = body["endpoint_id"].as_str().and_then(|id| observations().lock().unwrap().get(id).and_then(Weak::upgrade));
    source.map(|source| source.read(body["audio"] == true)).unwrap_or_else(|| serde_json::json!({"local":false}))
}
pub fn clear() {
    *selection().lock().unwrap() = None;
}
pub fn publish(output: String, instance: String, port: ControlPort) {
    *selection().lock().unwrap() = Some((output, instance, port));
}
async fn command(cmd: &Command) -> Result<Value> {
    let (id, instance, port) = selection()
        .lock()
        .unwrap()
        .clone()
        .context("当前曲目已切换")?;
    anyhow::ensure!(
        cmd.body["output"] == id && cmd.body["playback_id"] == instance,
        "当前曲目已切换"
    );
    anyhow::ensure!(
        matches!(
            cmd.body["type"].as_str(),
            Some("pause" | "resume" | "seek" | "stop" | "volume")
        ),
        "未知本机控制"
    );
    port.command(cmd.body.clone()).await?;
    Ok(Value::Null)
}
async fn next(input: &mut BufReader<tokio::io::Stdin>) -> Result<Option<Command>> {
    let mut line = Vec::new();
    loop {
        let bytes = input.fill_buf().await?;
        if bytes.is_empty() {
            break;
        }
        let length = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |n| n + 1);
        anyhow::ensure!(
            line.len() + length <= 4 * 1024 * 1024 + 65536,
            "command exceeds private protocol limit"
        );
        let done = bytes[length - 1] == b'\n';
        line.extend_from_slice(&bytes[..length]);
        input.consume(length);
        if done {
            break;
        }
    }
    if line.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&line)?))
}
pub(super) fn start(
    responses: mpsc::Sender<Value>,
) -> (mpsc::Receiver<Result<Command>>, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(40);
    let task = tokio::spawn(async move {
        let mut input = BufReader::new(tokio::io::stdin());
        loop {
            match next(&mut input).await {
                Ok(Some(cmd)) if cmd.operation == "playback_observation" => {
                    if responses.send(response(&cmd.id, Ok(observation(&cmd.body)))).await.is_err() { break; }
                }
                Ok(Some(cmd)) if cmd.operation == "local_transport" => {
                    let result = command(&cmd).await;
                    if responses.send(response(&cmd.id, result)).await.is_err() {
                        break;
                    }
                }
                Ok(Some(cmd)) => {
                    let closing=cmd.operation=="quit";
                    if tx.send(Ok(cmd)).await.is_err() || closing {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    break;
                }
            }
        }
    });
    (rx, task)
}
