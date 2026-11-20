//! Private JSON-lines child-process interface. Credentials only travel on inherited stdin;
//! stdout carries correlated responses and events, never diagnostics or session tokens.
mod api;
mod api_files;
mod artwork;
mod public_metadata;
mod downloads;
mod export_file;
mod file_exports;
mod endpoint_worker;
mod outputs;
mod offline;
mod transport_input;
mod local_outputs;
mod session;
mod streaming;
mod network_route;
mod volume_defaults;
mod core_inspection;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
struct Command {
    id: String,
    operation: String,
    #[serde(default)]
    body: Value,
}
#[tokio::main]
async fn main() -> Result<()> {
    rmusic_coreaudio::install()?;
    if std::env::args().nth(1).as_deref() == Some("--devices") {
        println!(
            "{}",
            serde_json::to_string(&rmusic_audio::output::devices()?)?
        );
        return Ok(());
    }
    let directory =
        std::env::var_os("RMUSIC_MAC_DATA").context("missing isolated app data directory")?;
    let directory = std::path::PathBuf::from(directory);
    if std::env::args().any(|arg|arg=="--standalone-bridge") {
        rmusic_bridge::connection::enable_standalone_bridge();
    } else { rmusic_bridge::connection::initialize_connection_cache(&directory)?; }
    let store = rmusic_bridge::listens::Store::open(&directory)?;
    let discovery = rmusic_discovery::Discovery::new();
    let mut current: Option<session::Session> = None;
    let mut offline: Option<offline::Library> = None;

    let (responses, mut response_rx) = tokio::sync::mpsc::channel::<Value>(32);
    let writer = tokio::spawn(async move {
        let mut output = tokio::io::stdout();
        while let Some(response) = response_rx.recv().await {
            output
                .write_all(serde_json::to_string(&response)?.as_bytes())
                .await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
        Ok::<_, anyhow::Error>(())
    });
    let mut resources = tokio::task::JoinSet::<(String,Result<Value>)>::new();
    let mut active=std::collections::HashMap::<String,(tokio::task::AbortHandle,bool)>::new();
    let mut protocol_error: Option<anyhow::Error> = None;
    let (mut commands, reader) = transport_input::start(responses.clone());
    loop {
        let command=tokio::select!{
            received=commands.recv()=>match received {Some(Ok(cmd))=>cmd,Some(Err(e))=>{protocol_error=Some(e);break;},None=>break},
            joined=resources.join_next(),if !resources.is_empty()=>{
                if let Some(Ok((id,result)))=joined {
                    active.remove(&id);
                    if unauthorized(&result) {
                        resources.abort_all();active.clear();while resources.join_next().await.is_some(){}
                        if let Some(old)=current.take(){old.close().await?;offline::forget(&directory)?;}
                    }
                    responses.send(response(&id,result)).await?;
                }
                continue;
            }
        };
        if command.operation=="cancel" {
            if let Some(id)=command.body["id"].as_str(){if let Some((task,_))=active.remove(id){task.abort();}}
            continue;
        }
        if matches!(command.operation.as_str(),"artwork"|"install_plugin"|"request"|"downloads"|"save_api_file"|"import_account_file"|"inspect_core"|"connection_metrics") {
            let priority=command.operation=="request"&&command.body["path"].as_str().is_some_and(|p|p.starts_with("/zones")||p.starts_with("/endpoints")||p.starts_with("/sync/starts"));
            if active.values().filter(|(_,class)|*class==priority).count()>=8||active.contains_key(&command.id) {
                responses.send(json!({"id":command.id,"error":"请求繁忙，请稍后重试","status":429})).await?;
                continue;
            }
            let task=if command.operation=="inspect_core" {core_inspection::task(command.body)}else{current.as_ref().context("请先连接 Core").and_then(|session|match command.operation.as_str(){
                "artwork"=>session.artwork_task(command.body),
                "connection_metrics"=>session.connection_metrics_task(command.body),
                "downloads"=>session.downloads_task(command.body),
                "save_api_file"=>session.save_file_task(command.body),
                "import_account_file"=>session.import_account_task(command.body),
                "install_plugin"=>session.install_plugin_task(command.body),
                _=>session.request_task(command.body),
            })};
            let id=command.id;let task_id=id.clone();
            let handle=resources.spawn(async move{let result=match task{Ok(task)=>task.await,Err(error)=>Err(error)};(task_id,result)});
            active.insert(id,(handle,priority));continue;
        }
        if matches!(
            command.operation.as_str(),
            "connect" | "disconnect" | "quit"
        ) {
            resources.abort_all();active.clear();
            while resources.join_next().await.is_some() {}
        }
        let result: Result<Value> = async {
            match command.operation.as_str() {
                "discover" => match &discovery {Ok(discovery)=>Ok(json!({"items":discovery.snapshot().into_iter().filter(|s|s.kind=="core").collect::<Vec<_>>()})),Err(error)=>bail!("局域网发现不可用：{error}")},
                "local_outputs" => local_outputs::LocalOutputs::open(&directory)?.snapshot(command.body["host_id"].as_str().unwrap_or("")),
                "local_configure" => local_outputs::LocalOutputs::open(&directory)?.configure(command.body["host_id"].as_str().unwrap_or(""),&command.body),
                "stream_control" => current.as_mut().context("请登录后播放音乐")?.outputs.stream_control(command.body).await,
                "outputs"=> current.as_mut().context("请先连接 Core")?.outputs.refresh().await,
                "suspend_outputs" => Ok(current.as_mut().context("请先连接 Core")?.outputs.suspend().await),
                "resume_outputs" => current.as_mut().context("请先连接 Core")?.outputs.resume().await,
                "set_device_mute"=>current.as_mut().context("请先连接 Core")?.outputs.set_device_mute(command.body),
                "set_device_volume"=> current.as_mut().context("请先连接 Core")?.outputs.set_device_volume(command.body).await,
                "configure_output"=> current.as_mut().context("请先连接 Core")?.outputs.configure(command.body).await,
                "restore_outputs"=> current.as_mut().context("请先连接 Core")?.outputs.restore().await,
                "devices" => Ok(serde_json::to_value(rmusic_audio::output::devices()?)?),
                "playback_observation" => Ok(current.as_ref().context("请先连接 Core")?.outputs.observation(&command.body)),
                "device_volume_state" => Ok(current.as_ref().context("请先连接 Core")?.outputs.device_volume_state(&command.body)),
                "offline_list" if current.is_none() && !rmusic_bridge::connection::standalone_bridge()=>{
                    if offline.is_none(){offline=offline::Library::open(&directory)?;}
                    match &offline {Some(library)=>library.list(command.body["offset"].as_u64().unwrap_or(0) as usize),None=>Ok(Value::Null)}
                }
                "offline_status"=>Ok(offline.as_ref().map(|l|l.status()).unwrap_or(Value::Null)),
                "offline_control"=>offline.as_mut().context("请先打开离线音乐")?.control(command.body).await,
                "offline_close"=>{if let Some(mut library)=offline.take(){library.stop().await;}Ok(Value::Null)}
                "connect" => {
                    if let Some(mut library)=offline.take(){library.stop().await;}
                    if let Some(old) = current.take() {
                        old.close().await?;
                    }
                    let include_session=command.body["persist_session"]==true;
                    let connected =
                        session::Session::connect(command.body, store.clone(), &directory,responses.clone()).await.inspect_err(|error|eprintln!("Core connection failed: {error:#}"))?;
                    let info = connected.info(include_session);
                    if !rmusic_bridge::connection::standalone_bridge() {offline::remember(&directory,&info)?;}
                    current = Some(connected);
                    Ok(info)
                }
                "download_files"=>current.as_ref().context("请先连接 Core")?.exports.enqueue(command.body),
                "change_file_download"=>current.as_ref().context("请先连接 Core")?.exports.change(command.body),
                "download" => current.as_ref().context("请先连接 Core")?.downloads.start(
                    command.body["track_id"]
                        .as_str()
                        .context("missing track")?
                        .into(),
                ),
                "cancel_download" => {
                    current
                        .as_ref()
                        .context("请先连接 Core")?
                        .downloads
                        .cancel(command.body["track_id"].as_str().context("missing track")?)
                        .await?;
                    Ok(Value::Null)
                }
                "remove_download" => {
                    current
                        .as_ref()
                        .context("请先连接 Core")?
                        .downloads
                        .remove(
                            command.body["id"]
                                .as_str()
                                .context("missing cache identity")?,
                        )?;
                    Ok(Value::Null)
                }
                "cache_limit" => {
                    current
                        .as_ref()
                        .context("请先连接 Core")?
                        .downloads
                        .limit(command.body["bytes"].as_u64().context("missing bytes")?)?;
                    Ok(Value::Null)
                }
                "metadata_policy" => current.as_ref().context("请先连接 Core")?.configure_metadata(command.body),
                "streaming_policy" => current.as_ref().context("请先连接 Core")?.configure_streaming(command.body),
                "ui_visibility" => Ok(current.as_mut().context("请先连接 Core")?.set_ui_visible(command.body["visible"].as_bool().unwrap_or(false))),
                "status" => Ok(current.as_ref().context("请先连接 Core")?.status()),
                "clear_error" => {
                    current.as_ref().context("请先连接 Core")?.clear_error(command.body["id"].as_str());
                    Ok(Value::Null)
                }
                "disconnect" | "quit" => {
                    if command.operation=="disconnect" {offline::forget(&directory)?;}
                    if let Some(mut library)=offline.take(){library.stop().await;}
                    if let Some(old) = current.take() {
                        if command.operation == "disconnect" { old.logout().await?; } else { old.close().await?; }
                    }
                    Ok(Value::Null)
                }
                _ => bail!("unknown runtime operation"),
            }
        }
        .await;
        if unauthorized(&result) {
            resources.abort_all();active.clear();while resources.join_next().await.is_some(){}
            if let Some(old)=current.take(){old.close().await?;offline::forget(&directory)?;}
        }
        responses.send(response(&command.id,result)).await?;
        if command.operation == "quit" {
            break;
        }
    }
    resources.abort_all();
    while resources.join_next().await.is_some() {}
    if let Some(mut library)=offline.take(){library.stop().await;}
    if let Some(old) = current {
        old.close().await?;
    }
    tokio::task::spawn_blocking(move || store.drain()).await??;
    reader.abort();let _=reader.await;
    drop(responses);
    writer.await??;
    if let Some(error) = protocol_error {
        return Err(error);
    }
    Ok(())
}

fn unauthorized(result:&Result<Value>)->bool {
    result.as_ref().err().and_then(|error|error.downcast_ref::<session::HttpFailure>()).is_some_and(|e|e.status==401)
}
fn response(id:&str,result:Result<Value>)->Value {
    match result {Ok(body)=>json!({"id":id,"result":body}),Err(error)=>{
        let network=error.downcast_ref::<reqwest::Error>().is_some_and(|e|e.is_connect()||e.is_timeout()||e.is_request());
        let status=error.downcast_ref::<session::HttpFailure>().map(|e|e.status).or_else(||error.downcast_ref::<reqwest::Error>().and_then(|e|e.status()).map(|s|s.as_u16())).or(network.then_some(503));
        let message=if network {"暂时无法连接 Core，正在自动重试；已保留当前曲库和播放位置。".into()}else{format!("{error:#}")};
        json!({"id":id,"error":message,"status":status})
    }}
}
