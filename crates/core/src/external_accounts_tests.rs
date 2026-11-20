use super::*;
async fn enable(f:&Fixture){f.s.db.run(|c|{c.execute("INSERT INTO settings(key,value) VALUES('external_accounts_development','true') ON CONFLICT(key) DO UPDATE SET value='true'",[])?;Ok(())}).await.unwrap();}
fn create_payload(name:&str)->Value{json!({"username":name,"password":"external-test-password","current_password":"correct-test-password"})}
async fn create(f:&Fixture,name:&str)->(String,String){
 let (status,user)=f.call("POST","/api/v1/auth/external-accounts",Some(create_payload(name))).await;assert_eq!(status,StatusCode::OK,"{user}");
 let (status,login)=call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":name,"password":"external-test-password"}))).await;assert_eq!(status,StatusCode::OK,"{login}");
 (user["id"].as_str().unwrap().into(),login["token"].as_str().unwrap().into())
}
#[tokio::test]async fn external_accounts_can_be_disabled_and_default_source_scope_denies_all(){
 let f=Fixture::new().await;f.s.db.run(|c|{c.execute("INSERT INTO settings(key,value) VALUES('external_accounts_development','false')",[])?;Ok(())}).await.unwrap();
 assert_eq!(f.call("POST","/api/v1/auth/external-accounts",Some(create_payload("visitor"))).await.0,StatusCode::FORBIDDEN);
 enable(&f).await;let(id,token)=create(&f,"visitor").await;
 let(status,users)=f.call("GET","/api/v1/auth/external-accounts",None).await;assert_eq!(status,StatusCode::OK);let u=&users["items"][0];
 assert_eq!(u["id"],id);assert_eq!(u["policy"],json!({"providers":[],"local_cache":false,"local_library":false,"bandwidth_bytes_per_second":0}));
 for key in ["password","totp_secret","totp_pending","recovery","token"]{assert!(u.get(key).is_none());}
 assert_eq!(call(&f.app,"GET","/api/v1/auth/me",Some(&token),None).await.1["role"],"external");
 for path in ["/api/v1/auth/users","/api/v1/sync/starts","/api/v1/output-groups","/api/v1/maintenance/backups"]{assert_eq!(call(&f.app,"GET",path,Some(&token),None).await.0,StatusCode::FORBIDDEN,"{path}");}
 for path in ["/api/v1/library/tracks","/api/v1/devices","/api/v1/zones","/api/v1/events/ticket","/api/v1/stats","/api/v1/settings"]{let(status,value)=call(&f.app,"GET",path,Some(&token),None).await;assert_eq!(status,StatusCode::OK,"{path}: {value}");}
 let ticket=f.s.ticket_for("media:test".into(),60,Some(auth::hash(&token))).await;
 assert_eq!(call(&f.app,"GET",&format!("/api/v1/media/test?ticket={ticket}"),None,None).await.0,StatusCode::NOT_FOUND);
 assert_eq!(call(&f.app,"GET","/api/v1/status",Some(&token),None).await.0,StatusCode::OK);
}
#[tokio::test]async fn external_account_admin_changes_reauthenticate_and_revoke_old_sessions(){
 let f=Fixture::new().await;enable(&f).await;let(id,old)=create(&f,"visitor").await;let path=format!("/api/v1/auth/external-accounts/{id}");
 let (_,named)=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"automation","scopes":["admin"]}))).await;
 let body=json!({"username":"new-visitor","new_password":"new-external-password","reset_totp":true,"policy":{"providers":["qqmusic"],"local_cache":true,"bandwidth_bytes_per_second":65536},"current_password":"correct-test-password"});
 assert_eq!(call(&f.app,"PATCH",&path,Some(named["token"].as_str().unwrap()),Some(body.clone())).await.0,StatusCode::FORBIDDEN);
 let mut bad=body.clone();bad["current_password"]=json!("incorrect-password");assert_eq!(f.call("PATCH",&path,Some(bad)).await.0,StatusCode::BAD_REQUEST);
 let(status,user)=f.call("PATCH",&path,Some(body)).await;assert_eq!(status,StatusCode::OK,"{user}");assert_eq!(user["username"],"new-visitor");assert_eq!(user["policy"]["providers"],json!(["qqmusic"]));
 assert_eq!(call(&f.app,"GET","/api/v1/auth/me",Some(&old),None).await.0,StatusCode::UNAUTHORIZED);
 assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"new-visitor","password":"external-test-password"}))).await.0,StatusCode::UNAUTHORIZED);
 assert_eq!(call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"new-visitor","password":"new-external-password"}))).await.0,StatusCode::OK);
}
#[tokio::test]async fn external_account_deletion_is_confirmed_and_does_not_touch_family_data(){
 let f=Fixture::new().await;enable(&f).await;let(id,token)=create(&f,"visitor").await;
 f.s.db.run({let id=id.clone();move|c|{c.execute("INSERT INTO preferences(user_id,value) VALUES(?1,'{}')",[&id])?;c.execute("INSERT INTO playlists(id,name,user_id) VALUES('external-list','private',?1)",[&id])?;c.execute("INSERT INTO playlists(id,name,user_id) VALUES('family-list','family','family-user')",[])?;Ok(())}}).await.unwrap();
 let path=format!("/api/v1/auth/external-accounts/{id}");assert_eq!(f.call("DELETE",&path,Some(json!({"confirmation":"wrong","current_password":"correct-test-password"}))).await.0,StatusCode::BAD_REQUEST);
 assert_eq!(f.call("DELETE",&path,Some(json!({"confirmation":"visitor","current_password":"correct-test-password"}))).await.0,StatusCode::OK);
 assert_eq!(call(&f.app,"GET","/api/v1/auth/me",Some(&token),None).await.0,StatusCode::UNAUTHORIZED);
 let left=f.s.db.read(move|c|Ok((c.query_row("SELECT COUNT(*) FROM playlists WHERE id='family-list'",[],|r|r.get::<_,i64>(0))?,c.query_row("SELECT COUNT(*) FROM playlists WHERE user_id=?1",[&id],|r|r.get::<_,i64>(0))?))).await.unwrap();assert_eq!(left,(1,0));
}
#[tokio::test]async fn external_account_invalid_scope_and_case_collisions_are_rejected(){
 let f=Fixture::new().await;enable(&f).await;create(&f,"Visitor").await;
 assert_eq!(f.call("POST","/api/v1/auth/external-accounts",Some(create_payload("visitor"))).await.0,StatusCode::BAD_REQUEST);
 let mut bad=create_payload("second");bad["policy"]=json!({"providers":["qqmusic') OR 1=1 --"]});assert_eq!(f.call("POST","/api/v1/auth/external-accounts",Some(bad)).await.0,StatusCode::BAD_REQUEST);
}
#[tokio::test]async fn external_account_archive_replays_only_new_personal_contributions(){
 let f=Fixture::new().await;enable(&f).await;let(id,_)=create(&f,"visitor").await;
 f.s.db.run(move|c|{use crate::accounts::archive::{records,Record,StatEntry,Stat,Origin,Playlist};let origin=Origin{core:"source-core".into(),user:"source-user".into(),id:"bucket-1".into()};
  let stat=Stat{revision:0,minute:100,track_id:"song".into(),title:"Song".into(),artist:"Artist".into(),cover_url:None,play_count:1,skip_count:0,play_seconds:30.,last_played_at:Some(6000)};
  let tx=c.transaction()?;
  for _ in 0..2{records::apply_record(&tx,&id,"core","source-core","source-user",Record::Stat{data:StatEntry{origin:origin.clone(),record:stat.clone()}})?;}
  let mut next=stat;next.play_seconds=45.;records::apply_record(&tx,&id,"core","source-core","source-user",Record::Stat{data:StatEntry{origin:origin.clone(),record:next}})?;
  let(plays,seconds):(i64,f64)=tx.query_row("SELECT play_count,play_seconds FROM account_track_stats WHERE user_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?)))?;assert_eq!((plays,seconds),(1,45.));
  assert_eq!(tx.query_row("SELECT COUNT(*) FROM track_stats",[],|r|r.get::<_,i64>(0))?,0);
  for tracks in [vec!["a".into(),"b".into()],vec!["b".into(),"c".into()]]{records::apply_record(&tx,&id,"core","source-core","source-user",Record::Playlist{data:Playlist{id:"shared-id".into(),name:"Favorites".into(),track_ids:tracks,smart_query:None,favorites:true}})?;}
  let tracks:String=tx.query_row("SELECT tracks FROM playlists WHERE user_id=?1",[&id],|r|r.get(0))?;assert_eq!(serde_json::from_str::<Value>(&tracks)?,json!(["a","b","c"]));
  records::apply_record(&tx,&id,"core","source-core","source-user",Record::Preferences{data:json!({"theme":"dark","token":"secret","nested":{"password":"secret","value":2}})})?;
  let prefs:String=tx.query_row("SELECT value FROM preferences WHERE user_id=?1",[id],|r|r.get(0))?;assert!(!prefs.contains("secret"));tx.commit()?;Ok(())}).await.unwrap();
}
#[tokio::test]async fn external_account_boundary_never_upgrades_media_tickets_to_api_credentials(){
 let f=Fixture::new().await;
 let ticket=f.s.ticket_for("media:song".into(),60,Some(auth::hash(&f.token))).await;
 for bearer in [None,Some("invalid-bearer")] {
  assert_eq!(call(&f.app,"GET",&format!("/api/v1/auth/users?ticket={ticket}"),bearer,None).await.0,StatusCode::UNAUTHORIZED);
 }
}
#[tokio::test]async fn external_account_import_requires_explicit_target_and_preserves_archive_identity(){
 let f=Fixture::new().await;enable(&f).await;
 let(status,job)=f.call("POST","/api/v1/auth/account-imports",Some(json!({"current_password":"correct-test-password"}))).await;assert_eq!(status,StatusCode::OK,"{job}");let id=job["id"].as_str().unwrap();
 let archive=[json!({"kind":"manifest","schema":1,"origin_core":"other-core","origin_user":"other-user","username":"visitor"}),json!({"kind":"playlist","data":{"id":"playlist-id","name":"Imported","track_ids":["track-a"],"smart_query":null,"favorites":false}}),json!({"kind":"preferences","data":{"theme":"dark","token":"not-allowed"}}),json!({"kind":"music_preference","data":{"type":"version","entity_id":"unresolved-recording","track_id":"unresolved-track","revision":3}})].iter().map(|v|format!("{v}\n")).collect::<String>();
 let request=Request::builder().method("PUT").uri(format!("/api/v1/auth/account-imports/{id}/file")).header("Authorization",format!("Bearer {}",f.token)).header("Content-Type","application/x-ndjson").body(Body::from(archive)).unwrap();
 let response=f.app.clone().oneshot(request).await.unwrap();let status=response.status();let bytes=to_bytes(response.into_body(),1024*1024).await.unwrap();assert_eq!(status,StatusCode::OK,"{}",String::from_utf8_lossy(&bytes));
 let path=format!("/api/v1/auth/account-imports/{id}/apply");let base=json!({"current_password":"correct-test-password","mode":"merge"});assert_eq!(f.call("POST",&path,Some(base)).await.0,StatusCode::BAD_REQUEST);
 let(status,result)=f.call("POST",&path,Some(json!({"current_password":"correct-test-password","mode":"new","username":"imported-visitor","password":"import-test-password"}))).await;assert_eq!(status,StatusCode::OK,"{result}");
 let mut result=result;for _ in 0..100 {if result["state"]=="completed"{break;}tokio::task::yield_now().await;result=f.call("GET",&format!("/api/v1/auth/account-imports/{id}"),None).await.1;}
 assert_eq!(result["state"],"completed","{result}");let user=result["target_user"].as_str().unwrap();
 let(status,export)=f.call("POST",&format!("/api/v1/auth/external-accounts/{user}/export-ticket"),Some(json!({"current_password":"correct-test-password"}))).await;assert_eq!(status,StatusCode::OK,"{export}");
 let response=f.app.clone().oneshot(Request::builder().uri(format!("/{}",export["api_path"].as_str().unwrap())).body(Body::empty()).unwrap()).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
 let bytes=to_bytes(response.into_body(),1024*1024).await.unwrap();let text=String::from_utf8(bytes.to_vec()).unwrap();assert!(text.contains("Imported"));assert!(text.contains("unresolved-recording"));for line in text.lines(){let record:crate::accounts::archive::Record=serde_json::from_str(line).unwrap();assert!(record.valid(),"{line}");}assert!(!text.contains("not-allowed"));assert!(!text.contains("import-test-password"));
}
#[tokio::test]async fn stats_all_time_includes_old_data_and_bounds_chart(){
 let f=Fixture::new().await;
 f.s.db.run(|c|{c.execute("INSERT INTO daily_stats(day,track_id,title,artist,play_count,play_seconds) VALUES('2000-01-01','old','Old','QA',1,60),('2026-10-30','new','New','QA',1,90)",[])?;Ok(())}).await.unwrap();
 let(status,value)=f.call("GET","/api/v1/stats?period_days=0",None).await;assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["all_time"],true);assert_eq!(value["summary"]["listened_seconds"],150.);assert_eq!(value["summary"]["active_days"],2);assert_eq!(value["start_day"],"2000-01-01");assert!(value["days"].as_array().unwrap().len()<=366);
 assert_eq!(f.call("GET","/api/v1/stats?period_days=3651",None).await.0,StatusCode::BAD_REQUEST);
}
#[tokio::test]async fn lyrics_concurrent_requests_persist_and_do_not_repeat_upstream(){
 let f=Fixture::new().await;let(local,_)=f.add_audio().await;let mut track=library::get_track(&f.s,local).await.unwrap();track.id="fake:song".into();track.source="fake".into();track.provider_data=json!({});
 f.s.db.run(move|c|library::upsert(c,&track,None,None,0,0,None,None)).await.unwrap();
 let dir=f.s.config.plugin_dir.join("fake");std::fs::create_dir_all(&dir).unwrap();std::fs::write(dir.join("manifest.json"),r#"{"id":"fake","version":"1","enabled":true,"capabilities":["lyrics","isolated_lyrics_v1"]}"#).unwrap();
 std::fs::write(dir.join("provider.py"),r#"import sys,json,time,pathlib,os
for line in sys.stdin:
 r=json.loads(line)
 if r['method']=='lyrics':
  with pathlib.Path('requests').open('a') as f:f.write('lyrics\n')
  time.sleep(.15)
  result={'format':'lrc','text':'[00:01]Real','timing_source':'test','word_timing':[{'time':1,'text':'Real','words':[{'time':1,'duration':.4,'text':'Real'}]}]}
 else:result={}
 print(json.dumps({'id':r['id'],'result':result}),flush=True)
"#).unwrap();
 let(a,b,c)=tokio::join!(f.call("GET","/api/v1/library/tracks/fake:song/lyrics",None),f.call("GET","/api/v1/library/tracks/fake:song/lyrics",None),f.call("GET","/api/v1/library/tracks/fake:song/lyrics",None));
 assert_eq!(a.0,StatusCode::OK,"{:?}",a.1);assert_eq!(a.1,b.1);assert_eq!(b.1,c.1);assert_eq!(a.1["word_timing"][0]["words"][0]["duration"],0.4);
 assert_eq!(std::fs::read_to_string(dir.join("requests")).unwrap().lines().count(),1);
 let count=f.s.db.read(|c|Ok(c.query_row("SELECT COUNT(*) FROM lyric_documents",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(count,1);
 assert_eq!(f.call("GET","/api/v1/library/tracks/fake:song/lyrics",None).await.1,a.1);assert_eq!(std::fs::read_to_string(dir.join("requests")).unwrap().lines().count(),1);
 f.s.db.run(|c|{crate::provider_session::bump(c,"fake")?;Ok(())}).await.unwrap();assert_eq!(f.call("GET","/api/v1/library/tracks/fake:song/lyrics",None).await.0,StatusCode::OK);assert_eq!(std::fs::read_to_string(dir.join("requests")).unwrap().lines().count(),2);
}

async fn visitor_with_local(f:&Fixture,name:&str)->(String,String){
 let mut body=create_payload(name);body["policy"]=json!({"local_library":true});
 let(status,user)=f.call("POST","/api/v1/auth/external-accounts",Some(body)).await;assert_eq!(status,StatusCode::OK,"{user}");
 let(_,login)=call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":name,"password":"external-test-password"}))).await;
 (user["id"].as_str().unwrap().into(),login["token"].as_str().unwrap().into())
}
#[tokio::test]async fn external_realms_isolate_inventory_control_and_sources(){
 let f=Fixture::new().await;enable(&f).await;f.collection_tracks().await;
 let(_,a)=visitor_with_local(&f,"guest-a").await;let(_,b)=visitor_with_local(&f,"guest-b").await;
 for(token,id)in [(&a,"browser-external-a"),(&b,"browser-external-b")]{let(status,v)=call(&f.app,"POST","/api/v1/endpoints/browser",Some(token),Some(json!({"id":id,"name":id}))).await;assert_eq!(status,StatusCode::OK,"{v}");}
 let(_,za)=call(&f.app,"GET","/api/v1/zones",Some(&a),None).await;let(_,zb)=call(&f.app,"GET","/api/v1/zones",Some(&b),None).await;
 assert_eq!(za["items"].as_array().unwrap().len(),1);assert_eq!(zb["items"].as_array().unwrap().len(),1);let zone=za["items"][0]["id"].as_str().unwrap();
 for token in [&b,&f.token]{assert_eq!(call(&f.app,"POST",&format!("/api/v1/zones/{zone}/pause"),Some(token),Some(json!({}))).await.0,StatusCode::NOT_FOUND);}
 assert_eq!(call(&f.app,"POST","/api/v1/endpoints/browser",Some(&b),Some(json!({"id":"browser-external-a","name":"stolen"}))).await.0,StatusCode::NOT_FOUND);
 let(_,devices)=f.call("GET","/api/v1/devices",None).await;assert!(devices["items"].as_array().unwrap().iter().all(|v|!v["id"].as_str().unwrap().starts_with("browser-external")));
 let(_,library)=call(&f.app,"GET","/api/v1/library/tracks?limit=100",Some(&a),None).await;assert!(library["items"].as_array().unwrap().iter().all(|v|v["source"]=="local"));
 assert_eq!(call(&f.app,"GET","/api/v1/library/tracks/qq-only/ticket",Some(&a),None).await.0,StatusCode::NOT_FOUND);
 assert_eq!(call(&f.app,"PUT",&format!("/api/v1/zones/{zone}/queue"),Some(&a),Some(json!({"track_ids":["qq-only"]}))).await.0,StatusCode::NOT_FOUND);
 assert_eq!(call(&f.app,"POST",&format!("/api/v1/zones/{zone}/transfer"),Some(&a),Some(json!({"target_zone_id":zb["items"][0]["id"],"expected_playback_id":null}))).await.0,StatusCode::NOT_FOUND);
 for path in ["/api/v1/sync/starts","/api/v1/output-groups/x"]{assert_eq!(call(&f.app,"POST",path,Some(&a),Some(json!({}))).await.0,StatusCode::FORBIDDEN);}
}
#[tokio::test]async fn external_confirmed_listening_and_archive_do_not_credit_family_or_other_accounts(){
 let f=Fixture::new().await;enable(&f).await;f.collection_tracks().await;let(uid,a)=visitor_with_local(&f,"guest-a").await;let(_,b)=visitor_with_local(&f,"guest-b").await;
 call(&f.app,"POST","/api/v1/endpoints/browser",Some(&a),Some(json!({"id":"browser-personal","name":"Personal"}))).await;
 let(_,zones)=call(&f.app,"GET","/api/v1/zones",Some(&a),None).await;let id=zones["items"][0]["id"].as_str().unwrap();
 let(status,value)=call(&f.app,"PUT",&format!("/api/v1/zones/{id}/queue"),Some(&a),Some(json!({"track_ids":["local-shared"]}))).await;assert_eq!(status,StatusCode::OK,"{value}");
 let mut z=f.s.zones.read().await[id].clone();z.track=z.queue.first().cloned();z.status="playing".into();stats::record_progress(&f.s,&z).await.unwrap();
 let zid=id.to_owned();f.s.db.run(move|c|{c.execute("UPDATE play_sessions SET last_report_at=last_report_at-6000 WHERE zone_id=?1",[zid])?;Ok(())}).await.unwrap();z.position=6.;stats::record_progress(&f.s,&z).await.unwrap();stats::finish(&f.s,&z,"stop").await.unwrap();
 for token in [&f.token,&b]{let(_,v)=call(&f.app,"GET","/api/v1/stats?period_days=0",Some(token),None).await;assert_eq!(v["summary"]["listened_seconds"],0.,"{v}");let(_,v)=call(&f.app,"GET","/api/v1/stats/history",Some(token),None).await;assert_eq!(v["items"],json!([]));}
 let(status,v)=call(&f.app,"GET","/api/v1/stats?period_days=0",Some(&a),None).await;assert_eq!(status,StatusCode::OK,"{v}");assert_eq!(v["summary"]["listened_seconds"],6.,"{v}");
 let(_,v)=call(&f.app,"GET","/api/v1/stats/history",Some(&a),None).await;assert_eq!(v["items"].as_array().unwrap().len(),1);assert_eq!(v["items"][0]["listened_seconds"],6.);
 let raw=f.s.db.read(move|c|Ok((c.query_row("SELECT COUNT(*) FROM account_stat_origins WHERE user_id=?1",[&uid],|r|r.get::<_,i64>(0))?,c.query_row("SELECT COUNT(*) FROM track_stats",[],|r|r.get::<_,i64>(0))?))).await.unwrap();assert_eq!(raw,(1,0));
}
#[tokio::test]async fn external_media_bandwidth_is_shared_by_concurrent_responses(){
 let f=Fixture::new().await;enable(&f).await;f.collection_tracks().await;
 let mut body=create_payload("limited");body["policy"]=json!({"local_library":true,"bandwidth_bytes_per_second":8192});assert_eq!(f.call("POST","/api/v1/auth/external-accounts",Some(body)).await.0,StatusCode::OK);
 let(_,login)=call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"limited","password":"external-test-password"}))).await;let token=login["token"].as_str().unwrap();
 let request=||Request::builder().uri("/api/v1/media/local-shared").header("authorization",format!("Bearer {token}")).header("range","bytes=0-8191").body(Body::empty()).unwrap();
 let start=std::time::Instant::now();let(a,b)=tokio::join!(f.app.clone().oneshot(request()),f.app.clone().oneshot(request()));
 let read=|response:axum::response::Response|async move{assert_eq!(response.status(),StatusCode::PARTIAL_CONTENT);assert_eq!(response.headers()["content-length"],"8192");to_bytes(response.into_body(),16384).await.unwrap()};
 let(a,b)=tokio::join!(read(a.unwrap()),read(b.unwrap()));assert_eq!(a.len()+b.len(),16384);assert!(start.elapsed().as_secs_f64()>=1.8,"{:?}",start.elapsed());assert!(start.elapsed().as_secs_f64()<6.);
}

