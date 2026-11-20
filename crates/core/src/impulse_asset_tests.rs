use super::*;
use sha2::{Digest,Sha256};
use axum::http::HeaderMap;

pub(super) fn wave(values:&[f64],channels:u16)->Vec<u8> {
    let mut bytes=b"RIFF".to_vec();bytes.extend((36+values.len() as u32*8).to_le_bytes());bytes.extend(b"WAVEfmt ");bytes.extend(16_u32.to_le_bytes());bytes.extend(3_u16.to_le_bytes());bytes.extend(channels.to_le_bytes());bytes.extend(48000_u32.to_le_bytes());bytes.extend((48000*channels as u32*8).to_le_bytes());bytes.extend((channels*8).to_le_bytes());bytes.extend(64_u16.to_le_bytes());bytes.extend(b"data");bytes.extend((values.len() as u32*8).to_le_bytes());for value in values{bytes.extend(value.to_le_bytes());}bytes
}
pub(super) async fn raw(app:&Router,method:&str,path:&str,token:Option<&str>,bytes:&[u8])->(StatusCode,HeaderMap,Vec<u8>) {
    let mut request=Request::builder().method(method).uri(path).header("Content-Type","audio/wav");
    if let Some(token)=token {request=request.header("Authorization",format!("Bearer {token}"));}
    let response=app.clone().oneshot(request.body(Body::from(bytes.to_vec())).unwrap()).await.unwrap();
    let status=response.status();let headers=response.headers().clone();let bytes=to_bytes(response.into_body(),5*1024*1024).await.unwrap().to_vec();(status,headers,bytes)
}
pub(super) async fn upload(f:&Fixture,bytes:&[u8])->Value {
    let(status,_,body)=raw(&f.app,"POST","/api/v1/dsp/impulses?name=room.wav",Some(&f.token),bytes).await;assert_eq!(status,StatusCode::OK,"{}",String::from_utf8_lossy(&body));serde_json::from_slice(&body).unwrap()
}
#[tokio::test]
async fn impulse_import_keeps_original_bytes_gain_and_immutable_identity(){
    let f=Fixture::new().await;let bytes=wave(&[2.,-3.,-0.,0.25],2);let asset=upload(&f,&bytes).await;
    let id=asset["id"].as_str().unwrap();assert_eq!(id,format!("{:x}",Sha256::digest(&bytes)));assert_eq!(asset["sample_rate"],48000);assert_eq!(asset["channels"],2);assert_eq!(asset["frames"],2);
    let(status,headers,download)=raw(&f.app,"GET",&format!("/api/v1/dsp/impulses/{id}/file"),Some(&f.token),&[]).await;
    assert_eq!(status,StatusCode::OK);assert_eq!(download,bytes);assert_eq!(headers["content-type"],"audio/wav");assert_eq!(headers["etag"],format!("\"{id}\""));
    let parsed=rmusic_audio::impulse::ImpulseResponse::from_wav(&download).unwrap();assert_eq!(parsed.kernels()[0][0],2.);assert_eq!(parsed.kernels()[1][0],-3.);assert_eq!(parsed.kernels()[0][1].to_bits(),(-0_f64).to_bits());
    let(status,headers,body)=raw(&f.app,"HEAD",&format!("/api/v1/dsp/impulses/{id}/file"),Some(&f.token),&[]).await;assert_eq!(status,StatusCode::OK);assert!(body.is_empty());assert_eq!(headers["content-length"],bytes.len().to_string());
    let(status,_,body)=raw(&f.app,"POST","/api/v1/dsp/impulses?name=different.wav",Some(&f.token),&bytes).await;assert_eq!(status,StatusCode::OK);assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(),asset);
    let(_,list)=f.call("GET","/api/v1/dsp/impulses",None).await;assert_eq!(list["items"].as_array().unwrap().len(),1);assert_eq!(list["limits"]["max_upload_bytes"],4*1024*1024);
    assert_eq!(f.call("GET",&format!("/api/v1/dsp/impulses/{id}"),None).await.1,asset);
}
#[tokio::test]
async fn impulse_permissions_reject_unauthorized_writes_and_bad_uploads_without_state_changes(){
    let f=Fixture::new().await;let bytes=wave(&[1.],1);
    let(_,result)=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"read","scopes":["read"]}))).await;let read=result["token"].as_str().unwrap();
    for path in ["/api/v1/dsp/impulses","/api/v1/dsp/impulses/0000000000000000000000000000000000000000000000000000000000000000/file"] {assert_eq!(raw(&f.app,"GET",path,None,&[]).await.0,StatusCode::UNAUTHORIZED);}
    assert_eq!(raw(&f.app,"POST","/api/v1/dsp/impulses",None,&bytes).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(raw(&f.app,"POST","/api/v1/dsp/impulses",Some(read),&bytes).await.0,StatusCode::FORBIDDEN);
    for (path,bytes) in [("/api/v1/dsp/impulses",b"invalid".to_vec()),("/api/v1/dsp/impulses?name=%0A",bytes.clone())] {assert_eq!(raw(&f.app,"POST",path,Some(&f.token),&bytes).await.0,StatusCode::BAD_REQUEST);}
    assert_eq!(raw(&f.app,"POST","/api/v1/dsp/impulses",Some(&f.token),&vec![0;rmusic_audio::impulse::MAX_WAV_BYTES+1]).await.0,StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(f.call("GET","/api/v1/dsp/impulses/NOT-A-HASH",None).await.0,StatusCode::BAD_REQUEST);
    let(_,list)=f.call("GET","/api/v1/dsp/impulses",None).await;assert_eq!(list["items"],json!([]));
    let asset=upload(&f,&bytes).await;let id=asset["id"].as_str().unwrap();
    assert_eq!(raw(&f.app,"GET",&format!("/api/v1/dsp/impulses/{id}/file"),Some(read),&[]).await.2,bytes);
    assert_eq!(raw(&f.app,"DELETE",&format!("/api/v1/dsp/impulses/{id}"),Some(read),&[]).await.0,StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn maximum_f64_impulse_is_not_limited_to_the_default_two_mib_json_body(){
    let f=Fixture::new().await;let bytes=wave(&vec![0.001;rmusic_audio::convolution::MAX_COEFFICIENTS],1);assert!(bytes.len()>2*1024*1024);
    let asset=upload(&f,&bytes).await;assert_eq!(asset["frames"],rmusic_audio::convolution::MAX_COEFFICIENTS);assert_eq!(asset["bytes"],bytes.len());
}
#[tokio::test]
async fn unsupported_playback_and_referenced_deletion_are_rejected_and_corruption_is_detected(){
    let f=Fixture::new().await;let bytes=wave(&[2.],1);let asset=upload(&f,&bytes).await;let id=asset["id"].as_str().unwrap().to_owned();
    let before=f.s.zones.read().await["zone-local"].dsp.clone();
    assert_eq!(f.call("PUT","/api/v1/zones/zone-local/dsp",Some(json!({"enabled":true,"convolution":{"filter_id":id}}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(f.s.zones.read().await["zone-local"].dsp,before);
    let mut zone=f.s.zones.read().await["zone-local"].clone();zone.dsp["convolution"]=json!({"filter_id":id});zone.dsp["enabled"]=json!(false);f.s.save_zone(zone.clone()).await.unwrap();
    let path=format!("/api/v1/dsp/impulses/{id}");assert_eq!(f.call("DELETE",&path,None).await.0,StatusCode::CONFLICT);
    zone.dsp=before;f.s.save_zone(zone).await.unwrap();
    let wave=f.s.config.data.join(format!("dsp/impulses/{id}.wav"));let mut damaged=bytes.clone();*damaged.last_mut().unwrap()^=1;std::fs::write(&wave,damaged).unwrap();
    assert_eq!(raw(&f.app,"GET",&format!("{path}/file"),Some(&f.token),&[]).await.0,StatusCode::INTERNAL_SERVER_ERROR);
    std::fs::write(wave,bytes).unwrap();let mut bad=asset;bad["frames"]=json!(2);std::fs::write(f.s.config.data.join(format!("dsp/impulses/{id}.json")),bad.to_string()).unwrap();
    assert_eq!(raw(&f.app,"GET",&format!("{path}/file"),Some(&f.token),&[]).await.0,StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(f.call("DELETE",&path,None).await.0,StatusCode::NO_CONTENT);assert_eq!(f.call("GET",&path,None).await.0,StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn concurrent_duplicates_and_full_asset_count_are_idempotent_and_deletion_releases_capacity(){
    let f=Fixture::new().await;let bytes=wave(&[1.],1);
    let results=futures_util::future::join_all((0..8).map(|_|raw(&f.app,"POST","/api/v1/dsp/impulses",Some(&f.token),&bytes))).await;
    for(status,_,body) in results {assert_eq!(status,StatusCode::OK);assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["id"],format!("{:x}",Sha256::digest(&bytes)));}
    let mut last=Value::Null;for value in 1..128 {last=upload(&f,&wave(&[value as f64/128.],1)).await;}
    let(_,list)=f.call("GET","/api/v1/dsp/impulses",None).await;assert_eq!(list["items"].as_array().unwrap().len(),128);
    upload(&f,&bytes).await;
    assert_eq!(raw(&f.app,"POST","/api/v1/dsp/impulses",Some(&f.token),&wave(&[1.5],1)).await.0,StatusCode::CONFLICT);
    assert_eq!(f.call("DELETE",&format!("/api/v1/dsp/impulses/{}",last["id"].as_str().unwrap()),None).await.0,StatusCode::NO_CONTENT);upload(&f,&wave(&[1.5],1)).await;
}
#[tokio::test]
async fn orphan_file_storage_is_counted_and_can_be_explicitly_removed(){
    let f=Fixture::new().await;let root=f.s.config.data.join("dsp/impulses");std::fs::create_dir_all(&root).unwrap();let id="0".repeat(64);
    std::fs::File::create(root.join(format!("{id}.wav"))).unwrap().set_len(256*1024*1024).unwrap();
    assert_eq!(raw(&f.app,"POST","/api/v1/dsp/impulses",Some(&f.token),&wave(&[1.],1)).await.0,StatusCode::CONFLICT);
    assert_eq!(f.call("DELETE",&format!("/api/v1/dsp/impulses/{id}"),None).await.0,StatusCode::NO_CONTENT);upload(&f,&wave(&[1.],1)).await;
}
#[test]
fn cancelled_impulse_requests_keep_file_and_zone_locks_until_background_work_finishes(){
    tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap().block_on(async {
        for deleting in [false,true] {
            let f=Fixture::new().await;let asset=upload(&f,&wave(&[2.],1)).await;let id=asset["id"].as_str().unwrap().to_owned();
            let auth=crate::auth::Auth{id:"test".into(),username:"test".into(),role:"admin".into(),totp_enabled:false,interactive:false,scopes:vec!["read".into(),"control".into()],session_hash:"test".into()};
            let(started,ready)=tokio::sync::oneshot::channel();let(release,blocked)=std::sync::mpsc::channel();
            let blocker=tokio::task::spawn_blocking(move||{let _=started.send(());let _=blocked.recv();});ready.await.unwrap();
            let state=f.s.clone();
            let request=tokio::spawn(async move {
                if deleting {crate::impulse_assets::delete(axum::extract::State(state),auth,axum::extract::Path(id)).await.map(|_|())}
                else {auth.require("read")?;crate::impulse_assets::read_file(state,id).await.map(|_|())}
            });
            tokio::time::timeout(std::time::Duration::from_secs(2),async {loop{if f.s.impulse_lock.try_lock().is_err(){break;}tokio::task::yield_now().await;}}).await.unwrap();
            request.abort();assert!(matches!(request.await,Err(error) if error.is_cancelled()));
            let file_locked=f.s.impulse_lock.try_lock().is_err();let zone_locked=!deleting||f.s.zone_lock.try_lock().is_err();
            release.send(()).unwrap();blocker.await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(2),async {let _file=f.s.impulse_lock.lock().await;let _zones=f.s.zone_lock.lock().await;}).await.unwrap();
            assert!(file_locked&&zone_locked,"request cancellation released locks while its filesystem job was still queued");
        }
    });
}
