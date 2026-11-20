//! Complete only the metadata trailer of a naturally heard DSD file, without blocking output.
use crate::cache::CacheStore;
use anyhow::{bail,Context,Result};
use futures_util::StreamExt;
use rmusic_bridge::connection::ConnectionProfile;
use std::sync::{Arc,Mutex};

#[cfg(any(feature="tauri-app",test))]
#[derive(Default)]
pub(crate) struct MetadataJobs {pub generation:u64,jobs:tokio::task::JoinSet<()>}
#[cfg(any(feature="tauri-app",test))]
pub(crate) type MetadataTasks=Arc<Mutex<MetadataJobs>>;
#[cfg(any(feature="tauri-app",test))]
pub(crate) fn cancel(tasks:&MetadataTasks){if let Ok(mut tasks)=tasks.lock(){tasks.generation=tasks.generation.wrapping_add(1);tasks.jobs.abort_all();}}
#[cfg(any(feature="tauri-app",test))]
pub(crate) fn spawn(tasks:&MetadataTasks,generation:u64,job:impl std::future::Future<Output=()>+Send+'static) {
    if let Ok(mut tasks)=tasks.lock(){
        if tasks.generation!=generation{return;}
        while tasks.jobs.try_join_next().is_some(){}
        tasks.jobs.spawn(job);
    }
}

pub(crate) async fn complete_metadata(cache:Arc<Mutex<CacheStore>>,id:String,profile:ConnectionProfile,url:String)->Result<bool> {
    let plan={
        let cache=cache.lock().map_err(|_|anyhow::anyhow!("cache lock poisoned"))?;
        if cache.manifest.entries.get(&id).is_some_and(|entry|entry.complete){return Ok(true);}
        cache.dsd_metadata_tail(&id)?
    };
    let Some(plan)=plan else{return Ok(false);};
    profile.validate_media_url(&url)?;let client=profile.http()?;
    for gap in plan.gaps {
        let response=client.get(&url).header("range",format!("bytes={}-{}",gap.start,gap.end-1)).header("if-range",&plan.etag)
            .send().await.map_err(|error|anyhow::Error::new(error.without_url()))?;
        let expected=format!("bytes {}-{}/{}",gap.start,gap.end-1,plan.total);
        if response.status()!=reqwest::StatusCode::PARTIAL_CONTENT
            ||response.headers().get("etag").and_then(|value|value.to_str().ok())!=Some(plan.etag.as_str())
            ||response.headers().get("content-range").and_then(|value|value.to_str().ok())!=Some(expected.as_str())
            ||response.content_length()!=Some(gap.end-gap.start)
            ||response.headers().get("content-encoding").is_some_and(|value|value!="identity") {
            bail!("DSD 元数据资源或范围发生变化，保留部分缓存");
        }
        let mut cursor=gap.start;let mut body=response.bytes_stream();
        while let Some(bytes)=body.next().await {
            let bytes=bytes.map_err(|error|anyhow::Error::new(error.without_url()))?;
            let end=cursor.checked_add(bytes.len() as u64).context("metadata byte range overflow")?;
            if end>gap.end{bail!("DSD metadata body exceeds its verified range");}
            cache.lock().map_err(|_|anyhow::anyhow!("cache lock poisoned"))?.write_range(&id,cursor,&bytes)?;cursor=end;
        }
        if cursor!=gap.end{bail!("DSD metadata body was truncated");}
    }
    let complete=cache.lock().map_err(|_|anyhow::anyhow!("cache lock poisoned"))?.manifest.entries.get(&id).is_some_and(|entry|entry.complete);
    Ok(complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dsd_metadata_jobs_cancel_and_reject_obsolete_work() {
        let tasks=MetadataTasks::default();let generation=tasks.lock().unwrap().generation;
        let (started,ready)=tokio::sync::oneshot::channel();let (lifetime,dropped)=tokio::sync::oneshot::channel::<()>();
        spawn(&tasks,generation,async move{let _lifetime=lifetime;let _=started.send(());std::future::pending::<()>().await;});
        ready.await.unwrap();cancel(&tasks);assert!(dropped.await.is_err());
        let (stale,received)=tokio::sync::oneshot::channel();
        spawn(&tasks,generation,async move{let _=stale.send(());});assert!(received.await.is_err());
        let generation=tasks.lock().unwrap().generation;let (fresh,received)=tokio::sync::oneshot::channel();
        spawn(&tasks,generation,async move{let _=fresh.send(());});received.await.unwrap();
    }
}
