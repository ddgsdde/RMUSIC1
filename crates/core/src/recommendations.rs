//! Bounded local recommendations and explicitly reported provider fallback.
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
use rand::{rngs::StdRng, Rng, SeedableRng};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const MAX_POOL: usize = 1_000;
const CANDIDATE_COLUMNS: &str = "t.*,COALESCE(p.play_count,0) AS rec_plays,COALESCE(p.skip_count,0) AS rec_skips,COALESCE(p.play_seconds,0) AS rec_seconds,p.last_played_at AS rec_last";

#[derive(Debug, Serialize)]
pub struct Selection {
    pub items: Vec<Track>,
    pub source: String,
    pub fallback: bool,
    pub message: Option<String>,
    pub sections: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
pub struct RecommendationQuery {
    pub source: Option<String>,
    pub seed_track_id: Option<String>,
    pub limit: Option<usize>,
    #[serde(default = "yes")]
    pub fallback_local: bool,
    #[serde(default)]
    pub refresh: bool,
}
fn yes() -> bool {
    true
}

pub async fn handler(
    State(s): State<AppState>,
    auth: Auth,
    Query(q): Query<RecommendationQuery>,
) -> ApiResult<Json<Selection>> {
    auth.require("read")?;
    if auth.role=="external"{return Ok(Json(crate::accounts::catalog::select(&s,&auth.id,q.seed_track_id,vec![],q.limit.unwrap_or(20)).await?));}
    let mut selection=select_request(&s,q.source.as_deref().unwrap_or("mixed"),q.seed_track_id,vec![],q.limit.unwrap_or(20),q.fallback_local,q.refresh).await?;
    selection.items=crate::music_versions::order(&s,auth.id,selection.items).await?;
    let albums=s.db.read(|c| {
        let ids=c.prepare("SELECT MIN(t.id) FROM tracks t LEFT JOIN track_stats s ON s.track_id=t.id WHERE t.source='local' AND t.available=1 AND t.album<>'' GROUP BY t.album_id ORDER BY COALESCE(MAX(s.last_played_at),0),MAX(t.created_at) DESC,t.album_id LIMIT 12")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id|Ok(c.query_row("SELECT * FROM tracks WHERE id=?1",[id],Track::from_row)?)).collect::<anyhow::Result<Vec<_>>>()
    }).await?;
    if !albums.is_empty(){selection.sections.push(json!({"id":"local-albums","kind":"albums","title":"从曲库挑张专辑","source":"local","items":albums}));}

    let selection = s.db.run(move |c| {
        for section in &mut selection.sections {
            for key in ["items", "playlists"] { crate::media_assets::normalize_entries(c, &mut section[key]); }
        }
        Ok(selection)
    }).await?;
    Ok(Json(selection))
}

