use anyhow::{bail, Context, Result};
use clap::Parser;
use rmusic_audio::output::{self, OutputConfig};
use rmusic_bridge::connection::ConnectionProfile;
use rmusic_bridge::listens::{Store as ListenStore,sync as listen_sync};
use serde_json::json;
#[cfg(test)]
use serde_json::Value;
use std::{
    sync::Arc,
    time::Duration,
};

#[cfg(test)]
mod scheduled_start_tests;

const NATIVE_KIND:&str=if cfg!(windows){"wasapi"}else if cfg!(target_os="linux"){"alsa"}else{"unsupported"};
const DEFAULT_NAME:&str=if cfg!(windows){"Windows 本机"}else if cfg!(target_os="linux"){"Linux 本机"}else{"本机网桥"};
async fn shutdown_signal() {
    #[cfg(unix)] {let mut terminate=tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("cannot install SIGTERM handler");
        tokio::select!{_=terminate.recv()=>{},_=tokio::signal::ctrl_c()=>{}}}
    #[cfg(not(unix))] {let _=tokio::signal::ctrl_c().await;}
}

#[derive(Parser)]
#[command(about = "R-Music host bridge: authenticated HTTP pull and native PCM output")]
struct Args {
    #[arg(long, env = "RMUSIC_CORE")]
    core: Option<String>,
    /// Additional on-link LAN addresses for this same Core.
    #[arg(long)]
    core_fallback: Vec<String>,
    #[arg(long)]
    core_id: Option<String>,
    /// Discover nearby cores and bridges without authentication.
    #[arg(long)]
    discover: bool,
    /// Read the scoped API token from a local file; do not put secrets in command arguments.
    #[arg(long)]
    token_file: Option<std::path::PathBuf>,
    /// Durable per-endpoint listening journal. Defaults to the user's local data directory.
    #[arg(long,env="RMUSIC_BRIDGE_DATA_DIR")]
    data_dir:Option<std::path::PathBuf>,
    #[arg(long)]
    certificate_sha256: Option<String>,
    #[arg(long)]
    device: Option<String>,
    #[arg(long)]
    exclusive: bool,
    #[arg(long, default_value_t = 100)]
    buffer_ms: u32,
    /// Explicit raw DSD transport; native requires compatible ASIO or Linux ALSA hw: output.
    #[arg(long,default_value="disabled",value_parser=["disabled","dop","native"])]
    dsd_output:String,
    #[arg(long, default_value = DEFAULT_NAME)]
    name: String,
    #[arg(long)]
    endpoint_id: Option<String>,
    #[arg(long)]
    devices: bool,
    /// Print compiled audio features without opening devices or connecting to a core.
    #[arg(long)]
    build_info: bool,
    #[arg(long)]
    airplay_adapter: Option<std::path::PathBuf>,
    #[arg(long)]
    airplay_device: Option<String>,
    #[arg(long)]
    airplay_discover: bool,
    /// Pair a selected AirPlay device with the PIN displayed by the receiver.
    #[arg(long)]
    airplay_pair: Option<String>,
    /// HQPlayer Desktop or Embedded hostname/IP; this mode never opens a DAC itself.
    #[arg(long,conflicts_with_all=["airplay_device","airplay_pair","airplay_discover","devices"])]
    hqplayer: Option<String>,
    #[arg(long,default_value_t=4321)]
    hqplayer_port:u16,
    /// Read HQPlayer status without changing playback or needing a core token.
    #[arg(long,requires="hqplayer")]
    hqplayer_status:bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.build_info {println!("{}",output::build_info());return Ok(());}
    let hqplayer=args.hqplayer.as_deref().map(|host|rmusic_bridge::hqplayer::Target::new(host,args.hqplayer_port)).transpose()?;
    if args.hqplayer_status{println!("{}",serde_json::to_string_pretty(&rmusic_bridge::hqplayer::bridge::inspect(hqplayer.as_ref().unwrap()).await?)?);return Ok(());}
    if args.discover {
        let discovery=rmusic_discovery::Discovery::new()?;
        tokio::time::sleep(Duration::from_secs(4)).await;
        println!("{}",serde_json::to_string_pretty(&discovery.snapshot())?);return Ok(());
    }
    if args.devices {
        println!("{}", serde_json::to_string_pretty(&output::devices()?)?);
        return Ok(());
    }
    if args.airplay_discover || args.airplay_pair.is_some() {
        let path = args
            .airplay_adapter
            .as_deref()
            .context("--airplay-adapter path is required")?;
        let mut adapter = rmusic_bridge::airplay::AirPlayAdapter::start(path).await?;
        let devices = adapter.call("discover", json!({"timeout":5})).await?;
        if args.airplay_discover {
            println!("{}", serde_json::to_string_pretty(&devices)?);
        }
        if let Some(pin) = &args.airplay_pair {
            let device_id = args
                .airplay_device
                .as_deref()
                .context("--airplay-device ID is required")?;
            let session = adapter
                .call("pair_start", json!({"device_id":device_id}))
                .await?;
            let paired = adapter
                .call(
                    "pair_finish",
                    json!({"session_id":session["session_id"],"pin":pin}),
                )
                .await?;
            println!("{}", serde_json::to_string_pretty(&paired)?);
        }
        return Ok(());
    }
    rmusic_bridge::connection::enable_standalone_bridge();
    let automatic=args.token_file.is_none()&&std::env::var_os("RMUSIC_TOKEN").is_none();
    let mut token = match &args.token_file {
        Some(path) => std::fs::read_to_string(path).context("cannot read token file")?,
        None => std::env::var("RMUSIC_TOKEN").unwrap_or_default(),
    };
    token = token.trim().to_owned();
    if token.is_empty()&&!automatic {
        bail!("empty API token")
    }
    let mut profile = ConnectionProfile {core_public_key:None,
        id: "bridge".into(),
        name: args.name.clone(),
        address: args.core.clone().unwrap_or_else(||"http://127.0.0.1:7800".into()),
        skip_certificate_verification: false, certificate_sha256: args.certificate_sha256,
        alternatives: args.core_fallback.into_iter().map(|address|rmusic_bridge::connection::CoreAddress{from_core: Some(false), address,skip_certificate_verification: false, certificate_sha256: None}).collect(), core_id: args.core_id,
    };
    profile.base_url()?;
    if args.dsd_output=="native"&&hqplayer.is_none()&&args.airplay_device.is_none(){profile=profile.resolve_lan_only().await?;profile.require_clock(rmusic_audio::MAX_CLOCK_RATE).await?;}
    if let Some(target)=hqplayer{
        if automatic{bail!("HQPlayer 适配器暂需已有的受限 Core 凭据");}
        let name=if args.name==DEFAULT_NAME{"HQPlayer"}else{&args.name};
        return rmusic_bridge::hqplayer::bridge::run(&profile,&token,args.endpoint_id.as_deref().unwrap_or(&target.id()),name,&target).await;
    }
    if let Some(device_id) = &args.airplay_device {
        if automatic{bail!("AirPlay 适配器暂需已有的受限 Core 凭据");}
        let selected=profile.resolve_lan_only().await?;
        profile.core_id=selected.core_id.clone();
        let _announcement=rmusic_bridge::lan::Announcement::start(&format!("airplay:{device_id}"),&args.name,selected.core_id.as_deref()).await.ok();
        let path = args
            .airplay_adapter
            .as_deref()
            .context("--airplay-adapter path is required")?;
        return rmusic_bridge::airplay::run(
            &profile,
            &token,
            &args
                .endpoint_id
                .clone()
                .unwrap_or_else(|| format!("airplay:{device_id}")),
            &args.name,
            device_id,
            path,
        )
        .await;
    }
    let output_config = OutputConfig {
        limits:Default::default(),
        device_id: args.device,
        exclusive: args.exclusive,
        buffer_ms: args.buffer_ms,
        dsd_output:match args.dsd_output.as_str(){"dop"=>output::DsdOutput::Dop,"native"=>output::DsdOutput::Native,_=>output::DsdOutput::Disabled},
        android_mode:None,
    }.normalized();
    if NATIVE_KIND=="unsupported"{bail!("native PCM output is not implemented on this platform");}
    let endpoint_id = args.endpoint_id.unwrap_or_else(|| {
        // Device ID is stable; the caller can set an explicit ID for multiple bridge instances.
        format!(
            "{}:{}:{}",output_config.backend(),
            std::env::var("COMPUTERNAME").or_else(|_|std::env::var("HOSTNAME")).unwrap_or_else(|_| "host".into()),
            output_config.device_id.as_deref().unwrap_or("default")
        )
    });
    let mut backoff = 1u64;
    let journal_root=match args.data_dir {Some(path)=>path,None=>listen_sync::default_root(&endpoint_id)?};
    let endpoint_id=if automatic{rmusic_bridge::lan_join::endpoint_id(&journal_root.join("lan-identity"))?}else{endpoint_id};
    let listens=ListenStore::open(&journal_root).context("cannot open durable bridge listening journal")?;
    #[cfg(any(windows,target_os="linux"))] {let saved=listens.clone();let (generation,held)=listens.system_suspend_state()?;output::power::configure(listens.client_id.clone(),generation,held,move|generation,held|{#[cfg(target_os="linux")]saved.drain()?;saved.save_system_suspend(generation,held)})?;}
    let automatic_discovery=if automatic{Some(rmusic_discovery::Discovery::new()?)}else{None};
    let mut announcement=if automatic{rmusic_bridge::lan::Announcement::start(&endpoint_id,&args.name,None).await.ok()}else{None};
    let mut announced_core=None;
    loop {
        let result=async {
            if let Some(discovery)=automatic_discovery.as_ref(){let(joined,credential)=rmusic_bridge::lan_join::connect(&journal_root.join("lan-identity"),&endpoint_id,&args.name,discovery,if args.core.is_some(){Some(&profile)}else{None}).await?;profile=joined;token=credential;}
            let selected=profile.resolve_lan_only().await?;
            if profile.core_id.is_none(){profile.core_id=selected.core_id.clone();}
            if announcement.is_none()||announced_core!=profile.core_id{announcement=rmusic_bridge::lan::Announcement::start(&endpoint_id,&args.name,profile.core_id.as_deref()).await.ok();announced_core=profile.core_id.clone();}
            session(
            &selected,
            &token,
            &endpoint_id,
            &args.name,
            output_config.clone(),
            Some(listens.clone()),
        )
        .await
        }.await;
        match result
        {
            Ok(()) => break,
            Err(error) => {
                eprintln!("Bridge disconnected: {error:#}; retry in {backoff}s");
                tokio::select! { _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}, _ = shutdown_signal() => break }
                backoff = (backoff * 2).min(30);
            }
        }
    }
    if let Some(owner)=listens.owner(&profile.id)?.filter(|owner|profile.core_id.as_deref()==Some(&owner.core_id)) {
        rmusic_bridge::listens::cursor::close(&listens,&owner,&endpoint_id)?;
        tokio::task::spawn_blocking(move ||listens.drain()).await??;
    }
    Ok(())
}

