use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{extract::State, Json};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
pub async fn get(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let mut value = json!({"name":"R-Music","cache_limit_bytes":2_u64*1024*1024*1024,"metadata_enabled":true,"metadata_provider":null,"stream_quality":"best","low_performance":false,"client_cache_default_bytes":5_u64*1024*1024*1024,"catalog_scrape_enabled":false,"catalog_scrape_concurrency":1,"artwork_limit_bytes":crate::artwork_maintenance::DEFAULT_LIMIT,"strict_sync_available":false,"hqplayer_available":false,"statistics_timezone":crate::stats::DEFAULT_TIMEZONE});
    let items =
        s.db.run(|c| {
            let mut q = c.prepare("SELECT key,value FROM settings")?;
            let rows = q
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    for (k, v) in items {
        if !["name","cache_limit_bytes","metadata_enabled","metadata_provider","metadata_provider_priority","low_performance","stream_quality","managed_memory_bytes","background_concurrency","analysis_threads","playback_cache_mode","cache_enabled","provider_request_limits","song_fetch_concurrency","artwork_limit_bytes","catalog_scrape_enabled","catalog_scrape_concurrency","statistics_timezone"].contains(&k.as_str()) {continue;}
        value[k] = serde_json::from_str(&v).unwrap_or(Value::Null);
    }
    value["hqplayer_available"]=json!(s.endpoints.read().await.values().any(|endpoint|endpoint["kind"]=="hqplayer"&&endpoint["online"]==true));
    for (key,default) in [("managed_memory_bytes",json!(1024_u64*1024*1024)),("background_concurrency",json!(2)),("analysis_threads",json!(1)),("playback_cache_mode",json!("full"))] {if value.get(key).is_none(){value[key]=default;}}
    value["cache_enabled"]=json!(crate::cache_policy::enabled(&s).await);
    value["cache_forced_off"]=json!(false);
    value["lossless_transport_only"]=s.setting("lossless_transport_only",json!(true)).await;
    value["mobile_opus_enabled"]=s.setting("mobile_opus_enabled",json!(false)).await;
    value["resources"]=s.resources.status(&s).await;
    value["provider_request_limits"]=serde_json::to_value(s.traffic.policy.read().unwrap().clone()).unwrap();
    value["song_fetch_concurrency"]=json!(s.traffic.song_limit.load(std::sync::atomic::Ordering::Acquire));
    value["metadata_provider_priority"]=json!(crate::connectors::ordered_metadata_providers(&s).await);
    let mut choices=vec![];for id in crate::connectors::providers(&s,"metadata").await {if let Ok(m)=crate::connectors::manifest(&s,&id).await{choices.push(json!({"id":id,"name":m["name"]}));}}
    value["metadata_providers"]=json!(choices);
    if a.role=="external"{if let Some(o)=value.as_object_mut(){o.retain(|k,_|["name","client_cache_default_bytes","statistics_timezone","mobile_opus_enabled","lossless_transport_only"].contains(&k.as_str()));}}
    Ok(Json(value))
}
pub async fn patch(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<Value>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let Some(map) = v.as_object() else {
        return Err(ApiError::bad("设置必须为对象"));
    };
    for (k, v) in map {
        match k.as_str() {
            "statistics_timezone" => {if v.as_str().filter(|name|name.len()<=100).and_then(|name|name.parse::<chrono_tz::Tz>().ok()).is_none(){return Err(ApiError::bad("统计时区须为有效 IANA 时区，例如 Asia/Shanghai"));}}
            "catalog_scrape_concurrency"=>{if v.as_u64().is_none_or(|n|!(1..=4).contains(&n)){return Err(ApiError::bad("资料刮削并发应为1–4"));}}
            "artwork_limit_bytes"=>{if v.as_u64().is_none_or(|n|n==0||n>1024_u64.pow(4)){return Err(ApiError::bad("图片预算应为1字节至1TiB"));}}

            "provider_request_limits" => {if serde_json::from_value::<crate::traffic::Policy>(v.clone()).ok().is_none_or(|p|!p.valid()){return Err(ApiError::bad("接口间隔应为 50–60000 ms，限流冷却应为 1–3600 秒"));}}
            "song_fetch_concurrency" => {if v.as_u64().is_none_or(|n|!(1..=16).contains(&n)){return Err(ApiError::bad("同时拉取歌曲数量应为 1–16"));}}
            "managed_memory_bytes" => {if v.as_u64().is_none_or(|n|!(32*1024*1024..=128_u64*1024*1024*1024).contains(&n)){return Err(ApiError::bad("受控内存预算应为 32 MiB 至 128 GiB"));}}
            "background_concurrency"|"analysis_threads" => {if v.as_u64().is_none_or(|n|!(1..=16).contains(&n)){return Err(ApiError::bad("并发／线程限制应为 1–16"));}}
            "playback_cache_mode" => {if !matches!(v.as_str(),Some("full"|"partial")){return Err(ApiError::bad("播放缓存策略无效"));}}
            "metadata_provider_priority" => {
                let ids=v.as_array().filter(|v|v.len()<=16).ok_or_else(||ApiError::bad("资料优先级最多16个平台"))?;
                let enabled=crate::connectors::providers(&s,"metadata").await;let mut seen=std::collections::HashSet::new();
                for id in ids {let id=id.as_str().filter(|id|enabled.iter().any(|v|v==id)).ok_or_else(||ApiError::bad("资料平台未启用或不支持资料能力"))?;if !seen.insert(id){return Err(ApiError::bad("资料优先级不能重复"));}}
            }
            "metadata_provider" => {let id=v.as_str().ok_or_else(||ApiError::bad("请选择资料连接器"))?;if !crate::connectors::providers(&s,"metadata").await.iter().any(|p|p==id){return Err(ApiError::bad("此音源未声明资料能力"));}}
            "stream_quality" => {
                if v.as_str().is_none_or(|q|!crate::catalog::QUALITIES.contains(&q)){return Err(ApiError::bad("在线音质无效"));}
            }
            "name" => {
                if v.as_str().is_none_or(|s| s.is_empty() || s.len() > 100) {
                    return Err(ApiError::bad("核心名称无效"));
                }
            }
            "cache_limit_bytes" => {
                if v.as_u64().is_none_or(|n| n < 1 || n > 1024_u64.pow(5)) {
                    return Err(ApiError::bad("缓存容量无效"));
                }
            }
            "cache_enabled" if v != &json!(true) => return Err(ApiError::bad("音频缓存始终启用，请调整缓存预算；无需关闭缓存")),
            "mobile_opus_enabled" | "catalog_scrape_enabled" | "cache_enabled" | "metadata_enabled" | "low_performance" => {
                if !v.is_boolean() {
                    return Err(ApiError::bad("设置需要布尔值"));
                }
            }
            _ => return Err(ApiError::bad(format!("未知设置 {k}"))),
        }
    }
    let cache_toggle=v["cache_enabled"].as_bool();
    let cache_disabled=v["cache_enabled"]==false;
    let memory_limit=v["managed_memory_bytes"].as_u64();
    let policy=v.get("provider_request_limits").cloned();let songs=v["song_fetch_concurrency"].as_u64();
    let _artwork_lock=if let Some(limit)=v["artwork_limit_bytes"].as_u64(){let gate=crate::artwork_maintenance::storage_gate(&s.config.data);let guard=gate.lock_owned().await;crate::artwork_maintenance::validate_limit(&s,limit).await?;Some(guard)}else{None};
    let _cache_lock = s.cache_lock.lock().await;
    s.db.run(move|c|{let tx=c.transaction()?;if let Some(limit)=v["cache_limit_bytes"].as_u64(){if let Err(error)=crate::cache_budget::validate(&tx,limit){return Ok(Err(error));}}for(k,v)in v.as_object().unwrap(){tx.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![k,v.to_string()])?;}tx.commit()?;Ok(Ok(()))}).await??;
    if let Some(enabled)=cache_toggle {s.cache_enabled.store(enabled,std::sync::atomic::Ordering::Release);}
    if cache_disabled {crate::media::flush_buffers(&s).await?;}
    if let Some(limit)=memory_limit{s.resources.set_limit(limit);crate::media::flush_buffers(&s).await?;}
    if let Some(policy)=policy{*s.traffic.policy.write().unwrap()=serde_json::from_value(policy).unwrap();}
    if let Some(songs)=songs{s.traffic.song_limit.store(songs as usize,std::sync::atomic::Ordering::Release);}
    Ok(Json(json!({"saved":true})))
}
pub async fn preferences(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    let value =
        s.db.run(move |c| {
            Ok(c.query_row(
                "SELECT value FROM preferences WHERE user_id=?1",
                [a.id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| serde_json::from_str::<Value>(&v).ok())
            .unwrap_or(json!({})))
        })
        .await?;
    Ok(Json(value))
}
pub async fn save_preferences(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<Value>,
) -> ApiResult<Json<Value>> {
    if !v.is_object() || v.to_string().len() > 1024 * 1024 {
        return Err(ApiError::bad("偏好设置格式错误或过大"));
    }
    let result = v.clone();
    s.db.run(move|c|{c.execute("INSERT INTO preferences(user_id,value) VALUES(?1,?2) ON CONFLICT(user_id) DO UPDATE SET value=excluded.value",params![a.id,v.to_string()])?;Ok(())}).await?;
    Ok(Json(result))
}
