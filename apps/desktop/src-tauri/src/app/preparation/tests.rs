use super::*;
use super::super::tests::state;
use rmusic_audio::impulse::ConvolutionConfig;
use sha2::{Digest,Sha256};
use tokio::io::{AsyncReadExt,AsyncWriteExt};

fn wav(rate:u32,gain:f64)->Vec<u8> {
    let mut bytes=b"RIFF".to_vec();bytes.extend(60_u32.to_le_bytes());bytes.extend(b"WAVEfmt ");bytes.extend(16_u32.to_le_bytes());bytes.extend(3_u16.to_le_bytes());bytes.extend(1_u16.to_le_bytes());bytes.extend(rate.to_le_bytes());bytes.extend((rate*8).to_le_bytes());bytes.extend(8_u16.to_le_bytes());bytes.extend(64_u16.to_le_bytes());bytes.extend(b"data");bytes.extend(24_u32.to_le_bytes());for value in [gain,-3.,-0.]{bytes.extend(value.to_le_bytes());}bytes
}
fn id(bytes:&[u8])->String {format!("{:x}",Sha256::digest(bytes))}
fn dsp(bytes:&[u8],enabled:bool)->DspConfig {DspConfig{enabled,convolution:Some(ConvolutionConfig{response_ids:vec![id(bytes)]}),..Default::default()}}
fn profile(address:std::net::SocketAddr)->ConnectionProfile {
    ConnectionProfile{core_public_key:None,id:"test".into(),name:"QA".into(),address:format!("http://{address}/external/music"),core_id:Some("core-a".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]}
}
fn manifest(bytes:&[u8])->Value {let selected=id(bytes);json!({"items":[{"asset":{"id":selected,"bytes":bytes.len(),"sample_rate":u32::from_le_bytes(bytes[24..28].try_into().unwrap()),"channels":1,"frames":3,"channel_mask":0},"api_path":format!("api/v1/dsp/impulses/{selected}/file?ticket=selected&zone_id=z&selection=group")}],"expires_at":9999999999_u64})}
async fn request(socket:&mut tokio::net::TcpStream)->(String,Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(2),async {
        let mut headers=Vec::new();while !headers.ends_with(b"\r\n\r\n"){headers.push(socket.read_u8().await.unwrap());}let headers=String::from_utf8(headers).unwrap();let size=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(str::trim).map(str::to_owned)).map(|size|size.parse::<usize>().unwrap()).unwrap_or(0);let mut body=vec![0;size];socket.read_exact(&mut body).await.unwrap();(headers,body)
    }).await.unwrap()
}
async fn respond(socket:&mut tokio::net::TcpStream,status:&str,body:&[u8]) {
    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();socket.write_all(body).await.unwrap();
}
async fn accept(listener:&tokio::net::TcpListener)->tokio::net::TcpStream {tokio::time::timeout(Duration::from_secs(2),listener.accept()).await.unwrap().unwrap().0}

