//! Bounded, demand-driven local presentation data. Shares the existing decoder tap.
use serde_json::{json,Value};
use std::sync::{atomic::{AtomicU64,Ordering},Mutex,OnceLock};
use std::time::Instant;
fn now()->u64 {static START:OnceLock<Instant>=OnceLock::new();START.get_or_init(Instant::now).elapsed().as_millis() as u64}
#[derive(Default)]
pub struct PlaybackObservation {until:AtomicU64,audio_until:AtomicU64,latest:Mutex<Option<(Instant,Value)>>}
impl PlaybackObservation {
    pub fn active(&self)->bool {now()<self.until.load(Ordering::Relaxed)}
    pub fn audio(&self)->bool {now()<self.audio_until.load(Ordering::Relaxed)}
    pub fn read(&self,audio:bool)->Value {
        self.until.fetch_max(now()+600,Ordering::Relaxed);
        if audio {self.audio_until.fetch_max(now()+600,Ordering::Relaxed);}
        let Ok(latest)=self.latest.try_lock() else{return json!({"local":true,"frames":[]});};
        let Some((at,value))=&*latest else{return json!({"local":true,"frames":[]});};
        let mut value=value.clone();let age=at.elapsed().as_secs_f64()*1000.;value["age_ms"]=json!(age);
        if age>250. {value["playing"]=json!(false);value["frames"]=json!([]);}
        if !audio {value["frames"]=json!([]);}
        value
    }
    pub(crate) fn publish(&self,mut value:Value) {
        if let Ok(mut latest)=self.latest.try_lock() {
            // A stalled/rebuffering device cursor must not be extrapolated as audible progress.
            let moving=latest.as_ref().is_some_and(|(_,old)|old["playback_id"]==value["playback_id"]&&value["position"].as_f64().zip(old["position"].as_f64()).is_some_and(|(a,b)|a>b+0.00001));
            value["playing"]=json!(value["playing"]==true&&moving);value["local"]=json!(true);
            *latest=Some((Instant::now(),value));
        }
    }
    pub fn clear(&self){if let Ok(mut value)=self.latest.lock(){*value=None;}}
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn observation_is_bounded_reuses_a_lease_and_freezes_stalled_progress(){
        let source=PlaybackObservation::default();assert!(!source.active());source.read(true);assert!(source.active()&&source.audio());
        source.publish(json!({"playback_id":"p","position":1.,"playing":true,"frames":[1]}));
        source.publish(json!({"playback_id":"p","position":1.05,"playing":true,"frames":[2]}));
        assert_eq!(source.read(true)["playing"],true);assert_eq!(source.read(false)["frames"],json!([]));assert!(source.audio());
        source.publish(json!({"playback_id":"p","position":1.05,"playing":true,"frames":[]}));assert_eq!(source.read(true)["playing"],false);
        source.clear();assert!(source.read(true)["playback_id"].is_null());
    }
}
