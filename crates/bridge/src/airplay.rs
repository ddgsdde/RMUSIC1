use crate::{connection::ConnectionProfile, media_proxy::MediaProxy};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{client::IntoClientRequest, Message},
    Connector,
};

pub struct AirPlayAdapter {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    sequence: u64,
}
impl AirPlayAdapter {
    pub async fn start(path: &Path) -> Result<Self> {
        let python = std::env::var_os("RMUSIC_PYTHON").unwrap_or_else(|| "python".into());
        let mut command = Command::new(python);
        command
            .arg("-u")
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .context("cannot start AirPlay Python adapter")?;
        let stdin = child.stdin.take().context("missing adapter stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("missing adapter stdout")?);
        Ok(Self {
            child,
            stdin,
            stdout,
            sequence: 0,
        })
    }
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        if let Some(status) = self.child.try_wait()? {
            bail!("AirPlay adapter exited: {status}")
        }
        self.sequence += 1;
        let request = json!({"jsonrpc":"2.0","id":self.sequence,"method":method,"params":params});
        self.stdin
            .write_all(format!("{request}\n").as_bytes())
            .await?;
        self.stdin.flush().await?;
        let sequence = self.sequence;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let mut line = String::new();
                if self.stdout.read_line(&mut line).await? == 0 {
                    bail!("AirPlay adapter closed stdout")
                }
                if line.len() > 1024 * 1024 {
                    bail!("AirPlay adapter response exceeds limit")
                }
                let response: Value = serde_json::from_str(&line)?;
                if response["id"].as_u64() != Some(sequence) {
                    continue;
                }
                if let Some(error) = response.get("error") {
                    bail!(
                        "AirPlay: {}",
                        error["message"]
                            .as_str()
                            .unwrap_or("adapter request failed")
                    )
                }
                return Ok(response["result"].clone());
            }
        })
        .await
        .context("AirPlay adapter timeout")??;
        Ok(result)
    }
}

