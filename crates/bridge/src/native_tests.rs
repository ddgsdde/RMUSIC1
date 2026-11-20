//! Authenticated embedded WebSocket lifecycle only; no prepare command or audio device.
use super::*;
use axum::{extract::ws::{WebSocketUpgrade,Message as WsMessage},routing::get,Json,Router};

#[test]
fn ordinary_progress_is_coalesced_without_delaying_playback_transitions() {
    let start = std::time::Instant::now();
    let mut reports = StateReports::default();
    let mut progress = json!({"type":"state","status":"playing","track_id":"song","core_playback_id":"first","frames":0});
    let mut delivered = 0;
    for step in 0..50 {
        progress["frames"] = json!(step * 960);
        if reports.due(&progress, start + Duration::from_millis(step * 20)) {
            delivered += 1;
        }
    }
    assert_eq!(delivered, 4);
    let now = start + Duration::from_secs(1);
    progress["signal_path"] = json!({"processing":["Playback buffering: source data pending"]});
    assert!(reports.due(&progress, now));
    progress["status"] = json!("paused");
    assert!(reports.due(&progress, now));
    progress["core_playback_id"] = json!("next");
    progress["frames"] = json!(0);
    assert!(reports.due(&progress, now));
    assert!(reports.due(&json!({"type":"error"}), now));
}
struct Host(tokio::sync::watch::Sender<bool>);
impl EmbeddedHost for Host {
    fn prepare<'a>(&'a self,_:&'a Value)->futures_util::future::BoxFuture<'a,Result<()>>{Box::pin(async{bail!("unexpected prepare in no-output fixture")})}
    fn report(&self,v:&Value){if v["type"]=="registered"{self.0.send_replace(true);}}
    fn released(&self){}
}
#[tokio::test]
async fn group_final_v2_embedded_journal_authentication_preserves_the_personal_recovery_cursor() {
    let dir=tempfile::tempdir().unwrap();let store=ListenStore::open(dir.path()).unwrap();let owner=crate::listens::Owner{core_id:"qa-core".into(),user_id:"qa-user".into()};let endpoint="browser-qa";
    let _recorder=crate::listens::cursor::Recorder::begin_timed(store.clone(),owner.clone(),endpoint.into(),"z".into(),"plan".into(),768000,crate::listens::cursor::Boundary{playback_id:"ordinary".into(),track_id:"t".into(),position:0.,frames:0},2).unwrap();store.drain().unwrap();let(_,handle)=crate::listens::cursor::freeze(store.clone(),&owner,endpoint).unwrap().unwrap();
    let (sent,received)=tokio::sync::oneshot::channel();let sent=Arc::new(std::sync::Mutex::new(Some(sent)));let capture=sent.clone();
    let router=Router::new().route("/prefix/api/v1/status",get(||async{Json(json!({"core_id":"qa-core","capabilities":{"measured_listens":true,"continuous_recovery":true}}))})).route("/prefix/api/v1/auth/me",get(||async{Json(json!({"id":"qa-user"}))})).route("/prefix/api/v1/bridge/events",get(move|ws:WebSocketUpgrade|{let capture=capture.clone();async move{ws.on_upgrade(move|mut socket|async move{let Some(Ok(WsMessage::Text(value)))=socket.recv().await else{panic!("no native registration")};capture.lock().unwrap().take().unwrap().send(serde_json::from_str::<Value>(&value).unwrap()).unwrap();socket.send(WsMessage::Text(json!({"type":"registered","endpoint_id":"browser-qa"}).to_string().into())).await.unwrap();while socket.recv().await.is_some(){}})}}));
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});let profile=ConnectionProfile{core_public_key:None,id:"embedded-qa".into(),name:"QA".into(),address:format!("http://{address}/prefix"),core_id:Some("qa-core".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]};let(stop,mut closed)=tokio::sync::watch::channel(false);let host=Host(stop);
    tokio::time::timeout(Duration::from_secs(3),embedded_session(&profile,"qa-token",endpoint,"QA",Default::default(),Some(store.clone()),async{if !*closed.borrow(){closed.changed().await.unwrap();}},&host)).await.unwrap().unwrap();
    let registration=received.await.unwrap();assert_eq!(registration["endpoint"]["capabilities"]["group_output_final_v2"],cfg!(any(windows,target_os="linux")));assert_eq!(store.owner(&profile.id).unwrap(),Some(owner));assert!(handle.is_current().unwrap(),"embedded group shutdown must not retire the desktop ordinary cursor");server.abort();
}
