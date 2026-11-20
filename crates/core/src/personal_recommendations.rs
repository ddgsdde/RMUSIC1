//! Account-owned daily lists and on-demand recording neighbours.
use super::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Weak};
use tokio::sync::Mutex;

const DAILY_SIZE: usize = 40;
const MODEL_MATCH: &str = "AI 关联选曲，已匹配音源目录";

pub(crate) type DailyLocks = Mutex<HashMap<String, Weak<Mutex<()>>>>;

pub(super) struct Intent {
    hints: Vec<crate::llm::SongHint>,
    previous: HashSet<String>,
    collaborators: Vec<String>,
    genres: Vec<String>,
}
fn key(t: &Track) -> &str {
    if t.recording_id.is_empty() {
        &t.id
    } else {
        &t.recording_id
    }
}
fn same(a: &str, b: &str) -> bool {
    !a.trim().is_empty() && a.trim().to_lowercase() == b.trim().to_lowercase()
}
fn nomination_matches(hint: &crate::llm::SongHint, track: &Track) -> bool {
    if !same(&hint.artist, &track.artist) && !credits(track).next().is_some_and(|artist| same(&hint.artist, artist)) {
        return false;
    }
    if same(&hint.title, &track.title) { return true; }
    let Some((title, label)) = track.title.trim().strip_suffix(')').and_then(|s| s.rsplit_once(" (")) else { return false };
    if !same(&hint.title, title) { return false; }
    // QQ includes translated titles and explicit-content labels in its display title.
    same(label, "explicit") || same(label, "single version") ||
        label.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        && !["现场", "翻唱", "重制", "重录", "伴奏", "纯音乐", "不插电", "版本", "版"].iter().any(|v| label.contains(v))
}
fn credits(track: &Track) -> impl Iterator<Item = &str> {
    track
        .artist
        .split(" / ")
        .map(str::trim)
        .filter(|v| !v.is_empty() && *v != "未知歌手" && !v.eq_ignore_ascii_case("unknown artist"))
}
impl Intent {
    pub(super) fn score(
        &self,
        row: &Candidate,
        seed: Option<&Candidate>,
        base: f64,
        reason: &'static str,
        liked: bool,
    ) -> Option<(f64, &'static str)> {
        let track = &row.track;
        let proposed = self
            .hints
            .iter()
            .any(|h| nomination_matches(h, track));
        let unheard = row.seconds == 0. && row.plays == 0. && row.skips == 0.;
        if let Some(seed) = seed {
            let seed = &seed.track;
            if same(&seed.title, &track.title)
                && same(&seed.artist, &track.artist)
                && (seed.duration - track.duration).abs() <= 2.0
            {
                return None;
            }
            let shared = credits(seed).any(|a| credits(track).any(|b| same(a, b)));
            let genre = feature(track, "genre").is_some_and(|g| self.genres.contains(&g));
            let collaborator =
                credits(track).any(|name| self.collaborators.iter().any(|other| same(name, other)));
            let (weight, explanation) = if proposed {
                (if genre || shared { 29. } else { 25. }, MODEL_MATCH)
            } else if genre && shared {
                (25., "曲风相近，有共同演出者")
            } else if genre {
                (22., "曲风相近")
            } else if shared {
                (16., "有共同演出者")
            } else if collaborator {
                (14., "来自这首歌演出者的合作音乐人")
            } else {
                return None;
            };
            return Some((
                weight + base.clamp(-10., 12.) * 0.15 + if unheard { 3. } else { 0. },
                explanation,
            ));
        }
        let novelty = if unheard && !liked { 16. } else { 0. };
        let repetition = if self.previous.contains(key(track)) {
            26.
        } else {
            0.
        };
        let style = feature(track, "genre").is_some_and(|g| self.genres.contains(&g));
        Some((
            base + novelty + if style { 4. } else { 0. } - repetition
                + if proposed { 18. } else { 0. },
            if proposed {
                MODEL_MATCH
            } else {
                reason
            },
        ))
    }
}

