#[path="catalog_round3_tests.rs"]
mod catalog_round3;
#[path="multi_provider_tests.rs"]
mod multi_provider;
#[path="next_round_tests.rs"]
mod next_round;
#[path="library_query_tests.rs"]
mod library_query;
#[path="catalog_visibility_tests.rs"]
mod catalog_visibility;
#[path="client_metadata_tests.rs"]
mod client_metadata_tests;
#[path="artist_albums_tests.rs"]
mod artist_albums;
#[path="visualization_tests.rs"]
mod visualization_tests;
#[path="music_round_tests.rs"]
mod music_round;
#[path="playlist_edit_tests.rs"]
mod playlist_edits;
#[path="core_hardening_tests.rs"]
mod core_hardening;
#[path="related_catalog_tests.rs"]
mod related_catalog;
use super::*;
#[path="external_accounts_tests.rs"]
mod external_accounts;
use axum::{body::{Body,to_bytes},http::{Request,StatusCode}};
use serde_json::{json,Value};
use tower::ServiceExt;
#[path = "nas_hardening_tests.rs"]
mod nas_hardening;
#[path="zone_snapshots_tests.rs"]
mod zone_snapshots;
struct Fixture { _dir:tempfile::TempDir, s:AppState, app:Router, token:String }

#[tokio::test]async fn offline_zone_can_save_autoplay_without_sending_audio_commands(){
 let f=Fixture::new().await;let z=crate::models::Zone::new("offline-config".into(),"离线客户端".into(),"not-registered".into());f.s.save_zone(z).await.unwrap();
 let(status,value)=f.call("PATCH","/api/v1/zones/offline-config",Some(json!({"autoplay":{"enabled":true,"source":"mixed","fallback_local":true}}))).await;
 assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["autoplay"]["enabled"],true);
 assert_eq!(f.call("PATCH","/api/v1/zones/offline-config",Some(json!({"volume":0.5}))).await.0,StatusCode::BAD_REQUEST);
}
#[path="device_tests.rs"]
mod devices;
#[path="transfer_tests.rs"]
mod transfers;
#[path="cache_repair_tests.rs"]
mod cache_repair_tests;
#[path="background_tests.rs"]
mod background;
#[path="playlist_sync_tests.rs"]
mod playlist_sync;
#[path = "source_recovery_tests.rs"]
mod source_recovery;
#[path = "host_clock_tests.rs"]
mod host_clocks;
#[path = "sync_start_tests.rs"]
mod sync_starts;
#[path = "output_group_tests.rs"]
mod output_groups;
#[path = "playback_session_tests.rs"]
mod playback_sessions;
#[path = "hqplayer_tests.rs"]
mod hqplayer;
#[path = "media_address_tests.rs"]
mod media_addresses;
#[path = "migration_tests.rs"]
mod migration;
#[path = "dsd_catalog_tests.rs"]
mod dsd_catalog;
#[path = "smart_playlist_tests.rs"]
mod smart_playlists;
#[path = "impulse_asset_tests.rs"]
mod impulse_assets;
#[path = "impulse_config_tests.rs"]
mod impulse_config;
#[path = "offline_stats_tests.rs"]
mod offline_stats;
#[path = "sequence_tests.rs"]
mod sequences;
#[tokio::test]
async fn discovery_favorites_and_atomic_playlist_add_preserve_tracks_and_covers(){
    let f=Fixture::new().await;f.collection_tracks().await;
    f.s.db.run(|c|{c.execute("UPDATE tracks SET cover_url='https://covers.test/album.jpg' WHERE id='local-shared'",[])?;Ok(())}).await.unwrap();
    let(_,empty)=f.call("GET","/api/v1/favorites",None).await;assert_eq!(empty["track_ids"],json!([]));
    for _ in 0..2{assert_eq!(f.call("PUT","/api/v1/favorites/local-shared",Some(json!({"liked":true}))).await.0,StatusCode::OK);}
    let(_,fav)=f.call("GET","/api/v1/favorites",None).await;assert_eq!(fav["track_ids"],json!(["local-shared"]));
    let(_,created)=f.call("POST","/api/v1/playlists",Some(json!({"name":"封面歌单","track_ids":[]}))).await;
    let path=format!("/api/v1/playlists/{}/tracks",created["id"].as_str().unwrap());
    let(first,second)=tokio::join!(f.call("POST",&path,Some(json!({"track_id":"local-shared"}))),f.call("POST",&path,Some(json!({"track_id":"qq-only"}))));
    assert_eq!(first.0,StatusCode::OK);assert_eq!(second.0,StatusCode::OK);
    let(_,items)=f.call("GET","/api/v1/playlists",None).await;let playlist=items["items"].as_array().unwrap().iter().find(|v|v["id"]==created["id"]).unwrap();
    assert_eq!(playlist["track_ids"].as_array().unwrap().len(),2);assert_eq!(playlist["cover_url"],"https://covers.test/album.jpg");
    f.call("PUT","/api/v1/favorites/local-shared",Some(json!({"liked":false}))).await;
    let(_,fav)=f.call("GET","/api/v1/favorites",None).await;assert_eq!(fav["track_ids"],json!([]));
    let(status,_)=call(&f.app,"GET","/api/v1/favorites",None,None).await;assert_eq!(status,StatusCode::UNAUTHORIZED);
}
#[tokio::test]async fn collections_filter_sources_and_match_their_track_counts(){
    let f=Fixture::new().await;f.collection_tracks().await;
    for(source,albums,artists)in [("",3,3),("local",1,2),("qqmusic",3,2)] {
        for(kind,field,total)in [("albums","album_id",albums),("artists","artist_id",artists)] {
            let(status,result)=f.call("GET",&format!("/api/v1/library/{kind}?source={source}"),None).await;
            assert_eq!(status,StatusCode::OK);assert_eq!(result["total"],total,"{kind} source={source}");
            for item in result["items"].as_array().unwrap(){
                let(_,tracks)=f.call("GET",&format!("/api/v1/library/tracks?source={source}&{field}={}",item["id"].as_str().unwrap()),None).await;
                assert_eq!(item["track_count"],tracks["total"],"{kind}: {item}");assert!(tracks["total"].as_u64().unwrap()>0);
            }
        }
    }
}
#[tokio::test]async fn collections_show_explicit_unknown_labels_without_changing_metadata(){
    let f=Fixture::new().await;f.collection_tracks().await;
    let(_,albums)=f.call("GET","/api/v1/library/albums?source=local",None).await;
    assert!(!albums["items"].as_array().unwrap().iter().any(|a|a["title"]=="未知专辑"));assert_eq!(albums["unassigned_total"],1);
    let(_,unknown)=f.call("GET","/api/v1/library/tracks?source=local&album_status=unassigned",None).await;
    assert_eq!(unknown["total"],1);assert_eq!(unknown["items"][0]["id"],"local-unknown");
    let(_,artists)=f.call("GET","/api/v1/library/artists?source=local",None).await;
    assert!(artists["items"].as_array().unwrap().iter().any(|a|a["name"]=="未知艺术家"));
    // This fixture has no local file path; verify label projection did not mutate stored tags.
    let(album,artist)=f.s.db.run(|c|Ok(c.query_row("SELECT album,artist FROM tracks WHERE id='local-unknown'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?)).await.unwrap();
    assert_eq!(album,"   ");assert_eq!(artist,"");
}
#[tokio::test]async fn collections_search_and_pagination_use_the_same_filters(){
    let f=Fixture::new().await;f.collection_tracks().await;
    let(_,result)=f.call("GET","/api/v1/library/albums?q=%E5%A4%9C%E8%88%AA",None).await;
    assert_eq!(result["total"],2);assert!(result["items"].as_array().unwrap().iter().all(|a|a["title"].as_str().unwrap().contains("夜航")));
    for (kind,total) in [("albums",1),("artists",2)] {
        let(_,first)=f.call("GET",&format!("/api/v1/library/{kind}?source=local&limit=1"),None).await;
        let(_,second)=f.call("GET",&format!("/api/v1/library/{kind}?source=local&limit=1&offset=1"),None).await;
        assert_eq!(first["total"],total);assert_eq!(second["total"],total);if total==1 {assert_eq!(second["items"],json!([]));}else{assert_ne!(first["items"][0]["id"],second["items"][0]["id"]);}
        assert_eq!(f.call("GET",&format!("/api/v1/library/{kind}?q={}","a".repeat(501)),None).await.0,StatusCode::BAD_REQUEST);
    }
}
#[tokio::test]async fn transition_limits_are_atomic_and_persist_on_restart(){
    let f=Fixture::new().await;
    let path="/api/v1/zones/zone-local";
    let transitions=json!({"fade_in_ms":2000,"fade_out_ms":10000});
    assert_eq!(f.call("PATCH",path,Some(json!({"transitions":transitions}))).await.0,StatusCode::OK);
    for invalid in [json!({"fade_in_ms":10001}),json!({"fade_out_ms":-1}),json!({"fade_in_ms":1.5})] {
        assert_eq!(f.call("PATCH",path,Some(json!({"transitions":invalid}))).await.0,StatusCode::BAD_REQUEST);
        assert_eq!(f.call("GET",path,None).await.1["transitions"],transitions);
    }
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();
    assert_eq!(serde_json::to_value(&restored.zones.read().await["zone-local"].transitions).unwrap(),transitions);
}
#[tokio::test]async fn core_identity_survives_restart_and_discovery_requires_authentication(){
    let f=Fixture::new().await;
    let(status,public)=call(&f.app,"GET","/api/v1/status",None,None).await;
    assert_eq!(status,StatusCode::OK);uuid::Uuid::parse_str(public["core_id"].as_str().unwrap()).unwrap();
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();
    assert_eq!(restored.core_id,f.s.core_id);
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"core_id":"other"}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(call(&f.app,"GET","/api/v1/discovery",None,None).await.0,StatusCode::UNAUTHORIZED);
    let(status,discovery)=f.call("GET","/api/v1/discovery",None).await;
    assert_eq!(status,StatusCode::OK);assert_eq!(discovery,json!({"available":false,"items":[]}));
}
impl Fixture {
    async fn collection_tracks(&self){
        let root=self.s.config.data.join("catalog-fixtures");std::fs::create_dir_all(&root).unwrap();
        self.s.db.run(move|c|{
            for(id,source,album,artist,recording,available)in [
                ("local-shared","local","夜航","林间","same-recording",true),
                ("qq-shared","qqmusic","夜航","林间","same-recording",true),
                ("qq-live","qqmusic","夜航 Live","林间","live-recording",true),
                ("qq-only","qqmusic","海岸","回声","cloud-recording",true),
                ("local-unknown","local","   ","","unknown-recording",true),
                ("local-offline","local","离线专辑","回声","cloud-recording",false),
            ]{
                let track=models::Track{id:id.into(),source:source.into(),title:format!("{album} · 验收曲"),artist:artist.into(),album:album.into(),album_artist:artist.into(),duration:30.,track_number:1,disc_number:1,format:"flac".into(),sample_rate:96000,bit_depth:24,channels:2,bitrate:0,cover_url:None,recording_id:recording.into(),available,created_at:0,integrated_lufs:None,provider_data:json!({})};
                let path=if source=="local"{let path=root.join(format!("{id}.wav"));let frames=48000u32;let mut wav=Vec::new();wav.extend(b"RIFF");wav.extend((36+frames*2).to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(16u32.to_le_bytes());wav.extend(1u16.to_le_bytes());wav.extend(1u16.to_le_bytes());wav.extend(48000u32.to_le_bytes());wav.extend(96000u32.to_le_bytes());wav.extend(2u16.to_le_bytes());wav.extend(16u16.to_le_bytes());wav.extend(b"data");wav.extend((frames*2).to_le_bytes());wav.resize(44+frames as usize*2,0);std::fs::write(&path,wav)?;Some(path.to_string_lossy().to_string())}else{None};
                library::upsert(c,&track,path.as_deref(),None,0,0,None,None)?;
            }Ok(())
        }).await.unwrap();
    }
    async fn new()->Self{
        let dir=tempfile::tempdir().unwrap();let root=dir.path().to_path_buf();for d in ["covers","downloads","cache","plugins"]{std::fs::create_dir_all(root.join(d)).unwrap();}
        let config=Config{data:root.clone(),web:root.clone(),base_path:String::new(),public_url:None,plugin_dir:root.join("plugins"),python:if cfg!(windows){"python".into()}else{"python3".into()}};
        let s=AppState::new(db::Db::open(&root.join("db.sqlite")).unwrap(),config,"bootstrap-test".into()).await.unwrap();let app=app(s.clone());
        let (status,result)=call(&app,"POST","/api/v1/setup",None,Some(json!({"bootstrap_token":"bootstrap-test","username":"admin","password":"correct-test-password"}))).await;
        assert_eq!(status,StatusCode::OK);let token=result["token"].as_str().unwrap().to_string();Self{_dir:dir,s,app,token}
    }
    async fn call(&self,method:&str,path:&str,v:Option<Value>)->(StatusCode,Value){call(&self.app,method,path,Some(&self.token),v).await}
    async fn add_audio(&self)->(String,Vec<u8>){
        let mut wav=Vec::new();let n=48000_u32;let len=n*2;wav.extend(b"RIFF");wav.extend(&(36+len).to_le_bytes());wav.extend(b"WAVEfmt ");wav.extend(&16_u32.to_le_bytes());wav.extend(&1_u16.to_le_bytes());wav.extend(&1_u16.to_le_bytes());wav.extend(&48000_u32.to_le_bytes());wav.extend(&96000_u32.to_le_bytes());wav.extend(&2_u16.to_le_bytes());wav.extend(&16_u16.to_le_bytes());wav.extend(b"data");wav.extend(&len.to_le_bytes());for i in 0..n{wav.extend(&((i%1000)as i16-500).to_le_bytes());}
        let music=self.s.config.data.join("测试专辑");std::fs::create_dir_all(&music).unwrap();std::fs::write(music.join("夜航.wav"),&wav).unwrap();std::fs::write(music.join("夜航.lrc"),"[00:00.00]晚风掠过安静的海面\n[00:00.50]月光落在远行的船").unwrap();
        assert_eq!(self.call("POST","/api/v1/library/roots",Some(json!({"path":music}))).await.0,StatusCode::OK);
        assert_eq!(self.call("POST","/api/v1/library/scan",Some(json!({}))).await.0,StatusCode::OK);
        for _ in 0..300{let (_,j)=self.call("GET","/api/v1/jobs",None).await;if j["items"][0]["status"]=="completed"{break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
        let (_,v)=self.call("GET","/api/v1/library/tracks",None).await;assert_eq!(v["total"],1);(v["items"][0]["id"].as_str().unwrap().to_string(),wav)
    }
    async fn recommendations_fixture(&self)->String {
        let(id,wav)=self.add_audio().await;let seed=library::get_track(&self.s,id.clone()).await.unwrap();
        let size=wav.len() as u64;for i in 0..6 {let mut t=seed.clone();t.id=format!("local-recommend-{i}");t.title=format!("推荐测试{i}");t.recording_id=t.id.clone();let path=self.s.config.data.join(format!("recommend-{i}.wav"));std::fs::write(&path,&wav).unwrap();self.s.db.run(move|c|library::upsert(c,&t,Some(&path.to_string_lossy()),None,0,size,None,None)).await.unwrap();}
        id
    }
}
async fn call(app:&Router,method:&str,path:&str,token:Option<&str>,value:Option<Value>)->(StatusCode,Value){
    let mut request=Request::builder().method(method).uri(path);if let Some(t)=token{request=request.header("Authorization",format!("Bearer {t}"));}
    let body=if let Some(v)=value{request=request.header("Content-Type","application/json");Body::from(v.to_string())}else{Body::empty()};let r=app.clone().oneshot(request.body(body).unwrap()).await.unwrap();let status=r.status();let bytes=to_bytes(r.into_body(),16*1024*1024).await.unwrap();let data=serde_json::from_slice(&bytes).unwrap_or(Value::Null);(status,data)
}
#[tokio::test]async fn auth_bootstrap_scopes_and_revocation(){
    let f=Fixture::new().await;
    let(track_id,_)=f.add_audio().await;
    assert_eq!(call(&f.app,"GET","/api/v1/library/tracks",None,None).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(f.call("POST","/api/v1/setup",Some(json!({"bootstrap_token":"bootstrap-test","username":"x","password":"another-password"}))).await.0,StatusCode::BAD_REQUEST);
    let (_,v)=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"ha","scopes":["read"]}))).await;let t=v["token"].as_str().unwrap();
    assert_eq!(call(&f.app,"GET","/api/v1/zones",Some(t),None).await.0,StatusCode::OK);
    assert_eq!(call(&f.app,"POST","/api/v1/library/scan",Some(t),None).await.0,StatusCode::FORBIDDEN);
    assert_eq!(call(&f.app,"POST","/api/v1/auth/totp/start",Some(t),None).await.0,StatusCode::FORBIDDEN);
    let(_,media_ticket)=call(&f.app,"GET",&format!("/api/v1/library/tracks/{track_id}/ticket"),Some(t),None).await;
    assert_eq!(f.call("DELETE",&format!("/api/v1/auth/tokens/{}",v["id"].as_str().unwrap()),None).await.0,StatusCode::NO_CONTENT);
    assert_eq!(call(&f.app,"GET","/api/v1/zones",Some(t),None).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(call(&f.app,"GET",media_ticket["url"].as_str().unwrap(),None,None).await.0,StatusCode::UNAUTHORIZED);
    assert_eq!(f.call("GET","/api/v1/unknown",None).await.0,StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn login_session_lists_devices_and_replaces_only_the_same_installation() {
    let f = Fixture::new().await;
    let (_, api_key) = f.call("POST", "/api/v1/auth/tokens", Some(json!({"name":"automation","scopes":["read"]}))).await;
    let login = |id: &str, name: &str, kind: &str| json!({
        "username":"admin","password":"correct-test-password","client":{"id":id,"name":name,"type":kind}
    });
    let (_, old) = call(&f.app, "POST", "/api/v1/auth/login", None, Some(login("mac-one","旧名称","macos"))).await;
    let (_, phone) = call(&f.app, "POST", "/api/v1/auth/login", None, Some(login("phone-one","OnePlus","android"))).await;
    let (status, mac) = call(&f.app, "POST", "/api/v1/auth/login", None, Some(login("mac-one","我的 Mac","macos"))).await;
    assert_eq!(status, StatusCode::OK);
    let mac_token = mac["token"].as_str().unwrap();
    let (_, sessions) = call(&f.app, "GET", "/api/v1/auth/sessions", Some(mac_token), None).await;
    let sessions = sessions["items"].as_array().unwrap();
    assert_eq!(sessions.len(), 3, "两个设备与历史初始化会话");
    assert!(sessions.iter().any(|s| s["device_name"] == "我的 Mac" && s["device_type"] == "macos" && s["current"] == true));
    assert!(sessions.iter().any(|s| s["device_name"] == "OnePlus" && s["device_type"] == "android"));
    assert!(sessions.iter().any(|s| s["device_name"].is_null()));
    assert_eq!(call(&f.app, "GET", "/api/v1/auth/me", old["token"].as_str(), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&f.app, "GET", "/api/v1/auth/me", phone["token"].as_str(), None).await.0, StatusCode::OK);
    assert_eq!(call(&f.app, "GET", "/api/v1/zones", api_key["token"].as_str(), None).await.0, StatusCode::OK);
}

#[tokio::test]async fn browser_endpoints_are_independent_and_progress_is_small(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;
    for browser in ["browser-first","browser-second"]{assert_eq!(f.call("POST","/api/v1/endpoints/browser",Some(json!({"id":browser,"name":browser}))).await.0,StatusCode::OK);}
    let(_,zones)=f.call("GET","/api/v1/zones",None).await;assert_eq!(zones["items"].as_array().unwrap().len(),2);
    for zone in zones["items"].as_array().unwrap(){let zid=zone["id"].as_str().unwrap();assert_eq!(f.call("PUT",&format!("/api/v1/zones/{zid}/queue"),Some(json!({"track_ids":[id]}))).await.0,StatusCode::OK);assert_eq!(f.call("POST",&format!("/api/v1/zones/{zid}/play"),Some(json!({}))).await.0,StatusCode::OK);}
    assert!(f.s.zones.read().await.values().all(|z|z.status=="playing"));
    let mut events=f.s.events.subscribe();assert_eq!(f.call("POST","/api/v1/zones/zone-local/progress",Some(json!({"position":0.2}))).await.0,StatusCode::NO_CONTENT);let event=tokio::time::timeout(std::time::Duration::from_secs(3),async {loop {let event=events.recv().await.unwrap();if event["type"]=="zone_progress" {break event;}}}).await.expect("small progress event was not published");assert!(event["data"].get("queue").is_none());
}
#[tokio::test]async fn totp_and_single_use_recovery(){
    let f=Fixture::new().await;let(_,v)=f.call("POST","/api/v1/auth/totp/start",Some(json!({"current_password":"correct-test-password"}))).await;let secret=v["secret"].as_str().unwrap();let code=auth::totp(secret,chrono::Utc::now().timestamp()/30).unwrap();
    let(status,v)=f.call("POST","/api/v1/auth/totp/confirm",Some(json!({"code":code}))).await;assert_eq!(status,StatusCode::OK);
    assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"admin","password":"correct-test-password"}))).await.0,StatusCode::UNAUTHORIZED);
    let recovery=v["recovery_codes"][0].as_str().unwrap();let payload=json!({"username":"admin","password":"correct-test-password","totp":recovery});assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(payload.clone())).await.0,StatusCode::OK);assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(payload)).await.0,StatusCode::UNAUTHORIZED);
}
#[tokio::test]async fn provider_login_forwards_wechat_and_keeps_bodyless_default_with_admin_scope(){
    let f=Fixture::new().await;let dir=f.s.config.plugin_dir.join("qqmusic");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),json!({"id":"qqmusic","login_types":["qq","wx","mobile"]}).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);pathlib.Path('requests.jsonl').open('a').write(json.dumps(r)+'\\n')\n print(json.dumps({'id':r['id'],'result':{'session_id':'fixture','status':'waiting','login_type':r['params'].get('type','qq')}}),flush=True)\n").unwrap();
    let(_,limited)=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"control-only","scopes":["read","control"]}))).await;
    assert_eq!(call(&f.app,"POST","/api/v1/providers/qqmusic/login",limited["token"].as_str(),Some(json!({"type":"wx"}))).await.0,StatusCode::FORBIDDEN);
    for bad in [json!({"type":"other"}),json!({"type":null}),json!({"type":123}),json!([])] {assert_eq!(f.call("POST","/api/v1/providers/qqmusic/login",Some(bad)).await.0,StatusCode::BAD_REQUEST);}
    assert!(!dir.join("requests.jsonl").exists());
    for (payload,expected) in [(None,"qq"),(Some(json!({})),"qq"),(Some(json!({"type":"qq"})),"qq"),(Some(json!({"type":"wx"})),"wx")] {
        let(status,response)=f.call("POST","/api/v1/providers/qqmusic/login",payload).await;assert_eq!(status,StatusCode::OK);assert_eq!(response["login_type"],expected);
    }
    let requests:Vec<Value>=std::fs::read_to_string(dir.join("requests.jsonl")).unwrap().lines().map(|line|serde_json::from_str(line).unwrap()).collect();assert_eq!(requests.len(),4);assert_eq!(requests[0]["params"],json!({}));assert_eq!(requests[3]["params"]["type"],"wx");f.s.plugin.stop("qqmusic").await;
}
#[tokio::test]async fn natural_end_accounts_final_progress_without_skip_and_rejects_stale_events(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;
    let mut track=library::get_track(&f.s,id.clone()).await.unwrap();track.duration=30.;
    let zid="zone-local";
    let mut zone=f.s.zones.read().await.get(zid).unwrap().clone();zone.queue=vec![track];f.s.save_zone(zone).await.unwrap();
    assert_eq!(f.call("POST","/api/v1/zones/zone-local/play",Some(json!({}))).await.0,StatusCode::OK);
    assert_eq!(f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({}))).await.0,StatusCode::BAD_REQUEST);
    f.call("POST","/api/v1/zones/zone-local/progress",Some(json!({"track_id":"stale-track","position":30,"status":"stopped"}))).await;
    assert_eq!(f.s.zones.read().await[zid].status,"playing");
    let(_,stale)=f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":"stale-track","position":30}))).await;assert_eq!(stale["status"],"playing");
    // Simulate the last real progress checkpoint one second before EOF, without waiting 30s.
    f.s.db.run(|c|{c.execute("UPDATE play_sessions SET play_seconds=29,last_position=29,last_report_at=?1 WHERE zone_id='zone-local' AND finished_at IS NULL",[chrono::Utc::now().timestamp_millis()-1000])?;c.execute("INSERT INTO track_stats(track_id,play_seconds) SELECT track_id,29 FROM play_sessions WHERE zone_id='zone-local' AND finished_at IS NULL",[])?;Ok(())}).await.unwrap();
    let(status,ended)=f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":id,"position":30}))).await;assert_eq!(status,StatusCode::OK);assert_eq!(ended["status"],"stopped");
    let check_id=id.clone();f.s.db.run(move|c|{let(plays,skips,seconds):(i64,i64,f64)=c.query_row("SELECT play_count,skip_count,play_seconds FROM track_stats WHERE track_id=?1",[check_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;assert_eq!(plays,1);assert_eq!(skips,0);assert_eq!(seconds,30.);let reason:String=c.query_row("SELECT finish_reason FROM play_sessions WHERE zone_id='zone-local'",[],|r|r.get(0))?;assert_eq!(reason,"ended");Ok(())}).await.unwrap();
    let(_,duplicate)=f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":id,"position":30}))).await;assert_eq!(duplicate["status"],"stopped");
}
#[tokio::test]async fn scan_search_range_lyrics_queue_and_download(){
    let f=Fixture::new().await;let(id,wav)=f.add_audio().await;
    for q in ["yehang","yh","%E5%A4%9C%E8%88%AA"]{let(status,v)=f.call("GET",&format!("/api/v1/library/tracks?q={q}"),None).await;assert_eq!(status,StatusCode::OK);assert_eq!(v["total"],1,"query {q}");}
    let(_,lyric)=f.call("GET",&format!("/api/v1/library/tracks/{id}/lyrics"),None).await;assert!(lyric["text"].as_str().unwrap().contains("晚风"));
    let(_,ticket)=f.call("GET",&format!("/api/v1/library/tracks/{id}/ticket"),None).await;let url=ticket["url"].as_str().unwrap();
    let response=f.app.clone().oneshot(Request::builder().uri(url).header("Range","bytes=100-999").body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::PARTIAL_CONTENT);assert_eq!(response.headers()["content-range"],format!("bytes 100-999/{}",wav.len()));assert_eq!(&to_bytes(response.into_body(),1000).await.unwrap()[..],&wav[100..1000]);
    assert_eq!(f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":[id]}))).await.0,StatusCode::OK);
    assert_eq!(f.call("POST","/api/v1/zones/zone-local/play",Some(json!({"queue_index":0}))).await.0,StatusCode::OK);
    let(_,z)=f.call("POST","/api/v1/zones/zone-local/seek",Some(json!({"position":0.5}))).await;assert_eq!(z["position"],0.5);
    let(_,z)=f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":[id]}))).await;assert_eq!(z["position"],0.5);assert_eq!(z["status"],"playing");
    let(_,d)=f.call("POST","/api/v1/downloads",Some(json!({"track_id":id}))).await;let did=d["id"].as_str().unwrap();for _ in 0..100{let(_,v)=f.call("GET","/api/v1/downloads",None).await;if v["items"][0]["status"]=="complete"{break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
    let response=f.app.clone().oneshot(Request::builder().uri(format!("/api/v1/downloads/{did}/file")).header("Authorization",format!("Bearer {}",f.token)).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);assert_eq!(&to_bytes(response.into_body(),200000).await.unwrap()[..],&wav);
    let(_,v)=f.call("GET","/api/v1/openapi.json",None).await;assert_eq!(v["openapi"],"3.1.0");assert!(v["paths"].as_object().unwrap().len()>40);
}
#[tokio::test]async fn subpath_routes(){
    let f=Fixture::new().await;let mut s=f.s.clone();s.config.base_path="/music".into();let a=app(s);assert_eq!(call(&a,"GET","/music/api/v1/status",None,None).await.0,StatusCode::OK);assert_eq!(call(&a,"GET","/api/v1/status",None,None).await.0,StatusCode::NOT_FOUND);let response=a.oneshot(Request::builder().uri("/music").body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::PERMANENT_REDIRECT);assert_eq!(response.headers()["location"],"/music/");
}
#[tokio::test]async fn autoplay_prefetches_local_tracks_and_continues_with_correct_statistics(){
    let f=Fixture::new().await;let id=f.recommendations_fixture().await;
    let(_,config)=f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"autoplay":{"enabled":true,"source":"local","fallback_local":true}}))).await;assert_eq!(config["autoplay"]["enabled"],true);
    f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":[id]}))).await;
    let mut events=f.s.events.subscribe();let(_,first)=f.call("POST","/api/v1/zones/zone-local/play",Some(json!({}))).await;
    tokio::time::timeout(std::time::Duration::from_secs(3),async {loop{let e=events.recv().await.unwrap();if e["type"]=="autoplay"&&e["data"]["status"]=="ready" {break;}}}).await.unwrap();
    let(status,next)=f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":id,"position":1}))).await;assert_eq!(status,StatusCode::OK);assert_eq!(next["status"],"playing");assert_ne!(next["track"]["id"],id);assert!(next["queue"].as_array().unwrap().len()>1);
    assert!(next["playback_id"].is_string());assert_ne!(next["playback_id"],first["playback_id"]);
    let(_,duplicate)=f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":id,"position":1}))).await;assert_eq!(duplicate["track"]["id"],next["track"]["id"]);
    f.s.db.run(|c|{let reason:String=c.query_row("SELECT finish_reason FROM play_sessions WHERE finished_at IS NOT NULL",[],|r|r.get(0))?;assert_eq!(reason,"ended");let skips:i64=c.query_row("SELECT COALESCE(SUM(skip_count),0) FROM track_stats",[],|r|r.get(0))?;assert_eq!(skips,0);Ok(())}).await.unwrap();
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();assert!(restored.zones.read().await["zone-local"].autoplay.enabled);
}
#[tokio::test]async fn autoplay_respects_repeat_and_invalid_configuration(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;
    assert_eq!(f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"autoplay":{"enabled":true,"source":"invalid source!","fallback_local":true}}))).await.0,StatusCode::BAD_REQUEST);
    f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"repeat":"one","autoplay":{"enabled":true,"source":"local","fallback_local":true}}))).await;
    f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":[id]}))).await;f.call("POST","/api/v1/zones/zone-local/play",Some(json!({}))).await;
    let(_,repeated)=f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":id,"position":1}))).await;assert_eq!(repeated["status"],"playing");assert_eq!(repeated["queue"].as_array().unwrap().len(),1);
    let mut events=f.s.events.subscribe();
    f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"repeat":"off"}))).await;
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        while let Ok(event)=events.recv().await {
            if event["type"]=="autoplay"&&event["data"]["status"]=="ready"{break;}
        }
    }).await.unwrap();
    let(_,ended)=f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":id,"position":1}))).await;
    assert_eq!(ended["status"],"playing");
    assert_eq!(ended["track"]["id"],id);
    assert_eq!(ended["queue_index"],1);
    let(_,stopped)=f.call("POST","/api/v1/zones/zone-local/stop",Some(json!({}))).await;
    assert_eq!(stopped["status"],"stopped");
}
#[tokio::test]async fn shuffled_queue_finishes_after_each_track_once(){
    let f=Fixture::new().await;let seed=f.recommendations_fixture().await;
    let ids:Vec<String>=std::iter::once(seed.clone()).chain((0..6).map(|i|format!("local-recommend-{i}"))).collect();
    f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"autoplay":{"enabled":false,"source":"mixed","fallback_local":true}}))).await;
    f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":ids}))).await;
    f.call("POST","/api/v1/zones/zone-local/play",Some(json!({}))).await;
    let(_,shuffled)=f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"shuffle":true}))).await;
    assert_eq!(shuffled["track"]["id"],seed);
    let mut seen=std::collections::HashSet::from([seed]);
    for _ in 1..ids.len(){let(_,next)=f.call("POST","/api/v1/zones/zone-local/next",Some(json!({}))).await;assert_eq!(next["status"],"playing");assert!(seen.insert(next["track"]["id"].as_str().unwrap().to_string()));}
    let(_,ended)=f.call("POST","/api/v1/zones/zone-local/next",Some(json!({}))).await;assert_eq!(ended["status"],"stopped");assert_eq!(seen.len(),ids.len());
}
#[tokio::test]
async fn autoplay_legacy_provider_settings_do_not_call_plugin_recommendations(){
    let f=Fixture::new().await;
    let seed=f.recommendations_fixture().await;
    let dir=f.s.config.plugin_dir.join("qqmusic");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),r#"{"id":"qqmusic","capabilities":["recommendations"]}"#).unwrap();
    std::fs::write(dir.join("provider.py"),"from pathlib import Path\nPath('started').write_text('yes')\nraise SystemExit(1)\n").unwrap();
    let (status,_)=f.call("PATCH","/api/v1/zones/zone-local",Some(json!({"autoplay":{"enabled":true,"source":"qqmusic","fallback_local":false}}))).await;
    assert_eq!(status,StatusCode::OK);
    f.call("PUT","/api/v1/zones/zone-local/queue",Some(json!({"track_ids":[seed]}))).await;
    f.call("POST","/api/v1/zones/zone-local/play",Some(json!({}))).await;
    f.call("POST","/api/v1/zones/zone-local/ended",Some(json!({"track_id":seed,"position":1}))).await;
    for _ in 0..100 {
        if f.s.zones.read().await["zone-local"].queue.len()>1{break;}
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let zone=f.s.zones.read().await["zone-local"].clone();
    assert_eq!(zone.status,"playing");
    assert!(zone.queue.len()>1);
    assert_eq!(zone.track.unwrap().source,"local");
    assert!(!dir.join("started").exists());
}
#[tokio::test]async fn partial_cache_stops_with_consumer_and_can_complete_without_upstream(){
    use futures_util::{StreamExt,stream};use axum::body::Bytes;
    let f=Fixture::new().await;
    let input=stream::iter(vec![Ok(Bytes::from_static(b"abcd")),Ok(Bytes::from_static(b"efgh")),Ok(Bytes::from_static(b"ijkl"))]).boxed();
    let mut cached=media::caching_stream(f.s.clone(),"fixture-track".into(),"flac".into(),"fixture-etag".into(),12,0,input).await.unwrap();
    assert_eq!(cached.next().await.unwrap().unwrap(),"abcd");assert_eq!(cached.next().await.unwrap().unwrap(),"efgh");drop(cached);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let(_,v)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(v["used_bytes"],8);assert_eq!(v["entries"][0]["complete"],false);
    let cid=v["entries"][0]["id"].as_str().unwrap();assert_eq!(std::fs::read(f.s.config.data.join("cache").join(cid)).unwrap(),b"abcdefgh");
    let input=stream::iter(vec![Ok(Bytes::from_static(b"ijkl"))]).boxed();let mut cached=media::caching_stream(f.s.clone(),"fixture-track".into(),"flac".into(),"fixture-etag".into(),12,8,input).await.unwrap();while let Some(v)=cached.next().await{v.unwrap();}drop(cached);
    let(_,v)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(v["used_bytes"],12);assert_eq!(v["entries"][0]["complete"],true);
    let track:models::Track=serde_json::from_value(json!({"id":"fixture-track","title":"缓存验收","artist":"测试","album":"测试","album_artist":"测试","duration":1.,"track_number":1,"disc_number":1,"source":"unavailable-test-provider","format":"flac","sample_rate":48000,"bit_depth":16,"channels":2,"bitrate":0,"cover_url":null,"recording_id":"fixture-recording","available":true,"created_at":0})).unwrap();f.s.db.run(move|c|library::upsert(c,&track,None,None,0,12,None,None)).await.unwrap();
    let response=f.app.clone().oneshot(Request::builder().uri("/api/v1/media/fixture-track").header("Authorization",format!("Bearer {}",f.token)).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);assert_eq!(response.headers()["x-rmusic-cache"],"complete");assert_eq!(&to_bytes(response.into_body(),12).await.unwrap()[..],b"abcdefghijkl");
}