pub async fn run(
    profile: &ConnectionProfile,
    token: &str,
    endpoint_id: &str,
    name: &str,
    device_id: &str,
    adapter_path: &Path,
) -> Result<()> {
    let mut adapter = AirPlayAdapter::start(adapter_path).await?;
    let devices = adapter.call("discover", json!({"timeout":5})).await?;
    if !devices["items"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["id"] == device_id))
    {
        bail!("AirPlay device not discovered; use --airplay-discover, then pair if necessary")
    }
    let mut retry = 1;
    loop {
        let result=async {
            let selected=profile.resolve_lan_only().await?;
            session(&selected, token, endpoint_id, name, device_id, &mut adapter).await
        }.await;
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = adapter.call("stop", json!({"device_id":device_id})).await;
                eprintln!("AirPlay bridge disconnected: {error:#}; retry in {retry}s");
                tokio::select! { _ = tokio::time::sleep(Duration::from_secs(retry)) => {}, _ = tokio::signal::ctrl_c() => return Ok(()) }
                retry = (retry * 2).min(30);
            }
        }
    }
}
async fn session(
    profile: &ConnectionProfile,
    token: &str,
    endpoint_id: &str,
    name: &str,
    device_id: &str,
    adapter: &mut AirPlayAdapter,
) -> Result<()> {
    let mut url = profile.endpoint("api/v1/bridge/events")?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("invalid websocket scheme"))?;
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse()?);
    let (mut socket, _) = connect_async_tls_with_config(
        request,
        None,
        false,
        Some(Connector::Rustls(profile.tls_config()?)),
    )
    .await?;
    socket
        .send(Message::Text(
            json!({"type":"register","protocol_version":1,"endpoint":{
        "id":endpoint_id,"name":name,"kind":"airplay","online":true,"verified":false,
        "capabilities":{"http_pull":true,"playback_instance":true,"strict_sync":false,"gapless":false,"dsp":false,
            "pause":"rebuffer","seek":"rebuffer","airplay_adapter":"pyatv","bit_perfect":false}}})
            .to_string()
            .into(),
        ))
        .await?;
    let mut proxy: Option<MediaProxy> = None;
    let mut zone_id = String::new();
    let mut track_id = String::new();
    let mut core_playback_id=Value::Null;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { let _ = adapter.call("stop", json!({"device_id":device_id})).await; return Ok(()) },
            _ = interval.tick() => if !zone_id.is_empty() {
                match adapter.call("state", json!({"device_id":device_id})).await {
                    Ok(state) => socket.send(Message::Text(json!({"type":if state["status"] == "error" { "error" } else { "state" },"endpoint_id":endpoint_id,
                        "zone_id":zone_id,"track_id":track_id,"core_playback_id":core_playback_id,"status":if state["ended"].as_bool().unwrap_or(false) { json!("ended") } else { state["status"].clone() },"position":state["position"],"error":state["error"],"signal_path":{
                            "output":"AirPlay","processing":if state["fades_applied"]==true {vec!["pyatv receiver format conversion / buffering","Fade envelope"]}else{vec!["pyatv receiver format conversion / buffering"]},"bit_perfect_eligible":false},"adapter_state":state}).to_string().into())).await?,
                    Err(error) => socket.send(Message::Text(json!({"type":"error","endpoint_id":endpoint_id,"zone_id":zone_id,"track_id":track_id,"core_playback_id":core_playback_id,"error":error.to_string()}).to_string().into())).await?,
                }
            },
            message = socket.next() => {
                let text = match message.context("core closed AirPlay bridge")?? {
                    Message::Text(text) => text,
                    Message::Ping(data) => { socket.send(Message::Pong(data)).await?; continue },
                    Message::Close(_) => bail!("core closed AirPlay bridge"), _ => continue,
                };
                let value: Value = serde_json::from_str(&text)?;
                let command = value.get("command").unwrap_or(&value);
                let kind = command["type"].as_str().unwrap_or("");
                let action: Result<()> = async {
                    match kind {
                        "play" => {
                            adapter.call("stop",json!({"device_id":device_id})).await?;
                            if command["dsp"]["enabled"].as_bool().unwrap_or(false) { bail!("AirPlay adapter does not apply R-Music DSP; disable DSP for this endpoint") }
                            let source = command["url"].as_str().context("missing media ticket URL")?;
                            proxy = Some(MediaProxy::start(profile, source, None).await?);
                            zone_id = command["zone_id"].as_str().unwrap_or("").to_owned();
                            track_id = command["track"]["id"].as_str().unwrap_or("").to_owned();
                            core_playback_id=command["playback_id"].clone();
                            adapter.call("play", json!({"device_id":device_id,"url":proxy.as_ref().unwrap().url,
                                "position":command["position"].as_f64().unwrap_or(0.),"title":command["track"]["title"],
                                "artist":command["track"]["artist"],"album":command["track"]["album"],"duration":command["track"]["duration"],"transitions":command["transitions"]})).await?;
                        },
                        "pause"|"resume"|"stop" => {
                            adapter.call(kind, json!({"device_id":device_id})).await?;
                            if kind == "stop" { proxy = None; zone_id.clear(); }
                        },
                        "seek" => { adapter.call("seek",json!({"device_id":device_id,"position":command["position"]})).await?;core_playback_id=command["playback_id"].clone(); },
                        "volume" => { adapter.call("volume",json!({"device_id":device_id,"volume":command["volume"]})).await?; },
                        "dsp" => if command["dsp"]["enabled"].as_bool().unwrap_or(false) { bail!("AirPlay adapter DSP is not implemented") },
                        "registered"|"hello"|"ping"|"clock" => {},
                        _ => bail!("unsupported AirPlay control: {kind}"),
                    }
                    Ok(())
                }.await;
                if let Err(error) = action { socket.send(Message::Text(json!({"type":"error","endpoint_id":endpoint_id,"zone_id":command["zone_id"],"track_id":command["track"]["id"],"core_playback_id":command["playback_id"],"error":error.to_string()}).to_string().into())).await?; }
            }
        }
    }
}