#[derive(Default)]
struct Context {
    excluded: HashSet<String>,
    recordings: HashSet<String>,
    preferred: HashMap<String, f64>,
    seed: Option<Track>,
}
#[derive(Clone)]
struct Candidate {
    track: Track,
    plays: f64,
    skips: f64,
    seconds: f64,
    last: Option<i64>,
}
impl Candidate {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            track: Track::from_row(row)?,
            plays: row.get::<_, f64>("rec_plays")?.max(0.),
            skips: row.get::<_, f64>("rec_skips")?.max(0.),
            seconds: row.get::<_, f64>("rec_seconds")?.max(0.),
            last: row.get("rec_last")?,
        })
    }
}
fn exclude_recordings(c: &Connection, context: &mut Context) -> anyhow::Result<()> {
    let ids: Vec<_> = context.excluded.iter().cloned().collect();
    for batch in ids.chunks(400) {
        let marks = vec!["?"; batch.len()].join(",");
        let mut statement = c.prepare(&format!(
            "SELECT recording_id FROM tracks WHERE id IN ({marks})"
        ))?;
        for recording in statement.query_map(params_from_iter(batch), |r| r.get::<_, String>(0))? {
            let recording = recording?;
            if !recording.is_empty() {
                context.recordings.insert(recording);
            }
        }
    }
    Ok(())
}
fn context(
    c: &Connection,
    seed: Option<String>,
    exclude: Vec<String>,
    now: i64,
) -> anyhow::Result<Context> {
    let mut result = Context {
        excluded: exclude.into_iter().collect(),
        ..Default::default()
    };
    if let Some(id) = seed {
        result.excluded.insert(id.clone());
        result.seed = c
            .query_row("SELECT * FROM tracks WHERE id=?1", [id], Track::from_row)
            .optional()?;
    }
    // Both queries use time indexes; only a bounded recent history contributes to preferences.
    let mut recent = c.prepare("SELECT track_id,play_seconds,started_at FROM play_sessions ORDER BY started_at DESC LIMIT 64")?;
    let mut excluded_recent = 0;
    for row in recent.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, f64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })? {
        let (id, seconds, started) = row?;
        if seconds > 0.
            && started >= (now - 86_400) * 1000
            && excluded_recent < 20
            && result.excluded.insert(id)
        {
            excluded_recent += 1;
        }
    }
    let mut statistics = c.prepare("SELECT p.track_id,p.play_count,p.skip_count,p.play_seconds,p.last_played_at,t.artist_id FROM track_stats p JOIN tracks t ON t.id=p.track_id ORDER BY p.last_played_at DESC LIMIT 512")?;
    for row in statistics.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, f64>(1)?,
            r.get::<_, f64>(2)?,
            r.get::<_, f64>(3)?,
            r.get::<_, Option<i64>>(4)?,
            r.get::<_, String>(5)?,
        ))
    })? {
        let (id, plays, skips, seconds, last, artist) = row?;
        if last.is_some_and(|last| last >= now - 3_600) {
            result.excluded.insert(id);
        }
        let age_days = (now - last.unwrap_or(now)).max(0) as f64 / 86_400.;
        let preference = ((plays.max(0.) + 1.).ln() + seconds.max(0.).sqrt() / 40.
            - skips.max(0.) * 0.5)
            .max(0.)
            / (1. + age_days / 30.);
        if !artist.is_empty() && preference.is_finite() {
            *result.preferred.entry(artist).or_default() += preference;
        }
    }
    exclude_recordings(c, &mut result)?;
    Ok(result)
}
fn insert_candidates(
    pool: &mut HashMap<String, Candidate>,
    rows: impl IntoIterator<Item = rusqlite::Result<Candidate>>,
) -> anyhow::Result<()> {
    for row in rows {
        let row = row?;
        if pool.len() >= MAX_POOL {
            break;
        }
        pool.entry(row.track.id.clone()).or_insert(row);
    }
    Ok(())
}
fn local_pool(
    c: &Connection,
    context: &Context,
    rng: &mut StdRng,
) -> anyhow::Result<Vec<Candidate>> {
    let mut pool = HashMap::new();
    let mut artists: Vec<_> = context.preferred.iter().collect();
    artists.sort_by(|a, b| b.1.total_cmp(a.1).then_with(||a.0.cmp(b.0)));
    let mut artist_ids: Vec<(String, usize)> = artists
        .into_iter()
        .take(8)
        .map(|(id, _)| (id.clone(), 32))
        .collect();
    if let Some(seed) = &context.seed {
        artist_ids.insert(0, (crate::auth::hash(&seed.artist), 96));
        let album = crate::auth::hash(&format!("local\0{}\0{}", seed.album_artist, seed.album));
        let sql = format!("SELECT {CANDIDATE_COLUMNS} FROM tracks t LEFT JOIN track_stats p ON p.track_id=t.id WHERE t.album_id=?1 AND t.source='local' AND t.available=1 ORDER BY t.track_number LIMIT 96");
        insert_candidates(
            &mut pool,
            c.prepare(&sql)?.query_map([album], Candidate::from_row)?,
        )?;
    }
    for (artist, limit) in artist_ids {
        let sql = format!("SELECT {CANDIDATE_COLUMNS} FROM tracks t LEFT JOIN track_stats p ON p.track_id=t.id WHERE t.artist_id=?1 AND t.source='local' AND t.available=1 ORDER BY t.title LIMIT ?2");
        insert_candidates(
            &mut pool,
            c.prepare(&sql)?
                .query_map(params![artist, limit as i64], Candidate::from_row)?,
        )?;
    }
    let first = c.query_row("SELECT rowid FROM tracks INDEXED BY tracks_source WHERE source='local' AND available=1 ORDER BY rowid LIMIT 1",[],|r|r.get::<_,i64>(0)).optional()?;
    let last = c.query_row("SELECT rowid FROM tracks INDEXED BY tracks_source WHERE source='local' AND available=1 ORDER BY rowid DESC LIMIT 1",[],|r|r.get::<_,i64>(0)).optional()?;
    if let (Some(first), Some(last)) = (first, last) {
        let sql=format!("SELECT {CANDIDATE_COLUMNS} FROM tracks t INDEXED BY tracks_source LEFT JOIN track_stats p ON p.track_id=t.id WHERE t.source='local' AND t.available=1 AND t.rowid>=?1 ORDER BY t.rowid LIMIT 64");
        let mut statement = c.prepare(&sql)?;
        for _ in 0..8 {
            insert_candidates(
                &mut pool,
                statement.query_map([rng.gen_range(first..=last)], Candidate::from_row)?,
            )?;
        }
        if pool.len() < 64 {
            insert_candidates(
                &mut pool,
                statement.query_map([first], Candidate::from_row)?,
            )?;
        }
    }
    Ok(pool.into_values().collect())
}
fn eligible(track: &Track, context: &Context) -> bool {
    track.available
        && !context.excluded.contains(&track.id)
        && (track.recording_id.is_empty() || !context.recordings.contains(&track.recording_id))
}
fn local_selection(
    mut candidates: Vec<Candidate>,
    context: &Context,
    now: i64,
    limit: usize,
    rng: &mut StdRng,
) -> Vec<Track> {
    candidates.sort_by(|a,b|a.track.id.cmp(&b.track.id));
    let mut scored: Vec<_> = candidates
        .into_iter()
        .filter(|c| eligible(&c.track, context))
        .map(|candidate| {
            let artist = crate::auth::hash(&candidate.track.artist);
            let mut score = if candidate.seconds == 0. && candidate.plays == 0. {
                3.
            } else {
                0.
            };
            score += (context.preferred.get(&artist).copied().unwrap_or(0.) + 1.).ln() * 6.;
            if let Some(seed) = &context.seed {
                if candidate.track.artist == seed.artist {
                    score += 12.;
                }
                if candidate.track.album == seed.album
                    && candidate.track.album_artist == seed.album_artist
                {
                    score += 4.;
                }
            }
            score -= (candidate.plays + 1.).ln();
            score -= candidate.skips / (candidate.plays + candidate.skips + 1.) * 14.
                + (candidate.skips + 1.).ln() * 2.;
            if let Some(last) = candidate.last {
                let days = (now - last).max(0) as f64 / 86_400.;
                score -= 8. / (1. + days);
            }
            score += rng.gen_range(0.0..1.0);
            (candidate.track, score)
        })
        .collect();
    let mut result = vec![];
    let mut artists: HashMap<String, usize> = HashMap::new();
    let mut recordings = context.recordings.clone();
    while result.len() < limit && !scored.is_empty() {
        let best = (0..scored.len())
            .max_by(|a, b| {
                let score = |i: usize| {
                    scored[i].1 - artists.get(&scored[i].0.artist).copied().unwrap_or(0) as f64 * 6.
                };
                score(*a).total_cmp(&score(*b))
            })
            .unwrap();
        let (track, _) = scored.swap_remove(best);
        if !track.recording_id.is_empty() && !recordings.insert(track.recording_id.clone()) {
            continue;
        }
        *artists.entry(track.artist.clone()).or_default() += 1;
        result.push(track);
    }
    result
}

