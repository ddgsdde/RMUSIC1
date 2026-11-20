use super::*;
use crate::offline_stats::SyncResult;
pub(super) async fn upload(
    state: &DesktopState,
    id: String,
) -> CommandResult<Option<(ListenOwner, SyncResult)>> {
    let Some(owner) = state.listens.owner(&id).map_err(err)? else {
        return Ok(None);
    };
    if let Err(error)=upload_groups(state,&id,&owner).await {eprintln!("Group final upload deferred: {error}");}
    let store = state.listens.clone();
    let selected = owner.clone();
    let mut rows = tokio::task::spawn_blocking(move || store.pending(&selected))
        .await
        .map_err(err)?
        .map_err(err)?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.iter().any(|row|row.snapshot.measured.is_some()) {
        let status=core_request(state,id.clone(),"GET".into(),"api/v1/status".into(),None).await?;
        if status["capabilities"]["measured_listens"]!=true {
            let store=state.listens.clone();let selected=owner.clone();
            rows=tokio::task::spawn_blocking(move ||store.pending_supported(&selected,false)).await.map_err(err)?.map_err(err)?;
            if rows.is_empty(){return Ok(None);}
        }
    }
    let body = json!({"core_id":owner.core_id,"user_id":owner.user_id,"client_id":state.listens.client_id,"sessions":rows.into_iter().map(|row|row.snapshot).collect::<Vec<_>>()});
    let result = core_request(
        state,
        id,
        "POST".into(),
        "api/v1/stats/offline".into(),
        Some(body),
    )
    .await?;
    Ok(Some((owner, serde_json::from_value(result).map_err(err)?)))
}
async fn upload_groups(state:&DesktopState,id:&str,owner:&ListenOwner)->CommandResult<()> {
    let store=state.listens.clone();let selected=owner.clone();
    let rows=tokio::task::spawn_blocking(move||rmusic_bridge::listens::group::pending(&store,&selected)).await.map_err(err)?.map_err(err)?;
    let Some(first)=rows.first()else{return Ok(());};
    let selected=connected_profile(state,id).await?;
    let config=rmusic_bridge::sequence::Config::new(selected,token(state,id)?,first.body["endpoint_id"].as_str().ok_or("分组回执缺少端点")?.into()).map_err(err)?;
    rmusic_bridge::listens::sync::Sync::new(state.listens.clone(),config,owner.clone()).flush_groups().await.map_err(err)?;Ok(())
}
/// Runs independently of the WebView; tokens remain in native memory.
pub(super) fn schedule(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let state = app.state::<DesktopState>();
        let mut changes=state.listens.changes.subscribe();
        loop {
            let mut retry=false;
            let profiles = state
                .tokens
                .lock()
                .map(|tokens| tokens.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for id in profiles {
                let result =
                    tokio::time::timeout(std::time::Duration::from_secs(10), upload(&state, id))
                        .await;
                let (owner,result)=match result {
                    Ok(Ok(Some(value)))=>value,
                    Ok(Ok(None))=>continue,
                    _=>{retry=true;continue;}
                };
                let rejected = !result.rejected.is_empty();
                let store = state.listens.clone();
                let ack =
                    tokio::task::spawn_blocking(move || store.acknowledge(&owner, &result)).await;
                if let Err(error) = ack.map_err(err).and_then(|r| r.map_err(err)) {
                    retry=true;
                    let _ = app.emit("native-cache-error", error);
                }
                if rejected {
                    let _ = app.emit(
                        "native-cache-error",
                        "部分离线聆听记录存在冲突，已保留在本机，请检查设备时间。",
                    );
                }
            }
            if let Some(error) = state.listens.take_error() {
                let _ = app.emit("native-cache-error", error);
            }
            tokio::select! {
                _=changes.changed()=>{tokio::time::sleep(std::time::Duration::from_secs(20)).await;changes.borrow_and_update();},
                _=tokio::time::sleep(std::time::Duration::from_secs(if retry{60}else{300}))=>{}
            }
        }
    });
}
