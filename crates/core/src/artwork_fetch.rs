//! Bounded public image transport. Never sends Core or provider credentials.
use crate::error::{ApiError, ApiResult};
use std::net::IpAddr;
static COOL: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::LazyLock::new(Default::default);
pub fn retry_seconds(url: &str) -> u64 {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .and_then(|h| {
            COOL.lock().unwrap().get(&h).map(|at| {
                at.saturating_duration_since(std::time::Instant::now())
                    .as_secs()
                    + 1
            })
        })
        .unwrap_or(0)
}
const MAX: usize = crate::artwork_images::INPUT_LIMIT;
pub(crate) fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !v.is_private()
                && !v.is_loopback()
                && !v.is_link_local()
                && !v.is_multicast()
                && !v.is_unspecified()
                && !v.is_broadcast()
                && v.octets()[0] != 0
                && v.octets()[0] < 224
                && !(v.octets()[0] == 100 && (64..=127).contains(&v.octets()[1]))
        }
        IpAddr::V6(v) => v.to_ipv4_mapped().map(|v| public(v.into())).unwrap_or(
            !v.is_loopback()
                && !v.is_unspecified()
                && !v.is_multicast()
                && (v.segments()[0] & 0xfe00) != 0xfc00
                && (v.segments()[0] & 0xffc0) != 0xfe80,
        ),
    }
}
pub async fn fetch(url: &str) -> ApiResult<Vec<u8>> {
    tokio::time::timeout(std::time::Duration::from_secs(18), fetch_inner(url))
        .await
        .map_err(|_| {
            ApiError(
                axum::http::StatusCode::GATEWAY_TIMEOUT,
                "artwork_timeout".into(),
                "图片源读取超时，请稍后重试".into(),
            )
        })?
}
async fn fetch_inner(url: &str) -> ApiResult<Vec<u8>> {
    if retry_seconds(url) > 0 {
        return Err(ApiError(
            axum::http::StatusCode::BAD_GATEWAY,
            "artwork_rate_limited".into(),
            "图片来源限流中".into(),
        ));
    }
    let mut url = url::Url::parse(url).map_err(|_| ApiError::bad("图片地址无效"))?;
    for _ in 0..4 {
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
            return Err(ApiError::bad("图片必须使用公开 HTTPS 地址"));
        }
        let host = url
            .host_str()
            .ok_or_else(|| ApiError::bad("图片地址无效"))?;
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(anyhow::Error::from)?
            .collect::<Vec<_>>();
        if addresses.is_empty() || addresses.iter().any(|a| !public(a.ip())) {
            return Err(ApiError::bad("图片地址不能指向内部网络"));
        }
        let client = reqwest::Client::builder()
            .resolve_to_addrs(host, &addresses)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(4))
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .map_err(anyhow::Error::from)?;
        let mut response = client
            .get(url.clone())
            .send()
            .await
            .map_err(anyhow::Error::from)?;
        if response.status().is_redirection() {
            let next = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| ApiError::bad("图片重定向无效"))?;
            url = url
                .join(next)
                .map_err(|_| ApiError::bad("图片重定向无效"))?;
            continue;
        }
        if !response.status().is_success() {
            let status = response.status();
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let seconds = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| {
                        v.parse::<u64>().ok().or_else(|| {
                            chrono::DateTime::parse_from_rfc2822(v).ok().map(|at| {
                                (at.timestamp() - chrono::Utc::now().timestamp()).max(1) as u64
                            })
                        })
                    })
                    .unwrap_or(300)
                    .clamp(1, 86400);
                let mut cooldowns = COOL.lock().unwrap();
                cooldowns.retain(|_, at| *at > std::time::Instant::now());
                if cooldowns.len() >= 128 {
                    cooldowns.clear();
                }
                cooldowns.insert(
                    host.to_owned(),
                    std::time::Instant::now() + std::time::Duration::from_secs(seconds),
                );
            }
            let code = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                "artwork_rate_limited"
            } else if matches!(status.as_u16(), 401 | 403 | 404 | 410) {
                "artwork_source_expired"
            } else {
                "artwork_source_unavailable"
            };
            return Err(ApiError(
                axum::http::StatusCode::BAD_GATEWAY,
                code.into(),
                "图片来源暂不可用，请稍后刷新".into(),
            ));
        }
        if response.content_length().is_some_and(|n| n > MAX as u64) {
            return Err(ApiError::bad("图片超过20 MiB"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(anyhow::Error::from)? {
            if bytes.len() + chunk.len() > MAX {
                return Err(ApiError::bad("图片超过20 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if crate::artwork_files::mime(&bytes).is_none() {
            return Err(ApiError::bad("图片格式不支持"));
        }
        return Ok(bytes);
    }
    Err(ApiError::bad("图片重定向过多"))
}
