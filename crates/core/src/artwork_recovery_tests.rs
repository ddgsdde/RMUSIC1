use super::*;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock, Mutex,
    },
};
static REQUESTS: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(Default::default);
static SERIAL: AtomicUsize = AtomicUsize::new(0);
fn png() -> Vec<u8> {
    data_encoding::BASE64.decode(b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aXioAAAAASUVORK5CYII=").unwrap()
}
fn absent_load(url: String) -> futures_util::future::BoxFuture<'static, ApiResult<Vec<u8>>> {
    Box::pin(async move {
        *REQUESTS.lock().unwrap().entry(url.clone()).or_default()+=1;
        if url.ends_with("/provided.png") {return Ok(png());}
        Err(ApiError(StatusCode::NOT_FOUND,"artwork_source_expired".into(),"fixture".into()))
    })
}
#[tokio::test]
async fn artwork_provider_absence_stops_retries_but_manual_refresh_recovers_without_deleting_pixels() {
    let (s,dir,url)=fixture().await;
    let plugin=dir.path().join("plugins/fixture");std::fs::create_dir(&plugin).unwrap();
    std::fs::write(plugin.join("manifest.json"),serde_json::json!({"id":"fixture","enabled":true,"capabilities":["artwork_probe_v1"],"artwork_hosts":["artwork.example"]}).to_string()).unwrap();
    std::fs::write(plugin.join("reply.json"),serde_json::json!({"matched":true,"source":"fixture","url":url,"kind":"artist","id":"Artist123","name":"No photo","image_status":"unavailable","image_url":null}).to_string()).unwrap();
    std::fs::write(plugin.join("provider.py"),r#"import json,sys,os,pathlib
for line in sys.stdin:
 r=json.loads(line)
 assert os.environ.get('RMUSIC_PROVIDER_ANONYMOUS')=='1'
 v=json.loads(pathlib.Path('reply.json').read_text())
 print(json.dumps({'id':r['id'],'result':v}),flush=True)
"#).unwrap();
    let id=s.db.run({let url=url.clone();move|c|Ok(crate::media_assets::register(c,&url)?.trim_start_matches("/api/v1/artwork/").to_owned())}).await.unwrap();
    s.db.run({let id=id.clone();move |c| {c.execute("INSERT INTO catalog_profiles(kind,entity_id,automatic,manual)VALUES('artist','qa','{}',?1)",[serde_json::json!({"image_url":format!("/api/v1/artwork/{id}")}).to_string()])?;Ok(())}}).await.unwrap();
    assert_eq!(source_with(&s,&id,true,absent_load).await.err().unwrap().1,"artwork_source_expired","a human-selected source must not be replaced or marked absent by a provider");
    assert!(!crate::artwork_probe::known_absent(&s,&id).await.unwrap());
    s.db.run(|c| {c.execute("UPDATE catalog_profiles SET manual='{}' WHERE entity_id='qa'",[])?;Ok(())}).await.unwrap();
    assert_eq!(source_with(&s,&id,true,absent_load).await.err().unwrap().1,"artwork_not_provided");
    for _ in 0..3 {assert_eq!(source_with(&s,&id,false,absent_load).await.err().unwrap().1,"artwork_not_provided");}
    assert_eq!(REQUESTS.lock().unwrap()[&url],2,"known absence must not repeat CDN work");
    let auth=crate::auth::Auth{id:"qa".into(),username:"qa".into(),role:"admin".into(),totp_enabled:false,interactive:true,scopes:vec!["admin".into()],session_hash:String::new()};
    let axum::Json(status)=crate::artwork_maintenance::status(axum::extract::State(s.clone()),auth.clone(),axum::extract::Query(serde_json::from_value(serde_json::json!({"failed":true})).unwrap())).await.unwrap();
    assert_eq!(status["total"],0);
    let axum::Json(status)=crate::artwork_maintenance::status(axum::extract::State(s.clone()),auth,axum::extract::Query(serde_json::from_value(serde_json::json!({})).unwrap())).await.unwrap();
    assert_eq!(status["items"][0]["name"],"No photo");assert_eq!(status["items"][0]["state"],"unavailable");assert!(status["items"][0]["error"].is_null());
    std::fs::write(plugin.join("reply.json"),serde_json::json!({"matched":true,"source":"fixture","url":url,"kind":"artist","id":"Artist123","name":"No photo","image_url":"https://artwork.example/provided.png"}).to_string()).unwrap();
    let file=source_with(&s,&id,true,absent_load).await.unwrap();
    assert!(!crate::artwork_probe::known_absent(&s,&id).await.unwrap());
    assert_eq!(std::fs::read(dir.path().join("covers").join(&file.id)).unwrap(),png());
    assert_eq!(source_with(&s,&id,false,absent_load).await.unwrap().id,file.id);
}
fn load(url: String) -> futures_util::future::BoxFuture<'static, ApiResult<Vec<u8>>> {
    Box::pin(async move {
        *REQUESTS.lock().unwrap().entry(url.clone()).or_default() += 1;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        if url.contains("failure") {
            return Err(ApiError(
                StatusCode::BAD_GATEWAY,
                "artwork_rate_limited".into(),
                "fixture".into(),
            ));
        }
        Ok(png())
    })
}
async fn fixture() -> (crate::state::AppState, tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    std::fs::create_dir(root.join("plugins")).unwrap();
    let s = AppState::new(
        crate::db::Db::open(&root.join("db.sqlite")).unwrap(),
        crate::state::Config {
            data: root.clone(),
            web: root,
            base_path: String::new(),
            public_url: None,
            plugin_dir: dir.path().join("plugins"),
            python: "python3".into(),
        },
        String::new(),
    )
    .await
    .unwrap();
    let url = format!(
        "https://artwork.example/{}.png",
        SERIAL.fetch_add(1, Ordering::SeqCst)
    );
    (s, dir, url)
}
#[tokio::test]
async fn artwork_coalesces_missing_files_and_recovers_damage_before_conditional_response() {
    let (s, dir, url) = fixture().await;
    let handle =
        s.db.run({
            let url = url.clone();
            move |c| crate::media_assets::register(c, &url)
        })
        .await
        .unwrap();
    let id = handle.rsplit('/').next().unwrap();
    let (a, b) = tokio::join!(
        source_with(&s, id, false, load),
        source_with(&s, id, false, load)
    );
    let file = a.unwrap();
    assert_eq!(file.id, b.unwrap().id);
    assert_eq!(REQUESTS.lock().unwrap()[&url], 1);
    let content = file.id.clone();
    assert_eq!(
        std::fs::read(dir.path().join("covers").join(&content)).unwrap(),
        png()
    );
    let headers = axum::http::HeaderMap::from_iter([(
        axum::http::header::IF_NONE_MATCH,
        format!("W/\"{content}\"").parse().unwrap(),
    )]);
    assert_eq!(
        file.response(&headers, false).await.unwrap().status(),
        StatusCode::NOT_MODIFIED
    );
    std::fs::write(
        dir.path().join("covers").join(&content),
        b"\x89PNG\r\n\x1a\ndamaged",
    )
    .unwrap();
    let (a, b) = tokio::join!(
        source_with(&s, id, false, load),
        source_with(&s, id, false, load)
    );
    assert_eq!(a.unwrap().id, content);
    assert_eq!(b.unwrap().id, content);
    assert_eq!(REQUESTS.lock().unwrap()[&url], 2);
    std::fs::remove_file(dir.path().join("covers").join(&content)).unwrap();
    let file = source_with(&s, id, false, load).await.unwrap();
    assert_eq!(file.id, content);
    assert_eq!(REQUESTS.lock().unwrap()[&url], 3);
    let again =
        s.db.run({
            let url = url.clone();
            move |c| crate::media_assets::register(c, &url)
        })
        .await
        .unwrap();
    assert_eq!(
        again, handle,
        "stable handle must not become a file URL after fetch"
    );
    assert_eq!(
        file.response(&headers, false).await.unwrap().status(),
        StatusCode::NOT_MODIFIED
    );
}
#[tokio::test]
async fn artwork_failure_is_shared_and_refresh_preserves_a_good_file() {
    let (s, _dir, url) = fixture().await;
    let failure = format!("{url}?failure=1");
    let key =
        s.db.run({
            let url = failure.clone();
            move |c| {
                Ok(crate::media_assets::register(c, &url)?
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_owned())
            }
        })
        .await
        .unwrap();
    assert!(source_with(&s, &key, false, load).await.is_err());
    assert!(source_with(&s, &key, false, load).await.is_err());
    assert!(source_with(&s, &key, true, load).await.is_err());
    assert_eq!(REQUESTS.lock().unwrap()[&failure], 1);
    let cover = artwork_files::save(&s, &png()).await.unwrap();
    publish(&s, &key, &cover).await.unwrap();
    assert_eq!(
        source_with(&s, &key, true, load).await.unwrap().id,
        cover,
        "failed refresh retains good pixels"
    );
    assert_eq!(source_with(&s, &key, false, load).await.unwrap().id, cover);
    assert_eq!(REQUESTS.lock().unwrap()[&failure], 2);
}
#[test]
fn artwork_transport_rejects_internal_addresses() {
    for ip in ["127.0.0.1", "10.0.0.1", "::1", "169.254.1.1"] {
        assert!(!crate::artwork_fetch::public(ip.parse().unwrap()));
    }
    assert!(!artwork_files::complete(b"<html>"));
    assert!(artwork_files::complete(&png()));
}
#[tokio::test]
async fn artwork_shared_owner_survives_one_cancellation_and_last_drop_releases_it() {
    let (s, _dir, url) = fixture().await;
    let id =
        s.db.run({
            let u = url.clone();
            move |c| {
                Ok(crate::media_assets::register(c, &u)?
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_owned())
            }
        })
        .await
        .unwrap();
    let first = tokio::spawn({
        let s = s.clone();
        let id = id.clone();
        async move { source_with(&s, &id, false, load).await }
    });
    let second = tokio::spawn({
        let s = s.clone();
        let id = id.clone();
        async move { source_with(&s, &id, false, load).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(3)).await;
    first.abort();
    let _ = first.await;
    assert!(second.await.unwrap().is_ok());
    assert_eq!(REQUESTS.lock().unwrap()[&url], 1);
    let jobs = JOBS.lock().unwrap();
    assert!(jobs
        .iter()
        .filter(|(k, _)| k.starts_with(&s.config.data.display().to_string()))
        .all(|(_, j)| j.strong_count() == 0));
}
#[tokio::test]
async fn artwork_content_etag_survives_legacy_file_names_and_304_has_no_body() {
    use axum::http::{header, HeaderMap};
    use http_body_util::BodyExt;
    let (s, dir, _) = fixture().await;
    std::fs::create_dir(dir.path().join("covers")).unwrap();
    let bytes = png();
    let old = crate::auth::hash(&data_encoding::BASE64.encode(&bytes));
    std::fs::write(dir.path().join("covers").join(&old), &bytes).unwrap();
    let file = artwork_files::open(&s, &old).await.unwrap();
    let response = file.response(&HeaderMap::new(), true).await.unwrap();
    let version = artwork_files::content_id(&bytes);
    assert_eq!(response.headers()[header::ETAG], format!("\"{version}\""));
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        bytes
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::IF_NONE_MATCH,
        format!("\"{version}\"").parse().unwrap(),
    );
    let response = artwork_files::open(&s, &old)
        .await
        .unwrap()
        .response(&headers, true)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len(),
        0
    );
}