#[tokio::test]async fn external_revocation_waits_for_admitted_operations_and_rejects_queued_old_sessions(){
 let f=Fixture::new().await;let(uid,token)=create(&f,"race-visitor").await;
 let lease=crate::accounts::leases::read(&f.s,&uid).await;
 let app=f.app.clone();let admin=f.token.clone();let path=format!("/api/v1/auth/external-accounts/{uid}");
 let update=tokio::spawn(async move{call(&app,"PATCH",&path,Some(&admin),Some(json!({"enabled":false,"current_password":"correct-test-password"}))).await});
 tokio::time::sleep(std::time::Duration::from_millis(30)).await;assert!(!update.is_finished());drop(lease);assert_eq!(update.await.unwrap().0,StatusCode::OK);
 assert_eq!(call(&f.app,"GET","/api/v1/zones",Some(&token),None).await.0,StatusCode::UNAUTHORIZED);
}
#[tokio::test]async fn external_ticket_pressure_does_not_revoke_family_tickets(){
 let f=Fixture::new().await;let(_,token)=create(&f,"ticket-visitor").await;
 let family=f.s.ticket_for("family-keep".into(),86400,Some(auth::hash(&f.token))).await;
 for i in 0..520{f.s.ticket_for(format!("visitor-{i}"),86400,Some(auth::hash(&token))).await;}
 assert!(f.s.check_ticket(&family,"family-keep").await);assert_eq!(f.s.tickets.lock().await.values().filter(|v|v.account.is_some()).count(),512);
}
#[tokio::test]async fn external_revisioned_archive_can_correct_a_skip_without_double_credit(){
 let f=Fixture::new().await;let(uid,_)=create(&f,"archive-visitor").await;
 f.s.db.run(move|c|{use crate::accounts::archive::{records,Record,StatEntry,Stat,Origin};let origin=Origin{core:"portable-core".into(),user:"portable-user".into(),id:"minute-track".into()};
  let tx=c.transaction()?;let first=Stat{revision:1,minute:100,track_id:"song".into(),title:"Song".into(),artist:"Artist".into(),cover_url:None,play_count:1,skip_count:1,play_seconds:30.,last_played_at:Some(6000)};
  let mut second=first.clone();second.revision=2;second.skip_count=0;second.play_seconds=31.;
  for record in [first.clone(),second.clone(),first,second]{records::apply_record(&tx,&uid,"this-core","portable-core","portable-user",Record::Stat{data:StatEntry{origin:origin.clone(),record}})?;}
  let result:(i64,i64,f64)=tx.query_row("SELECT play_count,skip_count,play_seconds FROM account_track_stats WHERE user_id=?1",[uid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;assert_eq!(result,(1,0,31.));tx.commit()?;Ok(())}).await.unwrap();
}

#[tokio::test]async fn external_catalogue_statistics_do_not_disclose_family_counts(){
 let f=Fixture::new().await;f.collection_tracks().await;let(_,token)=visitor_with_local(&f,"catalogue-visitor").await;
 f.s.db.run(|c|{c.execute("INSERT INTO track_stats(track_id,play_count,play_seconds) VALUES('local-shared',999,99999)",[])?;Ok(())}).await.unwrap();
 let(status,tracks)=call(&f.app,"GET","/api/v1/library/tracks?source=local",Some(&token),None).await;assert_eq!(status,StatusCode::OK,"{tracks}");
 for track in tracks["items"].as_array().unwrap(){assert_eq!(track["provider_data"]["catalog_display"]["play_count"],0,"{track}");}
 let(_,albums)=call(&f.app,"GET","/api/v1/library/albums?source=local&sort=plays",Some(&token),None).await;for album in albums["items"].as_array().unwrap(){assert_eq!(album["play_count"],0);}
 let album=albums["items"][0]["id"].as_str().unwrap();let(status,page)=call(&f.app,"GET",&format!("/api/v1/library/catalog/album/{album}?remote=false"),Some(&token),None).await;assert_eq!(status,StatusCode::OK,"{page}");assert!(!page["local"]["items"].as_array().unwrap().is_empty());
 let(_,family)=f.call("GET","/api/v1/library/tracks?source=local",None).await;assert_eq!(family["items"].as_array().unwrap().iter().find(|v|v["id"]=="local-shared").unwrap()["provider_data"]["catalog_display"]["play_count"],999);
}

