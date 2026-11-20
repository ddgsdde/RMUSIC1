use super::*;

#[tokio::test]
async fn account_security_password_and_admin_name_change_revoke_other_sessions() {
    let f = Fixture::new().await;
    let (_, login) = call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"admin","password":"correct-test-password"}))).await;
    let other = login["token"].as_str().unwrap();
    let (_, external) = f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"external","scopes":["admin"]}))).await;
    let payload = json!({"current_password":"correct-test-password","username":"new-admin","new_password":"new-correct-password"});
    assert_eq!(call(&f.app,"PATCH","/api/v1/auth/account",Some(external["token"].as_str().unwrap()),Some(payload.clone())).await.0,StatusCode::FORBIDDEN);
    assert_eq!(f.call("PATCH","/api/v1/auth/account",Some(json!({"current_password":"wrong-password","new_password":"new-correct-password"}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(f.call("PATCH","/api/v1/auth/account",Some(json!({"current_password":"correct-test-password","new_password":"short"}))).await.0,StatusCode::BAD_REQUEST);
    let (status, changed) = f.call("PATCH","/api/v1/auth/account",Some(payload)).await;
    assert_eq!(status,StatusCode::OK,"{changed}");
    assert_eq!(changed["user"]["username"],"new-admin");
    assert_eq!(changed["revoked_sessions"],2);
    assert_eq!(call(&f.app,"GET","/api/v1/auth/me",Some(other),None).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(call(&f.app,"GET","/api/v1/auth/me",Some(external["token"].as_str().unwrap()),None).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(f.call("GET","/api/v1/auth/me",None).await.1["username"],"new-admin");
    assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"new-admin","password":"correct-test-password"}))).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"new-admin","password":"new-correct-password"}))).await.0,StatusCode::OK);
}

#[tokio::test]
async fn account_security_member_password_optional_totp_and_recovery_are_self_owned() {
    let f = Fixture::new().await;
    f.call("POST","/api/v1/auth/users",Some(json!({"username":"family","password":"family-test-password"}))).await;
    let (_, result) = call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"family","password":"family-test-password"}))).await;
    let token = result["token"].as_str().unwrap();
    let member = |method: &'static str,path: &'static str,body|call(&f.app,method,path,Some(token),body);
    assert_eq!(member("PATCH","/api/v1/auth/account",Some(json!({"username":"admin","current_password":"family-test-password"}))).await.0,StatusCode::FORBIDDEN);
    assert_eq!(member("PATCH","/api/v1/auth/account",Some(json!({"current_password":"family-test-password","new_password":"family-new-password"}))).await.0,StatusCode::OK);
    let (status, pending) = member("POST","/api/v1/auth/totp/start",Some(json!({"current_password":"family-new-password"}))).await;
    assert_eq!(status,StatusCode::OK);
    assert_eq!(member("POST","/api/v1/auth/totp/confirm",Some(json!({"code":"wrong"}))).await.0,StatusCode::BAD_REQUEST);
    let code=auth::totp(pending["secret"].as_str().unwrap(),chrono::Utc::now().timestamp()/30).unwrap();
    let (status, codes) = member("POST","/api/v1/auth/totp/confirm",Some(json!({"code":code}))).await;
    assert_eq!(status,StatusCode::OK);
    assert_eq!(codes["recovery_codes"].as_array().unwrap().len(),10);
    assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"family","password":"family-new-password"}))).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(member("POST","/api/v1/auth/totp/disable",Some(json!({"current_password":"family-new-password","totp":"invalid"}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(member("PATCH","/api/v1/auth/account",Some(json!({"current_password":"family-new-password","new_password":"must-not-change-password"}))).await.0,StatusCode::BAD_REQUEST);
    let recovery=codes["recovery_codes"][0].as_str().unwrap();
    let (status, login) = call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"family","password":"family-new-password","totp":recovery}))).await;
    assert_eq!(status,StatusCode::OK);
    assert_eq!(member("POST","/api/v1/auth/totp/disable",Some(json!({"current_password":"family-new-password","totp":recovery}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(member("POST","/api/v1/auth/totp/disable",Some(json!({"current_password":"family-new-password","totp":codes["recovery_codes"][1]}))).await.0,StatusCode::OK);
    assert_eq!(call(&f.app,"GET","/api/v1/auth/me",Some(login["token"].as_str().unwrap()),None).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(member("GET","/api/v1/auth/me",None).await.1["totp_enabled"],false);
    assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"family","password":"family-new-password"}))).await.0,StatusCode::OK);
    assert_eq!(f.call("GET","/api/v1/auth/me",None).await.1["username"],"admin");
}

#[tokio::test]
async fn account_security_enrollment_is_session_bound_and_throttled() {
    let f = Fixture::new().await;
    let (_, pending) = f.call("POST","/api/v1/auth/totp/start",Some(json!({"current_password":"correct-test-password"}))).await;
    let (_, other) = call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"admin","password":"correct-test-password"}))).await;
    let code=auth::totp(pending["secret"].as_str().unwrap(),chrono::Utc::now().timestamp()/30).unwrap();
    assert_eq!(call(&f.app,"POST","/api/v1/auth/totp/confirm",Some(other["token"].as_str().unwrap()),Some(json!({"code":code}))).await.0,StatusCode::BAD_REQUEST);
    for _ in 0..9 {assert_eq!(f.call("POST","/api/v1/auth/totp/confirm",Some(json!({"code":"invalid"}))).await.0,StatusCode::BAD_REQUEST);}
    assert_eq!(f.call("POST","/api/v1/auth/totp/confirm",Some(json!({"code":code}))).await.0,StatusCode::TOO_MANY_REQUESTS);
}
