//! Native endpoint control and output shared by the standalone bridge and embedded clients.
use anyhow::{bail,Context,Result};
use futures_util::{SinkExt,StreamExt};
use rmusic_audio::output::{self,OutputConfig};
use crate::{connection::ConnectionProfile,listens::{Store as ListenStore,sync::Sync as ListenSync},playback::Playback};
use serde_json::{json,Value};
use std::{sync::Arc,time::Duration};
use tokio_tungstenite::{connect_async_tls_with_config,tungstenite::{client::IntoClientRequest,Message},Connector};
#[cfg(test)]
#[path="native_tests.rs"]
mod tests;

#[derive(Default)]
struct StateReports {
    previous: Option<(Value, std::time::Instant)>,
}

impl StateReports {
    fn due(&mut self, report: &Value, now: std::time::Instant) -> bool {
        if report["type"] != "state" || report["status"] == "ended" {
            return true;
        }
        let unchanged = self.previous.as_ref().is_some_and(|(previous, sent)| {
            now.duration_since(*sent) < Duration::from_millis(250)
                && ["zone_id", "core_playback_id", "track_id", "status", "continuity_lost", "sample_rate"]
                    .iter().all(|&field| previous[field] == report[field])
                && previous["signal_path"] == report["signal_path"]
                && previous["frames"].as_u64() <= report["frames"].as_u64()
        });
        if !unchanged {
            self.previous = Some((report.clone(), now));
        }
        !unchanged
    }
}

/// Application arbitration for a group-only embedded endpoint. Ordinary commands return to IPC.
pub trait EmbeddedHost:Send+Sync {
    fn handles_ordinary(&self)->bool {false}
    fn removed(&self)->futures_util::future::BoxFuture<'_,Result<()>> {Box::pin(async{Ok(())})}
    fn prepare<'a>(&'a self,command:&'a Value)->futures_util::future::BoxFuture<'a,Result<()>>;
    fn report(&self,report:&Value);
    fn released(&self);
}

#[derive(Default)]
pub struct SessionOptions {
    pub observation: Option<Arc<crate::observation::PlaybackObservation>>,
    pub media: Option<Arc<dyn crate::media_proxy::MediaResolver>>,
    pub device_volume: Option<Arc<dyn Fn(Option<f64>)->Result<Value> + Send + Sync>>,
    pub report: Option<Arc<dyn Fn(&Value) + Send + Sync>>,
    /// Persist the measured cursor on a joined app shutdown. Explicit stop still retires it.
    pub preserve_cursor: bool,
    /// Apply an explicit local device/exclusive choice after inventory registration.
    pub configure_output: bool,
    /// Host already registered all physical outputs in one inventory.
    pub inventory_managed: bool,
}
pub async fn session_with_options(
    profile:&ConnectionProfile,token:&str,endpoint_id:&str,name:&str,output:OutputConfig,
    listens:Option<Arc<ListenStore>>,shutdown:impl std::future::Future<Output=()>,options:SessionOptions,
)->Result<()> {
    run(profile,token,endpoint_id,name,output,listens,shutdown,None,options).await
}

/// Run one authenticated native endpoint connection. The caller owns reconnection and signals.
/// Resolve `shutdown` and await this function to cancel preparation and join the output worker.
/// Dropping or aborting the future is not a joined shutdown. No device is opened before a play command.
pub async fn session(
    profile: &ConnectionProfile,
    token: &str,
    endpoint_id: &str,
    name: &str,
    output: OutputConfig,
    listens:Option<Arc<ListenStore>>,
    shutdown:impl std::future::Future<Output=()>,
) -> Result<()> {
    run(profile,token,endpoint_id,name,output,listens,shutdown,None,SessionOptions::default()).await
}