#[tokio::test]
async fn external_offline_history_cannot_probe_denied_catalogue_metadata() {
    let f=Fixture::new().await; f.collection_tracks().await;
    let (user,token)=create(&f,"offline-visitor").await;
    let now=(chrono::Utc::now().timestamp()/86400*86400-43200)*1000;
    let day=chrono::DateTime::from_timestamp_millis(now).unwrap().format("%Y-%m-%d").to_string();
    let body=json!({"core_id":*f.s.core_id,"user_id":user,"client_id":uuid::Uuid::new_v4().to_string(),"sessions":[{
        "id":uuid::Uuid::new_v4().to_string(),"version":1,"track_id":"qq-only","title":"Own offline title","artist":"Own offline artist","duration":120.,"started_at":now-40000,"updated_at":now,"position":40.,"reason":"ended","days":{(day):{"seconds":40.,"last_at":now}}
    }]});
    let(status,value)=call(&f.app,"POST","/api/v1/stats/offline",Some(&token),Some(body)).await;
    assert_eq!(status,StatusCode::OK,"{value}"); assert_eq!(value["accepted"].as_array().unwrap().len(),1,"{value}");
    for path in ["/api/v1/stats?period_days=0","/api/v1/stats/history","/api/v1/library/tracks/qq-only/stats"] {
        let(status,value)=call(&f.app,"GET",path,Some(&token),None).await;
        assert_eq!(status,StatusCode::OK,"{path}: {value}");
        let serialized=value.to_string(); assert!(!serialized.contains("海岸"),"{path}: {value}"); assert!(!serialized.contains("回声"),"{path}: {value}");
        assert!(serialized.contains("Own offline") || path.ends_with("/stats"),"{path}: {value}");
    }
    let(_,family)=f.call("GET","/api/v1/stats?period_days=0",None).await;
    assert_eq!(family["summary"]["listened_seconds"],0.);
}

