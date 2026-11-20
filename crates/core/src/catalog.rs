//! Shared provider resolution and conservative local-first recording selection.
use crate::{error::{ApiError,ApiResult}, models::Track, state::AppState};
use rusqlite::{params,OptionalExtension};
use serde_json::{json,Value};

pub const QUALITIES: &[&str] = &["native","best","HIRES","MASTER","FLAC","MP3_320","MP3_128"];
pub struct ResolvedSource {
    expires: i64,
    bytes:usize,
    value: Value,
    refresh_after: std::time::Instant,
}
pub async fn quality(s:&AppState,source:&str)->String {crate::connectors::quality(s,source).await}
pub async fn resolve_quality(s:&AppState,t:&Track,quality:&str)->ApiResult<Value> {
    resolve_source(s,t,quality,None).await
}
pub async fn refresh_quality(s:&AppState,t:&Track,quality:&str,failed:&Value)->ApiResult<Value> {
    resolve_source(s,t,quality,Some(failed)).await
}
async fn resolve_source(s:&AppState,t:&Track,quality:&str,failed:Option<&Value>)->ApiResult<Value> {
    if !crate::accounts::scope::online_allowed(&t.source){return Err(ApiError::forbidden());}
    crate::plugin::check_enabled(s,&t.source).await?;
    let generation=crate::provider_session::start(s,&t.source).await?;
    let key=format!("{}:{generation}:{quality}",t.id);
    let gate={
        let mut gates=s.source_resolutions.lock().await;
        gates.retain(|_,gate|gate.strong_count()>0);
        let gate=gates.get(&key).and_then(std::sync::Weak::upgrade).unwrap_or_default();
        gates.insert(key.clone(),std::sync::Arc::downgrade(&gate));gate
    };
    let _resolution=gate.lock().await;
    let now=std::time::Instant::now();
    {
        let mut sources=s.resolved_sources.lock().await;
        if let Some(cached)=sources.get_mut(&key) {
            if cached.expires>chrono::Utc::now().timestamp() && failed.is_none_or(|v|v!=&cached.value) {
                let value=cached.value.clone();drop(sources);
                crate::provider_session::validate(s,&t.source,generation).await?;
                return Ok(value);
            }
            if now<cached.refresh_after {
                return Err(ApiError(axum::http::StatusCode::BAD_GATEWAY,"source_refresh_cooldown".into(),"音源地址刷新暂不可用，请稍后重试".into()));
            }
            // Keep the failed value if resolution fails, so subsequent requests respect the cooldown.
            if failed.is_some(){cached.refresh_after=now+std::time::Duration::from_secs(30);}
        }
    }
    let value=s.plugin.call(&t.source,"resolve",json!({"track_id":t.provider_data["cached_from"]["track_id"].as_str().unwrap_or(&t.id),"quality":quality,"provider_data":t.provider_data})).await?;
    crate::provider_session::validate(s,&t.source,generation).await?;
    if value["preview"].as_bool()==Some(true)||value["protected"].as_bool()==Some(true){return Err(ApiError::bad("音源仅返回试听或受保护音频"));}
    let bytes=serde_json::to_vec(&value).map_err(anyhow::Error::from)?.len();
    if bytes>64*1024{return Err(ApiError(axum::http::StatusCode::BAD_GATEWAY,"source_response_too_large".into(),"音源播放地址响应超过64 KiB，请检查插件返回的数据".into()));}
    let seconds=value["expires_in"].as_i64().unwrap_or(120).clamp(1,120);
    let mut sources=s.resolved_sources.lock().await;
    let now=chrono::Utc::now().timestamp();
    sources.retain(|_,cached|cached.expires>now||cached.refresh_after>std::time::Instant::now());
    sources.remove(&key);
    while sources.len()>=256||sources.values().map(|v|v.bytes).sum::<usize>()+bytes>8*1024*1024 {
        if let Some(oldest)=sources.iter().min_by_key(|(_,v)|v.expires).map(|(k,_)|k.clone()){sources.remove(&oldest);}
    }
    sources.insert(key,ResolvedSource{bytes,expires:now+seconds,value:value.clone(),refresh_after:std::time::Instant::now()+if failed.is_some(){std::time::Duration::from_secs(30)}else{std::time::Duration::ZERO}});
    drop(sources);
    if let Some(observed)=crate::source_observation::evidence(&value,"source_header") {
        let (id,source)=(t.id.clone(),t.source.clone());
        let gain = value["qq_replay_gain"].clone();
        let has_gain = source == "qqmusic" && value["recording_id"] == id && crate::loudness::qq_lufs(&gain).is_some();
        // Metadata persistence must not turn an otherwise playable source into a failure.
        match s.db.run(move|c|{
            crate::provider_session::check(c,&source,generation)?;
            crate::source_observation::store(c,&id,&source,&observed)?;
            if !has_gain { return Ok(false); }
            Ok(c.execute("UPDATE tracks SET provider_data=json_set(provider_data,'$.qq_replay_gain',json(?2))
                WHERE id=?1 AND COALESCE(json_extract(provider_data,'$.qq_replay_gain'),'')<>?2",
                params![id,gain.to_string()])? > 0)
        }).await {
            Ok(true) => { tokio::spawn(crate::loudness::publish_qq_gain(s.clone(), t.id.clone())); }
            Ok(false) => {}
            Err(error) => tracing::warn!(%error,"source attributes could not be saved"),
        }
    }
    Ok(value)
}
pub(crate) async fn release_expired(s:&AppState){let now=chrono::Utc::now().timestamp();let instant=std::time::Instant::now();s.resolved_sources.lock().await.retain(|_,v|v.expires>now||v.refresh_after>instant);s.source_resolutions.lock().await.retain(|_,v|v.strong_count()>0);}
pub async fn prefer_local(s:&AppState,tracks:Vec<Track>)->ApiResult<Vec<Track>> {map_local(s,tracks,true).await}
pub async fn prefer_queue(s:&AppState,tracks:Vec<Track>)->ApiResult<Vec<Track>> {map_local(s,tracks,false).await}
async fn map_local(s:&AppState,tracks:Vec<Track>,deduplicate:bool)->ApiResult<Vec<Track>> {
    if crate::accounts::scope::current().is_some_and(|(_,p)|p.is_some()){return Ok(tracks);}
    let (result,upgrades)=s.db.run(move|c|{
        let mut output=vec![];let mut upgrades=vec![];let mut seen=std::collections::HashSet::new();
        for cloud in tracks {
            let mut track=cloud.clone();
            if cloud.source!="local" {
                let exact=c.query_row("SELECT * FROM tracks WHERE source='local' AND available=1 AND recording_id=?1 AND recording_id<>'' ORDER BY json_type(provider_data,'$.cached_from.cache_id') IS NOT NULL,bit_depth DESC,sample_rate DESC,id LIMIT 1",[&cloud.recording_id],Track::from_row).optional()?;
                if let Some(local)=exact {track=local;}
                else if cloud.duration>0. && !cloud.album.is_empty() {
                    let matches=c.prepare("SELECT * FROM tracks WHERE source='local' AND available=1 AND title=?1 COLLATE NOCASE AND artist=?2 COLLATE NOCASE AND album=?3 COLLATE NOCASE AND ABS(duration-?4)<=2 LIMIT 2")?.query_map(params![cloud.title,cloud.artist,cloud.album,cloud.duration],Track::from_row)?.collect::<Result<Vec<_>,_>>()?;
                    if matches.len()==1&&crate::related_catalog::same_edition(&matches[0],&cloud) {track=matches[0].clone();}
                }
            }
            if upgrades.len()<3&&cloud.source!="local"&&track.provider_data["cached_from"]["cache_id"].is_string(){
                let hint=cloud.provider_data["quality_hint"]["quality"].as_str().unwrap_or("");
                let higher=hint=="HIRES"&&(track.sample_rate<=48000||track.bit_depth<=16)||hint=="FLAC"&&!matches!(track.format.as_str(),"flac"|"wav"|"aiff"|"alac");
                let key=format!("upgrade:{}",cloud.id);let recent=c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE kind='cache_fill' AND cursor=?1 AND created_at>unixepoch()-86400)",[key],|r|r.get::<_,bool>(0))?;
                if higher&&!recent{upgrades.push((cloud.id.clone(),cloud.source.clone()));}
            }
            crate::media_assets::apply_stored_cover(c,&mut track)?;
            if let Some(url)=track.cover_url.as_deref(){track.cover_url=Some(crate::media_assets::register(c,url).unwrap_or_else(|_|url.into()));}
            if !deduplicate||seen.insert(track.id.clone()){output.push(track);}
        }
        Ok((output,upgrades))
    }).await?;
    if !crate::cache_policy::enabled(s).await {return Ok(result);}
    for (id,provider)in upgrades{let quality=quality(s,&provider).await;crate::background::enqueue(s,"cache_fill",&format!("upgrade:{id}"),json!({"track_id":id,"quality":quality,"upgrade":true}),-10).await?;}
    Ok(result)
}
pub async fn audio_info(axum::extract::State(s):axum::extract::State<AppState>,a:crate::auth::Auth,axum::extract::Path(id):axum::extract::Path<String>)->ApiResult<axum::Json<Value>> {
    a.require("read")?;let track=crate::library::get_track(&s,id).await?;
    Ok(axum::Json(source_audio_info(&s,&track).await?))
}
pub(crate) async fn source_audio_info(s:&AppState,t:&Track)->ApiResult<Value> {
    let policy=if crate::accounts::scope::online_allowed(&t.source){quality(s,&t.source).await}else{"cached".into()};
    if t.source=="local" {return Ok(json!({"track_id":t.id,"source":"local","format":t.format,"duration":t.duration,"sample_rate":t.sample_rate,"bit_depth":t.bit_depth,"channels":t.channels,"bitrate":t.bitrate,"verified":t.sample_rate>0,"quality":"original","requested_quality":"original"}));}
    if let Some(info)=cached_audio_info(s,t,&policy).await? {return Ok(info);}
    let value=resolve_quality(s,t,&policy).await?;
    let mut info=json!({"track_id":t.id,"source":t.source,"requested_quality":policy});
    for key in ["format","duration","sample_rate","bit_depth","channels","bitrate","verified","quality","quality_label","hires"]{info[key]=value[key].clone();}
    Ok(info)
}
pub(crate) async fn pcm_track(s:&AppState,mut track:Track)->ApiResult<Track> {
    if track.source!="local" {
        let info=source_audio_info(s,&track).await?;
        track.format=info["format"].as_str().unwrap_or("unknown").to_owned();
        track.sample_rate=info["sample_rate"].as_u64().and_then(|n|n.try_into().ok()).unwrap_or(0);
        track.bit_depth=info["bit_depth"].as_u64().and_then(|n|n.try_into().ok()).unwrap_or(0);
        track.channels=info["channels"].as_u64().and_then(|n|n.try_into().ok()).unwrap_or(0);
    }
    if !(8000..=768000).contains(&track.sample_rate)||track.bit_depth==1||matches!(track.format.as_str(),"dsf"|"dff"|"unknown"|"") {
        return Err(ApiError::bad("分组需要已解析的 PCM 音频格式，采样率须为 8–768 kHz"));
    }
    Ok(track)
}