#[tokio::test]async fn cache_default_directory_persists_and_existing_ranges_remain_readable(){
    use futures_util::{stream,StreamExt};use axum::body::Bytes;
    let f=Fixture::new().await;
    let(_,initial)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(initial["limit_bytes"],2_u64*1024*1024*1024);assert_eq!(initial["eviction"],"manual_only");
    let mut first=media::caching_stream(f.s.clone(),"old".into(),"flac".into(),"etag".into(),8,0,stream::iter([Ok(Bytes::from_static(b"abcd"))]).boxed()).await.unwrap();first.next().await.unwrap().unwrap();drop(first);
    let directory=f.s.config.data.join("separate-cache");
    assert_eq!(f.call("PATCH","/api/v1/cache",Some(json!({"directory":directory}))).await.0,StatusCode::OK);
    let mut resumed=media::caching_stream(f.s.clone(),"old".into(),"flac".into(),"etag".into(),8,4,stream::iter([Ok(Bytes::from_static(b"efgh"))]).boxed()).await.unwrap();resumed.next().await.unwrap().unwrap();drop(resumed);
    let mut fresh=media::caching_stream(f.s.clone(),"new".into(),"flac".into(),"etag2".into(),4,0,stream::iter([Ok(Bytes::from_static(b"ijkl"))]).boxed()).await.unwrap();fresh.next().await.unwrap().unwrap();drop(fresh);
    tokio::time::timeout(std::time::Duration::from_secs(3),async{while !f.s.cache_active.lock().await.is_empty(){tokio::task::yield_now().await;}}).await.unwrap();
    let old=auth::hash("old:etag");let new=auth::hash("new:etag2");
    assert_eq!(std::fs::read(f.s.config.data.join("cache").join(old)).unwrap(),b"abcdefgh");assert_eq!(std::fs::read(directory.join(new)).unwrap(),b"ijkl");
    let restarted=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();assert_eq!(restarted.cache_dir().await,std::fs::canonicalize(directory).unwrap());
    assert_eq!(f.call("PATCH","/api/v1/cache",Some(json!({"directory":"relative-path"}))).await.0,StatusCode::BAD_REQUEST);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    f.call("PATCH","/api/v1/cache",Some(json!({"limit_bytes":4}))).await;
    let mut sparse=media::caching_stream(f.s.clone(),"seek-hole".into(),"flac".into(),"etag3".into(),8192,4096,stream::iter([Ok(Bytes::from_static(b"mnop"))]).boxed()).await.unwrap();assert_eq!(sparse.next().await.unwrap().unwrap(),"mnop");drop(sparse);
    assert!(!f.s.cache_dir().await.join(auth::hash("seek-hole:etag3")).exists(),"seek holes must not bypass capacity accounting");
}

