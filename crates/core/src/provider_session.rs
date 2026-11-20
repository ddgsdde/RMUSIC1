//! Durable request epochs prevent late account/plugin responses from entering a new session.
use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};
use rusqlite::{Connection, OptionalExtension};
pub fn bump(c: &Connection, id: &str) -> anyhow::Result<()> {
    c.execute("INSERT INTO provider_generations(provider,revision)VALUES(?1,1) ON CONFLICT(provider)DO UPDATE SET revision=revision+1",[id])?;
    Ok(())
}
pub fn stamp(c: &Connection, id: &str) -> anyhow::Result<i64> {
    Ok(c.query_row(
        "SELECT revision FROM provider_generations WHERE provider=?1",
        [id],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or(0))
}
pub async fn start(s: &AppState, id: &str) -> ApiResult<i64> {
    let id = id.to_owned();
    Ok(s.db.read(move |c| stamp(c, &id)).await?)
}
#[derive(Debug)]
pub struct Changed;
impl std::fmt::Display for Changed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "音源账号或插件已变化，旧响应已取消")
    }
}
impl std::error::Error for Changed {}
pub fn check(c: &Connection, id: &str, expected: i64) -> anyhow::Result<()> {
    let enabled = c
        .query_row("SELECT enabled FROM plugins WHERE id=?1", [id], |r| {
            r.get::<_, bool>(0)
        })
        .optional()?
        .unwrap_or(true);
    if !enabled || stamp(c, id)? != expected {
        return Err(Changed.into());
    }
    Ok(())
}
pub async fn validate(s: &AppState, id: &str, expected: i64) -> ApiResult<()> {
    let id = id.to_owned();
    let valid =
        s.db.read(move |c| Ok(check(c, &id, expected).is_ok()))
            .await?;
    if !valid {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "provider_session_changed".into(),
            "音源账号或插件已变化，请重新读取；旧响应未保存".into(),
        ));
    }
    Ok(())
}
