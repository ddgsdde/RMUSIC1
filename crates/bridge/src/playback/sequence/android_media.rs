use super::*;
use rmusic_audio::output::android::{Media,SequenceMedia};

pub(super) fn bind(config:&Config,plan:&Plan,sequence:&PlaybackSequence,tracks:&Tracks,group:Option<&str>)->Result<()> {
    if let Some(group)=group {keep_transport(config,plan,tracks,group)?;}
    let metadata=tracks.clone();let config=config.clone();let group=group.map(String::from);let zone=plan.zone_id.clone();
    let runtime=tokio::runtime::Handle::current();let controls=tracks.clone();
    output::android::backend()?.bind_sequence(&plan.control,SequenceMedia {
        sequence:sequence.clone(),
        metadata:Arc::new(move|id| {
            let tracks=metadata.lock().map_err(|_|anyhow::anyhow!("Android metadata lock unavailable"))?;
            let entry=tracks.get(id).context("Android track metadata unavailable")?;
            Ok(Media{track_id:entry.track.clone(),playback_id:id.into(),title:entry.metadata["title"].as_str().unwrap_or(&entry.track).into(),
                artist:entry.metadata["artist"].as_str().unwrap_or("").into(),duration:entry.request.as_ref().and_then(|r|r.duration).unwrap_or(0.)})
        }),
        action:Arc::new(move|update,action| {
            let track=controls.lock().ok().and_then(|tracks|tracks.get(&action.playback_id).map(|entry|entry.track.clone()));
            let Some(track)=track else{return;};
            let config=config.clone();let group=group.clone();let zone=zone.clone();
            runtime.spawn(async move {
                let result=async {
                    if let Some(group)=group {
                        let path=if action.action=="stop"{format!("api/v1/sync/starts/{group}/cancel")}else{format!("api/v1/sync/starts/{group}/control")};
                        let body=if action.action=="stop"{json!({})}else{json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":if action.action=="play"{"resume"}else{"pause"}})};
                        config.post_path(&path,&body).await?.error_for_status()?;
                    }else {
                        config.post(&zone,&action.action,&json!({"control":{"track_id":track,"playback_id":action.playback_id,"endpoint_id":config.endpoint},"position":update.position})).await?.error_for_status()?;
                    }
                    Ok::<_,anyhow::Error>(())
                }.await;
                if let Err(error)=result {eprintln!("Android media control failed: {error:#}");}
            });
        }),
    })
}

// The group pause barrier destroys PCM workers; system controls must outlive those workers.
fn keep_transport(config:&Config,plan:&Plan,tracks:&Tracks,group:&str)->Result<()> {
    use std::sync::OnceLock;
    static GROUPS:OnceLock<Mutex<std::collections::HashSet<String>>>=OnceLock::new();
    let id=format!("group:{group}:{}",config.endpoint);
    if !GROUPS.get_or_init(Default::default).lock().map_err(|_|anyhow::anyhow!("Android transport lock unavailable"))?.insert(id.clone()){return Ok(());}
    let config=config.clone();let group=group.to_owned();let tracks=tracks.clone();let title=plan.track_id.clone();
    let backend=output::android::backend()?.clone();
    tokio::spawn(async move {
        let mut failures=0;
        loop {
            let job=match config.get_path(&format!("api/v1/sync/starts/{group}")).await {
                Ok(job)=>{failures=0;job},Err(_)=>{failures+=1;if failures>=3{break;}tokio::time::sleep(Duration::from_millis(500)).await;continue;}
            };
            if !matches!(job["phase"].as_str(),Some("preparing"|"committed"|"playing"|"pausing"|"paused"|"seeking"))||!job["members"].as_array().is_some_and(|rows|rows.iter().any(|m|m["endpoint_id"]==config.endpoint)){break;}
            let metadata=tracks.lock().ok().and_then(|rows|rows.values().find(|e|e.track==job["track_id"]).map(|e|e.metadata.clone())).unwrap_or(Value::Null);
            let state=json!({"title":metadata["title"].as_str().or(job["title"].as_str()).unwrap_or(&title),"artist":metadata["artist"].as_str().unwrap_or(""),
                "media_id":format!("{group}:{}",job["transport"]["revision"].as_u64().unwrap_or(0)),"duration_ms":metadata["duration"].as_f64().unwrap_or(0.)*1000.,
                "position_ms":job["playhead"]["position"].as_f64().unwrap_or(0.)*1000.,"paused":job["phase"]!="playing"});
            let output=backend.clone();let key=id.clone();
            let action=tokio::task::spawn_blocking(move||output.transport(&key,Some(state))).await;
            let Ok(Ok(action))=action else{break;};
            if let Some(action)=action.filter(|action|matches!(action.as_str(),"play"|"pause"|"stop")) {
                let path=if action=="stop"{format!("api/v1/sync/starts/{group}/cancel")}else{format!("api/v1/sync/starts/{group}/control")};
                let body=if action=="stop"{json!({})}else{json!({"request_id":uuid::Uuid::new_v4().to_string(),"action":if action=="play"{"resume"}else{"pause"}})};
                for attempt in 0..2 {match config.post_path(&path,&body).await {Ok(response) if response.status().is_success()=>break,Ok(response) if response.status().is_client_error()=>break,_=>{if attempt==0{tokio::time::sleep(Duration::from_millis(250)).await;}}}}
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let output=backend.clone();let key=id.clone();let _=tokio::task::spawn_blocking(move||output.transport(&key,None)).await;
        if let Ok(mut groups)=GROUPS.get_or_init(Default::default).lock(){groups.remove(&id);}
    });Ok(())
}