#[tokio::test]async fn cache_budget_rejects_shrinking_without_deleting_original_or_portable(){
    let f=Fixture::new().await;let root=f.s.cache_dir().await;
    for id in ["hot","cold"]{std::fs::write(root.join(id),[0;10]).unwrap();let path=root.join(id).to_string_lossy().to_string();let id=id.to_string();f.s.db.run(move|c|{c.execute("INSERT INTO cache_entries(id,track_id,format,size,total_size,complete,last_access,path) VALUES(?1,?1,'flac',10,10,1,?2,?3)",rusqlite::params![id,if id=="hot"{1}else{100},path])?;Ok(())}).await.unwrap();}
    let portable=root.join("portable/cold");std::fs::create_dir_all(&portable).unwrap();let export=portable.join("song.flac");std::fs::write(&export,[0;4]).unwrap();
    f.s.db.run(move|c|{c.execute("INSERT INTO cache_exports(cache_id,path,size,status,updated_at)VALUES('cold',?1,4,'ready',0)",[export.to_string_lossy().to_string()])?;c.execute("INSERT INTO track_stats(track_id,play_count,play_seconds)VALUES('hot',10,300),('cold',1,30)",[])?;Ok(())}).await.unwrap();
    let(_,before)=f.call("GET","/api/v1/cache?include_partial=true",None).await;assert_eq!(before["used_bytes"],24);assert_eq!(before["portable_bytes"],4);
    f.s.cache_active.lock().await.insert("hot".into(),1);
    let(status,error)=f.call("PATCH","/api/v1/cache",Some(json!({"limit_bytes":14}))).await;assert_eq!(status,StatusCode::CONFLICT);assert_eq!(error["error"]["code"],"cache_budget_below_usage");
    assert!(root.join("hot").is_file()&&root.join("cold").is_file()&&portable.is_dir());
    assert_eq!(f.call("PATCH","/api/v1/cache",Some(json!({"limit_bytes":24}))).await.0,StatusCode::OK);
    assert!(media::ensure_capacity(&f.s,0).await.unwrap());assert!(!media::ensure_capacity(&f.s,1).await.unwrap());
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"cache_limit_bytes":23,"cache_enabled":true}))).await.0,StatusCode::CONFLICT);
    assert!(root.join("cold").is_file()&&portable.is_dir());
    assert!(media::remove_export_directory("cold",&f.s.config.data.join("unsafe/song.flac").to_string_lossy()).await.is_err());
}
#[tokio::test]async fn single_cache_protects_saved_manual_and_smart_playlists_at_capacity(){
    let f=Fixture::new().await;f.collection_tracks().await;let root=f.s.cache_dir().await;
    for track in ["local-shared","qq-shared","disposable"] {let path=root.join(track);std::fs::write(&path,[0u8;10]).unwrap();let track=track.to_owned();f.s.db.run(move|c|{c.execute("INSERT INTO cache_entries(id,track_id,format,size,total_size,complete,last_access,path) VALUES(?1,?1,'flac',10,10,1,0,?2)",rusqlite::params![track,path.to_string_lossy()])?;Ok(())}).await.unwrap();}
    let(_,saved)=f.call("POST","/api/v1/playlists",Some(json!({"name":"保留云端缓存","track_ids":["qq-shared"]}))).await;
    assert_eq!(f.call("POST","/api/v1/playlists",Some(json!({"name":"智能本地","smart_query":{"source":"local","q":"夜航"}}))).await.0,StatusCode::OK);
    assert_eq!(f.call("PATCH","/api/v1/cache",Some(json!({"limit_bytes":20}))).await.0,StatusCode::CONFLICT);
    assert_eq!(f.call("PATCH","/api/v1/cache",Some(json!({"limit_bytes":30}))).await.0,StatusCode::OK);
    assert!(!media::ensure_capacity(&f.s,1).await.unwrap());
    f.call("DELETE",&format!("/api/v1/playlists/{}",saved["id"].as_str().unwrap()),None).await;
    assert!(media::ensure_capacity(&f.s,0).await.unwrap());
    for id in ["disposable","qq-shared","local-shared"]{assert!(root.join(id).is_file());}

}

