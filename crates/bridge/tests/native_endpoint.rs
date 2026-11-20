//! Embedded callers use an application shutdown future; no process signal or DAC is used.
use axum::{extract::ws::{Message,WebSocket,WebSocketUpgrade},routing::get,Router};
use rmusic_audio::output::{self,OutputConfig};
use rmusic_bridge::{connection::ConnectionProfile,listens::Store,native};
use serde_json::{json,Value};
use std::{sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Duration};

fn profile(address:String)->ConnectionProfile {
    ConnectionProfile{core_public_key:None,id:"embedded-qa".into(),core_id:None,name:"synthetic Core".into(),address,skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]}
}
async fn next(socket:&mut WebSocket,kind:&str)->Value {
    tokio::time::timeout(Duration::from_secs(3),async {
        loop {match socket.recv().await.unwrap().unwrap() {
            Message::Text(text)=>{let value:Value=serde_json::from_str(&text).unwrap();assert_ne!(value["type"],"start_error","unexpected start error: {value}");if value["type"]==kind{return value;}},
            Message::Ping(bytes)=>socket.send(Message::Pong(bytes)).await.unwrap(),
            other=>panic!("unexpected message: {other:?}"),
        }}
    }).await.unwrap_or_else(|_|panic!("timed out waiting for {kind}"))
}

#[tokio::test]
async fn embedded_shutdown_cancels_identity_lookup_before_registration() {
    let entered=Arc::new(tokio::sync::Notify::new());let blocked=entered.clone();
    let registrations=Arc::new(AtomicUsize::new(0));let accepted=registrations.clone();
    let router=Router::new()
        .route("/api/v1/status",get(move||{let blocked=blocked.clone();async move {blocked.notify_one();std::future::pending::<String>().await}}))
        .route("/api/v1/bridge/events",get(move||{accepted.fetch_add(1,Ordering::AcqRel);async{"unexpected registration"}}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(format!("http://{}",listener.local_addr().unwrap()));
    let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
    let root=tempfile::tempdir().unwrap();let store=Store::open(root.path()).unwrap();
    let (close,closed)=tokio::sync::oneshot::channel();
    let session=tokio::spawn(async move{native::session(&profile,"synthetic-token","embedded-qa","QA",OutputConfig{device_id:Some("rmusic-never-opened".into()),..Default::default()},Some(store),async{let _=closed.await;}).await});
    tokio::time::timeout(Duration::from_secs(3),entered.notified()).await.unwrap();close.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2),session).await.unwrap().unwrap().unwrap();
    assert_eq!(registrations.load(Ordering::Acquire),0,"shutdown must not wait for identity timeout or open a connection later");
    server.abort();
}

#[tokio::test]
async fn embedded_shutdown_cancels_armed_stalled_source_without_process_signal() {
    let (accepted,mut sockets)=tokio::sync::mpsc::channel(1);let entered=Arc::new(tokio::sync::Notify::new());let blocked=entered.clone();
    let router=Router::new()
        .route("/api/v1/bridge/events",get(move|headers:axum::http::HeaderMap,ws:WebSocketUpgrade|{assert_eq!(headers["authorization"],"Bearer synthetic-token");let accepted=accepted.clone();async move{ws.on_upgrade(move|socket|async move{accepted.send(socket).await.unwrap();})}}))
        .route("/api/v1/media/held",get(move||{let blocked=blocked.clone();async move{blocked.notify_one();std::future::pending::<String>().await}}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(format!("http://{}",listener.local_addr().unwrap()));
    let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
    let (close,closed)=tokio::sync::oneshot::channel();
    let session=tokio::spawn(async move{native::session(&profile,"synthetic-token","embedded-qa","QA",OutputConfig{device_id:Some("rmusic-never-opened".into()),..Default::default()},None,async{let _=closed.await;}).await});
    let mut socket=tokio::time::timeout(Duration::from_secs(3),sockets.recv()).await.unwrap().unwrap();
    let registration=next(&mut socket,"register").await;assert_eq!(registration["endpoint"]["id"],"embedded-qa");assert_eq!(registration["endpoint"]["capabilities"]["strict_sync"],false);
    let core=uuid::Uuid::new_v4().to_string();
    socket.send(Message::Text(json!({"type":"clock_probe","version":1,"endpoint_id":"embedded-qa","core_clock_id":core,"probe_id":uuid::Uuid::new_v4().to_string(),"t1_ns":1}).to_string().into())).await.unwrap();
    let reply=next(&mut socket,"clock_reply").await;
    socket.send(Message::Text(json!({"type":"prepare_start","zone_id":"z","playback_id":uuid::Uuid::new_v4().to_string(),"track":{"id":"held"},"url":"api/v1/media/held","position":0.,"volume":1.,"start":{"version":1,"plan_id":uuid::Uuid::new_v4().to_string(),"generation":1,"core_clock_id":core,"host_clock_id":reply["host_clock_id"],"expires_host_ns":output::clock::host_ns().unwrap()+5_000_000_000u64}}).to_string().into())).await.unwrap();
    let armed=next(&mut socket,"start_state").await;assert_eq!(armed["phase"],"armed");assert_eq!(armed["programme_frames_played"],0);
    tokio::time::timeout(Duration::from_secs(3),entered.notified()).await.unwrap();close.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2),session).await.unwrap().unwrap().unwrap();
    // Dropping the socket follows joined shutdown; a stalled source cannot emit a later Ready/play report.
    loop {
        let after=tokio::time::timeout(Duration::from_secs(2),socket.recv()).await.unwrap();
        let Some(Ok(Message::Text(text)))=after else{break;};let value:Value=serde_json::from_str(&text).unwrap();
        assert_eq!(value["type"],"start_state","late report: {value}");assert!(matches!(value["phase"].as_str(),Some("cancelled"|"expired")),"late ready/play report: {value}");assert_eq!(value["final"],true);assert_eq!(value["programme_frames_played"],0);
    }
    server.abort();
}
