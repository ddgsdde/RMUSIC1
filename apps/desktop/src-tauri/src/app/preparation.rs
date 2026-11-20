use super::*;
use rmusic_audio::impulse::ImpulseBank;
use std::{future::Future,time::Duration};

pub(super) fn current(state:&DesktopState,generation:u64,profile_id:&str,session:&str)->bool {
    !state.group_output.owned()&&generation==state.playback_generation.load(Ordering::Acquire)&&token(state,profile_id).ok().as_deref()==Some(session)
}
pub(super) async fn run<T>(state:&DesktopState,generation:u64,profile_id:&str,session:&str,operation:impl Future<Output=CommandResult<T>>)->CommandResult<Option<T>> {
    if !current(state,generation,profile_id,session){return Ok(None);}
    tokio::select! {biased;
        _=async {
            let mut interval=tokio::time::interval(Duration::from_millis(50));
            loop{interval.tick().await;if !current(state,generation,profile_id,session){return;}}
        }=>Ok(None),
        result=operation=>if current(state,generation,profile_id,session){result.map(Some)}else{Ok(None)},
    }
}
pub(super) async fn load_impulse(profile:&ConnectionProfile,session:&str,zone:Option<&str>,dsp:&DspConfig)->CommandResult<Option<Arc<ImpulseBank>>> {
    let Some(selection)=dsp.convolution.as_ref().filter(|_|dsp.enabled) else{return Ok(None)};
    let zone=zone.ok_or("卷积播放需要在线音区配置")?;
    rmusic_bridge::impulses::load_selected(profile,session,zone,&selection.response_ids).await.map(Some).map_err(err)
}

#[cfg(test)]
mod tests;
