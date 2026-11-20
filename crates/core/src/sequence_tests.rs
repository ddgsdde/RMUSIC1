use super::*;
use crate::models::Zone;
#[path = "sequence/recovery_tests.rs"]
mod recovery;
#[path="sequence/dual_listen_tests.rs"]
mod dual_listens;

const ZONE: &str = "/api/v1/zones/zone-local";
#[tokio::test]
async fn native_dsd_byte_clock_reports_preserve_statistics_and_reject_unsupported_rates() {
    let f=fixture(&["local-shared","local-unknown"]).await;
    let (_,status)=f.call("GET","/api/v1/status",None).await;
    assert_eq!(status["capabilities"]["frame_clock_max_rate"],3072000);
    let mut model=f.s.zones.read().await["zone-local"].clone();
    for (track,clock) in model.queue.iter_mut().zip([22579200,24576000]){track.sample_rate=clock;track.bit_depth=1;track.format="dsf".into();}
    model.track=Some(model.queue[0].clone());f.s.save_zone(model).await.unwrap();
    let initial=zone(&f).await;
    let body=json!({"track_id":"local-shared","playback_id":initial["playback_id"],"position":0.01,"frames":28224,"sample_rate":2822400});
    let mut bad=body.clone();bad["sample_rate"]=json!(3072001);
    assert_eq!(f.call("POST",&format!("{ZONE}/ended"),Some(bad)).await.0,StatusCode::BAD_REQUEST);
    let (status,next)=f.call("POST",&format!("{ZONE}/ended"),Some(body.clone())).await;assert_eq!(status,StatusCode::OK,"{next}");
    let second=json!({"track_id":"local-unknown","playback_id":next["playback_id"],"position":0.02,"frames":61440,"sample_rate":3072000});
    let (status,stopped)=f.call("POST",&format!("{ZONE}/ended"),Some(second.clone())).await;assert_eq!(status,StatusCode::OK,"{stopped}");assert_eq!(stopped["status"],"stopped");
    let before=totals(&f).await;let seconds:f64=before["tracks"].as_array().unwrap().iter().map(|row|row["seconds"].as_f64().unwrap()).sum();assert!((seconds-0.03).abs()<1e-12);
    for report in [body,second]{f.call("POST",&format!("{ZONE}/ended"),Some(report)).await;}
    assert_eq!(totals(&f).await,before);
}
#[tokio::test]
async fn native_format_transition_eof_advances_reserved_queue_with_independent_sample_clocks() {
    let f = fixture(&["local-shared", "local-unknown"]).await;
    let mut model = f.s.zones.read().await["zone-local"].clone();
    model.track.as_mut().unwrap().sample_rate = 48000;
    model.queue[0].sample_rate = 48000;
    model.queue[1].sample_rate = 768000;
    model.queue[1].bit_depth = 32;
    f.s.save_zone(model).await.unwrap();
    let original = zone(&f).await;
    let plan = prepare(&f).await;
    let reserved = plan["items"][0]["playback_id"].clone();
    let first = json!({"track_id":"local-shared","playback_id":original["playback_id"],
        "position":333./48000.,"frames":333,"sample_rate":48000});
    let (status, next) = f.call("POST", &format!("{ZONE}/ended"), Some(first.clone())).await;
    assert_eq!(status, StatusCode::OK, "{next}");
    assert_eq!(next["status"], "playing");
    assert_eq!(next["track"]["sample_rate"], 768000);
    assert_eq!(next["track"]["bit_depth"], 32);
    assert_ne!(next["playback_id"], original["playback_id"]);
    assert_ne!(next["playback_id"], reserved);
    assert_eq!(next["position"], 0.);
    let second = json!({"track_id":"local-unknown","playback_id":next["playback_id"],
        "position":2051./768000.,"frames":2051,"sample_rate":768000});
    let (status, stopped) = f.call("POST", &format!("{ZONE}/ended"), Some(second.clone())).await;
    assert_eq!(status, StatusCode::OK, "{stopped}");
    assert_eq!(stopped["status"], "stopped");
    let before = totals(&f).await;
    let seconds: f64 = before["tracks"].as_array().unwrap().iter().map(|track| track["seconds"].as_f64().unwrap()).sum();
    assert!((seconds - (333./48000. + 2051./768000.)).abs() < 1e-9);
    for body in [first, second] {f.call("POST", &format!("{ZONE}/ended"), Some(body)).await;}
    assert_eq!(zone(&f).await, stopped);
    assert_eq!(totals(&f).await, before);
}
#[tokio::test]
async fn reserved_recommendations_remain_ready_for_ordinary_eof_fallback() {
    let f=Fixture::new().await;let seed=f.recommendations_fixture().await;
    f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-continuous","name":"continuous","playback_instance":true,"continuous_pcm":true}))).await;
    f.call("PATCH",ZONE,Some(json!({"autoplay":{"enabled":true,"source":"local","fallback_local":true}}))).await;
    f.call("PUT",&format!("{ZONE}/queue"),Some(json!({"track_ids":[seed]}))).await;
    let mut events=f.s.events.subscribe();f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;
    tokio::time::timeout(std::time::Duration::from_secs(3),async{loop{let event=events.recv().await.unwrap();if event["type"]=="autoplay"&&event["data"]["status"]=="ready"{break;}}}).await.unwrap();
    let z=zone(&f).await;let p=prepare(&f).await;assert!(!p["items"].as_array().unwrap().is_empty());elapsed(&f,60).await;
    let(status,next)=f.call("POST",&format!("{ZONE}/ended"),Some(json!({"track_id":seed,"playback_id":z["playback_id"],"position":1.,"frames":48000,"sample_rate":48000}))).await;
    assert_eq!(status,StatusCode::OK,"{next}");assert_eq!(next["status"],"playing","reserving future recommendations consumed the live EOF fallback: {next}");
}
#[tokio::test]
async fn ready_recommendations_are_reserved_without_advancing_the_live_queue_or_statistics() {
    let f=Fixture::new().await;let seed=f.recommendations_fixture().await;
    f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-continuous","name":"continuous","playback_instance":true,"continuous_pcm":true}))).await;
    f.call("PATCH",ZONE,Some(json!({"autoplay":{"enabled":true,"source":"local","fallback_local":true}}))).await;
    f.call("PUT",&format!("{ZONE}/queue"),Some(json!({"track_ids":[seed]}))).await;
    let mut events=f.s.events.subscribe();f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;
    tokio::time::timeout(std::time::Duration::from_secs(3),async{loop{let event=events.recv().await.unwrap();if event["type"]=="autoplay"&&event["data"]["status"]=="ready"{break;}}}).await.unwrap();
    let z=zone(&f).await;let p=prepare(&f).await;
    assert_eq!(p["items"].as_array().unwrap().len(),6,"ready recommendations were not reserved: {p}");
    assert_eq!(p["items"][0]["queue"].as_array().unwrap().len(),7);assert_eq!(p["items"][0]["queue_index"],1);
    assert_eq!(p["refresh"],false,"unheard recommended queues must not replace the live fallback batch");
    assert_eq!(zone(&f).await["queue"].as_array().unwrap().len(),1);assert_eq!(zone(&f).await["playback_id"],z["playback_id"]);
    let again=prepare(&f).await;assert_eq!(again["plan_id"],p["plan_id"]);assert_eq!(again["items"].as_array().unwrap().iter().map(|slot|&slot["playback_id"]).collect::<Vec<_>>(),p["items"].as_array().unwrap().iter().map(|slot|&slot["playback_id"]).collect::<Vec<_>>());
    assert_eq!(f.s.db.run(|c|Ok(c.query_row("SELECT COALESCE(SUM(play_seconds),0) FROM track_stats",[],|r|r.get::<_,f64>(0))?)).await.unwrap(),0.);
    elapsed(&f,60).await;
    let(status,result)=f.call("POST",&format!("{ZONE}/handoff"),Some(handoff(&z,&p,1,1.,0.,0.25))).await;
    assert_eq!(status,StatusCode::OK,"{result}");assert_eq!(result["accepted"],true);assert_eq!(result["zone"]["queue_index"],1);assert_eq!(result["zone"]["queue"].as_array().unwrap().len(),7);
    assert_eq!(result["zone"]["track"]["id"],p["items"][0]["track"]["id"]);
    assert!((f.s.db.run(|c|Ok(c.query_row("SELECT COALESCE(SUM(play_seconds),0) FROM track_stats",[],|r|r.get::<_,f64>(0))?)).await.unwrap()-1.25).abs()<1e-7);
}
async fn fixture(queue: &[&str]) -> Fixture {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    assert_eq!(f.call("PATCH", ZONE, Some(json!({"autoplay":{
        "enabled":false,"source":"local","fallback_local":true
    }}))).await.0, StatusCode::OK);
    assert_eq!(f.call("POST", "/api/v1/endpoints/browser", Some(json!({
        "id":"browser-continuous","name":"continuous","playback_instance":true,"continuous_pcm":true
    }))).await.0, StatusCode::OK);
    assert_eq!(
        f.call(
            "PUT",
            &format!("{ZONE}/queue"),
            Some(json!({"track_ids":queue}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("POST", &format!("{ZONE}/play"), Some(json!({})))
            .await
            .0,
        StatusCode::OK
    );
    f
}
async fn zone(f: &Fixture) -> Value {
    f.call("GET", ZONE, None).await.1
}
async fn prepare(f: &Fixture) -> Value {
    let z = zone(f).await;
    let (status, value) = f.call("POST", &format!("{ZONE}/prepare"), Some(json!({
        "endpoint_id":z["endpoint_id"],"track_id":z["track"]["id"],"playback_id":z["playback_id"]
    }))).await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}
fn boundary(id: &Value, track: &Value, seconds: f64) -> Value {
    json!({"playback_id":id,"track_id":track,"position":seconds,"frames":(seconds*48000.).round() as u64})
}
fn handoff(z: &Value, p: &Value, count: usize, first: f64, later: f64, current: f64) -> Value {
    let mut ended = vec![boundary(&z["playback_id"], &z["track"]["id"], first)];
    for slot in p["items"].as_array().unwrap().iter().take(count - 1) {
        ended.push(boundary(&slot["playback_id"], &slot["track"]["id"], later));
    }
    let slot = &p["items"][count - 1];
    json!({"endpoint_id":z["endpoint_id"],"request_id":uuid::Uuid::new_v4().to_string(),
        "plan_id":p["plan_id"],"from_playback_id":z["playback_id"],"sample_rate":48000,
        "ended":ended,"current":boundary(&slot["playback_id"],&slot["track"]["id"],current)})
}
async fn elapsed(f: &Fixture, seconds: i64) {
    f.s.db.run(move |c| { c.execute("UPDATE play_sessions SET last_report_at=last_report_at-?1 WHERE finished_at IS NULL", [seconds * 1000])?; Ok(()) }).await.unwrap();
}
async fn totals(f: &Fixture) -> Value {
    f.s.db.run(|c| {
        let mut s = c.prepare("SELECT track_id,play_seconds,play_count,skip_count FROM track_stats ORDER BY track_id")?;
        let tracks = s.query_map([], |r| Ok(json!({"id":r.get::<_,String>(0)?,"seconds":r.get::<_,f64>(1)?,"plays":r.get::<_,i64>(2)?,"skips":r.get::<_,i64>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
        let sessions: i64 = c.query_row("SELECT COUNT(*) FROM play_sessions", [], |r| r.get(0))?;
        let receipts: i64 = c.query_row("SELECT COUNT(*) FROM sequence_receipts", [], |r| r.get(0))?;
        Ok(json!({"tracks":tracks,"sessions":sessions,"receipts":receipts}))
    }).await.unwrap()
}
fn measured_snapshot(playback:&Value,listen:&Value,track:&Value,from:f64,to:f64,reason:&str)->Value {
    let now=(chrono::Utc::now().timestamp()/86400*86400-43200)*1000;
    let day=chrono::DateTime::from_timestamp_millis(now).unwrap().format("%Y-%m-%d").to_string();
    json!({"id":uuid::Uuid::new_v4().to_string(),"version":1,"track_id":track,"title":"measured QA","artist":"QA","duration":30.,"started_at":now-60000,"updated_at":now,"position":to,"reason":reason,
        "days":{(day):{"seconds":to-from,"last_at":now}},"measured":{"playback_id":playback,"listen_session_id":listen,"segments":[{"start":from,"end":to,"last_at":now}]}})
}
async fn measured_batch(f:&Fixture,sessions:Vec<Value>)->Value {
    let (_,user)=f.call("GET","/api/v1/auth/me",None).await;
    json!({"core_id":*f.s.core_id,"user_id":user["id"],"client_id":uuid::Uuid::new_v4().to_string(),"sessions":sessions})
}
async fn device_progress(f:&Fixture,z:&Value,position:f64,seconds:f64) {
    elapsed(f,60).await;
    assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"position":position,"frames":(seconds*48000.).round() as u64,"sample_rate":48000}))).await.0,StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn measured_offline_and_online_reports_share_seconds_count_and_source_intervals() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    device_progress(&f,&z,20.,20.).await;
    let snapshot=measured_snapshot(&z["playback_id"],&p["listen_session_id"],&z["track"]["id"],0.,25.,"active");
    let mut batch=measured_batch(&f,vec![snapshot]).await;
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch.clone())).await.1["rejected"],json!([]));
    assert_eq!(totals(&f).await["tracks"][0],json!({"id":"local-shared","seconds":25.,"plays":1,"skips":0}));
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch.clone())).await.1["changed"],false);
    device_progress(&f,&z,27.,27.).await;assert_eq!(totals(&f).await["tracks"][0]["seconds"],27.);
    elapsed(&f,60).await;
    f.call("POST",&format!("{ZONE}/ended"),Some(json!({"track_id":z["track"]["id"],"playback_id":z["playback_id"],"position":30.,"frames":1440000,"sample_rate":48000}))).await;
    let s=&mut batch["sessions"][0];s["version"]=json!(2);s["position"]=json!(30.);s["reason"]=json!("ended");s["updated_at"]=json!(s["updated_at"].as_i64().unwrap()+1000);
    let updated=s["updated_at"].clone();let day=s["days"].as_object_mut().unwrap().values_mut().next().unwrap();day["seconds"]=json!(30.);day["last_at"]=updated;
    s["measured"]["segments"][0]["end"]=json!(30.);s["measured"]["segments"][0]["last_at"]=s["updated_at"].clone();
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch)).await.1["rejected"],json!([]));
    assert_eq!(totals(&f).await["tracks"][0],json!({"id":"local-shared","seconds":30.,"plays":1,"skips":0}));assert_eq!(totals(&f).await["sessions"],2);
}

