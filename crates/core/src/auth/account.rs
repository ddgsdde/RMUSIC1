use super::*;

#[derive(Deserialize)]
pub struct Reauthenticate {
    current_password: String,
    totp: Option<String>,
}

#[derive(Deserialize)]
pub struct AccountUpdate {
    #[serde(flatten)]
    credentials: Reauthenticate,
    username: Option<String>,
    new_password: Option<String>,
}

// Separate from login throttling: a stolen session cannot brute-force the password/factor.
pub(crate) async fn security_attempt(s: &AppState, a: &Auth) -> ApiResult<()> {
    if !a.interactive { return Err(ApiError::forbidden()); }
    let now = chrono::Utc::now().timestamp();
    let mut attempts = s.login_attempts.lock().await;
    attempts.retain(|_, (_, timestamp)| now - *timestamp < 300);
    let entry = attempts.entry(format!("\0security:{}", a.id)).or_insert((0, now));
    if entry.0 >= 10 {
        return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "rate_limited".into(), "验证尝试过多，请 5 分钟后重试".into()));
    }
    entry.0 += 1;
    Ok(())
}

pub(crate) async fn security_success(s: &AppState, a: &Auth) {
    s.login_attempts.lock().await.remove(&format!("\0security:{}", a.id));
}

fn invalid_credentials() -> ApiError {
    ApiError::bad("当前密码或两步验证码不正确")
}

// Called inside the same writer transaction as the change. Recheck the session so an
// already-revoked concurrent request cannot overwrite a newly changed password.
pub(crate) fn verify(c: &rusqlite::Connection, a: &Auth, v: &Reauthenticate) -> anyhow::Result<bool> {
    if v.current_password.is_empty() || v.current_password.len() > 1024 || v.totp.as_ref().is_some_and(|code| code.len() > 128) { return Ok(false); }
    let now = chrono::Utc::now().timestamp();
    let active: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE hash=?1 AND user_id=?2 AND name IS NULL AND expires>?3)", params![a.session_hash, a.id, now], |r| r.get(0))?;
    if !active { return Ok(false); }
    let (password, secret, recovery): (String, Option<String>, String) = c.query_row("SELECT password,totp_secret,recovery FROM users WHERE id=?1", [&a.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    if !PasswordHash::new(&password).ok().is_some_and(|hash| Argon2::default().verify_password(v.current_password.as_bytes(), &hash).is_ok()) { return Ok(false); }
    if let Some(secret) = secret {
        let code = v.totp.as_deref().unwrap_or_default();
        if let Some(step) = (-1..=1).map(|delta| now / 30 + delta).find(|step| totp(&secret, *step).as_deref() == Some(code)) {
            let key = format!("totp_last:{}", a.id);
            let last = c.query_row("SELECT value FROM settings WHERE key=?1", [&key], |r| r.get::<_, String>(0)).optional()?.and_then(|n| n.parse::<i64>().ok()).unwrap_or(-1);
            if step <= last { return Ok(false); }
            c.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, step.to_string()])?;
        } else {
            let mut codes: Vec<String> = serde_json::from_str(&recovery)?;
            let before = codes.len();
            codes.retain(|value| value != &hash(code));
            if codes.len() == before { return Ok(false); }
            c.execute("UPDATE users SET recovery=?1 WHERE id=?2", params![serde_json::to_string(&codes)?, a.id])?;
        }
    }
    Ok(true)
}

