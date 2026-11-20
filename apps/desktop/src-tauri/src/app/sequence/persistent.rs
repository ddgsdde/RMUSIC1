//! Restore an authenticated local cursor; playback admission still belongs to the native actor.
use super::*;
use rmusic_bridge::{listens::{cursor,sync::Sync},sequence::recovery::{restore as recover,BoundFailure}};

pub(in crate::app) struct Admission {
    pub generation:u64,
    pub measured_version:u32,
    pub control:Arc<PlaybackControl>,
}
struct Request<'a> {state:&'a DesktopState,id:String}
impl Drop for Request<'_> {
    fn drop(&mut self) {if let Ok(mut request)=self.state.recovery_request.lock(){if request.as_ref().is_some_and(|(id,_,_)|id==&self.id){request.take();}}}
}
async fn superseded(state:&DesktopState,generation:u64) {
    let mut interval=tokio::time::interval(Duration::from_millis(50));
    loop {interval.tick().await;if state.playback_generation.load(Ordering::Acquire)!=generation{return;}}
}
pub(in crate::app) async fn restore(request_id:String,profile_id:String,endpoint_id:String,output:Option<NativeOutput>,app:tauri::AppHandle,state:&DesktopState)->CommandResult<Value> {
    if state.group_output.owned(){return Ok(json!({"group_managed":true}));}
    let request_id=checked_playback_id(Some(request_id))?;
    if !endpoint_id.starts_with("browser-")||endpoint_id.len()>100||!endpoint_id.chars().all(|c|c.is_ascii_alphanumeric()||c=='-') {return Err("无效的本机端点".into());}
    let Ok(_restore)=state.recovery_gate.try_lock() else{return Ok(json!({"recovering":true}))};
    let previous_generation=state.playback_generation.load(Ordering::Acquire);
    *state.recovery_request.lock().map_err(err)?=Some((request_id.clone(),None,profile_id.clone()));
    let _request=Request {state,id:request_id.clone()};
    let profile=connected_profile(state,&profile_id).await?;let session=token(state,&profile_id)?;
    let core=Config::new(profile.clone(),session.clone(),endpoint_id.clone()).map_err(command_error)?;
    let Some(owner)=core.listener().await.map_err(command_error)? else{return Ok(Value::Null)};
    if state.listens.owner(&profile_id).map_err(err)?.as_ref()!=Some(&owner) {return Ok(Value::Null);}
    if state.player.lock().map_err(err)?.as_ref().is_some_and(|player|player.sequence.as_ref().is_some_and(|sequence|sequence.alive())||player.thread.as_ref().is_some_and(|thread|!thread.is_finished())) {return Ok(Value::Null);}
    let name=if cfg!(target_os="android"){"本机 Android 客户端"}else{"本机 Windows 客户端"};
    let registered=core_request(state,profile_id.clone(),"POST".into(),"api/v1/endpoints/browser".into(),Some(json!({"id":endpoint_id,"name":name,"platform":if cfg!(target_os="android"){"android"}else{"browser"},"output_config":output,"visualization_v1":true,"playback_instance":true,"continuous_pcm":true,"continuous_recovery":true,"recovery_controls":true,"measured_listens_v2":true,"measured_listens_recovery_v2":true}))).await?;
    if token(state,&profile_id).ok().as_deref()!=Some(session.as_str()) {return Ok(json!({"recovering":true}));}
    if registered["capabilities"]["continuous_recovery"]!=true {return Ok(Value::Null);}
    let (generation,control,revisions,handle)={
        let _gate=state.playback_gate.lock().await;
        if previous_generation!=state.playback_generation.load(Ordering::Acquire){return Ok(Value::Null);}
        if state.player.lock().map_err(err)?.as_ref().is_some_and(|player|
            player.sequence.as_ref().is_some_and(|sequence|sequence.alive()||sequence.cursor.is_none())||player.thread.as_ref().is_some_and(|thread|!thread.is_finished())) {return Ok(Value::Null);}
        let old=state.player.lock().map_err(err)?.take();if let Some(old)=old {old.stop(None).await;}
        let store=state.listens.clone();let saved_owner=owner.clone();let endpoint=endpoint_id.clone();
        let frozen=tokio::task::spawn_blocking(move ||{store.drain()?;cursor::freeze(store,&saved_owner,&endpoint)}).await.map_err(err)?.map_err(err)?;
        let Some((_,handle))=frozen else{return core.recovery_unavailable(&owner).await.map(|zone|zone.map(|zone|json!({"zone":zone})).unwrap_or(Value::Null)).map_err(command_error)};
        let control=reset_controls(state,0.,None,None,None,None)?;
        let generation=state.playback_generation.fetch_add(1,Ordering::AcqRel)+1;
        let revisions={let mut controls=state.playback_controls.lock().map_err(err)?;controls.profile_id=Some(profile_id.clone());controls.revisions};
        *state.recovery_cursor.lock().map_err(err)?=Some(handle.clone());
        *state.recovery_request.lock().map_err(err)?=Some((request_id.clone(),Some(generation),profile_id.clone()));
        (generation,control,revisions,handle)
    };
    let sync=Sync::new(state.listens.clone(),core.clone(),owner.clone());
    let result=tokio::select! {
        result=recover(core.clone(),state.listens.clone(),owner.clone(),sync)=>result,
        _=superseded(state,generation)=>return Ok(Value::Null),
        _=tokio::time::sleep(Duration::from_secs(20))=>return Err("音频恢复尚未完成，连接恢复后将继续尝试".into()),
    };
    let restored=match result {
        Ok(Some(restored))=>restored,Ok(None)=>return Ok(Value::Null),Err(error)=>{
            if let Some(bound)=error.downcast_ref::<BoundFailure>() {
                if bound.report["device_failed"]!=true {let _=bound.handle.retire();}
                let _=core.post(bound.report["zone_id"].as_str().unwrap_or(""),"progress",&json!({"playback_id":bound.report["core_playback_id"],"track_id":bound.report["track_id"],"recoverable":true,"device_failed":bound.report["device_failed"]==true,"system_suspend":bound.report["system_suspend"]})).await;
            }
            return Err(command_error(error));
        }
    };
    if generation!=state.playback_generation.load(Ordering::Acquire)
        ||token(state,&profile_id).ok().as_deref()!=Some(session.as_str())
        ||state.listens.owner(&profile_id).map_err(err)?.as_ref()!=Some(&owner)
        ||!handle.is_current().map_err(err)? {return Ok(Value::Null);}
    let mut zone=restored.command;
    let position=zone["position"].as_f64().ok_or("恢复音区缺少实际位置")?;
    let playback_id={
        let mut controls=state.playback_controls.lock().map_err(err)?;
        if generation!=state.playback_generation.load(Ordering::Acquire){return Ok(Value::Null);}
        if controls.revisions[0]==revisions[0] {control.set_paused(zone["status"]!="playing");}
        if controls.revisions[1]==revisions[1] {control.set_volume(zone["volume"].as_f64().unwrap_or(1.)).map_err(err)?;}
        if controls.revisions[2]==revisions[2] {control.muted.store(zone["muted"].as_bool().unwrap_or(false),Ordering::Relaxed);}
        controls.position=position;controls.playback_id.clone()
    };
    let context=Context {zone_id:zone["id"].as_str().ok_or("恢复音区缺少身份")?.into(),endpoint_id,playback_id:zone["playback_id"].as_str().ok_or("恢复音区缺少播放实例")?.into()};
    let track:NativeTrack=serde_json::from_value(zone["track"].clone()).map_err(err)?;
    let dsp=serde_json::from_value(zone["dsp"].clone()).map_err(err)?;
    let transitions=serde_json::from_value(zone["transitions"].clone()).map_err(err)?;
    let result=tokio::select! {
        result=native_play_inner(Some(playback_id),Some(context.clone()),profile_id,track,Some(position),Some(dsp),output,None,None,None,Some(transitions),app,state,Some(Admission {generation,control,measured_version:zone["measured_version"].as_u64().unwrap_or(1) as u32}),None)=>result,
        _=superseded(state,generation)=>return Ok(Value::Null),
    };
    match result {
        Ok(result)=>{
            if result["superseded"]==true{return Ok(Value::Null);}
            if result.get("zone").is_some(){return Ok(result);}
            if let Some(map)=zone.as_object_mut(){for field in ["type","url","zone_id"] {map.remove(field);}}
            Ok(json!({"zone":zone}))
        },
        Err(error)=>{
            if generation==state.playback_generation.load(Ordering::Acquire) {
                let transient=error.contains("error sending request")||error.contains("无法连接核心")||error.contains("MEDIA_SOURCE_UNAVAILABLE")||error.starts_with("HTTP 5");
                if !transient {let _=handle.retire();}
                let _=core.post(&context.zone_id,"progress",&json!({"track_id":zone["track"]["id"],"playback_id":context.playback_id,"recoverable":true})).await;
            }
            Err(error)
        }
    }
}
