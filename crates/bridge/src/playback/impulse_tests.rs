use super::*;
use super::tests::{play,profile,running,server,wav};
use axum::{extract::{Path,Query},http::{HeaderMap,StatusCode},routing::{get,post},Json,Router};
use sha2::{Digest,Sha256};
use std::{collections::HashMap,sync::atomic::AtomicUsize};
use tokio::io::{AsyncReadExt,AsyncWriteExt};
#[cfg(target_os="linux")]
mod alsa;

fn impulse(rate:u32,gain:f64)->Vec<u8> {
    impulse_samples(rate,&[gain,-3.,-0.])
}
fn impulse_samples(rate:u32,kernel:&[f64])->Vec<u8> {
    let size=kernel.len() as u32*8;
    let mut bytes=Vec::new();bytes.extend(b"RIFF");bytes.extend((36+size).to_le_bytes());bytes.extend(b"WAVEfmt ");bytes.extend(16u32.to_le_bytes());bytes.extend(3u16.to_le_bytes());bytes.extend(1u16.to_le_bytes());bytes.extend(rate.to_le_bytes());bytes.extend((rate*8).to_le_bytes());bytes.extend(8u16.to_le_bytes());bytes.extend(64u16.to_le_bytes());bytes.extend(b"data");bytes.extend(size.to_le_bytes());
    for sample in kernel {bytes.extend(sample.to_le_bytes());}bytes
}
fn id(bytes:&[u8])->String {format!("{:x}",Sha256::digest(bytes))}
fn dsp(bytes:&[u8],enabled:bool)->DspConfig {
    DspConfig {enabled,convolution:Some(ConvolutionConfig {response_ids:vec![id(bytes)]}),..Default::default()}
}
fn manifest(bytes:&[u8])->Value {
    let selected=id(bytes);json!({"items":[{"asset":{"id":selected,"bytes":bytes.len(),"sample_rate":u32::from_le_bytes(bytes[24..28].try_into().unwrap()),"channels":1,"frames":(bytes.len()-44)/8,"channel_mask":0},
        "api_path":format!("api/v1/dsp/impulses/{selected}/file?ticket=selected-only&zone_id=z&selection=snapshot")}],"expires_at":9999999999_u64})
}
async fn ready(playback:&mut Playback) {
    tokio::time::timeout(Duration::from_secs(5),async {
        while playback.pending.as_ref().is_some_and(|p|p.audio.is_none()||!p.impulse_ready||p.queue.is_none())||playback.retiring.is_some() {
            assert!(playback.update().await.is_none());
        }
    }).await.unwrap();
}
async fn pending(profile:&ConnectionProfile,config:DspConfig)->Playback {
    let audio=running(profile).await.audio.clone();
    let plan=Plan {zone_id:"z".into(),track_id:"current".into(),core_playback_id:Some("core-current".into()),position:0.,dsp:config,impulse:None,measured_lufs:None,transitions:Default::default(),control:Arc::new(PlaybackControl::default())};
    let mut playback=Playback::connected(profile.clone(),"control-only".into(),"endpoint".into()).unwrap();
    playback.pending=Some(Pending::new(plan,None,Some(audio)));playback.pending.as_mut().unwrap().queue=Some(None);playback
}

