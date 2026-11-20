use super::*;
use super::tests::{running, server, play, wav};
use axum::{routing::{get,post}, Json, Router};

async fn pending(profile: &ConnectionProfile) -> Playback {
    let audio = running(profile).await.audio.clone();
    let control = Arc::new(PlaybackControl::default());
    let plan = Plan { zone_id:"z".into(), track_id:"old".into(), core_playback_id:Some("core-old".into()), position:0., dsp:Default::default(), impulse:None, measured_lufs:None, transitions:Default::default(), control };
    let mut playback = Playback::connected(profile.clone(),"qa".into(),"endpoint".into()).unwrap();
    playback.pending = Some(Pending::new(plan,None,Some(audio)));
    playback
}

#[tokio::test]
async fn superseded_queue_preparation_releases_resources_without_reporting_a_playback_failure() {
    for current in [
        json!({"id":"z","endpoint_id":"endpoint","status":"playing","playback_id":"core-new","track":{"id":"new"}}),
        json!({"id":"z","endpoint_id":"endpoint","status":"paused","playback_id":null,"track":{"id":"old"}}),
        json!({"id":"z","endpoint_id":"endpoint","status":"stopped","playback_id":null,"track":null}),
    ] {
        let (profile, server) = server(Router::new().route("/api/v1/zones/z/prepare",post(move || {let current=current.clone();async move {Json(json!({"accepted":false,"zone":current}))}}))).await;
        let mut playback = pending(&profile).await;
        let control = playback.pending.as_ref().unwrap().plan.control.clone();
        let proxy = Arc::downgrade(&playback.pending.as_ref().unwrap().audio.as_ref().unwrap().proxy);
        let report = tokio::time::timeout(Duration::from_secs(5),playback.update()).await.unwrap();
        assert!(report.is_none(),"superseded request must be cancellation, got {report:?}");
        assert!(playback.pending.is_none() && playback.running.is_none());
        assert!(control.stopped.load(Ordering::Acquire));
        assert!(proxy.upgrade().is_none(),"cancelled preparation retained the media source");
        assert!(playback.take_ready().is_none(),"obsolete audio may never reach the device");
        playback.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn rejected_preparation_keeps_real_transport_and_contract_failures_visible() {
    for response in [json!({"accepted":false,"zone":{"id":"foreign","endpoint_id":"endpoint"}}),json!({"accepted":false})] {
        let (profile, server) = server(Router::new().route("/api/v1/zones/z/prepare",post(move || {let response=response.clone();async move {Json(response)}}))).await;
        let mut playback = pending(&profile).await;
        let report = tokio::time::timeout(Duration::from_secs(5),playback.update()).await.unwrap().expect("invalid authoritative identity must fail");
        assert_eq!(report["type"],"error"); assert!(playback.pending.is_none());
        playback.shutdown().await; server.abort();
    }
}

#[tokio::test]
async fn a_new_play_request_succeeds_after_a_superseded_preparation() {
    let router = Router::new()
        .route("/api/v1/media/new",get(|| async {wav()}))
        .route("/api/v1/zones/z/prepare",post(|Json(body):Json<Value>|async move {
            Json(if body["playback_id"]=="core-old" {
                json!({"accepted":false,"zone":{"id":"z","endpoint_id":"endpoint","status":"playing","playback_id":"core-new","track":{"id":"new"}}})
            } else { json!({"accepted":true,"plan_id":"current-plan","items":[]}) })
        }));
    let (profile, server) = server(router).await;
    let mut playback = pending(&profile).await;
    assert!(tokio::time::timeout(Duration::from_secs(5),playback.update()).await.unwrap().is_none());
    assert!(playback.pending.is_none());
    playback.command("play",&play(&profile,"new","new"),&profile).unwrap();
    tokio::time::timeout(Duration::from_secs(5),async {
        while playback.pending.as_ref().is_some_and(|p|p.audio.is_none()||p.queue.is_none()) {
            assert!(playback.update().await.is_none());
        }
    }).await.unwrap();
    let ready = playback.take_ready().expect("fresh playback should be admitted");
    assert_eq!(ready.plan.track_id,"new");
    assert_eq!(ready.plan.core_playback_id.as_deref(),Some("core-new"));
    assert_eq!(ready.queue.unwrap().unwrap().plan_id,"current-plan");
    assert!(!ready.plan.control.stopped.load(Ordering::Acquire));
    // No output is opened by this acceptance test.
    playback.shutdown().await; server.abort();
}

#[tokio::test]
async fn real_queue_transport_failure_is_not_suppressed_as_supersession() {
    let (profile, server) = server(Router::new().route("/api/v1/zones/z/prepare",post(||async{axum::http::StatusCode::SERVICE_UNAVAILABLE}))).await;
    let mut playback = pending(&profile).await;
    let report = tokio::time::timeout(Duration::from_secs(5),playback.update()).await.unwrap().expect("transport failure must remain visible");
    assert_eq!(report["type"],"error"); assert_eq!(report["core_playback_id"],"core-old");
    assert!(playback.pending.is_none()); playback.shutdown().await; server.abort();
}
