use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub duration: f64,
    pub track_number: u32,
    pub disc_number: u32,
    pub source: String,
    pub format: String,
    pub sample_rate: u32,
    pub bit_depth: u32,
    pub channels: u32,
    pub bitrate: u32,
    pub cover_url: Option<String>,
    pub recording_id: String,
    pub available: bool,
    pub created_at: i64,
    #[serde(default)]
    pub integrated_lufs: Option<f64>,
    #[serde(default)]
    pub provider_data: Value,
}
impl Track {
    pub fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let provider_data:Value=serde_json::from_str(&r.get::<_,String>("provider_data")?).unwrap_or(json!({}));
        let source: String = r.get("source")?;
        let integrated_lufs = provider_data["integrated_lufs"].as_f64().filter(|v|v.is_finite()).or_else(|| {
            if source == "qqmusic" && provider_data["source_audio"]["quality"] != "HIRES" {
                crate::loudness::qq_lufs(&provider_data["qq_replay_gain"])
            } else {
                None
            }
        });
        Ok(Self {
            id: r.get("id")?,
            title: r.get("title")?,
            artist: r.get("artist")?,
            album: r.get("album")?,
            album_artist: r.get("album_artist")?,
            duration: r.get("duration")?,
            track_number: r.get("track_number")?,
            disc_number: r.get("disc_number")?,
            source,
            format: r.get("format")?,
            sample_rate: r.get("sample_rate")?,
            bit_depth: r.get("bit_depth")?,
            channels: r.get("channels")?,
            bitrate: r.get("bitrate")?,
            cover_url: r.get("cover_url")?,
            recording_id: r.get("recording_id")?,
            available: r.get("available")?,
            created_at: r.get("created_at")?,
            integrated_lufs,
            provider_data,
        })
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Autoplay {
    pub enabled: bool,
    pub source: String,
    pub fallback_local: bool,
}
impl Default for Autoplay {
    fn default() -> Self {Self{enabled:true,source:"local".into(),fallback_local:true}}
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Transitions {
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
}
impl Default for Transitions {
    fn default()->Self {Self{fade_in_ms:0,fade_out_ms:0}}
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Zone {
    pub id: String,
    pub name: String,
    pub endpoint_id: String,
    pub status: String,
    pub volume: f64,
    pub muted: bool,
    pub repeat: String,
    pub shuffle: bool,
    pub position: f64,
    #[serde(default)]
    pub playback_id: Option<String>,
    pub track: Option<Track>,
    pub queue: Vec<Track>,
    #[serde(default)]
    pub queue_revision: u64,
    pub queue_index: usize,
    pub updated_at: i64,
    #[serde(default="default_dsp", deserialize_with="deserialize_dsp")]
    pub dsp: Value,
    #[serde(default)]
    pub autoplay: Autoplay,
    #[serde(default)]
    pub transitions: Transitions,
}
fn default_dsp()->Value {json!({"enabled":false,"preamp_db":0.,"balance":0.,"normalize":false,"target_lufs":-14.,"bands":[]})}
pub(crate) fn complete_dsp(mut value:Value)->Value {
    if let Some(config)=value.as_object_mut(){
        for (key,default) in default_dsp().as_object().unwrap(){config.entry(key.clone()).or_insert_with(||default.clone());}
        value
    }else{default_dsp()}
}
fn deserialize_dsp<'de,D:serde::Deserializer<'de>>(deserializer:D)->Result<Value,D::Error>{
    Value::deserialize(deserializer).map(complete_dsp)
}
impl Zone {
    pub fn new(id: String, name: String, endpoint_id: String) -> Self {
        Self {
            id,
            name,
            endpoint_id,
            status: "stopped".into(),
            volume: 1.,
            muted: false,
            repeat: "off".into(),
            shuffle: false,
            position: 0.,
            playback_id: None,
            track: None,
            queue: vec![],
            queue_revision: 0,
            queue_index: 0,
            updated_at: chrono::Utc::now().timestamp_millis(),
            dsp: default_dsp(),
            autoplay: Autoplay::default(),
            transitions: Transitions::default(),
        }
    }
    pub fn renew_playback(&mut self) {self.playback_id=Some(uuid::Uuid::new_v4().to_string());}
}
