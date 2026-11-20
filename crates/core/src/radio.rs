//! Account-scoped ranking shared by discovery and queue continuation.
use crate::{auth::Auth, error::{ApiError, ApiResult}, models::Track, state::AppState};
use axum::{extract::{Path, Query, State}, Json};
use rand::{rngs::StdRng, Rng, SeedableRng};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::{HashMap, HashSet}, hash::{DefaultHasher, Hash, Hasher}, time::Duration};

#[path = "personal_recommendations.rs"]
pub(crate) mod personal;

const POOL_LIMIT: usize = 1000;
const CANDIDATES: &str = "t.*,COALESCE(p.play_count,0) plays,COALESCE(p.skip_count,0) skips,COALESCE(p.play_seconds,0) seconds,p.last_played_at last";
const PERSONAL: &str = "LEFT JOIN account_track_stats p ON p.track_id=t.id AND p.user_id=?1";

#[derive(Clone, Default, Deserialize)]
pub struct QueryInput {
    seed_track_id: Option<String>,
    limit: Option<usize>,
    #[serde(default)]
    source: Source,
    #[serde(default)]
    quick: bool,
    #[serde(default)]
    refresh: bool,
}
#[derive(Clone, Copy, Default, Hash, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Source { #[default] All, Local, Cloud }

#[derive(Default, Clone, Deserialize)]
pub(crate) struct QueueContext {
    #[serde(default)] pub(crate) tail: Vec<String>,
    #[serde(default)] pub(crate) recent: Vec<String>,
}
impl QueueContext {
    pub(crate) fn from_zone(z:&crate::models::Zone)->Self {
        Self {
            tail: z.queue.iter().rev().take(12).map(|t|t.id.clone()).collect(),
            recent: z.queue.iter().rev().take(200).map(|t|t.id.clone()).collect(),
        }
    }
}
#[derive(Deserialize)]
pub struct RadioRequest {
    seed_track_id: String,
    #[serde(default)]
    exclude: Vec<String>,
    context: Option<QueueContext>,
}
pub async fn continuation(State(s):State<AppState>,a:Auth,Json(input):Json<RadioRequest>)->ApiResult<Json<Value>> {
    a.require("read")?;
    if input.seed_track_id.len()>256||input.exclude.len()>10_000||input.exclude.iter().any(|id|id.len()>256){
        return Err(ApiError::bad("电台队列或歌曲编号超过上限"));
    }
    let seed=Some(input.seed_track_id);
    let items=if let Some(context)=input.context {
        if context.tail.len()>12||context.recent.len()>200||context.tail.iter().chain(&context.recent).any(|id|id.len()>256) {
            return Err(ApiError::bad("电台上下文超过上限"));
        }
        next_for_queue(&s,Some(a.id),seed,input.exclude,context,10).await?
    }else{next(&s,Some(a.id),seed,input.exclude,10).await?};
    Ok(Json(json!({"items":items})))
}
#[derive(Deserialize)]
pub struct Algorithm {
    advanced: bool,
}
#[derive(Deserialize)]
pub struct Feedback {
    less: bool,
}

