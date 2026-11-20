use super::*;
const PROVIDER:&str=r#"import json,sys,pathlib,hashlib
p=pathlib.Path(__file__).with_name('remote.json')
def track(i):
 return dict(id=i,source='example',title=i,artist='Test',album='Test',album_artist='Test',duration=30,track_number=1,disc_number=1,format='flac',sample_rate=44100,bit_depth=24,channels=2,bitrate=0,cover_url=None,recording_id=i,available=True,created_at=0,provider_data={})
def snap(s):
 ids=s['tracks'];return dict(playlist_id='remote',name=s['name'],track_ids=ids,items=[track(i) for i in ids],revision=hashlib.sha256(json.dumps(ids).encode()).hexdigest(),favorite_order=s.get('favorite_order'))
for line in sys.stdin:
 r=json.loads(line);s=json.loads(p.read_text(encoding="utf-8"));m=r['method'];q=r['params']
 if m=='health':v=dict(status='ready',authenticated=True,account_context=s['account'])
 elif m=='playlists':v=dict(items=[dict(id='remote',name=s['name'],source='example',track_count=len(s['tracks']),cover_url=None,kind='created',writable=True,write_capabilities=['add','remove'])],next_page=None)
 elif m=='playlist_tracks':v=dict(playlist_id='remote',name=s['name'],items=[track(i) for i in s['tracks']],total=len(s['tracks']),next_page=None)
 elif m=='playlist_snapshot':v=snap(s)
 elif m=='playlist_apply':
  if q['revision']!=snap(s)['revision']:print(json.dumps(dict(id=r['id'],error=dict(code='playlist_conflict',message='changed'))),flush=True);continue
  if q['action']=='add':s['tracks']+=list(i for i in q['track_ids'] if i not in s['tracks'])
  elif q['action']=='remove':s['tracks']=[i for i in s['tracks'] if i not in q['track_ids']]
  s['writes']+=1
  if s.pop('fail_after_write',False):p.write_text(json.dumps(s));print(json.dumps(dict(id=r['id'],error=dict(code='network_uncertain',message='write reply lost'))),flush=True);continue
  p.write_text(json.dumps(s));v=snap(s)
 elif m=='resolve':v=s['source']
 elif m=='recommendations':v=dict(items=[track('example:recommendation')])
 else:v={}
 print(json.dumps(dict(id=r['id'],result=v)),flush=True)
