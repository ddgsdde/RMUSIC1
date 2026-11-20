use crate::{error::{ApiError, ApiResult}, state::AppState};
use axum::{extract::{Request, State, WebSocketUpgrade}, response::{IntoResponse, Response}, Extension};
use std::sync::{Arc, OnceLock};

pub async fn native(
    State(state): State<AppState>,
    proxy: Option<Extension<super::proxy::HttpsProxy>>,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    static LIMIT: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let permit = LIMIT.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(128)))
        .clone().try_acquire_owned().map_err(|_| ApiError::bad("原生连接数量过多"))?;
    let tls = state.remote.identity.tls(&state.core_id)?;
    let cancel = state.remote.cancel.child_token();
    let window = rmusic_remote::tunnel::WINDOW;
    Ok(ws.max_message_size(window).max_frame_size(window).on_upgrade(move |socket| async move {
        let _permit = permit;
        let mut router = internal_router(state).layer(Extension(super::SecureIngress));
        if let Some(proxy) = proxy { router = router.layer(proxy); }
        let (wire_io, server_io) = tokio::io::duplex(window);
        let wire = rmusic_remote::tunnel::bridge_server(socket, wire_io, cancel.clone());
        let server = rmusic_remote::native::serve_core(server_io, router, Some(tls), cancel.clone());
        let _ = rmusic_remote::tunnel::serve_and_drain(wire, server, cancel).await;
    }).into_response())
}
pub fn internal_router(state: AppState) -> axum::Router {
    let router = crate::app(state.clone());
    let prefix = state.config.base_path.clone();
    axum::Router::new().fallback(move |mut request: Request| {
        let router = router.clone();
        let prefix = prefix.clone();
        async move {
            use tower::ServiceExt;
            if !prefix.is_empty() {
                let path = format!(
                    "{}{}{}",
                    prefix,
                    request.uri().path(),
                    request
                        .uri()
                        .query()
                        .map(|q| format!("?{q}"))
                        .unwrap_or_default()
                );
                if let Ok(uri) = path.parse() {
                    *request.uri_mut() = uri;
                } else {
                    return axum::http::StatusCode::BAD_REQUEST.into_response();
                }
            }
            router.oneshot(request).await.unwrap()
        }
    })
}
