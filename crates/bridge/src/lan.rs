use anyhow::Result;
use axum::{routing::get,Json,Router};
use serde_json::json;
use std::sync::Arc;
pub struct Announcement { discovery:rmusic_discovery::Discovery,record:String,server:tokio::task::JoinHandle<()> }
impl Announcement {
    pub async fn start(id:&str,name:&str,core_id:Option<&str>)->Result<Self>{
        let listener=tokio::net::TcpListener::from_std(rmusic_discovery::dual_stack_listener(0)?)?;
        let port=listener.local_addr()?.port();let discovery=rmusic_discovery::Discovery::new()?;
        let info=Arc::new(json!({"id":id,"name":name,"kind":"bridge","core_id":core_id,"version":1}));
        let router=Router::new().route("/",get(move||{let info=info.clone();async move{Json((*info).clone())}}));
        let record=discovery.advertise("bridge",id,name,port,"",false,core_id,&[])?;
        let server=tokio::spawn(async move{let _=axum::serve(listener,router).await;});
        Ok(Self{discovery,record,server})
    }
}
impl Drop for Announcement {fn drop(&mut self){self.discovery.withdraw(&self.record);self.server.abort();}}