async fn session(
    profile:&ConnectionProfile,token:&str,endpoint_id:&str,name:&str,
    output:OutputConfig,listens:Option<Arc<ListenStore>>,
)->Result<()> {
    rmusic_bridge::native::session(profile,token,endpoint_id,name,output,listens,shutdown_signal()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::ws::{WebSocketUpgrade,Message as CoreMessage},routing::get,Router};
    fn wav()->Vec<u8> {
        let size=48000u32*2;let mut bytes=Vec::new();
        bytes.extend(b"RIFF");bytes.extend((36+size).to_le_bytes());bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());bytes.extend(1u16.to_le_bytes());bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());bytes.extend(96000u32.to_le_bytes());bytes.extend(2u16.to_le_bytes());bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");bytes.extend(size.to_le_bytes());bytes.resize(44+size as usize,0);bytes
    }
    #[tokio::test]
    async fn delayed_durable_recovery_does_not_block_stop_or_socket_ping() {
        let dir=tempfile::tempdir().unwrap();let store=ListenStore::open(dir.path()).unwrap();
        let owner=rmusic_bridge::listens::Owner {core_id:"core-a".into(),user_id:"user-a".into()};
        let run=rmusic_bridge::listens::cursor::Recorder::begin(store.clone(),owner.clone(),"test-endpoint".into(),"z".into(),uuid::Uuid::new_v4().to_string(),48000,
            rmusic_bridge::listens::cursor::Boundary {playback_id:uuid::Uuid::new_v4().to_string(),track_id:"track".into(),position:0.,frames:0}).unwrap();assert!(run.handle.ready().unwrap());
        let (accepted,mut sockets)=tokio::sync::mpsc::channel(1);let entered=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());
        let handler_entered=entered.clone();let handler_release=release.clone();
        let router=Router::new().route("/api/v1/status",get(||async{axum::Json(json!({"core_id":"core-a","capabilities":{"measured_listens":true,"continuous_recovery":true}}))}))
            .route("/api/v1/auth/me",get(||async{axum::Json(json!({"id":"user-a"}))}))
            .route("/api/v1/bridge/events",get(move |ws:WebSocketUpgrade|{let accepted=accepted.clone();async move{ws.on_upgrade(move |socket|async move{let _=accepted.send(socket).await;})}}))
            .route("/api/v1/zones/z/recover",axum::routing::post(move ||{let entered=handler_entered.clone();let release=handler_release.clone();async move{entered.notify_one();release.notified().await;axum::Json(json!({"accepted":false,"zone":{}}))}}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        let profile=ConnectionProfile {core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:Some("core-a".into())};
        let saved=store.clone();let bridge=tokio::spawn(async move{session(&profile,"qa-session","test-endpoint","test",OutputConfig::default(),Some(saved)).await});
        let mut socket=tokio::time::timeout(Duration::from_secs(5),sockets.recv()).await.unwrap().unwrap();
        assert!(matches!(socket.recv().await,Some(Ok(CoreMessage::Text(_)))));
        socket.send(CoreMessage::Text(json!({"type":"registered","endpoint_id":"test-endpoint"}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5),entered.notified()).await.unwrap();
        socket.send(CoreMessage::Text(json!({"type":"stop","zone_id":"z"}).to_string().into())).await.unwrap();socket.send(CoreMessage::Ping(vec![9,8,7].into())).await.unwrap();
        let responsive=tokio::time::timeout(Duration::from_millis(600),async {while let Some(Ok(message))=socket.recv().await{if let CoreMessage::Pong(data)=message{return data[..]==[9,8,7];}}false}).await.unwrap_or(false);
        store.drain().unwrap();assert!(rmusic_bridge::listens::cursor::freeze(store.clone(),&owner,"test-endpoint").unwrap().is_none());
        release.notify_one();let _=socket.send(CoreMessage::Close(None)).await;
        assert!(tokio::time::timeout(Duration::from_secs(5),bridge).await.unwrap().unwrap().is_err());server.abort();assert!(responsive,"recovery HTTP must not block stop or ping");
    }
    #[tokio::test]
    async fn delayed_impulse_loading_keeps_stop_and_websocket_ping_responsive_without_advertising_playback_support() {
        let (accepted,mut sockets)=tokio::sync::mpsc::channel(1);
        let entered=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());let loading=entered.clone();let response=release.clone();
        let router=Router::new().route("/api/v1/bridge/events",get(move|ws:WebSocketUpgrade|{let accepted=accepted.clone();async move{ws.on_upgrade(move|socket|async move{let _=accepted.send(socket).await;})}}))
            .route("/api/v1/media/source",get(||async{wav()}))
            .route("/api/v1/zones/z/dsp/impulses",axum::routing::post(move|headers:axum::http::HeaderMap|{let loading=loading.clone();let response=response.clone();async move{
                assert_eq!(headers["authorization"],"Bearer control-only");loading.notify_one();response.notified().await;axum::Json(json!({"items":[]}))
            }}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        let profile=ConnectionProfile{core_public_key:None,id:"test".into(),name:"test".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None};
        let bridge=tokio::spawn(async move{session(&profile,"control-only","test-endpoint","test",OutputConfig{device_id:Some("rmusic-nonexistent-qa-device".into()),..Default::default()},None).await});
        let mut socket=tokio::time::timeout(Duration::from_secs(5),sockets.recv()).await.unwrap().unwrap();
        let Some(Ok(CoreMessage::Text(registration)))=socket.recv().await else{panic!("missing registration")};let registration:Value=serde_json::from_str(&registration).unwrap();assert_ne!(registration["endpoint"]["capabilities"]["convolution_v1"],true);
        socket.send(CoreMessage::Text(json!({"type":"play","zone_id":"z","playback_id":"current","track":{"id":"source"},"url":format!("http://{address}/api/v1/media/source"),"dsp":{"enabled":true,"convolution":{"response_ids":["0".repeat(64)]}}}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5),entered.notified()).await.unwrap();socket.send(CoreMessage::Text(json!({"type":"stop","zone_id":"z"}).to_string().into())).await.unwrap();
        for ping in [vec![1,2,3],vec![4,5,6]] {
            socket.send(CoreMessage::Ping(ping.clone().into())).await.unwrap();tokio::time::timeout(Duration::from_millis(600),async{loop{match socket.recv().await.unwrap().unwrap(){
                CoreMessage::Pong(data)=>{assert_eq!(data[..],ping);break;},CoreMessage::Text(text)=>{let report:Value=serde_json::from_str(&text).unwrap();assert_ne!(report["type"],"error","{report}");},_=>{},
            }}}).await.unwrap();release.notify_one();
        }
        socket.send(CoreMessage::Close(None)).await.unwrap();assert!(tokio::time::timeout(Duration::from_secs(5),bridge).await.unwrap().unwrap().is_err());server.abort();
    }
    #[tokio::test]
    async fn delayed_source_does_not_block_stop_or_websocket_ping() {
        let (accepted,mut sockets)=tokio::sync::mpsc::channel(1);
        let entered=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());
        let source_entered=entered.clone();let source_release=release.clone();
        let router=Router::new().route("/api/v1/bridge/events",get(move|ws:WebSocketUpgrade|{
            let accepted=accepted.clone();async move {ws.on_upgrade(move|socket|async move{let _=accepted.send(socket).await;})}
        })).route("/api/v1/media/slow",get(move||{
            let entered=source_entered.clone();let release=source_release.clone();
            async move{entered.notify_one();release.notified().await;wav()}
        }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        let profile=ConnectionProfile{core_public_key:None,id:"test".into(),name:"test".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None};
        let bridge_profile=profile.clone();
        let bridge=tokio::spawn(async move{session(&bridge_profile,"test-only-token","test-endpoint","test",OutputConfig::default(),None).await});
        let mut socket=tokio::time::timeout(Duration::from_secs(5),sockets.recv()).await.unwrap().unwrap();
        assert!(matches!(socket.recv().await,Some(Ok(CoreMessage::Text(_)))));
        socket.send(CoreMessage::Text(json!({"type":"play","zone_id":"z","track":{"id":"slow"},"url":format!("http://{address}/api/v1/media/slow"),"muted":true}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5),entered.notified()).await.unwrap();
        socket.send(CoreMessage::Text(json!({"type":"stop","zone_id":"z"}).to_string().into())).await.unwrap();
        socket.send(CoreMessage::Ping(vec![1,2,3].into())).await.unwrap();
        let responsive=tokio::time::timeout(Duration::from_millis(600),async {
            while let Some(Ok(message))=socket.recv().await {if let CoreMessage::Pong(data)=message {return data[..]==[1,2,3];}}
            false
        }).await.unwrap_or(false);
        release.notify_one();let _=socket.send(CoreMessage::Close(None)).await;
        if tokio::time::timeout(Duration::from_secs(5),bridge).await.is_err(){server.abort();panic!("bridge failed to clean up");}
        server.abort();assert!(responsive,"音源预加载不应阻塞停止命令和 WebSocket 心跳");
    }
}

#[cfg(test)]
mod host_clock_tests {
    use super::*;
    #[tokio::test]
    async fn host_clock_probe_roundtrip_does_not_open_audio_and_preserves_ping() {
        use axum::{extract::ws::{WebSocketUpgrade,Message as CoreMessage},routing::get,Router};
        let(tx,mut rx)=tokio::sync::mpsc::channel(1);
        let router=Router::new().route("/api/v1/bridge/events",get(move|ws:WebSocketUpgrade|{let tx=tx.clone();async move{ws.on_upgrade(move|socket|async move{let _=tx.send(socket).await;})}}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        let profile=ConnectionProfile{core_public_key:None,id:"clock-qa".into(),name:"clock-qa".into(),address:format!("http://{address}"),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![],core_id:None};
        let bridge=tokio::spawn(async move{session(&profile,"synthetic-token","clock-qa","test",OutputConfig::default(),None).await});
        let mut socket=tokio::time::timeout(Duration::from_secs(4),rx.recv()).await.unwrap().unwrap();
        let CoreMessage::Text(register)=socket.recv().await.unwrap().unwrap() else{panic!("missing registration");};let register:Value=serde_json::from_str(&register).unwrap();assert_eq!(register["endpoint"]["capabilities"]["host_clock_v1"],true);assert_eq!(register["endpoint"]["capabilities"]["strict_sync"],false);
        let mut probe=json!({"type":"clock_probe","version":1,"endpoint_id":"wrong","core_clock_id":uuid::Uuid::new_v4().to_string(),"probe_id":uuid::Uuid::new_v4().to_string(),"t1_ns":1234});
        socket.send(CoreMessage::Text(probe.to_string().into())).await.unwrap();probe["endpoint_id"]=json!("clock-qa");socket.send(CoreMessage::Text(probe.to_string().into())).await.unwrap();
        let CoreMessage::Text(reply)=tokio::time::timeout(Duration::from_secs(1),socket.recv()).await.unwrap().unwrap().unwrap() else{panic!("missing clock reply");};let reply:Value=serde_json::from_str(&reply).unwrap();assert_eq!(reply["type"],"clock_reply");assert_eq!(reply["probe_id"],probe["probe_id"]);assert_eq!(reply["t1_ns"],1234);assert!(reply["t3_ns"].as_u64().unwrap()>=reply["t2_ns"].as_u64().unwrap());uuid::Uuid::parse_str(reply["host_clock_id"].as_str().unwrap()).unwrap();
        socket.send(CoreMessage::Ping(vec![7].into())).await.unwrap();assert!(matches!(socket.recv().await,Some(Ok(CoreMessage::Pong(_)))));
        socket.send(CoreMessage::Close(None)).await.unwrap();let _=tokio::time::timeout(Duration::from_secs(4),bridge).await.unwrap();server.abort();
    }
}
