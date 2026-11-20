//! Artist discography uses declared provider identities, never title-only album matching.
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

#[derive(Default, Deserialize)]
pub struct Page {
    pub sort: Option<String>,
    pub local_sort: Option<String>,
    pub page: Option<u32>,
    pub provider: Option<String>,
    #[serde(default)]
    pub refresh: bool,
}
fn invalid() -> ApiError {
    ApiError::bad("专辑目录与已确认的平台身份不一致")
}
fn release(id: &str) -> Option<(&str, &str)> {
    let (provider, key) = id.strip_prefix("release:")?.split_once(':')?;
    (crate::connectors::valid_id(provider)
        && !key.is_empty()
        && key.len() <= 128
        && !key.chars().any(char::is_control))
    .then_some((provider, key))
}
fn next_valid(v: &Value, page: u32) -> bool {
    v["next_page"].is_null() || v["next_page"].as_u64() == Some(u64::from(page) + 1)
}
fn card(t: &Track) -> Value {
    let id = crate::artist_credits::release_key(&t.source, &t.provider_data).unwrap_or_else(|| {
        crate::auth::hash(&format!("{}\0{}\0{}", t.source, t.album_artist, t.album))
    });
    let cover = t.cover_url.clone();
    json!({"id":id,"title":t.album,"artist":t.album_artist,"source":t.source,"cover_url":cover,"release_date":crate::catalog_order::release_date(t)})
}
pub async fn list(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Query(q): Query<Page>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let sort=q.sort.as_deref().unwrap_or("default");
    if !["default","hot","release_new","release_old"].contains(&sort){return Err(ApiError::bad("专辑平台排序无效"));}
    let local_sort=q.local_sort.as_deref().unwrap_or("added");
    if !["added","title","title_desc","release_new","release_old","plays"].contains(&local_sort){return Err(ApiError::bad("本地专辑排序无效"));}
    let order=if local_sort=="title"{"MIN(t.album) COLLATE NOCASE,t.id".into()}else if local_sort=="title_desc"{"MIN(t.album) COLLATE NOCASE DESC,t.id".into()}else{crate::library_query::order(Some(local_sort),true,"MAX(t.created_at) DESC")?};
    let page = q.page.unwrap_or(1);
    if id.len() > 256 || !(1..=100000).contains(&page) {
        return Err(ApiError::bad("歌手或专辑页码无效"));
    }
    let local_page = if q.provider.as_deref() == Some("local") { page } else { 1 };
    let permission=crate::accounts::scope::predicate("t");let preferred=if a.role=="external"{"1"}else{crate::library::PREFERRED_COPY};let order=crate::accounts::scope::stats_sql(&order);
    let key = id.clone();
    let (name,anchors,local)=s.db.read(move|c|{
        let seed=c.query_row(&format!("SELECT t.* FROM tracks t JOIN track_artists a ON a.track_id=t.id WHERE a.artist_id=?1 AND {permission} ORDER BY t.source='local',t.id LIMIT 1"),[&key],Track::from_row).optional()?;
        let Some(seed)=seed else{return Ok(None)};
        let name:String=c.query_row("SELECT name FROM track_artists WHERE track_id=?1 AND artist_id=?2",params![seed.id,key],|r|r.get(0))?;
        let condition=format!("{} AND {} AND {permission}",crate::catalog_detail::membership("artist"),crate::library::COLLECTION_VISIBILITY);
        let ids=c.prepare(&format!("SELECT MIN(t.id) FROM tracks t WHERE {condition} GROUP BY t.source ORDER BY t.source LIMIT 16"))?.query_map(params![key,seed.id,name],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let anchors=ids.into_iter().map(|id|c.query_row("SELECT * FROM tracks WHERE id=?1",[id],Track::from_row)).collect::<rusqlite::Result<Vec<_>>>()?;
        let condition=format!("{condition} AND t.available=1 AND {}",preferred);
        let grouping="COALESCE(json_extract(t.provider_data,'$.cached_from.source'),t.source),COALESCE(NULLIF(json_extract(t.provider_data,'$.release_id'),''),NULLIF(json_extract(t.provider_data,'$.album_mid'),''),t.album_id)";
        let total:i64=c.query_row(&format!("SELECT COUNT(*) FROM (SELECT 1 FROM tracks t WHERE {condition} AND {} GROUP BY {grouping})",crate::library_query::KNOWN_ALBUM),params![key,seed.id,name],|r|r.get(0))?;
        let tracks=c.prepare(&format!("SELECT t.* FROM tracks t WHERE {condition} AND {} GROUP BY {grouping} ORDER BY {order} LIMIT 30 OFFSET ?4",crate::library_query::KNOWN_ALBUM))?.query_map(params![key,seed.id,name,(local_page-1)*30],Track::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let items=tracks.iter().map(|t|card(t)).collect::<Vec<_>>();
        Ok(Some((name,anchors,json!({"items":items,"total":total,"page":local_page,"next_page":if i64::from(local_page)*30<total{Some(local_page+1)}else{None}}))))
    }).await?.ok_or_else(ApiError::missing)?;
    let mut platforms = Vec::new();
    for provider in crate::connectors::ordered_metadata_providers(&s)
        .await
        .into_iter()
        .take(16)
    {
        if !crate::accounts::scope::online_allowed(&provider){continue;}
        let Some(seed) = anchors.iter().find(|t| t.source == provider) else {
            continue;
        };
        let Some(artist) = crate::catalog_detail::provider_artist(seed, &id, &name) else {
            continue;
        };
        let manifest = crate::connectors::manifest(&s, &provider).await?;
        if !manifest["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|v| v == "artist_albums_v1"))
        {
            continue;
        }
        let p = if q
            .provider
            .as_deref()
            .is_none_or(|v| v.is_empty() || v == provider)
        {
            page
        } else {
            1
        };
        if sort!="default" && !manifest["capabilities"].as_array().is_some_and(|v|v.iter().any(|v|v=="catalog_order_v1")){platforms.push(json!({"source":provider,"items":[],"notice":"此插件尚未提供平台排序能力"}));continue;}
        let cache = if sort=="default"{format!("artist-albums:{provider}:{artist}:{}",seed.id)}else{format!("artist-albums:{provider}:{artist}:{}:{sort}",seed.id)};
        let gate = crate::media_assets::gate(&s, &cache).await;
        let _guard = gate.lock().await;
        let result=async {
            let cached=if q.refresh{None}else{crate::catalog_cache::load_page(&s,cache.clone(),p).await?};
            let fresh=cached.is_some()&&crate::catalog_cache::page_fresh(&s,cache.clone(),p).await?;
            let epoch=crate::provider_session::start(&s,&provider).await?;
            let (mut value,updated)=if fresh{(cached.clone().unwrap(),false)}else{match s.plugin.call(&provider,"artist_albums",json!({"track":seed,"artist_id":artist,"page":p,"refresh":q.refresh,"sort":sort})).await {Ok(v)=>(v,true),Err(e)=>if let Some(mut v)=cached{v["notice"]=json!("更新暂不可用，显示已保存目录");(v,false)}else{return Err(e);}}};
            crate::provider_session::validate(&s,&provider,epoch).await?;
            if value["source"]!=provider || value["artist_id"]!=artist || value["anchor"]["id"]!=seed.id || value["anchor"]["source"]!=provider || value["page"]!=p || !next_valid(&value,p) {return Err(invalid());}
            let items=value["items"].as_array().filter(|v|v.len()<=30).ok_or_else(invalid)?;
            for item in items {
                let key=item["release_id"].as_str().filter(|key|!key.is_empty()&&key.len()<=128&&!key.chars().any(char::is_control)).ok_or_else(invalid)?;
                if item["source"]!=provider || item["title"].as_str().is_none_or(|s|s.is_empty()||s.len()>4096){return Err(invalid());}
                let album_id=format!("release:{provider}:{key}");
                crate::catalog_cache::save(&s,"album",&album_id,json!({"source":provider,"release_id":key,"name":item["title"],"image_url":item["cover_url"],"release_date":item["release_date"],"artist_id":id})).await?;
            }
            if updated {crate::catalog_cache::save_page(&s,cache,p,&value,q.refresh).await?;}
            s.db.run(move|c|{
                for item in value["items"].as_array_mut().unwrap() {
                    let id=format!("release:{}:{}",item["source"].as_str().unwrap(),item["release_id"].as_str().unwrap());
                    let manual:Option<String>=c.query_row("SELECT json_extract(manual,'$.image_url') FROM catalog_profiles WHERE kind='album' AND entity_id=?1",[&id],|r|r.get(0)).optional()?.flatten();
                    if let Some(url)=manual {item["cover_url"]=json!(url);}
                    item["id"]=json!(id);
                }
                crate::media_assets::normalize_entries(c,&mut value["items"]);Ok(value)
            }).await.map_err(ApiError::from)
        }.await;
        match result {
            Ok(mut value) => {value["sorts"]=if manifest["capabilities"].as_array().is_some_and(|v|v.iter().any(|v|v=="catalog_order_v1")){json!(["default","hot","release_new","release_old"])}else{json!(["default"])};platforms.push(value);},
            Err(error) => platforms.push(json!({"source":provider,"items":[],"notice":error.2})),
        }
    }
    Ok(Json(
        json!({"artist_id":id,"name":name,"platforms":platforms,"local":local}),
    ))
}

/// A discography card is a known release, but not a fabricated song. Fetch its real
/// tracks only when opened, then reuse the regular detail/identity/navigation path.
pub async fn ensure_release(s: &AppState, id: &str, page: u32, refresh: bool) -> ApiResult<()> {
    let Some((provider, key)) = release(id) else {
        return Ok(());
    };
    let target = id.to_owned();
    let exists =
        s.db.read(move |c| {
            Ok(c.query_row(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM tracks t WHERE t.id IN ({}))",
                    crate::catalog_detail::direct_ids("album")
                ),
                [target],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await?;
    if exists {
        return Ok(());
    }
    let saved = crate::catalog_cache::profile(s, "album", id).await?;
    if saved["profile"]["source"] != provider
        || saved["profile"]["release_id"] != key
        || saved["profile"]["artist_id"].as_str().is_none()
    {
        return Ok(());
    }
    let manifest = crate::connectors::manifest(s, provider).await?;
    if !manifest["capabilities"]
        .as_array()
        .is_some_and(|v| v.iter().any(|v| v == "album_catalog_v1"))
    {
        return Ok(());
    }
    let cache = format!("release-catalog:{provider}:{key}");
    let gate = crate::media_assets::gate(s, &cache).await;
    let _guard = gate.lock().await;
    let cached = if refresh {
        None
    } else {
        crate::catalog_cache::load_page(s, cache.clone(), page).await?
    };
    let fresh = cached.is_some() && crate::catalog_cache::page_fresh(s,cache.clone(),page).await?;
    let epoch = crate::provider_session::start(s,provider).await?;
    let (value,updated) = if fresh {
        (cached.clone().unwrap(),false)
    } else {
        match s.plugin
            .call(
                provider,
                "album_catalog",
                json!({"release_id":key,"page":page,"refresh":refresh}),
            )
            .await {Ok(v)=>(v,true),Err(e)=>if let Some(v)=cached{(v,false)}else{return Err(e);}}
    };
    crate::provider_session::validate(s,provider,epoch).await?;
    if value["source"] != provider
        || value["release_id"] != key
        || value["page"] != page
        || !next_valid(&value, page)
    {
        return Err(invalid());
    }
    let tracks = crate::plugin::validated_tracks(provider, &value)?;
    if tracks.len() > 30
        || tracks.iter().any(|t| {
            t.provider_data["release_id"]
                .as_str()
                .or_else(|| t.provider_data["album_mid"].as_str())
                != Some(key)
        })
    {
        return Err(invalid());
    }
    crate::plugin::import_mode_epoch(s, provider, &value,true,Some(epoch)).await?;
    if updated {crate::catalog_cache::save_page(s, cache, page, &value, refresh).await?;}
    Ok(())
}

pub async fn empty_release(s: &AppState, id: &str) -> ApiResult<Option<Value>> {
    let Some((provider, key)) = release(id) else {
        return Ok(None);
    };
    let saved = crate::catalog_cache::profile(s, "album", id).await?;
    if saved["profile"]["source"] != provider || saved["profile"]["release_id"] != key {
        return Ok(None);
    }
    Ok(Some(
        json!({"id":id,"kind":"album","name":saved["profile"]["name"],"profile":saved["profile"],"profile_revision":saved["revision"],"manual_fields":saved["manual"],"provenance":{},"local":{"items":[],"total":0,"page":1},"platforms":[{"source":provider,"items":[],"notice":"平台暂未提供此专辑的曲目"}]}),
    ))
}
