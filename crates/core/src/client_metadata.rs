//! Optional anonymous client fetching. Core retains the authoritative, persistent cache.
//! Client responses are untrusted input from a library-authorized user, never identity evidence.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::Track,
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

fn mid(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|v| v.is_ascii_alphanumeric())
}
async fn identity(s: &AppState, kind: &str, id: &str) -> ApiResult<String> {
    if !["artist", "album"].contains(&kind) || id.len() > 256 {
        return Err(ApiError::bad("资料身份无效"));
    }
    let (kind, id) = (kind.to_owned(), id.to_owned());
    s.db.read(move|c|{
        let sql=format!("SELECT t.* FROM tracks t WHERE t.source='qqmusic' AND t.id IN ({}) ORDER BY t.id LIMIT 1",crate::catalog_detail::direct_ids(&kind));
        let track=c.query_row(&sql,[&id],Track::from_row).optional()?;
        Ok(track.and_then(|t|if kind=="album"{t.provider_data["album_mid"].as_str().filter(|_|crate::artist_credits::release_key(&t.source,&t.provider_data).as_deref()==Some(&id)).map(str::to_owned)}else{
            let credit=crate::artist_credits::credits(&t.source,&t.artist,&t.provider_data).into_iter().find(|v|v.id==id)?;
            crate::catalog_detail::provider_artist(&t,&id,&credit.name)
        }).filter(|v|mid(v)))
    }).await?.ok_or_else(ApiError::missing)
}
async fn snapshot(s: &AppState, kind: &str, id: &str) -> ApiResult<Value> {
    let (kind, id) = (kind.to_owned(), id.to_owned());
    Ok(s.db
        .read(move |c| {
            Ok(c.query_row(
                "SELECT automatic FROM catalog_profiles WHERE kind=?1 AND entity_id=?2",
                params![kind, id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|v| serde_json::from_str::<Value>(&v).ok())
            .unwrap_or(json!({})))
        })
        .await?)
}
fn revision(v: &Value) -> String {
    crate::auth::hash(&v.to_string())
}
#[derive(Default, Deserialize)]
pub struct Options {
    #[serde(default)]
    pub refresh: bool,
}
pub async fn describe(
    State(s): State<AppState>,
    a: Auth,
    Path((kind, id)): Path<(String, String)>,
    Query(q): Query<Options>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    let identity = identity(&s, &kind, &id).await?;
    let manifest = crate::connectors::manifest(&s, "qqmusic").await?;
    if !manifest["capabilities"]
        .as_array()
        .is_some_and(|v| v.iter().any(|v| v == "public_metadata_v1"))
    {
        return Err(ApiError::missing());
    }
    let epoch = crate::provider_session::start(&s, "qqmusic").await?;
    let current = snapshot(&s, &kind, &id).await?;
    let now = chrono::Utc::now().timestamp();
    let checked = current["_client_qqmusic_checked_at"].as_i64().unwrap_or(0);
    if !q.refresh
        && current["_client_qqmusic_profile"].is_object()
        && current["_client_qqmusic_epoch"] == epoch
        && checked > 0
        && (0..86400).contains(&(now - checked))
    {
        return Ok(Json(json!({"fresh":true})));
    }
    let request = s
        .plugin
        .call(
            "qqmusic",
            "public_metadata",
            json!({"action":"describe","kind":kind,"id":identity}),
        )
        .await?;
    crate::provider_session::validate(&s, "qqmusic", epoch).await?;
    // No credential-bearing headers/cookies may cross this boundary.
    if request["version"] != 1
        || request["url"] != "https://u.y.qq.com/cgi-bin/musicu.fcg"
        || request["method"] != "POST"
        || request.get("headers").is_some()
        || request.get("cookies").is_some()
        || request["json"]["comm"]["uin"] != 0
    {
        return Err(ApiError::bad("匿名资料请求不符合约定"));
    }
    Ok(Json(
        json!({"fresh":false,"request":request,"id":identity,"epoch":epoch,"revision":revision(&current),"expires_at":now+300}),
    ))
}
#[derive(Deserialize)]
pub struct Completion {
    pub id: String,
    pub epoch: i64,
    pub revision: String,
    pub expires_at: i64,
    pub response: Value,
}
pub async fn complete(
    State(s): State<AppState>,
    a: Auth,
    Path((kind, id)): Path<(String, String)>,
    Json(v): Json<Completion>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    let now = chrono::Utc::now().timestamp();
    if v.expires_at < now || v.expires_at > now + 300 || v.response.to_string().len() > 1024 * 1024
    {
        return Err(ApiError::bad("匿名资料响应已过期或过大"));
    }
    if identity(&s, &kind, &id).await? != v.id {
        return Err(ApiError::bad("资料身份已变化"));
    }
    crate::provider_session::validate(&s, "qqmusic", v.epoch).await?;
    let incoming = s
        .plugin
        .call(
            "qqmusic",
            "public_metadata",
            json!({"action":"parse","kind":kind,"id":v.id,"response":v.response}),
        )
        .await?;
    if incoming["id"] != v.id || incoming["source"] != "qqmusic" {
        return Err(ApiError::bad("平台资料身份不一致"));
    }
    let (k, key) = (kind.clone(), id.clone());
    let saved=s.db.run(move|c|{
        let tx=c.transaction()?;crate::provider_session::check(&tx,"qqmusic",v.epoch)?;
        let previous=tx.query_row("SELECT automatic FROM catalog_profiles WHERE kind=?1 AND entity_id=?2",params![k,key],|r|r.get::<_,String>(0)).optional()?.unwrap_or("{}".into());
        let mut current:Value=serde_json::from_str(&previous)?;
        if revision(&current)!=v.revision{return Ok(false);}
        // Retain this provider snapshot separately: ordered Core aggregation must not
        // lose QQ fields when another platform currently has higher priority.
        let mut provider_profile=json!({});
        for &field in crate::catalog_cache::FIELDS {
            if let Some(text)=incoming[field].as_str().filter(|v|!v.is_empty()&&v.len()<=80000) {
                if field!="image_url"||crate::client_artwork::public_url(text){provider_profile[field]=json!(text);}
            }
        }
        current["_client_qqmusic_profile"]=provider_profile;
        for &field in crate::catalog_cache::FIELDS {
            let Some(text)=incoming[field].as_str().filter(|v|!v.is_empty()&&v.len()<=80000) else{continue;};
            if current["_provenance"][field].as_str().is_some_and(|v|v!="qqmusic")&&current[field].as_str().is_some_and(|v|!v.is_empty()){continue;}
            if field=="image_url"&&!crate::client_artwork::public_url(text){continue;}
            current[field]=json!(text);
            if !current["_provenance"].is_object(){current["_provenance"]=json!({});}
            current["_provenance"][field]=json!("qqmusic");
        }
        current["_client_qqmusic_checked_at"]=json!(now);
        current["_client_qqmusic_epoch"]=json!(v.epoch);
        current["_client_qqmusic_basis"]=json!("client_anonymous_response");
        crate::catalog_cache::image(&tx,&mut current);
        if let Some(url)=current["image_url"].as_str(){crate::artwork_recovery::bind(&tx,&k,&key,url)?;}
        tx.execute("INSERT INTO catalog_profiles(kind,entity_id,automatic)VALUES(?1,?2,?3) ON CONFLICT(kind,entity_id)DO UPDATE SET automatic=excluded.automatic,updated_at=unixepoch()",params![k,key,current.to_string()])?;
        if k=="album"{crate::album_artwork::refresh(&tx,&key)?;}
        tx.commit()?;Ok(true)
    }).await?;
    if saved {
        s.emit("library", json!({"reason":"client_metadata"}));
        if kind == "album" {
            s.refresh_queue_metadata().await?;
        }
    }
    Ok(Json(
        json!({"saved":saved,"cache":crate::catalog_cache::profile(&s,&kind,&id).await?}),
    ))
}
