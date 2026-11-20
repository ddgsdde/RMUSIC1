//! Bounded Core artifact export. Private temporary files and exclusive publication.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub async fn save(api: crate::api::Api, body: Value) -> Result<Value> {
    let path = body["path"].as_str().context("缺少导出项目")?;
    validate_export(path)?;
    let directory = PathBuf::from(body["directory"].as_str().context("请选择保存目录")?);
    let name = body["filename"].as_str().context("缺少文件名")?;
    anyhow::ensure!(
        directory.is_absolute() && directory.is_dir(),
        "保存目录不存在或已断开"
    );
    anyhow::ensure!(
        !name.is_empty()
            && name.len() <= 240
            && ![".", ".."].contains(&name)
            && !name.chars().any(|c| c.is_control() || "/\\:".contains(c)),
        "文件名无效"
    );
    let directory = directory.canonicalize()?;
    let target = directory.join(name);
    anyhow::ensure!(
        !target.exists(),
        "目标文件已存在，请选择新的文件名；未覆盖已有文件"
    );
    let mut response = api.file_response(path).await?;
    let max: u64 = if path.starts_with("/cache/") || path.starts_with("/downloads/") { 16 * 1024 * 1024 * 1024 } else if path.starts_with("/auth/external-accounts/") { 2 * 1024 * 1024 * 1024 } else { 512 * 1024 * 1024 };
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= max),
        "导出文件超过本次大小上限，请在 Core 主机上保管"
    );
    let temporary =
        Temporary(directory.join(format!(".rmusic-export-{}.part", uuid::Uuid::new_v4())));
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary.0)
        .await?;
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .await?;
    let mut bytes = 0u64;
    while let Some(chunk) = response.chunk().await? {
        bytes = bytes
            .checked_add(chunk.len() as u64)
            .context("导出文件大小无效")?;
        anyhow::ensure!(bytes <= max, "导出文件超过本次大小上限");
        file.write_all(&chunk).await?;
    }
    file.sync_all().await?;
    drop(file);
    crate::export_file::publish_exclusive(&temporary.0, &target)?;
    Ok(json!({"path":target,"bytes":bytes}))
}
fn validate_export(path: &str) -> Result<()> {
    let route = path.split('?').next().unwrap_or("");
    let parts: Vec<_> = route.split('/').collect();
    let valid = matches!(
        parts.as_slice(),
        ["", "maintenance", "backups", _, "file"]
            | ["", "auth", "external-accounts", _, "export"]
            | ["", "playlists", _, "export"]
            | ["", "plugins", "development", "template"]
            | ["", "cache", _, "file"]
            | ["", "downloads", _, "file"]
            | ["", "openapi.json"]
    );
    anyhow::ensure!(valid, "此接口不是可导出的文件");
    Ok(())
}
#[cfg(test)]
mod tests {
    #[test]
    fn only_core_artifact_routes_can_write_a_file() {
        for path in [
            "/maintenance/backups/id/file",
            "/playlists/id/export?format=json",
            "/plugins/development/template?kind=source",
            "/openapi.json",
            "/cache/id/file",
            "/downloads/id/file",
        ] {
            assert!(super::validate_export(path).is_ok());
        }
        for path in [
            "/auth/tokens",
            "/media/id",
            "/settings",
            "https://example.com/file",
            "/maintenance/backups/id/file/extra",
        ] {
            assert!(super::validate_export(path).is_err());
        }
    }
}
