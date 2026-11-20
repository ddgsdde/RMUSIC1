use super::*;
async fn fixture()->Fixture{
    let f=Fixture::new().await;f.collection_tracks().await;
    let dir=f.s.config.plugin_dir.join("qqmusic");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("public_metadata.py"),include_str!("../../../plugins/qqmusic/public_metadata.py")).unwrap();
    std::fs::write(dir.join("manifest.json"),json!({"id":"qqmusic","name":"QQ QA","enabled":true,"version":"0.1.22","api_version":"1","capabilities":["public_metadata_v1"]}).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),r#"import sys,json
from public_metadata import dispatch
for line in sys.stdin:
 r=json.loads(line)
 try:v=dispatch(r.get('params',{})) if r['method']=='public_metadata' else {'status':'ready'};print(json.dumps({'id':r['id'],'result':v}),flush=True)
 except Exception:print(json.dumps({'id':r['id'],'error':{'code':'invalid_public_response','message':'identity mismatch'}}),flush=True)
"#).unwrap();
    f.s.db.run(|c|{let mut t=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],models::Track::from_row)?;t.provider_data=json!({"album_mid":"Abc123","artist_mids":["Singer123"]});library::upsert(c,&t,None,None,0,0,None,None)?;Ok(())}).await.unwrap();f
}
fn response(id:&str)->Value{json!({"code":0,"req":{"code":0,"data":{"basicInfo":{"albumMid":id,"desc":"Anonymous biography","publishDate":"2026-01-01"},"company":{"name":"Label"}}}})}
fn completion(hint:&Value,id:&str)->Value{json!({"id":hint["id"],"epoch":hint["epoch"],"revision":hint["revision"],"expires_at":hint["expires_at"],"response":response(id)})}
#[tokio::test]async fn anonymous_metadata_persists_without_overwriting_manual_edits_and_rejects_stale_epochs(){
    let f=fixture().await;let path="/api/v1/library/catalog/album/release:qqmusic:Abc123/public-fetch";
    let (status,hint)=f.call("GET",path,None).await;assert_eq!(status,StatusCode::OK,"{hint}");assert_eq!(hint["request"]["json"]["comm"]["uin"],0);
    assert_ne!(f.call("POST",path,Some(completion(&hint,"Wrong"))).await.0,StatusCode::OK);
    // Human overrides, including an intentionally empty cover, survive automatic fill.
    let (_,edited)=f.call("PATCH","/api/v1/library/catalog/album/release:qqmusic:Abc123/profile",Some(json!({"expected_revision":0,"fields":{"description":"Human text","image_url":""}}))).await;assert_eq!(edited["profile"]["description"],"Human text");
    let(status,done)=f.call("POST",path,Some(completion(&hint,"Abc123"))).await;assert_eq!(status,StatusCode::OK,"{done}");assert_eq!(done["saved"],true);assert_eq!(done["cache"]["profile"]["description"],"Human text");assert_eq!(done["cache"]["profile"]["image_url"],"");assert_eq!(done["cache"]["profile"]["release_date"],"2026-01-01");assert_eq!(done["cache"]["profile"]["_client_qqmusic_profile"]["description"],"Anonymous biography");
    assert_eq!(f.call("GET",path,None).await.1["fresh"],true);
    assert_eq!(f.call("POST",path,Some(completion(&hint,"Abc123"))).await.1["saved"],false);
    let(_,next)=f.call("GET",&format!("{path}?refresh=true"),None).await;
    f.s.db.run(|c|crate::provider_session::bump(c,"qqmusic")).await.unwrap();
    assert_eq!(f.call("POST",path,Some(completion(&next,"Abc123"))).await.0,StatusCode::CONFLICT);
    let stored=f.s.db.read(|c|Ok(c.query_row("SELECT automatic FROM catalog_profiles WHERE kind='album' AND entity_id='release:qqmusic:Abc123'",[],|r|r.get::<_,String>(0))?)).await.unwrap();assert!(stored.contains("Anonymous biography"));assert!(stored.contains("client_anonymous_response"));
    let(_,token)=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"reader","scopes":["read"]}))).await;
    assert_eq!(call(&f.app,"GET",path,token["token"].as_str(),None).await.0,StatusCode::FORBIDDEN);
}
#[tokio::test]async fn anonymous_images_fill_core_deduplicate_and_repair_missing_files(){
    let f=fixture().await;
    let key=f.s.db.run(|c|Ok(crate::media_assets::register(c,"https://y.gtimg.cn/music/photo_new/T002R500x500M000Abc123.jpg")?.trim_start_matches("/api/v1/artwork/").to_owned())).await.unwrap();
    let path=format!("/api/v1/artwork/{key}/public-fetch");
    let(status,hint)=f.call("GET",&path,None).await;assert_eq!(status,StatusCode::OK);assert_eq!(hint["cached"],false);
    let bytes=b"\xff\xd8\xff\xe0\xff\xd9";
    async fn put(f:&Fixture,path:&str,rev:&str,bytes:&[u8],expected:Option<&str>)->StatusCode{
        let mut builder=Request::builder().method("PUT").uri(path).header("Authorization",format!("Bearer {}",f.token)).header("x-rmusic-source-revision",rev);if let Some(hash)=expected{builder=builder.header("x-rmusic-cached-content",hash);}let request=builder.body(Body::from(bytes.to_vec())).unwrap();f.app.clone().oneshot(request).await.unwrap().status()
    }
    assert_eq!(put(&f,&path,"stale",bytes,None).await,StatusCode::BAD_REQUEST);
    assert_eq!(put(&f,&path,hint["revision"].as_str().unwrap(),bytes,None).await,StatusCode::OK);
    let file=crate::artwork_files::content_id(bytes);let saved=f.s.config.data.join("covers").join(&file);assert_eq!(std::fs::read(&saved).unwrap(),bytes);
    assert_eq!(f.call("GET",&path,None).await.1["cached"],true);
    assert_eq!(put(&f,&path,hint["revision"].as_str().unwrap(),b"invalid duplicate",None).await,StatusCode::OK);assert_eq!(std::fs::read(&saved).unwrap(),bytes);
    let changed=b"\xff\xd8\xff\xe1\xff\xd9";
    assert_eq!(put(&f,&path,hint["revision"].as_str().unwrap(),changed,Some(&file)).await,StatusCode::OK);
    let updated=f.call("GET",&path,None).await.1;assert_ne!(updated["input_sha"],file);
    assert_eq!(put(&f,&path,hint["revision"].as_str().unwrap(),bytes,Some(&file)).await,StatusCode::CONFLICT);
    std::fs::remove_file(f.s.config.data.join("covers").join(updated["cover_id"].as_str().unwrap())).unwrap();assert_eq!(f.call("GET",&path,None).await.1["cached"],false);
    assert_eq!(put(&f,&path,hint["revision"].as_str().unwrap(),bytes,None).await,StatusCode::OK);
}
