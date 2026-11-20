//! Synthetic Core control/media server. Only the Linux test opens ALSA file/null.
use super::*;
use axum::{extract::ws::{WebSocket,WebSocketUpgrade,Message as CoreMessage},routing::get,Router};
struct Fixture {socket:WebSocket,bridge:tokio::task::JoinHandle<Result<()>>,server:tokio::task::JoinHandle<()>,release:Arc<tokio::sync::Notify>,core:String,host:String,base:String}
fn pcm()->Vec<u8> {
    let frames=1110;let raw=(0..frames*2).flat_map(|n|match n%4{0=>i32::MIN,1=>i32::MAX,2=>n*7919,_=>-n*3217}.to_le_bytes()).collect::<Vec<_>>();let size=raw.len() as u32;
    let mut data=Vec::new();for bytes in [b"RIFF".as_slice(),&(36+size).to_le_bytes(),b"WAVEfmt ",&16u32.to_le_bytes(),&1u16.to_le_bytes(),&2u16.to_le_bytes(),&768000u32.to_le_bytes(),&(768000u32*8).to_le_bytes(),&8u16.to_le_bytes(),&32u16.to_le_bytes(),b"data",&size.to_le_bytes()]{data.extend(bytes);}data.extend(raw);data
}
impl Fixture {
    async fn new(output:OutputConfig)->Self {
        let(tx,mut rx)=tokio::sync::mpsc::channel(1);let release=Arc::new(tokio::sync::Notify::new());let held=release.clone();
        let app=Router::new().route("/api/v1/bridge/events",get(move|ws:WebSocketUpgrade|{let tx=tx.clone();async move{ws.on_upgrade(move|socket|async move{tx.send(socket).await.unwrap();})}}))
            .route("/api/v1/media/source",get(||async{pcm()}))
            .route("/api/v1/media/slow",get(move||{let held=held.clone();async move{held.notified().await;pcm()}}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let base=format!("http://{}",listener.local_addr().unwrap());let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
        let profile=ConnectionProfile{core_public_key:None,id:"scheduled-qa".into(),core_id:None,name:"synthetic Core".into(),address:base.clone(),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]};
        let bridge=tokio::spawn(async move{session(&profile,"synthetic-control","scheduled-qa","QA",output,None).await});
        let socket=rx.recv().await.unwrap();let mut f=Self{socket,bridge,server,release,core:uuid::Uuid::new_v4().to_string(),host:String::new(),base};
        let register=f.next("register",None).await;assert_eq!(register["endpoint"]["capabilities"]["scheduled_start_v1"],cfg!(any(windows,target_os="linux")));assert_eq!(register["endpoint"]["capabilities"]["strict_sync"],false);assert_eq!(register["endpoint"]["capabilities"]["conversion_locations"],rmusic_bridge::sequence::Config::conversion_locations());
        f.probe().await;f
    }
    async fn send(&mut self,v:&Value){self.socket.send(CoreMessage::Text(v.to_string().into())).await.unwrap();}
    async fn next(&mut self,kind:&str,phase:Option<&str>)->Value {
        tokio::time::timeout(Duration::from_secs(5),async{loop{match self.socket.recv().await.unwrap().unwrap(){
            CoreMessage::Text(text)=>{let v:Value=serde_json::from_str(&text).unwrap();if kind=="start_state"&&matches!(phase,Some("armed"|"ready"|"cancelled"|"expired")){assert_ne!(v["type"],"state","programme report arrived before release");}
                if v["type"]==kind&&phase.is_none_or(|p|v["phase"]==p){return v;}},
            CoreMessage::Ping(bytes)=>self.socket.send(CoreMessage::Pong(bytes)).await.unwrap(),_=>{}
        }}}).await.unwrap()
    }
    async fn probe(&mut self){self.send(&json!({"type":"clock_probe","version":1,"endpoint_id":"scheduled-qa","core_clock_id":self.core,"probe_id":uuid::Uuid::new_v4().to_string(),"t1_ns":1})).await;let reply=self.next("clock_reply",None).await;self.host=reply["host_clock_id"].as_str().unwrap().into();}
    fn prepare(&self,generation:u64,slow:bool,expires:u64)->Value {json!({"type":"prepare_start","zone_id":"z","playback_id":uuid::Uuid::new_v4().to_string(),"track":{"id":"t"},"url":format!("{}/api/v1/media/{}",self.base,if slow{"slow"}else{"source"}),"position":0.,"volume":1.,"dsp":{},"start":{"version":1,"plan_id":uuid::Uuid::new_v4().to_string(),"generation":generation,"core_clock_id":self.core,"host_clock_id":self.host,"expires_host_ns":expires}})}
    async fn close(mut self){self.socket.send(CoreMessage::Close(None)).await.unwrap();let _=tokio::time::timeout(Duration::from_secs(3),&mut self.bridge).await.unwrap().unwrap();self.release.notify_waiters();self.server.abort();}
}
fn control(prepare:&Value,kind:&str)->Value {json!({"type":kind,"zone_id":prepare["zone_id"],"playback_id":prepare["playback_id"],"start":prepare["start"]})}
#[tokio::test]
async fn scheduled_protocol_socket_cancels_stalled_preparation_rejects_old_clocks_and_expires() {
    let mut f=Fixture::new(OutputConfig{device_id:Some("rmusic-never-opened".into()),..Default::default()}).await;
    let first=f.prepare(1,true,output::clock::host_ns().unwrap()+5_000_000_000);f.send(&first).await;assert_eq!(f.next("start_state",Some("armed")).await["programme_frames_played"],0);
    f.socket.send(CoreMessage::Ping(vec![1,2,3].into())).await.unwrap();tokio::time::timeout(Duration::from_millis(600),async{loop{if let CoreMessage::Pong(bytes)=f.socket.recv().await.unwrap().unwrap(){assert_eq!(&bytes[..],&[1,2,3]);break;}}}).await.unwrap();
    f.send(&control(&first,"cancel_start")).await;assert_eq!(f.next("start_state",Some("cancelled")).await["programme_frames_played"],0);
    f.send(&control(&first,"cancel_start")).await;f.next("start_state",Some("cancelled")).await;
    let second=f.prepare(2,true,output::clock::host_ns().unwrap()+5_000_000_000);f.send(&second).await;f.next("start_state",Some("armed")).await;
    let mut old=control(&first,"commit_start");old["deadline_host_ns"]=json!(output::clock::host_ns().unwrap()+100_000_000u64);f.send(&old).await;f.next("start_error",None).await;
    f.core=uuid::Uuid::new_v4().to_string();f.probe().await;f.next("start_state",Some("cancelled")).await;
    f.send(&control(&second,"cancel_start")).await;f.next("start_error",None).await;
    let expired=f.prepare(1,true,output::clock::host_ns().unwrap()+300_000_000);f.send(&expired).await;f.next("start_state",Some("armed")).await;let report=f.next("start_state",Some("expired")).await;assert_eq!(report["programme_frames_played"],0);assert_eq!(report["gate"]["released_host_ns"],0);f.close().await;
}
#[cfg(target_os="linux")]
#[tokio::test]
async fn scheduled_protocol_socket_starts_real_alsa_pcm768k_after_commit_and_stops_a_new_ready_plan() {
    let root=tempfile::tempdir().unwrap();let capture=root.path().join("capture.pcm");let conf=root.path().join("alsa.conf");std::fs::write(&conf,format!("pcm.null {{type null}}\npcm.capture {{type file slave.pcm null file \"{}\" format raw}}\n",capture.display())).unwrap();std::env::set_var("ALSA_CONFIG_PATH",conf);
    let mut f=Fixture::new(OutputConfig{device_id:Some("capture".into()),..Default::default()}).await;
    let first=f.prepare(1,false,output::clock::host_ns().unwrap()+5_000_000_000);f.send(&first).await;let ready=f.next("start_state",Some("ready")).await;assert_eq!(ready["programme_frames_played"],0);assert_eq!(ready["buffered_frames"],1110);
    let mut commit=control(&first,"commit_start");let deadline=output::clock::host_ns().unwrap()+100_000_000;commit["deadline_host_ns"]=json!(deadline);f.send(&commit).await;f.next("start_state",Some("committed")).await;
    let ended=f.next("state",None).await;assert!(output::clock::host_ns().unwrap()>=deadline);assert!(ended["position"].as_f64().unwrap()>=0.);
    let released=f.next("start_state",Some("released")).await;assert_eq!(released["programme_frames_played"],1110);assert!((deadline..=deadline+20_000_000).contains(&released["gate"]["released_host_ns"].as_u64().unwrap()));assert_eq!(std::fs::read(&capture).unwrap(),pcm()[44..]);
    f.send(&commit).await;f.next("start_state",Some("released")).await;assert_eq!(std::fs::read(&capture).unwrap(),pcm()[44..]);
    let second=f.prepare(2,false,output::clock::host_ns().unwrap()+5_000_000_000);f.send(&second).await;f.next("start_state",Some("ready")).await;
    let mut stale=commit.clone();stale["deadline_host_ns"]=json!(output::clock::host_ns().unwrap()+100_000_000u64);f.send(&stale).await;f.next("start_error",None).await;
    f.send(&json!({"type":"stop"})).await;let cancelled=f.next("start_state",Some("cancelled")).await;assert_eq!(cancelled["programme_frames_played"],0);assert_eq!(cancelled["gate"]["released_host_ns"],0);
    println!("SCHEDULED_BRIDGE_QA={released}");f.close().await;
}
