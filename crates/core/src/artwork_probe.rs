//! Provider-confirmed absence is not a transport failure. Keep provenance without
//! deleting any pixels; a manual refresh or a later successful fetch can recover it.
use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};
use rusqlite::params;
use serde_json::{json, Value};

pub fn unavailable() -> ApiError {
    ApiError(
        axum::http::StatusCode::NOT_FOUND,
        "artwork_not_provided".into(),
        "平台尚未提供这张图片".into(),
    )
}
pub async fn known_absent(s: &AppState, id: &str) -> ApiResult<bool> {
    let id = id.to_owned();
    Ok(s.db.read(move |c| Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM artwork_absences p JOIN artwork_sources a ON a.id=p.source_id AND a.url=p.url WHERE p.source_id=?1 AND p.checked_at>unixepoch()-604800)",
        [id], |r| r.get(0),
    )?)).await?)
}
fn allowed(manifest: &Value, raw: &str) -> bool {
    raw.len() <= 4096
        && url::Url::parse(raw).ok().is_some_and(|u| {
            u.scheme() == "https"
                && u.port().is_none()
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
                && manifest["artwork_hosts"]
                    .as_array()
                    .is_some_and(|hosts| hosts.iter().any(|h| h.as_str() == u.host_str()))
        })
}
fn valid_reply(value: &Value, provider: &str, url: &str) -> bool {
    value["matched"] == true
        && value["source"] == provider
        && value["url"] == url
        && matches!(value["kind"].as_str(), Some("artist" | "album"))
        && value["id"].as_str().is_some_and(|id| {
            !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
        })
        && value["name"]
            .as_str()
            .is_some_and(|name| name.len() <= 512 && !name.chars().any(char::is_control))
}
pub async fn resolve(s: &AppState, id: &str, url: &str) -> ApiResult<Option<String>> {
    let (handle, original) = (format!("/api/v1/artwork/{id}"), url.to_owned());
    let manual = s.db.read(move |c| Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM catalog_profiles WHERE json_extract(manual,'$.image_url') IN (?1,?2))",
        params![handle, original], |r| r.get::<_, bool>(0),
    )?)).await?;
    if manual { return Ok(None); }
    for provider in crate::connectors::providers(s, "artwork_probe_v1")
        .await
        .into_iter()
        .take(16)
    {
        let Ok(manifest) = crate::connectors::manifest(s, &provider).await else {
            continue;
        };
        if !allowed(&manifest, url) {
            continue;
        }
        let Ok(Ok(value)) = tokio::time::timeout(
            std::time::Duration::from_secs(12),
            s.plugin
                .call(&provider, "artwork_probe", json!({"url":url})),
        )
        .await
        else {
            continue;
        };
        if !valid_reply(&value, &provider, url) {
            continue;
        }
        if value["image_status"] == "unavailable" && value["image_url"].is_null() {
            let (id, url) = (id.to_owned(), url.to_owned());
            let recorded=s.db.run(move |c| {
                let tx=c.transaction()?;
                // CAS the source: a concurrent metadata update must not mark a new URL absent.
                let recorded=tx.execute("INSERT INTO artwork_absences(source_id,url,provider,kind,entity_id,name,checked_at) SELECT id,url,?1,?2,?3,?4,unixepoch() FROM artwork_sources WHERE id=?5 AND url=?6 ON CONFLICT(source_id) DO UPDATE SET url=excluded.url,provider=excluded.provider,kind=excluded.kind,entity_id=excluded.entity_id,name=excluded.name,checked_at=excluded.checked_at",params![provider,value["kind"].as_str(),value["id"].as_str(),value["name"].as_str(),id,url])?>0;
                tx.execute("DELETE FROM artwork_failures WHERE source_id=?1 AND EXISTS(SELECT 1 FROM artwork_absences WHERE source_id=?1 AND url=?2)",params![id,url])?;
                tx.execute("INSERT INTO artwork_health(source_id,state,error,checked_at) SELECT id,'unavailable',NULL,unixepoch() FROM artwork_sources WHERE id=?1 AND url=?2 AND cover_id IS NULL ON CONFLICT(source_id) DO UPDATE SET state=excluded.state,error=NULL,checked_at=excluded.checked_at",params![id,url])?;
                tx.commit()?;Ok(recorded)
            }).await?;
            if recorded {
                return Err(unavailable());
            }
            return Ok(None);
        }
        if let Some(next) = value["image_url"]
            .as_str()
            .filter(|next| *next != url && allowed(&manifest, next))
        {
            // Only the owning provider can replace its exact source URL.
            let (key, old, new) = (id.to_owned(), url.to_owned(), next.to_owned());
            let changed =
                s.db.run(move |c| {
                    Ok(c.execute(
                        "UPDATE artwork_sources SET url=?1 WHERE id=?2 AND url=?3",
                        params![new, key, old],
                    )? > 0)
                })
                .await?;
            if changed {
                return Ok(Some(next.into()));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_probe_requires_declared_host_and_exact_source_binding() {
        let manifest=json!({"artwork_hosts":["images.example"]});
        assert!(allowed(&manifest,"https://images.example/photo.jpg"));
        for url in ["http://images.example/photo.jpg","https://other.example/photo.jpg","https://user@images.example/photo.jpg"] {
            assert!(!allowed(&manifest,url));
        }
        let mut value=json!({"matched":true,"source":"a","url":"https://images.example/a","kind":"artist","id":"Exact123","name":"Artist"});
        assert!(valid_reply(&value,"a","https://images.example/a"));
        assert!(!valid_reply(&value,"b","https://images.example/a"));
        assert!(!valid_reply(&value,"a","https://images.example/b"));
        value["id"]=json!("");assert!(!valid_reply(&value,"a","https://images.example/a"));
    }
}
