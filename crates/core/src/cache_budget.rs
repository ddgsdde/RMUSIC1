//! One disk quota contract for settings, downloads and portable copies. Never evict on save.
use crate::{error::ApiError,state::AppState};
use rusqlite::Connection;
pub fn used(c:&Connection)->rusqlite::Result<u64> {
 c.query_row("SELECT (SELECT COALESCE(SUM(MAX(size,allocated_size)),0) FROM cache_entries)+(SELECT COALESCE(SUM(size),0) FROM cache_exports)+(SELECT COALESCE(SUM(size),0) FROM cache_retired)+(SELECT COALESCE(SUM(size),0) FROM file_bin WHERE storage='cache' AND status IN ('planned','trashed','restoring'))",[],|r|r.get::<_,i64>(0).map(|n|n.max(0) as u64))
}
pub fn validate(c:&Connection,limit:u64)->Result<(),ApiError> {
 let occupied=used(c)?;
 if limit<occupied {return Err(ApiError(axum::http::StatusCode::CONFLICT,"cache_budget_below_usage".into(),format!("缓存预算不能低于当前占用 {occupied} 字节。请先在音乐库手动清理缓存；回收站也计入占用。")));}
 Ok(())
}
pub async fn fits(s:&AppState,additional:u64)->anyhow::Result<bool>{
 s.db.run(move|c|{use rusqlite::OptionalExtension;let limit=c.query_row("SELECT value FROM settings WHERE key='cache_limit_bytes'",[],|r|r.get::<_,String>(0)).optional()?.and_then(|v|serde_json::from_str::<u64>(&v).ok()).unwrap_or(2*1024*1024*1024);Ok(used(c)?.saturating_add(additional)<=limit)}).await
}