#[tokio::test]
async fn actual_play_waits_for_selected_responses_and_keeps_latest_controls_without_reloading_same_group() {
    let bytes=impulse(48000,2.);let selected=id(&bytes);let response=manifest(&bytes);let expected=selected.clone();
    let entered=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());let entered_file=entered.clone();let release_file=release.clone();
    let calls=Arc::new(AtomicUsize::new(0));let posted=calls.clone();let file=bytes.clone();
    let router=Router::new().route("/api/v1/media/source",get(||async{wav()})).route("/api/v1/zones/z/dsp/impulses",post(move|headers:HeaderMap,Json(body):Json<Value>|{let response=response.clone();let expected=expected.clone();let posted=posted.clone();async move{
        assert_eq!(headers["authorization"],"Bearer control-only");assert_eq!(body,json!({"response_ids":[expected]}));posted.fetch_add(1,Ordering::Relaxed);Json(response)
    }})).route("/api/v1/dsp/impulses/{id}/file",get(move|Path(requested):Path<String>,Query(query):Query<HashMap<String,String>>,headers:HeaderMap|{let file=file.clone();let selected=selected.clone();let entered=entered_file.clone();let release=release_file.clone();async move{
        assert_eq!(headers["authorization"],"Bearer control-only");assert_eq!(requested,selected);assert_eq!(query["ticket"],"selected-only");assert_eq!(query["zone_id"],"z");entered.notify_one();release.notified().await;file
    }}));
    let(mut profile,server)=server(Router::new().nest("/external/music",router)).await;profile.address.push_str("/external/music");
    let mut playback=Playback::connected(profile.clone(),"control-only".into(),"endpoint".into()).unwrap();let mut command=play(&profile,"current","source");command["dsp"]=serde_json::to_value(dsp(&bytes,true)).unwrap();
    playback.command("play",&command,&profile).unwrap();
    tokio::time::timeout(Duration::from_secs(5),async {loop {tokio::select!{_=entered.notified()=>break,report=playback.update()=>assert!(report.is_none())}}}).await.unwrap();
    assert!(playback.take_ready().is_none());
    playback.command("pause",&json!({}),&profile).unwrap();playback.command("volume",&json!({"volume":0.25,"muted":false}),&profile).unwrap();playback.command("seek",&json!({"position":0.25,"playback_id":"core-seek"}),&profile).unwrap();
    let mut updated=dsp(&bytes,true);updated.preamp_db=-6.;playback.command("dsp",&json!({"dsp":updated}),&profile).unwrap();assert!(playback.take_ready().is_none());release.notify_one();ready(&mut playback).await;
    let pending=playback.take_ready().unwrap();let request=pending.plan.request(pending.audio.as_ref().unwrap(),output::DsdOutput::Disabled).unwrap();
    assert_eq!(request.impulse.unwrap().response(48000).unwrap().kernels()[0],[2.,-3.,-0.]);assert_eq!(request.position,0.25);assert_eq!(request.dsp.preamp_db,-6.);assert_eq!(request.volume,0.25);assert!(pending.plan.control.paused.load(Ordering::Relaxed));assert!(!pending.plan.control.muted.load(Ordering::Relaxed));assert_eq!(pending.plan.core_playback_id.as_deref(),Some("core-seek"));assert_eq!(calls.load(Ordering::Relaxed),1);
    playback.shutdown().await;server.abort();
}

#[tokio::test]
async fn disabled_selected_group_never_downloads_or_attaches_responses() {
    let calls=Arc::new(AtomicUsize::new(0));let posted=calls.clone();let bytes=impulse(48000,2.);
    let(profile,server)=server(Router::new().route("/api/v1/media/source",get(||async{wav()})).route("/api/v1/zones/z/dsp/impulses",post(move||{let posted=posted.clone();async move{posted.fetch_add(1,Ordering::Relaxed);StatusCode::FORBIDDEN}}))).await;
    let mut playback=Playback::connected(profile.clone(),"control-only".into(),"endpoint".into()).unwrap();let mut command=play(&profile,"current","source");command["dsp"]=serde_json::to_value(dsp(&bytes,false)).unwrap();playback.command("play",&command,&profile).unwrap();ready(&mut playback).await;
    let pending=playback.take_ready().unwrap();let request=pending.plan.request(pending.audio.as_ref().unwrap(),output::DsdOutput::Disabled).unwrap();assert!(request.impulse.is_none());assert_eq!(request.dsp.convolution,Some(ConvolutionConfig{response_ids:vec![id(&bytes)]}));assert_eq!(calls.load(Ordering::Relaxed),0);playback.shutdown().await;server.abort();
}

#[tokio::test]
async fn selected_load_failures_report_current_source_and_cancel_all_pending_work() {
    for conflict in [false,true] {
        let bytes=impulse(48000,2.);let response=manifest(&bytes);let mut altered=bytes.clone();altered[44..52].copy_from_slice(&4_f64.to_le_bytes());
        let(profile,server)=server(Router::new().route("/api/v1/zones/z/dsp/impulses",post(move||{let response=response.clone();async move{if conflict{Err(StatusCode::CONFLICT)}else{Ok(Json(response))}}})).route("/api/v1/dsp/impulses/{id}/file",get(move||{let altered=altered.clone();async move{altered}}))).await;
        let mut playback=pending(&profile,dsp(&bytes,true)).await;let control=playback.control().unwrap().clone();let report=tokio::time::timeout(Duration::from_secs(5),playback.update()).await.unwrap().unwrap();
        assert_eq!(report["zone_id"],"z");assert_eq!(report["track_id"],"current");assert_eq!(report["core_playback_id"],"core-current");assert!(report["error"].as_str().unwrap().contains(if conflict{"HTTP 409"}else{"校验失败"}));assert!(playback.pending.is_none());assert!(playback.running.is_none());assert!(control.stopped.load(Ordering::Acquire));playback.shutdown().await;server.abort();
    }
}

