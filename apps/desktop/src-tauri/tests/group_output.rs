//! Actual desktop manager and embedded socket, with a stalled source and no audio device.
use axum::{extract::ws::{WebSocket,WebSocketUpgrade,Message},routing::{get,post},Router};
use futures_util::{future::BoxFuture,FutureExt};
use rmusic_desktop::group_output::{Manager,LocalPlayer,Registration};
use rmusic_bridge::connection::ConnectionProfile;
use rmusic_audio::output::{self,OutputConfig};
use serde_json::{json,Value};
use std::{sync::Arc,time::Duration};
struct Local {entered:Arc<tokio::sync::Notify>,release:Arc<tokio::sync::Notify>,stopped:Arc<tokio::sync::Notify>}
impl LocalPlayer for Local {
    fn prepare<'a>(&'a self,command:&'a Value)->BoxFuture<'a,anyhow::Result<()>> {Box::pin(async move{self.entered.notify_one();self.release.notified().await;if command["qa_reject"]==true{anyhow::bail!("offline output active");}Ok(())})}
    fn stop(&self)->BoxFuture<'_,anyhow::Result<()>> {Box::pin(async move{self.stopped.notify_one();Ok(())})}
}
async fn next(socket:&mut WebSocket,kind:&str)->Value {
    tokio::time::timeout(Duration::from_secs(3),async{loop {if let Some(Ok(Message::Text(text)))=socket.recv().await {let value:Value=serde_json::from_str(&text).unwrap();if value["type"]==kind{return value;}assert_ne!(value["type"],"start_error","{value}");}}}).await.unwrap()
}
struct Fixture {manager:Arc<Manager>,socket:WebSocket,profile:ConnectionProfile,registration:String,server:tokio::task::JoinHandle<()>,sockets:tokio::sync::mpsc::Receiver<WebSocket>,local:Arc<Local>}
#[tokio::test]
async fn native_connection_reconnects_without_renderer_or_replaying_playback_and_stops_retrying_on_close() {
    let mut f=Fixture::new().await;
    f.socket.send(Message::Close(None)).await.unwrap();
    let mut socket=tokio::time::timeout(Duration::from_secs(6),f.sockets.recv()).await.unwrap().unwrap();
    let registration=next(&mut socket,"register").await;assert_eq!(registration["endpoint"]["id"],"browser-native");
    socket.send(Message::Text(json!({"type":"registered","endpoint_id":"browser-native"}).to_string().into())).await.unwrap();
    let until=tokio::time::Instant::now()+Duration::from_secs(2);
    while f.manager.endpoint("qa","browser-native").is_none(){assert!(tokio::time::Instant::now()<until);tokio::time::sleep(Duration::from_millis(10)).await;}
    assert!(!f.manager.owned());assert!(tokio::time::timeout(Duration::from_millis(100),f.local.entered.notified()).await.is_err());
    f.manager.stop(None,None).await;
    assert!(tokio::time::timeout(Duration::from_millis(750),f.sockets.recv()).await.is_err());f.server.abort();
}
impl Fixture {
    async fn new()->Self {
        let (accepted,mut sockets)=tokio::sync::mpsc::channel(4);
        let router=Router::new().route("/api/v1/endpoints/browser",post(||async{axum::Json(json!({"id":"browser-native","kind":"browser"}))}))
            .route("/api/v1/bridge/events",get(move|ws:WebSocketUpgrade|{let accepted=accepted.clone();async move{ws.on_upgrade(move|socket|async move{accepted.send(socket).await.unwrap();})}}))
            .route("/api/v1/media/held",get(||async{std::future::pending::<String>().await}))
            .route("/api/v1/sync/starts/{id}/cancel",post(||async{axum::Json(json!({"phase":"cancelled"}))}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=format!("http://{}",listener.local_addr().unwrap());let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
        let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),address,name:"QA".into(),core_id:Some("synthetic-core".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]};
        let manager=Arc::new(Manager::default());let registration=uuid::Uuid::new_v4().to_string();let local=Arc::new(Local{entered:Arc::new(tokio::sync::Notify::new()),release:Arc::new(tokio::sync::Notify::new()),stopped:Arc::new(tokio::sync::Notify::new())});
        let config=config(&profile,&registration);let saved=manager.clone();let owner=local.clone();let register=tokio::spawn(async move{saved.register(config,owner).await});
        let mut socket=sockets.recv().await.unwrap();let v=next(&mut socket,"register").await;assert_eq!(v["endpoint"]["capabilities"]["native_group_v1"],true);assert_eq!(v["endpoint"]["capabilities"]["strict_sync"],false);
        socket.send(Message::Text(json!({"type":"registered","endpoint_id":"browser-native"}).to_string().into())).await.unwrap();register.await.unwrap().unwrap();
        Self{manager,socket,profile,registration,server,sockets,local}
    }
    async fn command(&mut self)->Value {
        let core=uuid::Uuid::new_v4().to_string();self.socket.send(Message::Text(json!({"type":"clock_probe","version":1,"endpoint_id":"browser-native","core_clock_id":core,"probe_id":uuid::Uuid::new_v4().to_string(),"t1_ns":1}).to_string().into())).await.unwrap();let reply=next(&mut self.socket,"clock_reply").await;
        let command=json!({"type":"prepare_start","zone_id":"z","playback_id":uuid::Uuid::new_v4().to_string(),"track":{"id":"held"},"url":"api/v1/media/held","position":0.,"start":{"version":1,"plan_id":uuid::Uuid::new_v4().to_string(),"generation":1,"core_clock_id":core,"host_clock_id":reply["host_clock_id"],"expires_host_ns":output::clock::host_ns().unwrap()+5_000_000_000u64}});
        command
    }
    async fn prepare(&mut self)->Value {
        let command=self.command().await;self.socket.send(Message::Text(command.to_string().into())).await.unwrap();tokio::time::timeout(Duration::from_secs(3),self.local.entered.notified()).await.unwrap();command
    }
}
#[tokio::test]
async fn desktop_rejected_and_duplicate_starts_preserve_ownership_and_reconnect_never_replays() {
    let mut f=Fixture::new().await;let command=f.command().await;
    for invalid in ["host_clock_id","expires_host_ns","playback_id","url","volume","dsp"] {
        let mut stale=command.clone();match invalid {"host_clock_id"=>stale["start"][invalid]=json!("retired-host"),"expires_host_ns"=>stale["start"][invalid]=json!(1),"url"=>stale[invalid]=json!("invalid-media"),"volume"=>stale[invalid]=json!(2),"dsp"=>stale[invalid]=json!("invalid-dsp"),_=>stale[invalid]=json!("invalid-instance")}
        f.socket.send(Message::Text(stale.to_string().into())).await.unwrap();next(&mut f.socket,"start_error").await;
        assert!(!f.manager.owned(),"rejection must precede desktop takeover: {invalid}");assert!(f.local.entered.notified().now_or_never().is_none());
    }
    f.socket.send(Message::Text(command.to_string().into())).await.unwrap();tokio::time::timeout(Duration::from_secs(3),f.local.entered.notified()).await.unwrap();f.local.release.notify_one();let armed=next(&mut f.socket,"start_state").await;assert_eq!(armed["phase"],"armed");
    f.socket.send(Message::Text(command.to_string().into())).await.unwrap();assert_eq!(next(&mut f.socket,"start_state").await["start"],armed["start"]);assert!(f.local.entered.notified().now_or_never().is_none(),"duplicate must not stop the local player again");
    f.socket.send(Message::Close(None)).await.unwrap();tokio::time::timeout(Duration::from_secs(3),async{while f.manager.owned()||f.manager.endpoint("qa","browser-native").is_some(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    let manager=f.manager.clone();let config=config(&f.profile,&f.registration);let local=f.local.clone();let register=tokio::spawn(async move{manager.register(config,local).await});
    f.socket=f.sockets.recv().await.unwrap();next(&mut f.socket,"register").await;f.socket.send(Message::Text(json!({"type":"registered","endpoint_id":"browser-native"}).to_string().into())).await.unwrap();register.await.unwrap().unwrap();
    let fresh=f.command().await;assert_ne!(fresh["start"]["host_clock_id"],command["start"]["host_clock_id"]);assert!(!f.manager.owned());assert!(f.local.entered.notified().now_or_never().is_none());
    f.socket.send(Message::Text(command.to_string().into())).await.unwrap();next(&mut f.socket,"start_error").await;assert!(!f.manager.owned());
    f.socket.send(Message::Text(fresh.to_string().into())).await.unwrap();tokio::time::timeout(Duration::from_secs(3),f.local.entered.notified()).await.unwrap();f.local.release.notify_one();assert_eq!(next(&mut f.socket,"start_state").await["phase"],"armed");
    f.manager.stop(None,None).await;assert!(!f.manager.owned());f.server.abort();
}
#[tokio::test]
async fn desktop_local_rejection_discards_staged_source_before_releasing_ownership() {
    let mut f=Fixture::new().await;let mut command=f.command().await;command["qa_reject"]=json!(true);
    f.socket.send(Message::Text(command.to_string().into())).await.unwrap();tokio::time::timeout(Duration::from_secs(3),f.local.entered.notified()).await.unwrap();assert!(f.manager.owned());f.local.release.notify_one();
    assert_eq!(next(&mut f.socket,"start_error").await["error"],"offline output active");assert!(!f.manager.owned());let report=next(&mut f.socket,"start_state").await;assert_eq!(report["final"],true);assert_eq!(report["programme_frames_played"],0);
    f.manager.stop(None,None).await;f.server.abort();
}
fn config(profile:&ConnectionProfile,id:&str)->Registration {Registration{id:id.into(),profile:profile.clone(),token:"synthetic-token".into(),endpoint:"browser-native".into(),name:"QA".into(),output:OutputConfig{device_id:Some("rmusic-never-opened".into()),..Default::default()},listens:None,primary:true}}
#[tokio::test]
async fn removed_endpoint_stops_the_primary_player_and_closes_registration() {
    let mut f=Fixture::new().await;
    f.socket.send(Message::Text(json!({"type":"remove_endpoint","endpoint_id":"browser-native"}).to_string().into())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3),f.local.stopped.notified()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3),async{while f.manager.endpoint("qa","browser-native").is_some(){tokio::task::yield_now().await;}}).await.unwrap();
    f.manager.stop(None,None).await;f.server.abort();
}
#[tokio::test]
async fn stopping_local_playback_keeps_all_output_connections_registered() {
    let mut f=Fixture::new().await;
    let manager=f.manager.clone();let local=f.local.clone();let mut cfg=config(&f.profile,&f.registration);cfg.endpoint="browser-second".into();cfg.primary=false;
    let register=tokio::spawn(async move{manager.register(cfg,local).await});
    let mut second=f.sockets.recv().await.unwrap();next(&mut second,"register").await;second.send(Message::Text(json!({"type":"registered","endpoint_id":"browser-second"}).to_string().into())).await.unwrap();register.await.unwrap().unwrap();
    f.manager.cancel_playback().await.unwrap();
    for (socket,id) in [(&mut f.socket,"browser-native"),(&mut second,"browser-second")] {
        assert!(f.manager.endpoint("qa",id).is_some());
        socket.send(Message::Text(json!({"type":"clock_probe","version":1,"endpoint_id":id,"core_clock_id":uuid::Uuid::new_v4().to_string(),"probe_id":uuid::Uuid::new_v4().to_string(),"t1_ns":1}).to_string().into())).await.unwrap();
        assert_eq!(next(socket,"clock_reply").await["endpoint_id"],id);
    }
    f.manager.stop(None,Some("another-registration")).await;assert!(f.manager.endpoint("qa","browser-second").is_some());
    f.manager.stop(None,None).await;f.server.abort();
}
#[tokio::test]
async fn desktop_claim_precedes_local_join_and_ordinary_commands_return_ownership() {
    let mut f=Fixture::new().await;f.prepare().await;assert!(f.manager.owned());
    assert!(tokio::time::timeout(Duration::from_millis(80),f.socket.recv()).await.is_err(),"no Armed/output report before the local player joins");
    f.local.release.notify_one();assert_eq!(next(&mut f.socket,"start_state").await["phase"],"armed");
    f.socket.send(Message::Text(json!({"type":"play","zone_id":"z","playback_id":"ordinary","track":{"id":"ordinary"},"url":"api/v1/media/held"}).to_string().into())).await.unwrap();
    let final_report=next(&mut f.socket,"start_state").await;assert_eq!(final_report["final"],true);assert_eq!(final_report["programme_frames_played"],0);
    tokio::time::timeout(Duration::from_secs(2),async{while f.manager.owned(){tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
    assert!(f.manager.endpoint("qa","browser-native").is_some(),"ordinary playback keeps the calibrated socket connected");
    f.manager.stop(None,None).await;assert!(!f.manager.owned());f.server.abort();
}
#[tokio::test]
async fn desktop_shutdown_waits_for_local_join_and_stale_scope_cannot_close_replacement() {
    let mut f=Fixture::new().await;f.prepare().await;let manager=f.manager.clone();let close=tokio::spawn(async move{manager.stop(None,None).await;});
    tokio::time::sleep(Duration::from_millis(80)).await;assert!(!close.is_finished());assert!(f.manager.owned());
    f.local.release.notify_one();tokio::time::timeout(Duration::from_secs(3),close).await.unwrap().unwrap();assert!(!f.manager.owned());
    let id=uuid::Uuid::new_v4().to_string();let manager=f.manager.clone();let profile=f.profile.clone();let local=f.local.clone();let next_id=id.clone();let register=tokio::spawn(async move{manager.register(config(&profile,&next_id),local).await});
    let mut socket=f.sockets.recv().await.unwrap();next(&mut socket,"register").await;socket.send(Message::Text(json!({"type":"registered","endpoint_id":"browser-native"}).to_string().into())).await.unwrap();register.await.unwrap().unwrap();
    f.manager.stop(None,Some(&f.registration)).await;assert!(f.manager.endpoint("qa","browser-native").is_some());
    f.manager.stop(None,Some(&id)).await;assert!(f.manager.endpoint("qa","browser-native").is_none());f.server.abort();
}