"#;
pub(super) async fn provider(f:&Fixture)->std::path::PathBuf {
    let dir=f.s.config.plugin_dir.join("example");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("provider.py"),PROVIDER).unwrap();
    std::fs::write(dir.join("manifest.json"),json!({"id":"example","capabilities":["playlists","playlist_tracks","playlist_write_v1","account_context_v1","recommendations","resolve"],"quality_options":[{"id":"clean","label":"原始"},{"id":"smaller","label":"节省"}]}).to_string()).unwrap();
    let path=dir.join("remote.json");write(&path,&json!({"account":"one","name":"隔离同步测试","tracks":["example:a","example:b","example:c"],"writes":0}));path
}
fn write(path:&std::path::Path,value:&Value){std::fs::write(path,value.to_string()).unwrap();}
fn read(path:&std::path::Path)->Value{serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()}
async fn job(f:&Fixture,kind:&str,key:&str,payload:Value)->String{
    let id=crate::background::enqueue(&f.s,kind,key,payload,0).await.unwrap();let key=id.clone();f.s.db.run(move|c|{c.execute("UPDATE jobs SET status='running' WHERE id=?1",[key])?;Ok(())}).await.unwrap();id
}
async fn run(f:&Fixture,id:&str)->anyhow::Result<bool>{
    let job=job(f,"playlist_sync",id,json!({"local_id":id})).await;
    let result=crate::playlist_sync::run_job(&f.s,&job,&json!({"local_id":id})).await;
    if result.as_ref().is_ok_and(|v|*v){crate::background::finish(&f.s,&job,"completed",None).await.unwrap();}result
}
async fn bind(f:&Fixture)->String{
    let user=f.s.db.run(|c|Ok(c.query_row("SELECT id FROM users",[],|r|r.get::<_,String>(0))?)).await.unwrap();
    let job=job(f,"playlist_directory","directory",json!({"provider":"example","user_id":user})).await;
    crate::playlist_sync::refresh_directory(&f.s,&job,&json!({"provider":"example","user_id":user})).await.unwrap();
    let(status,value)=f.call("POST","/api/v1/playlists/bind",Some(json!({"provider":"example","playlist_id":"remote"}))).await;assert_eq!(status,StatusCode::OK,"{value}");
    value["id"].as_str().unwrap().to_owned()
}
#[tokio::test]
async fn discovery_add_during_first_binding_preserves_existing_remote_tracks(){
    let f=Fixture::new().await;let path=provider(&f).await;let id=bind(&f).await;
    let mut track=crate::recommendations::select(&f.s,"example",None,vec![],1,false).await.unwrap().items.remove(0);
    track.id="example:added".into();track.recording_id=track.id.clone();f.s.db.run(move|c|crate::library::upsert(c,&track,None,None,0,0,None,None)).await.unwrap();
    assert_eq!(f.call("POST",&format!("/api/v1/playlists/{id}/tracks"),Some(json!({"track_id":"example:added"}))).await.0,StatusCode::OK);
    assert!(run(&f,&id).await.unwrap());assert_eq!(read(&path)["tracks"],json!(["example:a","example:b","example:c","example:added"]));
    f.s.plugin.stop("example").await;
}
#[tokio::test]
async fn playlist_sync_merges_changes_persists_operations_and_never_deletes_cloud_playlist(){
    let f=Fixture::new().await;let path=provider(&f).await;let id=bind(&f).await;assert!(run(&f,&id).await.unwrap());
    let(_,local)=f.call("GET",&format!("/api/v1/playlists/{id}"),None).await;assert_eq!(local["track_ids"],json!(["example:a","example:b","example:c"]));
    let mut track=local["tracks"].as_array().and_then(|t|t.first()).cloned().unwrap_or_else(||json!({}));
    if !track.is_object()||track.get("id").is_none(){let t=crate::library::get_track(&f.s,"example:a".into()).await.unwrap();track=serde_json::to_value(t).unwrap();}
    track["id"]=json!("example:d");track["recording_id"]=json!("example:d");crate::plugin::import(&f.s,"example",&json!({"items":[track]})).await.unwrap();
    assert_eq!(f.call("PUT",&format!("/api/v1/playlists/{id}"),Some(json!({"track_ids":["example:b","example:c","example:d"]}))).await.0,StatusCode::OK);
    let mut remote=read(&path);remote["tracks"]=json!(["example:a","example:b","example:c","example:e"]);write(&path,&remote);
    assert!(run(&f,&id).await.unwrap());let(_,local)=f.call("GET",&format!("/api/v1/playlists/{id}"),None).await;
    assert_eq!(local["track_ids"],json!(["example:b","example:c","example:d","example:e"]));
    assert_eq!(f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM playlist_operations WHERE status='pending'",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),0);
    let before=read(&path);assert!(run(&f,&id).await.unwrap());assert_eq!(read(&path)["writes"],before["writes"]);
    assert_eq!(f.call("DELETE",&format!("/api/v1/playlists/{id}"),None).await.0,StatusCode::NO_CONTENT);assert_eq!(read(&path),before);
    f.s.plugin.stop("example").await;
}
#[tokio::test]
async fn playlist_sync_conflicts_pause_and_account_switches_do_not_write(){
    let f=Fixture::new().await;let path=provider(&f).await;let id=bind(&f).await;run(&f,&id).await.unwrap();
    f.call("PUT",&format!("/api/v1/playlists/{id}"),Some(json!({"track_ids":["example:b","example:a","example:c"]}))).await;
    let mut remote=read(&path);remote["tracks"]=json!(["example:a","example:c","example:b"]);write(&path,&remote);assert!(!run(&f,&id).await.unwrap());
    let(_,local)=f.call("GET",&format!("/api/v1/playlists/{id}"),None).await;assert_eq!(local["sync"]["status"],"conflict");assert_eq!(read(&path)["writes"],0);
    let resolution=format!("/api/v1/playlists/{id}/sync");
    let previous=local["sync"]["conflict"]["id"].clone();
    remote["tracks"]=json!(["example:d","example:a","example:c","example:b"]);write(&path,&remote);
    let (status,error)=f.call("POST",&resolution,Some(json!({"resolve":"cloud","conflict_id":previous}))).await;
    assert_eq!(status,StatusCode::CONFLICT);assert_eq!(error["error"]["code"],"playlist_conflict_changed");
    let(_,current)=f.call("GET",&format!("/api/v1/playlists/{id}"),None).await;
    assert_eq!(current["track_ids"],local["track_ids"]);
    assert_ne!(current["sync"]["conflict"]["id"],previous);
    assert_eq!(current["sync"]["conflict"]["cloud"]["track_ids"],remote["tracks"]);
    assert_eq!(f.call("POST",&resolution,Some(json!({"resolve":"cloud","conflict_id":previous}))).await.0,StatusCode::CONFLICT);
    assert_eq!(f.call("POST",&resolution,Some(json!({"resolve":"cloud","conflict_id":current["sync"]["conflict"]["id"]}))).await.0,StatusCode::OK);run(&f,&id).await.unwrap();
    let(_,current)=f.call("GET",&format!("/api/v1/playlists/{id}"),None).await;assert_eq!(current["track_ids"],remote["tracks"]);
    let mut remote=read(&path);remote["account"]=json!("two");write(&path,&remote);assert!(run(&f,&id).await.is_err());assert_eq!(read(&path)["writes"],0);
    let(_,catalog)=f.call("GET","/api/v1/playlist-directory",None).await;assert!(catalog["items"].as_array().unwrap().is_empty());
    f.s.plugin.stop("example").await;
}

#[tokio::test]
async fn closure_playlist_sync_recovers_an_uncertain_write_after_core_restart(){
    let f=Fixture::new().await;let path=provider(&f).await;let id=bind(&f).await;run(&f,&id).await.unwrap();
    f.call("PUT",&format!("/api/v1/playlists/{id}"),Some(json!({"track_ids":["example:b","example:c"]}))).await;
    let mut remote=read(&path);remote["fail_after_write"]=json!(true);write(&path,&remote);assert!(!run(&f,&id).await.unwrap());assert_eq!(read(&path)["writes"],1);
    f.s.plugin.stop("example").await;let restarted=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();
    let app=app(restarted.clone());let(_,local)=call(&app,"GET",&format!("/api/v1/playlists/{id}"),Some(&f.token),None).await;assert_eq!(local["sync"]["status"],"conflict");
    assert_eq!(call(&app,"POST",&format!("/api/v1/playlists/{id}/sync"),Some(&f.token),Some(json!({"resolve":"cloud"}))).await.0,StatusCode::OK);
    let key=id.clone();let job=restarted.db.run(move|c|{let job:String=c.query_row("SELECT id FROM jobs WHERE kind='playlist_sync' AND cursor=?1 AND status='pending'",[key],|r|r.get(0))?;c.execute("UPDATE jobs SET status='running' WHERE id=?1",[&job])?;Ok(job)}).await.unwrap();
    assert!(crate::playlist_sync::run_job(&restarted,&job,&json!({"local_id":id})).await.unwrap());assert_eq!(read(&path)["writes"],1);
    assert_eq!(restarted.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM playlist_operations WHERE status='pending'",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),0);restarted.plugin.stop("example").await;
}

#[tokio::test]
async fn remove_during_initial_binding_is_not_lost_and_pages_are_account_isolated(){
 let f=Fixture::new().await;let path=provider(&f).await;let id=bind(&f).await;
 let(status,page)=f.call("GET","/api/v1/providers/example/playlists/remote/tracks?page=1",None).await;assert_eq!(status,StatusCode::OK);assert_eq!(page["total"],3);
 let(status,_)=f.call("POST",&format!("/api/v1/playlists/{id}/tracks"),Some(json!({"track_id":"example:a","remove":true}))).await;assert_eq!(status,StatusCode::OK);
 assert!(run(&f,&id).await.unwrap());assert_eq!(read(&path)["tracks"],json!(["example:b","example:c"]));
 let user=f.s.db.run(|c|Ok(c.query_row("SELECT id FROM users",[],|r|r.get::<_,String>(0))?)).await.unwrap();
 let cached=crate::playlist_pages::get(&f.s,&user,"example","remote",1).await.unwrap().unwrap();assert_eq!(cached["total"],2);
 f.s.db.run(|c|{c.execute("UPDATE provider_playlist_pages SET updated_at=0",[])?;Ok(())}).await.unwrap();
 let stale=crate::playlist_pages::get(&f.s,&user,"example","remote",1).await.unwrap().unwrap();assert_eq!(stale["total"],2);
 assert!(crate::playlist_pages::refresh(&f.s,&json!({"provider":"example","account":"one","user":user,"playlist":"remote","page":1})).await.unwrap());
 assert_eq!(crate::playlist_pages::get(&f.s,&user,"example","remote",1).await.unwrap().unwrap()["total"],2);
 crate::playlist_sync::record_account(&f.s,"example",&json!({"account_context":"other"}),true).await.unwrap();assert!(crate::playlist_pages::get(&f.s,&user,"example","remote",1).await.unwrap().is_none());
 f.s.plugin.stop("example").await;
}
#[tokio::test]
async fn connector_limits_are_validated_and_restored(){
 let f=Fixture::new().await;
 assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"song_fetch_concurrency":0}))).await.0,StatusCode::BAD_REQUEST);
 assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"provider_request_limits":{"lyrics_ms":0}}))).await.0,StatusCode::BAD_REQUEST);
 assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"song_fetch_concurrency":1,"provider_request_limits":{"lyrics_ms":250,"metadata_ms":500,"cooldown_seconds":15}}))).await.0,StatusCode::OK);
 let(_,settings)=f.call("GET","/api/v1/settings",None).await;assert_eq!(settings["song_fetch_concurrency"],1);assert_eq!(settings["provider_request_limits"]["lyrics_ms"],250);
 let reopened=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();assert_eq!(reopened.traffic.policy.read().unwrap().metadata_ms,500);assert_eq!(reopened.traffic.song_limit.load(std::sync::atomic::Ordering::Acquire),1);
 assert!(crate::models::Autoplay::default().enabled);
}