#[tokio::test]
async fn native_preparation_loads_scoped_original_coefficients_and_keeps_latest_controls() {
    let root=tempfile::tempdir().unwrap();let state=state(root.path());let bytes=wav(96000,2.);let expected=bytes.clone();let selection=dsp(&bytes,true);let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(listener.local_addr().unwrap());
    state.tokens.lock().unwrap().insert("test".into(),"control-only".into());let(generation,control)=begin_playback_change(&state,0.,Some(0.8),Some(false),Some(false),Some("play".into())).await.unwrap();
    let pending=state.clone();let task=tokio::spawn(async move{run(&pending,generation,"test","control-only",load_impulse(&profile,"control-only",Some("z"),&selection)).await});
    let mut socket=accept(&listener).await;let(headers,body)=request(&mut socket).await;assert!(headers.starts_with("POST /external/music/api/v1/zones/z/dsp/impulses "));assert!(headers.to_ascii_lowercase().contains("authorization: bearer control-only"));assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(),json!({"response_ids":[id(&expected)]}));respond(&mut socket,"200 OK",&serde_json::to_vec(&manifest(&expected)).unwrap()).await;drop(socket);
    let mut socket=accept(&listener).await;let(headers,_)=request(&mut socket).await;assert!(headers.starts_with(&format!("GET /external/music/api/v1/dsp/impulses/{}/file?ticket=selected",id(&expected))));assert!(headers.to_ascii_lowercase().contains("authorization: bearer control-only"));
    state.pause(true).unwrap();state.volume(Some(0.25),Some(true)).unwrap();state.prepare_seek(0.75,Some("seek".into())).unwrap();respond(&mut socket,"200 OK",&expected).await;
    let bank=task.await.unwrap().unwrap().unwrap().unwrap();assert_eq!(bank.response(96000).unwrap().kernels()[0],[2.,-3.,-0.]);assert!(bank.response(48000).is_err());assert!(control.paused.load(Ordering::Acquire));assert!(control.muted.load(Ordering::Acquire));assert_eq!(control.volume_setting(),0.25);assert_eq!(state.playback_controls.lock().unwrap().position,0.75);assert_eq!(state.playback_generation.load(Ordering::Acquire),generation);assert_eq!(state.tokens.lock().unwrap()["test"],"control-only");
}

#[tokio::test]
async fn native_preparation_requires_zone_and_never_downloads_disabled_or_unconfigured_fir() {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(listener.local_addr().unwrap());let bytes=wav(48000,2.);
    assert!(load_impulse(&profile,"control-only",None,&dsp(&bytes,true)).await.unwrap_err().contains("在线音区"));
    for configuration in [DspConfig::default(),dsp(&bytes,false),DspConfig{enabled:true,..Default::default()}] {assert!(load_impulse(&profile,"control-only",Some("z"),&configuration).await.unwrap().is_none());}
    assert!(tokio::time::timeout(Duration::from_millis(100),listener.accept()).await.is_err());
}

