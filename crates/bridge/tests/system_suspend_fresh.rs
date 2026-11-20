#![cfg(windows)]
use rmusic_audio::output::power;
use rmusic_bridge::{connection::ConnectionProfile,listens::{Store,Owner,sync::Sync},sequence::{Config,Preparation,recovery}};
use serde_json::{json,Value};
use tokio::io::{AsyncReadExt,AsyncWriteExt};

#[tokio::test]
async fn system_suspend_fresh_start_keeps_a_missing_cursor_held_until_a_manual_grant() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();store.save_system_suspend(9,true).unwrap();let saved=store.clone();
    power::configure(store.client_id.clone(),9,true,move|generation,held|saved.save_system_suspend(generation,held)).unwrap();
    let stamp=json!({"host_id":store.client_id,"generation":9});let old=uuid::Uuid::new_v4().to_string();let manual=uuid::Uuid::new_v4().to_string();
    let status=json!({"core_id":"core-a","capabilities":{"recovery_controls":true,"system_suspend_fresh_start":true}});
    let zone=|id:Value,state|json!({"id":"z","endpoint_id":"endpoint","playback_id":id,"status":state,"track":{"id":"track"}});
    let held=zone(Value::Null,"stopped");let active=zone(json!(manual),"playing");
    let mut wrong_ack=stamp.clone();wrong_ack["generation"]=json!(8);
    let replies=vec![("status",status.clone()),("zones",json!({"items":[zone(json!(old),"playing")]})),("status",status.clone()),
        ("zones/z/recover/unavailable",json!({"applied":true,"accepted":false,"waiting_for_play":true,"zone":held})),
        ("status",status.clone()),("zones/z/recover/unavailable",json!({"applied":false,"accepted":false,"waiting_for_play":true,"zone":held})),
        ("status",status.clone()),("zones/z/recover/unavailable",json!({"accepted":true,"system_suspend_ack":wrong_ack,"zone":active})),
        ("status",status),("zones/z/recover/unavailable",json!({"applied":false,"accepted":true,"system_suspend_ack":stamp,"zone":active})),
        ("zones/z/prepare",json!({"accepted":true,"plan_id":uuid::Uuid::new_v4().to_string(),"items":[],"continuous_recovery":true}))];
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let server=tokio::spawn(async move {let mut bodies=Vec::new();for(path,response)in replies {
        let(mut socket,_)=tokio::time::timeout(std::time::Duration::from_secs(5),listener.accept()).await.unwrap().unwrap();
        let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}
        let headers=String::from_utf8(headers).unwrap();assert!(headers.lines().next().unwrap().contains(&format!(" /api/v1/{path} ")),"{headers}");
        let size=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse::<usize>().unwrap())).unwrap_or(0);
        let mut body=vec![0;size];socket.read_exact(&mut body).await.unwrap();if size>0{bodies.push(serde_json::from_slice::<Value>(&body).unwrap());}
        let body=response.to_string();socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    }bodies});
    let config=Config::new(ConnectionProfile {core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),core_id:Some("core-a".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]},"qa-token".into(),"endpoint".into()).unwrap();
    let owner=Owner {core_id:"core-a".into(),user_id:"user-a".into()};let sync=Sync::new(store.clone(),config.clone(),owner.clone());
    let restored=tokio::time::timeout(std::time::Duration::from_secs(5),recovery::restore(config.clone(),store.clone(),owner,sync)).await.unwrap().unwrap();assert!(restored.is_none());assert!(power::held_stamp().unwrap().is_some());
    assert!(matches!(config.negotiate("z","track",&old).await.unwrap(),Preparation::Stale(_)));assert!(power::held_stamp().unwrap().is_some());
    assert!(config.negotiate("z","track",&manual).await.is_err());assert!(power::held_stamp().unwrap().is_some());
    assert!(matches!(config.negotiate("z","track",&manual).await.unwrap(),Preparation::Ready(_)));assert!(power::held_stamp().unwrap().is_none());assert_eq!(store.system_suspend_state().unwrap(),(9,false));
    let bodies=server.await.unwrap();assert_eq!(bodies[0]["system_suspend"],stamp);assert!(bodies[0]["playback_id"].is_null());assert_eq!(bodies[1]["playback_id"],old);assert_eq!(bodies[2]["playback_id"],manual);assert_eq!(bodies[3]["playback_id"],manual);
}