async fn cached_audio_info(s:&AppState,t:&Track,policy:&str)->ApiResult<Option<Value>> {
    let _lock=s.cache_lock.lock().await;
    let id=t.id.clone();let q=policy.to_owned();let suffix=format!("%-q{q}\"");
    let cached=s.db.run(move|c|Ok(c.query_row(crate::media::COMPLETE_CACHE_QUERY,params![id,suffix,q],|r|Ok((r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?))).optional()?)).await?;
    let Some((path,format,size))=cached else{return Ok(None);};
    if tokio::fs::metadata(&path).await.ok().is_none_or(|m|m.len()!=size as u64){return Ok(None);}
    let mut info=json!({"track_id":t.id,"source":t.source,"requested_quality":policy,"quality":policy,"quality_label":"核心完整缓存","duration":t.duration,"format":format,"verified":false});
    let attributes=tokio::task::spawn_blocking(move||{
        use lofty::file::AudioFile;
        let file=lofty::probe::Probe::open(path).ok()?.guess_file_type().ok()?.read().ok()?;
        let p=file.properties();Some((p.sample_rate(),p.bit_depth(),p.channels(),p.audio_bitrate()))
    }).await.map_err(anyhow::Error::from)?;
    if let Some((rate,bits,channels,bitrate))=attributes {
        info["sample_rate"]=json!(rate);info["bit_depth"]=json!(bits);info["channels"]=json!(channels);info["bitrate"]=json!(bitrate.map(|v|v*1000));info["verified"]=json!(rate.is_some());
    }
    Ok(Some(info))
}

