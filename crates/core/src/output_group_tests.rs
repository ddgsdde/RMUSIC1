use super::*;
async fn rooms(f:&Fixture){for(id,endpoint)in [("room-a","output-a"),("room-b","output-b"),("room-c","output-a")]{f.s.save_zone(models::Zone::new(id.into(),id.into(),endpoint.into())).await.unwrap();}}
fn body(name:&str,revision:u64)->Value{json!({"name":name,"zone_ids":["room-b","room-a"],"expected_revision":revision})}
#[tokio::test]
async fn output_groups_are_shared_persistent_and_never_start_playback(){
    let f=Fixture::new().await;rooms(&f).await;let path=format!("/api/v1/output-groups/{}",uuid::Uuid::new_v4());
    let(status,saved)=f.call("PUT",&path,Some(body(" 客厅组合 ",0))).await;assert_eq!(status,StatusCode::CREATED);assert_eq!(saved["name"],"客厅组合");assert_eq!(saved["zone_ids"],json!(["room-a","room-b"]));assert_eq!(saved["revision"],1);
    let credentials=json!({"username":"othergroupuser","password":"other-group-password-2026"});assert_eq!(f.call("POST","/api/v1/auth/users",Some(credentials.clone())).await.0,StatusCode::OK);
    let login=call(&f.app,"POST","/api/v1/auth/login",None,Some(credentials)).await.1;let token=login["token"].as_str().unwrap();assert_eq!(call(&f.app,"GET","/api/v1/output-groups",Some(token),None).await.1["items"],json!([saved]));
    let restored=AppState::new(f.s.db.clone(),f.s.config.clone(),String::new()).await.unwrap();let restored=app(restored);assert_eq!(call(&restored,"GET","/api/v1/output-groups",Some(token),None).await.1["items"][0]["revision"],1);
    assert_eq!(f.call("GET","/api/v1/sync/starts?active=true",None).await.1["total"],0);assert!(f.s.zones.read().await.values().all(|z|z.status=="stopped"&&z.playback_id.is_none()));
}
#[tokio::test]
async fn output_groups_retry_conflict_and_delete_cannot_resurrect_old_definitions(){
    let f=Fixture::new().await;rooms(&f).await;let path=format!("/api/v1/output-groups/{}",uuid::Uuid::new_v4());let first=f.call("PUT",&path,Some(body("全屋",0))).await.1;
    assert_eq!(f.call("PUT",&path,Some(body("全屋",0))).await.1,first);
    let(a,b)=tokio::join!(f.call("PUT",&path,Some(body("新版 A",1))),f.call("PUT",&path,Some(body("新版 B",1))));assert!([a.0,b.0].contains(&StatusCode::OK)&&[a.0,b.0].contains(&StatusCode::CONFLICT));let current=if a.0==StatusCode::OK{a.1}else{b.1};assert_eq!(current["revision"],2);
    assert_eq!(f.call("DELETE",&format!("{path}?expected_revision=1"),None).await.0,StatusCode::CONFLICT);
    for _ in 0..2{assert_eq!(f.call("DELETE",&format!("{path}?expected_revision=2"),None).await.0,StatusCode::NO_CONTENT);}
    assert_eq!(f.call("PUT",&path,Some(body("全屋",0))).await.0,StatusCode::CONFLICT);assert!(f.call("GET","/api/v1/output-groups",None).await.1["items"].as_array().unwrap().is_empty());
}
#[tokio::test]
async fn output_groups_validate_members_and_enforce_control_permissions(){
    let f=Fixture::new().await;rooms(&f).await;let path=format!("/api/v1/output-groups/{}",uuid::Uuid::new_v4());
    for v in [body(" ",0),json!({"name":"重复","zone_ids":["room-a","room-a"],"expected_revision":0}),json!({"name":"同端","zone_ids":["room-a","room-c"],"expected_revision":0}),json!({"name":"丢失","zone_ids":["room-a","missing"],"expected_revision":0})]{assert_eq!(f.call("PUT",&path,Some(v)).await.0,StatusCode::BAD_REQUEST);}
    let readonly=f.call("POST","/api/v1/auth/tokens",Some(json!({"name":"output-group read only","scopes":["read"]}))).await.1;let token=readonly["token"].as_str().unwrap();
    assert_eq!(call(&f.app,"GET","/api/v1/output-groups",Some(token),None).await.0,StatusCode::OK);assert_eq!(call(&f.app,"PUT",&path,Some(token),Some(body("拒绝",0))).await.0,StatusCode::FORBIDDEN);assert_eq!(call(&f.app,"DELETE",&format!("{path}?expected_revision=1"),Some(token),None).await.0,StatusCode::FORBIDDEN);
    assert!(f.call("GET","/api/v1/output-groups",None).await.1["items"].as_array().unwrap().is_empty());
}
