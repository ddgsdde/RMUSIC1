use super::*;

#[tokio::test]
async fn native_core_transport_encrypts_a_plain_lan_entry_and_accepts_secure_admin_settings() {
    let f = Fixture::new().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = f.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let gateway = rmusic_remote::native::open_core(
        &format!("http://{address}"),
        f.s.core_id.to_string(),
        f.s.remote.identity.public_key(),
        None,
        false,
    )
    .await
    .unwrap();
    assert!(!rmusic_remote::native::is_remote(&gateway.address));
    assert!(rmusic_remote::native::is_gateway(&gateway.address));
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let login: Value = client
        .post(format!("{}/api/v1/auth/login", gateway.address))
        .json(&json!({"username":"admin","password":"correct-test-password"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let response=client.put(format!("{}/api/v1/remote-access",gateway.address)).bearer_auth(login["token"].as_str().unwrap()).json(&json!({"expected_revision":0,"configuration":f.s.remote.configuration().await,"credentials":{"current_password":"correct-test-password"}})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let path = client
        .get(format!("{}/api/v1/status", gateway.address))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(path["core_id"], f.s.core_id.as_str());
    let closed:Value = client.get(format!("{}/api/v1/status",gateway.address))
        .header("connection","close").send().await.unwrap().error_for_status().unwrap().json().await.unwrap();
    assert_eq!(closed["core_id"],f.s.core_id.as_str(),"关闭 HTTP 连接前必须发送完整响应");
    server.abort();
    f.s.remote.cancel.cancel();
}

#[tokio::test]
async fn lan_bridge_enrolls_without_pairing_but_cannot_read_accounts_or_control_other_outputs() {
    let f = Fixture::new().await;
    async fn local_call(
        app: &Router,
        method: &str,
        path: &str,
        peer: &str,
        token: Option<&str>,
        body: Option<Value>,
        proxied: bool,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .extension(axum::extract::ConnectInfo(
                peer.parse::<std::net::SocketAddr>().unwrap(),
            ));
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        if proxied {
            request = request.header("x-forwarded-proto", "https");
        }
        let response = app
            .clone()
            .oneshot(
                request
                    .body(
                        body.map(|v| Body::from(v.to_string()))
                            .unwrap_or_else(Body::empty),
                    )
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    for (peer, proxy) in [("8.8.8.8:1000", false), ("127.0.0.1:1000", true)] {
        assert_eq!(
            local_call(
                &f.app,
                "GET",
                "/api/v1/lan/bridge-challenge",
                peer,
                None,
                None,
                proxy
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, challenge) = local_call(
        &f.app,
        "GET",
        "/api/v1/lan/bridge-challenge",
        "127.0.0.1:1000",
        None,
        None,
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let identity = rmusic_remote::identity::Identity::create();
    let secret = rmusic_remote::lan::secret();
    let endpoint_id=format!("lan-bridge:{}",crate::auth::hash(&identity.public_key()));let endpoint=endpoint_id.as_str();
    let request = identity
        .sign(rmusic_remote::lan::Enrollment {
            schema: 1,
            endpoint_id: endpoint.into(),
            core_id: f.s.core_id.to_string(),
            challenge: challenge["challenge"].as_str().unwrap().into(),
            expires: challenge["expires"].as_i64().unwrap(),
            encryption_key: rmusic_remote::lan::public_key(&secret).unwrap(),
        })
        .unwrap();
    let (status, response) = local_call(
        &f.app,
        "POST",
        "/api/v1/lan/bridges",
        "127.0.0.1:1000",
        None,
        Some(serde_json::to_value(request).unwrap()),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let signed: rmusic_remote::identity::Signed<rmusic_remote::lan::Capsule> =
        serde_json::from_value(response.clone()).unwrap();
    signed.verify(&f.s.remote.identity.public_key()).unwrap();
    let context = format!("{}/{endpoint}", f.s.core_id);
    let payload: Value =
        rmusic_remote::lan::decrypt(&secret, signed.payload, context.as_bytes()).unwrap();
    let token = payload["token"].as_str().unwrap();
    assert!(!response.to_string().contains(token));
    for path in [
        "/api/v1/auth/users",
        "/api/v1/settings",
        "/api/v1/remote-access",
        "/api/v1/library/tracks",
        "/api/v1/media/unassigned",
    ] {
        assert_eq!(
            call(&f.app, "GET", path, Some(token), None).await.0,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    let body = json!({"id":format!("bridge-{}",crate::auth::hash(endpoint)),"name":"LAN QA","platform":"linux","outputs":[{"id":endpoint,"name":"QA output","kind":"alsa","default":true}]});
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/api/v1/devices/register",
            Some(token),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut wrong = body;
    wrong["outputs"][0]["id"] = json!("another-output");
    assert_eq!(
        call(
            &f.app,
            "POST",
            "/api/v1/devices/register",
            Some(token),
            Some(wrong)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn forwarded_https_only_grants_secure_ingress_to_explicit_socket_peers() {
    let f = Fixture::new().await;
    let mut config = f.s.remote.configuration().await;
    config.core.trusted_proxy_addresses = vec!["10.0.0.2".parse().unwrap()];
    f.s.remote.save(config, 0).await.unwrap();
    let body = json!({"expected_revision":1,"configuration":f.s.remote.configuration().await,"credentials":{"current_password":"correct-test-password"}});
    for (peer, expected) in [
        ("10.0.0.3:54321", StatusCode::BAD_REQUEST),
        ("10.0.0.2:54321", StatusCode::OK),
    ] {
        let request = Request::builder()
            .method("PUT")
            .uri("/api/v1/remote-access")
            .header("authorization", format!("Bearer {}", f.token))
            .header("content-type", "application/json")
            .header("x-forwarded-proto", "https")
            .extension(axum::extract::ConnectInfo(
                peer.parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::from(body.to_string()))
            .unwrap();
        assert_eq!(
            f.app.clone().oneshot(request).await.unwrap().status(),
            expected
        );
    }
}

#[tokio::test]
async fn remote_settings_need_admin_secure_ingress_and_fresh_credentials() {
    let f = Fixture::new().await;
    assert_eq!(
        call(&f.app, "GET", "/api/v1/connection-info", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, manifest) = f.call("GET", "/api/v1/connection-info", None).await;
    assert_eq!(status, StatusCode::OK);
    let signed: rmusic_remote::identity::Signed<rmusic_remote::runtime::Manifest> =
        serde_json::from_value(manifest).unwrap();
    signed.verify(&f.s.remote.identity.public_key()).unwrap();
    let secure = f
        .app
        .clone()
        .layer(axum::Extension(crate::remote_access::SecureIngress));
    let body = json!({"expected_revision":0,"configuration":rmusic_remote::config::Configuration::default(),"credentials":{"current_password":"correct-test-password"},"secrets":{"core_cloudflare":"private-test-token"}});
    assert_eq!(
        f.call("PUT", "/api/v1/remote-access", Some(body.clone()))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let mut wrong = body.clone();
    wrong["credentials"]["current_password"] = json!("incorrect");
    assert_eq!(
        call(
            &secure,
            "PUT",
            "/api/v1/remote-access",
            Some(&f.token),
            Some(wrong)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &secure,
            "PUT",
            "/api/v1/remote-access",
            Some(&f.token),
            Some(body.clone())
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, value) = f.call("GET", "/api/v1/remote-access", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["secrets"]["core_cloudflare"], true);
    assert!(!value.to_string().contains("private-test-token"));
    assert_eq!(
        call(
            &secure,
            "PUT",
            "/api/v1/remote-access",
            Some(&f.token),
            Some(body)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let raw = std::fs::read(f.s.remote.root.join("secrets/providers.sealed")).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("private-test-token"));
}

#[tokio::test]
async fn two_https_direct_proxies_with_prefixes_are_verified() {
    let f=Fixture::new().await;
    let outer=rmusic_remote::identity::Identity::create();
    let tls=axum_server::tls_rustls::RustlsConfig::from_config(outer.tls(&f.s.core_id).unwrap());
    let client=rmusic_remote::identity::pinned_tls_client("127.0.0.1".into(),&outer.public_key()).unwrap();
    let mut urls=Vec::new();
    let mut tasks=Vec::new();
    for prefix in ["/one","/two"] {
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        urls.push(format!("https://{}{prefix}",listener.local_addr().unwrap()));
        let inner=f.app.clone();
        let app=axum::Router::new().nest(prefix,inner);
        let tls=tls.clone();
        tasks.push(tokio::spawn(async move {axum_server::from_tcp_rustls(listener,tls).unwrap().serve(app.into_make_service()).await.unwrap()}));
    }
    for (i,url) in urls.iter().enumerate() {
        let result=crate::remote_access::network::verify_url(&f.s,url,Some(client.clone())).await;
        assert_eq!(result.is_ok(),i<2,"entry {i}: {result:?}");
    }
    for task in tasks { task.abort(); }
}

#[tokio::test]
async fn network_settings_only_accept_credentials_inside_a_protected_channel() {
    let f = Fixture::new().await;
    let path = "/api/v1/remote-access";
    let config = json!({"expected_revision":0,"credentials":{"current_password":"correct-test-password"},
        "configuration":rmusic_remote::config::Configuration::default()});
    assert_eq!(f.call("GET",path,None).await.1["core_settings_transport"],"restricted");
    assert_ne!(f.call("PUT",path,Some(config.clone())).await.0,StatusCode::OK);
    let secure = f.app.clone().layer(axum::Extension(crate::remote_access::SecureIngress));
    assert_eq!(call(&secure,"PUT",path,Some(&f.token),Some(config)).await.0,StatusCode::OK);
}

#[tokio::test]
async fn manifest_watch_delivers_addition_reordering_and_removal_without_relogin() {
    use tower::ServiceExt;
    let f = Fixture::new().await;
    let runtime = &f.s.remote;
    let first_url = "https://direct.example.org/music".to_owned();
    let second_url = "https://backup.example.org/music".to_owned();
    let mut config = runtime.configuration().await;
    config.direct_urls = Some(vec![first_url.clone(), second_url.clone()]);
    let revision = runtime.save(config.clone(), 0).await.unwrap();
    let path = "/api/v1/connection-info";
    let (_, first) = f.call("GET", path, None).await;
    let mut cursor = first["payload"]["revision"].as_u64().unwrap();
    for (step, expected) in [
        vec![first_url.clone(), second_url.clone()],
        vec![second_url.clone(), first_url.clone()],
        vec![second_url.clone()],
    ].into_iter().enumerate() {
        let req = axum::http::Request::builder().uri(format!("{path}?after={cursor}"))
            .header("authorization", format!("Bearer {}", f.token)).body(axum::body::Body::empty()).unwrap();
        let reply = f.app.clone().oneshot(req).await.unwrap();
        assert_eq!(reply.headers()["x-rmusic-connection-watch"], "1");
        let mut pending = tokio::spawn(axum::body::to_bytes(reply.into_body(), 32768));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(30), &mut pending).await.is_err());
        if step == 0 {
            runtime.publish_direct_checks(revision, expected.clone()).await.unwrap();
        } else {
            config.direct_urls = Some(expected.clone());
            runtime.save(config.clone(), revision + step as u64 - 1).await.unwrap();
        }
        let bytes = tokio::time::timeout(std::time::Duration::from_secs(2), pending).await.unwrap().unwrap().unwrap();
        let signed: rmusic_remote::identity::Signed<rmusic_remote::runtime::Manifest> = serde_json::from_slice(&bytes).unwrap();
        signed.verify(&runtime.identity.public_key()).unwrap();
        assert_eq!(signed.payload.endpoints, expected);
        assert!(signed.payload.revision > cursor);
        cursor = signed.payload.revision;
    }
}

#[tokio::test]
async fn connection_measurement_requires_auth_and_confirms_exact_size() {
    use axum::http::Request;
    use tower::ServiceExt;
    let f=Fixture::new().await;
    assert_eq!(call(&f.app,"GET","/api/v1/connection-probe",None,None).await.0,StatusCode::UNAUTHORIZED);
    let ping=f.call("GET","/api/v1/connection-probe",None).await;
    assert_eq!(ping.0,StatusCode::OK);assert_eq!(ping.1["ingress"],"direct");
    let req=Request::builder().uri("/api/v1/connection-probe/payload").header("authorization",format!("Bearer {}",f.token)).body(axum::body::Body::empty()).unwrap();
    let res=f.app.clone().oneshot(req).await.unwrap();assert_eq!(res.headers()["cache-control"],"no-store, no-transform");
    let data=axum::body::to_bytes(res.into_body(),1024*1024).await.unwrap();assert_eq!(data.len(),1024*1024);
    let req=Request::builder().method("POST").uri("/api/v1/connection-probe/payload").header("authorization",format!("Bearer {}",f.token)).body(axum::body::Body::empty()).unwrap();
    assert_eq!(f.app.clone().oneshot(req).await.unwrap().status(),StatusCode::METHOD_NOT_ALLOWED);
}
