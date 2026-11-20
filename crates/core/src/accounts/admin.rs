use super::*;
use axum::{extract::{Path,Query,State},Json};
use rusqlite::params;
use crate::auth::account::Reauthenticate;

pub(super) async fn gate(s:&AppState,a:&Auth)->ApiResult<()> {a.require("admin")?;require_enabled(s).await?;crate::auth::account::security_attempt(s,a).await}
pub(super) fn verify(c:&rusqlite::Connection,a:&Auth,v:&Reauthenticate)->anyhow::Result<()> {if !crate::auth::account::verify(c,a,v)?{return Err(super::invalid("当前管理员密码或两步验证码不正确"));}Ok(())}
#[derive(Deserialize)]pub struct List {cursor:Option<String>,limit:Option<u32>}
pub async fn list(State(s):State<AppState>,a:Auth,Query(q):Query<List>)->ApiResult<Json<Value>> {
 a.require("admin")?;require_enabled(&s).await?;let after=q.cursor.unwrap_or_default();if after.len()>80{return Err(ApiError::bad("账号游标无效"));}let limit=q.limit.unwrap_or(50).clamp(1,100) as usize;
 let value=s.db.read(move|c|{let ids=c.prepare("SELECT id FROM users WHERE role='external' AND id>?1 ORDER BY id LIMIT ?2")?.query_map(params![after,(limit+1)as i64],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
  let more=ids.len()>limit;let items=ids.iter().take(limit).map(|id|public_user(c,id)?.ok_or_else(||anyhow::anyhow!("账号已移除"))).collect::<anyhow::Result<Vec<_>>>()?;
  Ok(json!({"items":items,"next_cursor":if more{ids.get(limit-1)}else{None}}))}).await?;Ok(Json(value))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub struct Create {
 #[serde(flatten)]credentials:Reauthenticate,pub username:String,pub password:String,#[serde(default)]pub policy:Policy,
}
pub async fn create(State(s):State<AppState>,a:Auth,Json(v):Json<Create>)->ApiResult<Json<Value>> {
 gate(&s,&a).await?;crate::auth::validate_credentials(&v.username,&v.password)?;v.policy.validate()?;
 let id=uuid::Uuid::new_v4().to_string();let name=v.username.trim().to_owned();let password=crate::auth::password_hash(&v.password)?;let guard=a.clone();
 let value=s.db.run(move|c|{let tx=c.transaction()?;verify(&tx,&guard,&v.credentials)?;
  if tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE username=?1)",[&name],|r|r.get::<_,bool>(0))?{return Err(super::invalid("此账号名称已被使用"));}
  tx.execute("INSERT INTO users(id,username,password,role) VALUES(?1,?2,?3,'external')",params![id,name,password])?;
  tx.execute("INSERT INTO external_accounts(user_id,policy,created_at) VALUES(?1,?2,?3)",params![id,serde_json::to_string(&v.policy)?,chrono::Utc::now().timestamp_millis()])?;
  let value=public_user(&tx,&id)?.unwrap();tx.commit()?;Ok(value)}).await?;crate::auth::account::security_success(&s,&a).await;Ok(Json(value))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub struct Update {
 #[serde(flatten)]credentials:Reauthenticate,username:Option<String>,new_password:Option<String>,policy:Option<Policy>,enabled:Option<bool>,#[serde(default)]reset_totp:bool,
}
pub async fn update(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Update>)->ApiResult<Json<Value>> {
 gate(&s,&a).await?;let target=id.clone();if s.db.read(move|c|public_user(c,&target)).await?.is_none(){return Err(ApiError::missing());}let _lease=super::leases::write(&s,&id).await;
 if let Some(name)=v.username.as_ref(){crate::auth::validate_credentials(name,"placeholder-validation-password")?;}
 if let Some(policy)=&v.policy{policy.validate()?;}
 let password=if let Some(password)=&v.new_password{crate::auth::validate_credentials("account",password)?;Some(crate::auth::password_hash(password)?)}else{None};let guard=a.clone();let revoke=id.clone();
 let value=s.db.run(move|c|{let tx=c.transaction()?;verify(&tx,&guard,&v.credentials)?;if public_user(&tx,&id)?.is_none(){return Err(super::invalid("外部账号不存在"));}
  if let Some(name)=v.username {let name=name.trim();if tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE username=?1 AND id<>?2)",params![name,id],|r|r.get::<_,bool>(0))?{return Err(super::invalid("此账号名称已被使用"));}tx.execute("UPDATE users SET username=?2 WHERE id=?1",params![id,name])?;}
  if let Some(password)=password{tx.execute("UPDATE users SET password=?2 WHERE id=?1",params![id,password])?;}
  if v.reset_totp{tx.execute("UPDATE users SET totp_secret=NULL,totp_pending=NULL,recovery='[]' WHERE id=?1",[&id])?;tx.execute("DELETE FROM settings WHERE key IN(?1,?2)",params![format!("totp_last:{id}"),format!("totp_enrollment:{id}")])?;}
  if let Some(policy)=v.policy{tx.execute("UPDATE external_accounts SET policy=?2 WHERE user_id=?1",params![id,serde_json::to_string(&policy)?])?;}
  if let Some(enabled)=v.enabled{tx.execute("UPDATE external_accounts SET enabled=?2 WHERE user_id=?1",params![id,enabled])?;}
  // A stale session must never retain broader permissions or an old credential.
  tx.execute("DELETE FROM sessions WHERE user_id=?1",[&id])?;let value=public_user(&tx,&id)?.unwrap();tx.commit()?;Ok(value)}).await?;
 crate::accounts::scope::disconnect(&s,&revoke).await?;
 crate::auth::account::security_success(&s,&a).await;Ok(Json(value))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]pub struct Delete {#[serde(flatten)]credentials:Reauthenticate,confirmation:String}
pub async fn delete(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<Delete>)->ApiResult<Json<Value>> {
 gate(&s,&a).await?;let target=id.clone();if s.db.read(move|c|public_user(c,&target)).await?.is_none(){return Err(ApiError::missing());}let _lease=super::leases::write(&s,&id).await;let _zones=s.zone_lock.lock().await;let guard=a.clone();let removed=id.clone();
 let result=s.db.run(move|c|{let tx=c.transaction()?;verify(&tx,&guard,&v.credentials)?;
  let user=public_user(&tx,&id)?.ok_or_else(||super::invalid("外部账号不存在"))?;if user["username"].as_str()!=Some(v.confirmation.as_str()){return Err(super::invalid("请输入完整账号名称以确认删除"));}
  clear_user_data(&tx,&id)?;tx.execute("DELETE FROM external_accounts WHERE user_id=?1",[&id])?;tx.execute("DELETE FROM users WHERE id=?1 AND role='external'",[&id])?;tx.commit()?;Ok(json!({"deleted":true}))}).await?;
 s.tickets.lock().await.retain(|_,ticket|ticket.account.as_deref()!=Some(removed.as_str()));
 let realm=format!("external:{removed}");
 let endpoint_ids=crate::accounts::scope::owners(&s,"endpoint").await?.into_iter().filter(|(_,r)|r==&realm).map(|(id,_)|id).collect::<Vec<_>>();
 for id in endpoint_ids{if let Some(tx)=s.bridges.write().await.remove(&id){let _=tx.try_send(json!({"type":"remove_endpoint","endpoint_id":id}));}s.browser_owners.write().await.remove(&id);s.bridge_owners.write().await.remove(&id);s.clock_filters.write().await.remove(&id);s.endpoints.write().await.remove(&id);}
 let zone_ids=crate::accounts::scope::owners(&s,"zone").await?.into_iter().filter(|(_,r)|r==&realm).map(|(id,_)|id).collect::<Vec<_>>();
 for id in zone_ids{crate::autoplay::cancel(&s,&id).await;crate::sequence::invalidate(&s,&id).await?;s.zones.write().await.remove(&id);}
 s.emit("endpoints",json!({}));s.emit("zones_changed",json!({}));
 crate::auth::account::security_success(&s,&a).await;Ok(Json(result))
}
pub(crate) fn clear_user_data(c:&rusqlite::Connection,id:&str)->anyhow::Result<()> {
 // Shared music/catalogue/cache and provider credentials are never user-owned data.
 let events=c.prepare("SELECT event_id FROM account_session_owners WHERE user_id=?1")?.query_map([id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
 for event in events{
  let claims=c.prepare("SELECT playback_id FROM listen_claims WHERE group_id=?1")?.query_map([&event],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
  for claim in claims{for table in ["listen_claim_users","listen_intervals","listen_output_intervals","listen_output_cursors","group_output_totals","listen_source_ends"]{c.execute(&format!("DELETE FROM {table} WHERE playback_id=?1"),[&claim])?;}c.execute("DELETE FROM listen_claims WHERE playback_id=?1",[&claim])?;}
  c.execute("DELETE FROM listen_groups WHERE id=?1",[&event])?;c.execute("DELETE FROM play_sessions WHERE id=?1",[&event])?;
 }
 for (table,kind) in [("zones","zone"),("devices","device"),("output_settings","endpoint")]{c.execute(&format!("DELETE FROM {table} WHERE id IN(SELECT id FROM external_resource_owners WHERE kind=?1 AND user_id=?2)"),params![kind,id])?;}
 c.execute("DELETE FROM settings WHERE key IN(SELECT 'output_alias:'||id FROM external_resource_owners WHERE kind='endpoint' AND user_id=?1)",[id])?;
 c.execute("DELETE FROM account_origins WHERE user_id=?1",[id])?;
 c.execute("DELETE FROM settings WHERE key IN(?1,?2)",params![format!("totp_last:{id}"),format!("totp_enrollment:{id}")])?;
 c.execute("DELETE FROM account_import_jobs WHERE target_user=?1",[id])?;

 for table in ["playlist_bindings","provider_playlist_directory","provider_directory_refresh","playlist_import_receipts","playlists","preferences","offline_listens","music_version_preferences","music_playback_policies","account_history","account_track_stats","account_listening_buckets","account_stat_origins","account_listening_metadata","account_music_preferences","account_session_owners","listen_claim_users","sync_group_recoveries","sessions"] {
  c.execute(&format!("DELETE FROM {table} WHERE user_id=?1"),[id])?;
 }
 Ok(())
}