#[tokio::test]async fn qq_metadata_preserves_file_audio_lyrics_and_manual_fields(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;
    let before=library::get_track(&f.s,id.clone()).await.unwrap();
    let result=json!({"source":"qqmusic","matched":true,"mid":"Abc12345","track":{"cover_url":"https://y.gtimg.cn/cover.jpg"},"lyrics":"cloud lyrics","background":{"release_date":"2020"},"artists":[{"name":before.artist,"image_url":"https://y.gtimg.cn/artist.jpg","description":"QQ artist biography"}]});
    metadata::save_track(&f.s,id.clone(),result.clone()).await.unwrap();
    let after=library::get_track(&f.s,id.clone()).await.unwrap();
    assert_eq!(after.sample_rate,before.sample_rate);assert_eq!(after.bit_depth,before.bit_depth);assert_eq!(after.title,before.title);
    assert_eq!(after.recording_id,before.recording_id);assert_eq!(after.provider_data["provider_metadata"]["source"],"qqmusic");
    let (_,artists)=f.call("GET","/api/v1/library/artists",None).await;assert_eq!(artists["items"][0]["image_url"],format!("/api/v1/artwork/{}",auth::hash("https://y.gtimg.cn/artist.jpg")));assert_eq!(artists["items"][0]["description"],"QQ artist biography");
    let (_,lyrics)=f.call("GET",&format!("/api/v1/library/tracks/{id}/lyrics"),None).await;assert!(lyrics["text"].as_str().unwrap().contains("晚风"));
    let tid=id.clone();f.s.db.run(move|c|{c.execute("UPDATE tracks SET provider_data='{\"manual_metadata\":true}' WHERE id=?1",[tid])?;Ok(())}).await.unwrap();
    metadata::save_track(&f.s,id.clone(),result).await.unwrap();assert_eq!(library::get_track(&f.s,id).await.unwrap().provider_data,json!({"manual_metadata":true}));
}
#[tokio::test]async fn local_priority_keeps_versions_and_ambiguous_matches_separate(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;let mut local=library::get_track(&f.s,id.clone()).await.unwrap();
    local.artist="明确演唱者".into();local.album_artist=local.artist.clone();let key=id.clone();f.s.db.run(move|c|{c.execute("UPDATE tracks SET artist='明确演唱者',album_artist='明确演唱者' WHERE id=?1",[key])?;Ok(())}).await.unwrap();
    let mut cloud=local.clone();cloud.id="qqmusic:Abc12345".into();cloud.source="qqmusic".into();cloud.recording_id=cloud.id.clone();
    let selected=catalog::prefer_local(&f.s,vec![cloud.clone(),local.clone()]).await.unwrap();assert_eq!(selected.len(),1);assert_eq!(selected[0].id,id);
    cloud.title.push_str(" (Live)");assert_eq!(catalog::prefer_local(&f.s,vec![cloud.clone()]).await.unwrap()[0].id,cloud.id);
}
#[tokio::test]async fn online_quality_validates_and_audio_info_excludes_private_urls(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"stream_quality":"FLAC"}))).await.0,StatusCode::OK);
    assert_eq!(catalog::quality(&f.s,"qqmusic").await,"FLAC");
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"stream_quality":"bad"}))).await.0,StatusCode::BAD_REQUEST);
    let(status,v)=f.call("GET",&format!("/api/v1/library/tracks/{id}/audio-info"),None).await;assert_eq!(status,StatusCode::OK);assert_eq!(v["sample_rate"],48000);assert_eq!(v["bit_depth"],16);assert_eq!(v["requested_quality"],"original");assert!(v.get("url").is_none());
}
#[tokio::test]async fn foreground_reads_preempt_anonymous_indexing_without_blocking_account_rpc(){
    let f=Fixture::new().await;
    f.s.foreground_last_activity.store(chrono::Utc::now().timestamp()-10,std::sync::atomic::Ordering::Relaxed);
    let dir=f.s.config.plugin_dir.join("qqmusic");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys,os,time\nfrom pathlib import Path\nfor line in sys.stdin:\n r=json.loads(line);anonymous=os.environ.get('RMUSIC_PROVIDER_ANONYMOUS')=='1'\n if anonymous and not r['params'].get('fast'):\n  Path('background-started').write_text('yes');time.sleep(5)\n print(json.dumps({'id':r['id'],'result':{'anonymous':anonymous,'directory':os.environ['RMUSIC_PROVIDER_DATA_DIR']}}),flush=True)\n").unwrap();
    let plugin=f.s.plugin.clone();
    let background=tokio::spawn(async move {plugin.background_metadata("qqmusic",json!({})).await});
    for _ in 0..100{if dir.join("background-started").exists(){break;}tokio::time::sleep(std::time::Duration::from_millis(10)).await;}
    assert!(dir.join("background-started").exists());
    let result=tokio::time::timeout(std::time::Duration::from_millis(500),f.call("GET","/api/v1/library/tracks",None)).await.unwrap();
    assert_eq!(result.0,StatusCode::OK);
    let cancelled=tokio::time::timeout(std::time::Duration::from_secs(1),background).await.unwrap().unwrap();
    assert!(cancelled.unwrap_err().2.contains("前台优先"));
    let account=tokio::time::timeout(std::time::Duration::from_secs(2),f.s.plugin.call("qqmusic","health",json!({}))).await.unwrap().unwrap();
    assert_eq!(account["anonymous"],false);
    let foreground=tokio::time::timeout(std::time::Duration::from_secs(2),f.s.plugin.call("qqmusic","metadata",json!({"fast":true}))).await.unwrap().unwrap();
    assert_eq!(foreground["anonymous"],true);
    assert!(foreground["directory"].as_str().unwrap().contains(".anonymous-qqmusic"));
    assert!(metadata::frontend_busy(&f.s));
    assert_eq!(f.s.foreground_requests.load(std::sync::atomic::Ordering::Relaxed),0);
    f.s.foreground_last_activity.store(chrono::Utc::now().timestamp()-10,std::sync::atomic::Ordering::Relaxed);
    assert!(!metadata::frontend_busy(&f.s));
    let resumed=f.s.plugin.background_metadata("qqmusic",json!({"fast":true})).await.unwrap();
    assert_eq!(resumed["anonymous"],true);
    assert!(resumed["directory"].as_str().unwrap().contains(".anonymous-qqmusic"));
    f.s.plugin.stop("qqmusic").await;
}
#[tokio::test]async fn background_starting_after_a_page_request_must_wait(){
    let f=Fixture::new().await;
    let dir=f.s.config.plugin_dir.join("qqmusic");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);pathlib.Path('request-sent').write_text('yes')\n print(json.dumps({'id':r['id'],'result':{}}),flush=True)\n").unwrap();
    let entered=std::sync::Arc::new(tokio::sync::Notify::new());
    let release=std::sync::Arc::new(tokio::sync::Notify::new());
    let page_entered=entered.clone();let page_release=release.clone();
    let page=Router::new().route("/page",get(move||{
        let entered=page_entered.clone();let release=page_release.clone();
        async move{entered.notify_one();release.notified().await;axum::Json(json!({"loaded":true}))}
    })).layer(axum::middleware::from_fn_with_state(f.s.clone(),metadata::foreground_priority)).with_state(f.s.clone());
    let request=tokio::spawn(async move{call(&page,"GET","/page",None,None).await});
    entered.notified().await;
    assert_eq!(f.s.foreground_requests.load(std::sync::atomic::Ordering::Relaxed),1);
    let background=tokio::time::timeout(std::time::Duration::from_millis(500),f.s.plugin.background_metadata("qqmusic",json!({}))).await;
    release.notify_one();assert_eq!(request.await.unwrap().0,StatusCode::OK);
    assert_eq!(background.unwrap().unwrap_err().1,"background_deferred");
    assert!(!dir.join("request-sent").exists());
    assert_eq!(f.s.foreground_requests.load(std::sync::atomic::Ordering::Relaxed),0);
    assert_eq!(f.s.plugin.background_metadata("qqmusic",json!({})).await.unwrap_err().1,"background_deferred");
    assert!(!dir.join("request-sent").exists());
    f.s.foreground_last_activity.store(chrono::Utc::now().timestamp()-10,std::sync::atomic::Ordering::Relaxed);
    assert!(f.s.plugin.background_metadata("qqmusic",json!({})).await.is_ok());
    f.s.plugin.stop("qqmusic").await;
}
#[tokio::test]async fn status_polling_does_not_starve_background_indexing(){
    let f=Fixture::new().await;
    f.s.foreground_last_activity.store(chrono::Utc::now().timestamp()-10,std::sync::atomic::Ordering::Relaxed);
    for path in ["/api/v1/status","/api/v1/jobs","/api/v1/endpoints","/api/v1/zones","/api/v1/zones/zone-local","/api/v1/cache","/api/v1/downloads"] {
        assert_eq!(f.call("GET",path,None).await.0,StatusCode::OK);
        assert!(!metadata::frontend_busy(&f.s));
    }
    assert_eq!(f.call("GET","/api/v1/library/tracks",None).await.0,StatusCode::OK);
    assert!(metadata::frontend_busy(&f.s));
    assert_eq!(f.s.foreground_requests.load(std::sync::atomic::Ordering::Relaxed),0);
}
#[tokio::test]async fn cached_audio_info_reports_the_actual_file_and_respects_quality_selection(){
    use futures_util::{stream,StreamExt};use axum::body::Bytes;
    let f=Fixture::new().await;let(id,wav)=f.add_audio().await;
    let mut cloud=library::get_track(&f.s,id).await.unwrap();cloud.id="qqmusic:Cache123".into();cloud.source="qqmusic".into();cloud.sample_rate=192000;cloud.bit_depth=24;
    f.s.db.run(move|c|library::upsert(c,&cloud,None,None,0,0,None,None)).await.unwrap();
    let total=wav.len() as u64;
    let input=stream::iter([Ok(Bytes::from(wav))]).boxed();
    let mut cached=media::caching_stream(f.s.clone(),"qqmusic:Cache123".into(),"wav".into(),"\"fixture-qFLAC\"".into(),total,0,input).await.unwrap();
    while let Some(chunk)=cached.next().await{chunk.unwrap();}drop(cached);
    f.call("PATCH","/api/v1/settings",Some(json!({"stream_quality":"FLAC"}))).await;
    let(status,info)=f.call("GET","/api/v1/library/tracks/qqmusic:Cache123/audio-info",None).await;
    assert_eq!(status,StatusCode::OK);assert_eq!(info["sample_rate"],48000);assert_eq!(info["bit_depth"],16);assert_eq!(info["verified"],true);assert_eq!(info["quality_label"],"核心完整缓存");
    f.call("PATCH","/api/v1/settings",Some(json!({"stream_quality":"MP3_320"}))).await;
    assert_ne!(f.call("GET","/api/v1/library/tracks/qqmusic:Cache123/audio-info",None).await.0,StatusCode::OK);
}
#[tokio::test]
async fn local_search_returns_files_without_waiting_for_plugins() {
    let f = Fixture::new().await;
    let (id, _) = f.add_audio().await;
    let mut cloud = library::get_track(&f.s, id.clone()).await.unwrap();
    cloud.id = "qqmusic:CatalogSearchFixture".into();
    cloud.source = "qqmusic".into();
    f.s.db.run(move |c| library::upsert(c, &cloud, None, None, 0, 0, None, None)).await.unwrap();
    let plugin = f.s.config.plugin_dir.join("qqmusic");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("manifest.json"), r#"{"id":"qqmusic","capabilities":["search"]}"#).unwrap();
    std::fs::write(plugin.join("provider.py"), "import time\nfor line in __import__('sys').stdin: time.sleep(10)\n").unwrap();

    let (status, result) = tokio::time::timeout(std::time::Duration::from_secs(2),
        f.call("GET", "/api/v1/library/search?q=%E5%A4%9C%E8%88%AA", None)).await.expect("local search waited for the plugin");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["source"], "local");
    assert_eq!(result["total"], 1);
    assert_eq!(result["items"][0]["id"], id);
}

