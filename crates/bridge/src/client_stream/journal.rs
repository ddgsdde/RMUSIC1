use crate::listens::{Owner, Recorder, Store};
use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

/// Local playback uses the same account-scoped durable records as other native outputs.
pub struct Journal {
    store: Arc<Store>,
    owner: Owner,
    current: Option<(String, Recorder)>,
}
impl Journal {
    pub fn new(store: Arc<Store>, owner: Owner) -> Self {
        Self { store, owner, current: None }
    }
    pub(super) fn sync(&self, profile: crate::connection::ConnectionProfile, token: String, endpoint: String) -> Result<crate::listens::sync::Job> {
        Ok(crate::listens::sync::Sync::new(self.store.clone(), crate::sequence::Config::new(profile,token,endpoint)?,self.owner.clone()).schedule())
    }
    pub(super) fn command(&mut self, command: &Value, state: &Value) -> Result<()> {
        let kind = command["type"].as_str().unwrap_or("");
        if kind == "play" {
            let instance = command["playback_id"].as_str().unwrap_or("");
            if !self.current.as_ref().is_some_and(|(id, _)| id == instance) {
                self.finish("replaced")?;
                let track = &command["track"];
                self.current = Some((instance.into(), Recorder::new(
                    self.store.clone(), self.owner.clone(),
                    track["id"].as_str().unwrap_or("").into(),
                    track["title"].as_str().unwrap_or("").into(),
                    track["artist"].as_str().unwrap_or("").into(),
                    track["duration"].as_f64().unwrap_or(0.),
                )));
            }
            if let Some((_, recorder)) = &mut self.current {
                recorder.sample(command["position"].as_f64().unwrap_or(0.), false, true)?;
            }
        } else if matches!(kind, "seek" | "pause" | "stop") {
            self.report(state)?;
            if let Some((_, recorder)) = &mut self.current {
                recorder.sample(command["position"].as_f64().or_else(||state["position"].as_f64()).unwrap_or(0.), false, kind == "seek")?;
                recorder.checkpoint()?;
            }
            if kind == "stop" { self.finish("stopped")?; }
        }
        Ok(())
    }
    pub(super) fn report(&mut self, report: &Value) -> Result<()> {
        if let Some((_, recorder)) = &mut self.current {
            if let Some(position) = report["position"].as_f64() {
                recorder.sample(position, report["status"] == "playing", false)?;
            }
        }
        if report["status"] == "ended" { self.finish("ended")?; }
        Ok(())
    }
    fn finish(&mut self, reason: &str) -> Result<()> {
        if let Some((_, mut recorder)) = self.current.take() { recorder.finish(reason)?; }
        Ok(())
    }
}
impl Drop for Journal {
    fn drop(&mut self) {
        let _ = self.finish("interrupted");
        let _ = self.store.drain();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn retries_and_seeks_keep_one_record_and_next_song_has_its_own_record() {
        let root=tempfile::tempdir().unwrap();let store=Store::open(root.path()).unwrap();
        let owner=Owner{core_id:"core".into(),user_id:"account".into()};
        let mut journal=Journal::new(store.clone(),owner.clone());
        let play=json!({"type":"play","playback_id":"one","position":0.,"track":{"id":"song","title":"曲目","artist":"作者","duration":100.}});
        journal.command(&play,&Value::Null).unwrap();
        journal.report(&json!({"status":"playing","position":8.})).unwrap();
        journal.command(&json!({"type":"seek","position":40.}),&json!({"status":"paused","position":8.})).unwrap();
        let mut retry=play.clone();retry["position"]=json!(40.);
        journal.command(&retry,&Value::Null).unwrap();
        journal.report(&json!({"status":"ended","position":100.})).unwrap();
        let mut next=play;next["playback_id"]=json!("two");
        journal.command(&next,&Value::Null).unwrap();
        journal.command(&json!({"type":"stop"}),&json!({"status":"paused","position":0.})).unwrap();
        store.drain().unwrap();
        let rows=store.pending(&owner).unwrap();
        assert_eq!(rows.len(),2);
        assert!(rows.iter().any(|r|r.snapshot.reason=="ended"&&r.snapshot.position==100.));
        assert!(rows.iter().any(|r|r.snapshot.reason=="stopped"));
        assert!(store.pending(&Owner{core_id:"core".into(),user_id:"other".into()}).unwrap().is_empty());
    }
}
