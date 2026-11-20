use super::*;
use axum::extract::{Path, State};
use std::time::{Duration, Instant};

pub(super) async fn job(s: &AppState, id: &str) -> ApiResult<Option<Value>> {
    let id = id.to_owned();
    Ok(s.db
        .read(move |c| {
            let v = c
                .query_row("SELECT result FROM sync_starts WHERE id=?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?;
            v.map(|v| serde_json::from_str::<Value>(&v))
                .transpose()
                .map_err(Into::into)
        })
        .await?)
}
fn active(v: &Value) -> bool {
    matches!(
        v["phase"].as_str(),
        Some("preparing" | "committed" | "playing" | "pausing" | "paused" | "seeking")
    )
}
async fn state(s: &AppState, g: &Group) -> ApiResult<Zone> {
    s.zones
        .read()
        .await
        .get(&g.zone_id)
        .cloned()
        .ok_or_else(ApiError::missing)
}
pub(super) async fn stop_member(s: &AppState, a: &Auth, id: &str) -> ApiResult<()> {
    Box::pin(crate::zones::command_internal(
        State(s.clone()),
        a.clone(),
        Path((id.to_owned(), "stop".into())),
        None,
        true,
    ))
    .await?;
    Ok(())
}
pub(super) async fn stop(s: &AppState, a: &Auth, g: &mut Group) -> ApiResult<()> {
    if let Some(id) = g.job_id.clone() {
        if let Some(v) = job(s, &id).await? {
            if active(&v) {
                let _ = crate::sync_start::cancel(State(s.clone()), a.clone(), Path(id.clone()))
                    .await?;
            }
            let until = Instant::now() + Duration::from_secs(7);
            while job(s, &id).await?.is_some_and(|v| v["settled"] != true) {
                if Instant::now() >= until {
                    return Err(ApiError(
                        StatusCode::CONFLICT,
                        "group_draining".into(),
                        "同步输出仍在收尾，请稍后重试。原队列已经保留。".into(),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        }
    }
    g.job_id = None;
    g.error = None;
    g.phase = "ready".into();
    save(s, g).await?;
    Ok(())
}
pub(super) async fn start(s: &AppState, a: &Auth, g: &mut Group, z: &Zone) -> ApiResult<()> {
    stop(s, a, g).await?;
    let Some(track) = z.track.as_ref() else {
        return Err(ApiError::bad("请先选择一首歌曲。"));
    };
    for id in &g.zone_ids {
        stop_member(s, a, id).await?;
        let mut member = s
            .zones
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(ApiError::missing)?;
        member.volume = z.volume;
        member.muted = z.muted;
        member.dsp = z.dsp.clone();
        member.transitions = z.transitions.clone();
        s.save_zone(member).await?;
    }
    let index = z.queue_index.min(z.queue.len().saturating_sub(1));
    let mut queue = z
        .queue
        .iter()
        .skip(index)
        .take(if z.repeat == "one" { 1 } else { 4 })
        .map(|t| t.id.clone())
        .collect::<Vec<_>>();
    if queue.first() != Some(&track.id) {
        queue.insert(0, track.id.clone());
    }
    let endpoints = s.endpoints.read().await;
    let zones = s.zones.read().await;
    let adaptive = g.zone_ids.iter().all(|id| {
        zones
            .get(id)
            .and_then(|z| endpoints.get(&z.endpoint_id))
            .is_some_and(|e| {
                [
                    "pcm_rate_adjustment_v1",
                    "programme_clock_v2",
                    "group_rate_lease_v1",
                ]
                .iter()
                .all(|k| e["capabilities"][*k] == true)
            })
    });
    drop(zones);
    drop(endpoints);
    let request_id = uuid::Uuid::new_v4().to_string();
    g.job_id = Some(request_id.clone());
    g.job_offset = index;
    g.controller = a.session_hash.clone();
    g.error = None;
    save(s, g).await?;
    s.save_zone(z.clone()).await?;
    let request=serde_json::from_value(json!({"request_id":request_id,"zone_ids":g.zone_ids,"track_id":track.id,"position":z.position,"queue":queue,"measured_version":2,"pcm_rate_adjustment":adaptive,"automatic_rate_adjustment":adaptive})).map_err(anyhow::Error::from)?;
    let result =
        crate::sync_start::start_for_device_group(s.clone(), a.clone(), request, g.id.clone())
            .await;
    if let Err(error) = result {
        g.error = Some(error.2.clone());
        g.job_id = None;
        save(s, g).await?;
        let mut paused = z.clone();
        paused.status = "paused".into();
        paused.playback_id = None;
        s.save_zone(paused).await?;
        publish_endpoint(s, g).await?;
        return Err(error);
    }
    if let Some(latest) = load(s, &g.id).await? {
        *g = latest;
    }
    publish_endpoint(s, g).await?;
    Ok(())
}
async fn control(s: &AppState, a: &Auth, g: &Group, fields: Value) -> ApiResult<()> {
    let Some(id) = g.job_id.as_ref() else {
        return Ok(());
    };
    let mut fields = fields;
    fields["request_id"] = json!(uuid::Uuid::new_v4().to_string());
    let body = serde_json::from_value(fields).map_err(anyhow::Error::from)?;
    let _ = crate::sync_start::control::command(
        State(s.clone()),
        a.clone(),
        Path(id.clone()),
        Json(body),
    )
    .await?;
    Ok(())
}
async fn apply(s: &AppState, a: &Auth, g: &mut Group, z: Zone, restart: bool) -> ApiResult<Zone> {
    if restart {
        stop(s, a, g).await?;
        s.save_zone(z.clone()).await?;
        start(s, a, g, &z).await?;
    } else {
        s.save_zone(z.clone()).await?;
    }
    state(s, g).await
}
/// Changes beyond the issued lookahead never interrupt the current output epoch.
async fn apply_queue(s:&AppState,a:&Auth,g:&mut Group,z:Zone,restart:bool)->ApiResult<Zone>{
 let mut restart=restart;
 if restart {if let Some(id)=&g.job_id{if let Some(job)=job(s,id).await?{
  let old=state(s,g).await?;let length=job["queue"]["length"].as_u64().unwrap_or(0) as usize;
  if let Some(local)=old.queue_index.checked_sub(g.job_offset){if let Some(offset)=z.queue_index.checked_sub(local){
   let previous=old.queue.get(g.job_offset..g.job_offset.saturating_add(length));let current=z.queue.get(offset..offset.saturating_add(length));
   if length>0&&previous.zip(current).is_some_and(|(a,b)|a.iter().map(|t|&t.id).eq(b.iter().map(|t|&t.id))){g.job_offset=offset;save(s,g).await?;restart=false;}
  }}
 }}}
 apply(s,a,g,z,restart).await
}
fn valid_position(z: &Zone, p: f64) -> ApiResult<f64> {
    if !p.is_finite()
        || p < 0.
        || z.track
            .as_ref()
            .is_some_and(|t| t.duration > 0. && p > t.duration)
    {
        Err(ApiError::bad("播放位置无效"))
    } else {
        Ok(p)
    }
}
pub(crate) async fn command(
    s: &AppState,
    a: &Auth,
    id: &str,
    action: &str,
    v: &Value,
) -> ApiResult<Option<Zone>> {
    if by_zone(s, id).await?.is_none() {
        return Ok(None);
    }
    family(a, "control")?;
    let _operation = s.device_group_lock.lock().await;
    let mut g = by_zone(s, id).await?.ok_or_else(ApiError::missing)?;
    let mut z = state(s, &g).await?;
    if !v["control"].is_null() {
        let c = &v["control"];
        if !matches!(action, "play" | "pause" | "stop")
            || !v["track_id"].is_null()
            || !v["queue_index"].is_null()
        {
            return Err(ApiError::bad("系统控制参数冲突"));
        }
        if c["endpoint_id"] != json!(z.endpoint_id)
            || c["playback_id"] != json!(z.playback_id)
            || c["track_id"] != json!(z.track.as_ref().map(|t| &t.id))
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "stale_playback".into(),
                "播放实例已变化，控制已拒绝。".into(),
            ));
        }
    }
    if [
        "frames",
        "sample_rate",
        "playback_id",
        "measured_version",
        "system_suspend",
    ]
    .iter()
    .any(|k| !v[*k].is_null())
        || v["recoverable"] == true
        || v["device_failed"] == true
    {
        return Err(ApiError::bad("组合设备不接受伪造的输出帧报告。"));
    }
    match action {
        "play" => {
            let mut changed = false;
            if let Some(track) = v["track_id"].as_str() {
                let t = crate::library::get_track(s, track.to_owned()).await?;
                let t = if v["exact_version"] != true && v["control"].is_null() {
                    crate::music_playback_policy::select(s, &a.id, t).await?.0
                } else {
                    t
                };
                if let Some(i) = z.queue.iter().position(|v| v.id == t.id) {
                    z.queue_index = i;
                } else {
                    z.queue.push(t);
                    z.queue_index = z.queue.len() - 1;
                }
                z.track = z.queue.get(z.queue_index).cloned();
                z.position = 0.;
                changed = true;
            }
            if let Some(i) = v["queue_index"].as_u64() {
                let i = i as usize;
                if i >= z.queue.len() {
                    return Err(ApiError::bad("队列位置无效"));
                }
                z.queue_index = i;
                z.track = z.queue.get(i).cloned();
                z.position = 0.;
                changed = true;
            }
            if let Some(p) = v["position"].as_f64() {
                z.position = valid_position(&z, p)?;
                changed = true;
            }
            if z.track.is_none() {
                z.track = z.queue.get(z.queue_index).cloned();
            }
            if z.track.is_none() {
                return Err(ApiError::bad("队列中还没有歌曲。"));
            }
            if !changed {
                if let Some(job) = if let Some(id) = &g.job_id {
                    job(s, id).await?
                } else {
                    None
                } {
                    match job["phase"].as_str() {
                        Some("paused") => {
                            control(s, a, &g, json!({"action":"resume"})).await?;
                            return Ok(Some(state(s, &g).await?));
                        }
                        Some("playing" | "committed" | "preparing") => return Ok(Some(z)),
                        _ => {}
                    }
                }
            }
            if z.track
                .as_ref()
                .is_some_and(|t| t.duration > 0. && z.position >= t.duration)
            {
                z.position = 0.;
            }
            z.status = "playing".into();
            return Ok(Some(apply(s, a, &mut g, z, true).await?));
        }
        "pause" => {
            if g.job_id.is_some() {
                control(s, a, &g, json!({"action":"pause"})).await?;
                return Ok(Some(state(s, &g).await?));
            }
            z.status = "paused".into();
        }
        "stop" => {
            stop(s, a, &mut g).await?;
            z.status = "stopped".into();
            z.playback_id = None;
        }
        "seek" => {
            let p = v["position"]
                .as_f64()
                .ok_or_else(|| ApiError::bad("请指定播放位置"))?;
            z.position = valid_position(&z, p)?;
            if g.job_id.is_some() {
                let index = z.queue_index.saturating_sub(g.job_offset);
                control(
                    s,
                    a,
                    &g,
                    json!({"action":"seek","queue_index":index,"position":p}),
                )
                .await?;
            } else {
                s.save_zone(z.clone()).await?;
            }
            return Ok(Some(state(s, &g).await?));
        }
        "next" | "previous" => {
            if z.queue.is_empty() {
                return Ok(Some(z));
            }
            let target = if action == "previous" {
                z.queue_index
                    .checked_sub(1)
                    .or_else(|| (z.repeat == "all").then_some(z.queue.len() - 1))
            } else {
                (z.queue_index + 1 < z.queue.len())
                    .then_some(z.queue_index + 1)
                    .or_else(|| (z.repeat == "all").then_some(0))
            };
            if let Some(i) = target {
                let was_playing = z.status == "playing";
                z.queue_index = i;
                z.track = z.queue.get(i).cloned();
                z.position = 0.;
                if was_playing {
                    return Ok(Some(apply(s, a, &mut g, z, true).await?));
                }
                stop(s, a, &mut g).await?;
                z.status = "paused".into();
                z.playback_id = None;
            } else {
                stop(s, a, &mut g).await?;
                z.status = "stopped".into();
                z.playback_id = None;
            }
        }
        "progress" | "ended" => {
            return Err(ApiError::bad("组合设备的进度只接受已授权的同步输出报告。"))
        }
        _ => return Err(ApiError::bad("组合设备不支持此操作。")),
    }
    s.save_zone(z.clone()).await?;
    Ok(Some(z))
}
pub(crate) async fn patch(s: &AppState, a: &Auth, id: &str, v: &Value) -> ApiResult<Option<Zone>> {
    if by_zone(s, id).await?.is_none() {
        return Ok(None);
    }
    family(a, "control")?;
    let _operation = s.device_group_lock.lock().await;
    let mut g = by_zone(s, id).await?.ok_or_else(ApiError::missing)?;
    let mut z = state(s, &g).await?;
    let fields = v.as_object().ok_or_else(|| ApiError::bad("设置须为对象"))?;
    if fields.keys().any(|k| {
        !matches!(
            k.as_str(),
            "name" | "volume" | "muted" | "repeat" | "shuffle" | "autoplay" | "transitions"
        )
    }) {
        return Err(ApiError::bad(
            "组合设备设置不支持此字段，请在成员设备中配置硬件。",
        ));
    }
    if let Some(value) = fields.get("name") {
        let name = value.as_str().unwrap_or("").trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(ApiError::bad("组合名称无效"));
        }
        g.name = name.into();
        z.name = name.into();
        g.revision += 1;
    }
    if let Some(value) = fields.get("volume") {
        z.volume = value
            .as_f64()
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .ok_or_else(|| ApiError::bad("音量范围为 0 到 1"))?;
    }
    if let Some(value) = fields.get("muted") {
        z.muted = value
            .as_bool()
            .ok_or_else(|| ApiError::bad("静音设置无效"))?;
    }
    if let Some(value) = fields.get("repeat") {
        let repeat = value.as_str().unwrap_or("");
        if !["off", "one", "all"].contains(&repeat) {
            return Err(ApiError::bad("循环模式无效"));
        }
        z.repeat = repeat.into();
    }
    let mut restart = fields.contains_key("repeat") && z.status == "playing";
    if let Some(value) = fields.get("shuffle") {
        let shuffle = value
            .as_bool()
            .ok_or_else(|| ApiError::bad("随机模式无效"))?;
        if shuffle && !z.shuffle {
            crate::zones::shuffle_tail(&mut z);
            restart = z.status == "playing";
        }
        z.shuffle = shuffle;
    }
    if let Some(value) = fields.get("autoplay") {
        let config: crate::models::Autoplay =
            serde_json::from_value(value.clone()).map_err(|_| ApiError::bad("自动续播设置无效"))?;
        if !crate::connectors::valid_id(&config.source) {
            return Err(ApiError::bad("推荐来源无效"));
        }
        z.autoplay = config;
    }
    if let Some(value) = fields.get("transitions") {
        let config: crate::models::Transitions =
            serde_json::from_value(value.clone()).map_err(|_| ApiError::bad("淡入淡出设置无效"))?;
        if config.fade_in_ms > 10000 || config.fade_out_ms > 10000 {
            return Err(ApiError::bad("淡入淡出最长十秒"));
        }
        z.transitions = config;
        restart = z.status == "playing";
    }
    for member in &g.zone_ids {
        let mut candidate = s
            .zones
            .read()
            .await
            .get(member)
            .cloned()
            .ok_or_else(ApiError::missing)?;
        candidate.dsp = z.dsp.clone();
        candidate.transitions = z.transitions.clone();
        crate::zones::processing_allowed(s, &candidate).await?;
    }
    if fields.contains_key("volume") || fields.contains_key("muted") {
        let _zone = s.zone_lock.lock().await;
        crate::sync_start::volume_all(s, &g.zone_ids, z.volume, z.muted).await?;
    }
    if z.status != "playing"
        && ["repeat", "shuffle", "transitions"]
            .iter()
            .any(|k| fields.contains_key(*k))
    {
        stop(s, a, &mut g).await?;
    }
    save(s, &g).await?;
    publish_endpoint(s, &g).await?;
    Ok(Some(apply(s, a, &mut g, z, restart).await?))
}
pub(crate) async fn dsp(s: &AppState, a: &Auth, id: &str, v: &Value) -> ApiResult<Option<Zone>> {
    if by_zone(s, id).await?.is_none() {
        return Ok(None);
    }
    family(a, "control")?;
    let _operation = s.device_group_lock.lock().await;
    let mut g = by_zone(s, id).await?.ok_or_else(ApiError::missing)?;
    let mut z = state(s, &g).await?;
    z.dsp = v.clone();
    for id in &g.zone_ids {
        let mut member = s
            .zones
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(ApiError::missing)?;
        member.dsp = v.clone();
        crate::zones::processing_allowed(s, &member).await?;
    }
    let restart = z.status == "playing";
    if !restart {
        stop(s, a, &mut g).await?;
    }
    Ok(Some(apply(s, a, &mut g, z, restart).await?))
}
pub(crate) async fn queue(
    s: &AppState,
    a: &Auth,
    id: &str,
    v: &Value,
    append: bool,
) -> ApiResult<Option<Zone>> {
    if by_zone(s, id).await?.is_none() {
        return Ok(None);
    }
    family(a, "control")?;
    let _operation = s.device_group_lock.lock().await;
    let mut g = by_zone(s, id).await?.ok_or_else(ApiError::missing)?;
    let mut z = state(s, &g).await?;
    let ids = v["track_ids"]
        .as_array()
        .ok_or_else(|| ApiError::bad("请指定歌曲列表"))?;
    if ids.len() > 10000 || append && z.queue.len() + ids.len() > 10000 {
        return Err(ApiError::bad("队列上限为一万首"));
    }
    let next = v["play_next"] == true;
    if next && (!append || !v["insertion_index"].is_null() || !v["queue_index"].is_null()) {
        return Err(ApiError::bad("下一首播放参数冲突"));
    }
    let mut tracks = Vec::new();
    for id in ids {
        tracks.push(
            crate::library::get_track(
                s,
                id.as_str()
                    .ok_or_else(|| ApiError::bad("歌曲标识无效"))?
                    .into(),
            )
            .await?,
        );
    }
    let tracks = crate::catalog::prefer_queue(s, tracks).await?;
    let playing = z.status == "playing";
    if append {
        let index = if next {
            if z.track.is_some() {
                z.queue_index + 1
            } else {
                0
            }
        } else {
            v["insertion_index"]
                .as_u64()
                .map(|v| v as usize)
                .unwrap_or(z.queue.len())
        }
        .min(z.queue.len());
        if index <= z.queue_index && z.track.is_some() {
            z.queue_index += tracks.len();
        }
        z.queue.splice(index..index, tracks);
    } else {
        let old = z.track.as_ref().map(|t| t.id.clone());
        z.queue = tracks;
        if let Some(index) = v["queue_index"].as_u64() {
            if index as usize >= z.queue.len() {
                return Err(ApiError::bad("队列位置无效"));
            }
            z.queue_index = index as usize;
            z.track = z.queue.get(z.queue_index).cloned();
            z.position = 0.;
        } else if let Some(i) = z.queue.iter().position(|t| Some(&t.id) == old.as_ref()) {
            z.queue_index = i;
        } else {
            z.queue_index = 0;
            z.track = None;
            z.position = 0.;
            z.status = "stopped".into();
            z.playback_id = None;
        }
        if let Some(shuffle) = v["shuffle"].as_bool() {
            z.shuffle = shuffle;
        }
        if z.shuffle {
            crate::zones::shuffle_tail(&mut z);
        }
    }
    if z.status != "playing" {
        stop(s, a, &mut g).await?;
    }
    let restart = playing && g.job_id.is_some();
    Ok(Some(if v["queue_index"].is_null(){apply_queue(s,a,&mut g,z,restart).await?}else{apply(s,a,&mut g,z,restart).await?}))
}
pub(crate) async fn reorder(
    s: &AppState,
    a: &Auth,
    id: &str,
    v: &Value,
) -> ApiResult<Option<Zone>> {
    if by_zone(s, id).await?.is_none() {
        return Ok(None);
    }
    family(a, "control")?;
    let _operation = s.device_group_lock.lock().await;
    let mut g = by_zone(s, id).await?.ok_or_else(ApiError::missing)?;
    let mut z = state(s, &g).await?;
    let expected = v["expected_queue"]
        .as_array()
        .ok_or_else(|| ApiError::bad("缺少原队列"))?;
    if v["expected_playback_id"] != json!(z.playback_id)
        || expected
            .iter()
            .map(Value::as_str)
            .ne(z.queue.iter().map(|t| Some(t.id.as_str())))
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "queue_edit_conflict".into(),
            "队列已经变化，请刷新后重试。".into(),
        ));
    }
    let order = v["order"]
        .as_array()
        .ok_or_else(|| ApiError::bad("缺少目标曲序"))?;
    let mut indices = Vec::new();
    let mut seen = HashSet::new();
    for v in order {
        let i = v.as_u64().ok_or_else(|| ApiError::bad("曲序无效"))? as usize;
        if i >= z.queue.len() || !seen.insert(i) {
            return Err(ApiError::bad("曲序重复或超出范围"));
        }
        indices.push(i);
    }
    if indices.len() != z.queue.len() {
        return Err(ApiError::bad("排序须包含每一个歌曲位置"));
    }
    let current = z.queue_index;
    z.queue = indices.iter().map(|i| z.queue[*i].clone()).collect();
    z.queue_index = indices.iter().position(|i| *i == current).unwrap_or(0);
    let restart = z.status == "playing" && g.job_id.is_some();
    if !restart {
        stop(s, a, &mut g).await?;
    }
    Ok(Some(apply_queue(s, a, &mut g, z, restart).await?))
}

