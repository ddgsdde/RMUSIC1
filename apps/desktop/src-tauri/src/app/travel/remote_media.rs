//! Android system controls follow the selected Core output independently of the WebView.
use super::*;

#[derive(Default)]
pub(super) struct RemoteMedia {
    target: Option<(String,bool)>,
    card: Option<Value>,
    connected: bool,
    command: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for RemoteMedia {
    fn drop(&mut self) {
        if let Some(command) = self.command.take() { command.abort(); }
        if self.card.is_some() { let _ = android::remote_media(None); }
    }
}

impl RemoteMedia {
    pub(super) fn select(&mut self, zone: Option<String>, legacy_groups: bool) {
        let target = zone.map(|id| (id, legacy_groups));
        if self.target != target {
            if let Some(command) = self.command.take() { command.abort(); }
            self.target = target;
        }
    }
    pub(super) fn connected(&mut self, connected: bool) { self.connected = connected; }
    pub(super) fn selected(&self) -> bool { self.target.is_some() }

    pub(super) fn publish(&mut self, app: &tauri::AppHandle, profile: &str, zones: &[Value], endpoints: &[Value]) {
        let zone = self.target.as_ref().and_then(|(id,_)| zones.iter().find(|z| z["id"] == *id));
        let endpoint = zone.and_then(|z| endpoints.iter().find(|e| e["id"] == z["endpoint_id"]));
        let card = zone.zip(endpoint).and_then(|(z,e)| card(z,e,self.connected));
        if self.card == card { return; }
        if android::remote_media(card.as_ref()).is_ok() {
            self.card = card;
            if let Some(zone) = zone.filter(|_| self.card.is_some()) {
                android::update_artwork(app, profile, &zone["track"]);
            }
        }
    }

    pub(super) fn command(&mut self, app: &tauri::AppHandle, profile: &str, zone: String, media: String, action: String) {
        let Some((target, legacy_groups)) = &self.target else { return; };
        let Some(card) = self.card.as_ref() else { return; };
        if target != &zone || card["zone"] != zone || card["media_id"] != media || card["available"] != true
            || self.command.as_ref().is_some_and(|t| !t.is_finished())
            || !matches!(action.as_str(), "play" | "pause" | "next" | "previous") { return; }
        let app = app.clone();
        let profile = profile.to_owned();
        let card = card.clone();
        let legacy_groups = *legacy_groups;
        self.command = Some(tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(10), send(&app.state(), &profile, &zone, &action, &card, legacy_groups)).await;
            match result {
                Ok(Ok(Some(zone))) => app.state::<DesktopState>().travel.send(Message::MediaZone { profile, zone }),
                Ok(Ok(None)) => {},
                Ok(Err(error)) => { let _ = app.emit("native-cache-error", error); },
                Err(_) => { let _ = app.emit("native-cache-error", "远程设备操作超时"); },
            }
        }));
    }
}

fn card(zone: &Value, endpoint: &Value, connected: bool) -> Option<Value> {
    let track = &zone["track"];
    let id = track["id"].as_str()?;
    let status = zone["status"].as_str()?;
    if !matches!(status, "playing" | "paused") { return None; }
    let playback = &endpoint["playback"];
    let measured = endpoint["capabilities"]["http_pull"] == true && endpoint["capabilities"]["continuous_pcm"] == true;
    let waiting = status == "playing" && (zone["buffering"] == true || measured && !(playback["core_playback_id"] == zone["playback_id"]
        && playback["track_id"] == id && playback["status"] == "playing" && playback["frames"].as_u64().unwrap_or(0) > 0));
    let millis = |value: &Value| (value.as_f64().unwrap_or(0.) * 1000.) as u64;
    Some(json!({
        "zone": zone["id"], "media_id": format!("{id}:{}", zone["playback_id"].as_str().unwrap_or_default()),
        "track_id": id, "title": track["title"].as_str().unwrap_or("未知歌曲"),
        "artist": track["artist"].as_str().unwrap_or(""), "device": endpoint["name"].as_str().or(zone["name"].as_str()).unwrap_or("播放设备"),
        "duration_ms": millis(&track["duration"]), "position_ms": millis(&zone["position"]), "updated_at": zone["updated_at"],
        "paused": status == "paused", "buffering": waiting, "available": connected && endpoint["online"] != false,
        "endpoint": zone["endpoint_id"], "playback": zone["playback_id"], "queue_index": zone["queue_index"],
        "group": endpoint["kind"] == "group", "portable": endpoint["capabilities"]["portable_control_v1"] == true
    }))
}

async fn send(state: &DesktopState, profile: &str, zone: &str, action: &str, card: &Value, legacy_groups: bool) -> CommandResult<Option<Value>> {
    let request = |method: &str, path: String, body| core_api_request(state, profile.into(), method.into(), path, body, None);
    let direct = card["group"] == true || card["portable"] == true;
    if legacy_groups && !direct {
        // A running synchronized start owns transport for all of its outputs.
        let starts = request("GET", "api/v1/sync/starts?active=true&recoverable=true&limit=32".into(), None).await?;
        let member = |group: &&Value| group["members"].as_array().is_some_and(|rows| rows.iter().any(|m| m["zone_id"] == zone));
        let groups = starts["items"].as_array().map(Vec::as_slice).unwrap_or_default();
        if let Some(group) = groups.iter().filter(|g| g["phase"] != "interrupted").find(member).or_else(||groups.iter().find(member)) {
            if group["phase"] == "interrupted" { return Err("分组已中断，请在播放设备页面恢复".into()); }
            let id = group["id"].as_str().ok_or("分组信息已更新")?;
            let request_id = uuid::Uuid::new_v4();
            let body = match action {
                "play" | "pause" => json!({"request_id":request_id,"action":if action == "play" {"resume"} else {"pause"}}),
                _ => {
                    let index = group["playhead"]["queue_index"].as_u64().unwrap_or(0);
                    if card["queue_index"] != index { return Err("队列已更新，请重试".into()); }
                    let target = if action == "next" {index.checked_add(1)} else {index.checked_sub(1)};
                    let target = target.filter(|n| group["queue"]["length"].as_u64().is_some_and(|len| *n < len)).ok_or("已到分组队列边界")?;
                    json!({"request_id":request_id,"action":"jump","from_queue_index":index,"queue_index":target,"position":0})
                },
            };
            request("POST", format!("api/v1/sync/starts/{}/control", urlencoding(id)), Some(body)).await?;
            return Ok(None);
        }
    }
    let body = (!direct && matches!(action,"play"|"pause") && card["playback"].is_string()).then(||
        json!({"control":{"track_id":card["track_id"],"playback_id":card["playback"],"endpoint_id":card["endpoint"]}}));
    request("POST", format!("api/v1/zones/{}/{action}", urlencoding(zone)), body).await.map(Some)
}
