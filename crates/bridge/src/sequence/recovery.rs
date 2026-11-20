//! Shared registered-endpoint restore; cancellation is owned by the adapter's control actor.
use super::*;
use crate::listens::{cursor, sync::Sync, Owner, Store};
use anyhow::Context;
use std::sync::Arc;
#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;

pub struct Restored {
    pub command: Value,
    pub handle: cursor::Handle,
    pub profile: ConnectionProfile,
}
pub struct BoundFailure {
    pub report: Value,
    pub handle: cursor::Handle,
    error: anyhow::Error,
}
impl std::fmt::Debug for BoundFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundFailure")
            .field("report", &self.report)
            .field("error", &self.error)
            .finish()
    }
}
impl std::fmt::Display for BoundFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for BoundFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.error.as_ref())
    }
}
pub async fn restore(
    config: Config,
    store: Arc<Store>,
    owner: Owner,
    sync: Arc<Sync>,
) -> Result<Option<Restored>> {
    let saved = store.clone();
    let user = owner.clone();
    let endpoint = config.endpoint.clone();
    let frozen = tokio::task::spawn_blocking(move || {
        saved.drain()?;
        cursor::freeze(saved, &user, &endpoint)
    })
    .await??;
    let Some((checkpoint, handle)) = frozen else {
        config.recovery_unavailable(&owner).await?;
        return Ok(None);
    };
    let body = checkpoint
        .request
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing frozen recovery request"))?;
    let mut backoff = 1;
    loop {
        let mut accepted = None;
        #[cfg(any(windows,target_os="linux"))] let mut system_suspend=None;
        let result:Result<Option<Restored>>=async {
            let saved=handle.clone();
            if !tokio::task::spawn_blocking(move ||saved.is_current()).await??{return Ok(None);}
            if !config.recovery_supported_timed(&owner,checkpoint.measured_version).await?{bail!("core no longer supports measured queue recovery");}
            loop {
                let saved=store.clone();let user=owner.clone();
                let pending=tokio::task::spawn_blocking(move ||saved.pending(&user)).await??;
                if pending.is_empty(){break;}
                if sync.flush().await?==0 {
                    let saved=store.clone();let user=owner.clone();
                    if !tokio::task::spawn_blocking(move ||saved.pending(&user)).await??.is_empty() {
                        bail!("pending listening journal could not be acknowledged before recovery");
                    }
                }
            }
            let saved=store.clone();let evidence=checkpoint.clone();
            if tokio::task::spawn_blocking(move ||cursor::rejected_evidence(&saved,&evidence)).await?? {
                bail!("source listening journal was rejected; retained cursor requires inspection before recovery");
            }
            #[cfg(not(any(windows,target_os="linux")))] let request=body.clone();
            #[cfg(any(windows,target_os="linux"))] let mut request=body.clone();
            #[cfg(any(windows,target_os="linux"))] {
                system_suspend=tokio::task::spawn_blocking(rmusic_audio::output::power::held_stamp).await??;
                if let Some(stamp)=&system_suspend {
                    let status=config.status().await?;
                    if status["core_id"]!=owner.core_id||status["capabilities"]["system_suspend_recovery"]!=true {return Err(rmusic_audio::output::DeviceUnavailable("核心尚不支持系统挂起恢复，请升级匹配的开发 Core 后再播放；游标已保留".into()).into());}
                    request["system_suspend"]=json!(stamp);
                }
            }
            let response=config.request_json(&checkpoint.zone,"recover",&request).await?;
            if checkpoint.measured_version==2&&response["measured_version"]!=2 {bail!("recovery timing version mismatch");}
            if response["waiting_for_play"]==true&&response["accepted"]==false {return Ok(None);}
            #[cfg(any(windows,target_os="linux"))] if let Some(stamp)=&system_suspend {if response["accepted"]==true&&response["system_suspend_ack"]!=json!(stamp){return Err(rmusic_audio::output::DeviceUnavailable("核心没有确认本次系统挂起，已拒绝恢复输出".into()).into());}}
            if response["accepted"]!=true||response["finished"]==true {
                #[cfg(any(windows,target_os="linux"))] if let Some(stamp)=system_suspend.clone().filter(|_|response["accepted"]==true) {
                    if response["zone"]["id"]!=checkpoint.zone||response["zone"]["endpoint_id"]!=config.endpoint||response["zone"]["status"]!="stopped"||!response["zone"]["playback_id"].is_null(){bail!("invalid terminal recovery identity");}
                    tokio::task::spawn_blocking(move||rmusic_audio::output::power::acknowledge(&stamp)).await??;
                }
                handle.retire()?;let saved=store.clone();tokio::task::spawn_blocking(move ||saved.drain()).await??;
                return Ok(None);
            }
            accepted=Some(response["zone"].clone());
            let recovered=&response["zone"];
            let latest=config.zone(&checkpoint.zone).await?;
            let zone=&latest;
            if zone["playback_id"]!=recovered["playback_id"]||!matches!(zone["status"].as_str(),Some("playing"|"paused")) {
                handle.retire()?;return Ok(None);
            }
            if zone["id"]!=checkpoint.zone||zone["endpoint_id"]!=config.endpoint
                ||zone["playback_id"].as_str().is_none_or(|id|uuid::Uuid::parse_str(id).is_err())
                ||!matches!(zone["status"].as_str(),Some("playing"|"paused")) {bail!("invalid recovered zone identity");}
            let track=zone["track"]["id"].as_str().context("recovered zone has no track")?;
            let url=config.playback_ticket(track,&checkpoint.zone).await?;
            #[cfg(any(windows,target_os="linux"))] if let Some(stamp)=system_suspend.clone(){tokio::task::spawn_blocking(move||rmusic_audio::output::power::acknowledge(&stamp)).await??;}
            let mut command=zone.clone();command["type"]=json!("play");command["zone_id"]=zone["id"].clone();
            if checkpoint.measured_version==2 {command["measured_version"]=json!(2);}
            command["paused"]=json!(zone["status"]=="paused");command["url"]=json!(url);
            Ok(Some(Restored {command,handle:handle.clone(),profile:config.profile.clone()}))
        }.await;
        match result {
            Ok(result) => return Ok(result),
            Err(error) => {
                let retry = error.downcast_ref::<reqwest::Error>().is_some_and(|error| {
                    !error
                        .status()
                        .is_some_and(|status| status.is_client_error())
                });
                if !retry {
                    if let Some(zone) = accepted {
                        #[cfg(not(any(windows,target_os="linux")))] let suspend=Value::Null;
                        #[cfg(any(windows,target_os="linux"))] let suspend=system_suspend.as_ref().map_or(Value::Null,|stamp|json!(stamp));
                        let device_failed=rmusic_audio::output::is_device_unavailable(&error);
                        let report=json!({"type":"error","zone_id":checkpoint.zone,"track_id":zone["track"]["id"],"core_playback_id":zone["playback_id"],"device_failed":device_failed,"recoverable":device_failed,"system_suspend":suspend,"error":error.to_string()});
                        return Err(BoundFailure {report,handle:handle.clone(),error}.into());
                    }
                    return Err(error);
                }
                eprintln!("Durable queue recovery waiting for core; retry in {backoff}s");
                tokio::time::sleep(Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(30);
            }
        }
    }
}