#[tokio::test]
async fn external_account_archive_upload_can_be_reauthorized_from_new_admin_session() {
    let f=Fixture::new().await;
    let(_,job)=f.call("POST","/api/v1/auth/account-imports",Some(json!({"current_password":"correct-test-password"}))).await;
    let id=job["id"].as_str().unwrap();
    let(_,login)=call(&f.app,"POST","/api/v1/auth/login",None,Some(json!({"username":"admin","password":"correct-test-password"}))).await;
    let token=login["token"].as_str().unwrap();
    let upload=||Request::builder().method("PUT").uri(format!("/api/v1/auth/account-imports/{id}/file")).header("Authorization",format!("Bearer {token}")).header("Content-Type","application/x-ndjson").body(Body::from(format!("{}\n",json!({"kind":"manifest","schema":1,"origin_core":"portable-core","origin_user":"portable-user","username":"Portable"})))).unwrap();
    assert_eq!(f.app.clone().oneshot(upload()).await.unwrap().status(),StatusCode::BAD_REQUEST);
    let path=format!("/api/v1/auth/account-imports/{id}/authorize-upload");
    assert_eq!(call(&f.app,"POST",&path,Some(token),Some(json!({"current_password":"incorrect-password"}))).await.0,StatusCode::BAD_REQUEST);
    let(status,value)=call(&f.app,"POST",&path,Some(token),Some(json!({"current_password":"correct-test-password"}))).await;
    assert_eq!(status,StatusCode::OK,"{value}");
    assert_eq!(f.app.clone().oneshot(upload()).await.unwrap().status(),StatusCode::OK);
    assert_eq!(call(&f.app,"POST",&path,Some(token),Some(json!({"current_password":"correct-test-password"}))).await.0,StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn external_playlist_exchange_filters_sources_and_hides_server_file_paths() {
    let f=Fixture::new().await; f.collection_tracks().await;
    let(_,token)=visitor_with_local(&f,"playlist-visitor").await;
    let(status,list)=call(&f.app,"POST","/api/v1/playlists/import",Some(&token),Some(json!({"name":"Own imported list","track_ids":["local-shared"]}))).await;
    assert_eq!(status,StatusCode::OK,"{list}");
    let path=format!("/api/v1/playlists/{}/export?format=json",list["id"].as_str().unwrap());
    let(status,value)=call(&f.app,"GET",&path,Some(&token),None).await;assert_eq!(status,StatusCode::OK,"{value}");
    assert_eq!(value["tracks"][0]["location"],"rmusic://track/local/local-shared"); assert!(!value.to_string().contains("catalog-fixtures"));
    assert_eq!(call(&f.app,"POST","/api/v1/playlists/import",Some(&token),Some(json!({"name":"Denied import","track_ids":["qq-only"]}))).await.0,StatusCode::NOT_FOUND);
    let(status,value)=call(&f.app,"POST","/api/v1/playlists/import/preview",Some(&token),Some(json!({"format":"json","content":json!({"format":"rmusic-playlist","version":1,"tracks":[{"id":"qq-only","source":"qqmusic","title":"","artist":""}]}).to_string()}))).await;
    assert_eq!(status,StatusCode::OK,"{value}");assert!(value["items"][0]["match"].is_null(),"{value}");
}

#[tokio::test]
async fn external_catalogue_links_use_only_authorized_indexed_metadata() {
    let f=Fixture::new().await;f.collection_tracks().await;
    let(_,token)=visitor_with_local(&f,"links-visitor").await;
    let body=|id|json!({"track_id":id,"artist":"Forged artist","album":"Forged album","source":"other","resolve_credits":true,"provider_data":{"artist_credits":[{"id":"forged","name":"forged"}]}});
    let(status,value)=call(&f.app,"POST","/api/v1/library/links",Some(&token),Some(body("local-shared"))).await;
    assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["album"]["title"],"夜航");assert!(!value.to_string().contains("Forged"));
    assert_eq!(call(&f.app,"POST","/api/v1/library/links",Some(&token),Some(body("qq-only"))).await.0,StatusCode::NOT_FOUND);
    assert_eq!(call(&f.app,"POST","/api/v1/library/links",Some(&token),Some(body("missing"))).await.0,StatusCode::NOT_FOUND);
    let(_,denied)=create(&f,"links-denied").await;
    assert_eq!(call(&f.app,"POST","/api/v1/library/links",Some(&denied),Some(body("local-shared"))).await.0,StatusCode::NOT_FOUND);
}
