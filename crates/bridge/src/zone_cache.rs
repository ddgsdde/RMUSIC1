//! Session-owned queue pages. Small state reads reuse an unchanged ordered queue.
use crate::connection::ConnectionProfile;
use anyhow::{Context, Result};
use futures_util::{stream, StreamExt, TryStreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct ZoneCache(tokio::sync::Mutex<HashMap<String, Cached>>);
#[derive(Clone)]
struct Cached {
    revision: u64,
    epoch: String,
    items: Vec<Value>,
}
async fn read(
    http: &reqwest::Client,
    profile: &ConnectionProfile,
    token: &str,
    path: &str,
) -> Result<Value> {
    let mut response = http
        .get(profile.endpoint(&format!("api/v1{path}"))?)
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
            "音区响应超过 4 MiB"
        );
        bytes.extend(chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Query, routing::get, Json};
    use std::sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    };
    #[tokio::test]
    async fn pages_are_reused_until_revision_changes_and_keep_duplicate_tracks() {
        let revision = Arc::new(AtomicU64::new(1));
        let reads = Arc::new(AtomicUsize::new(0));
        let epoch = Arc::new(AtomicU64::new(1));
        let state_epoch = epoch.clone(); let page_epoch = epoch.clone();
        let r = revision.clone();
        let q = reads.clone();
        let version = revision.clone();
        let router=axum::Router::new().route("/api/v1/zones",get(move ||{let r=r.clone();let epoch=state_epoch.clone();async move {
            Json(json!({"queue_view":"versioned","queue_epoch":epoch.load(Ordering::SeqCst).to_string(),"items":[{"id":"room","queue_length":501,"queue_revision":r.load(Ordering::SeqCst),"queue":[]}]}))
        }})).route("/api/v1/zones/room/queue",get(move |Query(params):Query<HashMap<String,String>>|{
            let q=q.clone();let version=version.clone();let epoch=page_epoch.clone();async move {
                q.fetch_add(1,Ordering::SeqCst);let offset:usize=params["offset"].parse().unwrap();let rev=version.load(Ordering::SeqCst);
                Json(json!({"epoch":epoch.load(Ordering::SeqCst).to_string(),"revision":rev,"offset":offset,"total":501,"items":(offset..(offset+500).min(501)).map(|i|json!({"id":"duplicate","at":i})).collect::<Vec<_>>()}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let profile: ConnectionProfile = serde_json::from_value(
            json!({"id":"cache-test","name":"QA","address":format!("http://{address}")}),
        )
        .unwrap();
        let cache = ZoneCache::default();
        let http = profile.http().unwrap();
        let first = cache.load(&profile, &http, "test-token").await.unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        assert_eq!(first["items"][0]["queue"].as_array().unwrap().len(), 501);
        assert_eq!(first["items"][0]["queue"][500]["at"], 500);
        assert_eq!(
            cache.load(&profile, &http, "test-token").await.unwrap(),
            first
        );
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        revision.store(2, Ordering::SeqCst);
        cache.load(&profile, &http, "test-token").await.unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 4);
        epoch.store(2, Ordering::SeqCst);
        cache.load(&profile, &http, "test-token").await.unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 6);
        server.abort();
    }
}
impl ZoneCache {
    pub async fn load(
        &self,
        profile: &ConnectionProfile,
        http: &reqwest::Client,
        token: &str,
    ) -> Result<Value> {
        let mut cache = self.0.lock().await;
        for attempt in 0..2 {
            let mut state = read(http, profile, token, "/zones?view=state").await?;
            // Existing Core releases return their complete snapshot for this query.
            if state["queue_view"] != "versioned" {
                return Ok(state);
            }
            let epoch = state["queue_epoch"].as_str().unwrap_or("").to_owned();
            let rows = state["items"]
                .as_array_mut()
                .context("Core 未返回音区状态")?;
            let changed = rows
                .iter()
                .filter_map(|zone| {
                    let id = zone["id"].as_str()?.to_owned();
                    let revision = zone["queue_revision"].as_u64()?;
                    let total = zone["queue_length"].as_u64()? as usize;
                    (!cache
                        .get(&id)
                        .is_some_and(|v| v.epoch == epoch && v.revision == revision && v.items.len() == total))
                    .then_some((id, revision, total))
                })
                .collect::<Vec<_>>();
            let result: Result<Vec<_>> = stream::iter(changed)
                .map(|(id, revision, total)| { let epoch = epoch.clone(); async move {
                    let mut items = Vec::new();
                    while items.len() < total {
                        let encoded: String =
                            url::form_urlencoded::byte_serialize(id.as_bytes()).collect();
                        let page = read(
                            http,
                            profile,
                            token,
                            &format!(
                                "/zones/{encoded}/queue?epoch={epoch}&revision={revision}&offset={}&limit=500",
                                items.len()
                            ),
                        )
                        .await?;
                        let part = page["items"].as_array().context("Core 未返回队列分页")?;
                        anyhow::ensure!(
                            page["epoch"].as_str().unwrap_or("") == epoch && page["revision"].as_u64() == Some(revision)
                                && page["total"].as_u64() == Some(total as u64)
                                && page["offset"].as_u64() == Some(items.len() as u64)
                                && !part.is_empty()
                                && items.len() + part.len() <= total,
                            "Core 队列分页发生变化"
                        );
                        items.extend(part.iter().cloned());
                    }
                    Ok((id, Cached { revision, epoch, items }))
                }})
                .buffer_unordered(3)
                .try_collect()
                .await;
            match result {
                Err(error)
                    if attempt == 0
                        && error
                            .downcast_ref::<reqwest::Error>()
                            .and_then(|e| e.status())
                            == Some(reqwest::StatusCode::CONFLICT) =>
                {
                    continue
                }
                Err(error) => return Err(error),
                Ok(loaded) => {
                    for (id, entry) in loaded {
                        cache.insert(id, entry);
                    }
                }
            }
            let live = rows
                .iter()
                .filter_map(|z| z["id"].as_str())
                .map(str::to_owned)
                .collect::<HashSet<_>>();
            cache.retain(|id, _| live.contains(id));
            for zone in rows {
                let id = zone["id"].as_str().context("音区缺少标识")?;
                let entry = cache.get(id).context("音区缺少队列版本")?;
                zone["queue"] = json!(entry.items);
            }
            return Ok(state);
        }
        unreachable!()
    }
}
