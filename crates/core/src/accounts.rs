//! Independent external accounts share only explicitly permitted catalogue sources.
use crate::{auth::Auth,error::{ApiError,ApiResult},state::AppState};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use rusqlite::OptionalExtension;
pub(crate) mod admin;
pub(crate) mod archive;
pub(crate) mod guard;
pub(crate) mod leases;
pub(crate) mod scope;
pub(crate) mod bandwidth;
pub(crate) mod listening;
pub(crate) mod catalog;
#[derive(Debug)]pub(crate) struct Invalid(pub String);
impl std::fmt::Display for Invalid {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{self.0.fmt(f)}}
impl std::error::Error for Invalid {}
pub(crate) fn invalid(message:&str)->anyhow::Error{Invalid(message.into()).into()}

#[derive(Clone,Debug,Default,Deserialize,Serialize,PartialEq)]
#[serde(default,deny_unknown_fields)]
pub struct Policy {
 pub providers:Vec<String>,pub local_cache:bool,pub local_library:bool,
 /// Aggregate bytes/second for this account, including concurrent media responses.
 pub bandwidth_bytes_per_second:u64,
}
impl Policy {
 pub fn validate(&self)->ApiResult<()> {
  if self.providers.len()>32||self.bandwidth_bytes_per_second>1024*1024*1024||self.providers.iter().any(|p|p.is_empty()||p.len()>80||p=="local"||!p.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-".contains(&b))){return Err(ApiError::bad("音源范围或带宽设置无效"));}
  if self.providers.iter().collect::<std::collections::HashSet<_>>().len()!=self.providers.len(){return Err(ApiError::bad("音源平台不能重复"));}Ok(())
 }
 pub fn sql(&self,alias:&str)->String {
  assert!(matches!(alias,"t"|"l"|"v"));
  let mut allowed=Vec::new();
  if !self.providers.is_empty(){allowed.push(format!("{alias}.source IN ({})",self.providers.iter().map(|v|format!("'{}'",v.replace('\'',"''"))).collect::<Vec<_>>().join(",")));}
  if self.local_library {allowed.push(format!("({alias}.source='local' AND json_type({alias}.provider_data,'$.cached_from.cache_id') IS NULL AND NOT EXISTS(SELECT 1 FROM cache_entries owned_cache WHERE owned_cache.path={alias}.path))"));}
  if self.local_cache {allowed.push(format!("EXISTS(SELECT 1 FROM cache_entries ac WHERE ac.complete=1 AND (ac.path={alias}.path OR ac.track_id={alias}.id))"));}
  if allowed.is_empty(){"0".into()}else{format!("({})",allowed.join(" OR "))}
 }
}
/// A deployment can explicitly disable the feature without deleting account data.
pub async fn enabled(s:&AppState)->bool {s.setting("external_accounts_development",json!(true)).await==true}
pub async fn require_enabled(s:&AppState)->ApiResult<()> {if enabled(s).await{Ok(())}else{Err(ApiError::forbidden())}}
pub async fn login_allowed(s:&AppState,id:&str)->ApiResult<()> {
 require_enabled(s).await?;let id=id.to_owned();
 let active=s.db.read(move|c|Ok(c.query_row("SELECT enabled FROM external_accounts WHERE user_id=?1",[id],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false))).await?;
 if active{Ok(())}else{Err(ApiError::auth())}
}
pub async fn policy(s:&AppState,a:&Auth)->ApiResult<Option<Policy>> {
 if a.role!="external"{return Ok(None);}login_allowed(s,&a.id).await?;let id=a.id.clone();
 let raw=s.db.read(move|c|Ok(c.query_row("SELECT policy FROM external_accounts WHERE user_id=?1",[id],|r|r.get::<_,String>(0))?)).await?;
 let policy:Policy=serde_json::from_str(&raw).map_err(anyhow::Error::from)?;policy.validate()?;Ok(Some(policy))
}
pub(crate) fn public_user(c:&rusqlite::Connection,id:&str)->anyhow::Result<Option<Value>> {
 c.query_row("SELECT u.id,u.username,u.role,u.totp_secret IS NOT NULL,e.policy,e.enabled,e.created_at FROM users u JOIN external_accounts e ON e.user_id=u.id WHERE u.id=?1 AND u.role='external'",[id],|r|{
  let policy=r.get::<_,String>(4)?;
  Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?,"role":"external","totp_enabled":r.get::<_,bool>(3)?,"policy":serde_json::from_str::<Value>(&policy).unwrap_or(json!({})),"enabled":r.get::<_,bool>(5)?,"created_at":r.get::<_,i64>(6)?}))
 }).optional().map_err(Into::into)
}
