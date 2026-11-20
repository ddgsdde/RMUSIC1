use super::*;
use std::time::Duration;
async fn provider(f: &Fixture, id: &str) -> std::path::PathBuf {
    let dir = f.s.config.plugin_dir.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let data = f.s.config.data.join("providers").join(id);
    std::fs::create_dir_all(&data).unwrap();
    let mut template = crate::library::get_track(&f.s, "qq-only".into())
        .await
        .unwrap();
    template.id = format!("{id}:Same001");
    template.source = id.into();
    template.recording_id = template.id.clone();
    template.provider_data =
        json!({"quality_hint":{"label":"FLAC 无损","quality":"FLAC","verified":false}});
    std::fs::write(
        dir.join("track.json"),
        serde_json::to_vec(&template).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.join("manifest.json"),json!({"id":id,"name":id,"version":"0.1.0","type":"source","entry":"provider.py","capabilities":if id=="first"{vec!["search","playlists","playlist_tracks","lyrics","resolve","recommendations"]}else{vec!["search","playlists","playlist_tracks","resolve","recommendations"]}}).to_string()).unwrap();
    std::fs::write(
        data.join("test.json"),
        json!({"account":"A","delay":0}).to_string(),
    )
    .unwrap();
    std::fs::write(dir.join("provider.py"),r#"import json,os,pathlib,sys,time
root=pathlib.Path(os.environ['RMUSIC_PROVIDER_DATA_DIR']);directory=pathlib.Path(__file__).parent;provider=directory.name
for line in sys.stdin:
 q=json.loads(line);m=q['method'];s=json.loads((root/'test.json').read_text()) if (root/'test.json').exists() else {'account':'A','delay':0};t=json.loads((directory/'track.json').read_text());t['title']=provider+':'+s['account'];p=q.get('params') or {}
 if m=='manifest':v=json.loads((directory/'manifest.json').read_text())
 elif m=='contract':v={'version':1,'methods':['manifest','health','contract','search','playlists','playlist_tracks','lyrics','resolve','recommendations']}
 elif m=='health':v={'status':'ready','authenticated':True,'account_context':s['account']}
 elif m in ('search','playlist_tracks','recommendations'):
  (root/'started').write_text(s['account']);time.sleep(s.get('delay',0));t['source']='foreign' if s.get('foreign') else provider
  v={'items':[t],'total':1,'playlist_id':p.get('playlist_id'),'next_page':None}
 elif m=='playlists':
  page=p.get('page',1);v={'items':[{'id':'same-list','source':provider,'name':provider+':'+s['account'],'track_count':1,'kind':'created','cover_url':None}] if page==1 else [],'next_page':2 if page==1 and provider=='first' else None}
 elif m=='resolve':v={'url':s.get('url','https://audio.invalid/'+provider+'/'+s['account']+'.flac'),'quality':'original','format':'flac','expires_in':120,'cache_allowed':False,'download_allowed':s.get('download_allowed',False)}
 elif m=='lyrics':v={'format':'plain','text':provider+' lyrics'}
 else:v={}
 print(json.dumps({'id':q['id'],'result':v}),flush=True)
"#).unwrap();
    crate::playlist_sync::record_account(&f.s, id, &json!({"account_context":"A"}), true)
        .await
        .unwrap();
    data
}
#[tokio::test]
async fn multiple_providers_never_mix_ids_capabilities_accounts_or_late_imports() {
    let f = Fixture::new().await;
    f.collection_tracks().await;
    let first = provider(&f, "first").await;
    let second = provider(&f, "second").await;
    let (a, b) = tokio::join!(
        f.call("GET", "/api/v1/providers/first/search?q=same", None),
        f.call("GET", "/api/v1/providers/second/search?q=same", None)
    );
    assert_eq!(a.0, StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, StatusCode::OK, "{}", b.1);
    assert_eq!(a.1["items"][0]["id"], "first:Same001");
    assert_eq!(b.1["items"][0]["id"], "second:Same001");
    let (_, pages) = f
        .call("GET", "/api/v1/providers/first/playlists?page=1", None)
        .await;
    assert_eq!(pages["next_page"], 2);
    let (_, pages) = f
        .call("GET", "/api/v1/providers/second/playlists?page=1", None)
        .await;
    assert!(pages["next_page"].is_null());
    assert_eq!(
        crate::connectors::providers(&f.s, "lyrics").await,
        vec!["first"]
    );
    std::fs::write(
        first.join("test.json"),
        json!({"account":"A","delay":0.4}).to_string(),
    )
    .unwrap();
    let _ = std::fs::remove_file(first.join("started"));
    let app = f.app.clone();
    let token = f.token.clone();
    let delayed = tokio::spawn(async move {
        call(
            &app,
            "GET",
            "/api/v1/providers/first/search?q=slow",
            Some(&token),
            None,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !first.join("started").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // A -> B -> A is still a different session and cannot accept the old response.
    crate::playlist_sync::record_account(&f.s, "first", &json!({"account_context":"B"}), true)
        .await
        .unwrap();
    crate::playlist_sync::record_account(&f.s, "first", &json!({"account_context":"A"}), true)
        .await
        .unwrap();
    assert_eq!(delayed.await.unwrap().0, StatusCode::CONFLICT);
    let (status, _) = f
        .call(
            "PATCH",
            "/api/v1/plugins/first",
            Some(json!({"enabled":false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(
        f.call("GET", "/api/v1/providers/first/search?q=x", None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        f.call("GET", "/api/v1/providers/second/search?q=x", None)
            .await
            .0,
        StatusCode::OK
    );
    std::fs::write(
        second.join("test.json"),
        json!({"account":"B","foreign":true}).to_string(),
    )
    .unwrap();
    assert_eq!(
        f.call("GET", "/api/v1/providers/second/search?q=foreign", None)
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    let before =
        f.s.db
            .read(|c| Ok(crate::provider_session::stamp(c, "first")?))
            .await
            .unwrap();
    let reopened = AppState::new(f.s.db.clone(), f.s.config.clone(), String::new())
        .await
        .unwrap();
    assert_eq!(
        crate::provider_session::start(&reopened, "first")
            .await
            .unwrap(),
        before
    );
    f.s.plugin.stop_all().await;
    reopened.plugin.stop_all().await;
}

#[tokio::test]
async fn resolved_urls_follow_provider_account_and_package_generations() {
    let f=Fixture::new().await;f.collection_tracks().await;
    let first=provider(&f,"first").await;provider(&f,"second").await;
    for id in ["first","second"] {assert_eq!(f.call("GET",&format!("/api/v1/providers/{id}/search?q=same"),None).await.0,StatusCode::OK);}
    let a=crate::library::get_track(&f.s,"first:Same001".into()).await.unwrap();let b=crate::library::get_track(&f.s,"second:Same001".into()).await.unwrap();
    let resolved=crate::catalog::resolve_quality(&f.s,&a,"original").await.unwrap();assert!(resolved["url"].as_str().unwrap().contains("/first/A"));
    let resolved=crate::catalog::resolve_quality(&f.s,&b,"original").await.unwrap();assert!(resolved["url"].as_str().unwrap().contains("/second/A"));
    std::fs::write(first.join("test.json"),json!({"account":"B","delay":0}).to_string()).unwrap();
    crate::playlist_sync::record_account(&f.s,"first",&json!({"account_context":"B"}),true).await.unwrap();
    let resolved=crate::catalog::resolve_quality(&f.s,&a,"original").await.unwrap();assert!(resolved["url"].as_str().unwrap().contains("/first/B"),"cached signed URL must not cross an account epoch");
    let id="first".to_string();f.s.db.run(move|c|crate::provider_session::bump(c,&id)).await.unwrap();
    std::fs::write(first.join("test.json"),json!({"account":"package-upgraded","delay":0}).to_string()).unwrap();
    let resolved=crate::catalog::resolve_quality(&f.s,&a,"original").await.unwrap();assert!(resolved["url"].as_str().unwrap().contains("package-upgraded"));
    f.s.plugin.stop_all().await;
}

#[tokio::test]
async fn two_source_packages_install_browse_stream_disable_upgrade_and_rollback_independently() {
    use std::io::{Cursor,Write};use tower::ServiceExt;use http_body_util::BodyExt;
    let f=Fixture::new().await;f.collection_tracks().await;
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
    let mut wav=Vec::new();wav.extend(b"RIFF");wav.extend(1956u32.to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(16u32.to_le_bytes());wav.extend(1u16.to_le_bytes());wav.extend(2u16.to_le_bytes());wav.extend(48000u32.to_le_bytes());wav.extend(192000u32.to_le_bytes());wav.extend(4u16.to_le_bytes());wav.extend(16u16.to_le_bytes());wav.extend(b"data");wav.extend(1920u32.to_le_bytes());wav.extend(vec![0;1920]);
    let expected=wav.clone();let server=tokio::spawn(async move {axum::serve(listener,Router::new().route("/audio",axum::routing::get(move||{let wav=wav.clone();async move{([(axum::http::header::CONTENT_TYPE,"audio/wav")],wav)}}))).await.unwrap()});
    fn package(dir:&std::path::Path,version:&str)->Vec<u8> {
        let mut m:Value=serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        m["capabilities"].as_array_mut().unwrap().push(json!("contract_probe_v1"));m["api_version"]=json!("1");m["core_api"]=json!(">=0.1.0,<0.2.0");m["entrypoint"]=json!("provider.py");m["version"]=json!(version);
        let mut zip=zip::ZipWriter::new(Cursor::new(Vec::new()));
        for(name,bytes)in [("manifest.json",m.to_string().into_bytes()),("provider.py",std::fs::read(dir.join("provider.py")).unwrap()),("track.json",std::fs::read(dir.join("track.json")).unwrap())] {
            zip.start_file(name,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(&bytes).unwrap();
        }zip.finish().unwrap().into_inner()
    }
    async fn install(f:&Fixture,bytes:Vec<u8>){
        let response=f.app.clone().oneshot(axum::http::Request::builder().method("POST").uri("/api/v1/plugins/install").header("Authorization",format!("Bearer {}",f.token)).header("Content-Type","application/zip").body(axum::body::Body::from(bytes)).unwrap()).await.unwrap();
        let status=response.status();let body=response.into_body().collect().await.unwrap().to_bytes();assert_eq!(status,StatusCode::OK,"{}",String::from_utf8_lossy(&body));
    }
    let mut packages=Vec::new();
    for id in ["first","second"] {
        let data=provider(&f,id).await;std::fs::write(data.join("test.json"),json!({"account":"A","delay":0,"url":format!("http://{address}/audio")}).to_string()).unwrap();
        let source=f.s.config.plugin_dir.join(id);let bytes=package(&source,"0.1.0");let outside=f._dir.path().join(format!("package-{id}"));std::fs::rename(source,&outside).unwrap();
        install(&f,bytes).await;packages.push(outside);
        assert_eq!(f.call("GET",&format!("/api/v1/providers/{id}/search?q=qa"),None).await.0,StatusCode::OK);
        let(status,page)=f.call("GET",&format!("/api/v1/providers/{id}/playlists/same-list/tracks"),None).await;assert_eq!(status,StatusCode::OK,"{page}");assert_eq!(page["items"][0]["source"],id);
        let(status,recs)=f.call("GET",&format!("/api/v1/recommendations?source={id}&fallback_local=false"),None).await;
        assert_eq!(status,StatusCode::OK,"{recs}");assert_eq!(recs["items"][0]["source"],id);
        assert_eq!(f.call("POST","/api/v1/downloads",Some(json!({"track_id":format!("{id}:Same001")}))).await.0,StatusCode::BAD_REQUEST);
        let(_,ticket)=f.call("GET",&format!("/api/v1/library/tracks/{id}:Same001/ticket"),None).await;
        let response=f.app.clone().oneshot(axum::http::Request::builder().uri(format!("/{}",ticket["api_path"].as_str().unwrap())).body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);assert_eq!(response.into_body().collect().await.unwrap().to_bytes().as_ref(),expected.as_slice());
    }
    let first_data=f.s.config.data.join("providers/first/test.json");
    std::fs::write(&first_data,json!({"account":"A","delay":0,"download_allowed":true,"url":format!("http://{address}/audio")}).to_string()).unwrap();
    let(status,download)=f.call("POST","/api/v1/downloads",Some(json!({"track_id":"first:Same001"}))).await;assert_eq!(status,StatusCode::OK,"{download}");
    let did=download["id"].as_str().unwrap().to_string();
    let key=did.clone();let(job,payload)=f.s.db.read(move|c|Ok(c.query_row("SELECT id,payload FROM jobs WHERE kind='download' AND cursor=?1",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?)).await.unwrap();
    let running=job.clone();f.s.db.run(move|c|{c.execute("UPDATE jobs SET status='running' WHERE id=?1",[running])?;Ok(())}).await.unwrap();
    let payload:Value=serde_json::from_str(&payload).unwrap();assert!(crate::downloads::run_job(&f.s,&job,&payload).await.unwrap());
    let response=f.app.clone().oneshot(axum::http::Request::builder().uri(format!("/api/v1/downloads/{did}/file")).header("Authorization",format!("Bearer {}",f.token)).body(axum::body::Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(),StatusCode::OK);assert_eq!(response.into_body().collect().await.unwrap().to_bytes().as_ref(),expected.as_slice());
    let key=did.clone();f.s.db.run(move|c|{c.execute("UPDATE downloads SET status='interrupted' WHERE id=?1",[key])?;Ok(())}).await.unwrap();
    crate::playlist_sync::record_account(&f.s,"first",&json!({"account_context":"B"}),true).await.unwrap();
    assert!(crate::downloads::run_job(&f.s,&job,&payload).await.is_err(),"old download must not resume under another account");
    crate::playlist_sync::record_account(&f.s,"first",&json!({"account_context":"A"}),true).await.unwrap();
    assert_eq!(f.call("PATCH","/api/v1/plugins/first",Some(json!({"enabled":false}))).await.0,StatusCode::OK);
    assert_eq!(f.call("GET","/api/v1/providers/second/search?q=still-ready",None).await.0,StatusCode::OK);
    install(&f,package(&packages[0],"0.1.1")).await;
    let(_,listing)=f.call("GET","/api/v1/plugins",None).await;let first=listing["items"].as_array().unwrap().iter().find(|p|p["id"]=="first").unwrap();assert_eq!(first["version"],"0.1.1");assert_eq!(first["enabled"],false);
    assert_eq!(f.call("POST","/api/v1/plugins/first/rollback",Some(json!({}))).await.0,StatusCode::OK);
    let(_,listing)=f.call("GET","/api/v1/plugins",None).await;assert_eq!(listing["items"].as_array().unwrap().iter().find(|p|p["id"]=="first").unwrap()["version"],"0.1.0");
    assert_eq!(f.call("PATCH","/api/v1/plugins/first",Some(json!({"enabled":true}))).await.0,StatusCode::OK);
    assert_eq!(f.call("GET","/api/v1/providers/first/search?q=restored-account",None).await.1["items"][0]["title"],"first:A");
    f.s.plugin.stop_all().await;server.abort();
}
