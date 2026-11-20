//! Administrator-only network operations. Connection metadata is account-readable.
pub mod network;
pub mod proxy;
pub mod transport;
pub mod local_proxy;
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{extract::State, Json};
use rmusic_remote::{config::Configuration, runtime::Secrets};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
pub struct SecureIngress;
pub async fn reauthenticate(
    s: &AppState,
    a: &Auth,
    secure: Option<axum::Extension<SecureIngress>>,
    credentials: crate::auth::account::Reauthenticate,
) -> ApiResult<()> {
    a.require("admin")?;
    if secure.is_none() {
        return Err(ApiError::bad(
            "敏感设置只接受经过验证的 HTTPS／原生加密通道；请使用安全入口或本地部署工具",
        ));
    }
    verify_admin(s, a, credentials).await
}
async fn verify_admin(s:&AppState,a:&Auth,credentials:crate::auth::account::Reauthenticate)->ApiResult<()> {
    a.require("admin")?;
    crate::auth::account::security_attempt(s, a).await?;
    let auth = a.clone();
    let valid =
        s.db.run(move |c| crate::auth::account::verify(c, &auth, &credentials))
            .await?;
    if !valid {
        return Err(ApiError::bad("当前密码或两步验证码不正确"));
    }
    crate::auth::account::security_success(s, a).await;
    Ok(())
}
#[derive(Default, Deserialize)]
pub struct ManifestQuery { after: Option<u64> }

pub async fn connection_info(
    State(s): State<AppState>, a: Auth, axum::extract::Query(query): axum::extract::Query<ManifestQuery>,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    a.require("read")?;
    let legacy = s.config.public_url.as_ref().map(|url| format!("{}{}", url.trim_end_matches('/'), s.config.base_path));
    let mut changes = s.remote.manifest_changed.subscribe();
    changes.borrow_and_update();
    let snapshot = s.remote.manifest_snapshot(&s.core_id, legacy.clone()).await?;
    let headers = [("x-rmusic-connection-watch", "1"), ("cache-control", "no-store")];
    if query.after != Some(snapshot.revision) {
        return Ok((headers, Json(s.remote.identity.sign(snapshot)?)).into_response());
    }
    // Send headers immediately. Whitespace keeps an idle HTTP stream alive without signing
    // or writing a manifest. A finite lifetime reauthenticates idle sessions and renews device leases.
    let revision = snapshot.revision;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(240);
    let stream = futures_util::stream::unfold(Some((s, changes, legacy, a.session_hash)), move |state| async move {
        let (s, mut changes, legacy, owner) = state?;
        let item: Result<axum::body::Bytes, std::io::Error> = loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return None,
                _ = tokio::time::sleep(std::time::Duration::from_secs(35)) => break Ok(axum::body::Bytes::from_static(b"\n")),
                result = changes.changed() => {
                    if result.is_err() || !s.session_active(owner.clone()).await { return None; }
                    match s.remote.manifest_snapshot(&s.core_id, legacy.clone()).await {
                        Ok(current) if current.revision == revision => continue,
                        Ok(current) => {
                            let doc = s.remote.identity.sign(current).and_then(|doc| Ok(serde_json::to_vec(&doc)?));
                            return Some((doc.map(axum::body::Bytes::from).map_err(std::io::Error::other), None));
                        }
                        Err(e) => break Err(std::io::Error::other(e)),
                    }
                }
            }
        };
        Some((item, Some((s, changes, legacy, owner))))
    });
    Ok((headers, axum::body::Body::from_stream(stream)).into_response())
}
pub async fn get(State(s): State<AppState>, a: Auth, secure: Option<axum::Extension<SecureIngress>>) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let secret = s
        .remote
        .vault
        .get::<Secrets>("providers")?
        .unwrap_or_default();
    Ok(Json(
        json!({"core_settings_transport":if secure.is_some() {"secure"} else {"restricted"},"configuration":s.remote.configuration().await,"status":*s.remote.status.read().await,"core_public_key":s.remote.identity.public_key(),"secrets":{"core_cloudflare":secret.core_cloudflare.is_some(),"core_eab":secret.core_eab.is_some()}}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    expected_revision: u64,
    configuration: Configuration,
    credentials: crate::auth::account::Reauthenticate,
    #[serde(default)]
    secrets: Option<Secrets>,
}
pub async fn save(
    State(s): State<AppState>,
    a: Auth,
    secure: Option<axum::Extension<SecureIngress>>,
    Json(mut v): Json<Edit>,
) -> ApiResult<Json<Value>> {
    reauthenticate(&s, &a, secure, v.credentials).await?;
    v.configuration
        .validate()
        .map_err(|e| ApiError::bad(e.to_string()))?;
    let ingress=&v.configuration.core;
    if ingress.managed() && matches!(ingress.certificate.challenge,rmusic_remote::config::Challenge::Imported|rmusic_remote::config::Challenge::External) {
        rmusic_remote::certificate::validate(&rmusic_remote::certificate::Files {
            cert:ingress.certificate.external_cert.as_deref().unwrap_or_default().into(),
            key:ingress.certificate.external_key.as_deref().unwrap_or_default().into(),
        },&ingress.domain).await.map_err(|_|ApiError::bad("证书的域名、有效期、证书链或私钥未通过验证；原配置保留"))?;
    }
    let _lock = s.remote.update_lock.lock().await;
    if s.remote.configuration().await.revision != v.expected_revision {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "configuration_changed".into(),
            "连接设置已变化，请刷新后重试".into(),
        ));
    }
    let current = s.remote.configuration().await;
    if v.configuration.direct_urls.is_none() {
        v.configuration.direct_urls=current.direct_urls.clone();
    }
    // Only supplied secret values replace existing values; an empty value removes it.
    if let Some(input) = v.secrets {
        let mut old = s
            .remote
            .vault
            .get::<Secrets>("providers")?
            .unwrap_or_default();
        for (target, value) in [
            (&mut old.core_cloudflare, input.core_cloudflare),
            (&mut old.core_eab, input.core_eab),
        ] {
            if let Some(value) = value {
                if value.len() > 8192 || value.chars().any(char::is_control) {
                    return Err(ApiError::bad("密钥格式无效"));
                }
                *target = if value.is_empty() { None } else { Some(value) };
            }
        }
        s.remote.vault.put("providers", &old)?;
    }
    let revision = s.remote.save(v.configuration, v.expected_revision).await?;
    crate::auth::account::security_success(&s, &a).await;
    s.emit("connection_info", json!({"revision":revision}));
    Ok(Json(json!({"revision":revision,"saved":true})))
}
