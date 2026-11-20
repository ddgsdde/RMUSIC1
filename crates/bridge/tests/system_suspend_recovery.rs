#![cfg(windows)]
use rmusic_bridge::{connection::ConnectionProfile,listens::{Store,Owner,cursor::{self,Boundary,Recorder},sync::Sync},sequence::{Config,recovery}};
use rmusic_audio::output::{power,DeviceUnavailable};
use serde_json::{json,Value};
use tokio::io::{AsyncReadExt,AsyncWriteExt};

#[tokio::test]
async fn system_suspend_gate_requires_matching_core_ack_after_lost_response() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    store.save_system_suspend(7,true).unwrap();let saved=store.clone();
    power::configure(store.client_id.clone(),7,true,move|generation,held|saved.save_system_suspend(generation,held)).unwrap();
    let owner=Owner {core_id:"core-a".into(),user_id:"user-a".into()};
    let playback=uuid::Uuid::new_v4().to_string();
    let mut recorder=Recorder::begin(store.clone(),owner.clone(),"endpoint".into(),"z".into(),uuid::Uuid::new_v4().to_string(),768000,Boundary {playback_id:playback.clone(),track_id:"track".into(),position:0.,frames:0}).unwrap();
    recorder.sample(vec![],Boundary {playback_id:playback,track_id:"track".into(),position:0.25,frames:192000},768000,false,true).unwrap();
    let handle=recorder.handle.clone();store.drain().unwrap();
    let stamp=json!({"host_id":store.client_id,"generation":7});
    let status=|supported|json!({"core_id":"core-a","capabilities":{"continuous_recovery":true,"measured_listens":true,"system_suspend_recovery":supported}});
    let zone=json!({"id":"z","endpoint_id":"endpoint","playback_id":uuid::Uuid::new_v4().to_string(),"track":{"id":"track"},"status":"playing","position":0.25});
    let accepted=json!({"accepted":true,"zone":zone,"system_suspend_ack":stamp});
    let replies=vec![
        ("status",Some(status(false))),("status",Some(status(false))),
        ("status",Some(status(true))),("status",Some(status(true))),("zones/z/recover",None),
        ("zones/z/recover",Some(json!({"accepted":false,"waiting_for_play":true}))),
        ("status",Some(status(true))),("status",Some(status(true))),("zones/z/recover",Some(accepted)),
        ("zones/z",Some(zone)),("library/tracks/track/ticket",Some(json!({"api_path":"api/v1/media/track?ticket=qa"})))
    ];
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let server=tokio::spawn(async move {
        let mut requests=Vec::new();
        for (path,response) in replies {
            let (mut socket,_)=tokio::time::timeout(std::time::Duration::from_secs(5),listener.accept()).await.unwrap().unwrap();
            let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}
            let headers=String::from_utf8(headers).unwrap();assert!(headers.lines().next().unwrap().contains(&format!(" /api/v1/{path} ")),"expected {path}, got {headers}");
            let len=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse::<usize>().unwrap())).unwrap_or(0);
            let mut body=vec![0;len];socket.read_exact(&mut body).await.unwrap();
            if path.ends_with("recover"){requests.push(serde_json::from_slice::<Value>(&body).unwrap());}
            if let Some(response)=response {let body=response.to_string();socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();}
            else {socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();}
        }
        requests
    });
    let config=Config::new(ConnectionProfile {core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),core_id:Some(owner.core_id.clone()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]},"qa-token".into(),"endpoint".into()).unwrap();
    let sync=Sync::new(store.clone(),config.clone(),owner.clone());
    let error=tokio::time::timeout(std::time::Duration::from_secs(15),recovery::restore(config.clone(),store.clone(),owner.clone(),sync.clone())).await.unwrap().err().unwrap();assert!(error.downcast_ref::<DeviceUnavailable>().is_some());assert!(handle.ready().unwrap());
    assert!(tokio::time::timeout(std::time::Duration::from_secs(15),recovery::restore(config.clone(),store.clone(),owner.clone(),sync.clone())).await.unwrap().unwrap().is_none());
    assert_eq!(power::held_stamp().unwrap().map(|s|json!(s)),Some(stamp.clone()));assert_eq!(store.system_suspend_state().unwrap(),(7,true));
    let restored=tokio::time::timeout(std::time::Duration::from_secs(15),recovery::restore(config,store.clone(),owner,sync)).await.unwrap().unwrap().unwrap();assert_eq!(restored.command["position"],0.25);
    assert!(power::held_stamp().unwrap().is_none());assert_eq!(Store::open(dir.path()).unwrap().system_suspend_state().unwrap(),(7,false));
    let requests=server.await.unwrap();assert_eq!(requests.len(),3);assert!(requests.iter().all(|body|body["system_suspend"]==stamp));
    assert_eq!(requests[0],requests[1]);assert_eq!(requests[1],requests[2]);
    assert!(cursor::freeze(store,&Owner {core_id:"core-a".into(),user_id:"user-a".into()},"endpoint").unwrap().is_some());
}
