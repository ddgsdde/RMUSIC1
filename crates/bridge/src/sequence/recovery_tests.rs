use super::*;
use crate::listens::{
    cursor::{Boundary, Recorder as CursorRecorder},
    Recorder, Snapshot,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test]
async fn dual_recovery_v2_restore_never_downgrades_and_propagates_version_to_new_worker() {
 for supported in [false,true] {
  let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();let cursor=CursorRecorder::begin_timed(store.clone(),owner(),"endpoint".into(),"z".into(),uuid::Uuid::new_v4().to_string(),48000,Boundary{playback_id:uuid::Uuid::new_v4().to_string(),track_id:"track".into(),position:0.,frames:0},2).unwrap();let handle=cursor.handle.clone();assert!(handle.ready().unwrap());
  let frozen=cursor::freeze(store.clone(),&owner(),"endpoint").unwrap().unwrap().0;let mut status=status("core-a");for flag in ["measured_listens_v2","measured_listens_online_v2","measured_listens_socket_v2","measured_listens_recovery_v2"]{status["capabilities"][flag]=json!(supported);}
  let id=uuid::Uuid::new_v4().to_string();let mut replies=vec![("status",false,200,Some(status))];if supported{replies.extend([("zones/z/recover",true,200,Some(json!({"accepted":true,"measured_version":2,"zone":zone(&id,"paused")}))),("zones/z",true,200,Some(zone(&id,"paused"))),("library/tracks/track/ticket",true,200,Some(json!({"api_path":"/api/v1/audio/track"})))]);}
  let(config,task)=server(replies).await;let sync=Sync::new(store.clone(),config.clone(),owner());let result=restore(config,store,owner(),sync).await;let bodies=task.await.unwrap();assert!(handle.is_current().unwrap());
  if supported{let restored=result.unwrap().unwrap();assert_eq!(restored.command["measured_version"],2);assert_eq!(restored.command["playback_id"],id);assert_eq!(bodies[1],frozen.request.unwrap());}else{assert!(result.is_err());assert_eq!(bodies.len(),1);}
 }
}
fn owner() -> Owner {
    Owner {
        core_id: "core-a".into(),
        user_id: "user-a".into(),
    }
}
fn status(core: &str) -> Value {
    json!({"core_id":core,"capabilities":{"continuous_recovery":true,"measured_listens":true}})
}
fn zone(id: &str, status: &str) -> Value {
    json!({"id":"z","endpoint_id":"endpoint","playback_id":id,"track":{"id":"track"},"position":0.25,"status":status,"volume":0.17,"muted":true,"dsp":{},"transitions":{}})
}
fn checkpoint(store: Arc<Store>, listen: bool) -> (cursor::Handle, Option<Snapshot>) {
    let playback = uuid::Uuid::new_v4().to_string();
    let group = uuid::Uuid::new_v4().to_string();
    let snapshot = if listen {
        let mut recorder = Recorder::new(
            store.clone(),
            owner(),
            "track".into(),
            "track".into(),
            "QA".into(),
            2.,
        )
        .with_measured(playback.clone(), group, std::time::Instant::now());
        recorder.sample_device(0.25, 12000, 48000).unwrap();
        recorder.finish("interrupted").unwrap();
        Some(recorder.row.snapshot)
    } else {
        None
    };
    let mut recorder = CursorRecorder::begin(
        store,
        owner(),
        "endpoint".into(),
        "z".into(),
        uuid::Uuid::new_v4().to_string(),
        48000,
        Boundary {
            playback_id: playback.clone(),
            track_id: "track".into(),
            position: 0.,
            frames: 0,
        },
    )
    .unwrap();
    recorder
        .sample(
            vec![],
            Boundary {
                playback_id: playback,
                track_id: "track".into(),
                position: 0.25,
                frames: 12000,
            },
            48000,
            false,
            true,
        )
        .unwrap();
    assert!(recorder.handle.ready().unwrap());
    (recorder.handle, snapshot)
}
async fn server(
    replies: Vec<(&'static str, bool, u16, Option<Value>)>,
) -> (Config, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = Config::new(
        ConnectionProfile {core_public_key:None,
            id: "qa".into(),
            name: "qa".into(),
            address: format!("http://{address}/external/music"),
            core_id: Some("core-a".into()),
            skip_certificate_verification: false, certificate_sha256: None,
            alternatives: vec![],
        },
        "qa-session".into(),
        "endpoint".into(),
    )
    .unwrap();
    let task = tokio::spawn(async move {
        let mut bodies = Vec::new();
        for (path, authenticated, status, response) in replies {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut headers = Vec::new();
            while !headers.ends_with(b"\r\n\r\n") {
                headers.push(socket.read_u8().await.unwrap());
            }
            let headers = String::from_utf8(headers).unwrap();
            assert!(headers
                .lines()
                .next()
                .unwrap()
                .contains(&format!(" /external/music/api/v1/{path} ")));
            assert_eq!(headers.contains("Bearer qa-session"), authenticated);
            let len = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; len];
            socket.read_exact(&mut body).await.unwrap();
            bodies.push(if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&body).unwrap()
            });
            if let Some(response) = response {
                let response = response.to_string();
                socket.write_all(format!("HTTP/1.1 {status} QA\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
            } else {
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{",
                    )
                    .await
                    .unwrap();
            }
        }
        bodies
    });
    (config, task)
}
#[tokio::test]
async fn device_wait_preserves_the_frozen_cursor_for_later_explicit_play() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    let (handle,_)=checkpoint(store.clone(),false);
    let (config,task)=server(vec![
        ("status",false,200,Some(status("core-a"))),
        ("zones/z/recover",true,200,Some(json!({"accepted":false,"waiting_for_play":true,"zone":zone("old","paused")}))),
        ("status",false,200,Some(status("core-a"))),
        ("zones/z/recover",true,200,Some(json!({"accepted":false,"finished":true}))),
    ]).await;
    let sync=Sync::new(store.clone(),config.clone(),owner());
    assert!(restore(config.clone(),store.clone(),owner(),sync.clone()).await.unwrap().is_none());
    assert!(handle.is_current().unwrap());
    assert!(restore(config,store,owner(),sync).await.unwrap().is_none());
    let bodies=task.await.unwrap();assert_eq!(bodies[1],bodies[3]);assert!(!handle.is_current().unwrap());
}
#[tokio::test]
async fn background_ack_between_pending_read_and_flush_does_not_block_recovery() {
    use axum::{routing::{get,post},Json,Router};
    use std::sync::atomic::{AtomicUsize,Ordering};
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    let (handle,snapshot)=checkpoint(store.clone(),true);let snapshot=snapshot.unwrap();
    let entered=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());
    let statuses=Arc::new(AtomicUsize::new(0));let restores=Arc::new(AtomicUsize::new(0));
    let router=Router::new()
        .route("/external/music/api/v1/status",get({let statuses=statuses.clone();let release=release.clone();move ||{
            let count=statuses.fetch_add(1,Ordering::SeqCst);let release=release.clone();async move {
                if count==1 {tokio::spawn(async move {
                    // Let restore read the pending row and wait behind the background upload gate.
                    tokio::time::sleep(Duration::from_millis(125)).await;release.notify_one();
                });}
                Json(status("core-a"))
            }
        }}))
        .route("/external/music/api/v1/stats/offline",post({let entered=entered.clone();move ||{
            let entered=entered.clone();let release=release.clone();let snapshot=snapshot.clone();async move {
                entered.notify_one();release.notified().await;
                Json(json!({"accepted":[{"id":snapshot.id,"version":snapshot.version}],"rejected":[]}))
            }
        }}))
        .route("/external/music/api/v1/zones/z/recover",post({let restores=restores.clone();move ||{
            restores.fetch_add(1,Ordering::SeqCst);async {Json(json!({"accepted":false,"finished":true}))}
        }}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let server=tokio::spawn(async move {axum::serve(listener,router).await.unwrap()});
    let config=Config::new(ConnectionProfile {core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}/external/music"),core_id:Some("core-a".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]},"qa-session".into(),"endpoint".into()).unwrap();
    let sync=Sync::new(store.clone(),config.clone(),owner());
    let background={let sync=sync.clone();tokio::spawn(async move {sync.flush().await})};
    entered.notified().await;
    let result=restore(config,store.clone(),owner(),sync).await;
    assert_eq!(background.await.unwrap().unwrap(),1);
    assert!(result.unwrap().is_none());
    assert_eq!(statuses.load(Ordering::SeqCst),2);assert_eq!(restores.load(Ordering::SeqCst),1);
    assert!(!handle.ready().unwrap());server.abort();
}
#[tokio::test]
async fn listening_flush_precedes_frozen_restore_retries_and_latest_controls_win() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let (_, snapshot) = checkpoint(store.clone(), true);
    let snapshot = snapshot.unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let latest = zone(&id, "paused");
    let (config, task) = server(vec![
        ("status", false, 200, Some(status("core-a"))),
        ("status", false, 200, Some(status("core-a"))),
        (
            "stats/offline",
            true,
            200,
            Some(json!({"accepted":[{"id":snapshot.id,"version":snapshot.version}],"rejected":[]})),
        ),
        ("zones/z/recover", true, 200, None),
        (
            "zones/z/recover",
            true,
            200,
            Some(json!({"accepted":true,"zone":zone(&id,"playing"),"finished":false})),
        ),
        ("zones/z", true, 200, Some(latest.clone())),
        (
            "library/tracks/track/ticket",
            true,
            200,
            Some(json!({"api_path":"api/v1/media/track?ticket=fresh-qa"})),
        ),
    ])
    .await;
    let sync = Sync::new(store.clone(), config.clone(), owner());
    let result = restore(config, store.clone(), owner(), sync)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.command["paused"], true);
    assert_eq!(result.command["volume"], 0.17);
    assert_eq!(result.command["muted"], true);
    assert_eq!(result.command["playback_id"], id);
    assert_eq!(result.command["url"], "api/v1/media/track?ticket=fresh-qa");
    let requests = task.await.unwrap();
    assert_eq!(requests[3], requests[4]);
    assert_eq!(
        requests[2]["sessions"][0]["measured"]["playback_id"],
        requests[3]["heard"][0]["playback_id"]
    );
    assert!(store.pending(&owner()).unwrap().is_empty());
    assert_eq!(
        cursor::freeze(store, &owner(), "endpoint")
            .unwrap()
            .unwrap()
            .0
            .request,
        Some(requests[3].clone())
    );
}
#[tokio::test]
async fn a_newer_stop_after_restore_ack_prevents_any_media_ticket_or_output() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    checkpoint(store.clone(), false);
    let id = uuid::Uuid::new_v4().to_string();
    let mut stopped = zone(&id, "stopped");
    stopped["playback_id"] = Value::Null;
    let (config, task) = server(vec![
        ("status", false, 200, Some(status("core-a"))),
        (
            "zones/z/recover",
            true,
            200,
            Some(json!({"accepted":true,"zone":zone(&id,"playing")})),
        ),
        ("zones/z", true, 200, Some(stopped)),
    ])
    .await;
    let sync = Sync::new(store.clone(), config.clone(), owner());
    assert!(restore(config, store.clone(), owner(), sync)
        .await
        .unwrap()
        .is_none());
    task.await.unwrap();
    store.drain().unwrap();
    assert!(cursor::freeze(store, &owner(), "endpoint")
        .unwrap()
        .is_none());
}
#[tokio::test]
async fn rejected_authorization_retains_the_original_account_cursor_without_retry() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    checkpoint(store.clone(), false);
    let (config, task) = server(vec![
        ("status", false, 200, Some(status("core-a"))),
        (
            "zones/z/recover",
            true,
            403,
            Some(json!({"error":"forbidden"})),
        ),
    ])
    .await;
    let sync = Sync::new(store.clone(), config.clone(), owner());
    assert!(restore(config, store.clone(), owner(), sync).await.is_err());
    task.await.unwrap();
    assert!(cursor::freeze(store, &owner(), "endpoint")
        .unwrap()
        .is_some());
}
#[tokio::test]
async fn changed_core_or_another_account_never_receive_saved_recovery_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    checkpoint(store.clone(), false);
    let (config, task) = server(vec![("status", false, 200, Some(status("changed-core")))]).await;
    let sync = Sync::new(store.clone(), config.clone(), owner());
    assert!(restore(config.clone(), store.clone(), owner(), sync)
        .await
        .is_err());
    task.await.unwrap();
    let other = Owner {
        core_id: "core-a".into(),
        user_id: "user-b".into(),
    };
    let (config,task)=server(vec![("status",false,200,Some(status("core-a")))]).await;
    let sync = Sync::new(store.clone(), config.clone(), other.clone());
    assert!(restore(config, store.clone(), other, sync)
        .await
        .unwrap()
        .is_none());
    task.await.unwrap();
    assert!(cursor::freeze(store, &owner(), "endpoint")
        .unwrap()
        .is_some());
}
#[tokio::test]
async fn an_empty_local_journal_notifies_only_its_registered_endpoint_and_negotiated_core() {
    let dir=tempfile::tempdir().unwrap();let store=Store::open(dir.path()).unwrap();
    let mut available=status("core-a");available["capabilities"]["recovery_controls"]=json!(true);
    let mut pending=zone("old","playing");pending["playback_id"]=Value::Null;
    let mut foreign=pending.clone();foreign["endpoint_id"]=json!("elsewhere");
    let result=zone("new","playing");
    let (config,task)=server(vec![
        ("status",false,200,Some(available)),
        ("zones",true,200,Some(json!({"items":[foreign,pending]}))),
        ("zones/z/recover/unavailable",true,200,Some(json!({"applied":true,"zone":result}))),
    ]).await;
    assert_eq!(config.recovery_unavailable(&owner()).await.unwrap().unwrap()["playback_id"],"new");
    let bodies=task.await.unwrap();assert_eq!(bodies[2],json!({"endpoint_id":"endpoint"}));
    assert!(store.pending(&owner()).unwrap().is_empty());
}
#[tokio::test]
async fn a_rejected_source_snapshot_cannot_be_skipped_by_restoring_a_newer_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let (_, snapshot) = checkpoint(store.clone(), true);
    let snapshot = snapshot.unwrap();
    store
        .acknowledge(
            &owner(),
            &crate::listens::SyncResult {
                accepted: vec![],
                rejected: vec![crate::listens::Rejected {
                    id: snapshot.id,
                    version: snapshot.version,
                    code: "invalid_snapshot".into(),
                }],
            },
        )
        .unwrap();
    let (config, task) = server(vec![("status", false, 200, Some(status("core-a")))]).await;
    let sync = Sync::new(store.clone(), config.clone(), owner());
    let result = restore(config, store.clone(), owner(), sync).await;
    assert!(result
        .err()
        .unwrap()
        .to_string()
        .contains("source listening journal was rejected"));
    task.await.unwrap();
    assert!(cursor::freeze(store, &owner(), "endpoint")
        .unwrap()
        .is_some());
}
#[tokio::test]
async fn failure_after_binding_reports_only_the_new_instance_for_actor_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    checkpoint(store.clone(), false);
    let id = uuid::Uuid::new_v4().to_string();
    let active = zone(&id, "playing");
    let (config, task) = server(vec![
        ("status", false, 200, Some(status("core-a"))),
        (
            "zones/z/recover",
            true,
            200,
            Some(json!({"accepted":true,"zone":active})),
        ),
        ("zones/z", true, 200, Some(zone(&id, "playing"))),
        (
            "library/tracks/track/ticket",
            true,
            403,
            Some(json!({"error":"forbidden"})),
        ),
    ])
    .await;
    let sync = Sync::new(store.clone(), config.clone(), owner());
    let error = restore(config, store.clone(), owner(), sync)
        .await
        .err()
        .unwrap();
    let failure = error.downcast_ref::<BoundFailure>().unwrap();
    assert_eq!(failure.report["core_playback_id"], id);
    assert_eq!(failure.report["track_id"], "track");
    failure.handle.retire().unwrap();
    store.drain().unwrap();
    assert!(cursor::freeze(store, &owner(), "endpoint")
        .unwrap()
        .is_none());
    task.await.unwrap();
}