/// The desktop retains its existing cached single-zone player. Only scheduled group commands
/// reach this output worker; an ordinary command joins group output before returning ownership.
pub async fn embedded_session(
    profile:&ConnectionProfile,token:&str,endpoint_id:&str,name:&str,output:OutputConfig,
    listens:Option<Arc<ListenStore>>,shutdown:impl std::future::Future<Output=()>,host:&dyn EmbeddedHost,
)->Result<()> {
    run(profile,token,endpoint_id,name,output,listens,shutdown,Some(host),SessionOptions::default()).await
}
async fn run(
    profile:&ConnectionProfile,token:&str,endpoint_id:&str,name:&str,mut output:OutputConfig,
    listens:Option<Arc<ListenStore>>,shutdown:impl std::future::Future<Output=()>,host:Option<&dyn EmbeddedHost>,options:SessionOptions,
)->Result<()> {
    tokio::pin!(shutdown);
    if host.is_none()&&!options.inventory_managed {let config=rmusic_bridge::sequence::Config::new(profile.clone(),token.into(),endpoint_id.into())?;
        let selected=tokio::select!{result=config.register_output(name,&output)=>result?,_=&mut shutdown=>return Ok(())};
        let Some(mut selected)=selected else{return Ok(());};
        if options.configure_output && (selected.device_id!=output.device_id || selected.exclusive!=output.exclusive) {
            selected.device_id=output.device_id.clone();selected.exclusive=output.exclusive;
            let mut body=serde_json::to_value(&selected)?;body["enabled"]=json!(true);
            profile.http()?.put(profile.endpoint(&format!("api/v1/outputs/{endpoint_id}/config"))?).bearer_auth(token)
                .timeout(Duration::from_secs(5)).json(&body).send().await?.error_for_status()?;
        }
        output=selected;
    }
    let (mut socket,listener,recovery_enabled,sync) = tokio::select! {
        result=async {
    let config=rmusic_bridge::sequence::Config::new(profile.clone(),token.into(),endpoint_id.into())?;
    let listener=if listens.is_some(){config.listener().await?}else{None};
    let recovery_enabled=if let Some(owner)=listener.as_ref().filter(|_|host.is_none_or(|h|h.handles_ordinary())) {config.recovery_supported(owner).await?}else{false};
    let sync=if let (Some(store),Some(owner))=(&listens,&listener){
        store.remember(&profile.id,owner)?;Some(ListenSync::new(store.clone(),config.clone(),owner.clone()))
    }else{None};
    let mut url = profile.endpoint("api/v1/bridge/events")?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("invalid websocket scheme"))?;
    let mut request = url.as_str().into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse()?);
    let connector = Connector::Rustls(profile.tls_config()?);
    let (socket, _) =
        connect_async_tls_with_config(request, None, false, Some(connector)).await?;
            Ok::<_,anyhow::Error>((socket,listener,recovery_enabled,sync))
        } => result?,
        _=&mut shutdown => return Ok(()),
    };
    #[cfg(not(target_os="macos"))] let group_supported=cfg!(any(windows,target_os="linux"));
    #[cfg(target_os="macos")] let group_supported=output::external::scheduled_available();
    let mut registration=json!({"type":"register","protocol_version":1,"endpoint":{
        "id":endpoint_id,"name":name,"kind":output.backend(),"online":true,"verified":false,"output_config":output,
        "capabilities":{"http_pull":true,"visualization_v1":true,"playback_instance":true,"continuous_pcm":true,"core_pcm_dsp_v1":rmusic_bridge::connection::standalone_bridge(),"continuous_queue_refresh":host.is_none(),"measured_listens_v2":true,"measured_listens_recovery_v2":recovery_enabled,"continuous_recovery":recovery_enabled,"recovery_controls":recovery_enabled,"media_url_api_relative":true,"pcm_frame_v1":false,"exclusive":output.exclusive,
            "conversion_locations":rmusic_bridge::sequence::Config::conversion_locations(),"device_selection":true,"dsp":true,"host_clock_v1":true,"device_clock_v1":true,"programme_clock_v1":group_supported,"scheduled_start_v1":group_supported,"scheduled_queue_v1":group_supported,"group_output_v2":group_supported,"strict_sync":false,"gapless":false,"platform":std::env::consts::OS,
            "buffer_ms":output.buffer_ms}}});
    #[cfg(any(windows,target_os="linux"))] {registration["endpoint"]["capabilities"]["group_power_v1"]=json!(output::power::group_stamp()?.is_some());}
    registration["endpoint"]["capabilities"]["group_output_final_v2"]=json!(listener.is_some()&&listens.is_some()&&group_supported);
    registration["endpoint"]["capabilities"]["group_volume_v1"]=json!(group_supported);
    registration["endpoint"]["capabilities"]["programme_clock_v2"]=json!(group_supported&&output::PlaybackSequence::rate_adjustment_supported());
    registration["endpoint"]["capabilities"]["pcm_rate_adjustment_v1"]=json!(group_supported&&Playback::rate_adjustment_supported()&&output.android_mode!=Some(output::AndroidMode::UsbBitperfect));
    registration["endpoint"]["capabilities"]["group_rate_lease_v1"]=json!(group_supported&&Playback::rate_adjustment_supported()&&output.android_mode!=Some(output::AndroidMode::UsbBitperfect));
    if let Some(host)=host {
        let caps=&mut registration["endpoint"]["capabilities"];
        caps["native_group_v1"]=json!(!host.handles_ordinary());
        if !host.handles_ordinary(){caps["continuous_recovery"]=json!(true);caps["recovery_controls"]=json!(true);}
        host.report(&json!({"type":"endpoint","endpoint":registration["endpoint"]}));
    }
    let mut last_device_volume=Value::Null;
    if let Some(callback)=options.device_volume.as_ref(){if let Ok(state)=callback(None){registration["endpoint"]["device_volume"]=state.clone();last_device_volume=state;}}
    let mut volume_polled=std::time::Instant::now();
    socket.send(Message::Text(registration.to_string().into())).await?;
    eprintln!("Bridge connected: {name} ({endpoint_id})");
    let (reports, mut report_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let mut playback = Playback::connected(profile.clone(),token.into(),endpoint_id.into())?;
    if let (Some(store),Some(owner))=(&listens,&listener){playback.attach_journal(store.clone(),owner.clone());}
    playback.enable_recovery(recovery_enabled);
    playback.attach_media(options.media.clone());
    let upload=sync.as_ref().map(|sync|sync.schedule());
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut presentation = tokio::time::interval(Duration::from_millis(50));
    presentation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut remote_visual_until=std::time::Instant::now();
    let host_clock_id=uuid::Uuid::new_v4().to_string();
    let mut recovery_attempts=std::collections::HashMap::new();
    let mut state_reports = StateReports::default();
    let result: Result<()> = async {
        loop {
            tokio::select! {
                _ = &mut shutdown => return Ok(()),
                _ = presentation.tick(), if options.observation.as_ref().is_some_and(|v|v.active()) => {
                    if let Some(observer)=&options.observation {
                        observer.publish(playback.running().map(|run|run.observation(observer.audio())).unwrap_or_else(||json!({"frames":[]})));
                    }
                },
                _ = interval.tick() => {
                    if volume_polled.elapsed()>=Duration::from_millis(250){
                        volume_polled=std::time::Instant::now();
                        if let Some(callback)=options.device_volume.as_ref(){if let Ok(state)=callback(None){if state!=last_device_volume {
                            socket.send(Message::Text(json!({"type":"device_volume","endpoint_id":endpoint_id,"state":state}).to_string().into())).await?;last_device_volume=state;
                        }}}
                    }
                    if let Err(error)=playback.check_rate_lease(){eprintln!("Automatic rate lease reset failed: {error:#}");}
                    if let Some(mut report)=playback.start_report(){report["endpoint_id"]=json!(endpoint_id);if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}socket.send(Message::Text(report.to_string().into())).await?;}
                    if std::time::Instant::now()<remote_visual_until {if let Some(current)=playback.running() { if let Some(mut report)=current.visualization_report() { report["endpoint_id"]=json!(endpoint_id);socket.send(Message::Text(report.to_string().into())).await?; } }}
                    if let (Some(current),Some(now)) = (playback.running(),output::clock::host_ns()) {
                        let mut report=current.clock(endpoint_id, Duration::from_nanos(now));report["host_clock_basis"]=json!(output::clock::host_basis());report["host_clock_id"]=json!(host_clock_id);
                        if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}socket.send(Message::Text(report.to_string().into())).await?;
                    }
                },
                report = report_rx.recv() => if let Some(mut report) = report {
                    if playback.accepts(&report) {
                        let scheduled = playback.running().is_some_and(|current| current.scheduled());
                        if !scheduled && !state_reports.due(&report, std::time::Instant::now()) {
                            continue;
                        }
                        if report["type"] == "state" && matches!(report["status"].as_str(), Some("playing" | "paused")) {
                            if let Some(current) = playback.running() {
                                let (fraction, until, filling) = current.buffered(&report);
                                report["buffered_fraction"] = json!(fraction);
                                report["buffered_until"] = json!(until);
                                report["filling_current"] = json!(filling);
                            }
                        }
                        let recover=report["type"]=="error"&&report["recoverable"]==true;
                        if report["type"]=="error"&&!recover {playback.close_cursors();}
                        let automatic = if recover && report["retryable"]==true && report["system_suspend"].is_null() && sync.is_some() {
                            let key=(report["zone_id"].as_str().unwrap_or("").to_owned(),report["track_id"].as_str().unwrap_or("").to_owned());
                            let now=std::time::Instant::now();
                            recovery_attempts.retain(|_,(at,_)|now.duration_since(*at)<Duration::from_secs(60));
                            let (at,attempt)=recovery_attempts.entry(key).or_insert((now,0u32));
                            *at=now;
                            if *attempt<3 {
                                let delay=Duration::from_millis(750u64 << *attempt);
                                *attempt+=1;
                                report["automatic_recovery"]=json!(true);
                                Some(delay)
                            }else{None}
                        }else{None};
                        report["endpoint_id"] = json!(endpoint_id);
                        if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}socket.send(Message::Text(report.to_string().into())).await?;
                        if recover {
                            if let Some(delay)=automatic {
                                playback.finish_source_failure(&report).await;
                                if let Some(sync)=&sync {playback.begin_recovery_after(sync.clone(),delay);}
                            }else if report["source_failed"]==true {playback.finish_source_failure(&report).await;}
                            else {bail!("continuous control recovery requires a fresh registration");}
                        }
                    }
                },
                report = playback.update() => {
                    let started = playback.start_ready(&output, &reports);
                    if let Some(mut report) = report.or(started) {
                        report["endpoint_id"] = json!(endpoint_id);
                        if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}socket.send(Message::Text(report.to_string().into())).await?;
                    }
                },
                message = socket.next() => {
                    let received=output::clock::host_ns();
                    let message = message.context("core closed bridge connection")??;
                    let text = match message {
                        Message::Text(text) => text,
                        Message::Ping(data) => { socket.send(Message::Pong(data)).await?; continue },
                        Message::Close(_) => bail!("core closed bridge connection"),
                        _ => continue,
                    };
                    let value: Value = match serde_json::from_str(&text) {
                        Ok(value) => value,
                        Err(error) => { socket.send(Message::Text(json!({"type":"error","error":format!("invalid control JSON: {error}")}).to_string().into())).await?; continue },
                    };
                    if value["type"]=="clock_probe" {
                        if let Some(mut reply)=received.zip(output::clock::host_ns()).and_then(|(a,b)|rmusic_bridge::clock::reply(&value,endpoint_id,&host_clock_id,a,b)) {
                            #[cfg(any(windows,target_os="linux"))] if let Some(stamp)=output::power::group_stamp()?{reply["group_power"]=json!(stamp);}
                            playback.clock_identity(value["core_clock_id"].as_str().unwrap(),&host_clock_id);socket.send(Message::Text(reply.to_string().into())).await?;
                        }
                        continue;
                    }
                    let command = value.get("command").unwrap_or(&value);
                    let kind = command["type"].as_str().unwrap_or("");
                    if kind=="visualization_lease" {let milliseconds=command["milliseconds"].as_u64().unwrap_or(0).min(5000);remote_visual_until=std::time::Instant::now()+Duration::from_millis(milliseconds);if milliseconds>0 {if let Some(current)=playback.running() { current.visualization_lease(milliseconds); }} continue; }
                    if kind=="device_volume" {
                        let result=if let Some(callback)=options.device_volume.as_ref(){callback(command["value"].as_f64())}else{Err(anyhow::anyhow!("设备未声明音量控制"))};
                        let mut report=json!({"type":"device_volume","endpoint_id":endpoint_id,"request_id":command["request_id"]});
                        match result{Ok(state)=>report["state"]=state,Err(error)=>report["error"]=json!(error.to_string())};
                        socket.send(Message::Text(report.to_string().into())).await?;continue;
                    }
                    if kind=="loudness" {playback.command(kind,command,profile)?;continue;}
                    if kind=="output_config" {
                        if options.inventory_managed&&command["config"]["enabled"]==false {return Ok(());}
                        if host.is_none() {let next:OutputConfig=serde_json::from_value(command["config"].clone())?;next.limits.validate()?;
                            if options.inventory_managed&&(next.device_id!=output.device_id||next.dsd_output!=output::DsdOutput::Disabled) {bail!("managed output identity or PCM contract changed");}
                            playback.volume_limits(&next.limits)?;output=next.normalized();}
                        continue;
                    }
                    if kind=="remove_endpoint" {playback.shutdown().await;if let Some(host)=host{host.released();host.removed().await?;}return Ok(());}
                    if kind=="registered" {if let Some(callback)=&options.report{callback(command);}}
                    if let Some(host)=host {
                        if kind=="registered"&&command["endpoint_id"]==endpoint_id {host.report(command);if !host.handles_ordinary(){continue;}}
                        if !host.handles_ordinary()&&!matches!(kind,"prepare_start"|"commit_start"|"cancel_start"|"group_volume"|"group_rate") {
                            playback.shutdown().await;
                            while let Ok(mut report)=report_rx.try_recv(){if playback.accepts(&report){report["endpoint_id"]=json!(endpoint_id);host.report(&report);socket.send(Message::Text(report.to_string().into())).await?;}}
                            if let Some(mut report)=playback.start_report(){report["endpoint_id"]=json!(endpoint_id);host.report(&report);socket.send(Message::Text(report.to_string().into())).await?;}
                            host.released();continue;
                        }
                    }
                    if kind=="registered"&&command["endpoint_id"]==endpoint_id {if let Some(sync)=&sync{playback.begin_recovery(sync.clone());}}
                    if matches!(kind,"play"|"resume")&&command["playback_id"].is_null()&&recovery_enabled {if let Some(sync)=&sync {playback.request_recovery(sync.clone());}continue;}
                    let action = async {
                        let fresh=kind=="prepare_start"&&!playback.duplicate_start(command)?;
                        // Validate and stage first; start_ready cannot open an output until the
                        // local player has joined. Replays never take desktop ownership again.
                        playback.command(kind,command,profile)?;
                        if fresh {
                            let admission=async {playback.admit_start(command).await?;if let Some(host)=host{host.prepare(command).await?;}Ok::<(),anyhow::Error>(())}.await;
                            if let Err(error)=admission {playback.shutdown().await;if let Some(host)=host{host.released();}return Err(error);}
                        }
                        Ok::<(),anyhow::Error>(())
                    }.await;
                    if let Err(error) = action {
                        if matches!(kind,"group_volume"|"group_rate"){socket.send(Message::Text(json!({"type":if kind=="group_rate"{"group_rate_error"}else{"group_volume_error"},"zone_id":command["zone_id"],"core_playback_id":command["playback_id"],"start":command["start"],"error":error.to_string()}).to_string().into())).await?;continue;}
                        if matches!(kind,"prepare_start"|"commit_start"|"cancel_start"){let mut report=Playback::start_error(command,error);report["endpoint_id"]=json!(endpoint_id);if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}socket.send(Message::Text(report.to_string().into())).await?;continue;}
                        socket.send(Message::Text(json!({"type":"error","endpoint_id":endpoint_id,
                            "zone_id":command["zone_id"],"track_id":command["track"]["id"],"core_playback_id":command["playback_id"],"error":error.to_string()}).to_string().into())).await?;
                    }
                    if let Some(mut report)=playback.start_report(){report["endpoint_id"]=json!(endpoint_id);if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}socket.send(Message::Text(report.to_string().into())).await?;}
                    if let Some(mut report) = playback.start_ready(&output, &reports) {
                        report["endpoint_id"] = json!(endpoint_id);
                        if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}socket.send(Message::Text(report.to_string().into())).await?;
                    }
                }
            }
        }
    }.await;
    playback.shutdown().await;
    while let Ok(mut report)=report_rx.try_recv(){if playback.accepts(&report){report["endpoint_id"]=json!(endpoint_id);if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}let _=socket.send(Message::Text(report.to_string().into())).await;}}
    if let Some(mut report)=playback.start_report(){report["endpoint_id"]=json!(endpoint_id);if let Some(callback)=&options.report{callback(&report);}if let Some(host)=host{host.report(&report);}let _=socket.send(Message::Text(report.to_string().into())).await;}
    if let Some(host)=host{host.released();}
    if result.is_ok()&&host.is_none()&&!options.preserve_cursor {if let (Some(store),Some(owner))=(&listens,&listener){rmusic_bridge::listens::cursor::close(store,owner,endpoint_id)?;}}
    drop(upload);
    if let Some(store)=listens {tokio::task::spawn_blocking(move ||store.drain()).await??;}
    if let Some(sync)=sync {if let Err(error)=sync.flush().await {eprintln!("Final listening upload deferred: {error:#}");}}
    result
}