pub async fn discovery(State(s): State<AppState>, a: Auth, Query(query): Query<QueryInput>) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let limit = query.limit.unwrap_or(24);
    if !(1..=50).contains(&limit) || query.seed_track_id.as_ref().is_some_and(|id| id.len()>256) {
        return Err(ApiError::bad("推荐数量应为 1–50，种子歌曲编号最多 256 字节"));
    }
    let policy=crate::accounts::policy(&s,&a).await?;
    let mut providers=Vec::new();
    if query.source!=Source::Local {
        for provider in crate::connectors::providers(&s,"resolve").await {
            if policy.as_ref().is_none_or(|p|p.providers.contains(&provider)){
                let epoch=crate::provider_session::start(&s,&provider).await?;
                providers.push((provider,epoch));
            }
        }
        providers.sort_unstable();
    }
    let model=policy.is_none()&&s.db.read(|c|advanced(c)).await?;
    if providers.is_empty()&&!model {
        return Ok(Json(discover_ranked(&s,a.id,query,limit,false).await?));
    }
    let key=DiscoveryKey{user:a.id.clone(),source:query.source,seed:query.seed_track_id.clone(),limit,providers,
        policy:policy.as_ref().map(|p|p.sql("t")),model};
    let (mut update, earlier)={
        let now=std::time::Instant::now();
        let mut cache=s.discovery_snapshots.lock().await;
        cache.retain(|_,entry|now.duration_since(entry.at)<Duration::from_secs(600));
        if let Some(entry)=cache.get(&key) {
            let ready=entry.update.borrow().clone();
            let ttl=if query.source!=Source::Local&&ready.as_ref().and_then(|v|v.as_ref().ok()).is_some_and(|v|v["source"]=="local"||v["items"].as_array().is_some_and(Vec::is_empty)){30}else{120};
            if ready.is_none()||(!query.refresh&&ready.as_ref().is_some_and(Result::is_ok)&&now.duration_since(entry.at)<Duration::from_secs(ttl)){
                if let Some(ready)=ready {return ready.map(Json);}
                (entry.update.clone(),entry.earlier.clone())
            }else{
                let previous=ready.and_then(Result::ok).or_else(||entry.earlier.clone());
                start_discovery(&mut cache,&key,&s,&a,policy,query.clone(),limit,previous,now)
            }
        }else{
            let previous=if query.source==Source::Cloud{
                let mixed=DiscoveryKey{source:Source::All,..key.clone()};
                cache.get(&mixed).and_then(|e|e.update.borrow().clone()).and_then(Result::ok).and_then(|value|{
                    let cloud=value["sections"].as_array()?.iter().find(|section|section["id"]=="cloud")?;
                    Some(json!({"source":"cloud","items":cloud["items"],"sections":[],"reasons":value["reasons"],"algorithm":"local","fallback":false}))
                })
            }else{None};
            start_discovery(&mut cache,&key,&s,&a,policy,query.clone(),limit,previous,now)
        }
    };
    if !query.quick {
        let value=update.wait_for(Option::is_some).await.map_err(|_|ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE,"recommendations_unavailable".into(),"推荐暂不可用，请重试".into()))?;
        return value.as_ref().unwrap().clone().map(Json);
    }
    let mut value=if let Some(earlier)=earlier{earlier}else{
        discover_ranked(&s,a.id,query,limit,false).await?
    };
    value["refreshing"]=json!(true);
    Ok(Json(value))
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(crate) struct DiscoveryKey {
    user:String,source:Source,seed:Option<String>,limit:usize,providers:Vec<(String,i64)>,policy:Option<String>,model:bool,
}
type DiscoveryUpdate=tokio::sync::watch::Receiver<Option<ApiResult<Value>>>;
pub(crate) struct DiscoveryEntry {at:std::time::Instant,update:DiscoveryUpdate,earlier:Option<Value>}
fn start_discovery(cache:&mut HashMap<DiscoveryKey,DiscoveryEntry>,key:&DiscoveryKey,s:&AppState,a:&Auth,
    policy:Option<crate::accounts::Policy>,query:QueryInput,limit:usize,earlier:Option<Value>,at:std::time::Instant)->(DiscoveryUpdate,Option<Value>){
    if cache.len()>=32&&!cache.contains_key(key) {
        if let Some(oldest)=cache.iter().min_by_key(|(_,entry)|entry.at).map(|(key,_)|key.clone()){cache.remove(&oldest);}
    }
    let (send,update)=tokio::sync::watch::channel(None);
    cache.insert(key.clone(),DiscoveryEntry{at,update:update.clone(),earlier:earlier.clone()});
    let (s,a)=(s.clone(),a.clone());
    let providers=key.providers.clone();
    tokio::spawn(async move {
        let user=a.id.clone();
        let result=crate::accounts::scope::REQUEST.scope((a,policy),async {
            let value=crate::traffic::BACKGROUND.scope(true,discover_ranked(&s,user,query,limit,true)).await?;
            for (provider,epoch) in providers {crate::provider_session::validate(&s,&provider,epoch).await?;}
            Ok(value)
        }).await;
        send.send_replace(Some(result));
    });
    (update,earlier)
}

