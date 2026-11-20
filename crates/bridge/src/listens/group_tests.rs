use super::*;
use crate::{connection::ConnectionProfile,listens::sync::Sync,sequence::Config};
use tokio::io::{AsyncReadExt,AsyncWriteExt};
fn owner()->Owner{Owner{core_id:"core-a".into(),user_id:"user-a".into()}}
fn final_value()->Value {json!({"type":"start_state","version":1,"measured_version":2,"phase":"cancelled","final":true,"zone_id":"z","core_playback_id":uuid::Uuid::new_v4().to_string(),"start":{"version":1,"plan_id":uuid::Uuid::new_v4().to_string(),"generation":1,"core_clock_id":"clock","host_clock_id":"host","expires_host_ns":9007199254740993u64},"gate":{"phase":"released"},"sample_rate":768000,"programme_frames_played":43,"sequence_final":[{"playback_id":"p","track_id":"t","position":43./768000.,"frames":43,"ended":false}]})}
#[test]
fn group_final_v2_journal_freezes_once_reopens_and_separates_owners_and_legacy_rows() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let recorder=Recorder::new(store.clone(),owner(),"endpoint".into());let mut value=final_value();value["sequence_final"][0]["position"]=json!(8189./768000.);recorder.capture(&value).unwrap();let first=pending(&store,&owner()).unwrap().remove(0).body;recorder.capture(&value).unwrap();assert_eq!(pending(&store,&owner()).unwrap()[0].body,first);
    let mut changed=value;changed["programme_frames_played"]=json!(44);assert!(recorder.capture(&changed).is_err());assert!(store.pending(&owner()).unwrap().is_empty());
    let reopened=Store::open(dir.path()).unwrap();assert_eq!(pending(&reopened,&owner()).unwrap()[0].body,first);assert_eq!(first["report"]["start"]["expires_host_ns"].as_u64(),Some(9007199254740993));
    assert!(pending(&reopened,&Owner{user_id:"user-b".into(),..owner()}).unwrap().is_empty());assert!(pending(&reopened,&Owner{core_id:"other".into(),..owner()}).unwrap().is_empty());
}
async fn server(replies:Vec<(String,u16,Option<Value>)>)->(Config,tokio::task::JoinHandle<Vec<String>>) {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let config=Config::new(ConnectionProfile{core_public_key:None,id:"qa".into(),name:"QA".into(),address:format!("http://{address}/external/music"),core_id:Some("core-a".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]},"qa-token".into(),"endpoint".into()).unwrap();
    let task=tokio::spawn(async move{let mut bodies=Vec::new();for(path,status,response)in replies {
        let(mut socket,_)=tokio::time::timeout(Duration::from_secs(3),listener.accept()).await.unwrap().unwrap();let mut header=Vec::new();while !header.ends_with(b"\r\n\r\n"){header.push(socket.read_u8().await.unwrap());}let header=String::from_utf8(header).unwrap();assert!(header.lines().next().unwrap().contains(&format!(" /external/music/api/v1/{path} ")));assert_eq!(header.contains("Bearer qa-token"),path!="status");let len=header.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse::<usize>().unwrap())).unwrap_or(0);let mut body=vec![0;len];socket.read_exact(&mut body).await.unwrap();if !body.is_empty(){bodies.push(String::from_utf8(body).unwrap());}
        if let Some(body)=response{let body=body.to_string();socket.write_all(format!("HTTP/1.1 {status} QA\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();}else{socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();}
    }bodies});(config,task)
}
fn status(enabled:bool)->Value {json!({"core_id":"core-a","capabilities":{"measured_listens":true,"group_output_final_v2":enabled}})}
#[tokio::test]
async fn group_final_v2_upload_waits_for_capability_and_retries_exact_body_until_valid_ack() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let mut value=final_value();value["sequence_final"][0]["position"]=json!(8189./768000.);value["sequence_final"][0]["frames"]=json!(8189);value["programme_frames_played"]=json!(8189);Recorder::new(store.clone(),owner(),"endpoint".into()).capture(&value).unwrap();let row=pending(&store,&owner()).unwrap().remove(0);let path=format!("sync/starts/{}/final-receipts",row.group_id);let frozen:String=store.db.lock().unwrap().query_row("SELECT payload FROM group_output_finals",[],|r|r.get(0)).unwrap();
    let mut replies=vec![("status".into(),200,Some(status(false)))];for(code,body)in[(409,Some(json!({}))), (200,None), (200,Some(json!({}))), (200,Some(json!({"applied":false})))]{replies.push(("status".into(),200,Some(status(true))));replies.push((path.clone(),code,body));}
    let(config,task)=server(replies).await;let sync=Sync::new(store.clone(),config,owner());assert_eq!(sync.flush().await.unwrap(),0);assert_eq!(sync.flush().await.unwrap(),0);assert!(sync.flush().await.is_err());assert!(sync.flush().await.is_err());assert_eq!(pending(&store,&owner()).unwrap().len(),1);assert_eq!(sync.flush().await.unwrap(),1);assert!(pending(&store,&owner()).unwrap().is_empty());let bodies=task.await.unwrap();assert_eq!(bodies.len(),4);assert!(bodies.iter().all(|body|body==&frozen),"each upload must match the frozen journal bytes, including source-position decimals");
}
#[tokio::test]
async fn group_final_v2_rejection_retains_evidence_and_changed_core_never_gets_a_post() {
    for wrong_core in [false,true] {
        let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();Recorder::new(store.clone(),owner(),"endpoint".into()).capture(&final_value()).unwrap();let row=pending(&store,&owner()).unwrap().remove(0);let mut value=status(true);if wrong_core{value["core_id"]=json!("other");}let mut replies=vec![("status".into(),200,Some(value))];if !wrong_core{replies.push((format!("sync/starts/{}/final-receipts",row.group_id),400,Some(json!({}))));}
        let(config,task)=server(replies).await;let result=Sync::new(store.clone(),config,owner()).flush_groups().await;if wrong_core{assert!(result.is_err());assert_eq!(pending(&store,&owner()).unwrap().len(),1);}else{assert_eq!(result.unwrap(),0);assert!(pending(&store,&owner()).unwrap().is_empty());assert_eq!(store.db.lock().unwrap().query_row("SELECT error FROM group_output_finals",[],|r|r.get::<_,String>(0)).unwrap(),"http_400");}task.await.unwrap();assert_eq!(store.db.lock().unwrap().query_row("SELECT COUNT(*) FROM group_output_finals",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
}
