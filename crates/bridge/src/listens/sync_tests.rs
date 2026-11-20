use super::*;
use crate::connection::ConnectionProfile;
use serde_json::{json,Value};
use tokio::io::{AsyncReadExt,AsyncWriteExt};
fn owner()->Owner {Owner {core_id:"core-a".into(),user_id:"user-a".into()}}
fn record(store:Arc<Store>)->Snapshot {
    let mut recorder=Recorder::new(store.clone(),owner(),"track".into(),"曲目".into(),"QA".into(),2.)
        .with_measured(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),Instant::now());
    recorder.sample_device(0.25,12000,48000).unwrap();recorder.finish("ended").unwrap();store.drain().unwrap();recorder.row.snapshot
}
async fn server(replies:Vec<(&'static str,bool,u16,Option<Value>)>)->(Config,tokio::task::JoinHandle<Vec<Value>>) {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let config=Config::new(ConnectionProfile {core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}/external/music"),core_id:Some("core-a".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]},"qa-session".into(),"endpoint".into()).unwrap();
    let task=tokio::spawn(async move {
        let mut bodies=Vec::new();
        for (path,authenticated,status,response) in replies {
            let(mut socket,_)=tokio::time::timeout(Duration::from_secs(3),listener.accept()).await.unwrap().unwrap();
            let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}
            let headers=String::from_utf8(headers).unwrap();assert!(headers.lines().next().unwrap().contains(&format!(" /external/music/api/v1/{path} ")));assert_eq!(headers.contains("Bearer qa-session"),authenticated);
            let len=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse::<usize>().unwrap())).unwrap_or(0);
            let mut body=vec![0;len];socket.read_exact(&mut body).await.unwrap();bodies.push(if body.is_empty(){Value::Null}else{serde_json::from_slice(&body).unwrap()});
            if let Some(response)=response {let response=response.to_string();socket.write_all(format!("HTTP/1.1 {status} QA\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();}
            else {socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();}
        }bodies
    });(config,task)
}
fn status()->Value {json!({"core_id":"core-a","capabilities":{"measured_listens":true}})}
#[tokio::test]
async fn dual_listen_upload_waits_for_v2_and_retries_the_exact_integer_payload() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let mut r=Recorder::new(store.clone(),owner(),"track".into(),"PCM".into(),"QA".into(),120.).with_measured_output(uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),Instant::now(),10.).unwrap();
    r.sample_device_at(11.0005,768000,768000,chrono::Utc::now().timestamp_millis(),1.).unwrap();r.finish("ended").unwrap();store.drain().unwrap();let snapshot=r.row.snapshot;
    let mut v2=status();v2["capabilities"]["measured_listens_v2"]=json!(true);
    let(config,task)=server(vec![("status",false,200,Some(status())),("status",false,200,Some(v2.clone())),("stats/offline",true,200,None),("status",false,200,Some(v2)),("stats/offline",true,200,Some(json!({"accepted":[{"id":snapshot.id,"version":snapshot.version}],"rejected":[]})))]).await;
    let sync=Sync::new(store.clone(),config,owner());assert_eq!(sync.flush().await.unwrap(),0);assert!(sync.flush().await.is_err());assert_eq!(store.pending(&owner()).unwrap().len(),1);assert_eq!(sync.flush().await.unwrap(),1);assert!(store.pending(&owner()).unwrap().is_empty());
    let requests=task.await.unwrap();assert_eq!(requests[2],requests[4]);assert_eq!(requests[2]["sessions"][0]["measured"]["version"],2);assert_eq!(requests[2]["sessions"][0]["measured"]["segments"][0]["output"]["end"],768000);
}
#[tokio::test]
async fn authenticated_listener_is_bound_to_the_selected_core_and_old_cores_need_no_user_probe() {
    let(config,task)=server(vec![("status",false,200,Some(status())),("auth/me",true,200,Some(json!({"id":"user-a"})))]).await;
    assert_eq!(config.listener().await.unwrap(),Some(owner()));task.await.unwrap();
    for core in ["core-a","other-core"] {
        let(config,task)=server(vec![("status",false,200,Some(json!({"core_id":core}))) ]).await;
        let result=config.listener().await;if core=="core-a"{assert!(result.unwrap().is_none());}else{assert!(result.is_err());}task.await.unwrap();
    }
}
#[tokio::test]
async fn a_lost_upload_ack_keeps_the_durable_snapshot_and_retries_identical_owner_and_version() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let snapshot=record(store.clone());
    let(config,task)=server(vec![("status",false,200,Some(status())),("stats/offline",true,200,None),
        ("status",false,200,Some(status())),("stats/offline",true,200,Some(json!({"accepted":[{"id":snapshot.id,"version":snapshot.version}],"rejected":[]})))]).await;
    let sync=Sync::new(store.clone(),config,owner());assert!(sync.flush().await.is_err());assert_eq!(store.pending(&owner()).unwrap().len(),1);
    assert_eq!(sync.flush().await.unwrap(),1);assert!(store.pending(&owner()).unwrap().is_empty());
    let requests=task.await.unwrap();assert_eq!(requests[1],requests[3]);assert_eq!(requests[1]["core_id"],"core-a");assert_eq!(requests[1]["user_id"],"user-a");assert!(requests[1]["sessions"][0]["measured"].is_object());
}
#[tokio::test]
async fn unsupported_or_changed_core_keeps_measured_rows_without_legacy_conversion() {
    for changed in [false,true] {
        let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();record(store.clone());
        let(config,task)=server(vec![("status",false,200,Some(json!({"core_id":if changed{"other-core"}else{"core-a"}})))]).await;
        let result=Sync::new(store.clone(),config,owner()).flush().await;if changed{assert!(result.is_err());}else{assert_eq!(result.unwrap(),0);}
        task.await.unwrap();assert_eq!(store.pending(&owner()).unwrap().len(),1);assert!(store.pending_supported(&owner(),false).unwrap().is_empty());
    }
}
#[tokio::test]
async fn switching_authenticated_users_does_not_upload_another_users_pending_rows() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();record(store.clone());
    let profile=ConnectionProfile {core_public_key:None,id:"qa".into(),name:"qa".into(),address:"http://127.0.0.1:9".into(),core_id:Some("core-a".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]};
    let config=Config::new(profile,"other-user-session".into(),"endpoint".into()).unwrap();
    assert_eq!(Sync::new(store.clone(),config,Owner {user_id:"user-b".into(),..owner()}).flush().await.unwrap(),0);
    assert_eq!(store.pending(&owner()).unwrap().len(),1);
}