#[derive(Deserialize)]
pub struct PlatformQuery {
    page: Option<u32>,
    #[serde(default)]
    refresh: bool,
}
pub async fn platform(
    State(s):State<AppState>,a:Auth,Path(provider):Path<String>,Query(query):Query<PlatformQuery>,
)->ApiResult<Json<Value>> {
    a.require("read")?;
    crate::plugin::check_enabled(&s,&provider).await?;
    let manifest=crate::connectors::manifest(&s,&provider).await?;
    let caps=manifest["capabilities"].as_array().ok_or_else(ApiError::missing)?;
    let public=a.role=="external";
    if public {
        let policy=crate::accounts::policy(&s,&a).await?.ok_or_else(ApiError::forbidden)?;
        if !policy.providers.contains(&provider)||!caps.iter().any(|cap|cap=="public_recommendations_v1") {return Err(ApiError::forbidden());}
    } else if !caps.iter().any(|cap|cap=="recommendations"){return Err(ApiError::missing());}
    let page=query.page.unwrap_or(1);
    if !(1..=1000).contains(&page){return Err(ApiError::bad("推荐页码无效"));}
    let generation=crate::provider_session::start(&s,&provider).await?;
    if page==1&&!public&&daily_enabled(&provider,caps){
        let version=manifest["version"].as_str().unwrap_or("").to_owned();
        if let Some((day,value))=daily_snapshot(&s,&provider,&version,generation).await?{
            if query.refresh&&day!=local_day(){
                tokio::spawn(async move{let _=refresh_daily(&s,&provider,&version,generation,true).await;});
            }
            return Ok(Json(value));
        }
        return Ok(Json(refresh_daily(&s,&provider,&version,generation,false).await?));
    }
    Ok(Json(fetch_platform(&s,&provider,public,page,query.refresh,generation,None).await?))
}

fn daily_enabled(provider:&str,caps:&[Value])->bool {
    // Older installed QQ manifests already implement a daily upstream snapshot.
    provider=="qqmusic"||caps.iter().any(|cap|cap=="daily_recommendations_v1")
}
fn local_day()->String{chrono::Local::now().format("%Y-%m-%d").to_string()}

async fn daily_snapshot(s:&AppState,provider:&str,version:&str,generation:i64)->ApiResult<Option<(String,Value)>>{
    let (provider,version)=(provider.to_owned(),version.to_owned());
    Ok(s.db.read(move|c|{
        crate::provider_session::check(c,&provider,generation)?;
        let context=c.query_row("SELECT context FROM provider_accounts WHERE provider=?1",[&provider],|r|r.get::<_,String>(0)).optional()?;
        if context.as_deref()==Some("switching"){return Err(crate::provider_session::Changed.into());}
        let row=c.query_row("SELECT day,data FROM provider_daily_recommendations WHERE provider=?1 AND generation=?2 AND version=?3",
            params![provider,generation,version],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        row.map(|(day,data)|Ok((day,serde_json::from_str(&data)?))).transpose()
    }).await?)
}

async fn refresh_daily(s:&AppState,provider:&str,version:&str,generation:i64,background:bool)->ApiResult<Value>{
    let lock=s.daily_recommendation_locks.lock().await.entry(provider.into()).or_default().clone();
    let _guard=lock.lock().await;
    let day=local_day();
    if let Some((saved_day,value))=daily_snapshot(s,provider,version,generation).await?{
        if saved_day==day||!background{return Ok(value);}
    }
    let fetch=fetch_platform(s,provider,false,1,true,generation,Some((version.to_owned(),day)));
    if background{crate::traffic::BACKGROUND.scope(true,fetch).await}else{fetch.await}
}

pub fn spawn_daily(s:AppState){
    tokio::spawn(async move{
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let mut tick=tokio::time::interval(std::time::Duration::from_secs(1800));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop{
            tick.tick().await;
            crate::radio::personal::refresh_due(&s).await;
            refresh_daily_due(&s).await;
        }
    });
}

pub(crate) async fn refresh_daily_due(s:&AppState){
    for provider in crate::connectors::providers(s,"recommendations").await{
        let Ok(manifest)=crate::connectors::manifest(s,&provider).await else{continue;};
        if !manifest["capabilities"].as_array().is_some_and(|caps|daily_enabled(&provider,caps)){continue;}
        let name=provider.clone();
        let connected=s.db.read(move|c|Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM provider_accounts WHERE provider=?1 AND context NOT IN ('anonymous','public','switching'))",
            [name],|r|r.get::<_,bool>(0))?)).await.unwrap_or(false);
        if !connected{continue;}
        let Ok(generation)=crate::provider_session::start(s,&provider).await else{continue;};
        let version=manifest["version"].as_str().unwrap_or("");
        let _=refresh_daily(s,&provider,version,generation,true).await;
    }
}

