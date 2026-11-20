//! Durable completion work. A cancelled HTTP reader never owns the lifetime of a job.
use crate::state::AppState;
use rusqlite::{params,OptionalExtension};
use serde_json::{json,Value};
use std::time::Duration;

pub async fn enqueue(s:&AppState,kind:&str,key:&str,payload:Value,priority:i64 )->anyhow::Result<String> {
    let kind=kind.to_owned();let key=key.to_owned();
    let id=s.db.run(move|c|{
        let tx=c.transaction()?;
        let id=enqueue_tx(&tx,&kind,&key,&payload,priority)?;
        tx.commit()?;Ok(id)
    }).await?;s.emit("jobs",json!({}));Ok(id)
}
pub(crate) fn enqueue_tx(tx:&rusqlite::Transaction<'_>,kind:&str,key:&str,payload:&Value,priority:i64)->anyhow::Result<String> {
    if let Some(id)=tx.query_row("SELECT id FROM jobs WHERE kind=?1 AND cursor=?2 AND status IN ('pending','running','interrupted','paused','waiting') ORDER BY created_at DESC LIMIT 1",params![kind,key],|r|r.get::<_,String>(0)).optional()? {
        tx.execute("UPDATE jobs SET priority=MAX(priority,?2) WHERE id=?1",params![id,priority])?;
        return Ok(id);
    }
    let id=uuid::Uuid::new_v4().to_string();let now=chrono::Utc::now().timestamp();
    tx.execute("INSERT INTO jobs(id,kind,status,created_at,updated_at,cursor,payload,priority,phase) VALUES(?1,?2,'pending',?3,?3,?4,?5,?6,'等待执行')",params![id,kind,now,key,payload.to_string(),priority])?;
    Ok(id)
}
pub async fn progress(s:&AppState,id:&str,phase:&str,done:u64,total:u64)->anyhow::Result<bool> {
    let id=id.to_owned();let phase=phase.to_owned();
    s.db.run(move|c|Ok(c.execute("UPDATE jobs SET phase=?2,processed=?3,total=?4,updated_at=?5 WHERE id=?1 AND status='running'",params![id,phase,done as i64,total as i64,chrono::Utc::now().timestamp()])?>0)).await
}
pub async fn disposition(s:&AppState,id:&str)->anyhow::Result<bool> {
    let id=id.to_owned();s.db.run(move|c|Ok(c.query_row("SELECT status='running' FROM jobs WHERE id=?1",[id],|r|r.get(0))?)).await
}
pub async fn waiting(s:&AppState,id:&str,reason:&str)->anyhow::Result<()> {
    let id=id.to_owned();let reason=reason.to_owned();s.db.run(move|c|{c.execute("UPDATE jobs SET status='waiting',phase=?2,updated_at=?3,next_try=?3+5 WHERE id=?1 AND status NOT IN ('paused','cancelled')",params![id,reason,chrono::Utc::now().timestamp()])?;Ok(())}).await
}
pub async fn finish(s:&AppState,id:&str,status:&str,error:Option<String>)->anyhow::Result<()> {
    let id=id.to_owned();let status=status.to_owned();s.db.run(move|c|{c.execute("UPDATE jobs SET status=?2,error=?3,phase=CASE WHEN ?2='completed' THEN '完成' ELSE phase END,updated_at=?4 WHERE id=?1 AND status NOT IN ('paused','cancelled')",params![id,status,error,chrono::Utc::now().timestamp()])?;if status=="completed"{if let Some(local)=c.query_row("SELECT cursor FROM jobs WHERE id=?1 AND kind='playlist_sync' AND EXISTS(SELECT 1 FROM playlist_operations WHERE local_id=jobs.cursor AND status='pending')",[id],|r|r.get::<_,String>(0)).optional()?{crate::playlist_sync::schedule(c,&local)?;}}Ok(())}).await?;s.emit("jobs",json!({}));Ok(())
}
pub fn spawn(s:AppState) {
    if !s.resources.start_worker(){return;}
    tokio::spawn(async move {
    if let Err(error)=crate::media::cleanup_incoming(&s).await{tracing::warn!(%error,"uncommitted cache staging cleanup deferred");}
    if let Ok(downloads)=s.db.run(|c|Ok(c.prepare("SELECT id,track_id FROM downloads WHERE status IN ('pending','interrupted')")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await{for(id,track)in downloads{let _=enqueue(&s,"download",&id,json!({"download_id":id,"track_id":track}),0).await;}}
    let mut tick=tokio::time::interval(Duration::from_millis(500));let mut scrape_tick=0u32;loop{
        tick.tick().await;scrape_tick+=1;if scrape_tick==1||scrape_tick>=120{scrape_tick=1;let _=crate::catalog_scrape::schedule(&s).await;}
        if let Err(error) = crate::media::discard_inactive_previews(&s).await {
            tracing::warn!(%error, "could not release finished playback buffers");
        }
        let next=s.db.run(|c|Ok(c.query_row("SELECT id,kind,payload FROM jobs WHERE kind IN ('cache_fill','download','scan','playlist_sync','playlist_directory','playlist_page','library_files','cache_repair','fingerprint','identity_index','catalog_scrape','artwork_check','artwork_refresh') AND status IN ('pending','interrupted','waiting') AND next_try<=?1 ORDER BY priority DESC,created_at,id LIMIT 1",[chrono::Utc::now().timestamp()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional()?)).await;
        let Ok(Some((id,kind,payload)))=next else{continue;};
        if matches!(kind.as_str(),"scan"|"fingerprint"|"catalog_scrape")&&(s.zones.read().await.values().any(|z|z.status=="playing")||s.foreground_requests.load(std::sync::atomic::Ordering::Acquire)>0) {let _=waiting(&s,&id,"播放优先，等待缓冲空闲").await;continue;}
        if kind=="cache_fill" && !crate::cache_repair::admitted(&s,&serde_json::from_str::<Value>(&payload).unwrap_or_default()).await.unwrap_or(false) {let _=waiting(&s,&id,"等待补齐并发名额或主任务恢复").await;continue;}
        let Some(running)=s.resources.job(&id)else{continue;};
        let permit=match s.resources.background(&s,if matches!(kind.as_str(),"scan"|"playlist_sync"|"playlist_directory"|"playlist_page"|"library_files"|"fingerprint"|"catalog_scrape"){64*1024*1024}else{1024*1024},kind=="fingerprint").await {Ok(p)=>p,Err(reason)=>{if kind=="playlist_sync"&&reason=="等待后台并发名额"{let _=s.db.run(|c|{c.execute("UPDATE jobs SET status='waiting',phase='等待歌单同步完成',next_try=unixepoch()+5,updated_at=unixepoch() WHERE kind IN ('download','cache_fill') AND status='running'",[])?;Ok(())}).await;}let _=waiting(&s,&id,reason).await;continue;}};
        let key=id.clone();let claimed=s.db.run(move|c|Ok(c.execute("UPDATE jobs SET status='running',error=NULL,phase='准备来源',updated_at=?2 WHERE id=?1 AND status IN ('pending','interrupted','waiting')",params![key,chrono::Utc::now().timestamp()])?>0)).await.unwrap_or(false);
        if !claimed{continue;}
        let task=s.clone();tokio::spawn(async move{let _permit=permit;let _running=running;let payload:Value=serde_json::from_str(&payload).unwrap_or_default();
            let operation=async{if kind=="catalog_scrape"{crate::catalog_scrape::run(&task,&id).await}else if matches!(kind.as_str(),"artwork_check"|"artwork_refresh"){crate::artwork_maintenance::run(&task,&id,&kind,&payload).await}else if kind=="identity_index"{crate::music_identity::run_index(&task,&id).await}else if kind=="fingerprint"{crate::fingerprints::run(&task,&id,&payload).await}else if kind=="cache_repair"{crate::cache_repair::run(&task,&id,&payload).await}else if kind=="library_files"{crate::library_files::run_job(&task,&id,&payload).await}else if kind=="playlist_page"{crate::playlist_pages::refresh(&task,&payload).await}else if kind=="playlist_sync"{crate::playlist_sync::run_job(&task,&id,&payload).await}else if kind=="playlist_directory"{crate::playlist_sync::refresh_directory(&task,&id,&payload).await}else if kind=="scan"{crate::library::run_scan_job(&task,&id).await}else if kind=="download" {crate::downloads::run_job(&task,&id,&payload).await}else{crate::media::fill_cache(&task,&id,&payload).await}};
            let cancel=async{if kind=="scan"{std::future::pending::<()>().await;}loop{tokio::time::sleep(Duration::from_millis(250)).await;if !disposition(&task,&id).await.unwrap_or(false)||task.resources.over_budget()||matches!(kind.as_str(),"download"|"cache_fill")&&task.traffic.playback_waiting()||kind=="catalog_scrape"&&task.setting("catalog_scrape_enabled",json!(false)).await!=true||matches!(kind.as_str(),"fingerprint"|"catalog_scrape")&&task.zones.read().await.values().any(|z|z.status=="playing")||kind=="catalog_scrape"&&task.foreground_requests.load(std::sync::atomic::Ordering::Acquire)>0||kind=="cache_fill"&&!crate::cache_policy::enabled(&task).await{break;}}};
            let result=crate::traffic::BACKGROUND.scope(true,async{tokio::select!{result=operation=>result,_=cancel=>Ok(false)}}).await;
            match result {Ok(true)=>{let _=finish(&task,&id,"completed",None).await;},Ok(false)=>{if matches!(kind.as_str(),"identity_index"|"catalog_scrape"|"artwork_check"|"artwork_refresh"){let key=id.clone();let _=task.db.run(move|c|{c.execute("UPDATE jobs SET status='pending',next_try=0,updated_at=unixepoch() WHERE id=?1 AND status='running'",[key])?;Ok(())}).await;}else if kind=="cache_repair"{let key=id.clone();let _=task.db.run(move|c|{c.execute("UPDATE jobs SET status='waiting',next_try=unixepoch()+5,updated_at=unixepoch() WHERE id=?1 AND status NOT IN ('paused','cancelled')",[key])?;Ok(())}).await;}else{let _=waiting(&task,&id,"已保存进度，等待资源或播放空闲").await;}},Err(error) if task.traffic.cooling_down()=>{let _=waiting(&task,&id,&format!("音源限流冷却，保留任务：{error}")).await;},Err(error)=>{tracing::warn!(%error,job=%id,"background job failed");let _=finish(&task,&id,"failed",Some(error.to_string())).await;}}
        });
    }});
}
