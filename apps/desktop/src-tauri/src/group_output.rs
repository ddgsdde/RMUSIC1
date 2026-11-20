//! Independent native output connections; the primary output also arbitrates local IPC playback.
use anyhow::{bail,Context,Result};
use futures_util::future::BoxFuture;
use rmusic_audio::output::OutputConfig;
use rmusic_bridge::{connection::ConnectionProfile,native::{self,EmbeddedHost}};
use serde_json::{json,Value};
use std::{collections::HashMap,sync::{Arc,Mutex},time::Duration};
use tokio::sync::{watch,Notify};

pub trait LocalPlayer:Send+Sync {
    fn prepare<'a>(&'a self,command:&'a Value)->BoxFuture<'a,Result<()>>;
    fn stop(&self)->BoxFuture<'_,Result<()>> {Box::pin(async{Ok(())})}
}
#[derive(Clone)]
pub struct Registration {
    pub id:String,pub profile:ConnectionProfile,pub token:String,pub endpoint:String,pub name:String,pub output:OutputConfig,
    pub listens:Option<Arc<rmusic_bridge::listens::Store>>,
    pub primary:bool,
}
struct Session {
    config:Registration,local:Arc<dyn LocalPlayer>,view:Mutex<Value>,endpoint:Mutex<Value>,
    stop:watch::Sender<bool>,ready:Notify,task:Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl Session {
    fn alive(&self)->bool {self.task.lock().is_ok_and(|task|task.as_ref().is_some_and(|task|!task.is_finished()))}
    fn matches(&self,next:&Registration)->bool {
        self.config.id==next.id&&self.config.profile.id==next.profile.id&&self.config.profile.address==next.profile.address
        &&self.config.profile.core_id==next.profile.core_id&&self.config.profile.certificate_sha256==next.profile.certificate_sha256&&self.config.profile.skip_certificate_verification==next.profile.skip_certificate_verification
        &&self.config.token==next.token&&self.config.endpoint==next.endpoint
        &&match (&self.config.listens,&next.listens){(Some(a),Some(b))=>Arc::ptr_eq(a,b),(None,None)=>true,_=>false}
        &&self.config.primary==next.primary&&self.config.output==next.output
    }
    async fn cancel_plan(&self) {
        let plan=self.view.lock().ok().and_then(|view|view["group_id"].as_str().map(String::from));
        if let Some(plan)=plan.filter(|id|uuid::Uuid::parse_str(id).is_ok()) {
            if let (Ok(http),Ok(url))=(self.config.profile.http(),self.config.profile.endpoint(&format!("api/v1/sync/starts/{plan}/cancel"))) {
                let _=http.post(url).bearer_auth(&self.config.token).timeout(Duration::from_secs(5)).send().await;
            }
        }
    }
    async fn close(&self) {
        self.cancel_plan().await;
        self.stop.send_replace(true);
        let task=self.task.lock().ok().and_then(|mut task|task.take());
        if let Some(task)=task {let _=task.await;}
    }
}
impl EmbeddedHost for Session {
    fn handles_ordinary(&self)->bool {!self.config.primary}
    fn removed(&self)->BoxFuture<'_,Result<()>> {self.local.stop()}
    fn prepare<'a>(&'a self,command:&'a Value)->BoxFuture<'a,Result<()>> {Box::pin(async move {
        let previous={let mut view=self.view.lock().map_err(|_|anyhow::anyhow!("group state lock unavailable"))?;let old=view.clone();
            *view=json!({"group_managed":true,"core_managed":true,"profile_id":self.config.profile.id,"core_id":self.config.profile.core_id,"endpoint_id":self.config.endpoint,"group_id":command["start"]["plan_id"],"zone_id":command["zone_id"],"track_id":command["track"]["id"],"core_playback_id":command["playback_id"],"position":command["position"],"status":"preparing","paused":false});old};
        if let Err(error)=self.local.prepare(command).await {if let Ok(mut view)=self.view.lock(){*view=previous;}return Err(error);}
        Ok(())
    })}
    fn report(&self,report:&Value) {
        if report["type"]=="endpoint" {if let Ok(mut endpoint)=self.endpoint.lock(){*endpoint=report["endpoint"].clone();}return;}
        if report["type"]=="registered" {if let Ok(mut endpoint)=self.endpoint.lock(){endpoint["registered"]=json!(true);}self.ready.notify_one();return;}
        let Ok(mut view)=self.view.lock() else{return;};
        if view["group_managed"]!=true{return;}
        if report.get("start").is_some()&&report["start"]["plan_id"]!=view["group_id"]{return;}
        if let Some(id)=report["core_playback_id"].as_str(){if report["type"]=="start_state"&&view["core_playback_id"]!=id{return;}}
        for key in ["position","track_id","core_playback_id","error","device_clock","frames","sample_rate"] {if let Some(value)=report.get(key){view[key]=value.clone();}}
        if report["type"]=="start_state" {
            view["start_phase"]=report["phase"].clone();view["frames_played"]=report["programme_frames_played"].clone();
            view["status"]=json!(match report["phase"].as_str(){Some("released")=>"playing",Some("cancelled"|"expired")=>"stopped",_=>"preparing"});
        }else if report["type"]=="state" {view["status"]=report["status"].clone();}
        else if matches!(report["type"].as_str(),Some("error"|"start_error")){view["status"]=json!("error");}
    }
    fn released(&self) {if let Ok(mut view)=self.view.lock(){*view=json!({"group_managed":false});}}
}

#[derive(Default)]
pub struct Manager {gate:tokio::sync::Mutex<()>,current:Mutex<HashMap<String,Arc<Session>>>}
impl Manager {
    pub async fn cancel_playback(&self)->Result<()> {
        let session=self.current.lock().ok().and_then(|current|current.values().find(|s|s.config.primary).cloned());
        if let Some(session)=session.filter(|s|s.view.lock().is_ok_and(|v|v["group_managed"]==true)) {
            // A group must join its worker before IPC can claim this physical output.
            // Ordinary playback never tears down the calibrated control connection.
            self.disable(&session.config.endpoint).await;
            self.register(session.config.clone(),session.local.clone()).await?;
        }
        Ok(())
    }
    pub fn owned(&self)->bool {self.status().is_some()}
    pub fn status(&self)->Option<Value> {self.current.lock().ok().and_then(|current|current.values().find(|s|s.config.primary).and_then(|session|session.view.lock().ok().filter(|view|view["group_managed"]==true).map(|view|view.clone())))}
    pub fn endpoint(&self,profile:&str,endpoint:&str)->Option<Value> {
        self.current.lock().ok().and_then(|current|current.get(endpoint).filter(|session|session.config.profile.id==profile&&session.alive()).and_then(|session|session.endpoint.lock().ok().filter(|value|value["registered"]==true).map(|value|value.clone())))
    }
    pub async fn register(&self,config:Registration,local:Arc<dyn LocalPlayer>)->Result<Value> {
        if uuid::Uuid::parse_str(&config.id).is_err()||!config.endpoint.starts_with("browser-")||config.endpoint.len()>100||!config.endpoint.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'-'){bail!("无效的本机分组注册标识");}
        let _gate=self.gate.lock().await;
        let old=self.current.lock().map_err(|_|anyhow::anyhow!("endpoint lock unavailable"))?.get(&config.endpoint).cloned();
        if let Some(old)=&old {if old.matches(&config)&&old.alive(){return self.endpoint(&config.profile.id,&config.endpoint).context("本机端点仍在连接");}}
        if let Some(old)=old {old.close().await;}
        self.current.lock().map_err(|_|anyhow::anyhow!("endpoint lock unavailable"))?.remove(&config.endpoint);
        // Preserve automatic local-zone creation/migration, then replace the browser declaration
        // with the real authenticated native socket before returning to the renderer.
        config.profile.http()?.post(config.profile.endpoint("api/v1/endpoints/browser")?).bearer_auth(&config.token)
            .timeout(Duration::from_secs(5)).json(&json!({"id":config.endpoint,"name":config.name,"playback_instance":true,"continuous_pcm":true,"continuous_recovery":true,"recovery_controls":true})).send().await?.error_for_status()?;
        let (stop,mut closed)=watch::channel(false);
        let session=Arc::new(Session {config,local,view:Mutex::new(json!({"group_managed":false})),endpoint:Mutex::new(Value::Null),stop,ready:Notify::new(),task:Mutex::new(None)});
        self.current.lock().map_err(|_|anyhow::anyhow!("endpoint lock unavailable"))?.insert(session.config.endpoint.clone(),session.clone());
        let worker=session.clone();let task=tokio::spawn(async move {
            let config=&worker.config;
            let mut failures=0u32;
            loop {
                if *closed.borrow(){break;}
                let began=std::time::Instant::now();
                let result=native::embedded_session(&config.profile,&config.token,&config.endpoint,&config.name,config.output.clone(),config.listens.clone(),async {if !*closed.borrow(){let _=closed.changed().await;}},worker.as_ref()).await;
                if *closed.borrow(){break;}
                let permanent=result.as_ref().err().and_then(|error|error.downcast_ref::<reqwest::Error>()).and_then(|error|error.status()).is_some_and(|code|matches!(code.as_u16(),401|403));
                if let Ok(mut endpoint)=worker.endpoint.lock(){endpoint["registered"]=json!(false);endpoint["online"]=json!(false);endpoint["error"]=json!(result.err().map(|e|e.to_string()).unwrap_or_else(||"本机输出连接已关闭".into()));}
                worker.ready.notify_one();
                if permanent{break;}
                failures=if began.elapsed()>=Duration::from_secs(30){0}else{failures.saturating_add(1)};
                let delay=Duration::from_millis((250u64<<failures.min(4)).min(5000));
                tokio::select!{_=tokio::time::sleep(delay)=>{},_=closed.changed()=>{}}
            }
            worker.ready.notify_one();
        });
        *session.task.lock().map_err(|_|anyhow::anyhow!("endpoint task lock unavailable"))?=Some(task);
        tokio::time::timeout(Duration::from_secs(8),session.ready.notified()).await.context("本机端点连接超时")?;
        let endpoint=session.endpoint.lock().map_err(|_|anyhow::anyhow!("endpoint state lock unavailable"))?.clone();
        if endpoint["registered"]!=true{bail!("本机端点未完成注册：{}",endpoint["error"].as_str().unwrap_or("连接已关闭"));}Ok(endpoint)
    }
    pub async fn stop(&self,profile:Option<&str>,registration:Option<&str>) {
        let _gate=self.gate.lock().await;
        let old:Vec<_>=self.current.lock().map(|current|current.values().filter(|session|profile.is_none_or(|id|session.config.profile.id==id)&&registration.is_none_or(|id|session.config.id==id)).cloned().collect()).unwrap_or_default();
        for old in old {old.close().await;if let Ok(mut current)=self.current.lock(){current.remove(&old.config.endpoint);}}
    }
    pub async fn disable(&self,endpoint:&str) {
        let _gate=self.gate.lock().await;
        let old=self.current.lock().ok().and_then(|current|current.get(endpoint).cloned());
        if let Some(old)=old {old.close().await;if let Ok(mut current)=self.current.lock(){current.remove(endpoint);}}
    }
}
