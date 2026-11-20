//! Session-owned routing and bounded HTTP. Dropping a request future closes its transport.
use anyhow::{bail, Context, Result};
use rmusic_bridge::connection::ConnectionProfile;
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Api {
    profile: ConnectionProfile,
    token: String,
    route: Arc<Mutex<Route>>,
    network_changed: Arc<std::sync::atomic::AtomicBool>,
    zones: Arc<rmusic_bridge::zone_cache::ZoneCache>,
    routes: tokio::sync::watch::Sender<ConnectionProfile>,
}
struct Route {
    profile: ConnectionProfile,
    http: reqwest::Client,
    checked: Instant,
    retry_after: Option<Instant>,
    refreshing: Option<tokio::task::AbortHandle>,
    closed: bool,
}
impl Api {
    pub fn transport_status(&self) -> Option<Value> {
        self.route.try_lock().ok().map(|route|rmusic_bridge::connection::remote_status(&route.profile.address))
    }
    pub fn new(
        profile: ConnectionProfile,
        selected: ConnectionProfile,
        token: String,
    ) -> Result<Self> {
        let http = selected.http()?;
        let checked = Instant::now();
        Ok(Self {
            profile,
            token,
            zones:Default::default(), routes:tokio::sync::watch::channel(selected.clone()).0,
            network_changed:Default::default(), route: Arc::new(Mutex::new(Route {
                profile: selected,
                http,
                checked,
                retry_after: None,
                refreshing: None,
                closed: false,
            })),
        })
    }
    pub async fn connection_metrics(&self,speed:bool)->Result<Value>{
        let (profile,_)=self.selected_route().await?;
        profile.connection_metrics(&self.token,speed).await
    }
    pub fn routes(&self) -> tokio::sync::watch::Receiver<ConnectionProfile> { self.routes.subscribe() }
    pub fn network_changed(&self){self.network_changed.store(true,std::sync::atomic::Ordering::Release);}
    pub(crate) async fn selected_route(&self) -> Result<(ConnectionProfile, reqwest::Client)> {
        Ok({
            let mut route = self.route.lock().await;
            anyhow::ensure!(!route.closed,"Core 会话已关闭");
            if self.network_changed.swap(false,std::sync::atomic::Ordering::AcqRel) {
                route.checked=Instant::now()-Duration::from_secs(61);
                route.retry_after=None;
                if let Some(task)=route.refreshing.take() { task.abort(); }
            }
            if !route.profile.route_fresh(route.checked) && route.refreshing.is_none()
                && route.retry_after.is_none_or(|time|Instant::now()>=time) {
                let api = self.clone();
                let task=tokio::spawn(async move { api.refresh_route().await; });
                route.refreshing = Some(task.abort_handle());
            }
            (route.profile.clone(), route.http.clone())
        })
    }
    async fn refresh_route(&self) {
        let started = Instant::now();
        let result = async {
            let mut selected = self.profile.resolve().await?;
            if selected.core_public_key.is_some() {
                match self.profile.synchronize(&selected,&self.token).await {
                    Ok(synchronized) if !selected.route_fresh(started) => selected=synchronized.resolve().await?,
                    Ok(_) => {},
                    Err(e) if crate::session::transient_http(&e) => {},
                    Err(e) => return Err(e),
                }
            }
            Ok::<_,anyhow::Error>((selected.http()?, selected))
        }.await;
        let mut route = self.route.lock().await;
        if route.closed||route.checked>started {route.refreshing=None;return;}
        let changed=result.as_ref().is_ok_and(|(_,selected)|selected.address!=route.profile.address);
        if let Ok((http, selected)) = result {
            route.http=http;
            if changed { self.routes.send_replace(selected.clone()); }
            route.profile=selected;
            route.checked=started;
            route.retry_after=None;
        } else {
            route.checked=Instant::now()-Duration::from_secs(61);
            route.retry_after=Some(Instant::now()+Duration::from_secs(1));
        }
        route.refreshing=None;
    }
    pub async fn close(&self) {
        let mut route=self.route.lock().await;
        route.closed=true;
        if let Some(task)=route.refreshing.take() {task.abort();}
    }
    pub async fn file_response(&self, path: &str) -> Result<reqwest::Response> {
        validate_path(path)?;
        let (profile, http) = self.selected_route().await?;
        let url = profile.endpoint(&format!("api/v1{path}"))?;
        profile.validate_media_url(url.as_str())?;
        let response = http
            .get(url)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(if path.starts_with("/cache/") || path.starts_with("/downloads/") { 14_400 } else { 300 }))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(super::session::HttpFailure {
                status: response.status().as_u16(),
                message: format!("Core 导出失败：{}", response.status()),
            }
            .into());
        }
        Ok(response)
    }
    pub async fn import_account_file(&self, body:Value)->Result<Value>{
        let job=body["job_id"].as_str().context("缺少导入任务")?;
        anyhow::ensure!(uuid::Uuid::parse_str(job).is_ok(),"导入任务无效");
        let path=std::path::Path::new(body["path"].as_str().context("请选择账号档案")?);
        anyhow::ensure!(path.is_absolute(),"账号档案必须是本机文件");
        let file=tokio::fs::File::open(path).await?;let metadata=file.metadata().await?;
        anyhow::ensure!(metadata.is_file()&&metadata.len()>0&&metadata.len()<=2*1024*1024*1024,"账号档案不能超过 2 GiB");
        use tokio::io::AsyncReadExt;
        let stream=futures_util::stream::try_unfold(file,|mut file|async move{let mut bytes=vec![0;64*1024];let n=file.read(&mut bytes).await?;if n==0{Ok::<_,std::io::Error>(None)}else{bytes.truncate(n);Ok(Some((bytes,file)))}});
        let(profile,http)=self.selected_route().await?;
        let response=http.put(profile.endpoint(&format!("api/v1/auth/account-imports/{job}/file"))?).bearer_auth(&self.token).header("Content-Type","application/x-ndjson").header("Content-Length",metadata.len()).body(reqwest::Body::wrap_stream(stream)).send().await?;
        let status=response.status();let value:Value=response.json().await?;
        if !status.is_success(){return Err(super::session::HttpFailure{status:status.as_u16(),message:value["error"]["message"].as_str().unwrap_or("账号档案上传失败").into()}.into());}Ok(value)
    }
    async fn load_zones(&self) -> Result<Value> {
        let mut routes=self.routes();
        let (profile,http)=self.selected_route().await?;
        tokio::select! {
            result=self.zones.load(&profile,&http,&self.token)=>result,
            _=routes.changed()=>{
                let (profile,http)=self.selected_route().await?;
                self.zones.load(&profile,&http,&self.token).await
            }
        }
    }
    pub async fn request(&self, body: Value) -> Result<Value> {
        let path = body["path"].as_str().context("missing API path")?;
        validate_path(path)?;
        let method = body["method"].as_str().unwrap_or("GET");
        if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method) {
            bail!("unsupported API method");
        }
        if path=="/zones" && method=="GET" {
            return self.load_zones().await.map(|mut value| {
                if body["json"]["queue_epoch"].is_string() && body["json"]["queue_epoch"]==value["queue_epoch"] {
                    for zone in value["items"].as_array_mut().into_iter().flatten() {
                        let id=zone["id"].as_str().unwrap_or("");
                        if body["json"]["queue_revisions"][id].is_u64() && body["json"]["queue_revisions"][id]==zone["queue_revision"] {
                            zone["queue"]=serde_json::json!([]);zone["queue_unchanged"]=serde_json::json!(true);
                        }
                    }
                }
                value
            }).map_err(|error| {
                if let Some(status)=error.downcast_ref::<reqwest::Error>().and_then(|e|e.status()) {
                    super::session::HttpFailure{status:status.as_u16(),message:format!("音区读取失败：{status}")}.into()
                } else {error}
            });
        }
        let (profile, http) = self.selected_route().await?;
        let url = profile.endpoint(&format!("api/v1{path}"))?;
        profile.validate_media_url(url.as_str())?;
        // Models / recommendations / contract checks have bounded Core deadlines above 15 s.
        let (http, timeout) = if matches!(path, "/recommendations/daily" | "/recommendations/similar") {
            (profile.http_with_read_timeout(Some(Duration::from_secs(300)))?, 300)
        } else if path.starts_with("/llm/")
            || path.ends_with("/contract-check")
            || path.ends_with("/verify")
        {
            (profile.http_with_read_timeout(Some(Duration::from_secs(55)))?, 55)
        } else {
            (http, 15)
        };
        let origin = profile.admin_write_origin(method, path, body.get("json"))?;
        let mut request = http
            .request(method.parse()?, url)
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(timeout));
        if let Some(origin) = origin { request = request.header(reqwest::header::ORIGIN, origin); }
        if let Some(json) = body.get("json") {
            request = request.json(json);
        }
        let endpoint=format!("api/v1{path}");
        let response = if method=="GET"&&rmusic_bridge::connection::latency_sensitive_read(&endpoint) {
            self.profile.interactive_read(&profile,&endpoint,&self.token,request).await
        }else{request.send().await.map(|response|(response,None)).map_err(anyhow::Error::from)};
        let mut response = match response {
            Ok((response,alternative)) => {
                if let Some(alternative)=alternative {
                    let mut route=self.route.lock().await;
                    if !route.closed&&route.profile.address==profile.address {
                        route.http=alternative.http()?;
                        self.routes.send_replace(alternative.clone());
                        route.profile=alternative;
                        route.checked=Instant::now();
                        route.retry_after=None;
                    }
                }
                response
            }
            Err(error) => {
                self.mark_route_unreachable(&profile).await;
                return Err(error);
            }
        };
        let status = response.status();
        if matches!(status.as_u16(), 408 | 502 | 503 | 504) {
            self.mark_route_unreachable(&profile).await;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= 4 * 1024 * 1024,
                "Core response exceeds 4 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(super::session::HttpFailure {
                status: status.as_u16(),
                message: serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .and_then(|v| {
                        v["error"]["message"]
                            .as_str()
                            .or_else(|| v["message"].as_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| status.to_string()),
            }
            .into());
        }
        Ok(if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)?
        })
    }
    async fn mark_route_unreachable(&self, profile: &ConnectionProfile) {
        let mut route=self.route.lock().await;
        if route.profile.address==profile.address {
            route.checked=Instant::now()-Duration::from_secs(61);
        }
    }
}
fn validate_path(path: &str) -> Result<()> {
    // Reject encoded traversal too: url joining must never escape this authenticated API.
    let route = path.split('?').next().unwrap_or("").to_ascii_lowercase();
    if !route.starts_with('/')
        || route.starts_with("//")
        || path.contains('#')
        || route.contains('\\')
        || route.contains('%')
        || route.split('/').any(|p| p == ".." || p == ".")
    {
        // Percent encoding is needed for opaque IDs, but encoded dot/slash/backslash isn't.
        if route.starts_with('/')
            && !route.starts_with("//")
            && !path.contains('#')
            && !route.contains('\\')
            && !route.split('/').any(|p| p == ".." || p == ".")
            && !route.contains("%2e")
            && !route.contains("%2f")
            && !route.contains("%5c")
            && !route.contains("%25")
        {
            return Ok(());
        }
        bail!("invalid Core API path");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn refreshing_routes_keeps_controls_available_and_starts_only_one_probe() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address=format!("http://{}",listener.local_addr().unwrap());
        let entered=Arc::new(tokio::sync::Notify::new());let release=Arc::new(tokio::sync::Notify::new());
        let (signal,gate)=(entered.clone(),release.clone());
        let server=tokio::spawn(async move {
            let(mut socket,_)=listener.accept().await.unwrap();let mut bytes=[0;2048];socket.read(&mut bytes).await.unwrap();
            signal.notify_one();gate.notified().await;
            let body=r#"{"core_id":"route-qa"}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            assert!(tokio::time::timeout(Duration::from_millis(100),listener.accept()).await.is_err(),"仅一项刷新任务");
        });
        let profile=ConnectionProfile{id:"qa".into(),name:"QA".into(),address:address.clone(),core_id:Some("route-qa".into()),core_public_key:None,certificate_sha256:None,skip_certificate_verification:false,alternatives:vec![]};
        let selected=ConnectionProfile{address:"http://127.0.0.1:1".into(),..profile.clone()};
        let api=Api::new(profile,selected.clone(),"qa".into()).unwrap();
        api.network_changed();
        let(a,b)=tokio::time::timeout(Duration::from_millis(100),async {tokio::join!(api.selected_route(),api.selected_route())}).await.unwrap();
        assert_eq!(a.unwrap().0.address,selected.address);assert_eq!(b.unwrap().0.address,selected.address);
        entered.notified().await;
        assert!(api.route.lock().await.refreshing.is_some());
        assert_eq!(api.selected_route().await.unwrap().0.address,selected.address);
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1),async {loop {
            if api.selected_route().await.unwrap().0.address==address {break;}
            tokio::time::sleep(Duration::from_millis(5)).await;
        }}).await.unwrap();
        server.await.unwrap();
    }
    #[tokio::test]
    async fn unsuccessful_route_refresh_remains_retryable_after_a_short_delay() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (status, body) in [("503 Service Unavailable", "{}"), ("200 OK", r#"{"core_id":"retry"}"#)] {
                let (mut socket, _) = listener.accept().await.unwrap();
                socket.read(&mut [0; 2048]).await.unwrap();
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let profile: ConnectionProfile = serde_json::from_value(serde_json::json!({
            "id":"reconnect","name":"QA","address":address,"core_id":"retry"
        })).unwrap();
        let old = ConnectionProfile { address: "http://127.0.0.1:1".into(), ..profile.clone() };
        let api = Api::new(profile, old, "fixture".into()).unwrap();
        api.refresh_route().await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while api.selected_route().await.unwrap().0.address != address {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        }).await.expect("first failed probe must not postpone recovery for a minute");
        server.await.unwrap();
    }
    #[tokio::test]
    async fn a_ready_route_replaces_a_pending_queue_read() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let old=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let new=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let old_address=format!("http://{}",old.local_addr().unwrap());
        let new_address=format!("http://{}",new.local_addr().unwrap());
        let (entered,waiting)=tokio::sync::oneshot::channel();
        let held=tokio::spawn(async move {
            let (mut socket,_)=old.accept().await.unwrap();let mut request=[0;4096];socket.read(&mut request).await.unwrap();
            entered.send(()).unwrap();let mut remaining=Vec::new();let _=socket.read_to_end(&mut remaining).await;
        });
        let ready=tokio::spawn(async move {
            let (mut socket,_)=new.accept().await.unwrap();let mut request=[0;4096];socket.read(&mut request).await.unwrap();
            let body=r#"{"items":[{"id":"ready"}]}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let profile:ConnectionProfile=serde_json::from_value(serde_json::json!({"id":"queue-route","name":"QA","address":old_address})).unwrap();
        let api=Api::new(profile.clone(),profile.clone(),"qa".into()).unwrap();
        let worker=api.clone();let read=tokio::spawn(async move{worker.load_zones().await});
        waiting.await.unwrap();
        {let mut route=api.route.lock().await;route.profile.address=new_address;route.http=route.profile.http().unwrap();api.routes.send_replace(route.profile.clone());}
        let result=tokio::time::timeout(Duration::from_millis(300),read).await.unwrap().unwrap().unwrap();
        assert_eq!(result["items"][0]["id"],"ready");
        ready.await.unwrap();held.await.unwrap();
    }
    #[test]
    fn only_scoped_api_paths_are_accepted() {
        for p in [
            "/zones",
            "/library/tracks/qq%3Atrack",
            "/library/tracks?q=%2F",
        ] {
            validate_path(p).unwrap();
        }
        for p in [
            "//elsewhere",
            "/../auth",
            "/%2e%2e/auth",
            "/%252e%252e/auth",
            "/a%2fb",
            "/a\\b",
        ] {
            assert!(validate_path(p).is_err(), "{p}");
        }
    }
}
