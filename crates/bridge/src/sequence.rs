//! Shared core reservation/commit protocol for native bridges and clients.
use anyhow::{bail,Result};
use crate::connection::ConnectionProfile;
use serde::Deserialize;
use serde_json::{json,Value};
use std::time::Duration;
pub mod recovery;
mod media;
pub use media::core_rendered_metadata;

#[derive(Debug)]
pub struct UnavailableProgramme;

impl std::fmt::Display for UnavailableProgramme {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("当前音源无法提供此曲的完整音频，请检查平台登录、会员或地区授权")
    }
}
impl std::error::Error for UnavailableProgramme {}

pub async fn media_json(response: reqwest::Response) -> Result<Value> {
    if response.status() == reqwest::StatusCode::BAD_GATEWAY {
        let error = response.error_for_status_ref().unwrap_err();
        if response.json::<Value>().await.is_ok_and(|value| value["error"]["code"] == "permission_or_unavailable") {
            return Err(UnavailableProgramme.into());
        }
        return Err(error.into());
    }
    Ok(response.error_for_status()?.json().await?)
}

#[derive(Clone)]
pub struct Config {
    pub media: Option<std::sync::Arc<dyn crate::media_proxy::MediaResolver>>,
    pub profile: ConnectionProfile,
    token: String,
    pub endpoint: String,
    http: reqwest::Client,
}
#[derive(Clone, Deserialize)]
pub struct Item {
    pub playback_id: String,
    #[serde(default)]
    pub listen_session_id: Option<String>,
    pub track: Value,
    pub media_path: String,
}
#[derive(Clone, Deserialize)]
pub struct Initial {
    #[serde(default="legacy_timing")]
    pub measured_version:u32,
    #[serde(default)]
    pub group_start_id: Option<String>,
    pub plan_id: String,
    #[serde(default)]
    pub listen_session_id: Option<String>,
    pub items: Vec<Item>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub continuous_recovery: bool,
}
fn legacy_timing()->u32{1}
impl Initial {
    pub fn validate_timing(&self)->Result<()> {
        let invalid_ids=if self.group_start_id.is_some(){self.listen_session_id.is_some()||self.items.iter().any(|item|item.listen_session_id.is_some())}else{self.listen_session_id.is_none()||self.items.iter().any(|item|item.listen_session_id.is_none())};
        if ![1,2].contains(&self.measured_version)||(self.measured_version==2&&((self.group_start_id.is_some()&&self.continuous_recovery)||invalid_ids)){bail!("invalid prepared listening timing contract");}Ok(())
    }
}
pub enum Preparation {Unsupported,Stale(Value),Ready(Initial)}
impl Config {
    pub async fn visualization_requested(&self,zone:&str)->Result<bool> {
        let value:Value=self.http.get(self.profile.endpoint(&format!("api/v1/zones/{zone}/visualization/lease"))?).bearer_auth(&self.token).timeout(Duration::from_secs(2)).send().await?.error_for_status()?.json().await?;
        Ok(value["active"]==true)
    }
    pub async fn publish_visualization(&self,zone:&str,value:&Value)->Result<()> {self.post(zone,"visualization/report",value).await?.error_for_status()?;Ok(())}
    pub fn conversion_locations()->Value {if cfg!(any(windows,target_os="linux")){json!(["device","core"])}else{json!(["device"])}}
    pub async fn media_source(&self, track: &str, source: &str) -> Result<std::sync::Arc<crate::media_proxy::MediaProxy>> {
        if crate::media_proxy::MediaProxy::core_pcm(source).is_some() {
            return Ok(std::sync::Arc::new(crate::media_proxy::MediaProxy::start(&self.profile,source,None).await?));
        }
        if let Some(media) = &self.media { media.resolve(&self.profile, track, source).await }
        else { Ok(std::sync::Arc::new(crate::media_proxy::MediaProxy::start(&self.profile, source, None).await?)) }
    }