async fn discover_ranked(s:&AppState,user:String,query:QueryInput,limit:usize,complete:bool)->ApiResult<Value> {
    let source=if complete {query.source}else{Source::Local};
    if !complete&&query.source==Source::Cloud {
        return Ok(json!({"source":"cloud","items":[],"sections":[],"algorithm":"local","fallback":false}));
    }
    let cloud=if complete&&source!=Source::Local {cloud_candidates(s,&user,query.seed_track_id.as_deref(),&[],Duration::from_secs(5)).await?}else{vec![]};
    let ranked_limit=if query.source==Source::Local {limit}else{(limit+12).min(50)};
    let mut result = rank(s, Some(user), query.seed_track_id, vec![], ranked_limit, RankOptions{sections:true,source,cloud,skip_model:!complete,..Default::default()}).await?;
    let mut cloud:Vec<_>=result.items.iter().filter(|t|t.source!="local").cloned().collect();
    if let Some(section)=result.sections.iter().find(|section|section["id"]=="cloud") {
        cloud.extend(serde_json::from_value::<Vec<Track>>(section["items"].clone()).unwrap_or_default());
    }
    let available=verified_cloud(s,cloud.iter(),Duration::from_secs(8),12,12).await;
    result.items.retain(|t|t.source=="local"||available.contains(&t.id));
    // Keep a cloud pick in a mixed feed even when many local versions score highly.
    if query.source==Source::All&&limit>=4&&result.items.iter().take(limit).all(|t|t.source=="local"){
        if let Some(index)=result.items.iter().position(|t|t.source!="local"){
            let pick=result.items.remove(index);
            result.items.insert((limit/2).min(result.items.len()),pick);
        }
    }
    result.items.truncate(limit);
    for section in &mut result.sections {
        if let Some(items)=section["items"].as_array_mut() {
            items.retain(|item|item["source"]=="local"||item["id"].as_str().is_some_and(|id|available.contains(id)));
        }
    }
    result.sections.retain(|s|s["items"].as_array().is_some_and(|items|!items.is_empty()));
    let source=match query.source{Source::Cloud=>"cloud",Source::Local=>"local",Source::All=>if available.is_empty(){"local"}else{"mixed"}};
    let mut value = json!({"source":source,"items":result.items,"sections":result.sections,
        "algorithm":result.algorithm,"fallback":result.fallback,"reasons":result.reasons});
    if !complete{return Ok(value);}
    let value = s.db.run(move |c| {
        crate::media_assets::normalize_entries(c, &mut value["items"]);
        for section in value["sections"].as_array_mut().unwrap() {
            crate::media_assets::normalize_entries(c, &mut section["items"]);
        }
        Ok(value)
    }).await?;
    Ok(value)
}
pub async fn settings(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let household = a.role != "external";
    let allowed = a.require("admin").is_ok();
    let advanced = household && s.db.read(|c| advanced(c)).await?;
    let available = household && crate::llm::ranking_available(&s).await;
    Ok(Json(json!({"advanced":advanced,"available":available,"allowed":allowed})))
}
pub async fn set_algorithm(State(s): State<AppState>, a: Auth, Json(value): Json<Algorithm>) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    s.db.run(move |c| {
        c.execute("INSERT INTO settings(key,value) VALUES('recommendation_advanced',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [value.advanced.to_string()])?;
        Ok(())
    }).await?;
    Ok(Json(json!({"advanced":value.advanced,"allowed":true,"available":crate::llm::ranking_available(&s).await})))
}
pub async fn feedback(State(s): State<AppState>, a: Auth, Path(track): Path<String>, Json(value): Json<Feedback>) -> ApiResult<Json<Value>> {
    a.require("control")?;
    if let Some(policy) = crate::accounts::policy(&s, &a).await? {
        crate::accounts::scope::track(&s,&policy,&track).await?;
    }
    if track.len()>256 { return Err(ApiError::bad("歌曲编号过长")); }
    let less = value.less;
    let user=a.id.clone();
    s.db.run(move |c| {
        if less {
            c.execute("INSERT OR IGNORE INTO recommendation_feedback(user_id,track_id) SELECT ?1,id FROM tracks WHERE id=?2",params![a.id,track])?;
        } else {
            c.execute("DELETE FROM recommendation_feedback WHERE user_id=?1 AND track_id=?2",params![a.id,track])?;
        }
        Ok(())
    }).await?;
    s.discovery_snapshots.lock().await.retain(|key,_|key.user!=user);
    Ok(Json(json!({"less":less})))
}
fn advanced(c: &Connection) -> anyhow::Result<bool> {
    Ok(c.query_row("SELECT value='true' FROM settings WHERE key='recommendation_advanced'",[],|r|r.get(0)).optional()?.unwrap_or(false))
}
pub(crate) async fn set_owner(s: &AppState, zone: &str, user: &str, replace: bool) -> ApiResult<()> {
    let (zone,user) = (zone.to_owned(),user.to_owned());
    s.db.run(move |c| {
        c.execute("INSERT INTO recommendation_queue_owner(zone_id,user_id) SELECT ?1,id FROM users WHERE id=?2 ON CONFLICT(zone_id) DO UPDATE SET user_id=excluded.user_id WHERE ?3",
            params![zone,user,replace])?;
        Ok(())
    }).await?;
    Ok(())
}
pub(crate) fn owner_at(c: &Connection, zone: &str) -> anyhow::Result<Option<String>> {
    Ok(c.query_row("SELECT user_id FROM recommendation_queue_owner WHERE zone_id=?1",[zone],|r|r.get(0)).optional()?)
}
pub(crate) async fn owner(s: &AppState, zone: &str) -> ApiResult<Option<String>> {
    if let Some(external) = crate::accounts::scope::zone_user(s,zone).await? { return Ok(Some(external)); }
    let zone=zone.to_owned();
    Ok(s.db.read(move |c| owner_at(c,&zone)).await?)
}

struct ResultSet {
    scores: HashMap<String,f64>,
    items: Vec<Track>,
    reasons: HashMap<String,String>,
    sections: Vec<Value>,
    algorithm: &'static str,
    fallback: bool,
}
pub(crate) async fn next(s: &AppState, user: Option<String>, seed: Option<String>, exclude: Vec<String>, limit: usize) -> ApiResult<Vec<Track>> {
    let mut items=rank(s,user,seed,exclude,(limit+12).min(50),RankOptions::default()).await?.items;
    filter_playable(s,&mut items,limit).await;
    Ok(items)
}

pub(crate) async fn next_for_queue(s:&AppState,user:Option<String>,seed:Option<String>,exclude:Vec<String>,context:QueueContext,limit:usize)->ApiResult<Vec<Track>> {
    let cloud=if let Some(user)=&user {cloud_candidates(s,user,seed.as_deref(),&context.tail,Duration::from_millis(850)).await?}else{vec![]};
    let mut strict=exclude.clone();
    strict.extend(context.recent.iter().cloned());
    let mut items=rank(s,user.clone(),seed.clone(),strict,(limit+12).min(50),
        RankOptions{tail:context.tail.clone(),cloud:cloud.clone(),..Default::default()}).await?.items;
    filter_playable(s,&mut items,limit).await;
    if !items.is_empty() {return Ok(items);}
    items=rank(s,user.clone(),seed.clone(),exclude.clone(),(limit+12).min(50),
        RankOptions{tail:context.tail,cloud:cloud.clone(),replay:context.recent,skip_model:true,..Default::default()}).await?.items;
    filter_playable(s,&mut items,limit).await;
    if !items.is_empty() {return Ok(items);}
    // If there is no other playable recording, the current song can keep the radio alive.
    if let Some(seed)=seed.filter(|id|!exclude.contains(id)) {
        items=rank(s,user,None,exclude,1,RankOptions{only:Some(seed),skip_model:true,..Default::default()}).await?.items;
        filter_playable(s,&mut items,1).await;
    }
    Ok(items)
}

