//! Persistent logical output devices. Member queues are leases, never the group's queue.
mod transport;
mod members;
pub(crate) mod transfer;
pub use members::edit;
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::Zone,
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
pub(crate) use transport::{command, dsp, patch, queue, reorder};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Group {
    pub id: String,
    pub name: String,
    pub zone_id: String,
    pub endpoint_id: String,
    pub zone_ids: Vec<String>,
    pub source_zone_id: String,
    pub revision: u64,
    pub phase: String,
    pub job_id: Option<String>,
    pub job_offset: usize,
    pub controller: String,
    pub error: Option<String>,
    pub deleted: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    request_id: String,
    name: String,
    zone_ids: Vec<String>,
    source_zone_id: String,
}
pub fn schema(c: &rusqlite::Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS device_groups(id TEXT PRIMARY KEY,zone_id TEXT NOT NULL UNIQUE,job_id TEXT,data TEXT NOT NULL);CREATE INDEX IF NOT EXISTS device_groups_job ON device_groups(job_id);CREATE TABLE IF NOT EXISTS device_group_members(zone_id TEXT PRIMARY KEY,endpoint_id TEXT NOT NULL UNIQUE,group_id TEXT NOT NULL,snapshot TEXT NOT NULL);")?;
    Ok(())
}
fn family(a: &Auth, scope: &str) -> ApiResult<()> {
    a.require(scope)?;
    if a.role == "external" {
        return Err(ApiError::forbidden());
    }
    Ok(())
}
pub(crate) async fn by_zone(s: &AppState, id: &str) -> ApiResult<Option<Group>> {
    let id = id.to_owned();
    Ok(s.db
        .read(move |c| {
            let v = c
                .query_row(
                    "SELECT data FROM device_groups WHERE zone_id=?1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            v.map(|v| serde_json::from_str::<Group>(&v))
                .transpose()
                .map(|v| v.filter(|g| !g.deleted))
                .map_err(Into::into)
        })
        .await?)
}
pub(crate) async fn load(s: &AppState, id: &str) -> ApiResult<Option<Group>> {
    let id = id.to_owned();
    Ok(s.db
        .read(move |c| {
            let v = c
                .query_row("SELECT data FROM device_groups WHERE id=?1", [id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?;
            v.map(|v| serde_json::from_str::<Group>(&v))
                .transpose()
                .map_err(Into::into)
        })
        .await?)
}
pub(crate) async fn save(s: &AppState, g: &Group) -> ApiResult<()> {
    let g = g.clone();
    s.db.run(move |c| {
        c.execute(
            "UPDATE device_groups SET job_id=?2,data=?3 WHERE id=?1",
            params![g.id, g.job_id, serde_json::to_string(&g)?],
        )?;
        Ok(())
    })
    .await?;
    s.emit("device_groups", json!({}));
    Ok(())
}
pub(crate) async fn member_guard(s: &AppState, zone: &str, allowed: Option<&str>) -> ApiResult<()> {
    let id = zone.to_owned();
    let owner =
        s.db.read(move |c| {
            Ok(c.query_row(
                "SELECT group_id FROM device_group_members WHERE zone_id=?1",
                [id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
        })
        .await?;
    if owner.as_deref().is_some_and(|g| Some(g) != allowed) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "device_group_member".into(),
            "此输出已加入组合，请选择组合设备播放，或先将它从组合中拆开。".into(),
        ));
    }
    Ok(())
}
pub(crate) async fn endpoint_guard(s: &AppState, endpoint: &str) -> ApiResult<()> {
    if s.endpoints
        .read()
        .await
        .get(endpoint)
        .is_some_and(|e| e["kind"] == "group")
    {
        return Err(ApiError::bad(
            "请使用拆开设备组合来恢复原设备，不要删除组合输出。",
        ));
    }
    let id = endpoint.to_owned();
    let held =
        s.db.read(move |c| {
            Ok(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM device_group_members WHERE endpoint_id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?;
    if held {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "device_group_member".into(),
            "此输出已加入组合，请先拆开设备组合。".into(),
        ));
    }
    Ok(())
}
pub(crate) fn eligible(e: &Value) -> bool {
    let kind = e["kind"].as_str().unwrap_or("");
    let platform = e["capabilities"]["platform"].as_str().unwrap_or("");
    !matches!(kind, "browser" | "android" | "group")
        && !matches!(platform, "android" | "ios")
        && matches!(kind, "coreaudio" | "wasapi" | "asio" | "alsa")
        && [
            "playback_instance",
            "scheduled_start_v1",
            "scheduled_queue_v1",
            "group_output_v2",
            "group_output_final_v2",
            "measured_listens_v2",
            "host_clock_v1",
            "device_clock_v1",
            "programme_clock_v1",
        ]
        .iter()
        .all(|k| e["capabilities"][*k] == true)
}
pub(crate) async fn publish_endpoint(s: &AppState, g: &Group) -> ApiResult<()> {
    let zones = s.zones.read().await;
    let members = g
        .zone_ids
        .iter()
        .filter_map(|id| zones.get(id))
        .map(|z| z.endpoint_id.clone())
        .collect::<Vec<_>>();
    drop(zones);
    let endpoints = s.endpoints.read().await;
    let online = members.len() == g.zone_ids.len()
        && members
            .iter()
            .all(|id| endpoints.get(id).is_some_and(|e| e["online"] == true));
    drop(endpoints);
    let mut endpoint = json!({"id":g.endpoint_id,"name":g.name,"kind":"group","online":online,"verified":false,"capabilities":{"virtual_device_v1":true,"device_group_v1":true,"strict_sync":false,"dsp":false},"group":{"id":g.id,"zone_ids":g.zone_ids,"endpoint_ids":members,"phase":g.phase,"error":g.error}});
    decorate(s, &mut endpoint).await;
    s.endpoints
        .write()
        .await
        .insert(g.endpoint_id.clone(), endpoint);
    s.emit("endpoints", json!({}));
    Ok(())
}
fn public(g: &Group) -> Value {
    json!({"id":g.id,"name":g.name,"zone_id":g.zone_id,"endpoint_id":g.endpoint_id,"zone_ids":g.zone_ids,"source_zone_id":g.source_zone_id,"revision":g.revision,"phase":g.phase,"error":g.error,"job_id":g.job_id})
}
pub async fn list(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    family(&a, "read")?;
    let groups =
        s.db.read(|c| {
            let rows = c
                .prepare("SELECT data FROM device_groups ORDER BY id")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(|v| serde_json::from_str::<Group>(&v).map_err(Into::into))
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await?;
    Ok(Json(
        json!({"items":groups.iter().filter(|g|!g.deleted).map(public).collect::<Vec<_>>()}),
    ))
}
pub async fn create(
    State(s): State<AppState>,
    a: Auth,
    Json(input): Json<Create>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    family(&a, "control")?;
    tokio::spawn(async move { create_inner(s, a, input).await })
        .await
        .map_err(anyhow::Error::from)?
}
async fn create_inner(
    s: AppState,
    a: Auth,
    mut input: Create,
) -> ApiResult<(StatusCode, Json<Value>)> {
    family(&a, "control")?;
    let id = uuid::Uuid::parse_str(&input.request_id)
        .map_err(|_| ApiError::bad("组合请求须包含 UUID"))?
        .to_string();
    input.name = input.name.trim().into();
    input.zone_ids.sort();
    if input.name.is_empty()
        || input.name.chars().count() > 80
        || input.name.chars().any(char::is_control)
        || !(2..=4).contains(&input.zone_ids.len())
        || input.zone_ids.windows(2).any(|v| v[0] == v[1])
        || !input.zone_ids.contains(&input.source_zone_id)
    {
        return Err(ApiError::bad(
            "请输入组合名称，选择二至四个不同输出，并指定其中一个作为初始队列来源。",
        ));
    }
    let _operation = s.device_group_lock.lock().await;
    if let Some(g) = load(&s, &id).await? {
        if g.deleted
            || g.name != input.name
            || g.zone_ids != input.zone_ids
            || g.source_zone_id != input.source_zone_id
        {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "group_conflict".into(),
                "请求已用于其他组合，或组合已经拆开。".into(),
            ));
        }
        return Ok((StatusCode::OK, Json(public(&g))));
    }
    let guard = s.zone_lock.lock().await;
    let mut snapshots = Vec::new();
    let mut ids = HashSet::new();
    for id in &input.zone_ids {
        member_guard(&s, id, None).await?;
        crate::accounts::scope::check(&s, &a, "zone", id).await?;
        let z = s
            .zones
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(ApiError::missing)?;
        crate::accounts::scope::check(&s, &a, "endpoint", &z.endpoint_id).await?;
        let e = s
            .endpoints
            .read()
            .await
            .get(&z.endpoint_id)
            .cloned()
            .unwrap_or(Value::Null);
        if !eligible(&e)
            || e["online"] != true
            || !ids.insert(z.endpoint_id.clone())
            || !s.bridges.read().await.contains_key(&z.endpoint_id)
        {
            return Err(ApiError::bad("请选择在线且支持同步的 Mac、Windows 原生输出或 Linux 网桥。网页与安卓不能作为同步成员。"));
        }
        if crate::sync_start::active_zone(&s, id).await {
            return Err(ApiError::bad("输出仍在旧的分组任务中，请先结束旧任务。"));
        }
        if s.zones.read().await.values().any(|other|other.id!=z.id&&other.endpoint_id==z.endpoint_id&&(other.status!="stopped"||other.playback_id.is_some())){return Err(ApiError::bad("同一输出仍有其他音区在使用，请先停止或删除重复音区。"));}
        snapshots.push(z);
    }
    let source = snapshots
        .iter()
        .find(|z| z.id == input.source_zone_id)
        .unwrap();
    let mut virtual_zone = source.clone();
    virtual_zone.id = format!("group-zone-{id}");
    virtual_zone.endpoint_id = format!("group-{id}");
    virtual_zone.name = input.name.clone();
    virtual_zone.status = if source.track.is_some() {
        "paused"
    } else {
        "stopped"
    }
    .into();
    virtual_zone.playback_id = None;
    virtual_zone.updated_at = chrono::Utc::now().timestamp_millis();
    let mut g = Group {
        id,
        name: input.name,
        zone_id: virtual_zone.id.clone(),
        endpoint_id: virtual_zone.endpoint_id.clone(),
        zone_ids: input.zone_ids,
        source_zone_id: input.source_zone_id,
        revision: 1,
        phase: "joining".into(),
        job_id: None,
        job_offset: 0,
        controller: a.session_hash.clone(),
        error: None,
        deleted: false,
    };
    let stored = g.clone();
    let original = snapshots.clone();
    let saved = virtual_zone.clone();
    s.db.run(move|c|{let tx=c.transaction()?;tx.execute("INSERT INTO device_groups(id,zone_id,job_id,data) VALUES(?1,?2,NULL,?3)",params![stored.id,stored.zone_id,serde_json::to_string(&stored)?])?;
  for z in original{tx.execute("INSERT INTO device_group_members(zone_id,endpoint_id,group_id,snapshot) VALUES(?1,?2,?3,?4)",params![z.id,z.endpoint_id,stored.id,serde_json::to_string(&z)?])?;}
  tx.execute("INSERT INTO zones(id,data) VALUES(?1,?2)",params![saved.id,serde_json::to_string(&saved)?])?;tx.commit()?;Ok(())}).await?;
    s.zones
        .write()
        .await
        .insert(virtual_zone.id.clone(), virtual_zone.clone());
    drop(guard);
    // Preserve source queues before issuing any stop. The joined worker serializes stop/prepare.
    for z in &snapshots {
        if let Err(error) = transport::stop_member(&s, &a, &z.id).await {
            g.error = Some(error.2.clone());
            save(&s, &g).await?;
            restore_members(&s, &mut g).await?;
            return Err(error);
        }
    }
    g.phase = "ready".into();
    save(&s, &g).await?;
    publish_endpoint(&s, &g).await?;
    if source.status == "playing" && source.track.is_some() {
        virtual_zone.status = "playing".into();
        s.save_zone(virtual_zone.clone()).await?;
        if let Err(error) = transport::start(&s, &a, &mut g, &virtual_zone).await {
            g.error = Some(error.2.clone());
            virtual_zone.status = "paused".into();
            s.save_zone(virtual_zone).await?;
            save(&s, &g).await?;
            publish_endpoint(&s, &g).await?;
        }
    }
    s.emit("zones_changed", json!({}));
    Ok((StatusCode::CREATED, Json(public(&g))))
}
async fn restore_members(s: &AppState, g: &mut Group) -> ApiResult<()> {
    let id = g.id.clone();
    let rows =
        s.db.read(move |c| {
            Ok(c.prepare(
                "SELECT snapshot FROM device_group_members WHERE group_id=?1 ORDER BY zone_id",
            )?
            .query_map([id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await?;
    let mut zones = rows
        .into_iter()
        .map(|v| serde_json::from_str::<Zone>(&v).map_err(anyhow::Error::from))
        .collect::<anyhow::Result<Vec<_>>>()?;
    for z in &mut zones {
        if z.status == "playing" {
            z.status = "paused".into();
        }
        z.playback_id = None;
        z.updated_at = chrono::Utc::now().timestamp_millis();
    }
    let _lock = s.zone_lock.lock().await;
    g.deleted = true;
    g.phase = "dissolved".into();
    g.job_id = None;
    g.revision += 1;
    let stored = g.clone();
    s.revise_queues(&mut zones).await;
    let values = zones.clone();
    s.db.run(move |c| {
        let tx = c.transaction()?;
        for z in values {
            tx.execute(
                "UPDATE zones SET data=?2 WHERE id=?1",
                params![z.id, serde_json::to_string(&z)?],
            )?;
        }
        tx.execute("DELETE FROM zones WHERE id=?1", [&stored.zone_id])?;
        tx.execute(
            "DELETE FROM device_group_members WHERE group_id=?1",
            [&stored.id],
        )?;
        tx.execute(
            "UPDATE device_groups SET job_id=NULL,data=?2 WHERE id=?1",
            params![stored.id, serde_json::to_string(&stored)?],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    {
        let mut map = s.zones.write().await;
        map.remove(&g.zone_id);
        map.extend(zones.into_iter().map(|z| (z.id.clone(), z)));
    }
    s.endpoints.write().await.remove(&g.endpoint_id);
    s.emit("zones_changed", json!({}));
    s.emit("endpoints", json!({}));
    s.emit("device_groups", json!({}));
    Ok(())
}
pub async fn delete(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    family(&a, "control")?;
    tokio::spawn(async move { delete_inner(s, a, id).await })
        .await
        .map_err(anyhow::Error::from)?
}
async fn delete_inner(s: AppState, a: Auth, id: String) -> ApiResult<StatusCode> {
    family(&a, "control")?;
    let _operation = s.device_group_lock.lock().await;
    let Some(mut g) = load(&s, &id).await? else {
        return Ok(StatusCode::NO_CONTENT);
    };
    if g.deleted {
        return Ok(StatusCode::NO_CONTENT);
    }
    transport::stop(&s, &a, &mut g).await?;
    restore_members(&s, &mut g).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) async fn restore(s: &AppState) -> anyhow::Result<()> {
    let groups =
        s.db.read(|c| {
            let data = c
                .prepare("SELECT data FROM device_groups")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            data.into_iter()
                .map(|v| serde_json::from_str::<Group>(&v).map_err(Into::into))
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .await?;
    for mut g in groups.into_iter().filter(|g| !g.deleted) {
        if g.phase == "joining" {
            restore_members(s, &mut g)
                .await
                .map_err(|e| anyhow::anyhow!(e.2))?;
            continue;
        }
        g.job_id = None;
        g.phase = "ready".into();
        g.controller.clear();
        save(s, &g).await.map_err(|e| anyhow::anyhow!(e.2))?;
        let restored_zone = s.zones.read().await.get(&g.zone_id).cloned();
        if let Some(mut z) = restored_zone {
            z.status = if z.track.is_some() {
                "paused"
            } else {
                "stopped"
            }
            .into();
            z.playback_id = None;
            s.save_zone(z).await?;
        }
        publish_endpoint(s, &g)
            .await
            .map_err(|e| anyhow::anyhow!(e.2))?;
    }
    Ok(())
}
/// Project job state onto its persistent virtual zone without changing member snapshots.
pub(crate) async fn project(s: &AppState, job: &Value) -> anyhow::Result<()> {
    let id = job["id"].as_str().unwrap_or("").to_owned();
    let group =
        s.db.read(move |c| {
            let v = c
                .query_row(
                    "SELECT data FROM device_groups WHERE job_id=?1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            v.map(|v| serde_json::from_str::<Group>(&v))
                .transpose()
                .map_err(Into::into)
        })
        .await?;
    let Some(mut g) = group.filter(|g| !g.deleted) else {
        return Ok(());
    };
    let Some(mut zone) = s.zones.read().await.get(&g.zone_id).cloned() else {
        return Ok(());
    };
    let phase = job["phase"].as_str().unwrap_or("");
    let finished = phase == "ended" && g.phase != "ended";
    zone.status = match phase {
        "preparing" | "committed" | "playing" => "playing",
        "paused" | "pausing" | "seeking" => "paused",
        _ => "stopped",
    }
    .into();
    zone.queue_index = (g.job_offset
        + job["playhead"]["queue_index"].as_u64().unwrap_or(0) as usize)
        .min(zone.queue.len().saturating_sub(1));
    zone.track = zone.queue.get(zone.queue_index).cloned();
    zone.position = job["playhead"]["position"]
        .as_f64()
        .unwrap_or(zone.position);
    zone.updated_at = chrono::Utc::now().timestamp_millis();
    zone.playback_id = if zone.status == "stopped" {
        None
    } else {
        Some(format!(
            "{}:{}",
            g.id,
            job["members"][0]["current_playback_id"]
                .as_str()
                .unwrap_or("pending")
        ))
    };
    s.save_zone(zone).await?;
    if g.phase != phase || g.error.as_deref() != job["error"].as_str() {
        g.phase = phase.into();
        g.error = job["error"].as_str().map(str::to_owned);
        save(s, &g).await.map_err(|e| anyhow::anyhow!(e.2))?;
        publish_endpoint(s, &g)
            .await
            .map_err(|e| anyhow::anyhow!(e.2))?;
    }
    if finished {
        transport::schedule_continue(
            s.clone(),
            g.id,
            job["id"].as_str().unwrap_or("").into(),
            job["queue"]["length"].as_u64().unwrap_or(1) as usize,
        );
    }
    Ok(())
}
/// A group has one public signal/visualization identity, backed by its first native output.
pub(crate) async fn leader(s: &AppState, zone: &Zone) -> ApiResult<Option<Zone>> {
    let Some(g) = by_zone(s, &zone.id).await? else {
        return Ok(None);
    };
    let zones = s.zones.read().await;
    Ok(g.zone_ids
        .iter()
        .filter_map(|id| zones.get(id))
        .find(|member| {
            member.track.as_ref().map(|t| &t.id) == zone.track.as_ref().map(|t| &t.id)
                && member.status == "playing"
        })
        .cloned())
}
pub(crate) async fn decorate(s: &AppState, endpoint: &mut Value) {
    if endpoint["kind"] != "group" {
        return;
    }
    let all = s.endpoints.read().await;
    let ids = endpoint["group"]["endpoint_ids"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    endpoint["online"] = json!(
        !ids.is_empty()
            && ids.iter().all(|id| id
                .as_str()
                .and_then(|id| all.get(id))
                .is_some_and(|e| e["online"] == true))
    );
    let first = ids
        .first()
        .and_then(Value::as_str)
        .and_then(|id| all.get(id));
    for cap in ["dsp", "convolution_v1", "transitions"] {
        endpoint["capabilities"][cap] = json!(
            !ids.is_empty()
                && ids.iter().all(|id| id
                    .as_str()
                    .and_then(|id| all.get(id))
                    .is_some_and(|e| e["capabilities"][cap] != false
                        && (cap != "convolution_v1" || e["capabilities"][cap] == true)))
        );
    }
    endpoint["capabilities"]["visualization_v1"] =
        json!(first.is_some_and(|e| e["capabilities"]["visualization_v1"] == true));
    if let Some(playback) = first.and_then(|e| e.get("playback")) {
        endpoint["playback"] = playback.clone();
    }
    drop(all);
    let zone = s
        .zones
        .read()
        .await
        .values()
        .find(|z| z.endpoint_id == endpoint["id"])
        .cloned();
    if let Some(z) = zone {
        if endpoint["playback"].is_object() {
            endpoint["playback"]["core_playback_id"] = json!(z.playback_id);
            endpoint["playback"]["zone_id"] = json!(z.id);
            endpoint["playback"]["status"] = json!(z.status);
        }
    }
}
