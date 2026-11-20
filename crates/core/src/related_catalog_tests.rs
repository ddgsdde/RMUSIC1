use super::*;
#[tokio::test]
async fn album_detail_shows_one_hundred_tracks_before_paging() {
    let f=Fixture::new().await;f.collection_tracks().await;
    let mut seed=library::get_track(&f.s,"qq-only".into()).await.unwrap();
    seed.id="example:Song000".into();seed.source="example".into();seed.recording_id=seed.id.clone();
    seed.provider_data=json!({"album_mid":"HundredAlbum","artist_mids":["Artist001"]});
    let items=(0..101).map(|i|{let mut t=seed.clone();t.id=format!("example:Song{i:03}");t.recording_id=t.id.clone();t.track_number=i+1;t}).collect::<Vec<_>>();
    let records=items.clone();f.s.db.run(move|c|{for track in records{library::upsert(c,&track,None,None,0,0,None,None)?;}Ok(())}).await.unwrap();
    let dir=f.s.config.plugin_dir.join("example");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),json!({"id":"example","name":"fixture","version":"1","capabilities":["metadata","related_catalog_v1"]}).to_string()).unwrap();
    std::fs::write(dir.join("response.json"),json!({"matched":true,"source":"example","anchor":seed,"kind":"album","collection":{"id":"HundredAlbum","name":"Album"},"artists":[{"mid":"Artist001","name":"Artist"}],"items":items,"total":101}).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);p=r['params'].get('page',1);v=json.loads(pathlib.Path(__file__).with_name('response.json').read_text());v['items']=v['items'][(p-1)*50:p*50];v['page']=p;v['next_page']=p+1 if p*50<v['total'] else None\n print(json.dumps({'id':r['id'],'result':v}),flush=True)\n").unwrap();
    let uri="/api/v1/library/catalog/album/release:example:HundredAlbum";
    let(status,first)=f.call("GET",uri,None).await;assert_eq!(status,StatusCode::OK,"{first}");
    assert_eq!(first["local"]["items"].as_array().unwrap().len(),100);assert_eq!(first["local"]["next_page"],2);
    let platform=&first["platforms"][0];assert_eq!(platform["items"].as_array().unwrap().len(),100,"{first}");assert_eq!(platform["next_page"],2);
    let(status,second)=f.call("GET",&(uri.to_owned()+"?page=2&local_page=2"),None).await;assert_eq!(status,StatusCode::OK,"{second}");
    assert_eq!(second["local"]["items"].as_array().unwrap().len(),1);assert!(second["local"]["next_page"].is_null());
    assert_eq!(second["platforms"][0]["items"].as_array().unwrap().len(),1);assert!(second["platforms"][0]["next_page"].is_null());
    f.s.db.run(|c|{c.execute("DELETE FROM tracks WHERE id='example:Song100'",[])?;Ok(())}).await.unwrap();
    let(_,hundred)=f.call("GET",&(uri.to_owned()+"?remote=false"),None).await;
    assert_eq!(hundred["local"]["items"].as_array().unwrap().len(),100);assert!(hundred["local"]["next_page"].is_null());
    f.s.plugin.stop("example").await;
}
#[tokio::test]
async fn related_catalog_checks_identity_before_import_and_never_fetches_audio(){
    let f=Fixture::new().await;f.collection_tracks().await;
    let path=f._dir.path().join("seed.flac");std::fs::write(&path,b"synthetic-no-audio").unwrap();
    f.s.db.run(move|c|{c.execute("UPDATE tracks SET path=?1 WHERE id='local-shared'",[path.to_string_lossy().to_string()])?;Ok(())}).await.unwrap();
    let seed=library::get_track(&f.s,"local-shared".into()).await.unwrap();
    let mut anchor=seed.clone();anchor.id="example:Anchor001".into();anchor.source="example".into();anchor.recording_id=anchor.id.clone();anchor.provider_data=json!({"album_mid":"Album001","artist_mids":["Artist001"]});
    let mut second=anchor.clone();second.id="example:Missing002".into();second.recording_id=second.id.clone();second.title="其他曲目".into();second.track_number=2;
    let dir=f.s.config.plugin_dir.join("example");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),json!({"id":"example","name":"fixture","version":"1","capabilities":["metadata","related_catalog_v1"]}).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);p=r['params'];v=json.loads(pathlib.Path(__file__).with_name('response.json').read_text());v['page']=p.get('page',1)\n print(json.dumps({'id':r['id'],'result':v}),flush=True)\n").unwrap();
    let mut reply=json!({"matched":true,"source":"example","anchor":anchor,"kind":"album","collection":{"id":"Album001","name":"夜航"},"artists":[{"mid":"Artist001","name":"林间"}],"items":[anchor,second],"page":1,"total":51,"next_page":2});
    std::fs::write(dir.join("response.json"),reply.to_string()).unwrap();
    let uri="/api/v1/library/tracks/local-shared/related?kind=album&artist_mid=";
    assert_eq!(call(&f.app,"GET",uri,None,None).await.0,StatusCode::UNAUTHORIZED);
    let(status,page)=f.call("GET",uri,None).await;assert_eq!(status,StatusCode::OK,"{page}");assert_eq!(page["items"][0]["id"],seed.id);assert_eq!(page["items"][1]["id"],second.id);assert_eq!(page["next_page"],2);
    assert_eq!(page["identity_basis"],"exact_tags_duration_unique_candidate");
    let(_,library_page)=f.call("GET","/api/v1/library/tracks?source=example",None).await;assert_eq!(library_page["total"],0);
    let(_,albums)=f.call("GET","/api/v1/library/albums?source=example",None).await;assert_eq!(albums["total"],0);
    let(_,home)=f.call("GET","/api/v1/library/home",None).await;let(_,visible_albums)=f.call("GET","/api/v1/library/albums",None).await;assert_eq!(home["albums"].as_array().unwrap().len() as u64,visible_albums["total"].as_u64().unwrap());
    // Two recordings can open the same release. A snapshot confirmed against another
    // anchor must be refreshed, never trusted blindly or surfaced as an identity error.
    let saved_cover=format!("/api/v1/covers/{}",crate::auth::hash("confirmed-song-cover"));
    f.s.db.run({let cover=saved_cover.clone();move|c|{c.execute("UPDATE tracks SET cover_url=?1 WHERE id='example:Missing002'",[cover])?;Ok(())}}).await.unwrap();
    let uri_anchor="/api/v1/library/tracks/example:Anchor001/related?kind=album";
    let(status,opened)=f.call("GET",uri_anchor,None).await;assert_eq!(status,StatusCode::OK,"{opened}");assert_eq!(opened["items"][1]["cover_url"],saved_cover);
    f.s.db.run(|c|{c.execute("INSERT INTO catalog_profiles(kind,entity_id,manual)VALUES('track','example:Missing002',?1)",[json!({"image_url":""}).to_string()])?;Ok(())}).await.unwrap();
    reply["anchor"]=json!(second);std::fs::write(dir.join("response.json"),reply.to_string()).unwrap();
    let(status,opened)=f.call("GET","/api/v1/library/tracks/example:Missing002/related?kind=album",None).await;assert_eq!(status,StatusCode::OK,"{opened}");assert_eq!(opened["collection"]["id"],"Album001");assert!(opened["items"][1]["cover_url"].is_null());
    reply["anchor"]=json!(anchor);std::fs::write(dir.join("response.json"),reply.to_string()).unwrap();
    let counts=f.s.db.run(|c|Ok((c.query_row("SELECT COUNT(*) FROM cache_entries",[],|r|r.get::<_,i64>(0))?,c.query_row("SELECT COUNT(*) FROM jobs WHERE kind='cache_fill'",[],|r|r.get::<_,i64>(0))?))).await.unwrap();assert_eq!(counts,(0,0));
    reply["items"][1]["id"]=json!("example:Foreign003");reply["items"][1]["provider_data"]["album_mid"]=json!("DifferentAlbum");std::fs::write(dir.join("response.json"),reply.to_string()).unwrap();
    assert_eq!(f.call("GET",&(uri.to_owned()+"&refresh=true"),None).await.0,StatusCode::BAD_REQUEST);
    assert!(library::get_track(&f.s,"example:Foreign003".into()).await.is_err());
    reply["items"]=json!([]);reply["anchor"]["provider_data"]["subtitle"]=json!("Live");std::fs::write(dir.join("response.json"),reply.to_string()).unwrap();
    assert_eq!(f.call("GET",&(uri.to_owned()+"&refresh=true"),None).await.0,StatusCode::BAD_REQUEST);
    reply["anchor"]["provider_data"]["subtitle"]=json!("");reply["kind"]=json!("artist");reply["collection"]["id"]=json!("OtherSinger");std::fs::write(dir.join("response.json"),reply.to_string()).unwrap();
    assert_eq!(f.call("GET","/api/v1/library/tracks/local-shared/related?kind=artist",None).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(library::get_track(&f.s,seed.id.clone()).await.unwrap().album,seed.album);
    f.s.plugin.stop("example").await;
}