async fn filter_playable(s:&AppState,items:&mut Vec<Track>,limit:usize) {
    let playable=verified_cloud(s,items.iter(),Duration::from_millis(850),12,12).await;
    items.retain(|t|t.source=="local"||playable.contains(&t.id));
    items.truncate(limit);
}

async fn verified_cloud<'a>(s:&AppState,items:impl IntoIterator<Item=&'a Track>,budget:Duration,max_candidates:usize,max_matches:usize)->HashSet<String> {
    use futures_util::{stream,StreamExt};
    let mut seen=HashSet::new();
    let candidates=items.into_iter().filter(|t|t.source!="local"&&seen.insert(t.id.clone())).take(max_candidates).cloned().collect::<Vec<_>>();
    let check=stream::iter(candidates).map(|track|async move {
        let quality=crate::catalog::quality(s,&track.source).await;
        crate::catalog::resolve_quality(s,&track,&quality).await.is_ok_and(|value|value["url"].as_str().is_some_and(|url| !url.trim().is_empty())).then_some(track.id)
    }).buffer_unordered(4);
    // Keep completed checks if a different provider is slow.
    let mut check=std::pin::pin!(check);
    let mut verified=HashSet::new();
    let deadline=tokio::time::sleep(budget);
    tokio::pin!(deadline);
    loop { tokio::select!{
        _=&mut deadline=>break,
        row=check.next()=>match row{Some(Some(id))=>{verified.insert(id);if verified.len()>=max_matches{break;}},Some(None)=>{},None=>break}
    }}
    verified
}

type CloudKey=(String,String,i64,String);
type CloudEntry=(std::time::Instant,std::sync::Arc<tokio::sync::OnceCell<Vec<String>>>);
pub(crate) type CloudCache=tokio::sync::Mutex<HashMap<CloudKey,CloudEntry>>;

async fn cloud_candidates(s:&AppState,user:&str,seed:Option<&str>,tail:&[String],budget:Duration)->ApiResult<Vec<String>> {
    let (owner,seed,tail)=(user.to_owned(),seed.map(str::to_owned),tail.to_vec());
    let external_enabled=crate::accounts::enabled(s).await;
    let discovery=budget>Duration::from_secs(1);
    let (allowed,terms)=s.db.read(move |c| {
        let role=c.query_row("SELECT role FROM users WHERE id=?1",[&owner],|r|r.get::<_,String>(0)).optional()?;
        let policy=match role.as_deref() {
            Some("external") if external_enabled=>{
                let raw=c.query_row("SELECT policy FROM external_accounts WHERE user_id=?1 AND enabled=1",[&owner],|r|r.get::<_,String>(0)).optional()?;
                let Some(raw)=raw else{return Ok((Some(vec![]),vec![]));};
                Some(serde_json::from_str::<crate::accounts::Policy>(&raw)?)
            }
            Some("external")|None=>return Ok((Some(vec![]),vec![])),
            Some(_)=>None,
        };
        let predicate=policy.as_ref().map_or_else(||"1".into(),|p|p.sql("t"));
        let mut terms=Vec::<String>::new();
        let mut add=|artist:String|{
            for artist in artist.split(" / ").map(str::trim) {
                if !artist.is_empty()&&artist!="未知歌手"&&!artist.eq_ignore_ascii_case("unknown artist") {
                    let term=artist.chars().take(64).collect::<String>();
                    if !terms.contains(&term){terms.push(term);}
                }
            }
        };
        let mut read=c.prepare(&format!("SELECT t.artist FROM tracks t WHERE t.id=?1 AND {predicate}"))?;
        for id in tail.iter().take(2).chain(seed.iter()) {
            if let Some(artist)=read.query_row([id],|r|r.get(0)).optional()?{add(artist);}
        }
        for sql in [
            format!("SELECT t.artist FROM account_track_stats p JOIN tracks t ON t.id=p.track_id WHERE p.user_id=?1 AND p.play_count>p.skip_count AND {predicate} ORDER BY p.last_played_at DESC LIMIT 12"),
            format!("SELECT t.artist FROM account_track_stats p JOIN tracks t ON t.id=p.track_id WHERE p.user_id=?1 AND p.play_count>p.skip_count AND {predicate} ORDER BY p.play_seconds DESC LIMIT 12"),
            format!("SELECT t.artist FROM playlists l JOIN json_each(l.tracks) i JOIN tracks t ON t.id=i.value WHERE l.user_id=?1 AND (l.id=?2 OR EXISTS(SELECT 1 FROM playlist_bindings b WHERE b.local_id=l.id AND b.user_id=l.user_id)) AND {predicate} ORDER BY l.id=?2 DESC,i.key DESC LIMIT 12"),
            format!("SELECT t.artist FROM tracks t WHERE t.source='local' AND t.available=1 AND {predicate} ORDER BY t.created_at DESC LIMIT 8"),
        ] {
            let mut stmt=c.prepare(&sql)?;
            let fav=format!("favorites-{}",crate::auth::hash(&owner));
            let params:Vec<&dyn rusqlite::ToSql>=match stmt.parameter_count(){0=>vec![],1=>vec![&owner],_=>vec![&owner,&fav]};
            for artist in stmt.query_map(params.as_slice(),|r|r.get(0))?.take(1) {add(artist?);}
        }
        terms.truncate(if discovery {4}else{2});
        Ok((policy.map(|p|p.providers),terms))
    }).await?;
    search_catalog(s,user,allowed,terms,budget,4).await
}