fn examples(
    c: &Connection,
    user: &str,
    seed: Option<&str>,
    predicate: &str,
) -> anyhow::Result<Vec<Track>> {
    if let Some(seed) = seed {
        let sql = format!("SELECT t.* FROM tracks t WHERE t.id=?1 AND {predicate}");
        return Ok(c
            .query_row(&sql, [seed], Track::from_row)
            .optional()?
            .into_iter()
            .collect());
    }
    let fav = format!("favorites-{}", crate::auth::hash(user));
    let mut result = vec![];
    for query in [
        format!("SELECT t.* FROM account_track_stats p JOIN tracks t ON t.id=p.track_id WHERE p.user_id=?1 AND p.play_count>p.skip_count AND {predicate} ORDER BY p.last_played_at DESC LIMIT 6"),
        format!("SELECT t.* FROM playlists l JOIN json_each(l.tracks) i JOIN tracks t ON t.id=i.value WHERE l.user_id=?1 AND l.id=?2 AND {predicate} ORDER BY i.key DESC LIMIT 6"),
        format!("SELECT t.* FROM account_track_stats p JOIN tracks t ON t.id=p.track_id WHERE p.user_id=?1 AND p.play_count>p.skip_count AND {predicate} ORDER BY p.play_seconds DESC LIMIT 4"),
    ] {
        let mut stmt=c.prepare(&query)?;
        let args=if stmt.parameter_count()==2{vec![user,fav.as_str()]}else{vec![user]};
        for row in stmt.query_map(rusqlite::params_from_iter(args),Track::from_row)? {
            let track=row?;if !result.iter().any(|t:&Track|t.id==track.id){result.push(track);}
        }
    }
    if result.is_empty() {
        let sql=format!("SELECT t.* FROM tracks t WHERE t.available=1 AND {predicate} ORDER BY t.created_at DESC LIMIT 4");
        result = c
            .prepare(&sql)?
            .query_map([], Track::from_row)?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(result)
}

fn collaborators(
    c: &Connection,
    anchors: &[Track],
    predicate: &str,
) -> anyhow::Result<Vec<String>> {
    let sql = format!(
        "SELECT b.name FROM track_artists a
        JOIN track_artists b ON b.track_id=a.track_id AND b.artist_id<>a.artist_id
        JOIN tracks t ON t.id=a.track_id
        WHERE a.artist_id IN (SELECT artist_id FROM track_artists WHERE track_id=?1) AND {predicate}
        GROUP BY b.name ORDER BY count(*) DESC, b.name LIMIT 4"
    );
    let mut statement = c.prepare(&sql)?;
    let mut names = Vec::new();
    for anchor in anchors.iter().take(3) {
        for row in statement.query_map([&anchor.id], |r| r.get::<_, String>(0))? {
            let name = row?;
            if !anchors
                .iter()
                .any(|t| credits(t).any(|own| same(own, &name)))
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
    }
    Ok(names)
}

fn metadata_candidates(
    c: &Connection,
    anchors: &[Track],
    predicate: &str,
) -> anyhow::Result<Vec<String>> {
    let sql=format!("SELECT t.id FROM tracks t WHERE t.available=1 AND {predicate} AND (
      (?1<>'' AND lower(COALESCE(json_extract(t.provider_data,'$.genre'),json_extract(t.provider_data,'$.provider_metadata.background.genre'))) = ?1)
      OR t.id IN (SELECT b.track_id FROM track_artists a JOIN track_artists b ON b.artist_id=a.artist_id WHERE a.track_id=?2)
      OR (?3<>'' AND t.artist=?3)) ORDER BY t.created_at DESC LIMIT 120");
    let mut stmt = c.prepare(&sql)?;
    let mut ids = Vec::new();
    for t in anchors.iter().take(4) {
        ids.extend(
            stmt.query_map(
                params![feature(t, "genre").unwrap_or_default(), t.id, t.artist],
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    Ok(ids)
}

#[derive(Serialize, Deserialize)]
struct Pick {
    id: String,
    recording: String,
    reason: String,
    epoch: Option<i64>,
    #[serde(default)]
    score: Option<f64>,
}

// Request just the first reference recording's basic text tags. The connector owns its metadata cache.
async fn fill_genre(s: &AppState, track: &mut Track, providers: &[String]) {
    if feature(track, "genre").is_some()
        || !providers.contains(&track.source)
        || !crate::connectors::providers(s, "metadata")
            .await
            .contains(&track.source)
    {
        return;
    }
    let request = json!({"track_id": track.id, "track": {"id": track.id, "source": track.source, "title": track.title, "artist": track.artist}, "section": "basic"});
    let Ok(Ok(metadata)) = tokio::time::timeout(
        Duration::from_millis(2500),
        s.plugin.call(&track.source, "metadata", request),
    )
    .await
    else {
        return;
    };
    if metadata["matched"] == true
        && metadata["source"] == track.source
        && metadata["track"]["id"] == track.id
    {
        if let Some(style) = metadata["background"]["genre"]
            .as_str()
            .filter(|v| !v.trim().is_empty())
        {
            track.provider_data["genre"] = json!(style);
        }
    }
}

async fn platforms(s:&AppState, policy:Option<&crate::accounts::Policy>)->Vec<String> {
    let (search,resolve)=tokio::join!(crate::connectors::providers(s,"search"),crate::connectors::providers(s,"resolve"));
    search.into_iter().filter(|p|resolve.contains(p)&&policy.is_none_or(|v|v.providers.contains(p))).collect()
}

#[derive(Default)]
struct CatalogSearch {
    ids: Vec<String>,
    hints: Vec<crate::llm::SongHint>,
    playable: HashSet<String>,
}

impl CatalogSearch {
    async fn discover(&mut self,s:&AppState,owner:&str,providers:&[String],examples:&[Value],similar:bool,goal:usize)->ApiResult<()> {
        let mut history=Vec::new();
        let mut tried=HashSet::new();
        for _ in 0..3 {
            let remaining=goal.saturating_sub(self.hints.len()).clamp(2,8);
            let mut batch=match crate::llm::search_hints(s,examples,similar,&history,remaining).await {
                Ok(v)=>v,
                Err(e)=>{history.push(json!({"outcome":"invalid or unavailable model response","detail":e.2}));if history.len()==1 {continue;} break;}
            };
            batch.retain(|h|tried.insert((h.title.trim().to_lowercase(),h.artist.trim().to_lowercase())));
            if batch.is_empty(){break;}
            let terms=batch.iter().map(|h|format!("{} {}",h.artist,h.title)).collect();
            let ids=search_catalog(s,owner,Some(providers.to_vec()),terms,Duration::from_secs(14),usize::MAX).await?;
            self.ids.extend(ids.iter().cloned());
            let tracks=s.db.read(move|c|{
                let mut statement=c.prepare("SELECT * FROM tracks WHERE id=?1 AND available=1")?;
                let mut rows=Vec::new();
                for id in ids.into_iter().collect::<HashSet<_>>() { if let Some(track)=statement.query_row([id],Track::from_row).optional()? { rows.push(track); } }
                rows.sort_by(|a,b|a.id.cmp(&b.id)); Ok(rows)
            }).await?;
            let matches=tracks.iter().filter(|t|batch.iter().any(|h|nomination_matches(h,t)));
            self.playable.extend(verified_cloud(s,matches,Duration::from_secs(18),usize::MAX,usize::MAX).await);
            for h in batch {
                let matching:Vec<_>=tracks.iter().filter(|t|nomination_matches(&h,t)).collect();
                let ok=matching.iter().any(|t|self.playable.contains(&t.id));
                let nearby:Vec<_>=tracks.iter().filter(|t|same(&t.title,&h.title)||credits(t).any(|v|same(&h.artist,v)))
                    .take(3).map(|t|json!({"title":t.title,"artist":t.artist})).collect();
                history.push(json!({"title":h.title,"artist":h.artist,"outcome":if ok{"verified"}else if matching.is_empty(){"not found"}else{"not playable"},"catalogue":nearby}));
                if ok {self.hints.push(h);}
            }
            if self.hints.len()>=goal {break;}
        }
        Ok(())
    }
}

async fn generate(
    s: &AppState,
    a: &Auth,
    seed: Option<String>,
    day: &str,
    limit: usize,
) -> ApiResult<Vec<Pick>> {
    let policy = crate::accounts::policy(s, a).await?;
    let predicate = policy.as_ref().map_or_else(|| "1".into(), |p| p.sql("t"));
    let owner = a.id.clone();
    let seed_id = seed.clone();
    let scope = predicate.clone();
    let mut anchors =
        s.db.read(move |c| examples(c, &owner, seed_id.as_deref(), &scope))
            .await?;
    if seed.is_some() && anchors.is_empty() {
        return Err(ApiError::missing());
    }
    let providers = platforms(s,policy.as_ref()).await;
    if let Some(anchor) = anchors.first_mut() {
        fill_genre(s, anchor, &providers).await;
    }
    let genres = anchors.iter().filter_map(|t| feature(t, "genre")).collect();
    let owner = a.id.clone();
    let date = day.to_owned();
    let daily = seed.is_none();
    let (anchors, mut ids, previous, model, collaborators) = s.db.read(move |c| {
        let ids = metadata_candidates(c, &anchors, &predicate)?;
        let mut previous = HashSet::new();
        if daily {
            let mut stmt = c.prepare("SELECT json_extract(p.value,'$.recording') FROM personal_daily_lists d,json_each(d.picks) p WHERE d.user_id=?1 AND d.day>=date(?2,'-7 days') AND d.day<?2")?;
            previous = stmt.query_map(params![owner,date], |r| r.get::<_,String>(0))?.collect::<rusqlite::Result<_>>()?;
        }
        let names = collaborators(c, &anchors, &predicate)?;
        Ok((anchors, ids, previous, advanced(c)?, names))
    }).await?;
    let mut epochs = HashMap::new();
    for provider in &providers {
        epochs.insert(
            provider.clone(),
            crate::provider_session::start(s, provider).await?,
        );
    }
    let mut terms = Vec::new();
    let mut term = |v: String| {
        let short = v.trim().chars().take(144).collect::<String>();
        if !short.is_empty() && !terms.contains(&short) {
            terms.push(short);
        }
    };
    let online=!providers.is_empty();
    for t in anchors.iter().take(3) {
        if let Some(style) = feature(t, "genre") {
            term(style);
        }
        for artist in credits(t).take(2) {
            term(artist.to_owned());
        }
        if let Some(name) = collaborators.first() {
            term(name.clone());
        }
    }
    for name in collaborators.iter().skip(1) {
        term(name.clone());
    }
    if terms.is_empty() {
        terms.extend(["爵士", "民谣", "流行"].map(str::to_owned));
    }
    terms.truncate(8);
    let mut search=CatalogSearch::default();
    let propose=async {
        if online && model && policy.is_none() && !anchors.is_empty() {
            let examples=anchors.iter().map(|t|json!({"title":t.title,"artist":t.artist,"genre":feature(t,"genre")})).collect::<Vec<_>>();
            let goal=if daily {8} else {4};
            let _=tokio::time::timeout(Duration::from_secs(135),search.discover(s,&a.id,&providers,&examples,!daily,goal)).await;
        }
    };
    let catalogue=search_catalog(s,&a.id,Some(providers.clone()),terms,Duration::from_secs(12),usize::MAX);
    let ((),candidates)=tokio::join!(propose,catalogue);
    let mut candidates=candidates?;
    candidates.append(&mut search.ids);
    candidates.append(&mut ids);
    let mut seen = HashSet::new();
    candidates.retain(|id| seen.insert(id.clone()));
    let mut ranked = rank(
        s,
        Some(a.id.clone()),
        seed,
        vec![],
        limit * 3,
        RankOptions {
            cloud: candidates,
            skip_model: true,
            source: if online {Source::Cloud} else {Source::Local},
            personal: Some(Intent {
                hints:search.hints,
                previous,
                collaborators,
                genres,
            }),
            ..Default::default()
        },
    )
    .await?;
    ranked
        .items
        .retain(|track| track.source == "local" || epochs.contains_key(&track.source));
    if daily {ranked.items.sort_by(|a,b|ranked.scores[&b.id].total_cmp(&ranked.scores[&a.id]));}
    let mut to_verify: Vec<_> = ranked.items.iter().collect();
    to_verify.sort_by_key(|t| !ranked.reasons.get(&t.id).is_some_and(|r| r == MODEL_MATCH));
    to_verify.retain(|t|!search.playable.contains(&t.id));
    let mut playable=search.playable;
    let confirmed=ranked.items.iter().filter(|t|playable.contains(&t.id)).count();
    let needed=limit.saturating_sub(confirmed);
    if needed>0 {
        playable.extend(verified_cloud(s,to_verify,Duration::from_secs(if daily {125} else {22}),limit*3,needed).await);
    }
    let picks = ranked
        .items
        .iter()
        .filter(|t| {
            t.source == "local" || playable.contains(&t.id) && epochs.contains_key(&t.source)
        })
        .take(limit)
        .map(|t| Pick {
            id: t.id.clone(),
            recording: key(t).to_owned(),
            reason: ranked.reasons.remove(&t.id).unwrap_or_default(),
            epoch: epochs.get(&t.source).copied(),
            score: ranked.scores.get(&t.id).copied(),
        })
        .collect();
    Ok(picks)
}

async fn present(
    s: &AppState,
    a: &Auth,
    picks: Vec<Pick>,
    day: Option<String>,
) -> ApiResult<Value> {
    let policy = crate::accounts::policy(s, a).await?;
    let predicate = policy.as_ref().map_or_else(|| "1".into(), |p| p.sql("t"));
    let owner = a.id.clone();
    Ok(s.db.run(move|c|{
        let sql=format!("SELECT t.* FROM tracks t WHERE t.id=?1 AND t.available=1 AND {predicate} AND NOT EXISTS(SELECT 1 FROM recommendation_feedback f JOIN tracks other ON other.id=f.track_id WHERE f.user_id=?2 AND (f.track_id=t.id OR (t.recording_id<>'' AND other.recording_id=t.recording_id)))");
        let mut stmt=c.prepare(&sql)?;
        let mut items=Vec::new();let mut reasons=HashMap::new();let mut scores=HashMap::new();
        for pick in picks{
            let Some(t)=stmt.query_row(params![pick.id,owner],Track::from_row).optional()? else{continue};
            if let Some(epoch)=pick.epoch{if policy.as_ref().is_some_and(|p|!p.providers.contains(&t.source)) || crate::provider_session::check(c,&t.source,epoch).is_err(){continue;}}
            if let Some(score)=pick.score {scores.insert(t.id.clone(),score);}
            reasons.insert(t.id.clone(),pick.reason);
            items.push(t);
        }
        let mut response=json!({"date":day,"items":items,"reasons":reasons,"scores":scores});
        crate::media_assets::normalize_entries(c,&mut response["items"]);
        Ok(response)
    }).await?)
}

async fn local_day(s: &AppState) -> String {
    let tz = s
        .setting("statistics_timezone", json!(crate::stats::DEFAULT_TIMEZONE))
        .await;
    let tz = tz
        .as_str()
        .and_then(|name| name.parse::<chrono_tz::Tz>().ok())
        .unwrap_or(chrono_tz::Asia::Shanghai);
    Utc::now().with_timezone(&tz).date_naive().to_string()
}
async fn saved(s: &AppState, user: &str, day: &str) -> ApiResult<Option<Vec<Pick>>> {
    let (user, day) = (user.to_owned(), day.to_owned());
    Ok(s.db
        .read(move |c| {
            let raw = c
                .query_row(
                    "SELECT picks FROM personal_daily_lists WHERE user_id=?1 AND day=?2",
                    params![user, day],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
        })
        .await?)
}
async fn current_daily(s:&AppState,a:&Auth,day:&str)->ApiResult<Option<Vec<Pick>>> {
    let Some(picks)=saved(s,&a.id,day).await? else {return Ok(None)};
    if picks.iter().any(|p|p.score.is_none()) {return Ok(None);}
    if picks.iter().any(|p|p.epoch.is_none()) && !platforms(s,crate::accounts::policy(s,a).await?.as_ref()).await.is_empty() {return Ok(None);}
    let epochs = picks.iter().filter_map(|pick| pick.epoch.map(|epoch| (&pick.id, epoch))).collect::<Vec<_>>();
    if !epochs.is_empty() {
        let tracks = epochs.into_iter().map(|(id, epoch)| (id.clone(), epoch)).collect::<Vec<_>>();
        let current = s.db.read(move |c| {
            let mut source = c.prepare("SELECT source FROM tracks WHERE id=?1")?;
            for (id, epoch) in tracks {
                let provider = source.query_row([id], |r| r.get::<_, String>(0)).optional()?;
                if let Some(provider) = provider {
                    if crate::provider_session::check(c, &provider, epoch).is_ok() { return Ok(true); }
                }
            }
            Ok(false)
        }).await?;
        if !current { return Ok(None); }
    }
    Ok(Some(picks))
}
async fn daily_picks(s: &AppState, a: &Auth, day: &str) -> ApiResult<Vec<Pick>> {
    if let Some(picks)=current_daily(s,a,day).await? {return Ok(picks);}
    let lock = {
        let mut locks = s.daily_locks.lock().await;
        locks.retain(|_, v| v.strong_count() > 0);
        let entry = locks.entry(a.id.clone()).or_default();
        entry.upgrade().unwrap_or_else(|| {
            let lock = Arc::new(Mutex::new(()));
            *entry = Arc::downgrade(&lock);
            lock
        })
    };
    let _guard = lock.lock().await;
    if let Some(picks)=current_daily(s,a,day).await? {return Ok(picks);}
    let picks = generate(s, a, None, day, DAILY_SIZE).await?;
    if !picks.is_empty() {
        let raw = serde_json::to_string(&picks).map_err(anyhow::Error::from)?;
        let (user, date) = (a.id.clone(), day.to_owned());
        s.db.run(move |c| {
            c.execute(
                "INSERT INTO personal_daily_lists(user_id,day,picks) VALUES(?1,?2,?3) ON CONFLICT(user_id,day) DO UPDATE SET picks=excluded.picks",
                params![user, date, raw],
            )?;
            Ok(())
        })
        .await?;
        return Ok(saved(s, &a.id, day).await?.unwrap_or_default());
    }
    Ok(picks)
}
pub async fn daily(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let day = local_day(&s).await;
    let picks = daily_picks(&s, &a, &day).await?;
    Ok(Json(present(&s, &a, picks, Some(day)).await?))
}
#[derive(Deserialize)]
pub struct SimilarRequest {
    track_id: String,
}
pub async fn similar(
    State(s): State<AppState>,
    a: Auth,
    Json(q): Json<SimilarRequest>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    if q.track_id.is_empty() || q.track_id.len() > 256 {
        return Err(ApiError::bad("歌曲编号无效"));
    }
    let picks = generate(&s, &a, Some(q.track_id), &local_day(&s).await, 8).await?;
    Ok(Json(present(&s, &a, picks, None).await?))
}
pub async fn refresh_due(s: &AppState) {
    let day = local_day(s).await;
    let date = day.clone();
    let users=s.db.read(move|c|Ok(c.prepare("SELECT u.id,u.username,u.role FROM users u WHERE (u.role<>'external' OR EXISTS(SELECT 1 FROM external_accounts e WHERE e.user_id=u.id AND e.enabled=1)) AND NOT EXISTS(SELECT 1 FROM personal_daily_lists d WHERE d.user_id=u.id AND d.day=?1)")?.query_map([date],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await;
    let Ok(users) = users else { return };
    for (id, username, role) in users {
        let a = Auth {
            id,
            username,
            role,
            interactive: false,
            totp_enabled: false,
            session_hash: String::new(),
            scopes: vec!["read".into()],
        };
        let Ok(policy) = crate::accounts::policy(s, &a).await else {
            continue;
        };
        let task =
            crate::accounts::scope::REQUEST.scope((a.clone(), policy), daily_picks(s, &a, &day));
        if let Err(e) = crate::traffic::BACKGROUND.scope(true, task).await {
            tracing::debug!("personal daily list: {}", e.2);
        }
    }
}