#[tokio::test]
async fn a_queued_image_must_not_become_a_failure_before_an_upstream_request_starts() {
    let (s, _dir, url) = fixture().await;
    let handle =
        s.db.run(move |c| crate::media_assets::register(c, &url))
            .await
            .unwrap();
    let id = handle.rsplit('/').next().unwrap().to_owned();
    let slots = SLOTS.acquire_many(10).await.unwrap();
    let copy = s.clone();
    let key = id.clone();
    let task = tokio::spawn(async move { source_with(&copy, &key, false, load).await });
    tokio::time::sleep(std::time::Duration::from_millis(4200)).await;
    let prematurely_failed = task.is_finished();
    drop(slots);
    let result = task.await.unwrap();
    assert!(!prematurely_failed,"a healthy cold image was turned into artwork_busy solely by the four-second admission deadline");
    assert!(result.is_ok(), "admitted image should load successfully");
}

#[tokio::test]
async fn artwork_validation_buffer_stays_out_of_async_stack_frames() {
    let (s, _dir, _url) = fixture().await;
    let id = "0".repeat(64);
    let validation=std::mem::size_of_val(&crate::artwork_files::open(&s,&id));
    assert!(validation<16*1024,"图片校验把固定读缓冲内联进异步状态机：{validation} bytes");
    let recovery = std::mem::size_of_val(&resolve_source(&s, &id, true, load));
    let download = std::mem::size_of_val(&download(
        &s,
        &id,
        "https://artwork.example/size.png",
        true,
        load,
    ));
    eprintln!("image validation future={validation} bytes, recovery future={recovery} bytes, download future={download} bytes");
    assert!(
        recovery < 16 * 1024,
        "图片刷新把完整平台目录请求内联进了常用恢复状态机：{recovery} bytes"
    );
    assert!(
        download < 16 * 1024,
        "图片下载包含过大的冷路径状态机：{download} bytes"
    );
}