#[tokio::test]
async fn foreground_requests_do_not_wait_for_a_large_background_playlist(){
 let f=Fixture::new().await;provider(&f).await;let dir=f.s.config.plugin_dir.join("example");
 let manifest=dir.join("manifest.json");let mut value:Value=serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();value["capabilities"].as_array_mut().unwrap().push(json!("isolated_background_v1"));std::fs::write(manifest,value.to_string()).unwrap();
 let source=PROVIDER.replace("elif m=='playlist_snapshot':v=snap(s)","elif m=='playlist_snapshot':\n  import time\n  p.with_name('background-started').write_text('ready');time.sleep(1);v=snap(s)");std::fs::write(dir.join("provider.py"),source).unwrap();
 let state=f.s.clone();let slow=tokio::spawn(async move{crate::traffic::BACKGROUND.scope(true,state.plugin.call("example","playlist_snapshot",json!({"playlist_id":"remote"}))).await});
 tokio::time::timeout(std::time::Duration::from_secs(3),async{while !dir.join("background-started").is_file(){tokio::time::sleep(std::time::Duration::from_millis(10)).await;}}).await.unwrap();
 let health=tokio::time::timeout(std::time::Duration::from_millis(500),f.s.plugin.call("example","health",json!({}))).await.expect("前台不应等待一秒的后台歌单读取").unwrap();assert_eq!(health["authenticated"],true);assert!(!slow.is_finished());assert!(slow.await.unwrap().is_ok());f.s.plugin.stop("example").await;
}

