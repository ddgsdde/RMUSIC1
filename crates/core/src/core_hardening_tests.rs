//! Isolated API contracts; no production account, audio device or network provider.
use super::*;
#[tokio::test]
async fn music_identity_keeps_cover_recordings_separate_and_manual_assertions_are_reversible(){
 let f=Fixture::new().await;f.collection_tracks().await;
 let first=f.call("GET","/api/v1/library/tracks/local-shared/identity",None).await;assert_eq!(first.0,StatusCode::OK);assert_eq!(first.1["revision"],0);assert_eq!(first.1["automatic_merge"],false);
 let work=uuid::Uuid::new_v4().to_string();let assertion=json!({"kind":"work","namespace":"musicbrainz","external_id":work,"title":"同一作品","role":"work"});
 for id in ["local-shared","qq-live"]{let(status,value)=f.call("PUT",&format!("/api/v1/library/tracks/{id}/identity"),Some(json!({"expected_revision":0,"assertions":[assertion.clone()]}))).await;assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["revision"],1);}
 let graph=f.call("GET","/api/v1/library/tracks/local-shared/identity",None).await.1;let entity=graph["entities"].as_array().unwrap().iter().find(|e|e["kind"]=="work").unwrap()["id"].as_str().unwrap();
 let result=f.call("GET",&format!("/api/v1/music/entities/{entity}?limit=1"),None).await.1;assert_eq!(result["total"],2);assert_eq!(result["items"].as_array().unwrap().len(),1);
 assert_eq!(f.call("PUT","/api/v1/library/tracks/local-shared/identity",Some(json!({"expected_revision":0,"assertions":[]}))).await.0,StatusCode::CONFLICT);
 assert_eq!(f.call("PUT","/api/v1/library/tracks/local-shared/identity",Some(json!({"expected_revision":1,"assertions":[]}))).await.1["revision"],2);
 let records=f.s.db.run(|c|Ok((c.query_row("SELECT recording_id FROM tracks WHERE id='local-shared'",[],|r|r.get::<_,String>(0))?,c.query_row("SELECT recording_id FROM tracks WHERE id='qq-live'",[],|r|r.get::<_,String>(0))?,c.query_row("SELECT COUNT(*) FROM music_identity_audit",[],|r|r.get::<_,i64>(0))?))).await.unwrap();assert_ne!(records.0,records.1);assert_eq!(records.2,3);
 let read=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"identity-read","scopes":["read"]}))).await.1["token"].as_str().unwrap().to_owned();assert_eq!(call(&f.app,"PUT","/api/v1/library/tracks/local-shared/identity",Some(&read),Some(json!({"expected_revision":2,"assertions":[]}))).await.0,StatusCode::FORBIDDEN);
 assert_eq!(f.call("PUT","/api/v1/library/tracks/local-shared/identity",Some(json!({"expected_revision":2,"assertions":[{"kind":"work","namespace":"musicbrainz","external_id":"not-a-uuid","title":"x","role":"work"}]}))).await.0,StatusCode::BAD_REQUEST);
}
#[tokio::test]
async fn fingerprint_never_downloads_cloud_or_analyzes_from_read_permission(){
 let f=Fixture::new().await;f.collection_tracks().await;
 let value=f.call("POST","/api/v1/library/tracks/qq-only/fingerprint",None).await;assert_eq!(value.0,StatusCode::BAD_REQUEST);assert_eq!(f.call("GET","/api/v1/library/tracks/qq-only/fingerprint",None).await.1["available"],false);
 let jobs=f.s.db.run(|c|Ok(c.query_row("SELECT COUNT(*) FROM jobs WHERE kind IN ('cache_fill','download','fingerprint')",[],|r|r.get::<_,i64>(0))?)).await.unwrap();assert_eq!(jobs,0);
 assert_eq!(call(&f.app,"GET","/api/v1/library/tracks/qq-only/fingerprint",None,None).await.0,StatusCode::UNAUTHORIZED);
}
fn extension_manifest()->Value{json!({"id":"contract-example","name":"Example","version":"0.1.0","kind":"extension","extension_api":"1","api_version":"1","core_api":">=0.1.0,<0.2.0","entrypoint":"provider.py","configuration_v1":{"type":"object","additionalProperties":false,"properties":{"password":{"type":"string","maxLength":128,"secret":true},"label":{"type":"string","maxLength":64}},"required":["password"]},"extensions":[{"method":"summarize","kind":"statistics","scope":"read","effect":"none","context":"library_summary_v1","input":{"type":"object","properties":{},"additionalProperties":false},"output":{"type":"object","properties":{"tracks":{"type":"integer","minimum":0}},"required":["tracks"],"additionalProperties":false}},{"method":"propose","kind":"automation","scope":"control","effect":"proposal","input":{"type":"object","properties":{},"additionalProperties":false},"output":{"type":"boolean"}}]})}
#[tokio::test]
async fn extension_rpc_checks_schema_permission_disable_and_configuration_redaction(){
 let f=Fixture::new().await;let dir=f.s.config.plugin_dir.join("contract-example");std::fs::create_dir_all(&dir).unwrap();let manifest=extension_manifest();assert!(extensions::validate_manifest(&manifest).is_ok());std::fs::write(dir.join("manifest.json"),manifest.to_string()).unwrap();
 std::fs::write(dir.join("provider.py"),"import json,sys\nfor line in sys.stdin:\n r=json.loads(line);p=r['params'];print(json.dumps({'id':r['id'],'result':{'version':1,'output':{'tracks':p.get('context',{}).get('track_count',0)} if p['method']=='summarize' else False}}),flush=True)\n").unwrap();
 let mut obsolete=manifest.clone();obsolete["version"]=json!("0.0.1");
 assert_eq!(f.s.plugin.call_declared("contract-example",json!({}),obsolete).await.unwrap_err().1,"plugin_manifest_changed");
 assert_eq!(f.call("GET","/api/v1/plugins",None).await.1["items"][0]["capabilities"],json!([]));
 let uri="/api/v1/extensions/contract-example/invoke/summarize";
 assert_eq!(f.call("GET","/api/v1/extensions",None).await.1["items"].as_array().unwrap().len(),1);
 let(status,value)=f.call("POST",uri,Some(json!({}))).await;assert_eq!(status,StatusCode::OK,"{value}");assert_eq!(value["host_actions_applied"],false);assert_eq!(value["output"]["tracks"],0);
 assert_eq!(f.call("POST",uri,Some(json!({"extra":1}))).await.0,StatusCode::BAD_REQUEST);assert_eq!(f.call("POST","/api/v1/extensions/contract-example/invoke/missing",Some(json!({}))).await.0,StatusCode::BAD_REQUEST);
 let read=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"extension-read","scopes":["read"]}))).await.1["token"].as_str().unwrap().to_owned();assert_eq!(call(&f.app,"POST","/api/v1/extensions/contract-example/invoke/propose",Some(&read),Some(json!({}))).await.0,StatusCode::FORBIDDEN);
 let path="/api/v1/plugins/contract-example/configuration";assert_eq!(call(&f.app,"GET",path,Some(&read),None).await.0,StatusCode::FORBIDDEN);
 let saved=f.call("PUT",path,Some(json!({"expected_revision":"0","values":{"password":"synthetic-only-secret","label":"first"}}))).await;assert_eq!(saved.0,StatusCode::OK,"{}",saved.1);let rev=saved.1["revision"].clone();
 let readback=f.call("GET",path,None).await.1;assert_eq!(readback["values"]["password"]["configured"],true);assert!(!readback.to_string().contains("synthetic-only-secret"));
 assert_eq!(f.call("PUT",path,Some(json!({"expected_revision":"0","values":{"label":"lost-update"}}))).await.0,StatusCode::CONFLICT);
 assert_eq!(f.call("PUT",path,Some(json!({"expected_revision":rev,"values":{"label":"second"}}))).await.0,StatusCode::OK);
 let value:Value=serde_json::from_slice(&std::fs::read(f.s.config.data.join("providers/contract-example/configuration.json")).unwrap()).unwrap();assert_eq!(value["password"],"synthetic-only-secret");assert_eq!(value["label"],"second");
 assert_eq!(f.call("PATCH","/api/v1/plugins/contract-example",Some(json!({"enabled":false}))).await.0,StatusCode::OK);assert_eq!(f.call("POST",uri,Some(json!({}))).await.0,StatusCode::BAD_REQUEST);f.s.plugin.stop("contract-example").await;
}