#[tokio::test]
async fn measured_snapshots_before_handoff_do_not_duplicate_future_sessions_or_credit() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    let batch=measured_batch(&f,vec![measured_snapshot(&z["playback_id"],&p["listen_session_id"],&z["track"]["id"],0.,30.,"ended"),
        measured_snapshot(&p["items"][0]["playback_id"],&p["items"][0]["listen_session_id"],&p["items"][0]["track"]["id"],0.,0.25,"active")]).await;
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch.clone())).await.1["rejected"],json!([]));
    elapsed(&f,60).await;let (status,next)=f.call("POST",&format!("{ZONE}/handoff"),Some(handoff(&z,&p,1,30.,0.,0.25))).await;
    assert_eq!(status,StatusCode::OK,"{next}");assert_eq!(next["accepted"],true);assert_eq!(totals(&f).await["sessions"],2);
    assert_eq!(totals(&f).await["tracks"][0]["seconds"],30.);assert_eq!(totals(&f).await["tracks"][1]["seconds"],0.25);
    device_progress(&f,&next["zone"],10.,10.).await;
    assert_eq!(totals(&f).await["tracks"][1]["seconds"],10.);f.call("POST","/api/v1/stats/offline",Some(batch)).await;
    assert_eq!(totals(&f).await["tracks"][1]["seconds"],10.);
}