async fn read_request(socket:&mut tokio::net::TcpStream)->String {
    let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}let text=String::from_utf8(headers).unwrap();
    let length=text.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(str::trim).map(str::to_owned)).map(|length|length.parse::<usize>().unwrap()).unwrap_or(0);let mut body=vec![0;length];socket.read_exact(&mut body).await.unwrap();text
}
async fn accept_loading(playback:&mut Playback,listener:&tokio::net::TcpListener)->tokio::net::TcpStream {
    tokio::time::timeout(Duration::from_secs(5),async {tokio::select! {socket=listener.accept()=>socket.unwrap().0,report=playback.update()=>panic!("premature loading completion: {report:?}")}}).await.unwrap()
}
#[tokio::test]
async fn control_replacement_or_drop_closes_real_manifest_and_file_header_or_body_waits() {
    for phase in ["manifest-headers","manifest-body","file-headers","file-body"] {
        for action in ["stop","flush","play","disable","replace","drop"] {
            let bytes=impulse(48000,2.);let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(format!("http://{}",listener.local_addr().unwrap()));let mut playback=pending(&profile,dsp(&bytes,true)).await;
            let mut socket=accept_loading(&mut playback,&listener).await;let headers=read_request(&mut socket).await;assert!(headers.starts_with("POST /api/v1/zones/z/dsp/impulses "));assert!(headers.to_ascii_lowercase().contains("authorization: bearer control-only"));
            if phase.starts_with("file") {
                let body=serde_json::to_vec(&manifest(&bytes)).unwrap();socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();socket.write_all(&body).await.unwrap();drop(socket);socket=accept_loading(&mut playback,&listener).await;assert!(read_request(&mut socket).await.starts_with(&format!("GET /api/v1/dsp/impulses/{}/file?ticket=selected-only",id(&bytes))));
            }
            if phase.ends_with("body") {socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\n\r\nx").await.unwrap();}
            let old_task=playback.pending.as_ref().unwrap().impulse_task.as_ref().unwrap().abort_handle();
            assert!(playback.command("dsp",&json!({"dsp":{"enabled":true,"convolution":{"response_ids":["invalid"]}}}),&profile).is_err());assert!(!old_task.is_finished());
            match action {
                "stop"|"flush"=>playback.command(action,&json!({}),&profile).unwrap(),
                "play"=>playback.command("play",&play(&profile,"new","new"),&profile).unwrap(),
                "disable"=>playback.command("dsp",&json!({"dsp":dsp(&bytes,false)}),&profile).unwrap(),
                "replace"=>playback.command("dsp",&json!({"dsp":dsp(&impulse(96000,5.),true)}),&profile).unwrap(),
                "drop"=>{drop(playback);playback=Playback::default();},
                _=>unreachable!(),
            }
            assert!(matches!(tokio::time::timeout(Duration::from_secs(2),socket.read_u8()).await.unwrap().unwrap_err().kind(),std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::ConnectionReset),"{phase}/{action}");assert!(old_task.is_finished());
            if let Some(pending)=&playback.pending{assert!(pending.plan.impulse.is_none());assert_eq!(pending.impulse_ready,action!="replace");}
            playback.shutdown().await;
        }
    }
}

#[tokio::test]
async fn seek_reuses_loaded_bank_but_changing_group_requires_a_new_load_before_admission() {
    let profile=profile("http://127.0.0.1:9".into());let bytes=impulse(48000,2.);let bank=Arc::new(ImpulseBank::new(vec![Arc::new(rmusic_audio::impulse::ImpulseResponse::from_wav(&bytes).unwrap())]).unwrap());
    let mut current=running(&profile).await;current.request.dsp=dsp(&bytes,true);current.request.impulse=Some(bank.clone());
    let mut playback=Playback::connected(profile.clone(),"control-only".into(),"endpoint".into()).unwrap();playback.running=Some(current);playback.command("seek",&json!({"position":0.5,"playback_id":"new-seek"}),&profile).unwrap();assert!(playback.pending.as_ref().unwrap().impulse_ready);assert!(Arc::ptr_eq(playback.pending.as_ref().unwrap().plan.impulse.as_ref().unwrap(),&bank));
    playback.command("dsp",&json!({"dsp":dsp(&impulse(96000,5.),true)}),&profile).unwrap();assert!(!playback.pending.as_ref().unwrap().impulse_ready);assert!(playback.pending.as_ref().unwrap().plan.impulse.is_none());assert!(playback.take_ready().is_none());playback.shutdown().await;
}

#[tokio::test]
async fn enabled_selection_without_authenticated_connection_reports_error_instead_of_basic_dsp_fallback() {
    let profile=profile("http://127.0.0.1:9".into());let mut playback=pending(&profile,dsp(&impulse(48000,2.),true)).await;playback.sequence_config=None;assert!(playback.take_ready().is_none());let report=playback.update().await.unwrap();assert!(report["error"].as_str().unwrap().contains("已认证的核心连接"));assert!(playback.pending.is_none());assert!(playback.running.is_none());
}
