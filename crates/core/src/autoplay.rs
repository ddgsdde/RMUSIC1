//! Prepare recommendations outside the zone lock; apply only to the requesting queue.
use crate::{models::{Track,Zone},state::AppState};
use serde_json::json;

pub struct Batch {
    signature: String,
    request_id: String,
    waiting: bool,
    items: Option<Vec<Track>>,
    skipped: Vec<String>,
}
fn signature(z:&Zone)->String {
    let tail=z.queue.iter().rev().take(12).map(|t|t.id.as_str()).collect::<Vec<_>>();
    format!("{}:{}:{tail:?}",z.endpoint_id,z.queue.len())
}
fn eligible(z:&Zone)->bool {
    z.autoplay.enabled && z.repeat=="off" && z.track.is_some() && !z.queue.is_empty()
        && z.queue.len().saturating_sub(z.queue_index+1)<=3
}
pub async fn cancel(s:&AppState,id:&str) {
    if s.autoplay_batches.lock().await.remove(id).is_some() {
        s.emit("autoplay",json!({"zone_id":id,"status":"idle"}));
    }
}
pub(crate) async fn waiting(s:&AppState,id:&str)->bool {
    s.autoplay_batches.lock().await.get(id).is_some_and(|batch|batch.waiting)
}
/// Caller holds zone_lock. Playback continues while provider I/O runs in the background.
pub async fn prepare(s:&AppState,z:&Zone) {prepare_with_exclusions(s,z,vec![]).await;}
async fn prepare_with_exclusions(s:&AppState,z:&Zone,skipped:Vec<String>) {
    if !eligible(z) || z.status!="playing" {return;}
    let key=signature(z);
    let mut batches=s.autoplay_batches.lock().await;
    if batches.get(&z.id).is_some_and(|b|b.signature==key) {return;}
    let request_id=uuid::Uuid::new_v4().to_string();
    batches.insert(z.id.clone(),Batch{signature:key.clone(),request_id:request_id.clone(),waiting:false,items:None,skipped});
    drop(batches);
    s.emit("autoplay",json!({"zone_id":z.id,"status":"preparing"}));
    let s=s.clone();let original=z.clone();
    tokio::spawn(async move {
        let mut retry=5;
        loop {
            let Ok(slot)=s.autoplay_slots.clone().acquire_owned().await else {return;};
            let excluded={
                let batches=s.autoplay_batches.lock().await;
                let Some(batch)=batches.get(&original.id).filter(|b|b.request_id==request_id) else {return;};
                batch.skipped.clone()
            };
            let context=crate::radio::QueueContext::from_zone(&original);
            let seed=original.queue.last().map(|t|t.id.clone());
            let result=async {
                let user=crate::radio::owner(&s,&original.id).await?;
                crate::radio::next_for_queue(&s,user,seed,excluded,context,10).await
            }.await;
            drop(slot);
            let (mut items,message)=match result {
                Ok(items)=>(items,None),
                Err(e)=>(vec![],Some(e.2)),
            };
            {
                let _lock=s.zone_lock.lock().await;
                let waiting={
                    let mut batches=s.autoplay_batches.lock().await;
                    let Some(batch)=batches.get_mut(&original.id).filter(|b|b.request_id==request_id) else {return;};
                    items.retain(|track|!batch.skipped.contains(&track.id));
                    batch.items=Some(items.clone());batch.waiting
                };
                s.emit("autoplay",json!({"zone_id":original.id,"status":if items.is_empty(){"unavailable"}else{"ready"},"message":message}));
                if !items.is_empty() {
                    if !waiting {return;}
                    let zone=s.zones.read().await.get(&original.id).cloned();
                    let Some(mut zone)=zone.filter(|z|z.status=="stopped"&&eligible(z)&&signature(z)==key) else {return;};
                    if s.zones.read().await.values().any(|other|other.id!=zone.id&&other.endpoint_id==zone.endpoint_id&&other.status=="playing") {return;}
                    append(&mut zone,items);
                    s.autoplay_batches.lock().await.remove(&zone.id);
                    if let Err(e)=crate::zones::dispatch(&s,&zone,"play").await {
                        zone.status="stopped".into();zone.playback_id=None;s.emit("playback_error",json!({"zone_id":zone.id,"message":e.2}));
                    } else {let _=crate::stats::record_progress(&s,&zone).await;}
                    let _=s.save_zone(zone).await;
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(retry)).await;
            retry=(retry*2).min(30);
        }
    });
}
/// Caller holds zone_lock. Returns true when a prefetched batch can play immediately.
pub async fn continue_queue(s:&AppState,z:&mut Zone)->bool {
    if !eligible(z)||z.queue_index+1!=z.queue.len() {return false;}
    prepare(s,z).await;
    let mut batches=s.autoplay_batches.lock().await;
    let Some(batch)=batches.get_mut(&z.id).filter(|b|b.signature==signature(z)) else {return false;};
    match batch.items.take() {
        Some(items) if !items.is_empty()=>{batches.remove(&z.id);drop(batches);append(z,items);true},
        Some(items)=>{batch.items=Some(items);batch.waiting=true;false},
        None=>{batch.waiting=true;false},
    }
}
/// Metadata remains available to ordinary EOF fallback until a reserved source is heard.
pub(crate) async fn reserve(s:&AppState,z:&Zone)->(Option<Vec<Track>>,bool) {
    if !eligible(z)||z.queue_index+1!=z.queue.len() {return (None,false);}
    // Do not replace the live fallback batch with another batch beyond unheard recommendations.
    let live=s.zones.read().await.get(&z.id).is_some_and(|live|live.endpoint_id==z.endpoint_id&&live.queue.iter().map(|track|&track.id).eq(z.queue.iter().map(|track|&track.id)));
    if !live {return (None,false);}
    prepare(s,z).await;
    let batches=s.autoplay_batches.lock().await;
    match batches.get(&z.id).filter(|batch|batch.signature==signature(z)).and_then(|batch|batch.items.as_ref()) {
        Some(items) if !items.is_empty()=>(Some(items.clone()),false),
        Some(_)=>(None,false),
        None=>(None,true),
    }
}
pub(crate) fn extend_queue(z:&mut Zone,items:Vec<Track>)->usize {
    let remove=z.queue.len().saturating_sub(200).min(z.queue_index);
    if remove>0 {z.queue.drain(..remove);z.queue_index-=remove;}
    let next=z.queue.len();z.queue.extend(items);next
}
fn append(z:&mut Zone,items:Vec<Track>) {
    // Bound automatic queue history; saved playlists are independent of the active queue.
    let next=extend_queue(z,items);z.queue_index=next;z.track=z.queue.get(next).cloned();z.position=0.;z.status="playing".into();z.renew_playback();z.updated_at=chrono::Utc::now().timestamp_millis();
}

pub async fn preview(
    axum::extract::State(s):axum::extract::State<AppState>,a:crate::auth::Auth,
    axum::extract::Path(id):axum::extract::Path<String>,
)->crate::error::ApiResult<axum::Json<serde_json::Value>> {
    a.require("read")?;
    crate::accounts::scope::check(&s,&a,"zone",&id).await?;
    let batches=s.autoplay_batches.lock().await;
    let batch=batches.get(&id);
    let items=batch.and_then(|b|b.items.as_ref());
    let status=match (batch,items) {
        (None,_)=>"idle",
        (_,None)=>"preparing",
        (Some(b),Some(rows)) if rows.is_empty()&&b.waiting=>"waiting",
        (_,Some(rows)) if rows.is_empty()=>"unavailable",
        _=>"ready",
    };
    Ok(axum::Json(json!({"status":status,"items":items.map(|rows|rows.iter().take(3).map(crate::zones::snapshots::track).collect::<Vec<_>>()).unwrap_or_default()})))
}

#[derive(serde::Deserialize)]
pub struct Skip { track_id: String }
pub async fn skip(
    axum::extract::State(s):axum::extract::State<AppState>,a:crate::auth::Auth,
    axum::extract::Path(id):axum::extract::Path<String>,axum::Json(skip):axum::Json<Skip>,
)->crate::error::ApiResult<axum::Json<serde_json::Value>> {
    a.require("control")?;
    crate::accounts::scope::check(&s,&a,"zone",&id).await?;
    let _lock=s.zone_lock.lock().await;
    let zone=s.zones.read().await.get(&id).cloned().ok_or_else(crate::error::ApiError::missing)?;
    let mut batches=s.autoplay_batches.lock().await;
    let batch=batches.get_mut(&id).filter(|batch|batch.signature==signature(&zone))
        .ok_or_else(||crate::error::ApiError::bad("电台候选已变化，请重新读取"))?;
    let rows=batch.items.as_mut().filter(|rows|rows.first().is_some_and(|track|track.id==skip.track_id))
        .ok_or_else(||crate::error::ApiError::bad("电台候选已变化，请重新读取"))?;
    rows.remove(0);
    let items=rows.iter().take(3).map(crate::zones::snapshots::track).collect::<Vec<_>>();
    batch.skipped.push(skip.track_id);
    // The current batch stays authoritative until its remaining candidates are consumed.
    let skipped=if items.is_empty(){batches.remove(&id).map(|batch|batch.skipped)}else{None};
    drop(batches);
    crate::sequence::invalidate_zone(&s,&id,Some(&zone)).await;
    if let Some(skipped)=skipped{prepare_with_exclusions(&s,&zone,skipped).await;}
    s.emit("autoplay",json!({"zone_id":id,"status":if items.is_empty(){"preparing"}else{"ready"}}));
    Ok(axum::Json(json!({"items":items})))
}
