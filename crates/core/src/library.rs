use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    models::Track,
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use lofty::{
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::Accessor,
};
use pinyin::ToPinyin;
mod dsd;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::Path as FsPath, time::UNIX_EPOCH};

pub fn tokens(text: &str) -> String {
    static J: std::sync::OnceLock<jieba_rs::Jieba> = std::sync::OnceLock::new();
    let lower = text.to_lowercase();
    let mut parts: Vec<String> = J
        .get_or_init(jieba_rs::Jieba::new)
        .cut(&lower, false)
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut roman = String::new();
    let mut initials = String::new();
    for p in text.to_pinyin().flatten() {
        roman.push_str(p.plain());
        initials.push_str(&p.plain()[..1]);
    }
    if !roman.is_empty() {
        parts.push(roman);
        parts.push(initials);
    }
    let chars: Vec<char> = text.chars().filter(|c| !c.is_ascii()).collect();
    for ch in &chars {
        parts.push(ch.to_string());
    }
    for pair in chars.windows(2) {
        parts.push(pair.iter().collect());
    }
    parts.push(lower);
    parts.join(" ")
}
pub fn query_tokens(text: &str) -> String {
    text.split_whitespace()
        .map(|p| format!("\"{}\"*", p.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}
pub fn upsert(
    c: &rusqlite::Connection,
    t: &Track,
    path: Option<&str>,
    root: Option<&str>,
    mtime: i64,
    size: u64,
    scan: Option<&str>,
    lyrics: Option<&str>,
) -> anyhow::Result<()> {
    let protected_cover=c.query_row("SELECT json_extract(manual,'$.image_url') FROM catalog_profiles WHERE kind='track' AND entity_id=?1",[&t.id],|r|r.get::<_,Option<String>>(0)).optional()?.flatten();
    let enriched=if t.provider_data["provider_metadata"]["matched"]==true {t.provider_data["provider_metadata"]["track"]["cover_url"].as_str().filter(|v|v.starts_with("https://")).map(str::to_owned)}else{None};
    let specific=protected_cover.as_ref().or(t.cover_url.as_ref().filter(|v|!v.is_empty())).or(enriched.as_ref()).filter(|v|!v.is_empty()).map(|url|crate::media_assets::register(c,url).unwrap_or_else(|_|url.clone()));
    let mut provider_data=t.provider_data.clone();
    let cover_url=crate::album_artwork::map(c,t,specific,&mut provider_data,protected_cover.is_some())?;
    let album_id = crate::auth::hash(&format!("{}\0{}\0{}", t.source, t.album_artist, t.album));
    let artist_id = crate::auth::hash(&t.artist);
    if t.source=="local"{c.execute("DELETE FROM jobs WHERE kind='loudness' AND cursor=?1 AND status IN ('completed','unable','failed') AND EXISTS(SELECT 1 FROM tracks WHERE id=?1 AND (mtime!=?2 OR size!=?3))",params![t.id,mtime,size as i64])?;}
    c.execute("INSERT INTO tracks(id,path,root_id,title,artist,album,album_artist,duration,track_number,disc_number,source,format,sample_rate,bit_depth,channels,bitrate,cover_url,recording_id,available,created_at,mtime,size,scan_id,lyrics,provider_data,album_id,artist_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27) ON CONFLICT(id) DO UPDATE SET path=excluded.path,root_id=excluded.root_id,title=excluded.title,artist=excluded.artist,album=excluded.album,album_artist=excluded.album_artist,duration=excluded.duration,track_number=excluded.track_number,disc_number=excluded.disc_number,format=excluded.format,sample_rate=excluded.sample_rate,bit_depth=excluded.bit_depth,channels=excluded.channels,bitrate=excluded.bitrate,cover_url=excluded.cover_url,available=excluded.available,mtime=excluded.mtime,size=excluded.size,scan_id=excluded.scan_id,lyrics=excluded.lyrics,provider_data=excluded.provider_data,album_id=excluded.album_id,artist_id=excluded.artist_id",params![t.id,path,root,t.title,t.artist,t.album,t.album_artist,t.duration,t.track_number,t.disc_number,t.source,t.format,t.sample_rate,t.bit_depth,t.channels,t.bitrate,cover_url,t.recording_id,t.available,t.created_at,mtime,size as i64,scan,lyrics,serde_json::to_string(&provider_data)?,album_id,artist_id])?;
    c.execute(
        "DELETE FROM track_search WHERE rowid=(SELECT rowid FROM tracks WHERE id=?1)",
        [&t.id],
    )?;
    c.execute("INSERT INTO track_search(rowid,track_id,tokens) SELECT rowid,id,?2 FROM tracks WHERE id=?1",params![t.id,tokens(&format!("{} {} {}",t.title,t.artist,t.album))])?;
    c.execute(
        "INSERT OR IGNORE INTO recordings(id,title) VALUES(?1,?2)",
        params![t.recording_id, t.title],
    )?;
    c.execute(
        "INSERT OR IGNORE INTO releases(id,title,artist) VALUES(?1,?2,?3)",
        params![album_id, t.album, t.album_artist],
    )?;
    crate::music_identity::index(c,t,path)?;
    crate::artist_credits::index(c,t)?;
    crate::source_observation::file(c,t)?;
    crate::cached_artwork::refresh(c,t)?;
    if let Some(url)=cover_url.as_deref(){crate::artwork_recovery::bind(c,"track",&t.id,url)?;}
    c.execute(
        "INSERT OR IGNORE INTO metadata_queue(album_id) VALUES(?1)",
        [album_id],
    )?;
    Ok(())
}
#[derive(Deserialize, Default)]
pub struct Page {
    pub view: Option<String>,
    #[serde(flatten)]
    pub filters: crate::library_query::Filters,
    pub q: Option<String>,
    pub source: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u64>,
    pub album_id: Option<String>,
    pub artist_id: Option<String>,
    pub storage:Option<String>,
    pub folder:Option<String>,
}
// Provider-scoped release IDs keep distinct editions separate even if their captions match.
pub(crate) const RELEASE_ID:&str="CASE WHEN COALESCE(NULLIF(json_extract(t.provider_data,'$.release_id'),''),NULLIF(json_extract(t.provider_data,'$.album_mid'),'')) IS NOT NULL THEN 'release:'||CASE WHEN t.source='local' THEN COALESCE(json_extract(t.provider_data,'$.cached_from.source'),t.source) ELSE t.source END||':'||COALESCE(NULLIF(json_extract(t.provider_data,'$.release_id'),''),json_extract(t.provider_data,'$.album_mid')) ELSE t.album_id END";
pub(crate) const COLLECTION_VISIBILITY:&str="(t.source='local' OR COALESCE(json_extract(t.provider_data,'$.catalog_browse_only'),0)!=1)";
// Shared browse policy, applied before count/sort/pagination. Recording identities
// are explicit; blank identities never match. Owned caches may replace only
// their exact source track, not another platform/release with the same caption.
pub(crate) const PREFERRED_COPY:&str="(t.source='local' OR t.recording_id='' OR NOT EXISTS(SELECT 1 FROM tracks l WHERE l.recording_id=t.recording_id AND l.source='local' AND l.available=1 AND (json_type(l.provider_data,'$.cached_from') IS NULL OR (json_extract(l.provider_data,'$.cached_from.track_id')=t.id AND json_extract(l.provider_data,'$.cached_from.source')=t.source)))) AND (t.source!='local' OR json_type(t.provider_data,'$.cached_from.cache_id') IS NULL OR t.recording_id='' OR t.id=(SELECT l.id FROM tracks l WHERE l.source='local' AND l.available=1 AND l.recording_id=t.recording_id AND json_extract(l.provider_data,'$.cached_from.track_id')=json_extract(t.provider_data,'$.cached_from.track_id') AND json_extract(l.provider_data,'$.cached_from.source')=json_extract(t.provider_data,'$.cached_from.source') ORDER BY (lower(l.format) IN ('flac','wav','aiff','alac','dsf','dff')) DESC,l.bit_depth DESC,l.sample_rate DESC,l.id LIMIT 1))";

pub(crate) fn track_filter(p: &Page) -> ApiResult<(String, Vec<rusqlite::types::Value>, bool)> {
    let q = p.q.as_deref().unwrap_or_default();
    if q.len() > 500 { return Err(ApiError::bad("搜索词过长")); }
    let mut condition = format!("t.available=1 AND {}",crate::accounts::scope::predicate("t"));
    if p.album_id.as_deref().unwrap_or("").is_empty()&&p.artist_id.as_deref().unwrap_or("").is_empty(){condition.push_str(" AND ");condition.push_str(COLLECTION_VISIBILITY);}
    let mut args = Vec::new();
    if !crate::accounts::scope::current().is_some_and(|(_,p)|p.is_some()) && p.source.as_deref().unwrap_or_default().is_empty() && p.storage.as_deref().unwrap_or_default().is_empty() {
        condition.push_str(" AND ");condition.push_str(PREFERRED_COPY);
    }
    for (column, value) in [("source", &p.source), ("album_id", &p.album_id), ("artist_id", &p.artist_id)] {
        if let Some(value) = value.as_ref().filter(|s| !s.is_empty()) {
            if column=="artist_id" {condition.push_str(" AND (t.artist_id=? OR EXISTS(SELECT 1 FROM track_artists credit WHERE credit.track_id=t.id AND credit.artist_id=?))");args.push(value.clone().into());}
            else if column=="album_id"&&value.starts_with("release:"){condition.push_str(&format!(" AND ({RELEASE_ID})=?"));}
            else {condition.push_str(&format!(" AND t.{column}=?"));}
            args.push(value.clone().into());
        }
    }
    match p.storage.as_deref().unwrap_or("") {
        ""=>{},
        "local"=>condition.push_str(" AND t.source='local' AND t.path IS NOT NULL AND json_type(t.provider_data,'$.cached_from.cache_id') IS NULL"),
        "cache"=>condition.push_str(" AND EXISTS(SELECT 1 FROM cache_entries e WHERE e.complete=1 AND (e.path=t.path OR (e.track_id=t.id AND NOT EXISTS(SELECT 1 FROM tracks l WHERE l.available=1 AND l.path=e.path))))"),
        _=>return Err(ApiError::bad("库类型无效")),
    }
    if let Some(folder)=p.folder.as_ref().filter(|v|!v.is_empty()) {condition.push_str(" AND t.path LIKE ? ESCAPE '\\'");args.push(format!("{}%",folder.replace('\\',"\\\\").replace('%',"\\%").replace('_',"\\_")).into());condition.push_str(" AND instr(substr(t.path,length(?)+1),'/')=0 AND instr(substr(t.path,length(?)+1),char(92))=0");args.push(folder.clone().into());args.push(folder.clone().into());}
    let searching = !q.trim().is_empty();
    if searching {
        condition.push_str(" AND track_search MATCH ?");
        args.push(query_tokens(q).into());
    }
    Ok((condition, args, searching))
}
pub async fn tracks(
    State(s): State<AppState>,
    a: Auth,
    Query(p): Query<Page>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let (mut condition, mut args, searching) = track_filter(&p)?;
    crate::library_query::predicates(&p.filters,&a.id,&mut condition,&mut args)?;
    let order=crate::accounts::scope::stats_sql(&crate::library_query::order(p.filters.sort.as_deref(),false,if searching{"rank"}else{"t.title"})?);
    condition=crate::accounts::scope::stats_sql(&condition);
    let limit = p.limit.unwrap_or(50).clamp(1, 200);
    let offset = p.offset.unwrap_or(0).min(10_000_000);
    let compact = p.view.as_deref()==Some("compact");
    let v=s.db.read(move|c|{
        // Only emit requested predicates so SQLite can use covering indexes for
        // unfiltered counts instead of fetching every row's optional columns.
        let from=if searching{"track_search CROSS JOIN tracks t ON t.rowid=track_search.rowid"}else{"tracks t"};
        let total:i64=c.query_row(&format!("SELECT COUNT(*) FROM {from} WHERE {condition}"),rusqlite::params_from_iter(args.clone()),|r|r.get(0))?;
        let sql=format!("SELECT t.* FROM {from} WHERE {condition} ORDER BY {order} LIMIT ? OFFSET ?");args.push(limit.into());args.push((offset as i64).into());
        let items=c.prepare(&sql)?.query_map(rusqlite::params_from_iter(args),Track::from_row)?.collect::<Result<Vec<_>,_>>()?;
        let mut items=if compact {json!(items.iter().map(crate::zones::snapshots::track).collect::<Vec<_>>())}else{serde_json::to_value(items)?};
        crate::catalog_order::decorate(c,&mut items)?;
        Ok(json!({"items":items,"total":total}))
    }).await?;
    Ok(Json(v))
}
pub async fn get_track(s: &AppState, id: String) -> ApiResult<Track> {
    crate::accounts::scope::current_track(s,&id).await?;
    crate::library_files::validate_cached(s,&id).await?;
    let requested=id.clone();
    let track=s.db.run(move |c| {
        let mut current=id.clone();for _ in 0..16{if let Some(next)=c.query_row("SELECT new_id FROM track_redirects WHERE old_id=?1",[&current],|r|r.get::<_,String>(0)).optional()?{current=next;}else{break;}}
        let value=c.query_row("SELECT * FROM tracks WHERE id=?1",[&current],Track::from_row).optional()?.map(|mut t|{if current!=id{t.id=id.clone();}t});
        let Some(mut track)=value else{return Ok(None);};crate::cached_artwork::restore_source(c,&mut track)?;if track.available{return Ok(Some(track));}
        if track.source=="local" {if let Some(original)=track.provider_data["cached_from"]["track_id"].as_str(){
            if let Some(mut source)=c.query_row("SELECT * FROM tracks WHERE id=?1 AND available=1 AND source!='local'",[original],Track::from_row).optional()?{source.id=id;source.provider_data["cached_from"]=track.provider_data["cached_from"].clone();return Ok(Some(source));}
        }}Ok(if track.source=="local"{Some(track)}else{None})
    }).await?.ok_or_else(ApiError::missing)?;
    if track.source=="local"{let key=requested;let(path,root)=s.db.run(move|c|{let mut key=key;for _ in 0..16{if let Some(next)=c.query_row("SELECT new_id FROM track_redirects WHERE old_id=?1",[&key],|r|r.get::<_,String>(0)).optional()?{key=next;}else{break;}}Ok(c.query_row("SELECT t.path,r.path FROM tracks t LEFT JOIN roots r ON r.id=t.root_id WHERE t.id=?1",[key],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?)))?)}).await?;
        let path=path.ok_or_else(||ApiError(axum::http::StatusCode::GONE,"media_missing".into(),"歌曲只有资料，尚无本地音频文件；请重新定位或导入文件，歌单和聆听记录已保留".into()))?;
        if !std::path::Path::new(&path).is_file(){if root.as_ref().is_some_and(|r|!std::path::Path::new(r).is_dir()){return Err(ApiError(axum::http::StatusCode::SERVICE_UNAVAILABLE,"library_offline".into(),"音乐目录离线或尚未挂载，请恢复连接；曲库记录已保留".into()));}let missing_path=path.clone();s.db.run(move|c|{c.execute("UPDATE tracks SET available=0 WHERE path=?1",[missing_path])?;Ok(())}).await?;return Err(ApiError(axum::http::StatusCode::GONE,"media_missing".into(),"音乐文件已丢失，请在库管理中检查文件、重新定位或恢复回收站；歌单和聆听记录已保留".into()));}
    }Ok(track)
}
pub(crate) async fn local_path(s:&AppState,id:String)->ApiResult<String>{
 Ok(s.db.run(move|c|{let mut current=id;for _ in 0..16{if let Some(next)=c.query_row("SELECT new_id FROM track_redirects WHERE old_id=?1",[&current],|r|r.get::<_,String>(0)).optional()?{current=next;}else{break;}}Ok(c.query_row("SELECT path FROM tracks WHERE id=?1 AND source='local' AND available=1",[current],|r|r.get::<_,String>(0))?)}).await?)
}
pub async fn track(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Track>> {
    a.require("read")?;
    Ok(Json(get_track(&s, id).await?))
}
pub async fn albums(
    State(s): State<AppState>,
    a: Auth,
    Query(p): Query<Page>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    group(&s, p, true, &a.id).await.map(Json)
}
pub async fn artists(
    State(s): State<AppState>,
    a: Auth,
    Query(p): Query<Page>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    group(&s, p, false, &a.id).await.map(Json)
}
async fn group(s: &AppState, p: Page, albums: bool, user: &str) -> ApiResult<Value> {
    let (mut condition, mut args, searching) = track_filter(&p)?;
    crate::library_query::predicates(&p.filters,user,&mut condition,&mut args)?;
    condition=crate::accounts::scope::stats_sql(&condition);
    let plays=crate::accounts::scope::stats_sql(crate::library_query::PLAYS);
    let requested_sort=p.filters.sort.clone();
    let unassigned_condition=condition.clone();
    if albums {condition.push_str(&format!(" AND {}",crate::library_query::KNOWN_ALBUM));}
    let v=s.db.read(move|c|{
        let field = if albums {RELEASE_ID} else {"credit.artist_id"};
        let from = if searching {"track_search CROSS JOIN tracks t ON t.rowid=track_search.rowid"} else {"tracks t"};
        let from=if albums{from.to_owned()}else{format!("{from} JOIN track_artists credit ON credit.track_id=t.id")};
        let unassigned_total=if albums{c.query_row(&format!("SELECT COUNT(*) FROM {from} WHERE {unassigned_condition} AND NOT ({})",crate::library_query::KNOWN_ALBUM),rusqlite::params_from_iter(args.clone()),|r|r.get::<_,i64>(0))?}else{0};

        let total:i64 = c.query_row(&format!("SELECT COUNT(DISTINCT {field}) FROM {from} WHERE {condition}"),rusqlite::params_from_iter(args.clone()),|r|r.get(0))?;
        let (columns, join, order) = if albums {
            (format!("{field},COALESCE(NULLIF(TRIM(t.album),''),'未知专辑'),COALESCE(NULLIF(TRIM(t.album_artist),''),'未知艺术家'),MAX(t.cover_url),COUNT(*)"), "", "t.album,1")
        } else {
            ("credit.artist_id,COALESCE(NULLIF(TRIM(credit.name),''),'未知艺术家'),COUNT(DISTINCT t.id),MAX(json_extract(m.data,'$.image_url')),MAX(json_extract(m.data,'$.description')),MIN(t.source)".to_owned(), " LEFT JOIN artist_metadata m ON m.artist_id=credit.artist_id OR (m.artist_id=t.artist_id AND credit.name=t.artist)", "credit.name,credit.artist_id")
        };
        let order=crate::library_query::order(requested_sort.as_deref(),true,order).map_err(|e|anyhow::anyhow!(e.2))?.replace(crate::library_query::PLAYS,&plays).replace("t.title",if albums{"t.album"}else{"credit.name"});
        let columns=format!("{columns},MAX({}),SUM({})",crate::library_query::RELEASE,plays);
        let sql = format!("SELECT {columns} FROM {from}{join} WHERE {condition} GROUP BY {field} ORDER BY {order} LIMIT ? OFFSET ?");
        args.push(p.limit.unwrap_or(50).clamp(1,200).into());
        args.push((p.offset.unwrap_or(0).min(10_000_000) as i64).into());
        let rows=c.prepare(&sql)?.query_map(rusqlite::params_from_iter(args),|r|{Ok(if albums{
            json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"artist":r.get::<_,String>(2)?,"cover_url":r.get::<_,Option<String>>(3)?,"track_count":r.get::<_,i64>(4)?,"release_date":r.get::<_,String>(5)?,"play_count":r.get::<_,i64>(6)?})
        }else{
            json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"track_count":r.get::<_,i64>(2)?,"image_url":r.get::<_,Option<String>>(3)?,"description":r.get::<_,Option<String>>(4)?,"source":r.get::<_,String>(5)?,"release_date":r.get::<_,String>(6)?,"play_count":r.get::<_,i64>(7)?})
        })})?.collect::<Result<Vec<_>,_>>()?;
        Ok(json!({"items":rows,"total":total,"unassigned_total":unassigned_total}))
    }).await?;
    Ok(v)
}
pub async fn roots(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("library")?;
    let v =
        s.db.run(|c| {
            let mut q = c.prepare("SELECT id,path FROM roots")?;
            let rows = q
                .query_map([], |r| {
                    Ok(json!({"id":r.get::<_,String>(0)?,"path":r.get::<_,String>(1)?}))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(json!({"items":v})))
}
#[derive(Deserialize)]
pub struct Root {
    path: String,
}
pub async fn add_root(
    State(s): State<AppState>,
    a: Auth,
    Json(v): Json<Root>,
) -> ApiResult<Json<Value>> {
    a.require("library")?;
    let path = tokio::fs::canonicalize(&v.path)
        .await
        .map_err(|_| ApiError::bad("目录不存在或核心没有访问权限；Docker 中请填写容器挂载路径"))?;
    if !path.is_dir() {
        return Err(ApiError::bad("请选择目录"));
    }
    let path = path.to_string_lossy().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let i = id.clone();
    let p = path.clone();
    let id=s.db.run(move |c| {
        c.execute("INSERT OR IGNORE INTO roots(id,path) VALUES(?1,?2)", params![i, p])?;
        Ok(c.query_row("SELECT id FROM roots WHERE path=?1",[p],|r|r.get::<_,String>(0))?)
    }).await?;
    Ok(Json(json!({"id":id,"path":path})))
}
pub async fn delete_root(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    a.require("library")?;
    s.db.run(move |c| {
        c.execute("DELETE FROM roots WHERE id=?1", [&id])?;
        c.execute("UPDATE tracks SET available=0 WHERE root_id=?1", [id])?;
        Ok(())
    })
    .await?;
    s.emit("library", json!({}));
    Ok(StatusCode::NO_CONTENT)
}
pub async fn scan(State(s):State<AppState>,a:Auth)->ApiResult<Json<Value>> {
    a.require("library")?;
    let id=crate::background::enqueue(&s,"scan","library",json!({}),0).await?;
    crate::background::spawn(s.clone());
    Ok(Json(json!({"id":id,"status":"pending"})))
}
#[derive(Debug)] struct ScanPaused;
impl std::fmt::Display for ScanPaused{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str("扫描已暂停")}}
impl std::error::Error for ScanPaused{}
pub(crate) async fn run_scan_job(s:&AppState,id:&str)->anyhow::Result<bool> {
    let Ok(_scanner)=s.scanner.try_lock()else{return Ok(false);};
    let state=s.clone();let job=id.to_owned();let result=tokio::task::spawn_blocking(move||scan_sync(&state,&job)).await?;
    if result.as_ref().is_err_and(|e|e.is::<ScanPaused>()){return Ok(false);}
    result?;s.emit("library",json!({}));s.refresh_queue_metadata().await?;Ok(true)
}
fn directory_stamp(path:&FsPath)->std::io::Result<(u64,u64)> {
    let metadata=std::fs::metadata(path)?;
    #[cfg(unix)] {use std::os::unix::fs::MetadataExt;Ok((metadata.dev(),metadata.ino()))}
    #[cfg(not(unix))] {Ok((metadata.created()?.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64,0))}
}
fn scan_sync(s: &AppState, job: &str) -> anyhow::Result<()> {
    let roots = {
        let c =
            s.db.0
                .lock()
                .map_err(|_| anyhow::anyhow!("lock poisoned"))?;
        let mut q = c.prepare("SELECT id,path FROM roots")?;
        let v = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        v
    };
    let mut processed = 0;
    let mut failures = 0;
    for (root_id, root) in roots {
        if !FsPath::new(&root).is_dir() {
            anyhow::bail!("曲库目录离线：{root}");
        }
        let mut root_seen=0;
        let root_stamp=directory_stamp(FsPath::new(&root))?;
        let mut complete = true;
        for entry in walkdir::WalkDir::new(&root).follow_links(false).into_iter().filter_entry(|e|e.file_name()!=".rmusic-trash") {
            if s.db.is_closed(){return Err(ScanPaused.into());}
            let e = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error=%e,"scan directory error");
                    complete = false;
                    continue;
                }
            };
            if !e.file_type().is_file() {
                continue;
            }
            let path = e.path();
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if ![
                "flac", "mp3", "m4a", "aac", "ogg", "opus", "wav", "aiff", "aif", "ape", "wv",
                "dsf", "dff",
            ]
            .contains(&ext.as_str())
            {
                continue;
            }
            loop {
                let status: String = s.db.0.lock().unwrap().query_row(
                    "SELECT status FROM jobs WHERE id=?1",
                    [job],
                    |r| r.get(0),
                )?;
                if status == "paused"||s.resources.over_budget() {
                    return Err(ScanPaused.into());
                }
                if status == "cancelled" {
                    anyhow::bail!("扫描已取消");
                }
                break;
            }
            let meta = e.metadata()?;
            let mut mtime = meta
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as i64;
            let mut companions=vec![path.with_extension("lrc")];
            if let Some(parent)=path.parent(){for name in ["cover.jpg","folder.jpg","cover.png","folder.png"]{companions.push(parent.join(name));}}
            for companion in companions{if let Ok(time)=companion.metadata().and_then(|m|m.modified()){mtime=mtime.max(time.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()as i64);}}
            let size = meta.len();
            let canonical = std::fs::canonicalize(path)?;
            let filepath = canonical.to_string_lossy().to_string();
            // An explicit migration can relocate the path while retaining playlists and history.
            let id = s.db.0.lock().unwrap().query_row("SELECT id FROM tracks WHERE path=?1",[&filepath],|r|r.get::<_,String>(0)).optional()?.unwrap_or_else(||crate::auth::hash(&filepath));
            let unchanged = {
                let c = s.db.0.lock().unwrap();
                c.query_row(
                    "SELECT mtime=?2 AND size=?3 AND NOT (format IN ('dsf','dff') AND (sample_rate=0 OR bit_depth=0 OR channels=0 OR duration=0 OR json_extract(provider_data,'$.dsd_metadata_v1') IS NOT 1 OR (format='dff' AND json_extract(provider_data,'$.dsd_diin_v1') IS NOT 1))) FROM tracks WHERE id=?1",
                    params![id, mtime, size as i64],
                    |r| r.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false)
            };
            if unchanged {
                s.db.0.lock().unwrap().execute(
                    "UPDATE tracks SET scan_id=?1,available=1 WHERE id=?2",
                    params![job, id],
                )?;
            } else {
                match probe_track(path, &id, &ext, &s.config.data) {
                    Ok((track, lyrics)) => {
                        let mut c = s.db.0.lock().unwrap();
                        let tx=c.transaction()?;
                        upsert(
                            &tx,
                            &track,
                            Some(&filepath),
                            Some(&root_id),
                            mtime,
                            size,
                            Some(job),
                            lyrics.as_deref(),
                        )?;
                        tx.commit()?;
                    }
                    Err(e) => {
                        failures += 1;
                        tracing::warn!(path=%path.display(),error=%e,"audio metadata unavailable");
                        let track = fallback_track(path, id.clone(), ext.clone());
                        let mut c = s.db.0.lock().unwrap();
                        let tx=c.transaction()?;
                        upsert(
                            &tx,
                            &track,
                            Some(&filepath),
                            Some(&root_id),
                            mtime,
                            size,
                            Some(job),
                            None,
                        )?;
                        tx.commit()?;
                    }
                }
            }
            processed += 1;
            root_seen+=1;
            if processed % 50 == 0 {
                s.db.0.lock().unwrap().execute(
                    "UPDATE jobs SET processed=?1,cursor=?2,phase=?2,updated_at=unixepoch() WHERE id=?3",
                    params![processed, filepath, job],
                )?;
                s.emit("jobs", json!({"id":job,"processed":processed}));
            }
            if processed % 25 == 0 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        if directory_stamp(FsPath::new(&root))?!=root_stamp {anyhow::bail!("曲库目录挂载发生变化，保留未扫描歌曲，请恢复挂载后重试");}
        if complete&&root_seen==0 {
            let prior:i64=s.db.0.lock().unwrap().query_row("SELECT COUNT(*) FROM tracks WHERE root_id=?1 AND available=1",[&root_id],|r|r.get(0))?;
            if prior>0 {anyhow::bail!("曲库目录为空，可能尚未挂载；已保留 {prior} 首歌曲的状态，请检查挂载后重试");}
        }
        if complete {
            s.db.0.lock().unwrap().execute("UPDATE tracks SET available=0 WHERE root_id=?1 AND (scan_id IS NULL OR scan_id<>?2)",params![root_id,job])?;
        }
    }
    s.db.0.lock().unwrap().execute(
        "UPDATE jobs SET processed=?1,total=?1,error=?2 WHERE id=?3",
        params![
            processed,
            if failures > 0 {
                Some(format!("{failures} 个文件未能解析标签，已按文件名导入"))
            } else {
                None
            },
            job
        ],
    )?;
    Ok(())
}
fn fallback_track(path: &FsPath, id: String, format: String) -> Track {
    Track {
        id: id.clone(),
        title: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        artist: "未知艺术家".into(),
        album: path
            .parent()
            .and_then(|p| p.file_name())
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        album_artist: "未知艺术家".into(),
        duration: 0.,
        track_number: 0,
        disc_number: 0,
        source: "local".into(),
        format,
        sample_rate: 0,
        bit_depth: 0,
        channels: 0,
        bitrate: 0,
        cover_url: None,
        recording_id: format!("local:{id}"),
        available: true,
        created_at: chrono::Utc::now().timestamp(),
        integrated_lufs: None,
        provider_data: json!({}),
    }
}
pub(crate) fn probe_track(
    path: &FsPath,
    id: &str,
    format: &str,
    data: &FsPath,
) -> anyhow::Result<(Track, Option<String>)> {
    let mut dsd_tag=None;
    let (mut t,file)=if matches!(format,"dsf"|"dff") {
        let mut source=std::fs::File::open(path)?;
        let layout=rmusic_audio::dsd::DsdLayout::read(&mut source)?;
        if source.metadata()?.len()<layout.data_end{anyhow::bail!("truncated DSD programme");}
        match dsd::tag(&mut source,&layout){Ok(tag)=>dsd_tag=tag,Err(error)=>tracing::warn!(path=%path.display(),%error,"DSD embedded tag unavailable; keeping audio properties")}
        let mut t=fallback_track(path,id.into(),format.into());
        t.duration=layout.duration();t.sample_rate=layout.sample_rate;t.bit_depth=1;t.channels=layout.channels as u32;
        t.bitrate=layout.sample_rate*t.channels;
        t.provider_data["dsd_metadata_v1"]=json!(true);
        if format=="dff"{t.provider_data["dsd_diin_v1"]=json!(true);}
        (t,None)
    }else{
        let file=Probe::open(path)?.read()?;let props=file.properties();
        let mut t=fallback_track(path,id.into(),format.into());
        t.duration=props.duration().as_secs_f64();t.sample_rate=props.sample_rate().unwrap_or(0);
        t.bit_depth=props.bit_depth().unwrap_or(0) as u32;t.channels=props.channels().unwrap_or(0) as u32;
        t.bitrate=props.audio_bitrate().unwrap_or(0)*1000;
        (t,Some(file))
    };
    let tag=dsd_tag.as_ref().or_else(||file.as_ref().and_then(|file|file.primary_tag().or_else(||file.first_tag())));
    let mut lyrics = None;
    if let Some(tag) = tag {
        if let Some(v) = tag.title() {
            t.title = v.into_owned();
        }
        if let Some(v) = tag.artist() {
            t.artist = v.into_owned();
        }
        let artists:Vec<_>=tag.get_strings(&lofty::tag::ItemKey::TrackArtist).filter(|v|!v.trim().is_empty()).take(64).collect();
        if artists.len()>1 {t.artist=artists.join(" / ");t.provider_data["artist_credits"]=json!(artists.iter().map(|name|json!({"name":name})).collect::<Vec<_>>());}
        if let Some(v) = tag.album() {
            t.album = v.into_owned();
            t.provider_data["identity"]["album_tagged"]=json!(true);
        }
        t.album_artist = tag
            .get_string(&lofty::tag::ItemKey::AlbumArtist)
            .unwrap_or(&t.artist)
            .to_string();
        t.track_number = tag.track().unwrap_or(0);
        t.disc_number = tag.disk().unwrap_or(0);
        for (key,field) in [(lofty::tag::ItemKey::MusicBrainzWorkId,"work"),(lofty::tag::ItemKey::MusicBrainzRecordingId,"recording"),(lofty::tag::ItemKey::MusicBrainzReleaseId,"release"),(lofty::tag::ItemKey::MusicBrainzArtistId,"artist"),(lofty::tag::ItemKey::MusicBrainzReleaseArtistId,"release_artist")] {
            let ids=tag.get_strings(&key).flat_map(|s|s.split([';', '/', '\0'])).filter_map(|s|uuid::Uuid::parse_str(s.trim()).ok()).map(|id|id.to_string()).take(32).collect::<Vec<_>>();
            if !ids.is_empty(){t.provider_data["identifiers"]["musicbrainz"][field]=json!(ids);}
        }
        if let Some(isrc)=tag.get_string(&lofty::tag::ItemKey::Isrc){t.provider_data["identifiers"]["isrc"]=json!(isrc);}
        for (key,role) in [(lofty::tag::ItemKey::Composer,"composer"),(lofty::tag::ItemKey::Conductor,"conductor"),(lofty::tag::ItemKey::Lyricist,"lyricist"),(lofty::tag::ItemKey::Producer,"producer"),(lofty::tag::ItemKey::Engineer,"engineer")] {
            let credits=tag.get_strings(&key).filter(|s|!s.trim().is_empty()&&s.len()<=500).take(32).collect::<Vec<_>>();if !credits.is_empty(){t.provider_data["credits"][role]=json!(credits);}
        }
        lyrics = tag
            .get_string(&lofty::tag::ItemKey::Lyrics)
            .map(String::from);
        if let Some(pic) = tag.pictures().first() {
            if pic.data().len() <= 10 * 1024 * 1024 {
                t.cover_url = crate::artwork_maintenance::embedded(data,pic.data())?;
            }
        }
    }
    if t.cover_url.is_none() {
        if let Some(parent) = path.parent() {
            for name in ["cover.jpg", "folder.jpg", "cover.png", "folder.png"] {
                let p = parent.join(name);
                if let Ok(m) = p.metadata() {
                    if m.len() <= 10 * 1024 * 1024 {
                        let bytes = std::fs::read(p)?;
                        t.cover_url = crate::artwork_maintenance::embedded(data,&bytes)?;
                        break;
                    }
                }
            }
        }
    }
    if let Ok(text) = std::fs::read_to_string(path.with_extension("lrc")) {
        if text.len() <= 1024 * 1024 {
            lyrics = Some(text);
        }
    }
    Ok((t, lyrics))
}
pub async fn lyrics(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    a.require("read")?;
    let t = get_track(&s, id.clone()).await?;
    Ok(Json(crate::lyric_cache::get(s,t).await?))
}
pub(crate) async fn fetch_lyrics(s:AppState,t:Track)->ApiResult<Value> {
    let id=t.id.clone();
    if t.source != "local" {
        return Ok(
            s.plugin
                .call(
                    &t.source,
                    "lyrics",
                    json!({"track_id":t.provider_data["cached_from"]["track_id"].as_str().unwrap_or(&id),"track":t,"provider_data":t.provider_data}),
                )
                .await?,
        );
    }
    let mut text =
        s.db.run(move |c| {
            Ok(
                c.query_row("SELECT lyrics FROM tracks WHERE id=?1", [id], |r| {
                    r.get::<_, Option<String>>(0)
                })?,
            )
        })
        .await?
        .unwrap_or_default();
    if let (Some(provider),Some(original))=(t.provider_data["cached_from"]["source"].as_str(),t.provider_data["cached_from"]["track_id"].as_str()){
        let mut document=s.plugin.call(provider,"lyrics",json!({"track_id":original})).await?;if !text.is_empty() && !crate::lyric_cache::is_qrc_container(&text) { document["text"]=json!(text); }return Ok(document);
    }
    let mut source="local";
    if text.is_empty()&&crate::connectors::metadata_provider(&s,&t.source).await.is_ok() {
        if let Ok(metadata)=crate::catalog::metadata_for_track(&s,&t).await {
            if metadata["matched"]==true {text=metadata["lyrics"].as_str().unwrap_or("").to_owned();source="provider";}
        }
    }
    Ok(
        json!({"format":if text.contains('['){"lrc"}else{"plain"},"text":text,"offset_ms":0,"source":source}),
    )
}
#[derive(Deserialize)] pub struct JobAction {status:String}
pub async fn job_action(State(s):State<AppState>,a:Auth,Path(id):Path<String>,Json(v):Json<JobAction>)->ApiResult<StatusCode> {
    a.require("library")?;
    if !["paused","running","cancelled","retry","priority"].contains(&v.status.as_str()){return Err(ApiError::bad("无效任务操作"));}
    let changed=s.db.run(move|c|{
        if let Some(key)=id.strip_prefix("cache-export:") {let status=match v.status.as_str(){"running"|"retry"=>"pending","priority"=>return Ok(0),v=>v};return Ok(c.execute("UPDATE cache_exports SET status=?2,error=NULL,updated_at=unixepoch() WHERE cache_id=?1 AND status!='ready'",params![key,status])?);}
        if v.status=="priority"{return Ok(c.execute("UPDATE jobs SET priority=1000,updated_at=unixepoch() WHERE id=?1 AND status NOT IN ('completed','cancelled','failed','unable')",[id])?);}
        let status=match v.status.as_str(){"running"|"retry"=>"pending",v=>v};
        // Pause/cancel/resume a repair as a unit; completed child transfers remain immutable.
        if c.query_row("SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND kind='cache_repair')",[&id],|r|r.get::<_,bool>(0))? {
            c.execute("UPDATE jobs SET status=?2,error=NULL,next_try=0,payload=json_set(payload,'$.manual_pause',?3),updated_at=unixepoch() WHERE kind='cache_fill' AND json_extract(payload,'$.repair_job')=?1 AND status!='completed'",params![&id,status,matches!(status,"paused"|"cancelled")])?;
        }
        Ok(c.execute("UPDATE jobs SET status=?2,error=NULL,next_try=0,updated_at=unixepoch(),payload=json_set(payload,'$.manual_pause',?3) WHERE id=?1 AND status!='completed'",params![id,status,matches!(status,"paused"|"cancelled")])? )
    }).await?;
    if changed==0{return Err(ApiError::bad("任务已结束或不支持此操作"));}s.emit("jobs",json!({}));Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dsd_scan_preserves_true_bitclock_duration_and_sidecar_lyrics() {
        let directory=tempfile::tempdir().unwrap();
        let mut dsf=b"DSD ".to_vec();dsf.extend(28_u64.to_le_bytes());dsf.extend(8284_u64.to_le_bytes());dsf.extend(0_u64.to_le_bytes());
        dsf.extend(b"fmt ");dsf.extend(52_u64.to_le_bytes());
        for value in [1_u32,0,2,2,11289600,1]{dsf.extend(value.to_le_bytes());}
        dsf.extend(32767_u64.to_le_bytes());dsf.extend(4096_u32.to_le_bytes());dsf.extend(0_u32.to_le_bytes());
        dsf.extend(b"data");dsf.extend(8204_u64.to_le_bytes());dsf.extend([0x96;8192]);
        let chunk=|kind:&[u8;4],value:&[u8]|{let mut bytes=kind.to_vec();bytes.extend((value.len() as u64).to_be_bytes());bytes.extend(value);if value.len()%2==1{bytes.push(0);}bytes};
        let mut prop=b"SND ".to_vec();prop.extend(chunk(b"FS  ",&5644800_u32.to_be_bytes()));prop.extend(chunk(b"CHNL",b"\0\x02SLFTSRGT"));prop.extend(chunk(b"CMPR",b"DSD \x03raw"));
        let mut form=b"DSD ".to_vec();form.extend(chunk(b"PROP",&prop));form.extend(chunk(b"DSD ",&[0x69;8]));
        let mut dff=b"FRM8".to_vec();dff.extend((form.len() as u64).to_be_bytes());dff.extend(form);
        for (format,bytes,rate,samples) in [("dsf",dsf.clone(),11289600,32767),("dff",dff,5644800,32)] {
            let path=directory.path().join(format!("recording.{format}"));std::fs::write(&path,bytes).unwrap();std::fs::write(path.with_extension("lrc"),"[00:00.00]原始歌词").unwrap();
            let (track,lyrics)=probe_track(&path,"stable-track",format,directory.path()).unwrap();
            assert_eq!(track.id,"stable-track");assert_eq!(track.format,format);assert_eq!(track.sample_rate,rate);assert_eq!(track.bit_depth,1);assert_eq!(track.channels,2);assert_eq!(track.bitrate,rate*2);
            assert_eq!(track.duration,samples as f64/rate as f64);assert_eq!(lyrics.as_deref(),Some("[00:00.00]原始歌词"));
        }
        dsf[60..64].copy_from_slice(&24_u32.to_le_bytes());let path=directory.path().join("invalid.dsf");std::fs::write(&path,dsf).unwrap();
        assert!(probe_track(&path,"invalid","dsf",directory.path()).is_err());
    }
    #[test]
    fn search_multilingual() {
        let t = tokens("夜航 林间回声");
        assert!(t.contains("yehang"));
        assert!(t.contains("yh"));
        assert_eq!(
            query_tokens("a\" OR *"),
            "\"a\"\"\"* AND \"OR\"* AND \"*\"*"
        );
    }
}
