use super::*;
use axum::{Router,routing::{get,post},Json,http::StatusCode};
use std::sync::{Arc,Mutex};
#[tokio::test]
async fn native_output_v2_negotiation_is_explicit_capability_bound_and_never_downgrades() {
    let status=json!({"core_id":"core-a","capabilities":{"measured_listens_v2":true,"measured_listens_online_v2":true,"measured_listens_socket_v2":true}});
    let legacy=json!({"accepted":true,"plan_id":"plan","items":[],"continuous_recovery":true});
    let current=json!({"accepted":true,"plan_id":"plan","items":[],"listen_session_id":"listen","measured_version":2});
    let mut no_online=status.clone();no_online["capabilities"]["measured_listens_online_v2"]=json!(false);
    let mut no_socket=status.clone();no_socket["capabilities"]["measured_listens_socket_v2"]=json!(false);
    let mut foreign=status.clone();foreign["core_id"]=json!("core-b");
    let mut group_status=status.clone();group_status["capabilities"]["group_output_v2"]=json!(true);
    let group_plan=json!({"accepted":true,"plan_id":"plan","group_start_id":uuid::Uuid::new_v4().to_string(),"items":[],"measured_version":2});
    let mut grouped=current.clone();grouped["group_start_id"]=json!("group");
    let mut recovery=current.clone();recovery["continuous_recovery"]=json!(true);
    let mut recovery_status=status.clone();recovery_status["capabilities"]["continuous_recovery"]=json!(true);recovery_status["capabilities"]["measured_listens_recovery_v2"]=json!(true);
    for (version,status,response,accepted,requests) in [(1,status.clone(),legacy.clone(),true,1),(2,status.clone(),current.clone(),true,1),(2,no_online,current.clone(),false,0),(2,no_socket,current.clone(),false,0),(2,foreign,current.clone(),false,0),(2,status.clone(),legacy,false,1),(2,status.clone(),json!({"missing":true}),false,1),(2,status.clone(),group_plan.clone(),false,1),(2,group_status,group_plan,true,1),(2,status.clone(),grouped,false,1),(2,status.clone(),recovery.clone(),false,1),(2,recovery_status,recovery,true,1),(1,status.clone(),current,false,1)] {
        let bodies=Arc::new(Mutex::new(Vec::<Value>::new()));let captured=bodies.clone();
        let app=Router::new().route("/prefix/api/v1/status",get(move||{let status=status.clone();async move{Json(status)}})).route("/prefix/api/v1/zones/z/prepare",post(move|Json(body):Json<Value>|{captured.lock().unwrap().push(body);let response=response.clone();async move{(if response["missing"]==true{StatusCode::NOT_FOUND}else{StatusCode::OK},Json(response))}}));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let server=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
        let config=Config::new(ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}/prefix"),core_id:Some("core-a".into()),skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]},"isolated-token".into(),"endpoint".into()).unwrap();
        let result=if version==1{config.prepare("z","track","playback").await}else{config.prepare_timed("z","track","playback",version).await};
        assert_eq!(result.is_ok(),accepted);if accepted{assert_eq!(result.unwrap().unwrap().measured_version,version);}
        let bodies=bodies.lock().unwrap();assert_eq!(bodies.len(),requests);for body in bodies.iter(){assert_eq!(body["endpoint_id"],"endpoint");if version==1{assert!(body.get("measured_version").is_none());}else{assert_eq!(body["measured_version"],2);}}server.abort();
    }
}