async fn fetch_platform(s:&AppState,provider:&str,public:bool,page:u32,refresh:bool,generation:i64,daily:Option<(String,String)>)->ApiResult<Value>{
    let method=if public{"public_recommendations"}else{"recommendations"};
    let response=s.plugin.call(provider,method,json!({"limit":24,"page":page,"refresh":refresh})).await?;
    let mut sections=parse_sections(&response,provider,public);
    if sections.is_empty() {
        let items=platform_tracks(&response["items"],provider);
        if !items.is_empty(){sections.push(json!({"id":"tracks","title":"平台精选","source":provider,"kind":"tracks","items":items}));}
    }
    let next=response["next_page"].as_u64().filter(|next|*next==u64::from(page)+1);
    let provider=provider.to_owned();
    Ok(s.db.run(move|c|{
        let tx=c.transaction()?;
        crate::provider_session::check(&tx,&provider,generation)?;
        for section in &mut sections {
            if let Some(items)=section["items"].as_array() {
                for item in items {
                    let track:Track=serde_json::from_value(item.clone())?;
                    if !tx.query_row("SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",[&track.id],|r|r.get::<_,bool>(0))? {
                        crate::library::upsert(&tx,&track,None,None,0,0,None,None)?;
                    }
                }
            }
            for field in ["items","playlists"]{crate::media_assets::normalize_entries(&tx,&mut section[field]);}
        }
        let value=json!({"source":provider,"page":page,"next_page":next,"sections":sections});
        if let Some((version,day))=daily{
            tx.execute("INSERT INTO provider_daily_recommendations(provider,generation,version,day,data) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(provider) DO UPDATE SET generation=excluded.generation,version=excluded.version,day=excluded.day,data=excluded.data",
                params![provider,generation,version,day,value.to_string()])?;
        }
        tx.commit()?;
        Ok(value)
    }).await?)
}
fn platform_tracks(items:&Value,provider:&str)->Vec<Track>{
    let mut seen=HashSet::new();
    items.as_array().into_iter().flatten().take(24)
        .filter_map(|item|serde_json::from_value::<Track>(item.clone()).ok())
        .filter(|track|valid_cloud_track(track,provider)&&seen.insert(track.id.clone())).collect()
}
fn parse_sections(response:&Value,provider:&str,public:bool)->Vec<Value>{
    let mut sections=vec![];
    let mut seen=HashSet::new();
    for section in response["sections"].as_array().into_iter().flatten().take(8) {
        let Some(id)=section["id"].as_str().filter(|id|crate::connectors::valid_id(id)&&seen.insert(*id)) else{continue;};
        let Some(title)=section["title"].as_str().filter(|title|!title.trim().is_empty()&&title.len()<=256) else{continue;};
        let items=platform_tracks(&section["items"],provider);
        let playlists:Vec<_>=if public{vec![]}else{section["playlists"].as_array().into_iter().flatten().take(12)
            .filter(|item|item["id"].as_str().is_some_and(|id|!id.is_empty()&&id.len()<=256)&&item["name"].as_str().is_some_and(|name|name.len()<=4096)&&item["source"]==provider)
            .cloned().collect()};
        if items.is_empty()&&playlists.is_empty(){continue;}
        let kind=if section["kind"]=="albums"{"albums"}else if items.is_empty(){"playlists"}else{"tracks"};
        sections.push(json!({"id":id,"title":title,"source":provider,"kind":kind,"items":items,"playlists":playlists}));
    }
    sections
}

async fn cloud(s:&AppState,provider:&str,context:&Context,limit:usize,refresh:bool)->Result<(Vec<Track>,Vec<serde_json::Value>),String> {
    crate::plugin::check_enabled(s,provider).await.map_err(|_|"音源插件已停用".to_owned())?;
    let manifest=crate::connectors::manifest(s,provider).await.map_err(|_|"未安装可用的推荐插件".to_owned())?;
    if !manifest["capabilities"].as_array().is_some_and(|caps|caps.iter().any(|cap|cap=="recommendations")){return Err("音源尚未声明推荐能力".into());}
    let response = s.plugin.call(provider, "recommendations", json!({
        "limit":limit,"refresh":refresh,
        "seed_track_id":context.seed.as_ref().filter(|track|track.source==provider).map(|track|&track.id),
        "exclude_track_ids":context.excluded,
        "exclude_recording_ids":context.recordings
    })).await.map_err(|error| -> String {
        if error.1=="login_required" { "音源推荐需要先登录".into() }
        else { "音源推荐暂时不可用".into() }
    })?;
    let items = response["items"]
        .as_array()
        .ok_or_else(|| "音源推荐响应格式错误".to_owned())?;
    let mut sections=vec![];
    for section in response["sections"].as_array().into_iter().flatten().take(8) {
        let Some(title)=section["title"].as_str().filter(|value|value.len()<=256) else{continue;};
        let tracks:Vec<Track>=section["items"].as_array().into_iter().flatten().take(24).filter_map(|value|serde_json::from_value(value.clone()).ok()).filter(|track|valid_cloud_track(track,provider)).collect();
        let playlists:Vec<_>=section["playlists"].as_array().into_iter().flatten().take(12).filter(|item|item["id"].as_str().is_some_and(|v|!v.is_empty()&&v.len()<=256)&&item["name"].as_str().is_some_and(|v|v.len()<=4096)&&item["source"]==provider).cloned().collect();
        sections.push(json!({"id":section["id"],"title":title,"source":provider,"items":tracks,"playlists":playlists}));
    }
    let mut tracks: Vec<_> = items
        .iter()
        .take(200)
        .filter_map(|value| serde_json::from_value::<Track>(value.clone()).ok())
        .filter(|track|valid_cloud_track(track,provider))
        .collect();
    let provider=provider.to_owned();
    let excluded = context.excluded.clone();
    let blocked = context.recordings.clone();
    tracks =
        s.db.run(move |c| {
            let mut result = vec![];
            let mut ids = excluded;
            let mut recordings = blocked;
            for incoming in tracks {
                let existing = c
                    .query_row(
                        "SELECT * FROM tracks WHERE id=?1",
                        [&incoming.id],
                        Track::from_row,
                    )
                    .optional()?;
                let mut track = existing.unwrap_or_else(||incoming.clone());
                if let Some(hint)=incoming.provider_data.get("quality_hint") {
                    if track.provider_data.is_object(){track.provider_data["quality_hint"]=hint.clone();}
                }
                if !valid_cloud_track(&track,&provider)
                    || !ids.insert(track.id.clone())
                    || (!track.recording_id.is_empty()
                        && !recordings.insert(track.recording_id.clone()))
                {
                    continue;
                }
                result.push(track);
                if result.len() >= limit {
                    break;
                }
            }
            Ok(result)
        })
        .await
        .map_err(|_| "音源推荐曲目处理失败".to_owned())?;
    if tracks.is_empty()&&sections.iter().all(|s|s["items"].as_array().is_none_or(Vec::is_empty)&&s["playlists"].as_array().is_none_or(Vec::is_empty)) {
        return Err("音源没有返回可用且未重复的推荐歌曲".into());
    }
    Ok((tracks,sections))
}
fn valid_cloud_track(track:&Track,provider:&str)->bool {
    let Some(mid) = track.id.strip_prefix(&format!("{provider}:")) else {
        return false;
    };
    track.source == provider
        && track.available
        && (1..=256).contains(&mid.len())
        && !mid.chars().any(|c|c.is_whitespace()||c.is_control())
        && !track.title.trim().is_empty()
        && track.title.len() <= 4096
        && track.artist.len() <= 4096
        && track.album.len() <= 4096
        && !track.recording_id.is_empty()
        && track.recording_id.len() <= 256
        && track.format.len() <= 32
        && track.duration.is_finite()
        && track.duration >= 0.
        && track.provider_data.to_string().len() <= 256 * 1024
}
fn combine(local: Vec<Track>, cloud: Vec<Track>, limit: usize) -> Vec<Track> {
    let mut local = local.into_iter();
    let mut cloud = cloud.into_iter();
    let mut recordings = HashSet::new();
    let mut ids = HashSet::new();
    let mut result = vec![];
    loop {
        let pair = [local.next(), cloud.next()];
        if pair.iter().all(Option::is_none) {
            break;
        }
        for track in pair.into_iter().flatten() {
            if !ids.insert(track.id.clone())
                || (!track.recording_id.is_empty()
                    && !recordings.insert(track.recording_id.clone()))
            {
                continue;
            }
            result.push(track);
            if result.len() >= limit {
                return result;
            }
        }
    }
    result
}

