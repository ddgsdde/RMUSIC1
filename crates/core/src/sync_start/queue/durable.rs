//! Private v2 source authority and confirmed output facts; never a playback restore command.
use super::*;
use anyhow::Context;
use rusqlite::{params,Transaction};

struct Epoch {id:String,group:String,listener:String,data:Value}
pub(super) fn epoch_key(group:&str,token:&Value)->String {crate::auth::hash(&json!([group,token["core_clock_id"],token["generation"]]).to_string())}
fn epoch(job:&Job)->Option<Epoch> {
    if job.request.measured_version!=2{return None;}
    let q=job.queue.as_ref()?;let first=&job.members[0];
    let id=epoch_key(&job.request.request_id,&first.token);
    let deadline_at=job.deadline.filter(|n|*n>0).map(|deadline|chrono::Utc::now().timestamp_millis()+((deadline as i128-crate::clock::now_ns() as i128)/1_000_000) as i64);
    let data=json!({"storage_version":1,"measured_version":2,"group_id":job.request.request_id,"core_clock_id":first.token["core_clock_id"],"generation":first.token["generation"],"deadline_core_ns":job.deadline,"expires_core_ns":job.expires,"dispatched":job.dispatched,"phase":job.phase,"segment":q.segment,"position":q.position,"jump_generation":q.jump_generation,"listened_seconds":q.total_seconds(),"members":job.members.iter().map(|m|json!({"zone_id":m.zone.id,"endpoint_id":m.zone.endpoint_id,"start_playback_id":m.playback_id,"current_playback_id":m.zone.playback_id,"queue_index":m.zone.queue_index,"frames":m.frames,"sample_rate":m.rate,"gate":m.gate,"last_seen_core_ns":m.last_seen,"final_report_received":m.final_report})).collect::<Vec<_>>()});
    let mut data=data;data["deadline_at_ms"]=json!(deadline_at);
    data["current_logical_playback_id"]=json!(q.logical[q.segment]);
    for (saved,m) in data["members"].as_array_mut().unwrap().iter_mut().zip(&job.members){saved["deadline_host_ns"]=json!(m.host_deadline);}
    Some(Epoch{id,group:job.request.request_id.clone(),listener:job.listener.clone(),data})
}
fn save_epoch(tx:&Transaction<'_>,epoch:&Epoch,now:i64)->anyhow::Result<()> {
    let mut data=epoch.data.clone();
    if data["dispatched"]!=true {
        let previous:Option<String>=tx.query_row("SELECT data FROM sync_output_epochs WHERE id=?1",[&epoch.id],|r|r.get(0)).optional()?;
        if let Some(previous)=previous {let previous:Value=serde_json::from_str(&previous)?;
            // A paused playhead may already refer to another source. Keep retired output authority separately.
            if previous["output_members"].is_array(){data["output_members"]=previous["output_members"].clone();}
            else if previous["dispatched"]==true{data["output_members"]=previous["members"].clone();}
        }
    }
    tx.execute("INSERT INTO sync_output_epochs(id,group_id,listener_id,data,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5) ON CONFLICT(id) DO UPDATE SET data=json_set(excluded.data,'$.deadline_at_ms',COALESCE(json_extract(sync_output_epochs.data,'$.deadline_at_ms'),json_extract(excluded.data,'$.deadline_at_ms'))),updated_at=excluded.updated_at",params![epoch.id,epoch.group,epoch.listener,data.to_string(),now])?;Ok(())
}
pub(in crate::sync_start) fn checkpoint_tx(tx:&Transaction<'_>,job:&Job,indices:&[usize],now:i64)->anyhow::Result<()> {
    let Some(epoch)=epoch(job)else{return Ok(())};save_epoch(tx,&epoch,now)?;
    let q=job.queue.as_ref().unwrap();
    // Update only touched sources. Whole playlists and known-source maps are not rewritten per tick.
    for(i,m)in job.members.iter().enumerate(){for index in indices {
        if let Some(p)=q.heard[i].get(index).filter(|p|p.rate>0) {
            tx.execute("UPDATE sync_output_sources SET frames=?4,sample_rate=?5,position=?6,ended=?7,updated_at=?8 WHERE epoch_id=?1 AND endpoint_id=?2 AND queue_index=?3",params![epoch.id,m.zone.endpoint_id,*index as i64,i64::try_from(p.frames)?,p.rate,p.position,p.ended,now])?;
        }
    }}Ok(())
}
pub(in crate::sync_start) struct Preparation {epoch:Epoch,endpoint:String,owner:String,grants:Vec<(String,usize,f64,Value)>}
async fn preparation(s:&AppState,job:&Job,member:usize,items:&[(String,usize)])->ApiResult<Option<Preparation>> {
    let Some(epoch)=epoch(job)else{return Ok(None)};let m=&job.members[member];let q=job.queue.as_ref().unwrap();
    let owner=s.bridge_owners.read().await.get(&m.zone.endpoint_id).cloned().ok_or_else(||ApiError::bad("端点认证归属不可用"))?;
    let endpoint=m.zone.endpoint_id.clone();let current=m.zone.playback_id.clone().ok_or_else(||ApiError::bad("分组当前源身份不可用"))?;
    let mut grants=Vec::with_capacity(items.len()+1);
    for(playback,index)in std::iter::once((current,m.zone.queue_index)).chain(items.iter().cloned()) {
        let track=m.zone.queue.get(index).ok_or_else(||ApiError::bad("分组源曲序无效"))?;let logical=q.logical.get(index).ok_or_else(||ApiError::bad("分组逻辑曲序无效"))?;
        let base=if index==q.segment{q.position}else{0.};
        let grant=json!({"storage_version":1,"measured_version":2,"group_id":epoch.group,"epoch_id":epoch.id,"listener_id":epoch.listener,"zone_id":m.zone.id,"endpoint_id":endpoint,"playback_id":playback,"queue_index":index,"logical_playback_id":logical,"source_position_base":base,"credited_seconds_before_epoch":q.credited.get(&index).copied().unwrap_or(0.),"track":track,"start_playback_id":m.playback_id,"start":m.token});
        grants.push((playback,index,base,grant));
    }
    Ok(Some(Preparation{epoch,endpoint,owner,grants}))
}
/// Freeze initial source identity in the same transaction as the new output epoch.
pub(in crate::sync_start) async fn preparing(s:&AppState,job:&Job)->anyhow::Result<Vec<Preparation>> {
    let mut prepared=Vec::new();if job.phase=="preparing"&&job.request.measured_version==2 {
        for member in 0..job.members.len(){if let Some(value)=preparation(s,job,member,&[]).await.map_err(|error|anyhow::anyhow!(error.2))?{prepared.push(value);}}
    }Ok(prepared)
}
pub(in crate::sync_start) fn prepare_tx(tx:&Transaction<'_>,prepared:&[Preparation],now:i64)->anyhow::Result<()> {
    for item in prepared {
        let account:String=tx.query_row("SELECT user_id FROM sessions WHERE hash=?1",[&item.owner],|r|r.get(0)).context("group endpoint account is unavailable")?;
        save_epoch(tx,&item.epoch,now)?;
        for(playback,index,base,grant)in &item.grants {
            let mut grant=grant.clone();grant["endpoint_account_id"]=json!(account);let data=grant.to_string();
            let previous:Option<String>=tx.query_row("SELECT grant FROM sync_output_sources WHERE epoch_id=?1 AND endpoint_id=?2 AND playback_id=?3",params![item.epoch.id,item.endpoint,playback],|r|r.get(0)).optional()?;
            if previous.as_ref().is_some_and(|old|old!=&data){anyhow::bail!("group source authority changed");}
            tx.execute("INSERT INTO sync_output_sources(epoch_id,endpoint_id,playback_id,queue_index,account_id,grant,position,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(epoch_id,endpoint_id,playback_id) DO NOTHING",params![item.epoch.id,item.endpoint,playback,*index as i64,account,data,base,now])?;
        }
    }Ok(())
}
pub(super) async fn issue(s:&AppState,job:&Job,member:usize,items:&[(String,usize)])->ApiResult<()> {
    let Some(prepared)=preparation(s,job,member,items).await?else{return Ok(())};
    s.db.run(move|c|{let tx=c.transaction()?;prepare_tx(&tx,&[prepared],chrono::Utc::now().timestamp_millis())?;tx.commit()?;Ok(())
    }).await?;Ok(())
}