/// Do not run another start while the coordinator holds its registry/zone locks.
pub(super) fn schedule_continue(s: AppState, id: String, expected: String, length: usize) {
    let work: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
        Box::pin(async move {
            let operation = s.device_group_lock.lock().await;
            let Ok(Some(mut g)) = load(&s, &id).await else {
                return;
            };
            if g.deleted || g.job_id.as_deref() != Some(expected.as_str()) || g.phase != "ended" {
                return;
            }
            let Ok(mut z) = state(&s, &g).await else {
                return;
            };
            let wants_radio=z.repeat=="off"&&g.job_offset+length>=z.queue.len()&&z.autoplay.enabled;
            let context=crate::radio::QueueContext::from_zone(&z);
            let seed=z.queue.last().map(|t|t.id.clone());
            let owner=crate::radio::owner(&s,&z.id).await.ok().flatten();
            let items=if wants_radio {
                drop(operation);
                let result=crate::radio::next_for_queue(&s,owner,seed,vec![],context.clone(),10).await;
                let operation=s.device_group_lock.lock().await;
                let Ok(Some(latest))=load(&s,&id).await else{return;};
                let Ok(current)=state(&s,&latest).await else{return;};
                if latest.deleted||latest.job_id.as_deref()!=Some(expected.as_str())||latest.phase!="ended"
                    ||!current.autoplay.enabled||current.repeat!="off"
                    ||current.queue.iter().map(|t|&t.id).ne(z.queue.iter().map(|t|&t.id)) {return;}
                g=latest;z=current;
                (operation,result)
            }else{(operation,Ok(vec![]))};
            let (_operation,items)=items;
            let result: ApiResult<()> = async {
                let a = crate::auth::authenticate_hash(&s, g.controller.clone()).await?;
                family(&a, "control")?;
                let next = if z.repeat == "one" {
                    Some(z.queue_index)
                } else if g.job_offset + length < z.queue.len() {
                    Some(g.job_offset + length)
                } else if z.repeat == "all" && !z.queue.is_empty() {
                    if z.shuffle {
                        crate::zones::shuffle_tail(&mut z);
                    }
                    Some(0)
                } else if z.autoplay.enabled {
                    let items=items?;
                    if items.is_empty() { None } else {Some(crate::autoplay::extend_queue(&mut z,items))}
                } else {
                    None
                };
                if let Some(index) = next {
                    z.queue_index = index;
                    z.track = z.queue.get(index).cloned();
                    z.position = 0.;
                    z.status = "playing".into();
                    start(&s, &a, &mut g, &z).await?;
                }
                Ok(())
            }
            .await;
            if let Err(e) = result {
                g.error = Some(e.2);
                g.phase = "failed".into();
                let _ = save(&s, &g).await;
                let _ = publish_endpoint(&s, &g).await;
            }
        });
    tokio::spawn(work);
}