#[tokio::test]
async fn native_preparation_cancels_real_header_and_body_waits_on_stop_replacement_session_change_or_ipc_abort() {
    for phase in ["manifest-headers","manifest-body","file-headers","file-body"] {
        for action in ["stop","replace","logout","new-session","ipc","drop"] {
            let root=tempfile::tempdir().unwrap();let state=state(root.path());let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(listener.local_addr().unwrap());let bytes=wav(48000,2.);let selection=dsp(&bytes,true);state.tokens.lock().unwrap().insert("test".into(),"control-only".into());
            let(generation,_)=begin_playback_change(&state,0.,None,None,Some(false),Some("old".into())).await.unwrap();state.playback_controls.lock().unwrap().profile_id=Some("test".into());state.connections.lock().await.insert("test".into(),(profile.clone(),std::time::Instant::now()));
            let pending=state.clone();let task=tokio::spawn(async move{pending.requests.run(Some("old-load".into()),run(&pending,generation,"test","control-only",load_impulse(&profile,"control-only",Some("z"),&selection))).await});
            let mut socket=accept(&listener).await;let(headers,_)=request(&mut socket).await;assert!(headers.starts_with("POST /external/music/api/v1/zones/z/dsp/impulses "));
            if phase.starts_with("file") {respond(&mut socket,"200 OK",&serde_json::to_vec(&manifest(&bytes)).unwrap()).await;drop(socket);socket=accept(&listener).await;assert!(request(&mut socket).await.0.starts_with("GET /external/music/api/v1/dsp/impulses/"));}
            if phase.ends_with("body"){socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10000\r\n\r\nx").await.unwrap();}
            let mut logout=None;
            match action {
                "stop"|"replace"=>{begin_playback_change(&state,0.,None,None,Some(action=="stop"),Some(format!("new-{action}"))).await.unwrap();},
                "logout"=>{let pending=state.clone();logout=Some(tokio::spawn(async move{core_request(&pending,"test".into(),"POST".into(),"api/v1/auth/logout".into(),None).await}));assert!(logout.take().unwrap().await.unwrap().unwrap_err().contains("加密"));},
                "new-session"=>{state.tokens.lock().unwrap().insert("test".into(),"new-session".into());},
                "ipc"=>cancel_native_request(&state,"old-load".into()).await.unwrap(),
                "drop"=>task.abort(),
                _=>unreachable!(),
            }
            let result=tokio::time::timeout(Duration::from_secs(2),task).await.unwrap();match action {"drop"=>assert!(result.unwrap_err().is_cancelled()),"ipc"=>assert_eq!(result.unwrap().unwrap_err(),"请求已取消"),_=>assert!(result.unwrap().unwrap().is_none())}
            let closed=tokio::time::timeout(Duration::from_secs(2),socket.read_u8()).await.unwrap().unwrap_err().kind();assert!(matches!(closed,std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::ConnectionReset),"{phase}/{action}: {closed:?}");if let Some(logout)=logout{logout.await.unwrap().unwrap();}
            assert!(state.player.lock().unwrap().is_none());if action=="new-session"{assert_eq!(state.tokens.lock().unwrap()["test"],"new-session");}
        }
    }
}

#[tokio::test]
async fn superseded_or_changed_session_never_polls_work_and_late_errors_cannot_clear_new_session() {
    let root=tempfile::tempdir().unwrap();let state=state(root.path());state.tokens.lock().unwrap().insert("test".into(),"old-session".into());let(generation,_)=begin_playback_change(&state,0.,None,None,None,None).await.unwrap();
    for old_generation in [generation-1,generation+1] {assert!(run(&state,old_generation,"test","old-session",async{panic!("superseded work was polled");#[allow(unreachable_code)] Ok(())}).await.unwrap().is_none());}
    state.tokens.lock().unwrap().insert("test".into(),"new-session".into());assert!(run(&state,generation,"test","old-session",async{panic!("old session was polled");#[allow(unreachable_code)] Ok(())}).await.unwrap().is_none());
    let operation=async{state.tokens.lock().unwrap().insert("test".into(),"latest-session".into());Err::<(),_>("HTTP 401: old session".into())};assert!(run(&state,generation,"test","new-session",operation).await.unwrap().is_none());assert_eq!(state.tokens.lock().unwrap()["test"],"latest-session");
}

#[tokio::test]
async fn native_preparation_propagates_current_group_conflict_and_cancels_parallel_source_work() {
    let root=tempfile::tempdir().unwrap();let state=state(root.path());state.tokens.lock().unwrap().insert("test".into(),"control-only".into());let(generation,_)=begin_playback_change(&state,0.,None,None,None,None).await.unwrap();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let profile=profile(listener.local_addr().unwrap());let selection=dsp(&wav(48000,2.),true);let source_dropped=Arc::new(std::sync::atomic::AtomicBool::new(false));
    struct Guard(Arc<std::sync::atomic::AtomicBool>);impl Drop for Guard {fn drop(&mut self){self.0.store(true,Ordering::Release);}}
    let guard=Guard(source_dropped.clone());let pending=state.clone();let task=tokio::spawn(async move{run(&pending,generation,"test","control-only",async{let (_,bank)=tokio::try_join!(async{let _guard=guard;std::future::pending::<CommandResult<()>>().await},load_impulse(&profile,"control-only",Some("z"),&selection))?;Ok(bank)}).await});
    let mut socket=accept(&listener).await;request(&mut socket).await;respond(&mut socket,"409 Conflict",b"").await;assert!(task.await.unwrap().unwrap_err().contains("HTTP 409"));assert!(source_dropped.load(Ordering::Acquire));assert!(state.player.lock().unwrap().is_none());assert_eq!(state.tokens.lock().unwrap()["test"],"control-only");
}