#[tokio::test]async fn malformed_cloud_search_keeps_local_results_available(){
    let f=Fixture::new().await;let(id,_)=f.add_audio().await;
    let dir=f.s.config.plugin_dir.join("qqmusic");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),"{\"id\":\"qqmusic\",\"capabilities\":[\"search\"]}").unwrap();
    std::fs::write(dir.join("provider.py"),"import json,sys\nfor line in sys.stdin:\n r=json.loads(line);print(json.dumps({'id':r['id'],'result':{'items':[{'track_number':-31073}]}}),flush=True)\n").unwrap();
    let(status,result)=f.call("GET","/api/v1/search?q=%E5%A4%9C%E8%88%AA",None).await;
    assert_eq!(status,StatusCode::OK);assert_eq!(result["items"][0]["id"],id);
    assert!(result["notice"].as_str().unwrap().contains("已保留"));f.s.plugin.stop("qqmusic").await;
}

#[tokio::test]
async fn partial_dsp_contract_defaults_and_validation(){
    let f=Fixture::new().await;
    let (status,zone)=f.call("PUT","/api/v1/zones/zone-local/dsp",Some(json!({"enabled":false}))).await;
    assert_eq!(status,StatusCode::OK);
    assert_eq!(zone["dsp"],json!({"enabled":false,"preamp_db":0.,"balance":0.,"normalize":false,"target_lufs":-14.,"bands":[]}));
    let mut old=zone.clone();old["dsp"]=json!({"enabled":true,"preamp_db":-6.});
    let restored:models::Zone=serde_json::from_value(old).unwrap();
    assert_eq!(restored.dsp["bands"],json!([]));assert_eq!(restored.dsp["preamp_db"],-6.);assert_eq!(restored.dsp["enabled"],true);
    for bad in [json!({"bands":null}),json!({"enabled":"false"}),json!({"preamp_db":"0"}),json!({"bands":[{"frequency":1000,"gain_db":0,"q":1}]})]{
        assert_eq!(f.call("PUT","/api/v1/zones/zone-local/dsp",Some(bad)).await.0,StatusCode::BAD_REQUEST);
    }
    let (_,current)=f.call("GET","/api/v1/zones/zone-local",None).await;
    assert_eq!(current["dsp"],zone["dsp"]);
}

#[tokio::test]
async fn job_pages_are_bounded_stable_and_individually_addressable(){
    let f=Fixture::new().await;
    f.s.db.run(|c|{for i in 0..23{c.execute("INSERT INTO jobs(id,kind,status,created_at,updated_at,priority)VALUES(?1,'scan',?2,0,?3,0)",rusqlite::params![format!("page-{i:02}"),if i<4{"running"}else{"completed"},i])?;}Ok(())}).await.unwrap();
    let(_,first)=f.call("GET","/api/v1/jobs?status=history&limit=5&offset=0",None).await;
    let(_,second)=f.call("GET","/api/v1/jobs?status=history&limit=5&offset=5",None).await;
    assert_eq!(first["total"],19);assert_eq!(first["items"].as_array().unwrap().len(),5);assert_eq!(first["counts"]["running"],4);
    assert_ne!(first["items"][4]["id"],second["items"][0]["id"]);
    let(_,one)=f.call("GET","/api/v1/jobs?id=page-00&limit=1",None).await;assert_eq!(one["items"][0]["id"],"page-00");assert_eq!(one["total"],1);
    let(_,active)=f.call("GET","/api/v1/jobs?status=active&limit=5",None).await;assert_eq!(active["total"],4);
}
#[tokio::test]
async fn saved_cache_always_enabled_and_survives_reopen(){
    let f=Fixture::new().await;
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"cache_enabled":false}))).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"cache_enabled":true}))).await.0,StatusCode::OK);
    let(_,settings)=f.call("GET","/api/v1/settings",None).await;assert_eq!(settings["cache_enabled"],true);assert_eq!(settings["cache_forced_off"],false);
    let reopened=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();assert!(reopened.cache_enabled.load(std::sync::atomic::Ordering::Acquire));
}

#[tokio::test]async fn artist_credits_migrate_and_keep_collaboration_tracks_in_each_catalog(){
 let f=Fixture::new().await;f.collection_tracks().await;
 f.s.db.run(|c|{let mut t=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],crate::models::Track::from_row)?;
 t.artist="Crush / IU".into();t.provider_data=json!({"artist_mids":["crush","iu"]});crate::library::upsert(c,&t,None,None,0,0,None,None)?;
 c.execute("DELETE FROM track_artists",[])?;c.execute("DELETE FROM settings WHERE key='artist_credits_v2'",[])?;crate::artist_credits::migrate(c)?;crate::artist_credits::migrate(c)?;Ok(())}).await.unwrap();
 let(status,artists)=f.call("GET","/api/v1/library/artists?source=qqmusic",None).await;assert_eq!(status,StatusCode::OK);
 for name in ["Crush","IU"] {let item=artists["items"].as_array().unwrap().iter().find(|v|v["name"]==name).unwrap();let(status,tracks)=f.call("GET",&format!("/api/v1/library/tracks?artist_id={}",item["id"].as_str().unwrap()),None).await;assert_eq!(status,StatusCode::OK);assert!(tracks["items"].as_array().unwrap().iter().any(|t|t["id"]=="qq-only"));}
 assert!(!artists["items"].as_array().unwrap().iter().any(|v|v["name"]=="Crush / IU"));
 let(status,links)=f.call("POST","/api/v1/library/links",Some(json!({"artist":"Crush / IU","album":"duet","source":"qqmusic","provider_data":{"artist_mids":["crush","iu"]}}))).await;assert_eq!(status,StatusCode::OK);assert_eq!(links["artists"].as_array().unwrap().len(),2);
}