    pub async fn register_output(&self,name:&str,output:&rmusic_audio::output::OutputConfig)->Result<Option<rmusic_audio::output::OutputConfig>> {
        use sha2::{Digest,Sha256};let device=format!("bridge-{:x}",Sha256::digest(self.endpoint.as_bytes()));
        let response=self.http.post(self.profile.endpoint("api/v1/devices/register")?).bearer_auth(&self.token).timeout(Duration::from_secs(5)).json(&json!({"id":device,"name":name,"platform":std::env::consts::OS,"outputs":[{"id":self.endpoint,"name":output.device_id.as_deref().unwrap_or("系统默认输出"),"kind":output.backend(),"native_device_id":output.device_id,"initial_config":output,"default":true,"configurable":true,"capabilities":{"conversion_locations":Self::conversion_locations()}}]})).send().await?;
        if response.status()==reqwest::StatusCode::NOT_FOUND{return Ok(Some(output.clone()));}
        let inventory:Value=response.error_for_status()?.json().await?;
        if inventory["removed"]==true{return Ok(None);}
        let config=&inventory["outputs"][0]["config"];if config["enabled"]==false{return Ok(None);}
        let config:rmusic_audio::output::OutputConfig=serde_json::from_value(config.clone())?;config.limits.validate()?;Ok(Some(config.normalized()))
    }
    pub async fn recovery_unavailable(&self,owner:&crate::listens::Owner)->Result<Option<Value>> {
        let status=self.status().await?;
        if status["core_id"]!=owner.core_id {bail!("recovery core identity mismatch");}
        if status["capabilities"]["recovery_controls"]!=true{return Ok(None);}
        let zones:Value=self.http.get(self.profile.endpoint("api/v1/zones")?).bearer_auth(&self.token)
            .timeout(Duration::from_secs(5)).send().await?.error_for_status()?.json().await?;
        for zone in zones["items"].as_array().into_iter().flatten().filter(|zone|zone["endpoint_id"]==self.endpoint) {
            let id=zone["id"].as_str().ok_or_else(||anyhow::anyhow!("zone has no identity"))?;
            #[cfg(any(windows,target_os="linux"))] if tokio::task::spawn_blocking(rmusic_audio::output::power::held_stamp).await??.is_some(){return self.fresh_output_admission(id,None).await;}
            if !zone["playback_id"].is_null(){continue;}
            let response=self.post(id,"recover/unavailable",&json!({"endpoint_id":self.endpoint})).await?.error_for_status()?.json::<Value>().await?;
            if response["applied"]==true {return Ok(Some(response["zone"].clone()));}
        }
        Ok(None)
    }
    pub async fn disconnect_browser(&self)->Result<()> {
        let endpoint=url::form_urlencoded::byte_serialize(self.endpoint.as_bytes()).collect::<String>();
        self.http.post(self.profile.endpoint(&format!("api/v1/endpoints/browser/{endpoint}/disconnect"))?).bearer_auth(&self.token)
            .timeout(Duration::from_secs(1)).send().await?.error_for_status()?;Ok(())
    }
    pub async fn recovery_supported(&self,owner:&crate::listens::Owner)->Result<bool> {
        self.recovery_supported_timed(owner,1).await
    }
    pub async fn recovery_supported_timed(&self,owner:&crate::listens::Owner,version:u32)->Result<bool> {
        let status=self.status().await?;
        if status["core_id"]!=owner.core_id {bail!("recovery core identity mismatch");}
        Ok(status["capabilities"]["continuous_recovery"]==true&&status["capabilities"]["measured_listens"]==true&&match version {1=>true,2=>status["capabilities"]["measured_listens_recovery_v2"]==true&&status["capabilities"]["measured_listens_v2"]==true&&status["capabilities"]["measured_listens_online_v2"]==true&&status["capabilities"]["measured_listens_socket_v2"]==true,_=>false})
    }
    pub async fn media_ticket(&self,track:&str)->Result<String> {
        self.request_media_ticket(track,None).await
    }
    pub async fn playback_ticket(&self,track:&str,zone:&str)->Result<String> {
        self.request_media_ticket(track,crate::connection::standalone_bridge().then_some(zone)).await
    }
    async fn request_media_ticket(&self,track:&str,zone:Option<&str>)->Result<String> {
        let track=url::form_urlencoded::byte_serialize(track.as_bytes()).collect::<String>();
        let mut url=self.profile.endpoint(&format!("api/v1/library/tracks/{track}/ticket"))?;
        if let Some(zone)=zone { url.query_pairs_mut().append_pair("zone",zone); }
        let ticket=media_json(self.http.get(url).bearer_auth(&self.token)
            .timeout(Duration::from_secs(5)).send().await?).await?;
        Ok(ticket["api_path"].as_str().or_else(||ticket["url"].as_str()).ok_or_else(||anyhow::anyhow!("core returned no media ticket"))?.into())
    }
    pub(crate) async fn status(&self)->Result<Value> {
        Ok(self.http.get(self.profile.endpoint("api/v1/status")?).timeout(Duration::from_secs(5)).send().await?.error_for_status()?.json().await?)
    }
    pub async fn listener(&self)->Result<Option<crate::listens::Owner>> {
        let status=self.status().await?;
        let core=status["core_id"].as_str().ok_or_else(||anyhow::anyhow!("core returned no identity"))?;
        if self.profile.core_id.as_deref().is_some_and(|expected|expected!=core){bail!("listening journal core identity mismatch");}
        if status["capabilities"]["measured_listens"]!=true{return Ok(None);}
        let user:Value=self.http.get(self.profile.endpoint("api/v1/auth/me")?).bearer_auth(&self.token)
            .timeout(Duration::from_secs(5)).send().await?.error_for_status()?.json().await?;
        let user=user["id"].as_str().filter(|id|!id.is_empty()).ok_or_else(||anyhow::anyhow!("core returned no listener identity"))?;
        Ok(Some(crate::listens::Owner {core_id:core.into(),user_id:user.into()}))
    }
    pub(crate) async fn upload_listens(&self,owner:&crate::listens::Owner,client:&str,rows:Vec<crate::listens::Row>)->Result<crate::listens::SyncResult> {
        Ok(self.http.post(self.profile.endpoint("api/v1/stats/offline")?).bearer_auth(&self.token).timeout(Duration::from_secs(5))
            .json(&json!({"core_id":owner.core_id,"user_id":owner.user_id,"client_id":client,"sessions":rows.into_iter().map(|row|row.snapshot).collect::<Vec<_>>()}))
            .send().await?.error_for_status()?.json().await?)
    }
    pub(crate) async fn upload_group_final(&self,group:&str,body:&str)->Result<reqwest::Response> {
        let group=uuid::Uuid::parse_str(group)?;
        Ok(self.http.post(self.profile.endpoint(&format!("api/v1/sync/starts/{group}/final-receipts"))?).bearer_auth(&self.token).timeout(Duration::from_secs(5)).header(reqwest::header::CONTENT_TYPE,"application/json").body(body.to_owned()).send().await?)
    }
    pub fn new(profile: ConnectionProfile, token: String, endpoint: String) -> Result<Self> {
        let http = profile.http()?;
        Ok(Self {
            media: None,
            profile,
            token,
            endpoint,
            http,
        })
    }
    pub async fn load_impulses(&self,zone:&str,selection:&rmusic_audio::impulse::ConvolutionConfig)->Result<std::sync::Arc<rmusic_audio::impulse::ImpulseBank>> {
        crate::impulses::load_selected(&self.profile,&self.token,zone,&selection.response_ids).await
    }
    pub async fn post(&self, zone: &str, action: &str, body: &Value) -> Result<reqwest::Response> {
        let zone = url::form_urlencoded::byte_serialize(zone.as_bytes()).collect::<String>();
        Ok(self
            .http
            .post(
                self.profile
                    .endpoint(&format!("api/v1/zones/{zone}/{action}"))?,
            )
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(5))
            .json(body)
            .send()
            .await?)
    }
    pub async fn get_path(&self,path:&str)->Result<Value> {
        media_json(self.http.get(self.profile.endpoint(path)?).bearer_auth(&self.token).timeout(Duration::from_secs(3)).send().await?).await
    }
    #[cfg(target_os="android")]
    pub async fn post_path(&self,path:&str,body:&Value)->Result<reqwest::Response> {
        Ok(self.http.post(self.profile.endpoint(path)?).bearer_auth(&self.token).timeout(Duration::from_secs(5)).json(body).send().await?)
    }
    pub async fn zone(&self,zone:&str)->Result<Value> {
        let zone=url::form_urlencoded::byte_serialize(zone.as_bytes()).collect::<String>();
        Ok(self.http.get(self.profile.endpoint(&format!("api/v1/zones/{zone}"))?).bearer_auth(&self.token)
            .timeout(Duration::from_secs(5)).send().await?.error_for_status()?.json().await?)
    }
    pub async fn prepare(&self, zone: &str, track: &str, id: &str) -> Result<Option<Initial>> {
        self.prepare_timed(zone,track,id,1).await
    }
    /// Explicit opt-in; legacy playback keeps its existing durable recovery contract.
    pub async fn prepare_timed(&self,zone:&str,track:&str,id:&str,version:u32)->Result<Option<Initial>> {
        match self.negotiate_timed(zone,track,id,version).await? {
            Preparation::Ready(initial)=>Ok(Some(initial)),Preparation::Unsupported=>Ok(None),
            Preparation::Stale(_)=>bail!("播放准备请求已被 Core 的较新状态替换")
        }
    }
    pub async fn negotiate(&self, zone: &str, track: &str, id: &str) -> Result<Preparation> {
        self.negotiate_timed(zone,track,id,1).await
    }
    pub async fn negotiate_timed(&self,zone:&str,track:&str,id:&str,version:u32)->Result<Preparation> {
        self.negotiate_queue(zone,track,id,version,None).await
    }
    pub async fn refresh_queue_timed(&self,zone:&str,track:&str,id:&str,version:u32,plan:&str,committed:&[String])->Result<Preparation> {
        self.negotiate_queue(zone,track,id,version,Some((plan,committed))).await
    }
    async fn negotiate_queue(&self,zone:&str,track:&str,id:&str,version:u32,refresh:Option<(&str,&[String])>)->Result<Preparation> {
        if ![1,2].contains(&version){bail!("unsupported listening timing version");}
        let (group_v2,recovery_v2)=if version==2 {
            let status=self.status().await?;
            if self.profile.core_id.as_ref().is_some_and(|id|status["core_id"]!=*id)||status["capabilities"]["measured_listens_v2"]!=true||status["capabilities"]["measured_listens_online_v2"]!=true||status["capabilities"]["measured_listens_socket_v2"]!=true {bail!("core does not support online output timing v2");}
            (status["capabilities"]["group_output_v2"]==true,status["capabilities"]["measured_listens_recovery_v2"]==true&&status["capabilities"]["continuous_recovery"]==true)
        }else{(false,false)};
        let mut body=json!({"endpoint_id":self.endpoint,"track_id":track,"playback_id":id});if version==2{body["measured_version"]=json!(2);}
        if let Some((plan,committed))=refresh {body["plan_id"]=json!(plan);body["committed_playback_ids"]=json!(committed);}
        #[cfg(any(windows,target_os="linux"))] if let Some(zone)=self.fresh_output_admission(zone,Some(id)).await? {return Ok(Preparation::Stale(zone));}
        let response = self
            .post(
                zone,
                "prepare",
                &body,
            )
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            if version==2{bail!("output timing v2 preparation is unavailable; refusing legacy fallback");}
            return Ok(Preparation::Unsupported);
        }
        let response: Value = response.error_for_status()?.json().await?;
        if response["accepted"] != true {
            // Reassignment to another endpoint is a legitimate supersession. An unrelated
            // zone or a malformed rejection is a contract error, not silent cancellation.
            anyhow::ensure!(response["accepted"]==false && response["zone"]["id"]==zone,
                "Core 返回的播放准备状态无效，请重新连接");
            return Ok(Preparation::Stale(response["zone"].clone()));
        }
        let initial: Initial = serde_json::from_value(response)?;
        initial.validate_timing()?;
        if version==2&&initial.continuous_recovery&&!recovery_v2 {bail!("core does not support output timing v2 recovery");}
        if initial.measured_version==2&&initial.group_start_id.is_some()&&!group_v2{bail!("core does not support group output timing v2");}
        if initial.measured_version!=version||initial.items.len() > 8 || initial.plan_id.is_empty() {
            bail!("invalid continuous queue response");
        }
        Ok(Preparation::Ready(initial))
    }
    #[cfg(any(windows,target_os="linux"))]
    async fn fresh_output_admission(&self,zone:&str,playback:Option<&str>)->Result<Option<Value>> {
        use rmusic_audio::output::{power,DeviceUnavailable};
        let Some(stamp)=tokio::task::spawn_blocking(power::held_stamp).await?? else{return Ok(None)};
        let status=self.status().await?;
        if self.profile.core_id.as_ref().is_none_or(|id|status["core_id"]!=*id)||status["capabilities"]["system_suspend_fresh_start"]!=true {
            return Err(DeviceUnavailable("核心尚不支持无游标挂起起播，请升级匹配开发 Core；等待状态已保留".into()).into());
        }
        let response=self.request_json(zone,"recover/unavailable",&json!({"endpoint_id":self.endpoint,"system_suspend":stamp,"playback_id":playback})).await?;
        let current=&response["zone"];
        if current["id"]!=zone||current["endpoint_id"]!=self.endpoint {bail!("invalid fresh output zone identity");}
        if response["accepted"]!=true{return Ok(Some(current.clone()));}
        if response["system_suspend_ack"]!=json!(stamp)||!matches!(current["status"].as_str(),Some("playing"|"paused"))
            ||current["playback_id"].as_str().is_none_or(|id|uuid::Uuid::parse_str(id).is_err())
            ||playback.is_some_and(|id|current["playback_id"]!=id) {return Err(DeviceUnavailable("核心没有确认当前手动起播实例，输出等待状态已保留".into()).into());}
        tokio::task::spawn_blocking(move||power::acknowledge(&stamp)).await??;Ok(None)
    }
    /// A cursor belongs to this Core and login even when the authenticated entry point changes.
    pub fn same_listener(&self, profile:&ConnectionProfile, token:&str, endpoint:&str)->bool {
        (self.profile.core_public_key==profile.core_public_key && self.same_connection(profile,token,endpoint)) || (self.profile.core_id.is_some()
            && self.profile.core_public_key.is_some() && self.profile.core_id==profile.core_id
            && self.profile.core_public_key==profile.core_public_key && self.token==token
            && self.endpoint==endpoint && self.profile.id==profile.id)
    }
    pub fn same_connection(&self,profile:&ConnectionProfile,token:&str,endpoint:&str)->bool {
        self.token==token && self.endpoint==endpoint && self.profile.id==profile.id
            && self.profile.address==profile.address && self.profile.core_id==profile.core_id
            && self.profile.certificate_sha256==profile.certificate_sha256 && self.profile.skip_certificate_verification==profile.skip_certificate_verification
    }
    pub async fn commit(&self, zone: &str, body: &Value) -> Result<Value> {
        self.request_json(zone,"handoff",body).await
    }
    pub async fn request_json(&self, zone: &str, action:&str, body: &Value) -> Result<Value> {
        for attempt in 0..3 {
            let result = match self.post(zone, action, body).await {
                Ok(response) if response.status().is_client_error() => {
                    let error=response.error_for_status_ref().unwrap_err().without_url();
                    let body=response.json::<Value>().await.unwrap_or(Value::Null);
                    if action == "handoff" && body["error"]["code"] == "handoff_timing_conflict" {
                        return Err(HandoffTimingConflict.into());
                    }
                    let message=body["error"]["message"].as_str().unwrap_or("Core 拒绝了播放状态上报").to_owned();
                    return Err(anyhow::Error::new(error).context(message))
                }
                Ok(response) => match response.error_for_status() {
                    Ok(response) => response.json::<Value>().await.map_err(anyhow::Error::from),
                    Err(error) => Err(error.into()),
                },
                Err(error) => Err(error),
            };
            match result {
                Ok(response) => return Ok(response),
                Err(error) if attempt == 2 => return Err(error),
                Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
        unreachable!("three bounded acknowledgement attempts")
    }
}

#[cfg(test)]
#[path="sequence/timing_tests.rs"]
mod timing_tests;

#[cfg(test)]mod output_inventory_tests {
 use super::*;
 #[tokio::test]async fn inventory_advertises_the_conversion_path_before_accepting_settings(){
  use axum::{routing::post,Json,Router};
  let router=Router::new().route("/api/v1/devices/register",post(|Json(value):Json<Value>|async move{
   assert_eq!(value["outputs"][0]["capabilities"]["conversion_locations"],Config::conversion_locations());
   Json(json!({"outputs":[{"config":value["outputs"][0]["initial_config"]}]}))
  }));
  let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let server=tokio::spawn(async move{axum::serve(listener,router).await.unwrap();});
  let profile=ConnectionProfile{core_public_key:None,id:"qa".into(),name:"qa".into(),address:format!("http://{address}"),core_id:None,skip_certificate_verification: false, certificate_sha256: None,alternatives:vec![]};
  let config=Config::new(profile,"synthetic".into(),"qa".into()).unwrap();assert!(config.register_output("QA",&Default::default()).await.unwrap().is_some());server.abort();
 }
}

/// The queue identities matched, but Core could not admit the live frame report.
#[derive(Debug)]
pub(crate) struct HandoffTimingConflict;

impl std::fmt::Display for HandoffTimingConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("交接的实际进度超出核心计时范围")
    }
}
impl std::error::Error for HandoffTimingConflict {}
