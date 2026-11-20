//! Bounded failure history. Never stores source URLs, credentials or decoder paths.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    extract::{Query, State},
    Json,
};
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Instant;
pub async fn record(s: &AppState, id: &str, stage: &str, e: &ApiError, start: Instant) {
    let (id, stage, code, status, ms) = (
        id.to_owned(),
        stage.to_owned(),
        e.1.clone(),
        e.0.as_u16(),
        start.elapsed().as_millis().min(i64::MAX as u128) as i64,
    );
    let _=s.db.run(move|c|{let tx=c.transaction()?;tx.execute("INSERT INTO artwork_events(source_id,stage,code,status,elapsed_ms,created_at)VALUES(?1,?2,?3,?4,?5,unixepoch())",params![id,stage,code,status,ms])?;tx.execute("DELETE FROM artwork_events WHERE id NOT IN(SELECT id FROM artwork_events ORDER BY id DESC LIMIT 512)",[])?;tx.commit()?;Ok(())}).await;
}
#[derive(Default, Deserialize)]
pub struct Options {
    pub source: Option<String>,
}
pub async fn list(
    State(s): State<AppState>,
    a: Auth,
    Query(q): Query<Options>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if q.source
        .as_ref()
        .is_some_and(|id| !crate::artwork_files::valid_id(id))
    {
        return Err(ApiError::bad("图片身份无效"));
    }
    Ok(Json(s.db.read(move|c|{let items=c.prepare("SELECT source_id,stage,code,status,elapsed_ms,created_at FROM artwork_events WHERE ?1 IS NULL OR source_id=?1 ORDER BY id DESC LIMIT 50")?.query_map([q.source],|r|Ok(json!({"source_id":r.get::<_,String>(0)?,"stage":r.get::<_,String>(1)?,"code":r.get::<_,String>(2)?,"status":r.get::<_,u16>(3)?,"elapsed_ms":r.get::<_,i64>(4)?,"created_at":r.get::<_,i64>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok(json!({"items":items,"limit":50,"history_limit":512,"contains_urls":false}))}).await?))
}
