//! Anonymous public metadata transport; Core authentication is restricted to the Core route.
use anyhow::{Context, Result};
use rmusic_bridge::connection::ConnectionProfile;
use serde_json::{json, Value};
use std::{sync::LazyLock, time::Duration};
static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .expect("public metadata HTTP configuration")
});
static SLOTS: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(6));
fn image_url(raw: &str) -> bool {
    reqwest::Url::parse(raw).ok().is_some_and(|u| {
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
async fn bounded(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    response.error_for_status_ref()?;
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= max as u64),
        "公开资料超过大小限制"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(bytes.len() + chunk.len() <= max, "公开资料超过大小限制");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn image(profile: &ConnectionProfile, token: &str, id: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(
        id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "图片标识无效"
    );
    let _slot = SLOTS.acquire().await?;
    let http = profile.http()?;
    let url = profile.endpoint(&format!("api/v1/artwork/{id}/public-fetch"))?;
    let hint: Value = http
        .get(url.clone())
        .bearer_auth(token)
        .timeout(Duration::from_secs(8))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let source = hint["url"].as_str().context("缺少公开图片地址")?;
    anyhow::ensure!(image_url(source), "不支持此匿名图片来源");
    let response = HTTP.get(source).send().await?;
    anyhow::ensure!(
        response
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("image/")),
        "公开图片响应无效"
    );
    let bytes = bounded(response, 20 * 1024 * 1024).await?;
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if hint["cached"] != true || hint["input_sha"].as_str() != Some(&digest) {
        // Do not declare success until Core has retained the missing original.
        let result: Value = http
            .put(url)
            .bearer_auth(token)
            .header(
                "x-rmusic-source-revision",
                hint["revision"].as_str().context("缺少图片版本")?,
            )
            .header(
                "x-rmusic-cached-content",
                hint["cover_id"].as_str().unwrap_or(""),
            )
            .header("Content-Type", "application/octet-stream")
            .body(bytes.clone())
            .timeout(Duration::from_secs(20))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        anyhow::ensure!(result["cached"] == true, "Core 图片回填未完成");
    }
    Ok(bytes)
}
fn catalog_path(body: &Value) -> Option<String> {
    if body["method"].as_str().unwrap_or("GET") != "GET" {
        return None;
    }
    let path = body["path"].as_str()?;
    let route = path.split('?').next()?;
    let parts: Vec<_> = route.split('/').collect();
    (parts.len() == 5
        && parts[1] == "library"
        && parts[2] == "catalog"
        && ["artist", "album"].contains(&parts[3])
        && !parts[4].is_empty()
        && parts[4].len() <= 768
        && !path.contains("remote=false"))
    .then(|| route.to_owned())
}
async fn prepare(api: &crate::api::Api, path: &str, refresh: bool) -> Result<()> {
    let route = format!("{path}/public-fetch");
    let hint = api
        .request(json!({"path":format!("{route}?refresh={refresh}")}))
        .await?;
    if hint["fresh"] == true {
        return Ok(());
    }
    let req = &hint["request"];
    anyhow::ensure!(
        req["version"] == 1
            && req["method"] == "POST"
            && req["url"] == "https://u.y.qq.com/cgi-bin/musicu.fcg"
            && req["json"]["comm"]["uin"] == 0
            && req.get("headers").is_none()
            && req.get("cookies").is_none(),
        "匿名资料请求不符合约定"
    );
    let _slot = SLOTS.acquire().await?;
    let bytes = bounded(
        HTTP.post("https://u.y.qq.com/cgi-bin/musicu.fcg")
            .json(&req["json"])
            .send()
            .await?,
        1024 * 1024,
    )
    .await?;
    let response: Value = serde_json::from_slice(&bytes)?;
    let result=api.request(json!({"path":route,"method":"POST","json":{"id":hint["id"],"epoch":hint["epoch"],"revision":hint["revision"],"expires_at":hint["expires_at"],"response":response}})).await?;
    anyhow::ensure!(result["cache"].is_object(), "Core 资料回填未完成");
    Ok(())
}
pub async fn request(api: crate::api::Api, mut body: Value) -> Result<Value> {
    if let Some(path) = catalog_path(&body) {
        let original = body["path"].as_str().unwrap().to_owned();
        if prepare(&api, &path, original.contains("refresh=true"))
            .await
            .is_ok()
        {
            body["path"] = json!(format!(
                "{original}{}client_metadata=true",
                if original.contains('?') { "&" } else { "?" }
            ));
        }
    }
    api.request(body).await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_fetch_is_limited_to_entity_pages_and_public_qq_images() {
        assert!(catalog_path(&json!({"path":"/library/catalog/artist/123?sort=hot"})).is_some());
        for path in [
            "/library/catalog/artist/123/albums",
            "/library/catalog/album/123/profile",
            "/library/catalog/album/123?remote=false",
            "/plugins/qqmusic/login",
        ] {
            assert!(catalog_path(&json!({"path":path})).is_none());
        }
        assert!(
            catalog_path(&json!({"method":"PATCH","path":"/library/catalog/album/123"})).is_none()
        );
        assert!(image_url(
            "https://y.gtimg.cn/music/photo_new/T002R500x500M000Abc.jpg"
        ));
        assert!(!image_url(
            "https://user:pass@y.gtimg.cn/music/photo_new/a.jpg"
        ));
        assert!(!image_url("https://y.gtimg.cn.evil/music/photo_new/a.jpg"));
    }
}