pub async fn select(
    s: &AppState,
    source: &str,
    seed_track_id: Option<String>,
    exclude: Vec<String>,
    limit: usize,
    fallback_local: bool,
) -> ApiResult<Selection> {
    select_request(s,source,seed_track_id,exclude,limit,fallback_local,false).await
}
async fn select_request(
    s:&AppState,source:&str,seed_track_id:Option<String>,exclude:Vec<String>,limit:usize,fallback_local:bool,refresh:bool,
) -> ApiResult<Selection> {
    if !crate::connectors::valid_id(source) || limit == 0 || limit > 50 {
        return Err(ApiError::bad(
            "推荐来源须为已启用的音源、local 或 mixed，数量应为 1..50",
        ));
    }
    if seed_track_id.as_ref().is_some_and(|id| id.len() > 256) {
        return Err(ApiError::bad("种子曲目 ID 过长"));
    }
    let now = chrono::Utc::now().timestamp();
    let (context, local) =
        s.db.read(move |c| {
            let context = context(c, seed_track_id, exclude, now)?;
            let day=chrono::Local::now().format("%Y-%m-%d").to_string();
            let mut rng = StdRng::seed_from_u64(u64::from_str_radix(&crate::auth::hash(&day)[..16],16).unwrap_or(0));
            let local = local_selection(
                local_pool(c, &context, &mut rng)?,
                &context,
                now,
                limit,
                &mut rng,
            );
            Ok((context, local))
        })
        .await?;
    let mut fallback = false;
    let mut message = None;
    let mut sections=vec![];
    let items = if source == "local" {
        local
    } else {
        let providers=if source=="mixed" {crate::connectors::providers(s,"recommendations").await}else{vec![source.to_owned()]};
        let mut all=vec![];let mut reason="没有已启用的推荐音源".to_owned();
        for provider in providers.into_iter().take(4) {match cloud(s,&provider,&context,limit,refresh).await {Ok((items,extra))=>{all.extend(items);sections.extend(extra);},Err(error)=>reason=error,}}
        let result=if all.is_empty()&&sections.is_empty(){Err(reason)}else{Ok(all)};
        match result {
            Ok(cloud) => {
                message = Some("推荐由已启用的音源提供；种子仅用于排除，关联能力取决于连接器声明".into());
                if source == "mixed" {
                    combine(local, cloud, limit)
                } else {
                    cloud
                }
            }
            Err(reason) => {
                message = Some(if fallback_local {
                    format!("{reason}，已回退本地推荐")
                } else {
                    reason
                });
                fallback = fallback_local;
                if fallback_local {
                    local
                } else {
                    vec![]
                }
            }
        }
    };
    let items: Vec<_> = crate::catalog::prefer_local(s,items).await?.into_iter()
        .filter(|track| !context.excluded.contains(&track.id)
            && !context.recordings.contains(&track.recording_id))
        .collect();
    let sources:HashSet<_>=items.iter().map(|t|t.source.as_str()).collect();
    let actual=if sources.len()>1{"mixed"}else{sources.iter().next().copied().unwrap_or(if source!="local"&&!fallback{source}else{"local"})}.to_owned();
    if items.is_empty() {
        let reason = "没有可用且未近期播放的候选歌曲";
        message = Some(message.map_or_else(|| reason.into(), |m| format!("{m}；{reason}")));
    }
    let imported: Vec<_> = items
        .iter()
        .filter(|t| t.source != "local")
        .cloned()
        .chain(sections.iter().flat_map(|section|section["items"].as_array().into_iter().flatten()).filter_map(|value|serde_json::from_value::<Track>(value.clone()).ok()))
        .collect();
    if !imported.is_empty() {
        s.db.run(move |c| {
            let tx = c.transaction()?;
            for track in imported {
                if !tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tracks WHERE id=?1)",
                    [&track.id],
                    |r| r.get::<_, bool>(0),
                )? {
                    crate::library::upsert(&tx, &track, None, None, 0, 0, None, None)?;
                } else if let Some(hint)=track.provider_data.get("quality_hint") {
                    tx.execute("UPDATE tracks SET provider_data=json_set(provider_data,'$.quality_hint',json(?2)) WHERE id=?1",params![track.id,hint.to_string()])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    }
    Ok(Selection {
        items,
        source: actual.into(),
        fallback,
        message,
        sections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Db, state::Config};

    fn track(id: &str, artist: &str) -> Track {
        Track {
            id: id.into(),
            title: format!("歌曲 {id}"),
            artist: artist.into(),
            album: "测试专辑".into(),
            album_artist: artist.into(),
            duration: 180.,
            track_number: 1,
            disc_number: 1,
            source: "local".into(),
            format: "flac".into(),
            sample_rate: 48_000,
            bit_depth: 24,
            channels: 2,
            bitrate: 1_000_000,
            cover_url: None,
            recording_id: format!("recording-{id}"),
            available: true,
            created_at: chrono::Utc::now().timestamp(),
            integrated_lufs: None,
            provider_data: json!({}),
        }
    }
    async fn fixture(tracks: Vec<Track>) -> (tempfile::TempDir, AppState) {
        let directory = tempfile::tempdir().unwrap();
        let db = Db::open(&directory.path().join("library.db")).unwrap();
        db.run(move |c| {
            for track in tracks {
                crate::library::upsert(c, &track, None, None, 0, 0, None, None)?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let state = AppState::new(
            db,
            Config {
                data: directory.path().into(),
                web: directory.path().into(),
                base_path: String::new(),
                public_url: None,
                plugin_dir: directory.path().join("plugins"),
                python: if cfg!(windows){"python".into()}else{"python3".into()},
            },
            String::new(),
        )
        .await
        .unwrap();
        (directory, state)
    }
    async fn statistics(s: &AppState, id: &str, plays: i64, skips: i64, seconds: f64, last: i64) {
        let id = id.to_owned();
        s.db.run(move|c| { c.execute("INSERT INTO track_stats(track_id,play_count,skip_count,play_seconds,last_played_at)VALUES(?1,?2,?3,?4,?5)",params![id,plays,skips,seconds,last])?;Ok(()) }).await.unwrap();
    }
    async fn fake_provider(s: &AppState, items: serde_json::Value, capability: bool) {
        let directory = s.config.plugin_dir.join("qqmusic");
        tokio::fs::create_dir_all(&directory).await.unwrap();
        tokio::fs::write(directory.join("manifest.json"),serde_json::to_vec(&json!({"id":"qqmusic","capabilities":if capability{vec!["recommendations"]}else{vec![]}})).unwrap()).await.unwrap();
        tokio::fs::write(
            directory.join("fixture.json"),
            serde_json::to_vec(&items).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(directory.join("provider.py"),r#"import json,sys
from pathlib import Path
payload=json.loads(Path(__file__).with_name('fixture.json').read_text('utf-8'))
for line in sys.stdin:
    request=json.loads(line)
    assert request['method']=='recommendations'
    assert request['params']['limit']<=50
    seed=request['params'].get('seed_track_id')
    assert seed is None or seed.startswith('qqmusic:')
    response={'id':request['id'],'error':payload['error']} if isinstance(payload,dict) and 'error' in payload else {'id':request['id'],'result':{'items':payload,'basis':'qq-personalized','seed_applied':False}}
    print(json.dumps(response),flush=True)
"#).await.unwrap();
    }
    fn qq(id: &str, artist: &str) -> Track {
        let mut track = track(id, artist);
        track.id = format!("qqmusic:{id}");
        track.source = "qqmusic".into();
        track.recording_id = track.id.clone();
        track
    }

    #[tokio::test]
    async fn no_history_explores_without_a_provider_and_bounds_the_pool() {
        let tracks: Vec<_> = (0..1200)
            .map(|i| track(&format!("song-{i}"), &format!("artist-{}", i % 12)))
            .collect();
        let (_directory, state) = fixture(tracks).await;
        let (context, pool) = state
            .db
            .run(|c| {
                let context = context(c, None, vec![], chrono::Utc::now().timestamp())?;
                let pool = local_pool(c, &context, &mut StdRng::seed_from_u64(7))?;
                Ok((context, pool))
            })
            .await
            .unwrap();
        assert!(pool.len() <= MAX_POOL);
        let a = local_selection(
            pool.clone(),
            &context,
            chrono::Utc::now().timestamp(),
            20,
            &mut StdRng::seed_from_u64(1),
        );
        let b = local_selection(
            pool,
            &context,
            chrono::Utc::now().timestamp(),
            20,
            &mut StdRng::seed_from_u64(2),
        );
        assert_ne!(
            a.iter().map(|t| &t.id).collect::<Vec<_>>(),
            b.iter().map(|t| &t.id).collect::<Vec<_>>()
        );
        let result = select(&state, "local", None, vec![], 20, true)
            .await
            .unwrap();
        assert_eq!(result.items.len(), 20);
        assert_eq!(result.source, "local");
        assert!(!result.fallback);
        assert!(result
            .items
            .iter()
            .all(|t| t.available && t.source == "local"));
    }
    #[tokio::test]
    async fn measured_artist_preference_and_seed_favor_unheard_songs() {
        let (_directory, state) = fixture(vec![
            track("history", "常听艺人"),
            track("unheard-favorite", "常听艺人"),
            track("exploration", "其他艺人"),
        ])
        .await;
        statistics(
            &state,
            "history",
            30,
            0,
            5400.,
            chrono::Utc::now().timestamp() - 30 * 86_400,
        )
        .await;
        let result = select(&state, "local", None, vec!["history".into()], 1, true)
            .await
            .unwrap();
        assert_eq!(result.items[0].id, "unheard-favorite");
        let result = select(&state, "local", Some("history".into()), vec![], 1, true)
            .await
            .unwrap();
        assert_eq!(result.items[0].id, "unheard-favorite");
    }
    #[tokio::test]
    async fn frequent_skips_reduce_a_candidates_rank() {
        let (_directory, state) = fixture(vec![
            track("skipped", "同一艺人"),
            track("unheard", "同一艺人"),
        ])
        .await;
        statistics(
            &state,
            "skipped",
            0,
            25,
            300.,
            chrono::Utc::now().timestamp() - 30 * 86_400,
        )
        .await;
        let result = select(&state, "local", None, vec![], 1, true)
            .await
            .unwrap();
        assert_eq!(result.items[0].id, "unheard");
    }
    #[tokio::test]
    async fn recent_actual_sessions_exclude_all_versions_of_the_recording() {
        let mut recent = track("recent", "A");
        recent.recording_id = "same-recording".into();
        let mut alternate = track("alternate", "A");
        alternate.recording_id = "same-recording".into();
        let (_directory, state) = fixture(vec![recent, alternate, track("fresh", "B")]).await;
        state.db.run(|c|{let now=chrono::Utc::now().timestamp_millis();c.execute("INSERT INTO play_sessions(id,zone_id,track_id,title,artist,duration,started_at,last_report_at,last_position,last_status,runtime_id,play_seconds)VALUES('recent-session','test-zone','recent','recent','A',180,?1,?1,20,'playing','test',20)",[now])?;Ok(())}).await.unwrap();
        let result = select(&state, "local", None, vec![], 20, true)
            .await
            .unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            vec!["fresh"]
        );
    }
    #[tokio::test]
    async fn queue_exclusion_seed_and_unavailable_tracks_never_repeat() {
        let mut a = track("seed", "A");
        a.recording_id = "recording-a".into();
        let mut a2 = track("seed-version", "A");
        a2.recording_id = a.recording_id.clone();
        let mut b = track("queued", "B");
        b.recording_id = "recording-b".into();
        let mut b2 = track("queued-version", "B");
        b2.recording_id = b.recording_id.clone();
        let mut missing = track("missing", "C");
        missing.available = false;
        let (_directory, state) = fixture(vec![a, a2, b, b2, missing]).await;
        let result = select(
            &state,
            "local",
            Some("seed".into()),
            vec!["queued".into()],
            20,
            true,
        )
        .await
        .unwrap();
        assert!(result.items.is_empty());
        assert!(result.message.is_some());
    }
    #[tokio::test]
    async fn diversity_spreads_equal_candidates_across_artists() {
        let (_directory, state) = fixture(
            (0..30)
                .map(|i| track(&format!("song-{i}"), &format!("artist-{}", i % 3)))
                .collect(),
        )
        .await;
        let result = select(&state, "local", None, vec![], 3, true)
            .await
            .unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .map(|t| &t.artist)
                .collect::<HashSet<_>>()
                .len(),
            3
        );
        let result = select(&state, "local", None, vec![], 50, true)
            .await
            .unwrap();
        assert_eq!(result.items.len(), 30);
        assert_eq!(
            result
                .items
                .iter()
                .map(|t| &t.recording_id)
                .collect::<HashSet<_>>()
                .len(),
            30
        );
    }
    #[tokio::test]
    async fn missing_or_incapable_cloud_provider_has_explicit_optional_fallback() {
        let (_directory, state) = fixture(vec![track("local-song", "A")]).await;
        let result = select(&state, "qqmusic", None, vec![], 5, true)
            .await
            .unwrap();
        assert!(result.fallback);
        assert_eq!(result.source, "local");
        assert_eq!(result.items.len(), 1);
        let result = select(&state, "qqmusic", None, vec![], 5, false)
            .await
            .unwrap();
        assert!(!result.fallback);
        assert!(result.items.is_empty());
        assert!(result.message.unwrap().contains("推荐插件"));
        fake_provider(&state, json!([]), false).await;
        let result = select(&state, "mixed", None, vec![], 5, true)
            .await
            .unwrap();
        assert!(result.fallback);
        assert!(result.message.unwrap().contains("推荐音源"));
        state
            .db
            .run(|c| {
                c.execute(
                    "INSERT INTO plugins(id,enabled,version)VALUES('qqmusic',0,'test')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let result = select(&state, "qqmusic", None, vec![], 5, true)
            .await
            .unwrap();
        assert!(result.message.unwrap().contains("停用"));
    }
    #[tokio::test]
    async fn real_jsonl_provider_validates_excludes_deduplicates_and_imports_cloud_tracks() {
        let mut blocked = track("seed", "local-artist");
        blocked.recording_id = "shared-recording".into();
        let (_directory, state) =
            fixture(vec![blocked, track("local-fresh", "local-artist")]).await;
        let mut duplicate = qq("000001", "Cloud-A");
        duplicate.recording_id = "shared-recording".into();
        let good = qq("000002", "Cloud-B");
        let mut second = qq("000003", "Cloud-C");
        second.recording_id = good.recording_id.clone();
        let mut invalid = qq("bad/../../secret", "Cloud-X");
        invalid.source = "local".into();
        fake_provider(
            &state,
            json!([
                duplicate,
                good.clone(),
                second,
                invalid,
                qq("000004", "Cloud-D")
            ]),
            true,
        )
        .await;
        let result = select(&state, "qqmusic", Some("seed".into()), vec![], 2, false)
            .await
            .unwrap();
        assert_eq!(result.source, "qqmusic");
        assert!(!result.fallback);
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].id, good.id);
        assert_ne!(result.items[0].recording_id, result.items[1].recording_id);
        let saved = crate::library::get_track(&state, good.id.clone())
            .await
            .unwrap();
        assert_eq!(saved.source, "qqmusic");
        let result = select(&state, "mixed", Some("seed".into()), vec![], 4, true)
            .await
            .unwrap();
        assert_eq!(result.source, "mixed");
        assert_eq!(result.items.len(), 3);
        assert!(result.message.unwrap().contains("种子仅用于排除"));
        state.plugin.stop("qqmusic").await;
    }
    #[tokio::test]
    async fn empty_cloud_result_falls_back_and_input_limits_are_enforced() {
        let (_directory, state) = fixture(vec![track("local", "A")]).await;
        fake_provider(&state, json!([]), true).await;
        let result = select(&state, "qqmusic", None, vec![], 1, true)
            .await
            .unwrap();
        assert!(result.fallback);
        assert_eq!(result.items[0].id, "local");
        assert!(result.message.unwrap().contains("未重复"));
        assert!(select(&state, "../all", None, vec![], 1, true).await.is_err());
        assert!(select(&state, "local", None, vec![], 0, true)
            .await
            .is_err());
        assert!(select(&state, "local", None, vec![], 51, true)
            .await
            .is_err());
        state.plugin.stop("qqmusic").await;
    }
    #[tokio::test]
    async fn existing_cloud_recording_corrections_survive_recommendation_refresh() {
        let mut existing = qq("000010", "人工校正艺人");
        existing.recording_id = "manually-matched-recording".into();
        existing.provider_data = json!({"integrated_lufs":-19.0,"manual_metadata":true});
        let mut seed = track("seed", "local artist");
        seed.recording_id = existing.recording_id.clone();
        let (_directory, state) = fixture(vec![existing.clone(), seed]).await;
        let incoming = qq("000010", "上游艺人");
        fake_provider(&state, json!([incoming, qq("000011", "new artist")]), true).await;
        let selected = select(&state, "qqmusic", None, vec![], 2, false)
            .await
            .unwrap();
        assert_eq!(selected.items[0].id, "seed");
        assert_eq!(selected.items[0].artist, "local artist");
        assert_eq!(selected.items[0].recording_id, existing.recording_id);
        let excluded = select(&state, "qqmusic", Some("seed".into()), vec![], 2, false)
            .await
            .unwrap();
        assert_eq!(excluded.items.len(), 1);
        assert_eq!(excluded.items[0].id, "qqmusic:000011");
        let saved = crate::library::get_track(&state, existing.id)
            .await
            .unwrap();
        assert_eq!(saved.provider_data["manual_metadata"], true);
        assert_eq!(saved.artist, existing.artist);
        assert_eq!(saved.integrated_lufs, Some(-19.));
        state.plugin.stop("qqmusic").await;
    }
    #[tokio::test]
    async fn cloud_mapping_cannot_recommend_the_local_seed_again() {
        let seed = track("seed", "artist");
        let mut cloud = seed.clone();
        cloud.id = "qqmusic:000020".into();
        cloud.source = "qqmusic".into();
        cloud.recording_id = cloud.id.clone();
        let (_directory, state) = fixture(vec![seed]).await;
        fake_provider(&state, json!([cloud]), true).await;
        let selected = select(&state, "qqmusic", Some("seed".into()), vec![], 2, false).await.unwrap();
        assert!(selected.items.is_empty());
        state.plugin.stop("qqmusic").await;
    }
    #[tokio::test]
    async fn provider_login_failure_is_an_explicit_fallback() {
        let (_directory, state) = fixture(vec![track("local-song", "A")]).await;
        fake_provider(
            &state,
            json!({"error":{"code":"login_required","message":"provider-secret-must-not-leak"}}),
            true,
        )
        .await;
        let selected = select(&state, "qqmusic", None, vec![], 5, true)
            .await
            .unwrap();
        assert_eq!(selected.source, "local");
        assert!(selected.fallback);
        let message = selected.message.unwrap();
        assert!(message.contains("需要先登录"));
        assert!(!message.contains("secret"));
        state.plugin.stop("qqmusic").await;
    }
    #[tokio::test]
    async fn http_endpoint_requires_read_scope_validates_limit_and_does_not_play() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let (_directory, state) = fixture(vec![track("song", "A")]).await;
        state.db.run(|c|{
            c.execute("INSERT INTO users(id,username,password,role)VALUES('reader','reader','unused','user')",[])?;
            c.execute("INSERT INTO sessions(hash,user_id,expires,scopes,created_at)VALUES(?1,'reader',?2,'[\"read\"]',0)",params![crate::auth::hash("reader-token"),chrono::Utc::now().timestamp()+3600])?;
            c.execute("INSERT INTO sessions(hash,user_id,expires,scopes,created_at)VALUES(?1,'reader',?2,'[\"control\"]',0)",params![crate::auth::hash("controller-token"),chrono::Utc::now().timestamp()+3600])?;
            Ok(())
        }).await.unwrap();
        let router = axum::Router::new()
            .route("/recommendations", axum::routing::get(handler))
            .with_state(state.clone());
        let request = |query: &str, token: Option<&str>| {
            let mut builder = Request::builder().uri(format!("/recommendations{query}"));
            if let Some(token) = token {
                builder = builder.header("Authorization", format!("Bearer {token}"));
            }
            builder.body(Body::empty()).unwrap()
        };
        assert_eq!(
            router
                .clone()
                .oneshot(request("?source=local", None))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            router
                .clone()
                .oneshot(request("?source=local", Some("controller-token")))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            router
                .clone()
                .oneshot(request("?source=local&limit=51", Some("reader-token")))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        let response = router
            .oneshot(request("", Some("reader-token")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["source"], "local");
        assert_eq!(body["fallback"], true);
        assert_eq!(body["items"].as_array().unwrap().len(), 1);
        assert!(state
            .zones
            .read()
            .await
            .values()
            .all(|zone| zone.status == "stopped" && zone.queue.is_empty()));
    }
}