#[tokio::test]
async fn late_measured_history_corrects_session_start_without_replacing_live_device_baselines() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    elapsed(&f,60).await;let (_,next)=f.call("POST",&format!("{ZONE}/handoff"),Some(handoff(&z,&p,1,30.,0.,0.25))).await;
    assert_eq!(next["accepted"],true);
    let listen=p["items"][0]["listen_session_id"].as_str().unwrap().to_owned();let id=listen.clone();
    let before=f.s.db.run(move |c|Ok(c.query_row("SELECT started_at,last_report_at,last_position,last_status,runtime_id FROM play_sessions WHERE id=?1",[id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,f64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?)).await.unwrap();
    let snapshot=measured_snapshot(&next["zone"]["playback_id"],&p["items"][0]["listen_session_id"],&next["zone"]["track"]["id"],0.,10.,"active");
    let started=snapshot["started_at"].as_i64().unwrap();assert!(started<before.0);
    let batch=measured_batch(&f,vec![snapshot]).await;
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch)).await.1["rejected"],json!([]));
    let after=f.s.db.run(move |c|Ok(c.query_row("SELECT started_at,last_report_at,last_position,last_status,runtime_id FROM play_sessions WHERE id=?1",[listen],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,f64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?)).await.unwrap();
    assert_eq!(after.0,started);assert_eq!((after.1,after.2,after.3,after.4),(before.1,before.2,before.3,before.4));
    assert_eq!(zone(&f).await,next["zone"]);
    device_progress(&f,&next["zone"],2.,2.).await;
    assert_eq!(totals(&f).await["tracks"][1]["seconds"],10.);
}

#[tokio::test]
async fn measured_seek_instances_share_one_listen_count_but_keep_separate_heard_ranges() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let first=prepare(&f).await;device_progress(&f,&z,20.,20.).await;
    let (_,seek)=f.call("POST",&format!("{ZONE}/seek"),Some(json!({"position":5.}))).await;let p=prepare(&f).await;
    assert_ne!(seek["playback_id"],z["playback_id"]);assert_eq!(p["listen_session_id"],first["listen_session_id"]);
    device_progress(&f,&seek,10.,5.).await;
    let batch=measured_batch(&f,vec![measured_snapshot(&seek["playback_id"],&p["listen_session_id"],&seek["track"]["id"],5.,10.,"active"),
        measured_snapshot(&z["playback_id"],&first["listen_session_id"],&z["track"]["id"],0.,20.,"interrupted")]).await;
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch)).await.1["rejected"],json!([]));
    assert_eq!(totals(&f).await["tracks"][0],json!({"id":"local-shared","seconds":25.,"plays":1,"skips":0}));assert_eq!(totals(&f).await["sessions"],1);
}