async fn search_catalog(s:&AppState,user:&str,allowed:Option<Vec<String>>,terms:Vec<String>,budget:Duration,provider_limit:usize)->ApiResult<Vec<String>> {
    if terms.is_empty(){return Ok(vec![]);}
    let (search,resolve)=tokio::join!(crate::connectors::providers(s,"search"),crate::connectors::providers(s,"resolve"));
    let providers=search.into_iter().filter(|p|resolve.contains(p)&&allowed.as_ref().is_none_or(|a|a.contains(p))).take(provider_limit).collect::<Vec<_>>();
    use futures_util::{stream,StreamExt};
    let mut requests=Vec::new();
    for provider in providers {for term in &terms {requests.push((provider.clone(),term.clone()));}}
    let stream=stream::iter(requests).map(|(provider,term)|async move {
        let epoch=crate::provider_session::start(s,&provider).await.ok()?;
        let key=(user.to_owned(),provider.clone(),epoch,term.clone());
        let cell={
            let now=std::time::Instant::now();
            let mut cache=s.radio_cloud.lock().await;
            cache.retain(|_,(stored,_)|now.duration_since(*stored)<Duration::from_secs(60));
            if cache.len()>=128&&!cache.contains_key(&key) {
                if let Some(oldest)=cache.iter().min_by_key(|(_,entry)|entry.0).map(|(key,_)|key.clone()){cache.remove(&oldest);}
            }
            cache.entry(key).or_insert_with(||(now,Default::default())).1.clone()
        };
        let items=cell.get_or_try_init(||async {
            let mut value=tokio::time::timeout(budget,s.plugin.call(&provider,"search",json!({"query":term,"q":term,"limit":12}))).await.map_err(|_|())?.map_err(|_|())?;
            if let Some(items)=value["items"].as_array_mut(){items.truncate(12);}
            let tracks=crate::plugin::validated_tracks(&provider,&value).map_err(|_|())?;
            if !tracks.is_empty(){crate::plugin::import_mode_epoch(s,&provider,&value,true,Some(epoch)).await.map_err(|_|())?;}
            Ok::<_,()>(tracks.into_iter().map(|track|track.id).collect())
        }).await.ok()?;
        crate::provider_session::validate(s,&provider,epoch).await.ok()?;
        Some(items.clone())
    }).buffer_unordered(8);
    Ok(stream.collect::<Vec<_>>().await.into_iter().flatten().flatten().collect())
}

struct Candidate {
    track: Track,
    plays: f64,
    skips: f64,
    seconds: f64,
    last: Option<i64>,
    artist: String,
    album: String,
}
impl Candidate {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {track:Track::from_row(row)?,plays:row.get("plays")?,skips:row.get("skips")?,
            seconds:row.get("seconds")?,last:row.get("last")?,artist:row.get("artist_id")?,album:row.get("album_id")?})
    }
}
fn add(pool: &mut HashMap<String,Candidate>, rows: impl Iterator<Item=rusqlite::Result<Candidate>>) -> anyhow::Result<()> {
    for row in rows {
        if pool.len()>=POOL_LIMIT { break; }
        let row=row?;
        pool.entry(row.track.id.clone()).or_insert(row);
    }
    Ok(())
}
fn feature(track: &Track, name: &str) -> Option<String> {
    track.provider_data[name].as_str().or_else(||track.provider_data["provider_metadata"]["background"][name].as_str())
        .map(str::trim).filter(|value|!value.is_empty()).map(str::to_lowercase)
}
fn year(track: &Track) -> Option<i32> {
    feature(track,"release_date").and_then(|value|value.get(..4)?.parse::<i32>().ok()).filter(|year|(1850..=2200).contains(year))
}

#[derive(Default)]
struct RankOptions { sections:bool,source:Source,cloud:Vec<String>,tail:Vec<String>,replay:Vec<String>,skip_model:bool,only:Option<String>,personal:Option<personal::Intent> }

