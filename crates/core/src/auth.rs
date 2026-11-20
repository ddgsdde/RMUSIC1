use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};
use argon2::{password_hash::SaltString, Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::{
    extract::{FromRequestParts, Path, State},
    http::{request::Parts, StatusCode},
    Json,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn random_token() -> String {
    let mut b = [0; 32];
    rand::thread_rng().fill_bytes(&mut b);
    data_encoding::BASE64URL_NOPAD.encode(&b)
}
pub fn hash(s: &str) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(s.as_bytes()))
}
#[derive(Clone, Serialize)]
pub struct Auth {
    pub id: String,
    pub username: String,
    pub role: String,
    pub totp_enabled: bool,
    #[serde(skip)]
    pub interactive: bool,
    #[serde(skip)]
    pub scopes: Vec<String>,
    #[serde(skip)]
    pub session_hash: String,
}
impl Auth {
    pub fn require(&self, scope: &str) -> ApiResult<()> {
        if self.role=="external"&&!matches!(scope,"read"|"control"){return Err(ApiError::forbidden());}
        if self.scopes.iter().any(|s| s == "admin" || s == scope) {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
}
impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> ApiResult<Self> {
        if let Some(auth)=parts.extensions.get::<Auth>(){return Ok(auth.clone());}
        let token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(ApiError::auth)?;
        authenticate(state, token).await
    }
}
pub async fn authenticate(s: &AppState, token: &str) -> ApiResult<Auth> {
    authenticate_hash(s,hash(token)).await
}
pub(crate) async fn authenticate_hash(s:&AppState,h:String)->ApiResult<Auth>{
    let sh = h.clone();
    let now = chrono::Utc::now().timestamp();
    let a=s.db.read(move|c|Ok(c.query_row("SELECT u.id,u.username,u.role,s.scopes,u.totp_secret IS NOT NULL,s.name IS NULL FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.hash=?1 AND s.expires>?2",params![h,now],|r|Ok(Auth{id:r.get(0)?,username:r.get(1)?,role:r.get(2)?,scopes:serde_json::from_str(&r.get::<_,String>(3)?).unwrap_or_default(),totp_enabled:r.get(4)?,interactive:r.get(5)?,session_hash:sh})).optional()?)).await?;
    let a=a.ok_or_else(ApiError::auth)?;if a.role=="external"{crate::accounts::login_allowed(s,&a.id).await?;}Ok(a)
}
#[derive(Deserialize)]
pub struct LoginClient {
    id: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
}
impl LoginClient {
    fn normalized(mut self) -> Option<Self> {
        self.name = self.name.trim().to_owned();
        let valid = !self.id.is_empty() && self.id.len() <= 100
            && self.id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && !self.name.is_empty() && self.name.chars().count() <= 100
            && !self.name.chars().any(char::is_control)
            && matches!(self.kind.as_str(), "macos" | "android" | "windows" | "linux" | "browser");
        valid.then_some(self)
    }
}
async fn session(
    s: &AppState,
    id: String,
    name: Option<String>,
    scopes: Vec<String>,
    days: i64,
    client: Option<LoginClient>,
) -> ApiResult<String> {
    let token = random_token();
    let h = hash(&token);
    let now = chrono::Utc::now().timestamp();
    s.db.run(move |c| {
        let tx = c.transaction()?;
        tx.execute("DELETE FROM sessions WHERE expires<=?1", [now])?;
        if let Some(client) = &client {
            tx.execute("DELETE FROM sessions WHERE user_id=?1 AND name IS NULL AND hash IN (SELECT session_hash FROM login_clients WHERE client_id=?2)", params![id,client.id])?;
        }
        tx.execute("INSERT INTO sessions(hash,user_id,expires,name,scopes,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![h,id,now+days*86400,name,serde_json::to_string(&scopes)?,now])?;
        if let Some(client) = client {
            tx.execute("INSERT INTO login_clients(session_hash,client_id,device_name,device_type) VALUES(?1,?2,?3,?4)",
                params![h,client.id,client.name,client.kind])?;
        }
        tx.commit()?;
        Ok(())
    }).await?;
    Ok(token)
}
pub async fn status(State(s): State<AppState>) -> ApiResult<Json<Value>> {
    let initialized = s
        .db
        .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))? > 0))
        .await?;
    let mut value=json!({"initialized":initialized,"core_id":s.core_id.as_str(),"version":env!("CARGO_PKG_VERSION"),"name":s.setting("name",json!("R-Music")).await,"capabilities":{"guarded_transport_v1":true,"group_start_v1":true,"group_start_list_v1":true,"group_queue_v1":true,"group_output_v2":true,"group_output_final_v2":true,"group_manual_recovery_v2":true,"group_volume_v1":true,"group_transport_v1":true,"group_resync_v1":true,"group_queue_jump_v1":true,"group_membership_v1":true,"group_clock_diagnostics_v1":true,"programme_clock_v2":true,"group_rate_adjustment_v1":true,"automatic_group_rate_v1":true,"output_groups_v1":true,"smart_playlists":true,"system_suspend_recovery":true,"system_suspend_fresh_start":true,"measured_listens":true,"measured_listens_v2":true,"measured_listens_online_v2":true,"measured_listens_socket_v2":true,"measured_listens_recovery_v2":true,"continuous_recovery":true,"recovery_controls":true,"browser_pcm":true,"browser_aac_pcm":s.setting("lossless_transport_only",json!(true)).await==json!(false),"lossless_transport_only":s.setting("lossless_transport_only",json!(true)).await!=json!(false),"maintenance_backup_v1":true,"library_file_management_v1":true,"server_directory_picker_v1":true,"library_hub_v1":true,"convolution_config_v1":true,"frame_clock_max_rate":rmusic_audio::MAX_CLOCK_RATE}});
    value["capabilities"]["device_groups_v1"]=json!(true);
    value["capabilities"]["dsp_workbench_v1"]=json!(true);
    value["capabilities"]["dsp_modules_v1"]=json!(true);
    value["capabilities"]["personal_radio_v1"]=json!(true);
    value["capabilities"]["portable_control_v1"]=json!(true);
    value["core_public_key"]=json!(s.remote.identity.public_key());
    value["capabilities"]["native_e2e_v1"]=json!(true);
    value["capabilities"]["client_public_metadata_v1"]=json!(true);
    value["capabilities"]["native_flac_v1"]=json!(true);
    value["capabilities"]["external_accounts_v1"]=json!(crate::accounts::enabled(&s).await);
    value["capabilities"]["mobile_opus_v1"]=json!(true);
    value["capabilities"]["mobile_opus_enabled"]=s.setting("mobile_opus_enabled",json!(false)).await;
    value["capabilities"]["visualization_v1"]=json!(true);
    for key in ["smart_playlist_rules_v2","music_versions_v1","llm_recommendations_v1","plugin_developer_v1"] {value["capabilities"][key]=json!(true);}

    value["capabilities"]["device_purge_v1"]=json!(true);
    for key in ["music_identity_v1","local_fingerprint_v1","extension_rpc_v1","plugin_configuration_v1","group_sync_policy_v1"]{value["capabilities"][key]=json!(true);}
    Ok(Json(value))
}
#[derive(Deserialize)]
pub struct Setup {
    bootstrap_token: String,
    username: String,
    password: String,
}
pub(crate) fn validate_credentials(name: &str, password: &str) -> ApiResult<()> {
    if name.trim().is_empty() || name.len() > 80 || password.len() < 10 || password.len() > 1024 {
        return Err(ApiError::bad("账号不能为空，密码至少 10 个字符"));
    }
    Ok(())
}
pub(crate) fn password_hash(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .to_string())
}
pub async fn setup(State(s): State<AppState>, Json(v): Json<Setup>) -> ApiResult<Json<Value>> {
    if hash(&v.bootstrap_token) != hash(&s.bootstrap) {
        return Err(ApiError::forbidden());
    }
    validate_credentials(&v.username, &v.password)?;
    let name = v.username.trim().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let uid = id.clone();
    let uname = name.clone();
    let count =
        s.db.run(move |c| {
            let tx = c.transaction()?;
            let n: i64 = tx.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
            if n == 0 {
                tx.execute(
                    "INSERT INTO users(id,username,password,role) VALUES(?1,?2,?3,'admin')",
                    params![uid, uname, password_hash(&v.password)?],
                )?;
                tx.commit()?;
            }
            Ok(n)
        })
        .await?;
    if count != 0 {
        return Err(ApiError::bad("核心已经初始化"));
    }
    let _ = tokio::fs::remove_file(s.config.data.join("bootstrap.token")).await;
    let token = session(&s, id.clone(), None, vec!["admin".into()], 30, None).await?;
    Ok(Json(
        json!({"token":token,"user":{"id":id,"username":name,"role":"admin"}}),
    ))
}
#[derive(Deserialize)]
pub struct Login {
    username: String,
    password: String,
    totp: Option<String>,
    client: Option<LoginClient>,
}
pub async fn login(State(s): State<AppState>, source:Option<axum::Extension<crate::remote_access::proxy::ClientAddress>>, Json(v): Json<Login>) -> ApiResult<Json<Value>> {
    if v.password.len() > 1024 || v.username.len() > 80 {
        return Err(ApiError::auth());
    }
    let key = v.username.to_lowercase();
    let now = chrono::Utc::now().timestamp();
    {
        let mut a = s.login_attempts.lock().await;
        a.retain(|_, (_, t)| now - *t < 300);
        // Bound expensive password hashing even for invented usernames.
        let global=a.entry("\0login-global".into()).or_insert((0,now));
        if now-global.1>=60 {*global=(0,now);}
        if global.0>=120 {return Err(ApiError(StatusCode::TOO_MANY_REQUESTS,"rate_limited".into(),"登录请求过多，请稍后重试".into()));}
        global.0+=1;
        if let Some(source)=source{let source=a.entry(format!("\0login-ip:{}",source.0.0)).or_insert((0,now));if now-source.1>=60{*source=(0,now);}if source.0>=30{return Err(ApiError(StatusCode::TOO_MANY_REQUESTS,"rate_limited".into(),"此来源的登录请求过多，请稍后重试".into()));}source.0+=1;}
        if a.len() > 10000 {
            a.clear();
        }
        let entry = a.entry(key.clone()).or_insert((0, now));
        if entry.0 >= 10 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited".into(),
                "登录尝试过多，请 5 分钟后重试".into(),
            ));
        }
        entry.0 += 1;
    }
    let candidate_name=v.username.clone();let candidate=s.db.read(move|c|Ok(c.query_row("SELECT id FROM users WHERE username=?1",[candidate_name],|r|r.get::<_,String>(0)).optional()?)).await?;
    let _account_lease=if let Some(id)=candidate{Some(crate::accounts::leases::read(&s,&id).await)}else{None};
    let client = v.client.and_then(LoginClient::normalized);
    let code = v.totp.clone();
    let found=s.db.run(move|c|{
        let user=c.query_row("SELECT id,username,password,role,totp_secret,recovery FROM users WHERE username=?1",[v.username],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,String>(5)?))).optional()?;
        if let Some((id,name,password,role,secret,recovery))=user {
            if !PasswordHash::new(&password).ok().is_some_and(|h|Argon2::default().verify_password(v.password.as_bytes(),&h).is_ok()){return Ok(None);}
            if let Some(secret)=secret {
                let code=code.unwrap_or_default();
                if let Some(step)=(-1..=1).map(|delta|now/30+delta).find(|step|totp(&secret,*step).as_deref()==Some(&code)) {
                    let key=format!("totp_last:{id}");let last=c.query_row("SELECT value FROM settings WHERE key=?1",[&key],|r|r.get::<_,String>(0)).optional()?.and_then(|n|n.parse::<i64>().ok()).unwrap_or(-1);if step<=last{return Ok(None);}
                    c.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,step.to_string()])?;
                }else{
                    let mut codes:Vec<String>=serde_json::from_str(&recovery)?;let before=codes.len();codes.retain(|s|s!=&hash(&code));if codes.len()==before{return Ok(None);}
                    c.execute("UPDATE users SET recovery=?1 WHERE id=?2",params![serde_json::to_string(&codes)?,id])?;
                }
            }
            Ok(Some((id,name,role)))
        }else {let _=password_hash(&v.password)?;Ok(None)}
    }).await?;
    let (id, name, role) = found.ok_or_else(ApiError::auth)?;
    if role=="external" {crate::accounts::login_allowed(&s,&id).await?;}
    s.login_attempts.lock().await.remove(&key);
    let scopes = if role == "admin" {
        vec!["admin".into()]
    } else {
        vec!["read".into(), "control".into()]
    };
    let token = session(&s, id.clone(), None, scopes, 30, client).await?;
    let mut user=json!({"id":id,"username":name,"role":role});
    if role=="external"{let account=authenticate(&s,&token).await?;user=me(State(s.clone()),account).await?.0;}
    Ok(Json(json!({"token":token,"user":user})))
}
pub async fn me(State(s):State<AppState>,a: Auth) -> ApiResult<Json<Value>> {
    let mut value=serde_json::to_value(&a).map_err(anyhow::Error::from)?;if let Some(policy)=crate::accounts::policy(&s,&a).await?{value["policy"]=json!(policy);value["capabilities"]=json!({"same_account_remote_control":true,"zone_sync":false});}Ok(Json(value))
}
pub async fn logout(State(s): State<AppState>, a: Auth) -> ApiResult<StatusCode> {
    s.db.run(move |c| {
        c.execute("DELETE FROM sessions WHERE hash=?1", [a.session_hash])?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub fn totp(secret: &str, step: i64) -> Option<String> {
    let key = data_encoding::BASE32_NOPAD.decode(secret.as_bytes()).ok()?;
    let mut h = Hmac::<sha1::Sha1>::new_from_slice(&key).ok()?;
    h.update(&(step as u64).to_be_bytes());
    let d = h.finalize().into_bytes();
    let i = (d[19] & 15) as usize;
    let n = u32::from_be_bytes(d[i..i + 4].try_into().ok()?) & 0x7fffffff;
    Some(format!("{:06}", n % 1_000_000))
}
pub fn valid_totp(secret: &str, code: &str, now: i64) -> bool {
    code.len() == 6 && (-1..=1).any(|i| totp(secret, now / 30 + i).as_deref() == Some(code))
}
pub mod account;
pub use account::start as totp_start;
#[derive(Deserialize)]
pub struct Code {
    code: String,
}
pub async fn totp_confirm(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<Code>,
) -> ApiResult<Json<Value>> {
    account::security_attempt(&s, &a).await?;
    let _account_lease=crate::accounts::leases::write(&s,&a.id).await;
    let auth = a.clone();
    let codes: Vec<_> = (0..10).map(|_| random_token()[..16].to_string()).collect();
    let hashed: Vec<_> = codes.iter().map(|s| hash(s)).collect();
    let ok =
        s.db.run(move |c| {
            let tx = c.transaction()?;
            let now = chrono::Utc::now().timestamp();
            let enrollment = tx.query_row("SELECT value FROM settings WHERE key=?1", [format!("totp_enrollment:{}", auth.id)], |r| r.get::<_,String>(0)).optional()?.and_then(|value| serde_json::from_str::<Value>(&value).ok()).unwrap_or(Value::Null);
            let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE hash=?1 AND user_id=?2 AND name IS NULL AND expires>?3)", params![auth.session_hash, auth.id, now], |r|r.get(0))?;
            if !active || enrollment["session"].as_str()!=Some(&auth.session_hash) || enrollment["expires"].as_i64().unwrap_or(0)<=now {return Ok(false);}
            let secret: Option<String> =
                tx.query_row("SELECT CASE WHEN totp_secret IS NULL THEN totp_pending END FROM users WHERE id=?1", [&auth.id], |r| {
                    r.get(0)
                })?;
            if let Some(secret) = secret {
                if let Some(step) = (-1..=1).map(|delta| now/30+delta).find(|step| totp(&secret,*step).as_deref()==Some(&v.code)) {
                    tx.execute(
                        "UPDATE users SET totp_secret=?1,totp_pending=NULL,recovery=?2 WHERE id=?3",
                        params![secret, serde_json::to_string(&hashed)?, auth.id],
                    )?;
                    tx.execute("DELETE FROM settings WHERE key=?1", [format!("totp_enrollment:{}", auth.id)])?;
                    tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("totp_last:{}", auth.id), step.to_string()])?;
                    tx.execute("DELETE FROM sessions WHERE user_id=?1 AND hash<>?2", params![auth.id,auth.session_hash])?;
                    tx.commit()?;
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .await?;
    if !ok {
        return Err(ApiError::bad("验证码不正确"));
    }
    account::security_success(&s, &a).await;
    Ok(Json(json!({"recovery_codes":codes})))
}
#[derive(Deserialize)]
pub struct NewToken {
    name: String,
    scopes: Vec<String>,
}
pub async fn create_token(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<NewToken>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if v.name.is_empty()
        || v.name.len() > 100
        || v.scopes.is_empty()
        || v.scopes
            .iter()
            .any(|s| !["read", "control", "library", "admin"].contains(&s.as_str()))
    {
        return Err(ApiError::bad("凭证名称或权限无效"));
    }
    let token = session(&s, a.id, Some(v.name), v.scopes, 365, None).await?;
    Ok(Json(json!({"id":hash(&token),"token":token})))
}
pub async fn list_tokens(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    if !a.interactive { a.require("admin")?; }
    let rows=s.db.run(move|c|{let mut q=c.prepare("SELECT hash,name,scopes,created_at,expires FROM sessions WHERE user_id=?1 AND name IS NOT NULL")?;let rows=q.query_map([a.id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"scopes":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or(json!([])),"created_at":r.get::<_,i64>(3)?,"expires_at":r.get::<_,i64>(4)?})))?.collect::<Result<Vec<_>,_>>()?;Ok(rows)}).await?;
    Ok(Json(json!({"items":rows})))
}
pub async fn delete_token(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    if !a.interactive { a.require("admin")?; }
    s.db.run(move |c| {
        c.execute(
            "DELETE FROM sessions WHERE hash=?1 AND user_id=?2",
            params![id, a.id],
        )?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn users(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let rows=s.db.run(|c|{let mut q=c.prepare("SELECT id,username,role,totp_secret IS NOT NULL FROM users WHERE role<>'external'")?;let v=q.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"username":r.get::<_,String>(1)?,"role":r.get::<_,String>(2)?,"totp_enabled":r.get::<_,bool>(3)?})))?.collect::<Result<Vec<_>,_>>()?;Ok(v)}).await?;
    Ok(Json(json!({"items":rows})))
}
pub async fn sessions(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    if !a.interactive { a.require("admin")?; }
    let rows = s.db.read(move |c| {
        let mut q = c.prepare("SELECT s.hash,s.created_at,s.expires,d.device_name,d.device_type
            FROM sessions s LEFT JOIN login_clients d ON d.session_hash=s.hash
            WHERE s.user_id=?1 AND s.name IS NULL AND s.expires>unixepoch() ORDER BY s.created_at DESC")?;
        let rows = q.query_map([a.id], |r| {
            let id = r.get::<_, String>(0)?;
            let name = r.get::<_, Option<String>>(3)?;
            Ok(json!({"id":id,"name":name.as_deref().unwrap_or("未标识设备"),
                "device_name":name,"device_type":r.get::<_,Option<String>>(4)?,
                "created_at":r.get::<_,i64>(1)?,"expires_at":r.get::<_,i64>(2)?,
                "current":id==a.session_hash}))
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }).await?;
    Ok(Json(json!({"items":rows})))
}
pub async fn identify_session(State(s):State<AppState>,a:Auth,Json(client):Json<LoginClient>)->ApiResult<StatusCode>{
    if !a.interactive {return Err(ApiError::forbidden());}
    let client=client.normalized().ok_or_else(||ApiError::bad("设备信息无效"))?;
    let changed=s.db.run(move|c|{
        let tx=c.transaction()?;
        let existing=tx.query_row("SELECT client_id FROM login_clients WHERE session_hash=?1",[&a.session_hash],|r|r.get::<_,String>(0)).optional()?;
        if existing.as_ref().is_some_and(|id|id!=&client.id){return Ok(false);}
        let other=tx.query_row("SELECT EXISTS(SELECT 1 FROM login_clients l JOIN sessions s ON s.hash=l.session_hash WHERE l.client_id=?1 AND s.user_id=?2 AND s.hash<>?3 AND s.expires>unixepoch())",params![client.id,a.id,a.session_hash],|r|r.get::<_,bool>(0))?;
        if other {return Ok(false);}
        tx.execute("INSERT INTO login_clients(session_hash,client_id,device_name,device_type) VALUES(?1,?2,?3,?4) ON CONFLICT(session_hash) DO UPDATE SET device_name=excluded.device_name,device_type=excluded.device_type",
            params![a.session_hash,client.id,client.name,client.kind])?;
        tx.commit()?;
        Ok(true)
    }).await?;
    if changed{Ok(StatusCode::NO_CONTENT)}else{Err(ApiError(StatusCode::CONFLICT,"login_client_in_use".into(),"该安装已有已标识会话".into()))}
}
#[derive(Deserialize)]
pub struct NewUser {
    username: String,
    password: String,
}
pub async fn create_user(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<NewUser>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    validate_credentials(&v.username, &v.password)?;
    let id = uuid::Uuid::new_v4().to_string();
    let uid = id.clone();
    let name = v.username.trim().to_string();
    let n = name.clone();
    s.db.run(move |c| {
        c.execute(
            "INSERT INTO users(id,username,password,role) VALUES(?1,?2,?3,'member')",
            params![uid, n, password_hash(&v.password)?],
        )?;
        Ok(())
    })
    .await?;
    Ok(Json(json!({"id":id,"username":name,"role":"member"})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc_totp() {
        let secret = data_encoding::BASE32_NOPAD.encode(b"12345678901234567890");
        assert_eq!(totp(&secret, 59 / 30).unwrap(), "287082");
        assert!(valid_totp(&secret, "287082", 59));
        assert!(!valid_totp(&secret, "123456", 59));
    }
}