fn expired(_url: String) -> futures_util::future::BoxFuture<'static, ApiResult<Vec<u8>>> {
    Box::pin(async {
        Err(ApiError(
            StatusCode::BAD_GATEWAY,
            "artwork_source_expired".into(),
            "fixture expired URL".into(),
        ))
    })
}
#[test]
fn artwork_refresh_job_on_default_worker_stack() {
    // A stack overflow aborts the process. Isolate this production-sized stack
    // regression from the test harness instead of relying on its larger main stack.
    const CHILD: &str = "RMUSIC_ARTWORK_STACK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        use std::process::{Command, Stdio};
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "artwork_recovery::tests::artwork_refresh_job_on_default_worker_stack",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("图片刷新默认栈验收超时");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "默认2 MiB worker栈刷新图片失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(2 * 1024 * 1024)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(Box::pin(async {
        let (s, _dir, _) = fixture().await;
        let key =
            s.db.run(|c| {
                Ok(
                    crate::media_assets::register(c, "https://127.0.0.1/blocked-fixture.png")?
                        .rsplit('/')
                        .next()
                        .unwrap()
                        .to_owned(),
                )
            })
            .await
            .unwrap();
        let id = crate::background::enqueue(
            &s,
            "artwork_refresh",
            "worker-stack-qa",
            serde_json::json!({"ids":[key],"after":0}),
            20,
        )
        .await
        .unwrap();
        crate::background::spawn(s.clone());
        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                let job = id.clone();
                let status =
                    s.db.read(move |c| {
                        Ok(
                            c.query_row("SELECT status FROM jobs WHERE id=?1", [job], |r| {
                                r.get::<_, String>(0)
                            })?,
                        )
                    })
                    .await
                    .unwrap();
                if status == "completed" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            }
        })
        .await
        .expect("刷新失败也必须收尾，不能拖垮服务");
        let k = key.clone();
        assert_eq!(
            s.db.read(move |c| Ok(c.query_row(
                "SELECT state FROM artwork_health WHERE source_id=?1",
                [k],
                |r| r.get::<_, String>(0)
            )?))
            .await
            .unwrap(),
            "failed"
        );
        // Exercise the cold expired-source branch on the same small worker too.
        let task = tokio::spawn(async move { source_with(&s, &key, true, expired).await });
        assert!(task.await.unwrap().is_err());
    }));
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
}
