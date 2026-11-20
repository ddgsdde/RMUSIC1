//! Mac host inventory. Core owns enabled/config state; UID-derived endpoint IDs survive reorder/replug.
use crate::endpoint_worker::{core_request, ContextData, Worker};
use anyhow::{Context, Result};
use rmusic_audio::output::{DeviceInfo, DsdOutput, OutputConfig};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

pub struct Outputs {
    context: ContextData,
    api: crate::api::Api,
    inventory_loaded: bool,
    descriptors: BTreeMap<String, Value>,
    inventory: Value,
    workers: HashMap<String, Worker>,
    worker_routes: HashMap<String, String>,
    volume_defaults: crate::volume_defaults::VolumeDefaults,
    retired_reports: HashMap<String, Value>,
    suspended: bool,
    root: std::path::PathBuf,
    roaming: bool,
    last_profile: Option<rmusic_bridge::connection::ConnectionProfile>,
    network_interfaces:(Vec<String>,Vec<String>),
    route_check:Option<tokio::task::JoinHandle<Result<rmusic_bridge::connection::ConnectionProfile>>>,
    home_available:bool,
    last_stream_state:Option<Value>,
    stream: Option<(String,rmusic_bridge::client_stream::ClientStream)>,
}
pub fn endpoint_id(host: &str, uid: &str) -> String {
    format!(
        "{host}-{}",
        &format!("{:x}", Sha256::digest(uid.as_bytes()))[..16]
    )
}
impl Outputs {
    pub async fn new(context: ContextData, defaults_path: std::path::PathBuf, api: crate::api::Api, defer_refresh: bool) -> Result<Self> {
        let mut result = Self {
            inventory: json!({"host_id":context.host,"items":[]}),
            context, api, inventory_loaded: false,
            descriptors: BTreeMap::new(),
            workers: HashMap::new(),
            worker_routes: HashMap::new(),
            retired_reports: HashMap::new(),
            suspended: false,
            root:defaults_path.parent().context("缺少本机数据目录")?.to_owned(),
            roaming:false, stream:None,last_profile:None,home_available:false,network_interfaces:crate::network_route::interfaces(),route_check:None,last_stream_state:None,
            volume_defaults: crate::volume_defaults::VolumeDefaults::open(defaults_path)?,
        };
        if !defer_refresh { result.refresh().await?; }
        Ok(result)
    }
    async fn load_inventory(&mut self) -> Result<()> {
        if self.inventory_loaded { return Ok(()); }
        let existing = self.api.request(json!({"path":"/devices"})).await?;
        for device in existing["items"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(existing["removed"].as_array().into_iter().flatten())
            .filter(|v| v["id"] == self.context.host)
        {
            for output in device["outputs"].as_array().into_iter().flatten() {
                if let (Some(id), Some(uid)) =
                    (output["id"].as_str(), output["native_device_id"].as_str())
                {
                    if id == endpoint_id(&self.context.host, uid) {
                        self.descriptors.insert(id.into(), json!({"id":id,"name":output["name"],"kind":"coreaudio","native_device_id":uid,"configurable":true,"default":false,"initial_config":output["config"]}));
                    }
                }
            }
        }
        self.inventory_loaded = true;
        Ok(())
    }
    fn descriptor(&self, device: &DeviceInfo) -> Value {
        let mut config = OutputConfig {
            device_id: Some(device.id.clone()),
            ..OutputConfig::default()
        };
        config.limits.volume_control = if volume_state(&device.id)["supported"]==true {rmusic_audio::output::VolumeControl::Device}else{rmusic_audio::output::VolumeControl::Fixed};
        let capabilities=rmusic_coreaudio::capabilities(&device.id).ok().and_then(|v|serde_json::to_value(v).ok());
        json!({"device_volume":volume_state(&device.id),"capabilities":capabilities,"id":endpoint_id(&self.context.host,&device.id),"name":device.name,"kind":"coreaudio","native_device_id":device.id,"configurable":true,"default":false,"initial_config":config})
    }
    pub async fn refresh(&mut self) -> Result<Value> {
        let mut local=crate::local_outputs::LocalOutputs::open(&self.root)?;
        if !rmusic_bridge::connection::standalone_bridge() {
            let network_interfaces=crate::network_route::interfaces();
            if self.network_interfaces!=network_interfaces {
                self.network_interfaces=network_interfaces;
                self.api.network_changed();
            }
            let profile=if self.roaming {
                if self.route_check.as_ref().is_some_and(|task|task.is_finished()){
                    self.route_check.take().unwrap().await??
                }else{
                    if self.route_check.is_none(){
                        let api=self.api.clone();
                        self.route_check=Some(tokio::spawn(async move{Ok(api.selected_route().await?.0)}));
                    }
                    self.last_profile.clone().context("尚未选择 Core 路径")?
                }
            }else{self.api.selected_route().await?.0};
            let remote=crate::network_route::selected_is_remote(&profile.address).await;
            self.last_profile=Some(profile);
            // Finish the current song on the same HAL owner when a home route reappears.
            self.home_available=!remote && self.roaming;
            let keep_playing=self.home_available && self.stream.as_ref().is_some_and(|(_,s)|{
                let state=s.status.lock().unwrap(); !matches!(state["status"].as_str(),Some("ended"|"stopped"|"paused")) && (state["type"]!="error" || state["source_failed"]==true)
            });
            if remote || keep_playing {
                if !self.roaming {
                    self.close_workers().await;
                    // Carry the existing installation forward without publishing or enabling a LAN endpoint.
                    if tokio::time::timeout(std::time::Duration::from_secs(2),self.load_inventory()).await.is_ok_and(|r|r.is_ok()) {
                        for d in self.descriptors.values() {
                            if let Some(uid)=d["native_device_id"].as_str(){local.seed(uid,&d["initial_config"])?;}
                        }
                    }
                    self.inventory_loaded=false;
                    self.retired_reports.clear();
                }
                self.roaming=true;self.inventory=local.snapshot(&self.context.host)?;
                if let Some((id,_))=&self.stream {if !self.inventory["items"].as_array().unwrap().iter().any(|o|o["id"]==*id && o["enabled"]==true) { self.close_stream().await; }}
                return Ok(self.snapshot());
            }
            if self.roaming {self.last_stream_state=self.stream.as_ref().map(|(id,s)|json!({"output":id,"report":s.status.lock().unwrap().clone()}));self.close_stream().await;self.roaming=false;}
        }
        self.load_inventory().await?;
        let devices = rmusic_audio::output::devices()?;
        for device in &devices {
            self.descriptors.insert(
                endpoint_id(&self.context.host, &device.id),
                self.descriptor(device),
            );
        }
        anyhow::ensure!(
            self.descriptors.len() <= 64,
            "当前 Core 最多接受一台主机的 64 路输出，请先整理已断开的设备记录"
        );
        if self.descriptors.is_empty() {
            self.inventory = json!({"host_id":self.context.host,"items":[]});
            return Ok(self.snapshot());
        }
        let body = json!({"id":self.context.host,"name":self.context.name,"platform":"macos","outputs":self.descriptors.values().collect::<Vec<_>>()});
        let inventory = self.api.request(json!({"path":"/devices/register","method":"POST","json":body})).await?;
        if inventory["removed"] == true {
            self.close_workers().await;
            self.inventory = json!({"host_id":self.context.host,"removed":true,"items":[]});
            return Ok(self.snapshot());
        }
        let mut items = vec![];
        for output in inventory["outputs"].as_array().into_iter().flatten() {
            let mut output = output.clone();
            let id = output["id"]
                .as_str()
                .context("missing output ID")?
                .to_owned();
            let uid = output["native_device_id"]
                .as_str()
                .context("missing native output UID")?
                .to_owned();
            let policy_key = format!(
                "{}:{uid}",
                self.context
                    .profile
                    .core_id
                    .as_deref()
                    .context("missing Core identity")?
            );
            let hardware = devices.iter().find(|d| d.id == uid);
            let volume = if hardware.is_some() { volume_state(&uid) } else { json!({"supported":false}) };
            let default_key=format!("device-default-v2:{policy_key}");
            if hardware.is_some()&&!self.volume_defaults.contains(&default_key) {
                // Preserve explicit software/fixed choices. The former automatic fixed default
                // becomes device volume only after an actual writable hardware control is read.
                let explicit_software=self.volume_defaults.contains(&policy_key)&&output["config"]["volume_control"]=="software";
                let mode=if explicit_software {"software"}else if self.volume_defaults.fixed(&policy_key)||volume["supported"]!=true {"fixed"}else{"device"};
                let mut config=output["config"].clone();config["volume_control"]=json!(mode);config["volume_max"]=json!(1.);
                if config!=output["config"] {self.api.request(json!({"path":format!("/outputs/{id}/config"),"method":"PUT","json":config})).await?;}
                output["config"]=config;self.volume_defaults.mark(policy_key.clone())?;self.volume_defaults.mark(default_key)?;
            }
            if !rmusic_bridge::connection::standalone_bridge() && hardware.is_some() {
                let merged=local.effective(&uid,&output["config"]);
                if merged!=output["config"] {self.api.request(json!({"path":format!("/outputs/{id}/config"),"method":"PUT","json":merged})).await?;output["config"]=merged;}
                local.remember(&uid,&output["config"])?;
            }
            let enabled = output["config"]["enabled"] == true;
            let route = if uid==rmusic_coreaudio::SYSTEM_DEFAULT_UID {
                devices.iter().find(|d|d.default&&d.id!=rmusic_coreaudio::SYSTEM_DEFAULT_UID).map(|d|d.id.clone())
            } else { hardware.map(|d|d.id.clone()) };
            if uid==rmusic_coreaudio::SYSTEM_DEFAULT_UID && self.worker_routes.get(&id)!=route.as_ref() {
                if let Some(worker)=self.workers.remove(&id){
                    crate::transport_input::unobserve(&id);
                    let status=worker.status.clone();worker.close().await;
                    self.retired_reports.insert(id.clone(),status.lock().unwrap().clone());
                }
                self.worker_routes.remove(&id);
            }
            if enabled && hardware.is_some() && !self.suspended {
                if !self.workers.contains_key(&id) {
                    self.workers.insert(
                        id.clone(),
                        Worker::start(
                            self.context.clone(), self.api.routes(),
                            id.clone(),
                            uid.clone(),
                            output["name"].as_str().unwrap_or("Mac 输出").into(),
                        ),
                    );
                    crate::transport_input::observe(&id, &self.workers[&id].observation);
                    if let Some(route)=route {self.worker_routes.insert(id.clone(),route);}
                }
            } else if let Some(worker) = self.workers.remove(&id) {
                crate::transport_input::unobserve(&id);
                self.worker_routes.remove(&id);
                let status = worker.status.clone();
                worker.close().await;
                self.retired_reports
                    .insert(id.clone(), status.lock().unwrap().clone());
            }
            items.push(json!({"id":id,"uid":uid,"name":output["name"],"available":hardware.is_some(),"enabled":enabled,"online":output["online"]==true,
                "route_name":if uid==rmusic_coreaudio::SYSTEM_DEFAULT_UID {devices.iter().find(|d|d.default&&d.id!=rmusic_coreaudio::SYSTEM_DEFAULT_UID).map(|d|d.name.clone())}else{None},
                "mix_rate":hardware.map_or(0,|d|d.mix_rate),"channels":hardware.map_or(0,|d|d.channels),"system_default":hardware.is_some_and(|d|d.default),"config":output["config"],"capabilities":output["capabilities"],"device_volume":volume,"hardware_volume_enabled":output["config"]["volume_control"]=="device"}));
        }
        self.inventory = json!({"host_id":self.context.host,"items":items});
        Ok(self.snapshot())
    }
    pub fn snapshot(&self) -> Value {
        let mut value = self.inventory.clone();
        value["roaming"]=json!(self.roaming);value["home_available"]=json!(self.home_available);
        if let Some((id,stream))=&self.stream {value["stream"]=stream.status.lock().unwrap().clone();value["stream_output"]=json!(id);}
        if !self.roaming {value["departed_stream"]=json!(self.last_stream_state);}
        value["suspended"] = json!(self.suspended);
        for item in value["items"].as_array_mut().into_iter().flatten() {
            if let Some(worker) = item["id"].as_str().and_then(|id| self.workers.get(id)) {
                item["report"] = worker.status.lock().unwrap().clone();
                item["online"] =
                    json!(item["online"] == true && item["report"]["registered"] == true);
            } else if let Some(report) = item["id"]
                .as_str()
                .and_then(|id| self.retired_reports.get(id))
            {
                item["report"] = report.clone();
            }
            if self.suspended {
                item["online"] = json!(false);
                item["report"]["registered"] = json!(false);
                item["report"]["suspended"] = json!(true);
            }
        }
        value
    }
    pub fn observation(&self,body:&Value)->Value {
        if let Some((id,stream))=&self.stream {if body["endpoint_id"]==*id{return stream.observation.read(body["audio"]==true);}}
        body["endpoint_id"].as_str().and_then(|id|self.workers.get(id)).filter(|_|!self.suspended)
            .map(|worker|worker.observation.read(body["audio"]==true)).unwrap_or_else(||json!({"local":false}))
    }
    pub fn device_volume_state(&self, body:&Value)->Value {
        let id=body["id"].as_str().unwrap_or("");
        if self.suspended {return json!({"local":false});}
        self.inventory["items"].as_array().and_then(|items|items.iter().find(|o|o["id"]==id)).and_then(|item|item["uid"].as_str())
            .map(|uid|json!({"local":true,"state":volume_state(uid)})).unwrap_or_else(||json!({"local":false}))
    }
    /// Keep authentication, queue cursors and per-UID configuration, but join every HAL owner.
    /// Set the gate before any await: inventory polling must never reopen a sleeping output.
    pub async fn suspend(&mut self) -> Value {
        self.suspended = true;
        self.close_stream().await;
        let workers = std::mem::take(&mut self.workers);
        for id in workers.keys() { crate::transport_input::unobserve(id); }
        let reports = futures_util::future::join_all(workers.into_iter().map(|(id, worker)| async move {
            let status = worker.status.clone();
            worker.close().await;
            let report = status.lock().unwrap().clone();
            (id, report)
        })).await;
        self.retired_reports.extend(reports);
        self.snapshot()
    }
    pub async fn resume(&mut self) -> Result<Value> {
        // If Core is still unreachable, subsequent ordinary refreshes can reconnect without login.
        self.suspended = false;
        self.refresh().await
    }
    pub async fn configure(&mut self, body: Value) -> Result<Value> {
        let id = body["id"].as_str().context("missing output identity")?;
        if self.roaming {
            let mut local=crate::local_outputs::LocalOutputs::open(&self.root)?;
            self.inventory=local.configure(&self.context.host,&body)?;
            if body["changes"]["enabled"]==false && self.stream.as_ref().is_some_and(|(output,_)|output==id) {self.close_stream().await;}
            return Ok(self.snapshot());
        }
        let descriptor = self.descriptors.get(id).context("输出不属于本机清单")?;
        let current = self.inventory["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|o| o["id"] == id)
            .context("请刷新本机输出清单")?;
        let mut config = current["config"].clone();
        for (key, value) in body["changes"]
            .as_object()
            .context("missing output changes")?
        {
            anyhow::ensure!(
                [
                    "enabled",
                    "exclusive",
                    "buffer_ms",
                    "max_sample_rate",
                    "max_bit_depth",
                    "channels",
                    "resync_delay_ms",
                    "volume_control",
                    "volume_max",
                    "conversion_location",
                    "hardware_volume_enabled"
                ]
                .contains(&key.as_str()),
                "不支持的输出设置"
            );
            if key != "hardware_volume_enabled" {
                config[key] = value.clone();
            }
        }
        // Compatibility for the previous private client mode; new clients send device explicitly.
        if body["changes"]["hardware_volume_enabled"]==true&&config["volume_control"]=="fixed" {config["volume_control"]=json!("device");}
        let parsed: OutputConfig = serde_json::from_value(config.clone())?;
        parsed.limits.validate()?;
        anyhow::ensure!(
            parsed.device_id.as_deref() == descriptor["native_device_id"].as_str()
                && parsed.dsd_output == DsdOutput::Disabled,
            "不能替换输出设备或启用 DSD"
        );

        let profile = self.api.selected_route().await?.0;
        core_request(
            &profile,
            &self.context.token,
            &format!("api/v1/outputs/{id}/config"),
            "PUT",
            Some(&config),
        )
        .await?;

        if !rmusic_bridge::connection::standalone_bridge() {
            crate::local_outputs::LocalOutputs::open(&self.root)?.configure(&self.context.host,&body)?;
        }

        if let Some(enabled) = body["changes"]["hardware_volume_enabled"].as_bool() {
            let uid = descriptor["native_device_id"]
                .as_str()
                .context("missing device UID")?;
            let key = format!(
                "{}:{uid}",
                self.context
                    .profile
                    .core_id
                    .as_deref()
                    .context("missing Core identity")?
            );
            self.volume_defaults.set_fixed(key, !enabled)?;
        }
        self.refresh().await
    }
    pub fn set_device_mute(&mut self, body: Value) -> Result<Value> {
        let id = body["id"].as_str().context("缺少输出标识")?;
        let muted = body["muted"].as_bool().context("缺少静音状态")?;
        let item = self.inventory["items"].as_array_mut().into_iter().flatten()
            .find(|item| item["id"] == id).context("输出不属于本机")?;
        anyhow::ensure!(!self.suspended && item["enabled"] == true && item["available"] == true,
            "请先启用并连接此输出");
        let uid = item["uid"].as_str().context("缺少设备 UID")?;
        let state = volume_json(rmusic_coreaudio::set_device_mute(uid, muted)?);
        item["device_volume"] = state.clone();
        Ok(state)
    }
    pub async fn set_device_volume(&mut self, body: Value) -> Result<Value> {
        let id = body["id"].as_str().context("missing output identity")?;
        let level = body["value"]
            .as_f64()
            .filter(|v| v.is_finite() && (0. ..=1.).contains(v))
            .context("音量应为 0–1")?;
        let output = self.inventory["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|o| o["id"] == id)
            .context("输出不属于本机")?;
        anyhow::ensure!(
            output["enabled"] == true && output["available"] == true,
            "请先启用并连接此输出"
        );
        anyhow::ensure!(
            output["hardware_volume_enabled"] == true,
            "设备音量控制已关闭"
        );
        anyhow::ensure!(
            output["config"]["volume_control"] == "device",
            "当前启用了软件音量，请先切回设备音量"
        );
        if self.roaming {
            let uid=output["uid"].as_str().context("缺少输出标识")?;
            rmusic_coreaudio::set_device_volume(uid,level)?;
            self.inventory=crate::local_outputs::LocalOutputs::open(&self.root)?.snapshot(&self.context.host)?;
            return Ok(self.snapshot());
        }
        let actual=core_request(&self.api.selected_route().await?.0,&self.context.token,&format!("api/v1/outputs/{id}/volume"),"PUT",Some(&json!({"value":level}))).await?;
        for item in self.inventory["items"].as_array_mut().into_iter().flatten(){if item["id"]==id {item["device_volume"]=actual.clone();}}
        Ok(self.snapshot())
    }
    pub async fn restore(&mut self) -> Result<Value> {
        let profile = self.api.selected_route().await?.0;
        core_request(
            &profile,
            &self.context.token,
            &format!("api/v1/devices/{}/restore", self.context.host),
            "POST",
            Some(&json!({})),
        )
        .await?;
        self.refresh().await
    }
    pub fn clear_error(&self, id: Option<&str>) {
        for (key, worker) in &self.workers {
            if id.is_none_or(|id| id == key) {
                worker.clear_error();
            }
        }
    }
    async fn close_stream(&mut self){crate::transport_input::clear();if let Some((id,s))=self.stream.take(){crate::transport_input::unobserve(&id);s.close().await;}}
    pub async fn stream_control(&mut self, mut cmd:Value)->Result<Value>{
        anyhow::ensure!(self.roaming && !self.suspended,"当前未使用外出播放");
        let id=cmd["output"].as_str().context("请选择本机输出")?.to_owned();
        let kind=cmd["type"].as_str().unwrap_or("").to_owned();
        if kind=="stop" {self.close_stream().await;return Ok(Value::Null);}
        if kind=="play" {
            if cmd["recover"]==true {self.api.network_changed();}
            let item=self.inventory["items"].as_array().unwrap().iter().find(|o|o["id"]==id && o["enabled"]==true && o["available"]==true).context("本机输出未启用或已断开")?;
            let config:OutputConfig=serde_json::from_value(item["config"].clone())?;
            let dsp:rmusic_audio::DspConfig=serde_json::from_value(cmd.get("dsp").cloned().unwrap_or_else(||json!({})))?;
            cmd["dsp"]=serde_json::to_value(dsp.loudness_only())?;
            let profile=self.last_profile.clone().context("尚未选择 Core 路径")?;
            let track=cmd["track"]["id"].as_str().context("缺少曲目")?;
            let escaped:String=track.bytes().map(|c|if c.is_ascii_alphanumeric(){(c as char).to_string()}else{format!("%{c:02X}")}).collect();
            // The worker resolves this local selection asynchronously. No network request may hold a transport command.
            cmd["url"]=json!(format!("api/v1/media/{escaped}"));
            if !self.stream.as_ref().is_some_and(|(output,stream)|output==&id && stream.output==config) {
                self.close_stream().await;
                let media=crate::streaming::TravelSource{api:self.api.clone(),inner:self.context.media.clone(),limits:config.limits.clone()};
                let replies=self.context.replies.clone();
                let terminal=Box::new(move |report:&Value| {
                    let _=replies.try_send(json!({"id":"travel-terminal","result":report["core_playback_id"]}));
                });
                let stream=rmusic_bridge::client_stream::ClientStream::start(profile,self.context.token.clone(),id.clone(),config,std::sync::Arc::new(media),Some(terminal),self.context.store.owner(&self.context.profile.id)?.map(|owner|rmusic_bridge::client_stream::Journal::new(self.context.store.clone(),owner)))?;
                crate::transport_input::observe(&id, &stream.observation);
                self.stream=Some((id.clone(),stream));
            }
        }
        anyhow::ensure!(matches!(kind.as_str(),"play"|"seek"|"resume"|"pause"|"volume"|"preload"|"loudness"),"未知播放操作");
        let (_,stream)=self.stream.as_ref().filter(|(output,_)|output==&id).context("请先选择曲目")?;
        let instance=cmd["playback_id"].as_str().map(str::to_owned);
        stream.command(cmd).await?;
        if kind=="play" {crate::transport_input::publish(id,instance.context("缺少播放标识")?,stream.port());}
        Ok(Value::Null)
    }
    async fn close_workers(&mut self) {
        self.worker_routes.clear();
        for id in self.workers.keys() { crate::transport_input::unobserve(id); }
        let workers = std::mem::take(&mut self.workers);
        let closes = workers.into_values().map(|w| w.close());
        futures_util::future::join_all(closes).await;
    }
    pub async fn close(mut self) {
        if let Some(task)=self.route_check.take(){task.abort();let _=task.await;}
        self.close_stream().await;
        self.close_workers().await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_identity_is_uid_bound_not_enumeration_order() {
        assert_eq!(
            endpoint_id("mac-host", "uid-A"),
            endpoint_id("mac-host", "uid-A")
        );
        assert_ne!(
            endpoint_id("mac-host", "uid-A"),
            endpoint_id("mac-host", "uid-B")
        );
        assert_ne!(
            endpoint_id("mac-host-2", "uid-A"),
            endpoint_id("mac-host", "uid-A")
        );
    }
}

pub(crate) fn volume_state(uid: &str) -> Value {
    match rmusic_coreaudio::device_volume(uid) {
        Ok(state) => volume_json(state),
        Err(error) => json!({"supported":false,"error":error.to_string()}),
    }
}


pub(crate) fn volume_json(state: rmusic_coreaudio::DeviceVolume) -> Value {
    json!({"supported":state.supported != 0,"value":state.value,"muted":state.muted != 0,
        "mute_supported":state.mute_supported != 0})
}