#[tokio::test]
async fn measured_claims_and_dedup_survive_core_restart_without_resuming_the_old_zone() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    let batch=measured_batch(&f,vec![measured_snapshot(&z["playback_id"],&p["listen_session_id"],&z["track"]["id"],0.,20.,"interrupted")]).await;
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch.clone())).await.1["rejected"],json!([]));
    let restored=app(AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap());
    assert_eq!(call(&restored,"POST","/api/v1/stats/offline",Some(&f.token),Some(batch.clone())).await.1["changed"],false);
    assert_eq!(call(&restored,"GET",ZONE,Some(&f.token),None).await.1["status"],"stopped");
    let mut unknown=batch;unknown["sessions"][0]["id"]=json!(uuid::Uuid::new_v4().to_string());unknown["sessions"][0]["measured"]["playback_id"]=json!(uuid::Uuid::new_v4().to_string());
    assert_eq!(call(&restored,"POST","/api/v1/stats/offline",Some(&f.token),Some(unknown)).await.1["rejected"][0]["code"],"invalid_snapshot");
    assert_eq!(totals(&f).await["tracks"][0],json!({"id":"local-shared","seconds":20.,"plays":1,"skips":0}));
}

#[tokio::test]
async fn measured_history_rejects_unissued_user_track_group_and_pre_seek_ranges() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    let original=measured_snapshot(&z["playback_id"],&p["listen_session_id"],&z["track"]["id"],0.,10.,"interrupted");
    for field in ["track","group","overlap"] {
        let mut snapshot=original.clone();
        match field {
            "track"=>snapshot["track_id"]=json!("local-unknown"),
            "group"=>snapshot["measured"]["listen_session_id"]=json!(uuid::Uuid::new_v4().to_string()),
            _=>{let mut extra=snapshot["measured"]["segments"][0].clone();extra["start"]=json!(5.);snapshot["measured"]["segments"].as_array_mut().unwrap().push(extra);}
        }
        let batch=measured_batch(&f,vec![snapshot]).await;
        assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch)).await.1["rejected"][0]["code"],"invalid_snapshot","{field}");
    }
    let token=crate::auth::random_token();let hash=crate::auth::hash(&token);
    f.s.db.run(move |c| {
        c.execute("INSERT INTO users(id,username,password,role) VALUES('unissued-user','unissued','unused','member')",[])?;
        c.execute("INSERT INTO sessions(hash,user_id,expires,scopes,created_at) VALUES(?1,'unissued-user',?2,'[\"control\"]',0)",rusqlite::params![hash,chrono::Utc::now().timestamp()+60])?;Ok(())
    }).await.unwrap();
    let mut batch=measured_batch(&f,vec![original]).await;batch["user_id"]=json!("unissued-user");
    let (status,result)=call(&f.app,"POST","/api/v1/stats/offline",Some(&token),Some(batch)).await;
    assert_eq!(status,StatusCode::OK);assert_eq!(result["rejected"][0]["code"],"invalid_snapshot");
    let (_,seek)=f.call("POST",&format!("{ZONE}/seek"),Some(json!({"position":5.}))).await;let p=prepare(&f).await;
    let snapshot=measured_snapshot(&seek["playback_id"],&p["listen_session_id"],&seek["track"]["id"],0.,10.,"interrupted");
    let batch=measured_batch(&f,vec![snapshot]).await;
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch)).await.1["rejected"][0]["code"],"invalid_snapshot");
    assert_eq!(totals(&f).await["tracks"],json!([]));
}

#[tokio::test]
async fn measured_eof_history_preserves_neutral_stop_without_resuming_audio() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;let p=prepare(&f).await;
    device_progress(&f,&z,10.,10.).await;f.call("POST",&format!("{ZONE}/stop"),Some(json!({}))).await;
    assert_eq!(totals(&f).await["tracks"][0]["skips"],0);
    let stopped=zone(&f).await;
    let batch=measured_batch(&f,vec![measured_snapshot(&z["playback_id"],&p["listen_session_id"],&z["track"]["id"],0.,30.,"ended")]).await;
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch.clone())).await.1["rejected"],json!([]));
    assert_eq!(totals(&f).await["tracks"][0],json!({"id":"local-shared","seconds":30.,"plays":1,"skips":0}));
    assert_eq!(f.call("POST","/api/v1/stats/offline",Some(batch)).await.1["changed"],false);
    assert_eq!(zone(&f).await,stopped);
}

#[tokio::test]
async fn preparing_is_idempotent_bounded_and_does_not_advance_or_count() {
    let f = fixture(&["local-shared", "local-unknown"]).await;
    let before = zone(&f).await;
    let stats = totals(&f).await;
    let first = prepare(&f).await;
    let second = prepare(&f).await;
    assert_eq!(first["plan_id"], second["plan_id"]);
    assert_eq!(
        first["items"][0]["playback_id"],
        second["items"][0]["playback_id"]
    );
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    assert_eq!(first["has_more"],false);
    assert_eq!(first["items"][0]["track"]["id"], "local-unknown");
    assert!(first["items"][0]["media_path"]
        .as_str()
        .unwrap()
        .starts_with("api/v1/media/local-unknown?ticket="));
    let ticket = first["items"][0]["media_path"]
        .as_str()
        .unwrap()
        .split("ticket=")
        .nth(1)
        .unwrap();
    assert!(f.s.check_ticket(ticket, "media:local-unknown").await);
    assert_eq!(zone(&f).await, before);
    assert_eq!(totals(&f).await, stats);
    f.call("PATCH", ZONE, Some(json!({"repeat":"one"}))).await;
    let repeated = prepare(&f).await;
    assert_ne!(first["plan_id"], repeated["plan_id"]);
    let slots = repeated["items"].as_array().unwrap();
    assert_eq!(slots.len(), 8);
    assert_eq!(repeated["has_more"],true);
    let ids: std::collections::HashSet<_> = slots
        .iter()
        .map(|s| s["playback_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 8);
    assert!(slots
        .iter()
        .all(|s| s["track"]["id"] == "local-shared" && s["playback_id"] != before["playback_id"]));
}

#[tokio::test]
async fn native_rest_frames_preserve_rounded_positions_through_pause_resume_and_terminal_eof() {
    let f=fixture(&["local-shared","local-unknown"]).await;
    let mut model=f.s.zones.read().await["zone-local"].clone();
    model.track.as_mut().unwrap().duration=0.002;model.queue[0].duration=0.002;
    f.s.save_zone(model).await.unwrap();
    let original=zone(&f).await;let id=original["playback_id"].clone();
    f.s.endpoints.write().await.get_mut("browser-continuous").unwrap()["last_seen_ms"]=json!(0);
    for (status,frames) in [("playing",113u64),("paused",240),("playing",333)] {
        if status=="paused" {f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await;}
        if frames==333 {f.call("POST",&format!("{ZONE}/play"),Some(json!({}))).await;}
        let body=json!({"track_id":"local-shared","playback_id":id,"status":status,"position":frames as f64/48000.,"frames":frames,"sample_rate":48000});
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(body.clone())).await.0,StatusCode::NO_CONTENT);
        assert_eq!(zone(&f).await["position"],frames as f64/48000.);
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(body)).await.0,StatusCode::NO_CONTENT);
    }
    assert!(f.s.endpoints.read().await["browser-continuous"]["last_seen_ms"].as_i64().unwrap()>0);
    let ended=json!({"track_id":"local-shared","playback_id":id,"position":333./48000.,"frames":333,"sample_rate":48000});
    let(status,next)=f.call("POST",&format!("{ZONE}/ended"),Some(ended.clone())).await;
    assert_eq!(status,StatusCode::OK);assert_eq!(next["track"]["id"],"local-unknown");
    let before=totals(&f).await;
    assert!((before["tracks"][0]["seconds"].as_f64().unwrap()-333./48000.).abs()<1e-9);
    assert_eq!(before["sessions"],2);
    assert_eq!(f.call("POST",&format!("{ZONE}/ended"),Some(ended)).await.1,next);
    assert_eq!(totals(&f).await,before);
}