#[tokio::test]async fn catalog_detail_never_merges_same_names_and_links_only_shared_entity_ids(){
 let f=Fixture::new().await;f.collection_tracks().await;
 let id=f.s.db.run(|c|{
  let mut t=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],crate::models::Track::from_row)?;
  t.artist="Crush / IU".into();t.provider_data=json!({"artist_credits":[{"id":"crush","name":"Crush"},{"id":"iu","name":"IU"}]});crate::library::upsert(c,&t,None,None,0,0,None,None)?;
  let mut other=t.clone();other.id="second:song".into();other.source="second".into();other.provider_data=json!({"artist_credits":[{"id":"another-iu","name":"IU"}]});other.artist="IU".into();crate::library::upsert(c,&other,None,None,0,0,None,None)?;
  Ok(crate::artist_credits::credits(&t.source,&t.artist,&t.provider_data)[1].id.clone())
 }).await.unwrap();
 let uri=format!("/api/v1/library/catalog/artist/{id}");
 assert_eq!(call(&f.app,"GET",&uri,None,None).await.0,StatusCode::UNAUTHORIZED);
 let(status,page)=f.call("GET",&uri,None).await;assert_eq!(status,StatusCode::OK,"{page}");assert_eq!(page["name"],"IU");assert_eq!(page["local"]["total"],1);
 f.s.db.run(|c|{
  for id in ["qq-only","second:song"] {let mut t=c.query_row("SELECT * FROM tracks WHERE id=?1",[id],crate::models::Track::from_row)?;t.provider_data["identifiers"]=json!({"musicbrainz":{"artist":["11111111-1111-4111-8111-111111111111"]}});crate::library::upsert(c,&t,None,None,0,0,None,None)?;}
  // An explicit single artist assertion is attributed to IU, not the combined display caption.
  c.execute("UPDATE music_entities SET title='IU' WHERE namespace='musicbrainz' AND kind='artist'",[])?;
  Ok(())
 }).await.unwrap();
 let(_,page)=f.call("GET",&uri,None).await;assert_eq!(page["local"]["total"],2);
 let(_,home)=f.call("GET","/api/v1/library/home",None).await;assert!(home["albums"].as_array().unwrap().iter().all(|v|v["track_id"].is_string()&&v["source"].is_string()));
 assert_eq!(f.call("GET",&(uri+"?page=0"),None).await.0,StatusCode::BAD_REQUEST);
}
#[tokio::test]async fn metadata_priority_is_validated_and_applied_in_order(){
 let f=Fixture::new().await;
 for id in ["first","second"] {let dir=f.s.config.plugin_dir.join(id);std::fs::create_dir_all(&dir).unwrap();std::fs::write(dir.join("manifest.json"),json!({"id":id,"name":id,"capabilities":["metadata"]}).to_string()).unwrap();}
 assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"metadata_provider_priority":["second","first"]}))).await.0,StatusCode::OK);
 assert_eq!(crate::connectors::ordered_metadata_providers(&f.s).await,vec!["second","first"]);
 assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"metadata_provider_priority":["first","first"]}))).await.0,StatusCode::BAD_REQUEST);
 assert_eq!(f.call("PATCH","/api/v1/settings",Some(json!({"metadata_provider_priority":["unknown"]}))).await.0,StatusCode::BAD_REQUEST);
 let(_,settings)=f.call("GET","/api/v1/settings",None).await;assert_eq!(settings["metadata_provider_priority"],json!(["second","first"]));assert_eq!(settings["metadata_providers"].as_array().unwrap().len(),2);
}
#[tokio::test]async fn catalog_detail_keeps_same_named_releases_separate(){
 let f=Fixture::new().await;f.collection_tracks().await;
 let (legacy,key)=f.s.db.run(|c|{let mut t=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],crate::models::Track::from_row)?;
 t.provider_data=json!({"album_mid":"release-a"});crate::library::upsert(c,&t,None,None,0,0,None,None)?;
 let key=crate::artist_credits::release_key(&t.source,&t.provider_data).unwrap();
 let legacy=crate::auth::hash(&format!("{}\0{}\0{}",t.source,t.album_artist,t.album));
 t.id="qqmusic:other-edition".into();t.provider_data=json!({"album_mid":"release-b"});crate::library::upsert(c,&t,None,None,0,0,None,None)?;
 Ok((legacy,key))}).await.unwrap();
 assert_eq!(f.call("GET",&format!("/api/v1/library/catalog/album/{legacy}?remote=false"),None).await.0,StatusCode::BAD_REQUEST);
 let(status,detail)=f.call("GET",&format!("/api/v1/library/catalog/album/{key}?remote=false"),None).await;assert_eq!(status,StatusCode::OK,"{detail}");assert_eq!(detail["local"]["total"],1);assert_eq!(detail["local"]["items"][0]["id"],"qq-only");
 let(_,home)=f.call("GET","/api/v1/library/home",None).await;
 assert!(home["albums"].as_array().unwrap().iter().any(|v|v["id"]==key));
}
#[tokio::test]async fn artist_credits_hydrate_legacy_ids_without_replacing_other_metadata(){
 let f=Fixture::new().await;f.collection_tracks().await;
 let anchor=f.s.db.run(|c|{let mut t=c.query_row("SELECT * FROM tracks WHERE id='qq-only'",[],crate::models::Track::from_row)?;t.id="example:legacy".into();t.source="example".into();t.artist="First / Second".into();t.provider_data=json!({"description":"keep me"});crate::library::upsert(c,&t,None,None,0,0,None,None)?;Ok(t)}).await.unwrap();
 let dir=f.s.config.plugin_dir.join("example");std::fs::create_dir_all(&dir).unwrap();
 std::fs::write(dir.join("manifest.json"),json!({"id":"example","name":"fixture","capabilities":["metadata"]}).to_string()).unwrap();
 let response=json!({"matched":true,"source":"example","track":anchor,"artists":[{"mid":"first-id","name":"First"},{"mid":"second-id","name":"Second"}]});
 std::fs::write(dir.join("response.json"),response.to_string()).unwrap();
 std::fs::write(dir.join("provider.py"),"import json,sys,pathlib\nfor line in sys.stdin:\n r=json.loads(line);v=json.loads(pathlib.Path(__file__).with_name('response.json').read_text());print(json.dumps({'id':r['id'],'result':v}),flush=True)\n").unwrap();
 let(status,links)=f.call("POST","/api/v1/library/links",Some(json!({"track_id":anchor.id,"artist":anchor.artist,"album":anchor.album,"resolve_credits":true}))).await;assert_eq!(status,StatusCode::OK,"{links}");assert_eq!(links["artists"].as_array().unwrap().len(),2);assert_eq!(links["artists"][1]["name"],"Second");
 let saved=crate::library::get_track(&f.s,anchor.id.clone()).await.unwrap();assert_eq!(saved.provider_data["description"],"keep me");assert_eq!(saved.provider_data["artist_credits"][1]["id"],"second-id");
 f.s.plugin.stop("example").await;
}

