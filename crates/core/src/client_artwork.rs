//! Public CDN hints and deduplicated cache fill. Never accepts a new URL from the client.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use rusqlite::OptionalExtension;
use serde_json::json;

pub(crate) fn public_url(raw: &str) -> bool {
    url::Url::parse(raw).ok().is_some_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("y.gtimg.cn")
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
            && u.path().starts_with("/music/photo_new/")
            && u.query().is_none()
            && u.fragment().is_none()
    })
}
async fn source(s: &AppState, id: &str) -> ApiResult<(String, Option<String>)> {
    if !crate::artwork_files::valid_id(id) {
        return Err(ApiError::missing());
    }
    let id = id.to_owned();
    let row =
        s.db.read(move |c| {
            Ok(c.query_row(
                "SELECT url,cover_id FROM artwork_sources WHERE id=?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?)
        })
        .await?
        .ok_or_else(ApiError::missing)?;
    if !public_url(&row.0) {
        return Err(ApiError::missing());
    }
    Ok(row)
}
async fn cached(s: &AppState, id: Option<&str>) -> ApiResult<bool> {
    let Some(id) = id else {
        return Ok(false);
    };
    match crate::artwork_files::open(s, id).await {
        Ok(_) => Ok(true),
        Err(e) if e.1 == "not_found" || e.1 == "artwork_corrupt" => Ok(false),
        Err(e) => Err(e),
    }
}
pub async fn describe(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    a.require("library")?;
    let (url, cover) = source(&s, &id).await?;
    let has_cached = cached(&s, cover.as_deref()).await?;
    if !has_cached && crate::artwork_probe::known_absent(&s, &id).await? {
        return Err(crate::artwork_probe::unavailable());
    }
    let key = id.clone();
    let current = cover.clone();
    let input =
        s.db.read(move |c| {
            Ok(c.query_row(
                "SELECT input_sha FROM artwork_client_inputs WHERE source_id=?1 AND cover_id=?2",
                rusqlite::params![key, current],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
        })
        .await?;
    Ok(Json(
        json!({"url":url,"revision":crate::auth::hash(&url),"cover_id":cover,"input_sha":input.or(cover.clone()),"cached":has_cached}),
    ))
}
static SLOTS: std::sync::LazyLock<tokio::sync::Semaphore> =
    std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(2));
pub async fn complete(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    headers: HeaderMap,
    bytes: Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    a.require("library")?;
    let _slot = SLOTS
        .try_acquire()
        .map_err(|_| ApiError::bad("图片回填繁忙，请回退 Core"))?;
    let gate = crate::media_assets::gate(&s, &format!("artwork:{id}")).await;
    let _guard = gate.lock().await;
    let (url, cover) = source(&s, &id).await?;
    if headers
        .get("x-rmusic-source-revision")
        .and_then(|h| h.to_str().ok())
        != Some(crate::auth::hash(&url).as_str())
    {
        return Err(ApiError::bad("图片来源已变化"));
    }
    let exists = cached(&s, cover.as_deref()).await?;
    let input = crate::artwork_files::content_id(&bytes);
    if exists {
        let expected = headers
            .get("x-rmusic-cached-content")
            .and_then(|h| h.to_str().ok());
        if expected.is_none() {
            return Ok(Json(json!({"saved":false,"cached":true})));
        }
        if expected != cover.as_deref() {
            return Err(ApiError(
                axum::http::StatusCode::CONFLICT,
                "artwork_version_changed".into(),
                "Core 图片已更新，请重新读取".into(),
            ));
        }
        let key = id.clone();
        let current = cover.clone();
        let hash = input.clone();
        let same=s.db.read(move|c|Ok(current.as_deref()==Some(&hash)||c.query_row("SELECT EXISTS(SELECT 1 FROM artwork_client_inputs WHERE source_id=?1 AND cover_id=?2 AND input_sha=?3)",rusqlite::params![key,current,hash],|r|r.get::<_,bool>(0))?)).await?;
        if same {
            return Ok(Json(json!({"saved":false,"cached":true})));
        }
    }
    let prepared = crate::artwork_images::prepare(bytes.to_vec()).await?;
    let cover = crate::artwork_files::save(&s, &prepared).await?;
    // Recovery and client fill serialize on the same source gate; file content is deduplicated too.
    crate::artwork_recovery::publish(&s, &id, &cover).await?;
    let (key, file) = (id.clone(), cover.clone());
    s.db.run(move|c|{c.execute("INSERT INTO artwork_client_inputs(source_id,cover_id,input_sha)VALUES(?1,?2,?3) ON CONFLICT(source_id)DO UPDATE SET cover_id=excluded.cover_id,input_sha=excluded.input_sha",rusqlite::params![key,file,input])?;Ok(())}).await?;
    let _ = crate::artwork_maintenance::health(&s, &id, "ready", None).await;
    Ok(Json(json!({"saved":true,"cached":true})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_image_scope_excludes_accounts_and_other_hosts() {
        assert!(public_url(
            "https://y.gtimg.cn/music/photo_new/T002R500x500M000Abc123.jpg"
        ));
        for bad in [
            "https://y.gtimg.cn.evil/music/photo_new/a.jpg",
            "https://user:secret@y.gtimg.cn/music/photo_new/a.jpg",
            "http://y.gtimg.cn/music/photo_new/a.jpg",
            "https://y.gtimg.cn/music/photo_new/a.jpg?token=secret",
            "https://u.y.qq.com/a",
            "https://y.gtimg.cn:8443/music/photo_new/a.jpg",
        ] {
            assert!(!public_url(bad));
        }
    }
}
