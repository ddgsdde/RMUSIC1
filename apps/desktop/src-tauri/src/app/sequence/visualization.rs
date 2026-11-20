use super::*;
/// Android's native audio worker keeps publishing while the WebView is backgrounded.
pub(super) fn start(core:Config,zone:String,sequence:PlaybackSequence,control:Arc<PlaybackControl>,entries:Entries) {
    tokio::spawn(async move {
        let mut poll=Instant::now();let mut until=Instant::now();let mut tick=tokio::time::interval(Duration::from_millis(100));
        while !control.stopped.load(Ordering::Acquire) {
            tick.tick().await;
            if control.paused.load(Ordering::Relaxed){continue;}
            if Instant::now()>=poll {poll=Instant::now()+Duration::from_secs(1);match core.visualization_requested(&zone).await {Ok(true)=>until=Instant::now()+Duration::from_secs(2),Ok(false)=>{},Err(_)=>poll=Instant::now()+Duration::from_secs(30)}}
            if Instant::now()>=until {continue;}
            control.visualization.renew(600);
            let Ok(point)=sequence.position(&control) else{continue;};
            let track=entries.lock().ok().and_then(|rows|rows.get(&point.playback_id).map(|e|e.track.clone()));
            let Some(track)=track else{continue;};
            let frames=control.visualization.snapshot(&point.playback_id,point.position);
            if !frames.is_empty(){let _=core.publish_visualization(&zone,&json!({"version":1,"core_playback_id":point.playback_id,"track_id":track,"position":point.position,"frames":frames})).await;}
        }
    });
}