async fn rank(s:&AppState,user:Option<String>,seed:Option<String>,exclude:Vec<String>,limit:usize,options:RankOptions)->ApiResult<ResultSet> {
    let RankOptions{sections,source,cloud,tail,replay,skip_model,only,personal}=options;
    let providers=if user.is_some()&&source!=Source::Local {crate::connectors::providers(s,"resolve").await} else {vec![]};
    let now=chrono::Utc::now().timestamp();
    let (mut result, prefer_model, preference)=s.db.read(move |c| {
        let mut predicate="1".to_owned();
        let mut model=false;
        if let Some(user)=&user {
            let role=c.query_row("SELECT role FROM users WHERE id=?1",[user],|r|r.get::<_,String>(0)).optional()?;
            match role.as_deref() {
                Some("external") => {
                    let raw=c.query_row("SELECT policy FROM external_accounts WHERE user_id=?1 AND enabled=1",[user],|r|r.get::<_,String>(0)).optional()?;
                    predicate=raw.map(|v|serde_json::from_str::<crate::accounts::Policy>(&v)).transpose()?.map_or_else(||"0".into(),|p|p.sql("t"));
                }
                Some(_) => model=!skip_model&&advanced(c)?,
                None => predicate="0".into(),
            }
        }
        let user=user.as_deref().unwrap_or("");
        let mut rng_seed=DefaultHasher::new();
        (user,now.div_euclid(86400),&seed).hash(&mut rng_seed);
        let mut rng=StdRng::seed_from_u64(rng_seed.finish());
        let favorite_list=format!("favorites-{}",crate::auth::hash(user));
        let mut artists=HashMap::<String,f64>::new();
        let mut genres=HashMap::<String,f64>::new();
        let mut pool=HashMap::new();
        let base=format!("SELECT {CANDIDATES} FROM tracks t {PERSONAL}");
        let permitted=format!("t.available=1 AND {predicate} AND NOT EXISTS(SELECT 1 FROM recommendation_feedback f WHERE f.user_id=?1 AND f.track_id=t.id)");
        let favorite_query=format!("SELECT {CANDIDATES} FROM playlists l JOIN json_each(l.tracks) f JOIN tracks t ON t.id=f.value {PERSONAL} WHERE l.id=?2 AND l.user_id=?1 AND {permitted} ORDER BY f.key DESC LIMIT 160");
        let favorites=c.prepare(&favorite_query)?.query_map(params![user,favorite_list],Candidate::read)?.collect::<rusqlite::Result<Vec<_>>>()?;
        let favored:HashSet<_>=favorites.iter().map(|row|row.track.id.clone()).collect();
        for row in &favorites {
            if !row.artist.is_empty() { *artists.entry(row.artist.clone()).or_default()+=2.; }
            if let Some(genre)=feature(&row.track,"genre") { *genres.entry(genre).or_default()+=2.; }
        }
        add(&mut pool, favorites.into_iter().filter(|row|row.track.source=="local"||providers.contains(&row.track.source)).map(Ok))?;
        let recent=format!("SELECT {CANDIDATES} FROM account_track_stats p JOIN tracks t ON t.id=p.track_id WHERE p.user_id=?1 AND {predicate} ORDER BY p.last_played_at DESC LIMIT 256");
        let mut played_cloud=Vec::new();
        for row in c.prepare(&recent)?.query_map([user],Candidate::read)? {
            let row=row?;
            if sections&&source!=Source::Local&&row.plays>0.&&providers.contains(&row.track.source)&&played_cloud.len()<16 {
                played_cloud.push(row.track.id.clone());
            }
            let age=(now-row.last.unwrap_or(now)).max(0) as f64/86400.;
            let completion=(row.seconds/row.track.duration.max(1.)/(row.plays+row.skips+1.)).clamp(0.,1.);
            let weight=(row.plays.ln_1p()*2.+completion*3.-row.skips.ln_1p()*2.).max(0.)/(1.+age/45.);
            if !row.artist.is_empty() {*artists.entry(row.artist.clone()).or_default()+=weight;}
            if let Some(genre)=feature(&row.track,"genre") {*genres.entry(genre).or_default()+=weight;}
        }
        let mut candidate=c.prepare(&format!("{base} WHERE t.id=?2 AND {permitted}"))?;
        for id in cloud.iter().chain(&played_cloud).chain(&replay).chain(only.iter()) {
            if let Some(row)=candidate.query_row(params![user,id],Candidate::read).optional()? {
                if row.track.source=="local"||providers.contains(&row.track.source){add(&mut pool,std::iter::once(Ok(row)))?;}
            }
        }
        let mut tail_artists=HashMap::<String,f64>::new();
        let mut tail_genres=HashMap::<String,f64>::new();
        for (position,id) in tail.iter().enumerate() {
            if let Some(row)=candidate.query_row(params![user,id],Candidate::read).optional()? {
                let weight=1./(1.+position as f64/4.);
                if !row.artist.is_empty(){*tail_artists.entry(row.artist).or_default()+=weight;}
                if let Some(genre)=feature(&row.track,"genre"){*tail_genres.entry(genre).or_default()+=weight;}
            }
        }
        let seed_row=if let Some(id)=&seed {
            c.query_row(&format!("{base} WHERE t.id=?2 AND {predicate}"),params![user,id],Candidate::read).optional()?
        }else{None};
        if let Some(row)=&seed_row {
            if !row.album.is_empty() {
                let sql=format!("{base} WHERE t.source='local' AND {permitted} AND t.album_id=?2 ORDER BY t.track_number LIMIT 48");
                add(&mut pool,c.prepare(&sql)?.query_map(params![user,row.album],Candidate::read)?)?;
            }
        }
        let mut top:Vec<_>=artists.iter().collect();
        top.sort_by(|a,b|b.1.total_cmp(a.1).then(a.0.cmp(b.0)));
        let seed_artist=seed_row.as_ref().filter(|row|!row.artist.is_empty()).map(|row|&row.artist);
        let sql=format!("{base} WHERE t.source='local' AND {permitted} AND t.artist_id=?2 ORDER BY t.title LIMIT 32");
        let mut stmt=c.prepare(&sql)?;
        for artist in seed_artist.into_iter().chain(tail_artists.keys().take(12)).chain(top.iter().take(8).map(|(id,_)|*id)) {
            add(&mut pool,stmt.query_map(params![user,artist],Candidate::read)?)?;
        }
        let first=(c.query_row("SELECT rowid FROM tracks WHERE source='local' AND available=1 ORDER BY rowid LIMIT 1",[],|r|r.get::<_,i64>(0)).optional()?,
            c.query_row("SELECT rowid FROM tracks WHERE source='local' AND available=1 ORDER BY rowid DESC LIMIT 1",[],|r|r.get::<_,i64>(0)).optional()?);
        if let (Some(first),Some(last))=first {
            let sql=format!("{base} WHERE t.source='local' AND {permitted} AND t.rowid>=?2 ORDER BY t.rowid LIMIT 64");
            let mut sample=c.prepare(&sql)?;
            for offset in std::iter::once(first).chain((0..7).map(|_|rng.gen_range(first..=last))) {
                add(&mut pool,sample.query_map(params![user,offset],Candidate::read)?)?;
            }
        }
        if sections {
            let sql=format!("{base} WHERE t.source='local' AND {permitted} ORDER BY t.created_at DESC,t.id LIMIT 32");
            add(&mut pool,c.prepare(&sql)?.query_map([user],Candidate::read)?)?;
        }

        let mut blocked:HashSet<_>=exclude.into_iter().chain(seed).collect();
        let mut recordings=HashSet::new();
        let mut identity=c.prepare("SELECT recording_id FROM tracks WHERE id=?1")?;
        for id in &blocked {
            if let Some(id)=identity.query_row([id],|r|r.get::<_,String>(0)).optional()?.filter(|id|!id.is_empty()) { recordings.insert(id); }
        }
        let mut disliked=c.prepare("SELECT t.recording_id FROM recommendation_feedback f JOIN tracks t ON t.id=f.track_id WHERE f.user_id=?1 AND t.recording_id<>''")?;
        for id in disliked.query_map([user],|r|r.get::<_,String>(0))? { recordings.insert(id?); }
        let recordings_to_resolve:Vec<_>=pool.values().filter(|r|r.track.source!="local"&&!r.track.recording_id.is_empty()).map(|r|r.track.recording_id.clone()).collect();
        let sql=format!("{base} WHERE t.source='local' AND {permitted} AND t.recording_id=?2 LIMIT 2");
        let mut local_versions=c.prepare(&sql)?;
        for id in recordings_to_resolve {
            add(&mut pool,local_versions.query_map(params![user,id],Candidate::read)?)?;
        }
        let mut pool:Vec<_>=pool.into_values().collect();
        pool.sort_by(|a,b|a.track.id.cmp(&b.track.id));
        let mut available:Vec<_>=pool.into_iter().filter(|row|!blocked.contains(&row.track.id)&&!recordings.contains(&row.track.recording_id)).collect();
        let mut local_versions=HashSet::new();
        for row in &available {
            if row.track.source=="local"&&!row.track.recording_id.is_empty() {local_versions.insert(row.track.recording_id.clone());}
        }
        available.retain(|row| {
            let local=row.track.source=="local";
            if only.as_ref().is_some_and(|id|id!=&row.track.id){return false;}
            match source {
                Source::Local=>local,
                Source::Cloud=>!local,
                Source::All=>local||!local_versions.contains(&row.track.recording_id),
            }
        });
        let mut shelves=vec![];
        if sections {
            let mut new:Vec<_>=available.iter().filter(|r|r.track.source=="local").collect();
            new.sort_by(|a,b|b.track.created_at.cmp(&a.track.created_at));
            let mut old:Vec<_>=available.iter().filter(|r|r.last.is_some_and(|v|v>0&&now-v>86400*30)).collect();
            old.sort_by_key(|r|r.last);
            let mut released:Vec<_>=available.iter().filter_map(|r|feature(&r.track,"release_date").filter(|v|chrono::NaiveDate::parse_from_str(v,"%Y-%m-%d").ok().is_some_and(|date|date<=chrono::Utc::now().date_naive())).map(|date|(date,r))).collect();
            released.sort_by(|a,b|b.0.cmp(&a.0));
            for (id,title,rows) in [("recently-added","新入库",new),("rediscover","久未重听",old),("new-releases","新发行",released.into_iter().map(|(_,r)|r).collect())] {
                if !rows.is_empty(){ shelves.push(shelf(id,title,rows)); }
            }
        }
        let seed_genre=seed_row.as_ref().and_then(|row|feature(&row.track,"genre"));
        let seed_year=seed_row.as_ref().and_then(|row|year(&row.track));
        let mut scored:Vec<_>=available.into_iter().filter_map(|row| {
            let same_artist=seed_row.as_ref().is_some_and(|s|!s.artist.is_empty()&&s.artist==row.artist);
            let same_album=seed_row.as_ref().is_some_and(|s|!s.album.is_empty()&&s.album==row.album);
            let genre=feature(&row.track,"genre");
            let same_genre=genre.is_some()&&genre==seed_genre;
            let near_year=seed_year.zip(year(&row.track)).is_some_and(|(a,b)|(a-b).abs()<=5);
            let artist=artists.get(&row.artist).copied().unwrap_or(0.).ln_1p();
            let style=genre.as_ref().and_then(|g|genres.get(g)).copied().unwrap_or(0.).ln_1p();
            let recent=row.last.filter(|&v|v>0).map(|v|(now-v).max(0));
            let liked=favored.contains(&row.track.id);
            let score=artist*5.+style*2.+if same_artist{11.}else{0.}+if same_album{5.}else{0.}
                +if same_genre{5.}else{0.}+if near_year{2.}else{0.}+if liked{2.}else{0.}
                +tail_artists.get(&row.artist).copied().unwrap_or(0.).ln_1p()*7.
                +genre.as_ref().and_then(|g|tail_genres.get(g)).copied().unwrap_or(0.).ln_1p()*5.
                +if row.track.source=="local"{1.}else{0.}
                +if row.seconds==0.{2.}else{0.}
                -row.skips/(row.plays+row.skips+1.)*13.-recent.map_or(0.,|age|10./(1.+age as f64/86400.))+rng.gen_range(0.0..1.0);
            let reason=if same_artist{"和种子歌曲来自同一歌手"}else if same_genre{"和种子歌曲风格相近"}else if liked{"来自你的收藏"}else if artist>0.{"符合你常听的歌手"}else if style>0.{"符合你常听的风格"}else if tail_artists.contains_key(&row.artist)||genre.as_ref().is_some_and(|g|tail_genres.contains_key(g)){"延续队列中的风格"}else if row.track.source!="local"{"从你常听的音乐延伸探索"}else{"探索你的曲库"};
            let (score,reason)=if let Some(intent)=&personal {
                intent.score(&row,seed_row.as_ref(),score,reason,liked)?
            } else {(score,reason)};
            Some((row,score,reason,recent.is_some_and(|age|age<3600)))
        }).collect();
        let mut reasons=HashMap::new();
        if sections&&source!=Source::Local {
            let mut cloud:Vec<_>=scored.iter().filter(|(row,_,_,_)|row.track.source!="local").collect();
            cloud.sort_by(|a,b|b.1.total_cmp(&a.1));
            if !cloud.is_empty(){
                for (row,_,reason,_) in cloud.iter().take(16){reasons.insert(row.track.id.clone(),(*reason).to_owned());}
                shelves.push(shelf("cloud","云端探索",cloud.into_iter().map(|(row,_,_,_)|row).collect()));
            }
        }
        let mut items=vec![];
        let mut scores=HashMap::new();
        let mut seen_artist=HashMap::<String,usize>::new();
        let requested=if model{limit.max(30)}else{limit};
        while items.len()<requested&&!scored.is_empty() {
            let pick=(0..scored.len()).max_by(|&a,&b| {
                let score=|i:usize|scored[i].1-seen_artist.get(&scored[i].0.track.artist).copied().unwrap_or(0)as f64*7.-if scored[i].3{24.}else{0.};
                score(a).total_cmp(&score(b))
            }).unwrap();
            let (row,score,reason,recent)=scored.swap_remove(pick);
            if !blocked.insert(row.track.id.clone())||!row.track.recording_id.is_empty()&&!recordings.insert(row.track.recording_id.clone()) {continue;}
            *seen_artist.entry(row.track.artist.clone()).or_default()+=1;
            reasons.insert(row.track.id.clone(),reason.to_owned());
            if personal.is_some() { scores.insert(row.track.id.clone(), ((score - if recent {24.}else{0.})*10.).round()/10.); }
            items.push(row.track);
        }
        let preference=seed_row.as_ref().map(|r|format!("以《{}》／{}为种子；",r.track.title.chars().take(80).collect::<String>(),r.track.artist.chars().take(80).collect::<String>())).unwrap_or_default();
        let preference=format!("{preference}优先契合候选列表前部的口味，兼顾多样性和发现新歌。");
        Ok((ResultSet{items,reasons,scores,sections:shelves,algorithm:"local",fallback:false},model,preference))
    }).await?;
    if prefer_model && !result.items.is_empty() {
        let budget=if sections{Duration::from_secs(3)}else{Duration::from_millis(800)};
        match tokio::time::timeout(budget,crate::llm::rerank(&s,&result.items,&preference)).await {
            Ok(Ok(ranked)) => {
                let positions:HashMap<_,_>=ranked.iter().enumerate().map(|(i,(id,_))|(id.clone(),i)).collect();
                result.items.sort_by_key(|track|positions.get(&track.id).copied().unwrap_or(usize::MAX));
                for (id,reason) in ranked {if !reason.is_empty(){result.reasons.insert(id,reason);}}
                result.algorithm="advanced";
            }
            _ => result.fallback=true,
        }
    }
    // Apply diversity after either algorithm, including a model that ranked one artist first.
    for i in 2..result.items.len() {
        if result.items[i].artist==result.items[i-1].artist&&result.items[i].artist==result.items[i-2].artist {
            if let Some(next)=(i+1..result.items.len()).find(|&j|result.items[j].artist!=result.items[i].artist){
                result.items[i..=next].rotate_right(1);
            }
        }
    }
    result.items.truncate(limit);
    Ok(result)
}

fn shelf(id:&str,title:&str,rows:Vec<&Candidate>)->Value {
    let mut seen=HashSet::new();
    let items:Vec<_>=rows.into_iter().filter(|r|seen.insert(if r.track.recording_id.is_empty(){&r.track.id}else{&r.track.recording_id})).take(12).map(|r|&r.track).collect();
    let source=if items.iter().all(|t|t.source=="local"){"local"}else if items.iter().all(|t|t.source!="local"){"cloud"}else{"mixed"};
    json!({"id":id,"title":title,"source":source,"kind":"tracks","items":items})
}
