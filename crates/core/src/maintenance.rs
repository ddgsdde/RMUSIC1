//! Administrator file browsing and durable online database backups. No hot restore.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
#[derive(Clone, Serialize, Deserialize)]
pub struct Policy {
    pub directory: String,
    pub enabled: bool,
    pub interval_hours: u32,
    pub keep: u32,
}
async fn policy(s: &AppState) -> Policy {
    serde_json::from_value(s.setting("backup_policy",json!({"directory":std::env::var("RMUSIC_BACKUP_DIR").unwrap_or_else(|_|s.config.data.join("backups").to_string_lossy().into_owned()),"enabled":false,"interval_hours":24,"keep":7})).await).unwrap_or(Policy{directory:s.config.data.join("backups").to_string_lossy().into_owned(),enabled:false,interval_hours:24,keep:7})
}
#[derive(Deserialize)]
pub struct DirectoryQuery {
    path: Option<String>,
    offset: Option<usize>,
}
pub async fn directories(
    State(s): State<AppState>,
    a: Auth,
    Query(q): Query<DirectoryQuery>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let default = s.config.data.clone();
    let v=tokio::task::spawn_blocking(move||->anyhow::Result<Value>{
 if q.path.as_deref().unwrap_or("").is_empty(){let mut roots=vec![default];#[cfg(windows)]for drive in b'C'..=b'Z'{let p=PathBuf::from(format!("{}:\\",drive as char));if p.is_dir(){roots.push(p);}}#[cfg(not(windows))]for p in ["/music","/cache","/backups","/data","/"]{if std::path::Path::new(p).is_dir(){roots.push(PathBuf::from(p));}}roots.sort();roots.dedup();return Ok(json!({"path":"","parent":null,"items":roots.into_iter().map(|p|json!({"name":p.to_string_lossy(),"path":p.to_string_lossy()})).collect::<Vec<_>>(),"total":0,"more":false}));}
 let path=std::fs::canonicalize(q.path.unwrap())?;if !path.is_dir(){anyhow::bail!("目录不存在或尚未挂载");}let mut items=std::fs::read_dir(&path)?.filter_map(Result::ok).filter(|e|e.file_type().is_ok_and(|t|t.is_dir()&&!t.is_symlink())).map(|e|(e.file_name().to_string_lossy().into_owned(),e.path())).collect::<Vec<_>>();items.sort_by_key(|e|e.0.to_lowercase());let total=items.len();let offset=q.offset.unwrap_or(0);Ok(json!({"path":path.to_string_lossy(),"parent":path.parent().map(|p|p.to_string_lossy()),"total":total,"more":offset+200<total,"items":items.into_iter().skip(offset).take(200).map(|(name,p)|json!({"name":name,"path":p.to_string_lossy()})).collect::<Vec<_>>()}))
 }).await.map_err(anyhow::Error::from)??;
    Ok(Json(v))
}
#[derive(Deserialize)]
pub struct NewDirectory {
    parent: String,
    name: String,
}
pub async fn mkdir(
    State(_s): State<AppState>,
    a: Auth,
    Json(q): Json<NewDirectory>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if q.name.trim().is_empty()
        || q.name.len() > 180
        || q.name == "."
        || q.name == ".."
        || q.name
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        return Err(ApiError::bad("文件夹名称无效"));
    }
    let parent = tokio::fs::canonicalize(q.parent)
        .await
        .map_err(anyhow::Error::from)?;
    let path = parent.join(q.name.trim());
    tokio::fs::create_dir(&path)
        .await
        .map_err(|e| ApiError::bad(format!("无法新建文件夹，请检查挂载及写入权限：{e}")))?;
    Ok(Json(json!({"path":path.to_string_lossy()})))
}
pub async fn settings(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    Ok(Json(serde_json::to_value(policy(&s).await).unwrap()))
}
pub async fn configure(
    State(s): State<AppState>,
    a: Auth,
    Json(mut p): Json<Policy>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    if !(1..=720).contains(&p.interval_hours) || !(1..=90).contains(&p.keep) {
        return Err(ApiError::bad("备份间隔须为1–720小时，自动备份保留1–90份"));
    }
    let path = tokio::fs::canonicalize(&p.directory)
        .await
        .map_err(|_| ApiError::bad("备份目录不存在，请使用目录选择器创建或选择已挂载目录"))?;
    if !path.is_dir() {
        return Err(ApiError::bad("请选择目录"));
    }
    if std::fs::canonicalize(&s.config.web).is_ok_and(|web| path.starts_with(web)) {
        return Err(ApiError::bad("备份含登录凭据，不能保存在网页公开目录"));
    }
    let probe = path.join(format!(".rmusic-write-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&probe, [])
        .await
        .map_err(|_| ApiError::bad("备份目录不可写，Docker请使用可写挂载"))?;
    tokio::fs::remove_file(&probe)
        .await
        .map_err(anyhow::Error::from)?;
    p.directory = path.to_string_lossy().into_owned();
    let raw = serde_json::to_string(&p).unwrap();
    s.db.run(move|c|{c.execute("INSERT INTO settings(key,value)VALUES('backup_policy',?1)ON CONFLICT(key)DO UPDATE SET value=excluded.value",[raw])?;Ok(())}).await?;
    settings(State(s), a).await
}
async fn enqueue(s: &AppState, automatic: bool) -> anyhow::Result<String> {
    let p = policy(s).await;
    let id = uuid::Uuid::new_v4().to_string();
    let path = PathBuf::from(&p.directory).join(format!("rmusic-{id}.zip"));
    let key = id.clone();
    s.db.run(move|c|{if c.query_row("SELECT COUNT(*) FROM maintenance_backups WHERE status IN ('pending','running')",[],|r|r.get::<_,i64>(0))?>0{anyhow::bail!("已有备份正在排队或执行");}c.execute("INSERT INTO maintenance_backups(id,path,status,automatic,created_at)VALUES(?1,?2,'pending',?3,unixepoch())",params![key,path.to_string_lossy(),automatic])?;Ok(())}).await?;
    Ok(id)
}
pub async fn create(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let p = policy(&s).await;
    if !std::path::Path::new(&p.directory).is_dir() {
        return Err(ApiError::bad("先选择并保存备份目录"));
    }
    Ok(Json(
        json!({"id":enqueue(&s,false).await?,"status":"pending"}),
    ))
}
pub async fn list(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let v=s.db.run(|c|Ok(c.prepare("SELECT b.id,b.path,b.status,b.automatic,b.created_at,b.finished_at,b.size,b.error,v.verified_at,v.ok,v.message FROM maintenance_backups b LEFT JOIN backup_verifications v ON v.backup_id=b.id ORDER BY b.created_at DESC,b.id DESC LIMIT 200")?.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"path":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"automatic":r.get::<_,bool>(3)?,"created_at":r.get::<_,i64>(4)?,"finished_at":r.get::<_,Option<i64>>(5)?,"size":r.get::<_,i64>(6)?,"error":r.get::<_,Option<String>>(7)?,"verified_at":r.get::<_,Option<i64>>(8)?,"verified_ok":r.get::<_,Option<bool>>(9)?,"verification_message":r.get::<_,Option<String>>(10)?})))?.collect::<rusqlite::Result<Vec<_>>>()?)).await?;
    Ok(Json(json!({"items":v})))
}
pub async fn file(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    a.require("admin")?;
    let key = id.clone();
    let path =
        s.db.run(move |c| {
            Ok(c.query_row(
                "SELECT path FROM maintenance_backups WHERE id=?1 AND status='completed'",
                [key],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
        })
        .await?
        .ok_or_else(ApiError::missing)?;
    if std::path::Path::new(&path)
        .file_name()
        .and_then(|v| v.to_str())
        != Some(format!("rmusic-{id}.zip").as_str())
    {
        return Err(ApiError::bad("备份路径无效"));
    }
    let f = tokio::fs::File::open(path)
        .await
        .map_err(|_| ApiError::bad("备份文件丢失或备份目录未挂载"))?;
    let len = f.metadata().await.map_err(anyhow::Error::from)?.len();
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_owned()),
            (header::CONTENT_LENGTH, len.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"rmusic-{id}.zip\""),
            ),
            (header::CACHE_CONTROL, "no-store".into()),
        ],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(f)),
    )
        .into_response())
}
pub fn spawn(s: AppState) {
    tokio::spawn(async move {
        let _=s.db.run(|c|{c.execute("UPDATE maintenance_backups SET status='pending',error='核心重启，重新创建快照' WHERE status='running'",[])?;Ok(())}).await;
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            timer.tick().await;
            crate::catalog::release_expired(&s).await;
            s.plugin.release_idle(std::time::Duration::from_secs(300)).await;
            let p = policy(&s).await;
            if p.enabled {
                let hours = p.interval_hours;
                let due=s.db.run(move|c|Ok(c.query_row("SELECT COALESCE(MAX(created_at),0) FROM maintenance_backups WHERE automatic=1",[],|r|r.get::<_,i64>(0))?+i64::from(hours)*3600<=chrono::Utc::now().timestamp())).await.unwrap_or(false);
                if due {
                    let _ = enqueue(&s, true).await;
                }
            }
            let next=s.db.run(|c|Ok(c.query_row("SELECT id,path FROM maintenance_backups WHERE status='pending' ORDER BY created_at,id LIMIT 1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?)).await;
            let Ok(Some((id, path))) = next else {
                continue;
            };
            let key = id.clone();
            let _ =
                s.db.run(move |c| {
                    c.execute(
                        "UPDATE maintenance_backups SET status='running',error=NULL WHERE id=?1",
                        [key],
                    )?;
                    Ok(())
                })
                .await;
            let result = async {
                let _remote = s.remote.update_lock.lock().await;
                let tools = s.config.data.join(".maintenance");
                tokio::fs::create_dir_all(&tools).await?;
                let tool = tools.join("backup.py");
                tokio::fs::write(&tool, include_str!("../../../scripts/backup.py")).await?;
                let mut cmd = tokio::process::Command::new(&s.config.python);
                cmd.arg(&tool)
                    .arg("create-database")
                    .arg("--data-dir")
                    .arg(&s.config.data)
                    .arg("--output")
                    .arg(&path)
                    .kill_on_drop(true);
                #[cfg(windows)]
                cmd.creation_flags(0x08000000);
                crate::operations::tool(cmd).await?;
                Ok::<_, anyhow::Error>(tokio::fs::metadata(&path).await?.len())
            }
            .await;
            let key = id.clone();
            let success = result.is_ok();
            let (size, error) = match result {
                Ok(size) => (size, None),
                Err(e) => (0, Some(e.to_string())),
            };
            let _=s.db.run(move|c|{c.execute("UPDATE maintenance_backups SET status=?2,size=?3,error=?4,finished_at=unixepoch() WHERE id=?1",params![key,if success{"completed"}else{"failed"},size as i64,error])?;Ok(())}).await;
            s.emit("backup", json!({"id":id,"completed":success}));
            if success {
                let keep = p.keep;
                let old=s.db.run(move|c|Ok(c.prepare("SELECT id,path FROM maintenance_backups WHERE automatic=1 AND status='completed' ORDER BY created_at DESC,id DESC LIMIT -1 OFFSET ?1")?.query_map([keep],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)).await.unwrap_or_default();
                for (id, path) in old {
                    let p = std::path::Path::new(&path);
                    if p.file_name().and_then(|v| v.to_str())
                        != Some(format!("rmusic-{id}.zip").as_str())
                        || tokio::fs::symlink_metadata(p)
                            .await
                            .is_ok_and(|m| m.file_type().is_symlink())
                    {
                        continue;
                    }
                    if tokio::fs::remove_file(p).await.is_ok() {
                        let _ =
                            s.db.run(move |c| {
                                c.execute(
                                    "UPDATE maintenance_backups SET status='expired' WHERE id=?1",
                                    [id],
                                )?;
                                Ok(())
                            })
                            .await;
                    }
                }
            }
        }
    });
}
