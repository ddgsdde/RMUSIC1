//! Reversible administrator operations over indexed files; never infer a file from a client path.
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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Filter {
    #[serde(default)]
    storage: String,
    #[serde(default)]
    availability: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    album_id: String,
    #[serde(default)]
    artist_id: String,
    #[serde(default)]
    folder: String,
    #[serde(default)]
    sort: String,
    max_play_count: Option<u64>,
    #[serde(default)]
    unlisted: bool,
    limit: Option<usize>,
    offset: Option<usize>,
}
const LISTEN_COUNT: &str = "COALESCE((SELECT SUM(st.play_count) FROM track_stats st WHERE st.track_id=t.id OR st.track_id=json_extract(t.provider_data,'$.cached_from.track_id')),0)";
fn protected_json(c:&rusqlite::Connection,f:&Filter)->anyhow::Result<String>{ if f.unlisted {Ok(serde_json::to_string(&crate::playlists::protected_tracks(c)?)?)}else{Ok(String::new())} }
fn filter_sql(f: &Filter) -> anyhow::Result<(String, String)> {
    let storage = if f.storage == "local" {
        "json_type(t.provider_data,'$.cached_from.cache_id') IS NULL"
    } else if f.storage == "cache" {
        "json_type(t.provider_data,'$.cached_from.cache_id')='text'"
    } else {
        anyhow::bail!("库类型无效");
    };
    let status = match f.availability.as_str() {
        "missing" => " AND t.available=0",
        "ready" => " AND t.available=1",
        "" | "all" => "",
        _ => anyhow::bail!("文件状态无效"),
    };
    if f.unlisted && f.storage != "cache" { anyhow::bail!("专项清理只允许筛选缓存库"); }
    if f.max_play_count.is_some_and(|n|n>1_000_000_000) { anyhow::bail!("聆听次数阈值无效"); }
    if !["", "title", "play_count_asc", "play_count_desc", "size_desc"].contains(&f.sort.as_str()) { anyhow::bail!("排序方式无效"); }
    let predicate=format!("FROM tracks t WHERE t.source='local' AND t.path IS NOT NULL AND {storage}{status} AND (?1='' OR t.album_id=?1) AND (?2='' OR t.artist_id=?2) AND (?3='' OR instr(lower(t.title||' '||t.artist||' '||t.album),lower(?3))>0) AND (?4='' OR substr(t.path,1,length(?4))=?4) AND (?5='' OR NOT EXISTS (SELECT 1 FROM json_each(?5) protected WHERE protected.value=t.id OR protected.value=json_extract(t.provider_data,'$.cached_from.track_id'))) AND (?6 IS NULL OR {LISTEN_COUNT}<=?6)");
    let folder = if f.folder.is_empty() {
        String::new()
    } else {
        format!(
            "{}{}",
            f.folder.trim_end_matches(['/', '\\']),
            std::path::MAIN_SEPARATOR
        )
    };
    Ok((predicate, folder))
}
fn selection(
    c: &rusqlite::Connection,
    f: &Filter,
    paged: bool,
) -> anyhow::Result<(Vec<(Track, String, u64, u64)>, usize)> {
    let (predicate, folder) = filter_sql(f)?;
    let protected=protected_json(c,f)?;
    let total = c.query_row(
        &format!("SELECT COUNT(*) {predicate}"),
        params![f.album_id, f.artist_id, f.q, folder, protected, f.max_play_count.map(|n|n as i64)],
        |r| r.get::<_, i64>(0).map(|v| v as usize),
    )?;
    let order=match f.sort.as_str(){"play_count_asc"=>"listened_count ASC,t.title COLLATE NOCASE,t.id","play_count_desc"=>"listened_count DESC,t.title COLLATE NOCASE,t.id","size_desc"=>"t.size DESC,t.id",_=>"t.title COLLATE NOCASE,t.id"};
    let sql=format!("SELECT t.*,{LISTEN_COUNT} AS listened_count {predicate} ORDER BY {order} LIMIT ?7 OFFSET ?8");
    let rows = c
        .prepare(&sql)?
        .query_map(
            params![
                f.album_id,
                f.artist_id,
                f.q,
                folder,
                protected, f.max_play_count.map(|n|n as i64),
                if paged {
                    f.limit.unwrap_or(50).clamp(1, 100) as i64
                } else {
                    100001
                },
                if paged { f.offset.unwrap_or(0) } else { 0 } as i64
            ],
            |r| {
                Ok((
                    Track::from_row(r)?,
                    r.get::<_, String>("path")?,
                    r.get::<_, i64>("size")?.max(0) as u64,
                    r.get::<_, i64>("listened_count")?.max(0) as u64,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((rows, total))
}

pub async fn list(
    State(s): State<AppState>,
    a: Auth,
    Query(f): Query<Filter>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let v =
        s.db.run(move |c| {
            let (rows, total) = selection(c, &f, true)?;
            let items = rows
                .into_iter()
                .map(|(t, path, size, plays)| {
                    let mut v = serde_json::to_value(t).unwrap();
                    v["path"] = json!(path);
                    v["size"] = json!(size);
                    v["play_count"] = json!(plays);
                    v
                })
                .collect::<Vec<_>>();
            Ok(json!({"total":total,"items":items}))
        })
        .await?;
    Ok(Json(v))
}
#[derive(Deserialize)]
pub struct Action {
    operation: String,
    #[serde(default)]
    ids: Vec<String>,
    filter: Option<Filter>,
    #[serde(default)]
    exclude_ids: Vec<String>,
    expected_count: Option<usize>,
    directory: Option<String>,
    cleanup: Option<Filter>,
}
pub async fn action(
    State(s): State<AppState>,
    a: Auth,
    Json(q): Json<Action>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if !["trash", "check", "recache", "relocate"].contains(&q.operation.as_str()) {
        return Err(ApiError::bad("操作类型无效"));
    }
    if q.exclude_ids.len() > 100000 {
        return Err(ApiError::bad("排除列表过长"));
    }
    let excluded = q
        .exclude_ids
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let cleanup=q.cleanup.or_else(||q.filter.as_ref().filter(|f|f.unlisted).cloned());
    if let Some(f)=cleanup.as_ref(){ if q.operation!="trash" || !f.unlisted || f.storage!="cache" || f.max_play_count.is_none(){return Err(ApiError::bad("专项清理必须限定缓存、未加入播放列表和聆听次数"));} filter_sql(f).map_err(|e|ApiError::bad(e.to_string()))?; }
    let ids = if let Some(f) = q.filter {
        let ids =
            s.db.run(move |c| {
                let (predicate, folder) = filter_sql(&f)?;
                let protected=protected_json(c,&f)?;
                let mut statement = c.prepare(&format!(
                    "SELECT t.id {predicate} ORDER BY t.id LIMIT 100001"
                ))?;
                let ids = statement
                    .query_map(params![f.album_id, f.artist_id, f.q, folder, protected, f.max_play_count.map(|n|n as i64)], |r| {
                        r.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                if ids.len() > 100000 {
                    anyhow::bail!("请缩小筛选范围后分批管理");
                }
                Ok(ids
                    .into_iter()
                    .filter(|id| !excluded.contains(id))
                    .collect::<Vec<_>>())
            })
            .await?;
        if q.expected_count != Some(ids.len()) {
            return Err(ApiError::bad("筛选结果已变化，请刷新后重新确认"));
        }
        ids
    } else {
        q.ids
    };
    if ids.is_empty() || ids.len() > 100000 {
        return Err(ApiError::bad(
            "请选择1–100000个文件，可缩小筛选范围后分批管理",
        ));
    }
    let ids = ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let total = ids.len();
    let id = crate::background::enqueue(
        &s,
        "library_files",
        &uuid::Uuid::new_v4().to_string(),
        json!({"operation":q.operation,"ids":ids,"directory":q.directory,"cleanup":cleanup}),
        10,
    )
    .await?;
    Ok(Json(json!({"job_id":id,"total":total})))
}
pub async fn bin(
    State(s): State<AppState>,
    a: Auth,
    Query(q): Query<Filter>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let (rows,total)=s.db.run(move|c|{let total=c.query_row("SELECT COUNT(*) FROM file_bin WHERE status IN ('planned','trashed','restoring','failed','missing')",[],|r|r.get::<_,i64>(0).map(|v|v as usize))?;let rows=c.prepare("SELECT b.*,t.title,t.artist FROM file_bin b LEFT JOIN tracks t ON t.id=b.track_id WHERE b.status IN ('planned','trashed','restoring','failed','missing') ORDER BY b.created_at DESC,b.id LIMIT ?1 OFFSET ?2")?.query_map(params![q.limit.unwrap_or(50).clamp(1,100) as i64,q.offset.unwrap_or(0) as i64],|r|Ok(json!({"id":r.get::<_,String>("id")?,"track_id":r.get::<_,String>("track_id")?,"title":r.get::<_,Option<String>>("title")?,"artist":r.get::<_,Option<String>>("artist")?,"path":r.get::<_,String>("original_path")?,"storage":r.get::<_,String>("storage")?,"status":r.get::<_,String>("status")?,"size":r.get::<_,i64>("size")?,"error":r.get::<_,Option<String>>("error")?})))?.collect::<rusqlite::Result<Vec<_>>>()?;Ok((rows,total))}).await?;
    Ok(Json(json!({"items":rows,"total":total})))
}
async fn indexed(s: &AppState, id: &str) -> anyhow::Result<(Track, String, Option<String>)> {
    let id = id.to_owned();
    s.db.run(move|c|{Ok(c.query_row("SELECT t.*,r.path AS root_path FROM tracks t LEFT JOIN roots r ON r.id=t.root_id WHERE t.id=?1 AND t.source='local'",[id],|r|Ok((Track::from_row(r)?,r.get("path")?,r.get("root_path")?)))?)}).await
}
async fn bin_status(
    s: &AppState,
    id: &str,
    status: &str,
    error: Option<String>,
) -> anyhow::Result<()> {
    let id = id.to_owned();
    let status = status.to_owned();
    s.db.run(move |c| {
        c.execute(
            "UPDATE file_bin SET status=?2,error=?3 WHERE id=?1",
            params![id, status, error],
        )?;
        Ok(())
    })
    .await
}
fn regular(path: &std::path::Path) -> anyhow::Result<std::fs::Metadata> {
    let m = std::fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() {
        anyhow::bail!("只允许普通音乐文件");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            anyhow::bail!("不操作重解析点");
        }
    }
    Ok(m)
}
async fn trash(s: &AppState, job: &str, id: &str) -> anyhow::Result<()> {
    let _zone = s.zone_lock.lock().await;
    let _cache = s.cache_lock.lock().await;
    let (t, path, root) = indexed(s, id).await?;
    let cache = t.provider_data["cached_from"]["cache_id"]
        .as_str()
        .map(str::to_owned);
    if s.zones.read().await.values().any(|z| {
        matches!(z.status.as_str(), "playing" | "paused")
            && z.track.as_ref().is_some_and(|x| {
                x.id == id
                    || cache.is_some()
                        && x.id
                            == t.provider_data["cached_from"]["track_id"]
                                .as_str()
                                .unwrap_or("")
            })
    }) || s
        .cache_active
        .lock()
        .await
        .keys()
        .any(|x| Some(x) == cache.as_ref())
    {
        anyhow::bail!("文件正在播放或暂停使用中，请先停止");
    }
    let p = PathBuf::from(&path);
    let root = if let Some(root) = root {
        PathBuf::from(root)
    } else if let Some(cid) = &cache {
        let key = cid.clone();
        let recorded =
            s.db.run(move |c| {
                Ok(c.query_row(
                    "SELECT path FROM cache_entries WHERE id=?1 AND single_file=1",
                    [key],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .await?;
        if recorded != path {
            anyhow::bail!("缓存索引与文件不一致，请先检查");
        }
        p.parent()
            .ok_or_else(|| anyhow::anyhow!("缓存路径无效"))?
            .to_owned()
    } else {
        anyhow::bail!("文件不属于已配置音乐目录或受管缓存");
    };
    let root =
        std::fs::canonicalize(root).map_err(|_| anyhow::anyhow!("音乐目录离线或尚未挂载"))?;
    let old =
        s.db.run({
            let job = job.to_owned();
            let id = id.to_owned();
            move |c| {
                Ok(c.query_row(
                    "SELECT id,trash_path,status FROM file_bin WHERE job_id=?1 AND track_id=?2",
                    params![job, id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?)
            }
        })
        .await?;
    if old
        .as_ref()
        .is_some_and(|(_, _, status)| status == "trashed")
    {
        return Ok(());
    }
    let bid = old
        .as_ref()
        .map(|v| v.0.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let destination = old
        .as_ref()
        .map(|v| PathBuf::from(&v.1))
        .unwrap_or_else(|| {
            root.join(".rmusic-trash").join(format!(
                "{bid}.{}",
                p.extension().and_then(|x| x.to_str()).unwrap_or("audio")
            ))
        });
    if old.is_some() && !p.exists() && destination.is_file() {
        return finish_trash(s, &bid, id, &cache).await;
    }
    let m = regular(&p)?;
    let resolved = std::fs::canonicalize(&p)?;
    if !resolved.starts_with(&root) || resolved == root {
        anyhow::bail!("文件越过配置目录边界");
    }
    let trash = root.join(".rmusic-trash");
    std::fs::create_dir_all(&trash)?;
    if std::fs::canonicalize(&trash)?.parent() != Some(root.as_path())
        || destination.parent() != Some(trash.as_path())
    {
        anyhow::bail!("回收站目录边界无效");
    }
    if destination.exists() {
        anyhow::bail!("回收站目标已存在，不覆盖文件");
    }
    let signature = if let Some(cid) = cache.clone() {
        Some(s.db.run(move|c|Ok(c.query_row("SELECT json_array(etag,content_etag,total_size,format) FROM cache_entries WHERE id=?1 AND complete=1",[cid],|r|r.get::<_,String>(0))?)).await?)
    } else {
        None
    };
    let key = bid.clone();
    let track = id.to_owned();
    let job = job.to_owned();
    let original = path;
    let dest = destination.to_string_lossy().into_owned();
    let cid = cache.clone();
    let storage = if cache.is_some() { "cache" } else { "local" };
    let source=p.clone();let target=destination.clone();let check_id=id.to_owned();
    // The SQLite worker serializes playlist/stat writes through the final recheck and rename.
    // Persist planned before moving so crash recovery retains the original path and signature.
    s.db.run(move|c|{
        let payload:String=c.query_row("SELECT payload FROM jobs WHERE id=?1",[&job],|r|r.get(0)).optional()?.unwrap_or_else(||"{}".into());
        if let Some(value)=serde_json::from_str::<Value>(&payload)?.get("cleanup").filter(|v|v.is_object()) {
            let f:Filter=serde_json::from_value(value.clone())?;let (predicate,folder)=filter_sql(&f)?;let protected=protected_json(c,&f)?;
            let allowed=c.query_row(&format!("SELECT COUNT(*) {predicate} AND t.id=?7"),params![f.album_id,f.artist_id,f.q,folder,protected,f.max_play_count.map(|n|n as i64),check_id],|r|r.get::<_,i64>(0))?;
            if f.storage!="cache"||!f.unlisted||allowed!=1 {anyhow::bail!("歌曲已加入播放列表、聆听次数已变化或不再属于缓存筛选，已跳过");}
        }
        c.execute("INSERT INTO file_bin(id,job_id,track_id,original_path,trash_path,storage,cache_id,status,size,cache_signature,created_at)VALUES(?1,?2,?3,?4,?5,?6,?7,'planned',?8,?9,unixepoch()) ON CONFLICT(id)DO UPDATE SET status='planned',error=NULL",params![key,job,track,original,dest,storage,cid,m.len()as i64,signature])?;
        if let Err(e)=std::fs::rename(source,target){let error=format!("无法移入回收站，请检查目录写权限：{e}");c.execute("UPDATE file_bin SET status='failed',error=?2 WHERE id=?1",params![key,error])?;anyhow::bail!(error);}
        Ok(())
    }).await?;
    finish_trash(s, &bid, id, &cache).await
}
async fn finish_trash(
    s: &AppState,
    bid: &str,
    id: &str,
    cache: &Option<String>,
) -> anyhow::Result<()> {
    let bid = bid.to_owned();
    let id = id.to_owned();
    let cache = cache.clone();
    s.db.run(move|c|{let tx=c.transaction()?;tx.execute("UPDATE tracks SET available=0 WHERE id=?1",[id])?;if let Some(cid)=cache{tx.execute("UPDATE cache_entries SET complete=0,size=0,allocated_size=0,ranges='[]' WHERE id=?1",[&cid])?;tx.execute("UPDATE cache_exports SET status='trashed',size=0 WHERE cache_id=?1",[cid])?;}tx.execute("UPDATE file_bin SET status='trashed',error=NULL WHERE id=?1",[bid])?;tx.commit()?;Ok(())}).await
}
pub async fn restore(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let _z = s.zone_lock.lock().await;
    let _c = s.cache_lock.lock().await;
    let key = id.clone();
    let(track,original,trash,cache,size,status,signature)=s.db.run(move|c|Ok(c.query_row("SELECT track_id,original_path,trash_path,cache_id,size,status,cache_signature FROM file_bin WHERE id=?1 AND status IN ('planned','trashed','restoring')",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<String>>(6)?)))?)).await?;
    if let Some(cid) = cache.clone() {
        let current=s.db.run(move|c|Ok(c.query_row("SELECT json_array(etag,content_etag,total_size,format) FROM cache_entries WHERE id=?1",[cid],|r|r.get::<_,String>(0))?)).await?;
        if signature.as_deref() != Some(current.as_str()) {
            return Err(ApiError::bad("缓存版本已变化，不能将旧文件恢复为新版本"));
        }
    }
    let recovering = status == "restoring"
        && std::path::Path::new(&original).is_file()
        && !std::path::Path::new(&trash).exists();
    if std::path::Path::new(&original).exists() && !recovering {
        return Err(ApiError::bad("原位置已有文件，不覆盖；请先检查新文件"));
    }
    let (_, _, root) = indexed(&s, &track).await?;
    let parent = std::fs::canonicalize(
        std::path::Path::new(&original)
            .parent()
            .ok_or_else(|| ApiError::bad("原目录无效"))?,
    )
    .map_err(|_| ApiError::bad("原目录离线，请恢复挂载后重试"))?;
    let root = std::fs::canonicalize(root.map(PathBuf::from).unwrap_or_else(|| parent.clone()))
        .map_err(anyhow::Error::from)?;
    let tp = std::fs::canonicalize(if recovering { &original } else { &trash })
        .map_err(|_| ApiError::bad("回收站文件丢失"))?;
    if !parent.starts_with(&root)
        || !recovering && tp.parent() != Some(root.join(".rmusic-trash").as_path())
        || regular(&tp)
            .map_err(|e| ApiError::bad(e.to_string()))?
            .len()
            != size as u64
    {
        return Err(ApiError::bad("回收站文件或路径校验失败"));
    }
    if !recovering {
        bin_status(&s, &id, "restoring", None).await?;
        tokio::fs::rename(&tp, &original)
            .await
            .map_err(anyhow::Error::from)?;
    }
    s.db.run(move|c|{let tx=c.transaction()?;tx.execute("UPDATE tracks SET available=1 WHERE id=?1",[track])?;if let Some(cid)=cache{tx.execute("UPDATE cache_entries SET complete=1,size=?2,total_size=?2,allocated_size=?2,ranges=?3 WHERE id=?1",params![cid,size,format!("[[0,{size}]]")])?;tx.execute("UPDATE cache_exports SET status='ready' WHERE cache_id=?1",[cid])?;}tx.execute("UPDATE file_bin SET status='restored',error=NULL WHERE id=?1",[id])?;tx.commit()?;Ok(())}).await?;
    s.emit("library", json!({}));
    Ok(Json(json!({"restored":true})))
}

#[derive(Deserialize)]
pub struct Purge {
    confirmation: String,
}
pub async fn purge(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
    Json(q): Json<Purge>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if q.confirmation != "永久删除" {
        return Err(ApiError::bad("需要明确确认永久删除"));
    }
    let _guard = s.cache_lock.lock().await;
    let key = id.clone();
    let (path,track,original)=s.db.run(move|c|Ok(c.query_row("SELECT trash_path,track_id,original_path FROM file_bin WHERE id=?1 AND status='trashed'",[key],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?)).await?;
    let (_, _, configured) = indexed(&s, &track).await?;
    let root = std::fs::canonicalize(
        configured
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&original).parent().unwrap().to_owned()),
    )
    .map_err(anyhow::Error::from)?;
    let p = std::path::Path::new(&path);
    if p.file_stem().and_then(|v| v.to_str()) != Some(id.as_str())
        || p.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            != Some(".rmusic-trash")
    {
        return Err(ApiError::bad("回收站路径无效"));
    }
    match regular(p) {
        Ok(_) => {
            let resolved = std::fs::canonicalize(p).map_err(anyhow::Error::from)?;
            if resolved.parent() != Some(root.join(".rmusic-trash").as_path()) {
                return Err(ApiError::bad("回收站路径变化"));
            }
            tokio::fs::remove_file(p)
                .await
                .map_err(anyhow::Error::from)?;
        }
        Err(_) if !p.exists() => {
            return Err(ApiError::bad(
                "回收站文件缺失或目录离线，未清理记录；请恢复挂载后重试",
            ))
        }
        Err(e) => return Err(ApiError::bad(e.to_string())),
    }
    bin_status(&s, &id, "purged", None).await?;
    s.emit("cache", json!({}));
    Ok(Json(json!({"purged":true})))
}
async fn relocate(s: &AppState, id: &str, directory: &str) -> anyhow::Result<()> {
    let _guard = s.zone_lock.lock().await;
    let (t, old, _) = indexed(s, id).await?;
    if t.available || t.provider_data["cached_from"].is_object() {
        anyhow::bail!("仅重新定位缺失的本地原文件；缓存请重新下载");
    }
    let directory = std::fs::canonicalize(directory)?;
    let file = directory.join(
        std::path::Path::new(&old)
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("原文件名无效"))?,
    );
    regular(&file)?;
    let file = std::fs::canonicalize(file)?;
    if !file.starts_with(&directory) {
        anyhow::bail!("目标文件越过所选目录");
    }
    let roots =
        s.db.run(|c| {
            Ok(c.prepare("SELECT id,path FROM roots")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await?;
    let root = roots
        .into_iter()
        .find(|(_, p)| std::fs::canonicalize(p).is_ok_and(|p| file.starts_with(p)))
        .ok_or_else(|| anyhow::anyhow!("请先把目标目录加入本地音乐目录"))?;
    let key = id.to_owned();
    let fp = file.clone();
    let data = s.config.data.clone();
    let parsed = tokio::task::spawn_blocking(move || {
        crate::library::probe_track(
            &fp,
            &key,
            fp.extension().and_then(|x| x.to_str()).unwrap_or(""),
            &data,
        )
    })
    .await??
    .0;
    if !parsed.title.eq_ignore_ascii_case(&t.title)
        || !parsed.artist.eq_ignore_ascii_case(&t.artist)
        || (parsed.duration - t.duration).abs() > 2.
    {
        anyhow::bail!("同名文件的歌曲资料／时长不符，未替换原曲目");
    }
    let meta = std::fs::metadata(&file)?;
    let path = file.to_string_lossy().into_owned();
    let mut track = t;
    track.available = true;
    s.db.run(move |c| {
        if c.query_row(
            "SELECT EXISTS(SELECT 1 FROM tracks WHERE path=?1 AND id!=?2)",
            params![path, track.id],
            |r| r.get::<_, bool>(0),
        )? {
            anyhow::bail!("目标文件已属于其他曲目，未覆盖");
        }
        crate::library::upsert(
            c,
            &track,
            Some(&path),
            Some(&root.0),
            meta.modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos() as i64,
            meta.len(),
            None,
            None,
        )?;
        Ok(())
    })
    .await
}
/// Cheap at-read validation. Only mark an individual missing cache, never a whole detached root.
pub(crate) async fn validate_cached(s: &AppState, id: &str) -> anyhow::Result<()> {
    let Ok(_guard) = s.cache_lock.try_lock() else {
        return Ok(());
    };
    let id = id.to_owned();
    let rows=s.db.run(move|c|Ok(c.prepare("WITH RECURSIVE target(id,depth) AS (SELECT ?1,0 UNION ALL SELECT r.new_id,depth+1 FROM track_redirects r JOIN target ON r.old_id=target.id WHERE depth<16) SELECT e.id,e.path,e.total_size FROM cache_entries e WHERE e.complete=1 AND (e.track_id IN (SELECT id FROM target) OR e.path IN(SELECT path FROM tracks WHERE id IN(SELECT id FROM target)))")?.query_map([id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    for (cid, path, total) in rows {
        let actual = tokio::fs::metadata(&path).await;
        let allocated = actual.as_ref().map(|m| m.len()).unwrap_or(0);
        match actual {
            Ok(m) if m.len() == total as u64 && m.len() > 0 => continue,
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => continue,
            _ => {}
        }
        s.db.run(move|c|{let tx=c.transaction()?;
            // A request made before this loss cannot authorize downloading the replacement.
            // A later playback/manual schedule validates first, then grants fresh intent.
            tx.execute("UPDATE jobs SET status='paused',payload=json_remove(payload,'$.origin'),phase='文件缺失，等待后续播放或手动补齐',updated_at=unixepoch() WHERE kind='cache_fill' AND status IN ('pending','running','interrupted','waiting','paused') AND json_extract(payload,'$.track_id') IN (SELECT track_id FROM cache_entries WHERE id=?1 UNION SELECT id FROM tracks WHERE source='local' AND path=?2)",params![&cid,&path])?;
            tx.execute("UPDATE cache_entries SET complete=0,size=0,ranges='[]',allocated_size=?2 WHERE id=?1",params![&cid,allocated as i64])?;tx.execute("UPDATE cache_exports SET status='missing',size=0,error='缓存文件丢失或大小变化，请重新缓存' WHERE cache_id=?1",[cid])?;tx.execute("UPDATE tracks SET available=0 WHERE source='local' AND path=?1",[path])?;tx.commit()?;Ok(())}).await?;
    }
    Ok(())
}
pub(crate) async fn run_job(s: &AppState, job: &str, p: &Value) -> anyhow::Result<bool> {
    let ids = p["ids"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("文件任务无效"))?;
    let mut failures = 0;
    let key = job.to_owned();
    let start =
        s.db.run(move |c| {
            Ok(
                c.query_row("SELECT processed FROM jobs WHERE id=?1", [key], |r| {
                    r.get::<_, i64>(0).map(|v| v as usize)
                })?,
            )
        })
        .await?;
    for (index, id) in ids.iter().enumerate().skip(start) {
        if !crate::background::disposition(s, job).await? {
            return Ok(false);
        }
        let id = id.as_str().unwrap_or("");
        let operation = p["operation"].as_str().unwrap_or("");
        let result=async{if operation=="trash"{trash(s,job,id).await}else if operation=="relocate"{relocate(s,id,p["directory"].as_str().unwrap_or("")).await}else{let result=indexed(s,id).await;match result{Err(e)=>Err(e),Ok((t,path,root))=>{if root.as_ref().is_some_and(|r|!std::path::Path::new(r).is_dir()){Err(anyhow::anyhow!("目录离线或未挂载；保留原曲目状态"))}else if operation=="recache"{if let Some(original)=t.provider_data["cached_from"]["track_id"].as_str(){let original=original.to_owned();let source=crate::library::get_track(s,original.clone()).await.map_err(|e|anyhow::anyhow!(e.2))?;let quality=crate::catalog::quality(s,&source.source).await;crate::cache_policy::schedule(s,&original,&quality,"manual",None,50).await.map(|_|())}else{Err(anyhow::anyhow!("本地原文件无法重新下载，请恢复文件或挂载后重新扫描"))}}else{validate_cached(s,id).await?;let ready=std::fs::metadata(&path).is_ok_and(|m|m.is_file()&&m.len()>0);let key=id.to_owned();s.db.run(move|c|{if !c.query_row("SELECT EXISTS(SELECT 1 FROM file_bin WHERE track_id=?1 AND status IN ('planned','trashed','restoring'))",[&key],|r|r.get::<_,bool>(0))?{c.execute("UPDATE tracks SET available=?2 WHERE id=?1",params![key,ready])?;}Ok(())}).await}}}}}.await;
        if let Err(error) = result {
            failures += 1;
            let key = format!("{job}:{index}");
            let job = job.to_owned();
            let id = id.to_owned();
            let error = error.to_string();
            s.db.run(move|c|{c.execute("INSERT OR IGNORE INTO file_bin(id,job_id,track_id,original_path,trash_path,storage,status,error,created_at)VALUES(?1,?2,?3,'','','','failed',?4,unixepoch())",params![key,job,id,error])?;Ok(())}).await?;
        }
        crate::background::progress(
            s,
            job,
            &format!("{operation}：{} / {}，失败{failures}", index + 1, ids.len()),
            (index + 1) as u64,
            ids.len() as u64,
        )
        .await?;
    }
    s.emit("library", json!({}));
    s.emit("cache", json!({}));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> (tempfile::TempDir, AppState, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("music");
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("song.flac");
        std::fs::write(&file, b"synthetic original bytes").unwrap();
        let db = crate::db::Db::open(&temp.path().join("library.db")).unwrap();
        let state = AppState::new(
            db,
            crate::state::Config {
                data: temp.path().into(),
                web: temp.path().join("web"),
                base_path: String::new(),
                public_url: None,
                plugin_dir: temp.path().join("plugins"),
                python: if cfg!(windows){"python".into()}else{"python3".into()},
            },
            String::new(),
        )
        .await
        .unwrap();
        let path = file.to_string_lossy().into_owned();
        let root = root.to_string_lossy().into_owned();
        state.db.run(move |c| {
            c.execute("INSERT INTO roots VALUES('test-root',?1)", [root])?;
            c.execute("INSERT INTO tracks(id,path,root_id,title,artist,album,album_artist,source,format,recording_id,created_at,size) VALUES('test-track',?1,'test-root','Title','Artist','Album','Artist','local','flac','test-recording',1,24)", [path])?;
            c.execute("INSERT INTO track_stats(track_id,play_seconds) VALUES('test-track',42)", [])?;
            Ok(())
        }).await.unwrap();
        (temp, state, file)
    }
    fn admin() -> Auth {
        Auth {
            id: "test-admin".into(),
            username: "test".into(),
            role: "admin".into(),
            totp_enabled: false,
            interactive: true,
            scopes: vec!["admin".into()],
            session_hash: String::new(),
        }
    }
    #[tokio::test]
    async fn cleanup_only_cache_aggregates_listens_and_rechecks_new_playlist_membership(){
        let(_temp,state,file)=fixture().await;let path=file.to_string_lossy().to_string();
        state.db.run(move|c|{
            c.execute("UPDATE tracks SET provider_data=?1 WHERE id='test-track'",[json!({"cached_from":{"track_id":"origin","cache_id":"cache","source":"example"}}).to_string()])?;
            c.execute("UPDATE track_stats SET play_count=3 WHERE track_id='test-track'",[])?;
            c.execute("INSERT INTO track_stats(track_id,play_count)VALUES('origin',2)",[])?;
            c.execute("INSERT INTO cache_entries(id,track_id,path,format,size,total_size,complete,single_file,last_access,allocated_size)VALUES('cache','origin',?1,'flac',24,24,1,1,1,24)",[path])?;
            Ok(())
        }).await.unwrap();
        let filter=Filter{storage:"cache".into(),unlisted:true,max_play_count:Some(5),sort:"play_count_asc".into(),..Default::default()};
        let query=filter.clone();let rows=state.db.run(move|c|{let(r,total)=selection(c,&query,true)?;assert_eq!(total,1);assert_eq!(r[0].3,5);Ok(())}).await.unwrap();let _=rows;
        let query=Filter{max_play_count:Some(4),..filter.clone()};state.db.run(move|c|{assert_eq!(selection(c,&query,true)?.1,0);Ok(())}).await.unwrap();
        assert!(filter_sql(&Filter{storage:"local".into(),..filter.clone()}).is_err());
        let payload=json!({"operation":"trash","cleanup":filter,"ids":["test-track"]}).to_string();
        state.db.run(move|c|{c.execute("INSERT INTO jobs(id,kind,status,created_at,payload)VALUES('cleanup-job','library_files','pending',0,?1)",[payload])?;
            c.execute("INSERT INTO playlists(id,name,user_id,tracks)VALUES('saved','Any user list','another-user','[\"origin\"]')",[])?;Ok(())}).await.unwrap();
        let error=trash(&state,"cleanup-job","test-track").await.unwrap_err();assert!(error.to_string().contains("播放列表"));assert!(file.is_file());
    }
    #[tokio::test]
    async fn library_files_trash_restore_and_purge_preserve_history() {
        let (_temp, state, file) = fixture().await;
        trash(&state, "job-one", "test-track").await.unwrap();
        assert!(!file.exists());
        let id = state
            .db
            .run(|c| {
                Ok(
                    c.query_row("SELECT id FROM file_bin WHERE status='trashed'", [], |r| {
                        r.get::<_, String>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        std::fs::write(&file, b"do not overwrite").unwrap();
        assert!(restore(State(state.clone()), admin(), Path(id.clone()))
            .await
            .is_err());
        std::fs::remove_file(&file).unwrap();
        let _ = restore(State(state.clone()), admin(), Path(id))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"synthetic original bytes");
        trash(&state, "job-two", "test-track").await.unwrap();
        let id = state
            .db
            .run(|c| {
                Ok(
                    c.query_row("SELECT id FROM file_bin WHERE status='trashed'", [], |r| {
                        r.get::<_, String>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert!(purge(
            State(state.clone()),
            admin(),
            Path(id.clone()),
            Json(Purge {
                confirmation: "".into()
            })
        )
        .await
        .is_err());
        let _ = purge(
            State(state.clone()),
            admin(),
            Path(id),
            Json(Purge {
                confirmation: "永久删除".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            state
                .db
                .run(|c| Ok(c.query_row(
                    "SELECT play_seconds FROM track_stats WHERE track_id='test-track'",
                    [],
                    |r| r.get::<_, f64>(0)
                )?))
                .await
                .unwrap(),
            42.0
        );
    }
    #[tokio::test]
    async fn library_files_offline_root_and_permissions() {
        let (_temp, state, file) = fixture().await;
        let original = file.parent().unwrap();
        let detached = original.with_extension("detached");
        std::fs::rename(original, &detached).unwrap();
        let error = crate::library::get_track(&state, "test-track".into())
            .await
            .unwrap_err();
        assert_eq!(error.1, "library_offline");
        assert!(state
            .db
            .run(|c| Ok(c.query_row(
                "SELECT available FROM tracks WHERE id='test-track'",
                [],
                |r| r.get::<_, bool>(0)
            )?))
            .await
            .unwrap());
        let mut reader = admin();
        reader.scopes = vec!["read".into()];
        assert_eq!(
            list(
                State(state),
                reader,
                Query(Filter {
                    storage: "local".into(),
                    ..Default::default()
                })
            )
            .await
            .unwrap_err()
            .0,
            axum::http::StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn library_files_pagination_and_changed_scope() {
        let (_temp, state, _file) = fixture().await;
        let response = list(
            State(state.clone()),
            admin(),
            Query(Filter {
                storage: "local".into(),
                offset: Some(99),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(response["total"], 1);
        assert_eq!(response["items"].as_array().unwrap().len(), 0);
        assert!(action(
            State(state),
            admin(),
            Json(Action {
                operation: "trash".into(),
                ids: vec![],
                exclude_ids: vec![],
                filter: Some(Filter {
                    storage: "local".into(),
                    ..Default::default()
                }),
                expected_count: Some(2),
                directory: None, cleanup: None
            })
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn library_files_public_routes_require_admin_and_never_cache_secrets() {
        use tower::ServiceExt;
        let (_temp, state, _file) = fixture().await;
        let hash = crate::auth::hash("synthetic-reader-session");
        state.db.run(move|c|{c.execute("INSERT INTO users(id,username,password,role) VALUES('reader','reader','unused','user')",[])?;c.execute("INSERT INTO sessions(hash,user_id,expires,scopes,created_at) VALUES(?1,'reader',9999999999,'[\"read\",\"control\"]',1)",[hash])?;Ok(())}).await.unwrap();
        let app = crate::app(state);
        let root=app.clone().oneshot(axum::http::Request::builder().uri("/").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(root.headers()["x-frame-options"],"SAMEORIGIN");
        for path in [
            "/filesystem/directories",
            "/maintenance/settings",
            "/maintenance/backups",
            "/maintenance/backups/fake/file",
            "/library/files?storage=local",
            "/library/files/bin",
        ] {
            for (token, status) in [(None, 401), (Some("synthetic-reader-session"), 403)] {
                let mut request = axum::http::Request::builder().uri(format!("/api/v1{path}"));
                if let Some(token) = token {
                    request = request.header("authorization", format!("Bearer {token}"));
                }
                let response = app
                    .clone()
                    .oneshot(request.body(axum::body::Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status().as_u16(), status, "{path}");
                assert_eq!(response.headers()["x-content-type-options"], "nosniff");
                if path.starts_with("/maintenance/") || path.starts_with("/filesystem/") {
                    assert_eq!(response.headers()["cache-control"], "no-store");
                }
            }
        }
    }
    #[tokio::test]
    async fn library_files_backup_policy_blocks_public_web_directory() {
        let (_temp, state, _file) = fixture().await;
        std::fs::create_dir_all(&state.config.web).unwrap();
        let result = crate::maintenance::configure(
            State(state.clone()),
            admin(),
            Json(crate::maintenance::Policy {
                directory: state.config.web.to_string_lossy().into_owned(),
                enabled: true,
                interval_hours: 24,
                keep: 7,
            }),
        )
        .await;
        assert!(result.unwrap_err().2.contains("公开目录"));
    }
    #[tokio::test]
    async fn library_files_missing_cache_falls_back_to_origin_without_losing_identity() {
        let (_temp, state, file) = fixture().await;
        let path = file.to_string_lossy().into_owned();
        state.db.run(move|c|{
            c.execute("INSERT INTO tracks(id,title,artist,album,album_artist,source,format,recording_id,created_at) VALUES('cloud-track','Title','Artist','Album','Artist','example','flac','test-recording',1)",[])?;
            c.execute("UPDATE tracks SET root_id=NULL,provider_data=?1 WHERE id='test-track'",[json!({"cached_from":{"track_id":"cloud-track","cache_id":"test-cache","source":"example"}}).to_string()])?;
            c.execute("INSERT INTO cache_entries(id,track_id,path,format,size,total_size,complete,single_file,last_access,allocated_size) VALUES('test-cache','cloud-track',?1,'flac',24,24,1,1,1,24)",[path])?;
            Ok(())
        }).await.unwrap();
        std::fs::remove_file(&file).unwrap();
        let track = crate::library::get_track(&state, "test-track".into())
            .await
            .unwrap();
        assert_eq!(track.id, "test-track");
        assert_eq!(track.source, "example");
        assert_eq!(
            track.provider_data["cached_from"]["track_id"],
            "cloud-track"
        );
        assert!(!state
            .db
            .run(|c| Ok(c.query_row(
                "SELECT complete FROM cache_entries WHERE id='test-cache'",
                [],
                |r| r.get::<_, bool>(0)
            )?))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn library_files_trash_index_survives_cache_pressure_and_restores() {
        let (_temp, state, file) = fixture().await;
        let path = file.to_string_lossy().into_owned();
        state.db.run(move|c|{
            c.execute("UPDATE tracks SET root_id=NULL,provider_data=?1 WHERE id='test-track'",[json!({"cached_from":{"track_id":"cloud-track","cache_id":"test-cache","source":"example"}}).to_string()])?;
            c.execute("INSERT INTO cache_entries(id,track_id,path,format,size,total_size,complete,single_file,last_access,allocated_size) VALUES('test-cache','cloud-track',?1,'flac',24,24,1,1,1,24)",[path])?;
            c.execute("INSERT INTO settings(key,value)VALUES('cache_limit_bytes','0')",[])?;
            Ok(())
        }).await.unwrap();
        trash(&state, "pressure-job", "test-track").await.unwrap();
        assert!(!crate::media::ensure_capacity(&state, 0).await.unwrap());
        let id = state
            .db
            .run(|c| {
                Ok(
                    c.query_row("SELECT id FROM file_bin WHERE status='trashed'", [], |r| {
                        r.get::<_, String>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        let _ = restore(State(state.clone()), admin(), Path(id))
            .await
            .unwrap();
        assert_eq!(std::fs::read(file).unwrap(), b"synthetic original bytes");
    }
}