#[tokio::test]
async fn favorites_optional_binding_merges_both_ways_and_detaches_without_deleting_contents(){
 let f=Fixture::new().await;let path=provider(&f).await;let mirror=bind(&f).await;assert!(run(&f,&mirror).await.unwrap());
 let mut track=serde_json::to_value(crate::library::get_track(&f.s,"example:a".into()).await.unwrap()).unwrap();track["id"]=json!("example:local-liked");track["recording_id"]=json!("example:local-liked");crate::plugin::import(&f.s,"example",&json!({"items":[track]})).await.unwrap();
 let(status,favorite)=f.call("PUT","/api/v1/favorites/example:local-liked",Some(json!({"liked":true}))).await;assert_eq!(status,StatusCode::OK);assert!(favorite.get("sync").is_none());let id=favorite["id"].as_str().unwrap().to_owned();
 let body=json!({"provider":"example","playlist_id":"remote"});assert_eq!(f.call("PUT","/api/v1/favorites/binding",Some(body.clone())).await.0,StatusCode::CONFLICT);
 let(status,value)=f.call("PUT","/api/v1/favorites/binding",Some(json!({"provider":"example","playlist_id":"remote","take_over":true}))).await;assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["id"],id);
 let(_,old)=f.call("GET",&format!("/api/v1/playlists/{mirror}"),None).await;assert!(old.get("sync").is_none());assert_eq!(old["track_ids"],json!(["example:a","example:b","example:c"]));
 assert!(run(&f,&id).await.unwrap());assert!(read(&path)["tracks"].as_array().unwrap().contains(&json!("example:local-liked")));assert_eq!(read(&path)["name"],"隔离同步测试");
 let(_,local)=f.call("GET","/api/v1/favorites",None).await;assert_eq!(local["name"],"我喜欢");assert_eq!(local["track_ids"].as_array().unwrap().len(),4);assert_eq!(local["sync"]["playlist_id"],"remote");
 f.call("PUT","/api/v1/favorites/example:a",Some(json!({"liked":false}))).await;assert!(run(&f,&id).await.unwrap());assert!(!read(&path)["tracks"].as_array().unwrap().contains(&json!("example:a")));
 let mut remote=read(&path);remote["tracks"]=json!(["example:b","example:c","example:local-liked","example:cloud-liked"]);write(&path,&remote);assert!(run(&f,&id).await.unwrap());
 let(_,local)=f.call("GET","/api/v1/favorites",None).await;assert_eq!(local["track_ids"],json!(["example:cloud-liked","example:c","example:b","example:local-liked"]));
 remote["tracks"]=json!(["example:local-liked","example:cloud-liked","example:b","example:c"]);write(&path,&remote);
 let writes=remote["writes"].clone();assert!(run(&f,&id).await.unwrap());assert_eq!(read(&path)["writes"],writes);
 let(_,reordered_cloud)=f.call("GET","/api/v1/favorites",None).await;assert_eq!(reordered_cloud["track_ids"],local["track_ids"]);
 let(_,page)=f.call("GET",&format!("/api/v1/playlists/{id}/tracks?limit=1&offset=1&sort=default"),None).await;assert_eq!(page["items"][0]["id"],"example:c");
 let job=job(&f,"playlist_sync",&id,json!({"local_id":id})).await;assert_eq!(f.call("DELETE","/api/v1/favorites/binding",None).await.0,StatusCode::CONFLICT);crate::background::finish(&f.s,&job,"completed",None).await.unwrap();
 let before=read(&path);assert_eq!(f.call("DELETE","/api/v1/favorites/binding",None).await.0,StatusCode::NO_CONTENT);
 let(_,unbound)=f.call("GET","/api/v1/favorites",None).await;assert_eq!(unbound["track_ids"],local["track_ids"]);assert!(unbound.get("sync").is_none());assert_eq!(read(&path),before);
 f.s.plugin.stop("example").await;
}

