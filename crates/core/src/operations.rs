//! NAS diagnostics and explicit backup verification; never restores into live data.
use crate::{
    auth::Auth,
    error::{ApiError, ApiResult},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    Json,
};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::path::Path as FsPath;

pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
    tracing::info!("核心正在停止接收新请求，等待连接结束");
}
#[cfg(target_os = "linux")]
fn filesystem(path: &FsPath) -> Option<String> {
    let mounts = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    mount_type(path, &mounts)
}
#[cfg(not(target_os = "linux"))]
fn filesystem(_path: &FsPath) -> Option<String> {
    None
}
#[cfg(any(target_os = "linux", test))]
fn mount_type(path: &FsPath, mounts: &str) -> Option<String> {
    mounts
        .lines()
        .filter_map(|line| {
            let (a, b) = line.split_once(" - ")?;
            let mount = a
                .split_whitespace()
                .nth(4)?
                .replace("\\040", " ")
                .replace("\\134", "\\");
            let kind = b.split_whitespace().next()?;
            path.starts_with(&mount)
                .then(|| (mount.len(), kind.to_owned()))
        })
        .max_by_key(|v| v.0)
        .map(|v| v.1)
}
pub fn validate_database_location(path: &FsPath) -> anyhow::Result<()> {
    let canonical = std::fs::canonicalize(path)?;
    if filesystem(&canonical).is_some_and(|kind| {
        matches!(
            kind.as_str(),
            "nfs" | "nfs4" | "cifs" | "smb3" | "fuse.sshfs"
        )
    }) {
        anyhow::bail!("SQLite 数据目录不能位于 NFS/SMB/SSHFS；请把 /data 放在 NAS 本地持久卷，音乐目录可以网络挂载");
    }
    Ok(())
}
fn free_bytes(path: &FsPath) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut value = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), value.as_mut_ptr()) } != 0 {
            return None;
        }
        let value = unsafe { value.assume_init() };
        Some((value.f_bavail as u64).saturating_mul(value.f_frsize as u64))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}
pub async fn health(State(s): State<AppState>, a: Auth) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let roots =
        s.db.read(|c| {
            Ok(c.prepare("SELECT id,path FROM roots ORDER BY id")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await?;
    let data = s.config.data.clone();
    let value=tokio::task::spawn_blocking(move||{
        let wal=data.join("library.db-wal");
        json!({"data_path":data,"filesystem":filesystem(&data),"available_bytes":free_bytes(&data),"wal_bytes":std::fs::metadata(wal).map(|m|m.len()).unwrap_or(0),"roots":roots.into_iter().map(|(id,path)|json!({"id":id,"online":FsPath::new(&path).is_dir(),"path":path})).collect::<Vec<_>>(),"durability":"FULL","restore_requires_stopped_core":true})
    }).await.map_err(anyhow::Error::from)?;
    Ok(Json(
        json!({"storage":value,"database":s.db.metrics(),"resources":s.resources.status(&s).await,"plugin_processes":s.plugin.process_count().await}),
    ))
}
/// Concurrent stdout/stderr caps and kill+wait on cancellation/error bound tool resources.
pub(crate) async fn tool(mut command: tokio::process::Command) -> anyhow::Result<String> {
    use tokio::io::AsyncReadExt;
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    async fn read(input: impl tokio::io::AsyncRead + Unpin) -> anyhow::Result<Vec<u8>> {
        let mut bytes = vec![];
        input.take(65537).read_to_end(&mut bytes).await?;
        if bytes.len() > 65536 {
            anyhow::bail!("维护工具响应超过限制");
        }
        Ok(bytes)
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(600), async {
        let (out, err, status) = tokio::try_join!(read(out), read(err), async {
            Ok::<_, anyhow::Error>(child.wait().await?)
        })?;
        if !status.success() {
            anyhow::bail!(
                "{}",
                String::from_utf8_lossy(if err.is_empty() { &out } else { &err })
                    .chars()
                    .take(500)
                    .collect::<String>()
            );
        }
        Ok::<_, anyhow::Error>(String::from_utf8_lossy(&out).into_owned())
    })
    .await;
    match result {
        Ok(Ok(out)) => Ok(out),
        other => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            match other {
                Ok(Err(e)) => Err(e),
                _ => anyhow::bail!("维护任务超过十分钟，已终止；请使用离线工具重试"),
            }
        }
    }
}
pub async fn verify_backup(
    State(s): State<AppState>,
    a: Auth,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    a.require("admin")?;
    let _permit = s
        .resources
        .background(&s, 2 * 1024 * 1024, true)
        .await
        .map_err(|m| {
            ApiError(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "maintenance_busy".into(),
                m.into(),
            )
        })?;
    let key = id.clone();
    let path =
        s.db.read(move |c| {
            Ok(c.query_row(
                "SELECT path FROM maintenance_backups WHERE id=?1 AND status='completed'",
                [key],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
        })
        .await?
        .ok_or_else(ApiError::missing)?;
    let directory = s.config.data.join(".maintenance");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(anyhow::Error::from)?;
    let tool_path = directory.join("backup-verify.py");
    tokio::fs::write(&tool_path, include_str!("../../../scripts/backup.py"))
        .await
        .map_err(anyhow::Error::from)?;
    let mut command = tokio::process::Command::new(&s.config.python);
    command
        .env("TMPDIR", &directory)
        .arg(tool_path)
        .arg("rehearse")
        .arg(path);
    let result = tool(command).await;
    let ok = result.is_ok();
    let message = result
        .err()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "完整性与恢复演练通过，原数据未修改".into());
    let response = json!({"ok":ok,"message":message});
    s.db.run(move|c|{c.execute("INSERT INTO backup_verifications VALUES(?1,unixepoch(),?2,?3) ON CONFLICT(backup_id) DO UPDATE SET verified_at=excluded.verified_at,ok=excluded.ok,message=excluded.message",rusqlite::params![id,ok,message])?;Ok(())}).await?;
    Ok(Json(response))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn longest_mount_boundary_is_used() {
        let mounts="1 0 0:1 / / rw - ext4 /dev/a rw\n2 1 0:2 / /nas rw - nfs4 server rw\n3 1 0:3 / /nas/local rw - btrfs /dev/b rw";
        assert_eq!(
            mount_type(FsPath::new("/nas/data"), mounts).as_deref(),
            Some("nfs4")
        );
        assert_eq!(
            mount_type(FsPath::new("/nas/local/data"), mounts).as_deref(),
            Some("btrfs")
        );
        assert_eq!(
            mount_type(FsPath::new("/nasty/data"), mounts).as_deref(),
            Some("ext4")
        );
    }
}
