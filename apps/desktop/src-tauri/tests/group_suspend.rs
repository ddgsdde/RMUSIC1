#![cfg(windows)]
//! A persisted suspend hold and the actual desktop socket; the source never opens a device.
use axum::{extract::ws::{WebSocket,WebSocketUpgrade,Message},routing::{get,post},Router};
use futures_util::future::BoxFuture;
use rmusic_desktop::group_output::{Manager,LocalPlayer,Registration};
use rmusic_bridge::{connection::ConnectionProfile,listens::Store};
use rmusic_audio::output::{self,power,OutputConfig};
use serde_json::{json,Value};
use std::{sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Duration};
struct Local(AtomicUsize);
impl LocalPlayer for Local {fn prepare<'a>(&'a self,_:&'a Value)->BoxFuture<'a,anyhow::Result<()>>{Box::pin(async move{self.0.fetch_add(1,Ordering::SeqCst);Ok(())})}}
async fn next(socket:&mut WebSocket,kind:&str)->Value {
    tokio::time::timeout(Duration::from_secs(3),async{loop{if let Some(Ok(Message::Text(text)))=socket.recv().await{let value:Value=serde_json::from_str(&text).unwrap();if value["type"]==kind{return value;}assert_ne!(value["type"],"start_error","{value}");}}}).await.unwrap()
}
#[tokio::test]
async fn desktop_group_manual_prepare_acknowledges_a_durable_suspend_hold_without_ordinary_play() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();store.save_system_suspend(9,true).unwrap();let saved=store.clone();power::configure(store.client_id.clone(),9,true,move|generation,held|saved.save_system_suspend(generation,held)).unwrap();
    let(accepted,mut sockets)=tokio::sync::mpsc::channel(1);
    let router=Router::new().route("/api/v1/endpoints/browser",post(||async{axum::Json(json!({}))}))
        .route("/api/v1/bridge/events",get(move|ws:WebSocketUpgrade|{let accepted=accepted.clone();async move{ws.on_upgrade(move|socket|async move{accepted.send(socket).await.unwrap();})}}))
        .route("/api/v1/media/held",get(||async{std::future::pending::<String>().await}))
        .route("/api/v1/sync/starts/{id}/cancel",post(||async{axum::Json(json!({"phase":"cancelled"}))}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=format!("http://{}",listener.local_addr().unwrap());let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
    let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"QA".into(),address,core_id:Some("synthetic-core".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]};let config=Registration{id:uuid::Uuid::new_v4().to_string(),profile,token:"synthetic-token".into(),endpoint:"browser-suspend".into(),name:"QA".into(),output:OutputConfig{device_id:Some("rmusic-never-opened".into()),..Default::default()}};
    let manager=Arc::new(Manager::default());let local=Arc::new(Local(AtomicUsize::new(0)));let worker=manager.clone();let owner=local.clone();let register=tokio::spawn(async move{worker.register(config,owner).await});
    let mut socket=sockets.recv().await.unwrap();next(&mut socket,"register").await;socket.send(Message::Text(json!({"type":"registered","endpoint_id":"browser-suspend"}).to_string().into())).await.unwrap();register.await.unwrap().unwrap();
    let core=uuid::Uuid::new_v4().to_string();socket.send(Message::Text(json!({"type":"clock_probe","version":1,"endpoint_id":"browser-suspend","core_clock_id":core,"probe_id":uuid::Uuid::new_v4().to_string(),"t1_ns":1}).to_string().into())).await.unwrap();let reply=next(&mut socket,"clock_reply").await;
    let mut command=json!({"type":"prepare_start","group_queue":true,"zone_id":"z","playback_id":uuid::Uuid::new_v4().to_string(),"track":{"id":"held"},"url":"api/v1/media/held","position":0.,"start":{"version":1,"plan_id":uuid::Uuid::new_v4().to_string(),"generation":1,"core_clock_id":core,"host_clock_id":reply["host_clock_id"],"expires_host_ns":output::clock::host_ns().unwrap()+5_000_000_000u64}});
    if let Some(stamp)=reply.get("group_power"){command["start"]["group_power"]=stamp.clone();}
    assert!(reply["group_power"].is_object());
    for field in ["generation","wake_generation","host_id","missing"] {
        let mut wrong=command.clone();match field{"host_id"=>wrong["start"]["group_power"][field]=json!("other-host"),"missing"=>{wrong["start"].as_object_mut().unwrap().remove("group_power");},_=>wrong["start"]["group_power"][field]=json!(99)}
        socket.send(Message::Text(wrong.to_string().into())).await.unwrap();next(&mut socket,"start_error").await;assert!(!manager.owned());assert_eq!(local.0.load(Ordering::SeqCst),0);assert!(power::held_stamp().unwrap().is_some());command["start"]["generation"]=json!(command["start"]["generation"].as_u64().unwrap()+1);
    }
    socket.send(Message::Text(command.to_string().into())).await.unwrap();loop{let report=next(&mut socket,"start_state").await;if report["start"]==command["start"]{assert_eq!(report["phase"],"armed");break;}}
    assert!(power::held_stamp().unwrap().is_none(),"manual group preparation must clear the durable hold without an ordinary zone play");assert_eq!(store.system_suspend_state().unwrap(),(9,false));assert_eq!(local.0.load(Ordering::SeqCst),1);
    manager.stop(None,None).await;assert!(!manager.owned());server.abort();
}
