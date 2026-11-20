use super::*;
use super::impulse_assets::{raw,upload,wave};
use axum::extract::State;

fn wave_at(values:&[f64],channels:u16,rate:u32)->Vec<u8>{let mut bytes=wave(values,channels);bytes[24..28].copy_from_slice(&rate.to_le_bytes());bytes[28..32].copy_from_slice(&(rate*channels as u32*8).to_le_bytes());bytes}
fn config(ids:&[String],enabled:bool)->Value{json!({"enabled":enabled,"convolution":{"response_ids":ids}})}
fn asset_id(asset:&Value)->String{asset["id"].as_str().unwrap().into()}
async fn set(f:&Fixture,ids:&[String],enabled:bool)->(StatusCode,Value){f.call("PUT","/api/v1/zones/zone-local/dsp",Some(config(ids,enabled))).await}
fn file_path(manifest:&Value,index:usize)->String{format!("/{}",manifest["items"][index]["api_path"].as_str().unwrap())}

#[tokio::test]
async fn selection_budget_counts_original_files_including_ignored_wave_chunks(){
    let f=Fixture::new().await;let mut ids=Vec::new();
    for rate in 8000..8009{let mut bytes=wave_at(&[1.],1,rate);let padding=rmusic_audio::impulse::MAX_WAV_BYTES-bytes.len()-8;bytes.extend(b"JUNK");bytes.extend((padding as u32).to_le_bytes());bytes.resize(rmusic_audio::impulse::MAX_WAV_BYTES,0);bytes[4..8].copy_from_slice(&((rmusic_audio::impulse::MAX_WAV_BYTES-8) as u32).to_le_bytes());ids.push(asset_id(&upload(&f,&bytes).await));}
    assert_eq!(set(&f,&ids[..8],false).await.0,StatusCode::OK);assert_eq!(set(&f,&ids,false).await.0,StatusCode::BAD_REQUEST);assert_eq!(f.s.zones.read().await["zone-local"].dsp["convolution"]["response_ids"],json!(&ids[..8]));
}
#[tokio::test]
async fn damaged_selection_is_rejected_before_configuration_or_ticket_issuance(){
    let f=Fixture::new().await;let bytes=wave(&[2.],1);let asset=upload(&f,&bytes).await;let id=asset_id(&asset);let ids=vec![id.clone()];let before=f.s.zones.read().await["zone-local"].dsp.clone();let metadata=f.s.config.data.join(format!("dsp/impulses/{id}.json"));let wave=f.s.config.data.join(format!("dsp/impulses/{id}.wav"));
    let mut bad=asset.clone();bad["frames"]=json!(2);std::fs::write(&metadata,bad.to_string()).unwrap();assert_eq!(set(&f,&ids,false).await.0,StatusCode::INTERNAL_SERVER_ERROR);assert_eq!(f.s.zones.read().await["zone-local"].dsp,before);std::fs::write(metadata,asset.to_string()).unwrap();assert_eq!(set(&f,&ids,false).await.0,StatusCode::OK);
    let mut damaged=bytes.clone();*damaged.last_mut().unwrap()^=1;std::fs::write(&wave,damaged).unwrap();let tickets=f.s.tickets.lock().await.len();assert_eq!(f.call("POST","/api/v1/zones/zone-local/dsp/impulses",Some(json!({"response_ids":ids}))).await.0,StatusCode::INTERNAL_SERVER_ERROR);assert_eq!(f.s.tickets.lock().await.len(),tickets);std::fs::write(wave,bytes).unwrap();assert_eq!(f.call("POST","/api/v1/zones/zone-local/dsp/impulses",Some(json!({"response_ids":ids}))).await.0,StatusCode::OK);
}
#[tokio::test]
async fn inactive_selection_persists_protects_files_and_old_endpoints_cannot_enable_it(){
    let f=Fixture::new().await;let a=upload(&f,&wave(&[2.],1)).await;let b=upload(&f,&wave_at(&[-3.],1,96000)).await;let ids=vec![asset_id(&a),asset_id(&b)];
    assert_eq!(set(&f,&ids,false).await.0,StatusCode::OK);let before=f.s.zones.read().await["zone-local"].dsp.clone();
    assert_eq!(set(&f,&ids,true).await.0,StatusCode::BAD_REQUEST);assert_eq!(f.s.zones.read().await["zone-local"].dsp,before);
    assert_eq!(f.call("DELETE",&format!("/api/v1/dsp/impulses/{}",ids[0]),None).await.0,StatusCode::CONFLICT);
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();assert_eq!(restored.zones.read().await["zone-local"].dsp,before);
    assert_eq!(f.call("PUT","/api/v1/zones/zone-local/dsp",Some(json!({"enabled":false,"convolution":null}))).await.0,StatusCode::OK);assert_eq!(f.call("DELETE",&format!("/api/v1/dsp/impulses/{}",ids[0]),None).await.0,StatusCode::NO_CONTENT);
}
#[tokio::test]
async fn selection_rejects_bad_shape_missing_files_same_rates_and_budget_overflow_atomically(){
    let f=Fixture::new().await;let a=upload(&f,&wave(&[2.],1)).await;let b=upload(&f,&wave(&[-3.],1)).await;let id=asset_id(&a);let before=f.s.zones.read().await["zone-local"].dsp.clone();
    for value in [json!({"response_ids":[]}),json!({"response_ids":[id,id]}),json!({"response_ids":["../bad"]}),json!({"response_ids":id}),json!({"response_ids":[id],"normalize":true}),json!({"filter_id":id}),json!({"response_ids":vec![id.clone();17]})]{assert_eq!(f.call("PUT","/api/v1/zones/zone-local/dsp",Some(json!({"enabled":false,"convolution":value}))).await.0,StatusCode::BAD_REQUEST);}
    assert_eq!(f.call("PUT","/api/v1/zones/zone-local/dsp",Some(json!({"enabled":"true","convolution":{"response_ids":[id]}}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(set(&f,&["0".repeat(64)],false).await.0,StatusCode::NOT_FOUND);assert_eq!(set(&f,&[id,asset_id(&b)],false).await.0,StatusCode::BAD_REQUEST);assert_eq!(f.s.zones.read().await["zone-local"].dsp,before);
    let mut ids=Vec::new();for rate in [44100,48000,88200,96000,192000]{ids.push(asset_id(&upload(&f,&wave_at(&vec![0.;262144],1,rate)).await));}
    assert_eq!(set(&f,&ids[..4],false).await.0,StatusCode::OK);assert_eq!(set(&f,&ids,false).await.0,StatusCode::BAD_REQUEST);assert_eq!(f.s.zones.read().await["zone-local"].dsp["convolution"]["response_ids"],json!(&ids[..4]));
}
#[tokio::test]
async fn control_tickets_authorize_only_the_current_selected_files_and_support_head(){
    let f=Fixture::new().await;let bytes=wave(&[2.,-3.],1);let a=upload(&f,&bytes).await;let b=upload(&f,&wave_at(&[5.],1,96000)).await;let other=upload(&f,&wave_at(&[7.],1,192000)).await;let ids=vec![asset_id(&a),asset_id(&b)];assert_eq!(set(&f,&ids,false).await.0,StatusCode::OK);
    let control=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"control","scopes":["control"]}))).await.1["token"].as_str().unwrap().to_owned();let read=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"read","scopes":["read"]}))).await.1["token"].as_str().unwrap().to_owned();
    let body=json!({"response_ids":[ids[1],ids[0]]});let path="/api/v1/zones/zone-local/dsp/impulses";
    assert_eq!(call(&f.app,"POST",path,None,Some(body.clone())).await.0,StatusCode::UNAUTHORIZED);assert_eq!(call(&f.app,"POST",path,Some(&read),Some(body.clone())).await.0,StatusCode::FORBIDDEN);
    let(status,manifest)=call(&f.app,"POST",path,Some(&control),Some(body)).await;assert_eq!(status,StatusCode::OK);assert_eq!(manifest["items"].as_array().unwrap().len(),2);let resource=file_path(&manifest,0);
    assert_eq!(raw(&f.app,"GET",&format!("/api/v1/dsp/impulses/{}/file",ids[0]),Some(&control),&[]).await.0,StatusCode::FORBIDDEN);
    let(status,headers,actual)=raw(&f.app,"GET",&resource,None,&[]).await;assert_eq!(status,StatusCode::OK);assert_eq!(actual,bytes);assert_eq!(headers["cache-control"],"private, no-store");
    let(status,headers,actual)=raw(&f.app,"HEAD",&resource,None,&[]).await;assert_eq!(status,StatusCode::OK);assert!(actual.is_empty());assert_eq!(headers["content-length"],bytes.len().to_string());
    assert_eq!(raw(&f.app,"GET",&resource.replace(&ids[0],&asset_id(&other)),None,&[]).await.0,StatusCode::UNAUTHORIZED);
    let mut zone=f.s.zones.read().await["zone-local"].clone();zone.id="zone-other".into();f.s.save_zone(zone).await.unwrap();assert_eq!(raw(&f.app,"GET",&resource.replace("zone_id=zone-local","zone_id=zone-other"),None,&[]).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(call(&f.app,"POST",path,Some(&control),Some(json!({"response_ids":[asset_id(&other)]}))).await.0,StatusCode::CONFLICT);
    assert_eq!(set(&f,&[asset_id(&other),ids[1].clone()],false).await.0,StatusCode::OK);assert_eq!(raw(&f.app,"GET",&resource,None,&[]).await.0,StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn ticket_expiry_session_revocation_and_zone_deletion_remove_selected_file_authority(){
    let f=Fixture::new().await;let a=upload(&f,&wave(&[2.],1)).await;let ids=vec![asset_id(&a)];assert_eq!(set(&f,&ids,false).await.0,StatusCode::OK);let control=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"control","scopes":["control"]}))).await.1["token"].as_str().unwrap().to_owned();let body=json!({"response_ids":ids});let path="/api/v1/zones/zone-local/dsp/impulses";
    let(_,manifest)=call(&f.app,"POST",path,Some(&control),Some(body.clone())).await;let resource=file_path(&manifest,0);let expiry=manifest["expires_at"].as_i64().unwrap();assert!((299..=301).contains(&(expiry-chrono::Utc::now().timestamp())));
    for ticket in f.s.tickets.lock().await.values_mut(){if ticket.scope.starts_with("impulse:"){ticket.expires=chrono::Utc::now().timestamp()-1;}}
    assert_eq!(raw(&f.app,"GET",&resource,None,&[]).await.0,StatusCode::UNAUTHORIZED);
    let(_,manifest)=call(&f.app,"POST",path,Some(&control),Some(body.clone())).await;let resource=file_path(&manifest,0);assert_eq!(f.call("DELETE",&format!("/api/v1/auth/tokens/{}",auth::hash(&control)),None).await.0,StatusCode::NO_CONTENT);assert_eq!(raw(&f.app,"GET",&resource,None,&[]).await.0,StatusCode::UNAUTHORIZED);
    let(_,manifest)=f.call("POST",path,Some(body)).await;let resource=file_path(&manifest,0);let mut zone=f.s.zones.read().await["zone-local"].clone();zone.id="zone-other".into();f.s.save_zone(zone).await.unwrap();assert_eq!(f.call("DELETE","/api/v1/zones/zone-local",None).await.0,StatusCode::NO_CONTENT);assert_eq!(raw(&f.app,"GET",&resource,None,&[]).await.0,StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn negotiated_endpoint_capability_is_required_and_never_implied_for_old_clients(){
    let f=Fixture::new().await;let asset=upload(&f,&wave(&[2.],1)).await;let ids=vec![asset_id(&asset)];
    assert_eq!(f.call("GET","/api/v1/status",None).await.1["capabilities"]["convolution_config_v1"],true);
    let(_,old)=f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-old","name":"old","playback_instance":true,"continuous_pcm":true}))).await;assert_eq!(old["capabilities"]["convolution_v1"],false);assert_eq!(set(&f,&ids,true).await.0,StatusCode::BAD_REQUEST);
    let(_,incomplete)=f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-old","name":"old","convolution_v1":true}))).await;assert_eq!(incomplete["capabilities"]["convolution_v1"],false);
    let(_,ready)=f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":"browser-old","name":"old","playback_instance":true,"continuous_pcm":true,"convolution_v1":true}))).await;assert_eq!(ready["capabilities"]["convolution_v1"],true);assert_eq!(set(&f,&ids,true).await.0,StatusCode::OK);
}
#[tokio::test]
async fn configuration_commit_survives_http_cancellation_and_keeps_referenced_deletion_serialized(){
    let f=Fixture::new().await;let asset=upload(&f,&wave(&[2.],1)).await;let id=asset_id(&asset);let admin=auth::authenticate(&f.s,&f.token).await.unwrap();
    let(tx,mut messages)=tokio::sync::mpsc::channel(1);f.s.bridges.write().await.insert("browser-local".into(),tx);
    let blocker=rusqlite::Connection::open(f.s.config.data.join("db.sqlite")).unwrap();blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let app=f.app.clone();let token=f.token.clone();let wanted=crate::models::complete_dsp(config(&[id.clone()],false));let body=wanted.clone();let request=tokio::spawn(async move{call(&app,"PUT","/api/v1/zones/zone-local/dsp",Some(&token),Some(body)).await});
    tokio::time::timeout(std::time::Duration::from_secs(2),messages.recv()).await.unwrap().unwrap();request.abort();assert!(request.await.unwrap_err().is_cancelled());
    let state=f.s.clone();let mut deletion=tokio::spawn(async move{crate::impulse_assets::delete(State(state),admin,axum::extract::Path(id)).await});
    let blocked=tokio::time::timeout(std::time::Duration::from_millis(100),&mut deletion).await;blocker.execute_batch("ROLLBACK").unwrap();
    if let Ok(result)=blocked{panic!("accepted DSP commit lost its zone lock after HTTP cancellation: {:?}",result.unwrap());}
    let error=tokio::time::timeout(std::time::Duration::from_secs(3),deletion).await.unwrap().unwrap().unwrap_err();assert_eq!(error.0,StatusCode::CONFLICT);assert_eq!(f.s.zones.read().await["zone-local"].dsp,wanted);
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();assert_eq!(restored.zones.read().await["zone-local"].dsp,wanted);
}