#[tokio::test]
async fn favorite_sync_repairs_recent_order_and_does_not_write_it_to_the_provider(){
    let f=Fixture::new().await;
    let path=provider(&f).await;
    f.collection_tracks().await;
    let _=bind(&f).await;
    let(status,favorites)=f.call("PUT","/api/v1/favorites/binding",Some(json!({
        "provider":"example","playlist_id":"remote","take_over":true
    }))).await;
    assert_eq!(status,StatusCode::OK,"{favorites}");
    let id=favorites["id"].as_str().unwrap();
    assert!(run(&f,id).await.unwrap());
    assert_eq!(f.call("PUT","/api/v1/favorites/local-shared",Some(json!({"liked":true}))).await.0,StatusCode::OK);
    let mut remote=read(&path);
    remote["favorite_order"]=json!("newest_first");
    remote["tracks"]=json!(["example:b","example:new","example:c","example:a"]);
    write(&path,&remote);
    assert!(run(&f,id).await.unwrap());
    let(_,page)=f.call("GET",&format!("/api/v1/playlists/{id}/tracks?limit=2&offset=0&sort=default"),None).await;
    assert_eq!(page["items"][0]["id"],"example:b");
    assert_eq!(page["items"][1]["id"],"local-shared");
    let(_,tail)=f.call("GET",&format!("/api/v1/playlists/{id}/tracks?limit=3&offset=2&sort=default"),None).await;
    let ids=tail["items"].as_array().unwrap().iter().map(|row|row["id"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(ids,["example:new","example:c","example:a"]);
    assert_eq!(read(&path),remote);
    f.s.plugin.stop("example").await;
}

#[tokio::test]
async fn library_redesign_home_storage_and_owned_cache_upgrade_are_safe(){
 let f=Fixture::new().await;f.collection_tracks().await;
 let mut original=crate::library::get_track(&f.s,"local-shared".into()).await.unwrap();original.recording_id="same-version".into();original.format="flac".into();original.sample_rate=44100;original.bit_depth=16;original.channels=2;
 let root=f.s.config.data.join("upgrade-cache");let low=root.join("low/song.flac");let high=root.join("high/song.flac");let user=root.join("user/song.flac");
 for p in [&low,&high,&user]{std::fs::create_dir_all(p.parent().unwrap()).unwrap();std::fs::write(p,b"unchanged-encoded-packets").unwrap();}
 let mut old=original.clone();old.id="old-local-cache".into();old.provider_data=json!({"cached_from":{"track_id":"cloud:original","source":"cloud","cache_id":"low"}});
 let mut new=old.clone();new.id="new-local-cache".into();new.bit_depth=24;new.sample_rate=96000;new.provider_data["cached_from"]["cache_id"]=json!("high");
 let mut local=original;local.id="user-file".into();local.provider_data=json!({});
 let(a,b,u,o,n,l)=(low.to_string_lossy().into_owned(),high.to_string_lossy().into_owned(),user.to_string_lossy().into_owned(),old.clone(),new.clone(),local.clone());
 f.s.db.run(move|c|{crate::library::upsert(c,&o,Some(&a),None,0,25,None,None)?;crate::library::upsert(c,&n,Some(&b),None,0,25,None,None)?;crate::library::upsert(c,&l,Some(&u),None,0,25,None,None)?;
 for(id,track,path)in[("low",o.id,a),("high",n.id,b)]{c.execute("INSERT INTO cache_entries(id,track_id,path,format,size,total_size,complete,last_access)VALUES(?1,?2,?3,'flac',25,25,1,0)",rusqlite::params![id,track,path])?;}
 Ok(())}).await.unwrap();
 let(status,home)=f.call("GET","/api/v1/library/home",None).await;assert_eq!(status,StatusCode::OK);assert!(!home["albums"].as_array().unwrap().is_empty());
 let(_,files)=f.call("GET","/api/v1/library/tracks?storage=cache",None).await;assert_eq!(files["total"],2);
 let(_,files)=f.call("GET","/api/v1/library/tracks?storage=local",None).await;assert!(files["items"].as_array().unwrap().iter().all(|t|t["provider_data"]["cached_from"]["cache_id"].is_null()));
 let(_,folders)=f.call("GET","/api/v1/library/folders?storage=cache",None).await;assert_eq!(folders["items"].as_array().unwrap().len(),2);
 let mut stopped=f.s.zones.read().await["zone-local"].clone();stopped.status="playing".into();stopped.track=Some(old.clone());f.s.save_zone(stopped.clone()).await.unwrap();
 let mut enhanced=new.clone();enhanced.provider_data["enhanced"]=json!(true);crate::cache_versions::consider(&f.s,enhanced).await.unwrap();crate::cache_versions::cleanup(&f.s).await.unwrap();assert!(low.exists());
 crate::cache_versions::consider(&f.s,new.clone()).await.unwrap();crate::cache_versions::cleanup(&f.s).await.unwrap();assert!(low.exists(),"playing version must remain intact");
 stopped.status="stopped".into();f.s.save_zone(stopped).await.unwrap();crate::cache_versions::cleanup(&f.s).await.unwrap();assert!(!low.exists());assert!(high.exists());assert!(user.exists());
 let redirected=crate::library::get_track(&f.s,old.id.clone()).await.unwrap();assert_eq!(redirected.sample_rate,96000);assert_eq!(crate::library::local_path(&f.s,old.id).await.unwrap(),high.to_string_lossy());assert_eq!(std::fs::read(high).unwrap(),b"unchanged-encoded-packets");
 let mut cloud=local.clone();cloud.id="cloud:original".into();cloud.source="cloud".into();assert_eq!(crate::catalog::prefer_queue(&f.s,vec![cloud.clone(),cloud]).await.unwrap().iter().map(|t|t.id.as_str()).collect::<Vec<_>>(),vec!["user-file","user-file"]);
 let(_,ready)=f.call("POST","/api/v1/library/tracks/user-file/cache",Some(json!({}))).await;assert_eq!(ready["status"],"ready");
}

#[tokio::test]
async fn binding_rejects_an_account_changed_since_the_controller_selected_its_playlist() {
    let f=Fixture::new().await;provider(&f).await;let original=bind(&f).await;
    let (status,value)=f.call("POST","/api/v1/playlists/bind",Some(json!({"provider":"example","playlist_id":"remote","account_context":"another-account"}))).await;
    assert_ne!(status,StatusCode::OK,"must not bind a different active account: {value}");
    let (status,_)=f.call("PUT","/api/v1/favorites/binding",Some(json!({"provider":"example","playlist_id":"remote","account_context":"another-account","take_over":true}))).await;
    assert_ne!(status,StatusCode::OK,"must not transfer a different active account");
    let (_,playlist)=f.call("GET",&format!("/api/v1/playlists/{original}"),None).await;
    assert_eq!(playlist["sync"]["account_context"],"one");
    f.s.plugin.stop("example").await;
}

#[tokio::test]
async fn completed_or_abandoned_login_releases_the_account_switch_gate() {
 let f=Fixture::new().await;
 crate::playlist_sync::switching(&f.s,"qqmusic").await.unwrap();
 crate::playlist_sync::record_account(&f.s,"qqmusic",&json!({"account_context":"anonymous","login_pending":true}),false).await.unwrap();
 async fn context(s:&crate::state::AppState)->String {s.db.read(|c|Ok(c.query_row("SELECT context FROM provider_accounts WHERE provider='qqmusic'",[],|r|r.get::<_,String>(0))?)).await.unwrap()}
 assert_eq!(context(&f.s).await,"switching");
 crate::playlist_sync::record_account(&f.s,"qqmusic",&json!({"account_context":"anonymous","login_pending":false}),false).await.unwrap();
 assert_eq!(context(&f.s).await,"anonymous");
 assert_eq!(f.call("GET","/api/v1/playlists",None).await.0,StatusCode::OK);
 assert_eq!(f.call("GET","/api/v1/favorites",None).await.0,StatusCode::OK);
 assert_eq!(f.call("GET","/api/v1/playlist-directory",None).await.0,StatusCode::OK);
}
