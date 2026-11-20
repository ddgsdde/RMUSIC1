use super::*;
use crate::group_output::{LocalPlayer,Registration};
use futures_util::future::BoxFuture;
struct Player {app:tauri::AppHandle,profile:String,session:String}
struct IndependentPlayer;
impl LocalPlayer for IndependentPlayer {
    fn prepare<'a>(&'a self,_command:&'a Value)->BoxFuture<'a,Result<()>> {Box::pin(async{Ok(())})}
}
impl LocalPlayer for Player {
    fn prepare<'a>(&'a self,_command:&'a Value)->BoxFuture<'a,Result<()>> {Box::pin(async move {
        let state=self.app.state::<DesktopState>();
        prepare(&state,&self.profile,&self.session,false).await.map_err(anyhow::Error::msg)
    })}
    fn stop(&self)->BoxFuture<'_,Result<()>> {Box::pin(async move{let state=self.app.state::<DesktopState>();prepare(&state,&self.profile,&self.session,true).await.map_err(anyhow::Error::msg)})}
}
async fn prepare(state:&DesktopState,profile:&str,session:&str,only_profile:bool)->CommandResult<()> {
    let _gate=state.playback_gate.lock().await;
    if token(state,profile).ok().as_deref()!=Some(session){return Err("本机账号已切换".into());}
    if only_profile&&state.playback_controls.lock().map_err(err)?.profile_id.as_deref()!=Some(profile){return Ok(());}
    if state.player.lock().map_err(err)?.as_ref().is_some_and(|player|player.proxy.is_none()&&player.cache_id.is_some()&&!player.control.stopped.load(Ordering::Acquire)) {
        return Err("本机仍在离线播放，请先停止后再加入分组".into());
    }
    state.playback_generation.fetch_add(1,Ordering::AcqRel);
    crate::cache_tail::cancel(&state.metadata_tasks);
    if let Some(cursor)=state.recovery_cursor.lock().map_err(err)?.take(){cursor.retire().map_err(err)?;}
    let old=state.player.lock().map_err(err)?.take();
    if let Some(old)=old {old.stop(Some("replaced")).await;}
    Ok(())
}
#[tauri::command]
pub(super) async fn native_endpoint_register(
    request_id:String,registration_id:String,profile_id:String,endpoint_id:String,output:Option<NativeOutput>,name:Option<String>,
    app:tauri::AppHandle,state:State<'_,DesktopState>,
)->CommandResult<Value> {
    if !cfg!(any(windows,target_os="android")){return Err("此平台尚未接入计划输出".into());}
    #[cfg(target_os="android")] {
        return state.travel.register(profile_id,endpoint_id,name.unwrap_or_else(||"此手机".into()),output.unwrap_or_default()).await;
    }
    state.requests.run(Some(request_id),async {
        let profile=connected_profile(&state,&profile_id).await?;let session=token(&state,&profile_id)?;
        let name=name.filter(|name|!name.trim().is_empty()&&name.len()<=100).unwrap_or_else(||std::env::var("COMPUTERNAME").unwrap_or_else(|_|"R-Music 客户端".into()));
        let primary:OutputConfig=output.unwrap_or_default().into();
        let mut outputs=vec![json!({"id":endpoint_id,"name":"系统默认输出","kind":primary.backend(),"native_device_id":primary.device_id,"initial_config":primary,"default":true,"configurable":true})];
        let devices=tokio::task::spawn_blocking(output::devices).await.map_err(err)?.map_err(err)?;
        for device in devices {use sha2::{Digest,Sha256};let suffix=format!("{:x}",Sha256::digest(device.id.as_bytes()));
            outputs.push(json!({"id":format!("{endpoint_id}-{}",&suffix[..12]),"name":device.name,"kind":if cfg!(target_os="android"){"android_audiotrack"}else if device.id.starts_with("asio:"){"asio"}else{"wasapi"},"native_device_id":device.id,"mix_rate":device.mix_rate,"channels":device.channels,"default":false,"configurable":true}));
        }
        let response=profile.http().map_err(err)?.post(profile.endpoint("api/v1/devices/register").map_err(err)?).bearer_auth(&session)
            .timeout(std::time::Duration::from_secs(8)).json(&json!({"id":endpoint_id,"name":name,"platform":std::env::consts::OS,"outputs":outputs})).send().await.map_err(err)?;
        // Older cores retain the single-output path.
        let inventory=if response.status()==reqwest::StatusCode::NOT_FOUND {json!({"name":name,"outputs":[{"id":endpoint_id,"config":primary,"default":true}]})}else{response.error_for_status().map_err(err)?.json::<Value>().await.map_err(err)?};
        let mut primary_endpoint=None;
        for item in inventory["outputs"].as_array().into_iter().flatten() {
            let Some(id)=item["id"].as_str() else{continue;};let is_primary=id==endpoint_id;
            let old=state.group_output.endpoint(&profile_id,id);
            if item["in_use"]==true&&old.is_some() {if is_primary {primary_endpoint=old;}continue;}
            if item["config"]["enabled"]==false {state.group_output.disable(id).await;continue;}
            let config=serde_json::from_value::<OutputConfig>(item["config"].clone()).map_err(err)?.normalized();
            let local:Arc<dyn LocalPlayer>=if is_primary {Arc::new(Player {app:app.clone(),profile:profile_id.clone(),session:session.clone()})}else{Arc::new(IndependentPlayer)};
            let display_name:String=format!("{} · {}",inventory["name"].as_str().unwrap_or(&name),item["name"].as_str().unwrap_or("输出")).chars().take(30).collect();
            let result=state.group_output.register(Registration {id:registration_id.clone(),profile:profile.clone(),token:session.clone(),endpoint:id.into(),name:display_name,output:config,listens:Some(state.listens.clone()),primary:is_primary},local).await;
            if is_primary {primary_endpoint=Some(result.map_err(err)?);}else if let Err(error)=result {eprintln!("独立输出 {id} 注册失败：{error}");}
        }
        Ok(primary_endpoint.unwrap_or_else(||json!({"id":endpoint_id,"name":name,"kind":primary.backend(),"online":false,"capabilities":{}})))
    }).await
}
#[tauri::command]
pub(super) async fn native_endpoint_disconnect(registration_id:String,state:State<'_,DesktopState>)->CommandResult<()> {
    state.group_output.stop(None,Some(&registration_id)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local_player(control:Arc<PlaybackControl>,thread:Option<std::thread::JoinHandle<()>>,offline:bool)->NativePlayer {
        NativePlayer {retired:false,track_id:"qa".into(),playback_id:"local".into(),control,
            request:PlaybackRequest {flac_samples: None, dsd_pcm: None, input:"never-opened".into(),position:0.,sample_rate:768000,channels:2,source_integer_lossless:true,source_bit_depth:32,dsp:Default::default(),impulse:None,measured_lufs:None,volume:1.,duration:Some(1.),transitions:Default::default()},
            output:Default::default(),proxy:None,local_file:None,thread,cache_id:offline.then(||"offline".into()),report:Arc::new(Mutex::new(json!({}))),listen:None,sequence:None}
    }
    #[tokio::test]
    async fn group_claim_rejects_old_account_before_stopping_local_output() {
        let root=tempfile::tempdir().unwrap();let state=super::super::tests::state(root.path());
        state.tokens.lock().unwrap().insert("test".into(),"new".into());let generation=state.playback_generation.load(Ordering::Acquire);
        assert!(prepare(&state,"test","old",false).await.unwrap_err().contains("账号"));
        assert_eq!(state.playback_generation.load(Ordering::Acquire),generation);
        prepare(&state,"test","new",false).await.unwrap();assert_eq!(state.playback_generation.load(Ordering::Acquire),generation+1);
    }
    #[tokio::test]
    async fn group_claim_stops_local_player_and_waits_for_its_thread() {
        let root=tempfile::tempdir().unwrap();let state=super::super::tests::state(root.path());state.tokens.lock().unwrap().insert("test".into(),"session".into());
        let control=Arc::new(PlaybackControl::default());let worker=control.clone();let (release,held)=std::sync::mpsc::channel();
        let thread=std::thread::spawn(move||{held.recv().unwrap();assert!(worker.stopped.load(Ordering::Acquire));});
        *state.player.lock().unwrap()=Some(local_player(control.clone(),Some(thread),false));
        let saved=state.clone();let claim=tokio::spawn(async move{prepare(&saved,"test","session",false).await});
        tokio::time::timeout(std::time::Duration::from_secs(2),async{while !control.stopped.load(Ordering::Acquire){tokio::task::yield_now().await;}}).await.unwrap();
        assert!(!claim.is_finished());release.send(()).unwrap();claim.await.unwrap().unwrap();assert!(state.player.lock().unwrap().is_none());
    }
    #[tokio::test]
    async fn group_claim_keeps_current_offline_output_untouched() {
        let root=tempfile::tempdir().unwrap();let state=super::super::tests::state(root.path());state.tokens.lock().unwrap().insert("test".into(),"session".into());
        let control=Arc::new(PlaybackControl::default());*state.player.lock().unwrap()=Some(local_player(control.clone(),None,true));
        assert!(prepare(&state,"test","session",false).await.unwrap_err().contains("离线"));assert!(!control.stopped.load(Ordering::Acquire));assert!(state.player.lock().unwrap().is_some());
    }
}
