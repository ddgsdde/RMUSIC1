use super::*;
#[tokio::test]
async fn prefixed_web_root_serves_the_application_index(){
    let f=Fixture::new().await;let mut state=f.s.clone();state.config.base_path="/internal-prefix".into();
    std::fs::create_dir_all(&state.config.web).unwrap();std::fs::write(state.config.web.join("index.html"),"<html>QA application</html>").unwrap();
    let app=app(state);
    for path in ["/internal-prefix/","/internal-prefix/index.html"]{
        let response=app.clone().oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(),StatusCode::OK,"{path}");
        assert_eq!(to_bytes(response.into_body(),4096).await.unwrap(),"<html>QA application</html>");
    }
}
#[tokio::test]
async fn ticket_keeps_legacy_url_and_offers_an_api_relative_reference(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;let mut state=f.s.clone();state.config.base_path="/internal-prefix".into();let app=app(state);
    let(status,ticket)=call(&app,"GET",&format!("/internal-prefix/api/v1/library/tracks/{id}/ticket"),Some(&f.token),None).await;
    assert_eq!(status,StatusCode::OK);let relative=ticket["api_path"].as_str().expect("ticket must include the API-relative media reference");
    assert!(relative.starts_with("api/v1/media/"));assert_eq!(ticket["url"],format!("/internal-prefix/{relative}"));
    let media=app.oneshot(Request::builder().uri(format!("/internal-prefix/{relative}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(media.status(),StatusCode::OK);
}
#[tokio::test]
async fn bridge_media_url_contract_is_negotiated_without_using_public_origin(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;let track=library::get_track(&f.s,id).await.unwrap();let mut state=f.s.clone();state.config.public_url=Some("https://public-unavailable.example".into());state.config.base_path="/internal-prefix".into();
    for relative in [false,true]{
        let(tx,mut rx)=tokio::sync::mpsc::channel(4);state.bridges.write().await.insert("bridge".into(),tx);
        state.endpoints.write().await.insert("bridge".into(),json!({"id":"bridge","capabilities":{"media_url_api_relative":relative}}));
        let mut zone=state.zones.read().await["zone-local"].clone();zone.endpoint_id="bridge".into();zone.track=Some(track.clone());
        zones::dispatch(&state,&zone,"play").await.unwrap();let command=rx.recv().await.unwrap();let url=command["url"].as_str().unwrap();
        assert!(url.starts_with(if relative{"api/v1/media/"}else{"https://public-unavailable.example/internal-prefix/api/v1/media/"}));
    }
}

#[tokio::test]
async fn artwork_http_head_conditional_auth_and_legacy_file_redirect_are_compatible(){
 let f=Fixture::new().await;
 let bytes=data_encoding::BASE64.decode(b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aXioAAAAASUVORK5CYII=").unwrap();
 let current=crate::artwork_files::save(&f.s,&bytes).await.unwrap();let missing=auth::hash("old-artwork");let old=missing.clone();let now=current.clone();
 let stable=f.s.db.run(move|c|{let url=crate::media_assets::register(c,"https://artwork.example/identity.png")?;let key=url.rsplit('/').next().unwrap();c.execute("UPDATE artwork_sources SET cover_id=?1 WHERE id=?2",rusqlite::params![now,key])?;c.execute("INSERT INTO artwork_versions(cover_id,source_id)VALUES(?1,?2)",rusqlite::params![old,key])?;Ok(url)}).await.unwrap();
 for path in [stable.clone(),format!("/api/v1/covers/{current}")] {
  let response=f.app.clone().oneshot(Request::builder().method("HEAD").uri(&path).header("Authorization",format!("Bearer {}",f.token)).body(Body::empty()).unwrap()).await.unwrap();
  assert_eq!(response.status(),StatusCode::OK);assert_eq!(response.headers()["content-length"],bytes.len().to_string());let etag=response.headers()["etag"].clone();assert!(to_bytes(response.into_body(),4096).await.unwrap().is_empty());
  let response=f.app.clone().oneshot(Request::builder().uri(&path).header("Authorization",format!("Bearer {}",f.token)).header("If-None-Match",etag).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::NOT_MODIFIED);assert!(to_bytes(response.into_body(),4096).await.unwrap().is_empty());
  assert_eq!(call(&f.app,"GET",&path,None,None).await.0,StatusCode::UNAUTHORIZED);
 }
 let ticket=f.s.ticket_for(format!("cover:{missing}"),45,None).await;
 let response=f.app.clone().oneshot(Request::builder().uri(format!("/api/v1/covers/{missing}?ticket={ticket}")).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::TEMPORARY_REDIRECT);let location=response.headers()["location"].to_str().unwrap();assert!(location.starts_with(&format!("/api/v1/covers/{current}?ticket=")));assert_eq!(response.headers()["cache-control"],"private, no-store");
 let next=location.split("?ticket=").nth(1).unwrap();assert!(f.s.check_ticket(next,&format!("cover:{current}")).await);assert!(!f.s.check_ticket(next,"media:unrelated").await);
 let response=f.app.clone().oneshot(Request::builder().uri(location).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);assert_eq!(to_bytes(response.into_body(),4096).await.unwrap(),bytes);
 assert!(!f.s.config.data.join("covers").join(missing).exists(),"old immutable version is never overwritten with new pixels");
}