#[derive(serde::Deserialize)]
pub struct Search { q: String }

pub async fn search(axum::extract::State(s):axum::extract::State<AppState>,a:crate::auth::Auth,axum::extract::Query(query):axum::extract::Query<Search>)->ApiResult<axum::Json<Value>> {
    a.require("read")?;
    search_impl(&s, &query.q, false).await.map(axum::Json)
}

pub async fn search_local(axum::extract::State(s):axum::extract::State<AppState>,a:crate::auth::Auth,axum::extract::Query(query):axum::extract::Query<Search>)->ApiResult<axum::Json<Value>> {
    a.require("read")?;
    search_impl(&s, &query.q, true).await.map(axum::Json)
}

async fn search_impl(s:&AppState, query:&str, local:bool)->ApiResult<Value> {
    use futures_util::{stream, StreamExt};

    let q = query.trim();
    if q.is_empty() || q.len() > 500 { return Err(ApiError::bad("搜索词长度须为 1–500 字节")); }
    let term = crate::library::query_tokens(q);
    let predicate = crate::accounts::scope::predicate("t");
    let mut tracks = s.db.read(move |c| {
        let source = if local { "AND t.source='local' AND t.path IS NOT NULL" } else { "" };
        let order = if local { "rank" } else { "t.source!='local',rank" };
        Ok(c.prepare(&format!("SELECT t.* FROM track_search CROSS JOIN tracks t ON t.rowid=track_search.rowid WHERE track_search MATCH ?1 AND t.available=1 AND {predicate} {source} ORDER BY {order} LIMIT 100"))?
            .query_map([term], Track::from_row)?.collect::<Result<Vec<_>,_>>()?)
    }).await?;
    if local {
        let mut result = crate::search_catalog::present(q, tracks, None);
        result["source"] = json!("local");
        return Ok(result);
    }

    let providers = crate::connectors::providers(s, "search").await.into_iter()
        .filter(|provider| crate::accounts::scope::online_allowed(provider));
    let mut cloud = stream::iter(providers.map(|provider| {
        async move {
            let response = s.plugin.call(&provider, "search", json!({"query":q,"limit":50})).await;
            (provider, response)
        }
    })).buffered(4);
    let mut notice = None;
    let mut imported = Vec::new();
    while let Some((provider, response)) = cloud.next().await {
        let Ok(mut value) = response else {
            notice = Some("部分音源暂时无法搜索，已保留其他结果");
            continue;
        };
        if let Some(items) = value["items"].as_array_mut() { items.truncate(50); }
        match crate::plugin::import_catalog(s, &provider, &value).await {
            Ok(()) => imported.extend(crate::plugin::validated_tracks(&provider, &value)?.into_iter().map(|t| t.id)),
            Err(_) => notice = Some("部分音源数据暂不可用，已保留核心曲库结果"),
        }
    }
    if !imported.is_empty() {
        let refreshed = s.db.read(move |c| {
            let mut read = c.prepare("SELECT * FROM tracks WHERE id=?1")?;
            let mut tracks = Vec::with_capacity(imported.len());
            for id in imported {
                if let Some(track) = read.query_row([id], Track::from_row).optional()? { tracks.push(track); }
            }
            Ok(tracks)
        }).await?;
        let refreshed_ids: std::collections::HashSet<_> = refreshed.iter().map(|t| t.id.as_str()).collect();
        tracks.retain(|t| !refreshed_ids.contains(t.id.as_str()));
        tracks.extend(refreshed);
    }
    let items=if crate::accounts::scope::current().is_some_and(|(_,p)|p.is_some()){
        let mut seen=std::collections::HashSet::new();
        tracks.retain(|t|seen.insert(t.id.clone()));
        tracks
    }else{prefer_local(s,tracks).await?};
    crate::search_catalog::artwork(s,crate::search_catalog::present(q,items,notice)).await
}
pub async fn background(axum::extract::State(s):axum::extract::State<AppState>,a:crate::auth::Auth,axum::extract::Path(id):axum::extract::Path<String>,axum::extract::Query(q):axum::extract::Query<BackgroundOptions>)->ApiResult<axum::Json<Value>> {
    a.require("read")?;let track=crate::library::get_track(&s,id.clone()).await?;
    let mut value=metadata_for_track_refresh(&s,&track,q.refresh).await?;
    if q.refresh {if let Some(url)=value["track"]["cover_url"].as_str().filter(|v|v.starts_with("https://")) {
        let url=url.to_owned();let source=s.db.run(move|c|crate::media_assets::register(c,&url)).await?;
        if let Some(id)=source.strip_prefix("/api/v1/artwork/"){let _=crate::artwork_recovery::source(&s,id,true).await;}
    }}
    let mut automatic=value["background"].clone();if !automatic.is_object(){automatic=json!({});}if let Some(url)=track.cover_url.as_ref(){automatic["image_url"]=json!(url);}
    crate::catalog_cache::save(&s,"track",&track.id,automatic).await?;
    let profile=crate::catalog_cache::profile(&s,"track",&track.id).await?;value["background"]=profile["profile"].clone();
    Ok(axum::Json(value))
}
#[derive(Default,serde::Deserialize)] pub struct BackgroundOptions {#[serde(default)] refresh:bool}
pub async fn metadata_for_track(s:&AppState,track:&Track)->ApiResult<Value> {
    metadata_for_track_refresh(s,track,false).await
}
pub(crate) async fn metadata_for_track_refresh(s:&AppState,track:&Track,refresh:bool)->ApiResult<Value> {
    for key in ["provider_metadata","qqmusic_metadata"] {let cached=&track.provider_data[key];if !refresh&&cached["matched"]==true{return Ok(cached.clone());}}
    let provider=crate::connectors::metadata_provider(s,&track.source).await?;
    let value=s.plugin.call(&provider,"metadata",json!({"track":track,"track_id":track.id,"refresh":true})).await?;
    if value["source"]!=provider{return Err(ApiError::bad("资料连接器返回的来源不匹配"));}
    crate::metadata::save_track(s,track.id.clone(),value.clone()).await?;
    if value["matched"]==true {s.refresh_queue_metadata().await?;s.emit("library",json!({"reason":"provider_metadata"}));}
    Ok(value)
}
