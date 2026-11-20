//! Optional OpenAI-compatible ranking, and catalogue search suggestions.
mod config;
mod ranking;
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{extract::State, http::StatusCode, Json};
pub use ranking::recommend;
pub(crate) use ranking::{rerank,search_hints,SongHint};
pub(crate) async fn ranking_available(s:&AppState)->bool {config::load(s).await.is_ok_and(|(c,_)|c.enabled)}
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

pub struct Runtime {
    pub gate: Arc<tokio::sync::Semaphore>,
    pub configuration: tokio::sync::Mutex<()>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            gate: Arc::new(tokio::sync::Semaphore::new(2)),
            configuration: tokio::sync::Mutex::new(()),
        }
    }
}
fn unavailable(message: &str) -> ApiError {
    ApiError(
        StatusCode::BAD_GATEWAY,
        "llm_unavailable".into(),
        message.into(),
    )
}
pub async fn settings(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let _lock = s.llm.configuration.lock().await;
    let (v, r) = config::load(&s).await?;
    Ok(Json(v.public(r)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configure {
    expected_revision: String,
    enabled: bool,
    base_url: String,
    model: String,
    key: Option<String>,
}
pub async fn configure(
    State(s): State<AppState>,
    a: Auth,
    Json(input): Json<Configure>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let work = tokio::spawn(async move {
        let _lock = s.llm.configuration.try_lock().map_err(|_| {
            ApiError(
                StatusCode::CONFLICT,
                "llm_configuration_busy".into(),
                "配置正在保存，请稍后重新读取".into(),
            )
        })?;
        let (mut value, revision) = config::load(&s).await?;
        if revision != input.expected_revision {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "llm_configuration_conflict".into(),
                "LLM 配置已改变，请重新读取，草稿可保留".into(),
            ));
        }
        value.enabled = input.enabled;
        let base: String = input.base_url.trim().trim_end_matches('/').into();
        if value.base_url != base && input.key.is_none() {
            value.key.clear();
        }
        value.base_url = base;
        value.model = input.model.trim().into();
        if let Some(key) = input.key {
            value.key = key;
        }
        value.validate()?;
        let revision = config::save(&s, &value).await?;
        Ok::<Value, ApiError>(value.public(revision))
    });
    Ok(Json(work.await.map_err(anyhow::Error::from)??))
}
async fn request(
    s: &AppState,
    value: &config::Configuration,
    path: &str,
    body: Option<Value>,
) -> ApiResult<Value> {
    request_with_timeout(s,value,path,body,Duration::from_secs(40)).await
}
async fn request_with_timeout(
    s:&AppState, value:&config::Configuration, path:&str, body:Option<Value>, timeout:Duration,
)->ApiResult<Value> {
    let _permit = s.llm.gate.clone().try_acquire_owned().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "llm_busy".into(),
            "LLM 正在处理其他请求，请稍后重试".into(),
        )
    })?;
    let operation = async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(timeout)
            .pool_max_idle_per_host(0)
            .build()
            .map_err(anyhow::Error::from)?;
        let url = config::endpoint(&value.base_url, path)?;
        let mut request = if let Some(body) = body {
            client.post(url).json(&body)
        } else {
            client.get(url)
        };
        if !value.key.is_empty() {
            request = request.bearer_auth(&value.key);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| unavailable("无法连接 LLM 服务，请检查地址与网络"))?;
        if !response.status().is_success() {
            return Err(unavailable(match response.status().as_u16() {
                401 | 403 => "LLM 服务拒绝认证，请检查密钥与权限",
                429 => "LLM 服务限流，请稍后重试",
                _ => "LLM 服务返回错误，请检查模型与服务状态",
            }));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| unavailable("LLM 回复中断，请重试"))?
        {
            if bytes.len() + chunk.len() > 128 * 1024 {
                return Err(unavailable("LLM 回复过大，已中止读取"));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| unavailable("LLM 回复不是有效 JSON"))
    };
    tokio::time::timeout(timeout+Duration::from_secs(5), operation)
        .await
        .map_err(|_| unavailable("LLM 请求超时，已释放连接"))?
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelRequest {
    base_url: Option<String>,
    key: Option<String>,
}
pub async fn models(
    State(s): State<AppState>,
    a: Auth,
    Json(input): Json<ModelRequest>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let (mut value, _) = config::load(&s).await?;
    if let Some(base) = input.base_url {
        let base: String = base.trim().trim_end_matches('/').into();
        if base != value.base_url && input.key.is_none() {
            value.key.clear();
        }
        value.base_url = base;
    }
    if let Some(key) = input.key {
        value.key = key;
    }
    value.validate()?;
    let response = request(&s, &value, "models", None).await?;
    let rows = response["data"]
        .as_array()
        .ok_or_else(|| unavailable("服务未返回模型列表"))?;
    let mut ids = rows
        .iter()
        .take(1000)
        .filter_map(|v| v["id"].as_str())
        .filter(|v| !v.is_empty() && v.len() <= 200 && !v.chars().any(char::is_control))
        .map(String::from)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok(Json(json!({"items":ids})))
}
