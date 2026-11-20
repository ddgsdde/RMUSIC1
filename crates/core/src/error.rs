use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
#[derive(Debug,Clone)]
pub struct ApiError(pub StatusCode, pub String, pub String);
impl ApiError {
    pub fn bad(s: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, "invalid_request".into(), s.into())
    }
    pub fn auth() -> Self {
        Self(
            StatusCode::UNAUTHORIZED,
            "unauthorized".into(),
            "登录已过期或凭证无效".into(),
        )
    }
    pub fn forbidden() -> Self {
        Self(
            StatusCode::FORBIDDEN,
            "forbidden".into(),
            "凭证权限不足".into(),
        )
    }
    pub fn missing() -> Self {
        Self(
            StatusCode::NOT_FOUND,
            "not_found".into(),
            "资源不存在".into(),
        )
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"error":{"code":self.1,"message":self.2}})),
        )
            .into_response()
    }
}
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        if let Some(error)=e.downcast_ref::<crate::accounts::Invalid>(){return Self::bad(&error.0);}
        tracing::error!(error=%e,"request failed");
        if e.is::<crate::provider_session::Changed>(){return Self(StatusCode::CONFLICT,"provider_session_changed".into(),e.to_string());}
        if e.is::<crate::db::Busy>() {return Self(StatusCode::SERVICE_UNAVAILABLE,"database_busy".into(),e.to_string());}
        if let Some(rusqlite::Error::SqliteFailure(code,_))=e.downcast_ref::<rusqlite::Error>() {
            let (id,message)=match code.code {
                rusqlite::ErrorCode::DiskFull=>("storage_full","核心存储已满，请释放空间后重试；不会自动删除音乐"),
                rusqlite::ErrorCode::DatabaseBusy|rusqlite::ErrorCode::DatabaseLocked=>("database_busy","数据库繁忙，请稍后重试"),
                rusqlite::ErrorCode::ReadOnly|rusqlite::ErrorCode::SystemIoFailure=>("storage_unavailable","核心存储不可写或连接中断，请检查 NAS 挂载和权限后重试"),
                _=>return Self(StatusCode::INTERNAL_SERVER_ERROR,"internal_error".into(),"核心内部错误，请检查日志".into()),
            };return Self(StatusCode::SERVICE_UNAVAILABLE,id.into(),message.into());
        }
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error".into(),
            "核心内部错误，请检查日志".into(),
        )
    }
}
impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        anyhow::Error::from(e).into()
    }
}
pub type ApiResult<T> = Result<T, ApiError>;
