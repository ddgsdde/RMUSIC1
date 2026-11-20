//! Device preferences are owned by this Mac. Only explicit offline edits are replayed to the LAN Core.
use anyhow::{ensure, Context, Result};
use rmusic_audio::output::{DeviceInfo, OutputConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
struct DevicePrefs {
    config: Value,
    #[serde(default)]
    pending: BTreeMap<String, Value>,
}

pub struct LocalOutputs {
    path: PathBuf,
    devices: BTreeMap<String, DevicePrefs>,
}
impl LocalOutputs {
    pub fn open(root: &Path) -> Result<Self> {
        let path = root.join("local-output-settings.json");
        let devices = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self { path, devices })
    }
    fn save(&self) -> Result<()> {
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(&self.devices)?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }
    pub fn effective(&self, uid: &str, core: &Value) -> Value {
        let mut config = core.clone();
        if let Some(saved) = self.devices.get(uid) {
            for (key, val) in &saved.pending {
                config[key] = val.clone();
            }
        }
        config
    }
    pub fn remember(&mut self, uid: &str, config: &Value) -> Result<()> {
        let saved = self.devices.entry(uid.to_owned()).or_default();
        if saved.config == *config && saved.pending.is_empty() {
            return Ok(());
        }
        saved.config = config.clone();
        saved.pending.clear();
        self.save()
    }
    pub fn seed(&mut self, uid: &str, config: &Value) -> Result<()> {
        if !self.devices.contains_key(uid)
            && config.get("enabled").and_then(Value::as_bool).is_some()
        {
            self.remember(uid, config)?;
        }
        Ok(())
    }
    pub fn snapshot(&self, host: &str) -> Result<Value> {
        ensure!(!host.is_empty() && host.len() <= 160, "缺少本机设备标识");
        let all = rmusic_audio::output::devices()?;
        let route = all
            .iter()
            .find(|d| d.default && d.id != rmusic_coreaudio::SYSTEM_DEFAULT_UID);
        let items = all
            .iter()
            .map(|dev| self.item(host, dev, route))
            .collect::<Vec<_>>();
        Ok(json!({"host_id":host,"items":items,"local_settings":true}))
    }
    fn item(&self, host: &str, dev: &DeviceInfo, route: Option<&DeviceInfo>) -> Value {
        let volume = super::outputs::volume_state(&dev.id);
        let system = dev.id == rmusic_coreaudio::SYSTEM_DEFAULT_UID;
        let mut config = serde_json::to_value(OutputConfig {
            device_id: Some(dev.id.clone()),
            ..Default::default()
        })
        .unwrap();
        config["enabled"] = json!(system);
        config["volume_control"] = json!(if volume["supported"] == true {
            "device"
        } else {
            "fixed"
        });
        if let Some(saved) = self.devices.get(&dev.id).filter(|p| p.config.is_object()) {
            config = saved.config.clone();
        }
        json!({"id":super::outputs::endpoint_id(host,&dev.id),"uid":dev.id,"name":dev.name,"enabled":config["enabled"],"available":true,"online":config["enabled"],"system_default":dev.default,"route_name":if system {route.map(|r|&r.name)}else{None},"channels":dev.channels,"mix_rate":dev.mix_rate,"capabilities":rmusic_coreaudio::capabilities(&dev.id).ok(),"hardware_volume_enabled":config["volume_control"]=="device","device_volume":volume,"config":config})
    }
    pub fn configure(&mut self, host: &str, body: &Value) -> Result<Value> {
        let inventory = self.snapshot(host)?;
        let item = inventory["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == body["id"])
            .context("请连接要配置的本机设备")?;
        let changes = body["changes"].as_object().context("缺少设备设置")?;
        let mut config = item["config"].clone();
        let mut pending = BTreeMap::new();
        for (key, value) in changes {
            match key.as_str() {
                "hardware_volume_enabled" => {}
                "enabled"
                | "exclusive"
                | "buffer_ms"
                | "max_sample_rate"
                | "max_bit_depth"
                | "channels"
                | "resync_delay_ms"
                | "volume_control"
                | "volume_max"
                | "conversion_location" => {
                    config[key] = value.clone();
                    pending.insert(key.clone(), value.clone());
                }
                _ => anyhow::bail!("不支持的设备设置"),
            }
        }
        let output: OutputConfig = serde_json::from_value(config.clone())?;
        output.limits.validate()?;
        ensure!(
            config["enabled"].is_boolean() && (20..=2000).contains(&output.buffer_ms),
            "启用状态或缓冲无效"
        );
        let saved = self
            .devices
            .entry(item["uid"].as_str().unwrap().to_owned())
            .or_default();
        saved.config = config;
        saved.pending.extend(pending);
        self.save()?;
        self.snapshot(host)
    }
}