#[tokio::test]
async fn native_rest_frames_require_registered_owner_and_reject_invalid_clock_without_advancing() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;
    let valid=json!({"track_id":"local-shared","playback_id":z["playback_id"],"position":0.125,"frames":6000,"sample_rate":48000});
    let(_,login)=call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"admin","password":"correct-test-password"}))).await;
    assert_eq!(call(&f.app,"POST",&format!("{ZONE}/progress"),login["token"].as_str(),Some(valid.clone())).await.0,StatusCode::FORBIDDEN);
    for body in [json!({"position":0.1,"frames":4800}),json!({"position":0.1,"sample_rate":48000}),
        json!({"position":-1.,"frames":4800,"sample_rate":48000}),json!({"position":0.1,"frames":4800,"sample_rate":1})] {
        assert_eq!(f.call("POST",&format!("{ZONE}/progress"),Some(body)).await.0,StatusCode::BAD_REQUEST);
    }
    let mut invalid=valid;invalid["position"]=json!(30.);invalid["frames"]=json!(1440000);
    assert_eq!(f.call("POST",&format!("{ZONE}/ended"),Some(invalid)).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(zone(&f).await,z);assert_eq!(totals(&f).await["sessions"],1);
}

#[tokio::test]
async fn native_rest_resume_keeps_time_for_frames_reported_after_a_gap() {
    let f = fixture(&["local-shared", "local-unknown"]).await;
    let playing = zone(&f).await;
    let first = json!({"track_id":"local-shared","playback_id":playing["playback_id"],
        "position":0.25,"frames":12000,"sample_rate":48000,"status":"playing"});
    assert_eq!(f.call("POST", &format!("{ZONE}/progress"), Some(first)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(zone(&f).await["position"], 0.25);
    elapsed(&f, 12).await;

    // The output continued while Core had no fresh frame checkpoint.
    f.call("POST", &format!("{ZONE}/pause"), Some(json!({}))).await;
    let resumed = f.call("POST", &format!("{ZONE}/play"), Some(json!({}))).await.1;
    assert_eq!(resumed["playback_id"], playing["playback_id"]);
    assert_eq!(resumed["position"], 0.25);

    let final_frames = json!({"track_id":"local-shared","playback_id":playing["playback_id"],
        "position":10.0,"frames":480000,"sample_rate":48000});
    let (status, advanced) = f.call("POST", &format!("{ZONE}/ended"), Some(final_frames.clone())).await;
    assert_eq!(status, StatusCode::OK, "{advanced}");
    assert_eq!(advanced["track"]["id"], "local-unknown");
    let stats = totals(&f).await;
    assert_eq!(stats["tracks"][0]["seconds"], 10.0);
    assert_eq!(f.call("POST", &format!("{ZONE}/ended"), Some(final_frames)).await.1, advanced);
    assert_eq!(totals(&f).await, stats);
}

#[tokio::test]
async fn native_rest_late_playing_report_does_not_resume_pause_but_actual_paused_frames_can_checkpoint() {
    let f=fixture(&["local-shared","local-unknown"]).await;let z=zone(&f).await;
    f.call("POST",&format!("{ZONE}/pause"),Some(json!({}))).await;let paused=zone(&f).await;
    let mut body=json!({"track_id":"local-shared","playback_id":z["playback_id"],"status":"playing","position":0.125,"frames":6000,"sample_rate":48000});
    f.call("POST",&format!("{ZONE}/progress"),Some(body.clone())).await;assert_eq!(zone(&f).await,paused);
    body["status"]=json!("paused");f.call("POST",&format!("{ZONE}/progress"),Some(body.clone())).await;
    body.as_object_mut().unwrap().remove("status");body["position"]=json!(0.25);body["frames"]=json!(12000);
    f.call("POST",&format!("{ZONE}/progress"),Some(body.clone())).await;
    let checkpoint=zone(&f).await;assert_eq!(checkpoint["status"],"paused");assert_eq!(checkpoint["position"],0.25);
    assert_eq!(f.call("POST",&format!("{ZONE}/ended"),Some(body)).await.1,checkpoint);
    assert_eq!(totals(&f).await["tracks"][0]["seconds"],0.25);
}

#[tokio::test]
async fn multiple_heard_boundaries_commit_once_with_fractional_stats_and_durable_receipt() {
    let f = fixture(&["local-shared", "local-unknown", "local-shared"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    elapsed(&f, 40).await;
    let body = handoff(&z, &p, 2, 30., 2., 0.125);
    let (status, result) = f
        .call("POST", &format!("{ZONE}/handoff"), Some(body.clone()))
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["applied"], true);
    assert_eq!(result["zone"]["queue_index"], 2);
    assert_eq!(result["zone"]["playback_id"], p["items"][1]["playback_id"]);
    assert_eq!(result["zone"]["position"], 0.125);
    let stats = totals(&f).await;
    assert_eq!(
        stats,
        json!({"tracks":[
        {"id":"local-shared","seconds":30.125,"plays":1,"skips":0},
        {"id":"local-unknown","seconds":2.,"plays":0,"skips":0}
    ],"sessions":3,"receipts":1})
    );
    let (_, duplicate) = f
        .call("POST", &format!("{ZONE}/handoff"), Some(body.clone()))
        .await;
    assert_eq!(duplicate["accepted"], true);
    assert_eq!(duplicate["applied"], false);
    assert_eq!(totals(&f).await, stats);
    let mut changed = body.clone();
    changed["current"]["position"] = json!(0.5);
    changed["current"]["frames"] = json!(24000);
    assert_eq!(
        f.call("POST", &format!("{ZONE}/handoff"), Some(changed))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let new_state = AppState::new(f.s.db.clone(), f.s.config.clone(), String::new())
        .await
        .unwrap();
    let new_app = app(new_state.clone());
    call(&new_app,"POST","/api/v1/endpoints/browser",Some(&f.token),Some(json!({"id":"browser-continuous","name":"restored","playback_instance":true,"continuous_pcm":true}))).await;
    let (status, recovered) = call(
        &new_app,
        "POST",
        &format!("{ZONE}/handoff"),
        Some(&f.token),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recovered["accepted"], true);
    assert_eq!(recovered["applied"], false);
    assert_eq!(recovered["zone"]["status"], "stopped");
    assert!(recovered["zone"]["playback_id"].is_null());
    assert_eq!(totals(&f).await, stats);
}

#[tokio::test]
async fn handoff_preserves_later_pause_gain_and_mute_and_reuses_remaining_slots() {
    let f = fixture(&["local-shared", "local-unknown"]).await;
    f.call("PATCH", ZONE, Some(json!({"repeat":"all"}))).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    f.call("POST", &format!("{ZONE}/pause"), Some(json!({})))
        .await;
    f.call("PATCH", ZONE, Some(json!({"volume":0.23,"muted":true})))
        .await;
    assert_eq!(prepare(&f).await["plan_id"], p["plan_id"]);
    elapsed(&f, 40).await;
    let (_, result) = f
        .call(
            "POST",
            &format!("{ZONE}/handoff"),
            Some(handoff(&z, &p, 1, 30., 0., 0.5)),
        )
        .await;
    assert_eq!(result["applied"], true, "{result}");
    assert_eq!(result["zone"]["status"], "paused");
    assert_eq!(result["zone"]["volume"], 0.23);
    assert_eq!(result["zone"]["muted"], true);
    let refilled = prepare(&f).await;
    assert_eq!(refilled["plan_id"], p["plan_id"]);
    assert_eq!(
        refilled["items"][0]["playback_id"],
        p["items"][1]["playback_id"]
    );
    assert_eq!(refilled["items"].as_array().unwrap().len(), 8);
    assert_ne!(
        refilled["items"][7]["playback_id"],
        p["items"][7]["playback_id"]
    );
    assert_eq!(totals(&f).await["tracks"][0]["seconds"], 30.);
}

#[tokio::test]
async fn stop_seek_queue_and_processing_mutations_cancel_old_reservations() {
    for (method, suffix, change) in [
        ("POST", "/stop", json!({})),
        ("POST", "/seek", json!({"position":10})),
        ("PUT", "/queue", json!({"track_ids":["local-shared"]})),
        ("PATCH", "", json!({"repeat":"one"})),
        ("PUT", "/dsp", json!({"enabled":true,"preamp_db":-3})),
    ] {
        let f = fixture(&["local-shared", "local-unknown"]).await;
        let z = zone(&f).await;
        let p = prepare(&f).await;
        let body = handoff(&z, &p, 1, 30., 0., 0.5);
        let mut events = f.s.events.subscribe();
        assert_eq!(
            f.call(method, &format!("{ZONE}{suffix}"), Some(change))
                .await
                .0,
            StatusCode::OK
        );
        let mut invalidated = false;
        while let Ok(event) = events.try_recv() {
            if event["type"] == "sequence_invalidated" {
                assert_eq!(event["data"]["plan_id"], p["plan_id"]);
                invalidated = true;
            }
        }
        assert!(invalidated, "{suffix}");
        let before = zone(&f).await;
        let stats = totals(&f).await;
        let (status, result) = f.call("POST", &format!("{ZONE}/handoff"), Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["accepted"], false);
        assert_eq!(zone(&f).await, before);
        assert_eq!(totals(&f).await, stats);
    }
}

#[tokio::test]
async fn wrong_order_position_and_clock_budget_cannot_advance_or_credit() {
    let f = fixture(&["local-shared", "local-unknown", "local-shared"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let before = totals(&f).await;
    let body = handoff(&z, &p, 1, 30., 0., 0.5);
    for value in [
        body.clone(),
        {
            let mut b = body.clone();
            b["current"]["playback_id"] = p["items"][1]["playback_id"].clone();
            b
        },
        {
            let mut b = body.clone();
            b["current"]["frames"] = json!(1);
            b
        },
        {
            let mut b = body.clone();
            b["ended"][0]["track_id"] = json!("local-unknown");
            b
        },
    ] {
        assert_eq!(
            f.call("POST", &format!("{ZONE}/handoff"), Some(value))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(zone(&f).await, z);
        assert_eq!(totals(&f).await, before);
    }
    elapsed(&f, 40).await;
    assert_eq!(
        f.call("POST", &format!("{ZONE}/handoff"), Some(body))
            .await
            .1["applied"],
        true
    );
}

#[tokio::test]
async fn failed_receipt_write_rolls_back_zone_and_every_statistics_update() {
    let f = fixture(&["local-shared", "local-unknown", "local-shared"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    elapsed(&f, 40).await;
    let body = handoff(&z, &p, 2, 30., 2., 0.125);
    let before = totals(&f).await;
    f.s.db.run(|c| { c.execute_batch("CREATE TRIGGER reject_receipt BEFORE INSERT ON sequence_receipts BEGIN SELECT RAISE(ABORT,'injected failure'); END;")?; Ok(()) }).await.unwrap();
    assert_eq!(
        f.call("POST", &format!("{ZONE}/handoff"), Some(body.clone()))
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(zone(&f).await, z);
    assert_eq!(totals(&f).await, before);
    let durable: Zone =
        f.s.db
            .run(|c| {
                Ok(serde_json::from_str(&c.query_row(
                    "SELECT data FROM zones WHERE id='zone-local'",
                    [],
                    |r| r.get::<_, String>(0),
                )?)?)
            })
            .await
            .unwrap();
    assert_eq!(serde_json::to_value(durable).unwrap(), z);
    assert_eq!(prepare(&f).await["plan_id"], p["plan_id"]);
    f.s.db
        .run(|c| {
            c.execute_batch("DROP TRIGGER reject_receipt")?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        f.call("POST", &format!("{ZONE}/handoff"), Some(body))
            .await
            .1["applied"],
        true
    );
}

#[tokio::test]
async fn sequence_calls_are_bound_to_registered_endpoint_session_and_capability() {
    let f = fixture(&["local-shared", "local-unknown"]).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let (_, login) = call(
        &f.app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(json!({"username":"admin","password":"correct-test-password"})),
    )
    .await;
    let other = login["token"].as_str().unwrap();
    let prepare_body = json!({"endpoint_id":z["endpoint_id"],"track_id":z["track"]["id"],"playback_id":z["playback_id"]});
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("{ZONE}/prepare"),
            Some(other),
            Some(prepare_body.clone())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &f.app,
            "POST",
            &format!("{ZONE}/handoff"),
            Some(other),
            Some(handoff(&z, &p, 1, 30., 0., 0.5))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    f.s.endpoints
        .write()
        .await
        .get_mut("browser-continuous")
        .unwrap()["capabilities"]["continuous_pcm"] = json!(false);
    assert_eq!(
        f.call("POST", &format!("{ZONE}/prepare"), Some(prepare_body))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(zone(&f).await, z);
}

#[tokio::test]
async fn concurrent_duplicate_handoffs_are_serialized_and_repeat_instances_stay_distinct() {
    let f = fixture(&["local-shared"]).await;
    f.call("PATCH", ZONE, Some(json!({"repeat":"one"}))).await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    elapsed(&f, 40).await;
    let body = handoff(&z, &p, 1, 30., 0., 0.25);
    let path = format!("{ZONE}/handoff");
    let (first, second) = tokio::join!(
        f.call("POST", &path, Some(body.clone())),
        f.call("POST", &path, Some(body))
    );
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(second.0, StatusCode::OK);
    assert_ne!(first.1["applied"], second.1["applied"]);
    assert_eq!(zone(&f).await["playback_id"], p["items"][0]["playback_id"]);
    assert_eq!(
        totals(&f).await,
        json!({"tracks":[{"id":"local-shared","seconds":30.25,"plays":1,"skips":0}],"sessions":2,"receipts":1})
    );
}

#[tokio::test]
async fn shuffled_wrap_commits_reserved_order_without_reverting_refreshed_titles() {
    let f = fixture(&[
        "local-shared",
        "local-unknown",
        "local-shared",
        "local-unknown",
    ])
    .await;
    f.call("PATCH", ZONE, Some(json!({"repeat":"all","shuffle":true})))
        .await;
    f.call(
        "POST",
        &format!("{ZONE}/play"),
        Some(json!({"queue_index":3})),
    )
    .await;
    let z = zone(&f).await;
    let p = prepare(&f).await;
    let mut refreshed = f.s.zones.read().await["zone-local"].clone();
    for track in &mut refreshed.queue {
        track.title = "更新后的标题".into();
    }
    refreshed.track = refreshed.queue.get(refreshed.queue_index).cloned();
    f.s.save_zone(refreshed).await.unwrap();
    assert_eq!(prepare(&f).await["plan_id"], p["plan_id"]);
    elapsed(&f, 40).await;
    let (_, result) = f
        .call(
            "POST",
            &format!("{ZONE}/handoff"),
            Some(handoff(&z, &p, 1, 30., 0., 0.25)),
        )
        .await;
    assert_eq!(result["applied"], true, "{result}");
    assert_eq!(result["zone"]["queue_index"], 0);
    assert_eq!(result["zone"]["track"]["title"], "更新后的标题");
    for (index, track) in result["zone"]["queue"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        assert_eq!(track["id"], p["items"][index]["track"]["id"]);
        assert_eq!(track["title"], "更新后的标题");
    }
    let refilled = prepare(&f).await;
    for i in 0..7 {
        assert_eq!(
            refilled["items"][i]["playback_id"],
            p["items"][i + 1]["playback_id"]
        );
    }
}

#[tokio::test]
async fn real_bridge_registration_authorizes_http_preparation_and_atomic_handoff() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    let f = fixture(&["local-shared", "local-unknown"]).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = f.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut request = format!("ws://{address}/api/v1/bridge/events")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", f.token).parse().unwrap(),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.send(Message::Text(json!({"type":"register","endpoint":{"id":"continuous-alsa","kind":"alsa","capabilities":{"continuous_pcm":true,"playback_instance":true,"media_url_api_relative":true}}}).to_string().into())).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while let Some(message) = ws.next().await {
            match message.unwrap() {
                Message::Text(text)
                    if serde_json::from_str::<Value>(&text).unwrap()["type"] == "registered" =>
                {
                    return
                }
                Message::Ping(bytes) => ws.send(Message::Pong(bytes)).await.unwrap(),
                _ => {}
            }
        }
        panic!("socket closed before registration");
    })
    .await
    .unwrap();
    assert_eq!(
        f.call(
            "PATCH",
            ZONE,
            Some(json!({"endpoint_id":"continuous-alsa"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("POST", &format!("{ZONE}/play"), Some(json!({})))
            .await
            .0,
        StatusCode::OK
    );
    let z = zone(&f).await;
    let client = reqwest::Client::new();
    let response = client.post(format!("http://{address}{ZONE}/prepare")).bearer_auth(&f.token)
        .json(&json!({"endpoint_id":z["endpoint_id"],"track_id":z["track"]["id"],"playback_id":z["playback_id"]})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let p: Value = response.json().await.unwrap();
    elapsed(&f, 40).await;
    let response = client
        .post(format!("http://{address}{ZONE}/handoff"))
        .bearer_auth(&f.token)
        .json(&handoff(&z, &p, 1, 30., 0., 0.25))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["applied"], true);
    assert_eq!(result["zone"]["track"]["id"], "local-unknown");
    assert_eq!(
        f.s.endpoints.read().await["continuous-alsa"]["verified"],
        false
    );
    ws.send(Message::Text(json!({"type":"state","zone_id":"zone-local","track_id":"local-unknown",
        "core_playback_id":result["zone"]["playback_id"],"status":"ended","position":1.,"frames":48000,"sample_rate":48000}).to_string().into())).await.unwrap();
    ws.send(Message::Ping(vec![19,20,21].into())).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        while let Some(message)=ws.next().await {
            match message.unwrap() {
                Message::Pong(bytes) if bytes[..]==[19,20,21]=>return,
                Message::Ping(bytes)=>ws.send(Message::Pong(bytes)).await.unwrap(),_=>{}
            }
        }
        panic!("bridge closed before final EOF barrier");
    }).await.unwrap();
    assert_eq!(zone(&f).await["status"],"stopped");
    let end:f64=f.s.db.run(|c|Ok(c.query_row("SELECT last_position FROM play_sessions WHERE track_id='local-unknown' AND finish_reason='ended'",[],|r|r.get(0))?)).await.unwrap();
    assert_eq!(end,1.,"actual EOF must not be replaced by the 30-second catalog estimate");
    ws.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn library_redesign_tail_append_retains_cursor_and_duplicate_queue_entries(){
 let f=fixture(&["local-shared","local-unknown"]).await;let before=zone(&f).await;let plan=prepare(&f).await;let mut events=f.s.events.subscribe();
 let(status,after)=f.call("POST",&format!("{ZONE}/queue"),Some(json!({"track_ids":["local-shared","local-shared"]}))).await;
 assert_eq!(status,StatusCode::OK,"{after}");assert_eq!(after["playback_id"],before["playback_id"]);assert_eq!(after["queue"].as_array().unwrap().len(),4);assert_eq!(prepare(&f).await["plan_id"],plan["plan_id"]);
 while let Ok(event)=events.try_recv(){assert_ne!(event["type"],"sequence_invalidated");}
 elapsed(&f,60).await;let body=handoff(&before,&plan,1,30.,0.,0.5);let(status,result)=f.call("POST",&format!("{ZONE}/handoff"),Some(body)).await;assert_eq!(status,StatusCode::OK,"{result}");
 assert_eq!(zone(&f).await["queue"].as_array().unwrap().len(),4);
}

async fn refresh_capable(f:&Fixture) {
    let mut endpoints=f.s.endpoints.write().await;let endpoint=endpoints.get_mut("browser-continuous").unwrap();
    endpoint["capabilities"]["continuous_queue_refresh"]=json!(true);
    endpoint["capabilities"]["continuous_recovery"]=json!(true);
}
async fn refresh_preparation(f:&Fixture,plan:&Value,committed:Vec<Value>)->Value {
    let z=zone(f).await;
    let(status,response)=f.call("POST",&format!("{ZONE}/prepare"),Some(json!({"endpoint_id":z["endpoint_id"],"track_id":z["track"]["id"],"playback_id":z["playback_id"],"plan_id":plan["plan_id"],"committed_playback_ids":committed}))).await;
    assert_eq!(status,StatusCode::OK,"{response}");assert_eq!(response["accepted"],true);response
}
#[tokio::test]
async fn queue_refresh_keeps_the_playback_instance_and_durable_plan_while_replacing_the_tail() {
    let f=fixture(&["local-shared","local-unknown","local-shared"]).await;refresh_capable(&f).await;
    let z=zone(&f).await;let p=prepare(&f).await;let stats=totals(&f).await;let mut events=f.s.events.subscribe();
    assert_eq!(f.call("PATCH",ZONE,Some(json!({"repeat":"one"}))).await.0,StatusCode::OK);
    let mut refreshed=false;
    while let Ok(event)=events.try_recv() {if event["type"]=="sequence_invalidated" {assert_eq!(event["data"]["plan_id"],p["plan_id"]);assert_eq!(event["data"]["preserve_current"],true);refreshed=true;}}
    assert!(refreshed);assert_eq!(zone(&f).await["playback_id"],z["playback_id"]);assert_eq!(totals(&f).await,stats);
    let next=refresh_preparation(&f,&p,vec![]).await;assert_eq!(next["plan_id"],p["plan_id"]);assert_eq!(next["items"][0]["track"]["id"],"local-shared");assert_ne!(next["items"][0]["playback_id"],p["items"][0]["playback_id"]);
    let lease:Value=f.s.db.run(|c|{let data:String=c.query_row("SELECT data FROM sequence_recovery WHERE zone_id='zone-local'",[],|r|r.get(0))?;Ok(serde_json::from_str(&data)?)}).await.unwrap();
    assert_eq!(lease["plan"]["id"],p["plan_id"]);
    // Replacing an identical queue must not revoke the active output either.
    assert_eq!(f.call("PUT",&format!("{ZONE}/queue"),Some(json!({"track_ids":["local-shared","local-unknown","local-shared"]}))).await.0,StatusCode::OK);
    assert_eq!(refresh_preparation(&f,&next,vec![]).await["plan_id"],p["plan_id"]);
    let mut metadata=f.s.zones.read().await["zone-local"].clone();
    metadata.queue[0].integrated_lufs=Some(-18.);metadata.track=metadata.queue.first().cloned();
    f.s.save_zone(metadata).await.unwrap();
    assert_eq!(refresh_preparation(&f,&next,vec![]).await["plan_id"],p["plan_id"],"measured loudness updates must use the live smoothing control without reopening audio");
    // Processing changes keep their explicit output replacement semantics.
    assert_eq!(f.call("PATCH",ZONE,Some(json!({"transitions":{"fade_in_ms":200,"fade_out_ms":300}}))).await.0,StatusCode::OK);
    assert!(f.s.sequences.lock().await.get("zone-local").is_none());
}
#[tokio::test]
async fn queue_refresh_acknowledges_admitted_audio_without_restoring_the_old_queue() {
    let f=fixture(&["local-shared","local-unknown","local-shared"]).await;refresh_capable(&f).await;
    let z=zone(&f).await;let p=prepare(&f).await;let admitted=p["items"][0]["playback_id"].clone();
    assert_eq!(f.call("PUT",&format!("{ZONE}/queue"),Some(json!({"track_ids":["local-shared","local-shared"]}))).await.0,StatusCode::OK);
    let next=refresh_preparation(&f,&p,vec![admitted.clone()]).await;
    assert_eq!(next["items"][0]["playback_id"],admitted);assert_eq!(next["items"][1]["track"]["id"],"local-shared");
    elapsed(&f,60).await;let(status,receipt)=f.call("POST",&format!("{ZONE}/handoff"),Some(handoff(&z,&next,1,10.,0.,0.5))).await;
    assert_eq!(status,StatusCode::OK,"{receipt}");assert_eq!(receipt["accepted"],true);
    assert_eq!(receipt["zone"]["queue"].as_array().unwrap().len(),2);assert_eq!(receipt["zone"]["track"]["id"],"local-unknown");assert_eq!(receipt["zone"]["queue_index"],0);
    let remaining=prepare(&f).await;assert_eq!(remaining["items"][0]["track"]["id"],"local-shared");
}
