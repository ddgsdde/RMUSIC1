use super::*;
use crate::sequence::Config;
#[cfg(test)]
#[path="sync_tests.rs"]
mod tests;

pub fn default_root(endpoint:&str)->Result<std::path::PathBuf> {
    use sha2::{Digest,Sha256};
    let base=if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).map(|p|p.join("R-Music"))
    }else {
        std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from)
            .or_else(||std::env::var_os("HOME").map(|p|std::path::PathBuf::from(p).join(".local/share"))).map(|p|p.join("rmusic"))
    }.context("cannot locate listening data directory; set --data-dir")?;
    let id:String=Sha256::digest(endpoint.as_bytes())[..16].iter().map(|byte|format!("{byte:02x}")).collect();
    Ok(base.join("bridge").join(id))
}
pub struct Sync {
    store:Arc<Store>,config:Config,owner:Owner,gate:tokio::sync::Mutex<()>,
}
impl Sync {
    pub fn new(store:Arc<Store>,config:Config,owner:Owner)->Arc<Self> {
        Arc::new(Self {store,config,owner,gate:Default::default()})
    }
    pub async fn flush(&self)->Result<usize> {
        let _gate=self.gate.lock().await;
        let store=self.store.clone();let owner=self.owner.clone();
        let (mut rows,groups)=tokio::task::spawn_blocking(move ||{store.drain()?;Ok::<_,anyhow::Error>((store.pending(&owner)?,group::pending(&store,&owner)?))}).await??;
        if rows.is_empty()&&groups.is_empty(){return Ok(0);}
        let mut config=self.config.clone();
        config.profile=self.config.profile.resolve().await?;
        let status=config.status().await?;
        if status["core_id"]!=self.owner.core_id {anyhow::bail!("listening upload core identity mismatch");}
        let group_accepted=group::upload(self.store.clone(),self.owner.clone(),&config,&status,groups).await?;
        let version=if status["capabilities"]["measured_listens_v2"]==true {2}else{u32::from(status["capabilities"]["measured_listens"]==true)};
        let store=self.store.clone();let owner=self.owner.clone();
        rows=tokio::task::spawn_blocking(move ||store.pending_timing(&owner,version)).await??;
        if rows.is_empty(){return Ok(group_accepted);}
        let result=config.upload_listens(&self.owner,&self.store.client_id,rows).await?;
        let accepted=result.accepted.len();let rejected=result.rejected.len();
        let store=self.store.clone();let owner=self.owner.clone();
        tokio::task::spawn_blocking(move ||store.acknowledge(&owner,&result)).await??;
        if rejected>0 {eprintln!("{rejected} listening snapshots rejected; retained in local journal");}
        Ok(accepted+group_accepted)
    }
    pub async fn flush_groups(&self)->Result<usize> {
        let _gate=self.gate.lock().await;let store=self.store.clone();let owner=self.owner.clone();
        let rows=tokio::task::spawn_blocking(move||group::pending(&store,&owner)).await??;
        if rows.is_empty(){return Ok(0);}
        let status=self.config.status().await?;group::upload(self.store.clone(),self.owner.clone(),&self.config,&status,rows).await
    }
    pub fn schedule(self:&Arc<Self>)->Job {
        let sync=self.clone();
        Job(tokio::spawn(async move {
            let mut changes=sync.store.changes.subscribe();
            let mut retry=Duration::from_secs(1);
            loop {
                tokio::select! {_=changes.changed()=>{tokio::time::sleep(Duration::from_secs(20)).await;changes.borrow_and_update();},_=tokio::time::sleep(retry)=>{}}
                match sync.flush().await {
                    Ok(_)=>retry=Duration::from_secs(300),
                    Err(error)=>{eprintln!("Listening upload deferred: {error:#}");retry=Duration::from_secs(60);}
                }
                if let Some(error)=sync.store.take_error(){eprintln!("{error}");}
            }
        }))
    }
}
pub struct Job(tokio::task::JoinHandle<()>);
impl Drop for Job {fn drop(&mut self){self.0.abort();}}
