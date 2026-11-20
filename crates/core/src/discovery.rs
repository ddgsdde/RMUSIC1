use crate::{auth::Auth,error::ApiResult,state::AppState};
use axum::{extract::State,Json};
use serde_json::{json,Value};
use std::{net::SocketAddr,sync::Arc};

pub async fn list(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>> {
    a.require("read")?;
    let discovery=s.discovery.lock().unwrap();
    Ok(Json(json!({"available":discovery.is_some(),"items":discovery.as_ref().map(|d|d.snapshot()).unwrap_or_default()})))
}
pub async fn start(s:&AppState,addr:SocketAddr) {
    if std::env::var("RMUSIC_DISCOVERY").as_deref()==Ok("0"){return;}
    match rmusic_discovery::Discovery::new() {
        Ok(discovery)=>{
            let name=s.setting("name",json!("R-Music")).await;
            let result=if addr.ip().is_loopback(){Ok(String::new())}else{discovery.advertise("core",&s.core_id,name.as_str().unwrap_or("R-Music"),addr.port(),&s.config.base_path,
                std::env::var_os("RMUSIC_TLS_CERT").is_some(),None,&[])};
            if let Err(error)=result {tracing::warn!(%error,"core discovery announcement unavailable");}
            *s.discovery.lock().unwrap()=Some(Arc::new(discovery));
        },
        Err(error)=>tracing::warn!(%error,"LAN discovery unavailable"),
    }
}