pub async fn update(State(s): State<AppState>, a: Auth, Json(v): Json<AccountUpdate>) -> ApiResult<Json<Value>> {
    security_attempt(&s, &a).await?;
    let _account_lease=crate::accounts::leases::write(&s,&a.id).await;
    let username = v.username.map(|name| name.trim().to_string());
    if username.is_some() && a.role != "admin" { return Err(ApiError::forbidden()); }
    if username.as_ref().is_some_and(|name| name.is_empty() || name.len() > 80) { return Err(ApiError::bad("账号不能为空，且不能超过 80 字节")); }
    if let Some(password) = &v.new_password { validate_credentials(username.as_deref().unwrap_or(&a.username), password)?; }
    if username.is_none() && v.new_password.is_none() { return Err(ApiError::bad("请填写新账号或新密码")); }
    let auth = a.clone();
    let result = s.db.run(move |c| {
        let tx = c.transaction()?;
        if !verify(&tx, &auth, &v.credentials)? { return Ok(Err(invalid_credentials())); }
        let name = username.unwrap_or_else(|| auth.username.clone());
        let taken: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE username=?1 AND id<>?2)", params![name, auth.id], |r| r.get(0))?;
        if taken { return Ok(Err(ApiError::bad("此账号名称已被使用"))); }
        if let Some(password) = v.new_password {
            tx.execute("UPDATE users SET username=?1,password=?2,totp_pending=NULL WHERE id=?3", params![name, password_hash(&password)?, auth.id])?;
        } else {
            tx.execute("UPDATE users SET username=?1,totp_pending=NULL WHERE id=?2", params![name, auth.id])?;
        }
        tx.execute("DELETE FROM settings WHERE key=?1", [format!("totp_enrollment:{}", auth.id)])?;
        let revoked = tx.execute("DELETE FROM sessions WHERE user_id=?1 AND hash<>?2", params![auth.id, auth.session_hash])?;
        let enabled: bool = tx.query_row("SELECT totp_secret IS NOT NULL FROM users WHERE id=?1", [&auth.id], |r| r.get(0))?;
        tx.commit()?;
        Ok(Ok(json!({"user":{"id":auth.id,"username":name,"role":auth.role,"totp_enabled":enabled},"revoked_sessions":revoked})))
    }).await??;
    security_success(&s, &a).await;
    Ok(Json(result))
}

pub async fn start(State(s): State<AppState>, a: Auth, body: Option<Json<Reauthenticate>>) -> ApiResult<Json<Value>> {
    security_attempt(&s, &a).await?;
    let _account_lease=crate::accounts::leases::write(&s,&a.id).await;
    let Json(v) = body.ok_or_else(|| ApiError::bad("请输入当前密码"))?;
    let mut bytes = [0; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    let secret = data_encoding::BASE32_NOPAD.encode(&bytes);
    let value = secret.clone();
    let auth = a.clone();
    s.db.run(move |c| {
        let tx = c.transaction()?;
        if !verify(&tx, &auth, &v)? { return Ok(Err(invalid_credentials())); }
        let enabled: bool = tx.query_row("SELECT totp_secret IS NOT NULL FROM users WHERE id=?1", [&auth.id], |r| r.get(0))?;
        if enabled { return Ok(Err(ApiError::bad("两步验证已经启用"))); }
        tx.execute("UPDATE users SET totp_pending=?1 WHERE id=?2", params![value, auth.id])?;
        let enrollment = json!({"session":auth.session_hash,"expires":chrono::Utc::now().timestamp()+600});
        tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![format!("totp_enrollment:{}", auth.id), enrollment.to_string()])?;
        tx.commit()?;
        Ok(Ok(()))
    }).await??;
    security_success(&s, &a).await;
    let label = url::form_urlencoded::byte_serialize(format!("R-Music:{}", a.username).as_bytes()).collect::<String>();
    Ok(Json(json!({"secret":secret,"uri":format!("otpauth://totp/{label}?secret={secret}&issuer=R-Music&algorithm=SHA1&digits=6&period=30")})))
}

pub async fn disable(State(s): State<AppState>, a: Auth, Json(v): Json<Reauthenticate>) -> ApiResult<Json<Value>> {
    security_attempt(&s, &a).await?;
    let _account_lease=crate::accounts::leases::write(&s,&a.id).await;
    let auth = a.clone();
    let revoked = s.db.run(move |c| {
        let tx = c.transaction()?;
        if !verify(&tx, &auth, &v)? { return Ok(Err(invalid_credentials())); }
        tx.execute("UPDATE users SET totp_secret=NULL,totp_pending=NULL,recovery='[]' WHERE id=?1", [&auth.id])?;
        tx.execute("DELETE FROM settings WHERE key IN (?1,?2)", params![format!("totp_last:{}", auth.id), format!("totp_enrollment:{}", auth.id)])?;
        let revoked = tx.execute("DELETE FROM sessions WHERE user_id=?1 AND hash<>?2", params![auth.id, auth.session_hash])?;
        tx.commit()?;
        Ok(Ok(revoked))
    }).await??;
    security_success(&s, &a).await;
    Ok(Json(json!({"totp_enabled":false,"revoked_sessions":revoked})))
}