#[tokio::test]async fn catalog_scrape_persists_locks_conflicts_and_unlocks_without_audio_cache(){
 let f=Fixture::new().await;f.collection_tracks().await;
 crate::catalog_cache::save(&f.s,"track","qq-only",json!({"description":"first","genre":"Pop"})).await.unwrap();
 let uri="/api/v1/library/catalog/track/qq-only/profile";
 assert_eq!(call(&f.app,"PATCH",uri,None,Some(json!({}))).await.0,StatusCode::UNAUTHORIZED);
 let(status,first)=f.call("PATCH",uri,Some(json!({"expected_revision":0,"fields":{"description":"Human"}}))).await;assert_eq!(status,StatusCode::OK,"{first}");assert_eq!(first["revision"],1);
 assert_eq!(f.call("PATCH",uri,Some(json!({"expected_revision":0,"fields":{"description":"stale"}}))).await.0,StatusCode::CONFLICT);
 crate::catalog_cache::save(&f.s,"track","qq-only",json!({"description":"fresh platform","genre":"Rock","credits":[{"role":"composer","names":["Updated"]}]})).await.unwrap();
 let(_,value)=f.call("GET",uri,None).await;assert_eq!(value["profile"]["description"],"Human");assert_eq!(value["profile"]["genre"],"Rock");assert_eq!(value["profile"]["credits"][0]["names"][0],"Updated");
 let reopened=crate::db::Db::open(&f._dir.path().join("db.sqlite")).unwrap();assert_eq!(reopened.read(|c|Ok(c.query_row("SELECT revision FROM catalog_profiles WHERE kind='track' AND entity_id='qq-only'",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),1);
 let(_,value)=f.call("PATCH",uri,Some(json!({"expected_revision":1,"fields":{},"unlock":["description"]}))).await;assert_eq!(value["profile"]["description"],"fresh platform");
 assert_eq!(f.call("PATCH","/api/v1/library/catalog/track/missing/profile",Some(json!({"expected_revision":0,"fields":{}}))).await.0,StatusCode::NOT_FOUND);
 assert_eq!(f.call("PATCH",uri,Some(json!({"expected_revision":2,"fields":{"image_url":"http://127.0.0.1/private"}}))).await.0,StatusCode::BAD_REQUEST);
 let original=crate::library::get_track(&f.s,"qq-only".into()).await.unwrap().cover_url;
 let replacement=format!("/api/v1/covers/{}","a".repeat(64));
 let(status,_)=f.call("PATCH",uri,Some(json!({"expected_revision":2,"fields":{"image_url":replacement}}))).await;assert_eq!(status,StatusCode::OK);
 assert_eq!(crate::library::get_track(&f.s,"qq-only".into()).await.unwrap().cover_url,Some(replacement));
 f.call("PATCH",uri,Some(json!({"expected_revision":3,"fields":{},"unlock":["image_url"]}))).await;
 assert_eq!(crate::library::get_track(&f.s,"qq-only".into()).await.unwrap().cover_url,original);
 assert_eq!(f.call("PATCH",uri,Some(json!({"expected_revision":4,"fields":{"image_url":"/api/v1/covers/../private"}}))).await.0,StatusCode::BAD_REQUEST);
 crate::catalog_cache::save_page(&f.s,"test:entity".into(),1,&json!({"matched":true,"items":[]}),false).await.unwrap();assert!(crate::catalog_cache::load_page(&f.s,"test:entity".into(),1).await.unwrap().is_some());
}
#[tokio::test]async fn catalog_scrape_artwork_is_authenticated_and_uses_persisted_files(){
 let f=Fixture::new().await;
 let url=f.s.db.run(|c|crate::media_assets::register(c,"https://images.example/portrait.jpg")).await.unwrap();
 let key=url.rsplit('/').next().unwrap().to_string();let bytes=data_encoding::BASE64.decode(b"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aXioAAAAASUVORK5CYII=").unwrap();let cover=crate::artwork_files::content_id(&bytes);std::fs::write(f._dir.path().join("covers").join(&cover),&bytes).unwrap();
 f.s.db.run({let key=key.clone();move|c|{c.execute("UPDATE artwork_sources SET cover_id=?1 WHERE id=?2",rusqlite::params![cover,key])?;Ok(())}}).await.unwrap();
 assert_eq!(call(&f.app,"GET",&url,None,None).await.0,StatusCode::UNAUTHORIZED);
 let response=f.app.clone().oneshot(axum::http::Request::builder().uri(&url).header("authorization",format!("Bearer {}",f.token)).body(axum::body::Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);assert_eq!(response.headers()["content-type"],"image/png");
 let body=http_body_util::BodyExt::collect(response.into_body()).await.unwrap().to_bytes();assert_eq!(body.as_ref(),bytes.as_slice());
}

#[tokio::test]
async fn android_download_ranges_are_bound_to_the_opened_approval() {
    use axum::http::header;
    let f=Fixture::new().await;
    let (track,bytes)=f.add_audio().await;
    let path=f.s.config.data.join("downloads/android-range.wav");
    std::fs::write(&path,&bytes).unwrap();
    let file=path.to_string_lossy().to_string();
    f.s.db.run(move|c|{c.execute("INSERT INTO downloads(id,track_id,status,path,size,created_at) VALUES('android-range',?1,'complete',?2,?3,0)",rusqlite::params![track,file,bytes.len() as i64])?;Ok(())}).await.unwrap();
    let request=|method:&str,range:Option<&str>,validator:Option<&str>| {
        let mut r=Request::builder().method(method).uri("/api/v1/downloads/android-range/file").header(header::AUTHORIZATION,format!("Bearer {}",f.token));
        if let Some(v)=range{r=r.header(header::RANGE,v);}if let Some(v)=validator{r=r.header(header::IF_RANGE,v);}
        r.body(Body::empty()).unwrap()
    };
    let head=f.app.clone().oneshot(request("HEAD",None,None)).await.unwrap();
    assert_eq!(head.status(),StatusCode::OK);
    let tag=head.headers()[header::ETAG].to_str().unwrap().to_owned();
    assert!(to_bytes(head.into_body(),1024).await.unwrap().is_empty());
    let partial=f.app.clone().oneshot(request("GET",Some("bytes=10-19"),Some(&tag))).await.unwrap();
    assert_eq!(partial.status(),StatusCode::PARTIAL_CONTENT);
    assert_eq!(to_bytes(partial.into_body(),1024).await.unwrap().len(),10);
    // Atomic replacement changes the entity; stale If-Range receives the whole new file.
    let replacement=path.with_extension("new");std::fs::write(&replacement,b"replacement audio").unwrap();std::fs::rename(replacement,&path).unwrap();
    let replaced=f.app.clone().oneshot(request("GET",Some("bytes=10-19"),Some(&tag))).await.unwrap();
    assert_eq!(replaced.status(),StatusCode::OK);assert_ne!(replaced.headers()[header::ETAG],tag);
    assert_eq!(to_bytes(replaced.into_body(),1024).await.unwrap().as_ref(),b"replacement audio");
}

#[path="account_security_tests.rs"]
mod account_security;

#[path = "device_group_tests.rs"]
mod device_groups_tests;

#[path="remote_access_tests.rs"]
mod remote_access_tests;

#[tokio::test]
async fn personal_radio_uses_only_the_callers_favorites_and_falls_back_without_model() {
    let f=Fixture::new().await;
    let a=crate::auth::authenticate(&f.s,&f.token).await.unwrap();
    let favorite=format!("favorites-{}",crate::auth::hash(&a.id));
    let uid=a.id.clone();
    let other_token="radio-other-session";
    let token_hash=crate::auth::hash(other_token);
    f.s.db.run(move|c|{
        for (id,source) in [("local-a","local"),("local-b","local"),("testfeed:liked","testfeed"),("testfeed:unliked","testfeed")]{
            c.execute("INSERT INTO tracks(id,title,artist,album,album_artist,source,format,recording_id,created_at) VALUES(?1,?1,'Artist','Album','Artist',?2,'flac',?1,0)",rusqlite::params![id,source])?;
        }
        c.execute("INSERT INTO playlists(id,user_id,name,tracks) VALUES(?1,?2,'我喜欢','[\"testfeed:liked\"]')",rusqlite::params![favorite,uid])?;
        c.execute("INSERT INTO users(id,username,password,role) VALUES('radio-other','radio-other','','member')",[])?;
        c.execute("INSERT INTO sessions(hash,user_id,expires,scopes,created_at) VALUES(?1,'radio-other',unixepoch()+3600,?2,unixepoch())",rusqlite::params![token_hash,json!(["read","control"]).to_string()])?;
        let other_list=format!("favorites-{}",crate::auth::hash("radio-other"));
        c.execute("INSERT INTO playlists(id,user_id,name,tracks) VALUES(?1,'radio-other','我喜欢',?2)",rusqlite::params![other_list,json!(["testfeed:unliked"]).to_string()])?;
        Ok(())
    }).await.unwrap();
    let dir=f.s.config.plugin_dir.join("testfeed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),r#"{"id":"testfeed","name":"测试平台","capabilities":["resolve"]}"#).unwrap();
    std::fs::write(dir.join("provider.py"),r#"import json,sys
for line in sys.stdin:
 r=json.loads(line)
 assert r['method']=='resolve'
 print(json.dumps({'id':r['id'],'result':{'url':'https://audio.invalid/fixture.flac','format':'flac'}}),flush=True)
"#).unwrap();
    let (status,_) = f.call("PUT","/api/v1/recommendations/settings",Some(json!({"advanced":true}))).await;
    assert_eq!(status,StatusCode::OK);
    let (status,value)=f.call("GET","/api/v1/recommendations/discover?limit=24",None).await;
    assert_eq!(status,StatusCode::OK,"{value}");
    assert_eq!(value["algorithm"],"local");
    assert_eq!(value["fallback"],true);
    let items=value["items"].as_array().unwrap();
    assert!(items.iter().any(|v|v["id"]=="testfeed:liked"),"{value}");
    assert!(!items.iter().any(|v|v["id"]=="testfeed:unliked"));
    let (status,other)=call(&f.app,"GET","/api/v1/recommendations/discover",Some(other_token),None).await;
    assert_eq!(status,StatusCode::OK,"{other}");
    assert_eq!(other["fallback"],true);
    let (status,settings)=call(&f.app,"GET","/api/v1/recommendations/settings",Some(other_token),None).await;
    assert_eq!(status,StatusCode::OK);
    assert_eq!(settings["advanced"],true);
    assert_eq!(settings["allowed"],false);
    let (status,_)=call(&f.app,"PUT","/api/v1/recommendations/settings",Some(other_token),Some(json!({"advanced":false}))).await;
    assert_eq!(status,StatusCode::FORBIDDEN);
    assert!(other["items"].as_array().unwrap().iter().any(|v|v["id"]=="testfeed:unliked"));
    assert!(!other["items"].as_array().unwrap().iter().any(|v|v["id"]=="testfeed:liked"));
    let (status,value)=f.call("POST","/api/v1/recommendations/radio",Some(json!({"seed_track_id":"local-a","exclude":["local-b","testfeed:liked"]}))).await;
    assert_eq!(status,StatusCode::OK,"{value}");
    assert_eq!(value["items"],json!([]));
}

#[tokio::test]
async fn personal_radio_searches_generic_cloud_catalog_and_excludes_preview_only_results() {
    let f=Fixture::new().await;
    let id=f.recommendations_fixture().await;
    let mut candidate=library::get_track(&f.s,id.clone()).await.unwrap();
    candidate.source="mockradio".into();
    candidate.id="mockradio:full".into();
    candidate.title="新云端曲目".into();
    candidate.recording_id=String::new();
    let mut preview=candidate.clone();preview.id="mockradio:preview".into();preview.title="试听曲目".into();
    let dir=f.s.config.plugin_dir.join("mockradio");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),r#"{"id":"mockradio","name":"测试云端","capabilities":["search","resolve","recommendations"]}"#).unwrap();
    std::fs::write(dir.join("tracks.json"),json!({"items":[candidate,preview]}).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),r#"import json,sys,pathlib,time
for line in sys.stdin:
 r=json.loads(line)
 if r['method']=='search':
  time.sleep(1.15)
  pathlib.Path(__file__).with_name('searches').open('a').write(r['params']['query']+'\n')
  v=json.loads(pathlib.Path(__file__).with_name('tracks.json').read_text())
 elif r['method']=='resolve':
  time.sleep(1.10)
  v={'url':'https://audio.invalid/radio.flac','format':'flac','preview':r['params']['track_id'].endswith(':preview')}
 else:
  raise AssertionError(r['method'])
 print(json.dumps({'id':r['id'],'result':v}),flush=True)
"#).unwrap();
    // Give the account a seed with searchable artist metadata.
    f.s.db.run({let id=id.clone();move |c|{c.execute("UPDATE tracks SET artist='Test Artist' WHERE id=?1",[id])?;Ok(())}}).await.unwrap();
    let path=format!("/api/v1/recommendations/discover?source=cloud&seed_track_id={id}");
    let mixed="/api/v1/recommendations/discover?quick=true";
    let (status,first)=tokio::time::timeout(std::time::Duration::from_secs(1),f.call("GET",mixed,None)).await.unwrap();
    assert_eq!(status,StatusCode::OK,"{first}");
    assert_eq!(first["refreshing"],true);
    assert!(first["items"].as_array().unwrap().iter().any(|v|v["source"]=="local"),"{first}");
    let (status,first)=f.call("GET",&format!("{path}&quick=true"),None).await;
    assert_eq!(status,StatusCode::OK,"{first}");
    assert_eq!(first["refreshing"],true);
    assert_eq!(first["items"],json!([]));
    let (a,b)=tokio::join!(f.call("GET",&path,None),f.call("GET",&path,None));
    for (status,result) in [a,b] {
        assert_eq!(status,StatusCode::OK,"{result}");
        assert_eq!(result["source"],"cloud");
        assert_eq!(result["items"].as_array().unwrap().iter().map(|v|v["id"].as_str().unwrap()).collect::<Vec<_>>(),["mockradio:full"]);
    }
    let searches=std::fs::read_to_string(dir.join("searches")).unwrap();
    let terms=searches.lines().collect::<std::collections::HashSet<_>>();
    assert!(terms.contains("Test Artist") && terms.len()<=4);
    assert_eq!(searches.lines().count(),terms.len(),"concurrent reads must share each search");
    let (status,warm)=tokio::time::timeout(std::time::Duration::from_secs(1),f.call("GET",&format!("{path}&quick=true"),None)).await.unwrap();
    assert_eq!(status,StatusCode::OK,"{warm}");
    assert_ne!(warm["refreshing"],true);
    assert_eq!(warm["items"][0]["id"],"mockradio:full");
    let(_,local)=f.call("GET","/api/v1/recommendations/discover?source=local",None).await;
    assert!(local["items"].as_array().unwrap().iter().all(|v|v["source"]=="local"));
}

#[tokio::test]
async fn portable_mac_output_keeps_its_home_zone_and_controls() {
    let f = Fixture::new().await;
    f.s.db.run(|db| {
        db.execute("INSERT INTO devices(id,name,platform,outputs) VALUES('mac-host','Mac','macos',?1)",
            [r#"[{"id":"usb-audio","name":"USB DAC","kind":"coreaudio"}]"#])?;
        Ok(())
    }).await.unwrap();
    let (status, _) = f.call("PUT", "/api/v1/auth/sessions/current-device", Some(json!({"id":"mac-host","name":"Mac","type":"macos"}))).await;
    assert!(status.is_success());
    let home = crate::models::Zone::new("home-zone".into(),"USB DAC".into(),"usb-audio".into());
    f.s.save_zone(home.clone()).await.unwrap();
    let mut snapshot = json!(home);
    snapshot["id"] = json!("travel-usb-audio");
    snapshot["status"] = json!("paused");
    snapshot["position"] = json!(47.);
    let state = "/api/v1/portable/portable-mac/state";
    let (status, _) = f.call("PUT",state,Some(json!({"platform":"macos","zone":snapshot,"stamp":1}))).await;
    assert_eq!(status,StatusCode::NO_CONTENT);
    let (_, list) = f.call("GET","/api/v1/zones?view=state",None).await;
    let rows = list["items"].as_array().unwrap();
    assert_eq!(rows.iter().filter(|z| z["endpoint_id"]=="usb-audio").count(),1);
    assert!(!rows.iter().any(|z| z["id"]=="portable-mac"));
    let (_, zone) = f.call("GET","/api/v1/zones/home-zone",None).await;
    assert_eq!((zone["id"].as_str(),zone["position"].as_f64()),(Some("home-zone"),Some(47.)));
    let mut events = f.s.events.subscribe();
    let (response,ack) = tokio::join!(f.call("POST","/api/v1/zones/home-zone/seek",Some(json!({"position":52}))), async {
        let command = tokio::time::timeout(std::time::Duration::from_secs(3),async {
            loop { let event=events.recv().await.unwrap(); if event["type"]=="portable_command" {break event;} }
        }).await.unwrap();
        assert_eq!(command["data"]["output"],"usb-audio");
        snapshot["position"]=json!(52.);
        f.call("POST","/api/v1/portable/portable-mac/ack",Some(json!({"request_id":command["data"]["request_id"],"stamp":2,"zone":snapshot}))).await
    });
    assert_eq!(ack.0,StatusCode::NO_CONTENT);
    assert_eq!(response.1["id"],"home-zone");assert_eq!(response.1["position"],52.);
    assert_eq!(f.call("DELETE",state,None).await.0,StatusCode::NO_CONTENT);
    assert_eq!(f.call("GET","/api/v1/zones/home-zone",None).await.1["id"],"home-zone");
}

#[tokio::test]
async fn portable_control_waits_for_the_owning_device_acknowledgement() {
    let f=Fixture::new().await;
    let mut zone=json!(crate::models::Zone::new("local".into(),"Phone".into(),"local".into()));
    let unloaded=json!({"id":"pending","title":"正在加载歌曲","artist":"","album":"","duration":0,"format":"unknown","sample_rate":0,"channels":0});
    let mut cached=unloaded.clone();cached["id"]=json!("cached");cached["bit_depth"]=Value::Null;
    zone["queue"]=json!([unloaded,cached]);
    zone["track"]=zone["queue"][0].clone();
    let (status,value)=f.call("PUT","/api/v1/portable/portable-phone/state",Some(json!({"platform":"android","zone":zone,"stamp":1}))).await;
    assert_eq!(status,StatusCode::NO_CONTENT,"{value}");
    let (status,value)=f.call("GET","/api/v1/zones?view=state",None).await;
    assert_eq!(status,StatusCode::OK,"{value}");
    assert!(value["items"].as_array().unwrap().iter().any(|row|row["id"]=="portable-phone"));
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address=listener.local_addr().unwrap();
    let app=f.app.clone();
    let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
    let mut request=format!("ws://{address}/api/v1/events").into_client_request().unwrap();
    request.headers_mut().insert("Authorization",format!("Bearer {}",f.token).parse().unwrap());
    let (mut socket,_)=tokio_tungstenite::connect_async(request).await.unwrap();
    assert!(socket.next().await.unwrap().unwrap().is_text());
    let (received,ack)=tokio::join!(f.call("POST","/api/v1/zones/portable-phone/pause",Some(json!({}))),async {
        let command=tokio::time::timeout(std::time::Duration::from_secs(2),async {
            loop {
                let frame=socket.next().await.unwrap().unwrap();
                if frame.is_text() {
                    let event:Value=serde_json::from_str(frame.to_text().unwrap()).unwrap();
                    if event["type"]=="portable_command"{break event;}
                }
            }
        }).await.unwrap();
        assert!(command["data"].get("session").is_none());
        assert_eq!(command["type"],"portable_command");
        assert_eq!(command["data"]["action"],"pause");
        let mut snapshot=zone;
        snapshot["status"]=json!("paused");
        f.call("POST","/api/v1/portable/portable-phone/ack",Some(json!({"request_id":command["data"]["request_id"],"stamp":2,"zone":snapshot}))).await
    });
    assert_eq!(ack.0,StatusCode::NO_CONTENT,"{ack:?}");
    assert_eq!(received.0,StatusCode::OK,"{received:?}");
    assert_eq!(received.1["status"],"paused");
    assert_eq!(received.1["queue"][0]["bit_depth"],0);
    assert_eq!(received.1["queue"][1]["bit_depth"],0);
    server.abort();
}

#[tokio::test]
async fn platform_shelves_accept_a_non_qq_plugin_without_polluting_personal_radio() {
    let f=Fixture::new().await;
    let (id,_)=f.add_audio().await;
    let mut track=crate::library::get_track(&f.s,id).await.unwrap();
    track.id="testfeed:track-1".into();
    track.source="testfeed".into();
    track.recording_id="testfeed:recording-1".into();
    let dir=f.s.config.plugin_dir.join("testfeed");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),r#"{"id":"testfeed","name":"自建音源","capabilities":["search","recommendations","daily_recommendations_v1"]}"#).unwrap();
    std::fs::write(dir.join("track.json"),serde_json::to_vec(&track).unwrap()).unwrap();
    std::fs::write(dir.join("provider.py"),r#"import json, sys
from pathlib import Path
directory = Path(__file__).parent
track = json.loads((directory / "track.json").read_text())
for line in sys.stdin:
    call = json.loads(line)
    if call["method"] == "search":
        result = {"items": [track], "total": 1}
    else:
        assert call["method"] == "recommendations"
        assert call["params"]["page"] == 1
        with (directory / "requests").open("a") as requests:
            print("refresh", file=requests)
        mode = (directory / "mode").read_text()
        if mode == "offline":
            print(json.dumps({"id":call["id"], "error":{"code":"offline","message":"fixture unavailable"}}), flush=True)
            continue
        result = {"sections": [{"id":"featured","kind":"tracks","title":mode,"items":[track]}], "next_page": 2}
    print(json.dumps({"id":call["id"], "result":result}), flush=True)
"#).unwrap();
    std::fs::write(dir.join("mode"),"独立精选").unwrap();
    f.s.db.run(|c|{c.execute("INSERT INTO provider_accounts VALUES('testfeed','fixture-account',unixepoch())",[])?;Ok(())}).await.unwrap();
    crate::recommendations::refresh_daily_due(&f.s).await;
    let (status,feed)=f.call("GET","/api/v1/providers/testfeed/recommendations",None).await;
    assert_eq!(status,StatusCode::OK,"{feed}");
    assert_eq!(feed["next_page"],2);
    assert_eq!(feed["sections"][0]["items"][0]["source"],"testfeed");
    let (status,value)=f.call("GET","/api/v1/library/tracks/testfeed:track-1",None).await;
    assert_eq!(status,StatusCode::OK,"{value}");
    let (status,value)=f.call("GET","/api/v1/recommendations/discover",None).await;
    assert_eq!(status,StatusCode::OK,"{value}");
    assert!(!value["items"].as_array().unwrap().iter().any(|row|row["source"]=="testfeed"));
    let request_count=||std::fs::read_to_string(dir.join("requests")).unwrap().lines().count();
    assert_eq!(request_count(),1,"the scheduler should have warmed the page");
    f.s.plugin.stop("testfeed").await;
    let (status,again)=f.call("GET","/api/v1/providers/testfeed/recommendations",None).await;
    assert_eq!((status,again),(StatusCode::OK,feed.clone()));
    assert_eq!(request_count(),1,"a plugin restart must not slow down cached reads");

    f.s.db.run(|c|{c.execute("UPDATE provider_daily_recommendations SET day='2000-01-01' WHERE provider='testfeed'",[])?;Ok(())}).await.unwrap();
    std::fs::write(dir.join("mode"),"offline").unwrap();
    crate::recommendations::refresh_daily_due(&f.s).await;
    assert_eq!(f.call("GET","/api/v1/providers/testfeed/recommendations",None).await.1,feed,"an upstream failure must keep the old feed");
    assert_eq!(request_count(),2);
    std::fs::write(dir.join("mode"),"今日精选").unwrap();
    crate::recommendations::refresh_daily_due(&f.s).await;
    assert_eq!(f.call("GET","/api/v1/providers/testfeed/recommendations",None).await.1["sections"][0]["title"],"今日精选");
    assert_eq!(request_count(),3);

    f.s.db.run(|c|crate::provider_session::bump(c,"testfeed")).await.unwrap();
    std::fs::write(dir.join("mode"),"新账号精选").unwrap();
    assert_eq!(f.call("GET","/api/v1/providers/testfeed/recommendations",None).await.1["sections"][0]["title"],"新账号精选");
    assert_eq!(request_count(),4);
    f.s.plugin.stop("testfeed").await;
}

#[tokio::test]
async fn personal_picks_daily_is_shared_across_reads_and_similar_needs_seed_evidence() {
    let f=Fixture::new().await;
    f.s.db.run(|c|{
        for (id,artist,style,rec) in [("seed","A","jazz","recording-A"),("alias","A","jazz","recording-A"),("neighbour","B","jazz","recording-B"),("other","C","folk","recording-C")] {
            c.execute("INSERT INTO tracks(id,title,artist,album,album_artist,source,format,recording_id,created_at,provider_data) VALUES(?1,?1,?2,'','', 'local','flac',?3,0,?4)",rusqlite::params![id,artist,rec,json!({"genre":style}).to_string()])?;
        }
        Ok(())
    }).await.unwrap();
    let (status,similar)=f.call("POST","/api/v1/recommendations/similar",Some(json!({"track_id":"seed"}))).await;
    assert_eq!(status,StatusCode::OK,"{similar}");
    assert_eq!(similar["items"].as_array().unwrap().iter().map(|i|&i["id"]).collect::<Vec<_>>(),[&json!("neighbour")]);
    assert_eq!(f.call("POST","/api/v1/recommendations/similar",Some(json!({"track_id":"unknown"}))).await.0,StatusCode::NOT_FOUND);
    let daily="/api/v1/recommendations/daily";
    let (a,b)=tokio::join!(f.call("GET",daily,None),f.call("GET",daily,None));
    assert_eq!(a.0,StatusCode::OK,"{}",a.1);assert_eq!(a,b);
    let order=a.1["items"].as_array().unwrap().iter().map(|v|v["id"].clone()).collect::<Vec<_>>();
    assert_eq!(order.len(),3);
    f.s.db.run(|c|{c.execute("INSERT INTO tracks(id,title,artist,album,album_artist,source,format,recording_id,created_at) VALUES('extra','Extra','Artist','','','local','flac','extra',100)",[])?;Ok(())}).await.unwrap();
    assert_eq!(a.1,f.call("GET",daily,None).await.1);
    f.s.db.run(|c|{
        c.execute("INSERT INTO users(id,username,password,role) VALUES('listener','listener','','member')",[])?;
        c.execute("INSERT INTO recommendation_feedback(user_id,track_id) SELECT id,'neighbour' FROM users WHERE id<>'listener'",[])?;
        Ok(())
    }).await.unwrap();
    let (filtered,())=tokio::join!(f.call("GET",daily,None),crate::radio::personal::refresh_due(&f.s));
    assert!(filtered.1["items"].as_array().unwrap().iter().all(|v|v["id"]!="neighbour"));
    assert!(f.s.db.read(|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM personal_daily_lists d,json_each(d.picks) p WHERE user_id='listener' AND json_extract(p.value,'$.id')='neighbour')",[],|r|r.get::<_,bool>(0))?)).await.unwrap());
    assert_eq!(f.s.db.read(|c|Ok(c.query_row("SELECT COUNT(*) FROM personal_daily_lists",[],|r|r.get::<_,i64>(0))?)).await.unwrap(),2);
}

#[tokio::test]
async fn personal_picks_ai_searches_unheard_artist_and_verifies_catalogue_recording() {
    let f=Fixture::new().await;
    let id=f.add_audio().await.0;
    f.s.db.run({let id=id.clone();move|c|{c.execute("UPDATE tracks SET artist='Seed Artist' WHERE id=?1",[id])?;Ok(())}}).await.unwrap();
    let mut novel=library::get_track(&f.s,id.clone()).await.unwrap();
    novel.id="newsource:one".into();novel.source="newsource".into();novel.artist="Unknown Ensemble".into();
    novel.title="New Recording".into();novel.recording_id="remote-new".into();
    let dir=f.s.config.plugin_dir.join("newsource");std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"),r#"{"id":"newsource","capabilities":["search","resolve"]}"#).unwrap();
    std::fs::write(dir.join("tracks.json"),json!({"items":[novel]}).to_string()).unwrap();
    std::fs::write(dir.join("provider.py"),r#"import sys,json,pathlib
for line in sys.stdin:
 r=json.loads(line); p=r["params"]
 if r["method"]=="search":
  v=json.loads(pathlib.Path(__file__).with_name("tracks.json").read_text()) if p["query"]=="Unknown Ensemble New Recording" else {"items":[]}
 elif r["method"]=="resolve": v={"url":"https://audio.invalid/new.flac","format":"flac"}
 else: raise ValueError(r["method"])
 print(json.dumps({"id":r["id"],"result":v}),flush=True)
"#).unwrap();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base=format!("http://{}/v1",listener.local_addr().unwrap());
    let app=Router::new().route("/v1/chat/completions",axum::routing::post(|axum::Json(request):axum::Json<Value>|async move {
        let prompt=request["messages"][1]["content"].as_str().unwrap();
        let reply=if prompt.contains("not found") && prompt.contains("Ghost") {
            r#"{"songs":[{"title":"New Recording","artist":"Unknown Ensemble"}]}"#
        }else{r#"{"songs":[{"title":"Invented","artist":"Ghost"}]}"#};
        axum::Json(json!({"choices":[{"message":{"content":reply}}]}))
    }));
    let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap()});
    assert_eq!(f.call("PUT","/api/v1/llm/settings",Some(json!({"expected_revision":"0","enabled":true,"base_url":base,"model":"test"}))).await.0,StatusCode::OK);
    f.call("PUT","/api/v1/recommendations/settings",Some(json!({"advanced":true}))).await;
    let (status,v)=f.call("POST","/api/v1/recommendations/similar",Some(json!({"track_id":id}))).await;
    server.abort();assert_eq!(status,StatusCode::OK,"{v}");
    assert_eq!(v["items"].as_array().unwrap().len(),1,"{v}");assert_eq!(v["items"][0]["id"],"newsource:one");
}

#[tokio::test]
async fn personal_picks_online_tags_discover_unheard_tracks_without_ai() {
    let f = Fixture::new().await;
    f.s.db.run(|c| {
        c.execute("INSERT INTO tracks(id,title,artist,album,album_artist,source,format,recording_id,created_at) VALUES('atlas:seed','Original','Seed artist','','','atlas','flac','seed',0)", [])?;
        Ok(())
    }).await.unwrap();
    let dir = f.s.config.plugin_dir.join("atlas");
    std::fs::create_dir_all(&dir).unwrap();
    let mut song = library::get_track(&f.s, "atlas:seed".into()).await.unwrap();
    song.id = "atlas:unheard".into();
    song.recording_id = "unheard".into();
    song.title = "Neighbour".into();
    song.artist = "Other".into();
    song.provider_data = json!({"genre":"dream pop"});
    let mut preview = song.clone();
    preview.id = "atlas:preview".into();
    preview.recording_id = "preview".into();
    preview.title = "Preview only".into();
    std::fs::write(dir.join("tracks.json"), json!({"items":[song,preview]}).to_string()).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"atlas","capabilities":["search","resolve","metadata"]}"#).unwrap();
    std::fs::write(dir.join("provider.py"),r#"import sys,json,pathlib
for line in sys.stdin:
 r=json.loads(line); p=r["params"]
 if r["method"]=="metadata" and p["section"]=="basic":
  v={"matched":True,"source":"atlas","track":{"id":"atlas:seed"},"background":{"genre":"Dream Pop"}}
 elif r["method"]=="search":
  v=json.loads(pathlib.Path(__file__).with_name("tracks.json").read_text()) if p["query"]=="dream pop" else {"items":[]}
 elif r["method"]=="resolve":
  v={"url":"https://audio.invalid/new.flac","format":"flac","preview":p["track_id"]=="atlas:preview"}
 else: raise ValueError(r["method"])
 print(json.dumps({"id":r["id"],"result":v}),flush=True)
"#).unwrap();
    let (status, result)=f.call("POST","/api/v1/recommendations/similar",Some(json!({"track_id":"atlas:seed"}))).await;
    assert_eq!(status,StatusCode::OK,"{result}");
    assert_eq!(result["items"].as_array().unwrap().len(),1,"{result}");
    assert_eq!(result["items"][0]["id"],"atlas:unheard","{result}");
    assert_eq!(result["reasons"]["atlas:unheard"],"曲风相近");
    f.s.plugin.stop("atlas").await;
}
