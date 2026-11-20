//! Permanent, content-addressed artwork, independent of the audio cache switch.
use crate::{
    auth::{hash, Auth},
    error::ApiResult,
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    response::Response,
};
use rusqlite::{params, OptionalExtension};
use std::{
    sync::Arc,
};
/// A fresh provider page may omit song-only artwork. Reuse the canonical row,
/// including a deliberate manual clear, after the exact track identity was imported.
pub fn apply_stored_cover(c: &rusqlite::Connection, track: &mut crate::models::Track) -> anyhow::Result<()> {
    if let Some(cover) = c.query_row("SELECT cover_url FROM tracks WHERE id=?1 AND source=?2",params![track.id,track.source],|r|r.get::<_,Option<String>>(0)).optional()? {
        track.cover_url = cover;
    }
    crate::cached_artwork::restore_source(c,track)?;
    Ok(())
}
pub fn register(c: &rusqlite::Connection, url: &str) -> anyhow::Result<String> {
    if let Some(key) = url.strip_prefix("/api/v1/covers/") {
        anyhow::ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid Core artwork path"
        );
        let sources=c.prepare("SELECT id FROM artwork_sources WHERE cover_id=?1 ORDER BY id LIMIT 2")?.query_map([key],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if sources.len()==1{return Ok(format!("/api/v1/artwork/{}",sources[0]));}
        return Ok(url.into());
    }
    if let Some(key) = url.strip_prefix("/api/v1/artwork/") {
        anyhow::ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid Core artwork path"
        );
        return Ok(url.into());
    }
    let mut parsed = url::Url::parse(url)?;
    if parsed.scheme() == "http"
        && parsed.host_str() == Some("y.gtimg.cn")
        && parsed.port().is_none()
    {
        parsed
            .set_scheme("https")
            .map_err(|_| anyhow::anyhow!("image scheme invalid"))?;
    }
    let url = parsed.as_str();
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || url.len() > 4096
    {
        anyhow::bail!("artwork URL invalid");
    }
    let key = hash(url);
    c.execute(
        "INSERT OR IGNORE INTO artwork_sources(id,url)VALUES(?1,?2)",
        params![key, url],
    )?;
    Ok(format!("/api/v1/artwork/{key}"))
}
pub fn repair_metadata_covers(c:&rusqlite::Connection)->anyhow::Result<()> {
    if c.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key='metadata_cover_repair_v2')",[],|r|r.get::<_,bool>(0))?{return Ok(());}
    let tx=c.unchecked_transaction()?;let mut after=0i64;
    loop {
        let rows=tx.prepare("SELECT rowid,id,json_extract(provider_data,'$.provider_metadata.track.cover_url') FROM tracks WHERE rowid>?1 AND COALESCE(cover_url,'')='' AND json_extract(provider_data,'$.provider_metadata.matched')=1 AND json_extract(provider_data,'$.provider_metadata.track.cover_url') LIKE 'https://%' AND NOT EXISTS(SELECT 1 FROM catalog_profiles p WHERE p.kind='track' AND p.entity_id=tracks.id AND json_type(p.manual,'$.image_url') IS NOT NULL) ORDER BY rowid LIMIT 128")?.query_map([after],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty(){break;}for(row,id,url)in rows {if let Ok(url)=register(&tx,&url){tx.execute("UPDATE tracks SET cover_url=?1 WHERE id=?2",params![url,id])?;}after=row;}
    }
    tx.execute("INSERT INTO settings(key,value)VALUES('metadata_cover_repair_v2','true')",[])?;tx.commit()?;Ok(())
}
pub fn migrate(c: &rusqlite::Connection) -> anyhow::Result<()> {
    if c.query_row(
        "SELECT EXISTS(SELECT 1 FROM settings WHERE key='persistent_artwork_v1')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    let tx = c.unchecked_transaction()?;
    let mut after = 0i64;
    loop {
        let rows=tx.prepare("SELECT rowid,id,cover_url FROM tracks WHERE rowid>?1 AND (cover_url LIKE 'https://%' OR cover_url LIKE 'http://y.gtimg.cn/%') ORDER BY rowid LIMIT 256")?.query_map([after],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            break;
        }
        for (row, id, url) in rows {
            if let Ok(url) = register(&tx, &url) {
                tx.execute(
                    "UPDATE tracks SET cover_url=?1 WHERE id=?2",
                    params![url, id],
                )?;
            }
            after = row;
        }
    }
    tx.execute(
        "INSERT INTO settings(key,value)VALUES('persistent_artwork_v1','true')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}
pub async fn gate(s: &AppState, key: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut gates = s.source_resolutions.lock().await;
    gates.retain(|_, g| g.strong_count() > 0);
    let gate = gates
        .get(key)
        .and_then(std::sync::Weak::upgrade)
        .unwrap_or_default();
    gates.insert(key.into(), Arc::downgrade(&gate));
    gate
}
#[derive(Default, serde::Deserialize)]
pub struct Options { #[serde(default)] pub refresh: bool }
pub async fn get(
    State(s): State<AppState>, a: Auth, Path(id): Path<String>, Query(q): Query<Options>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Response> {
    a.require("read")?;if let Some(p)=crate::accounts::policy(&s,&a).await?{crate::accounts::scope::artwork(&s,&p,&id,false).await?;}
    let file=crate::artwork_recovery::source(&s,&id,q.refresh).await?;
    file.response(&headers,false).await
}
pub fn normalize_entries(c: &rusqlite::Connection, value: &mut serde_json::Value) {
    if let Some(items) = value.as_array_mut() {
        for item in items {
            if let Ok(mut track) = serde_json::from_value::<crate::models::Track>(item.clone()) {
                if apply_stored_cover(c,&mut track).is_ok() {item["cover_url"]=serde_json::json!(track.cover_url);}
            }
            if let Some(url) = item["cover_url"].as_str().filter(|s| !s.is_empty()) {
                if let Ok(url) = register(c, url) { item["cover_url"] = serde_json::json!(url); }
            }
        }
    }
}
