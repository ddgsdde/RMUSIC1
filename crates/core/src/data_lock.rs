use anyhow::{Context,Result};
use std::{fs::{File,OpenOptions},path::Path};

/// Shared with the offline backup tool; hold this handle for the Core's lifetime.
pub fn acquire(data:&Path)->Result<File> {
    let mut file=OpenOptions::new().read(true).write(true).create(true).truncate(false).open(data.join(".core.lock"))?;
    file.try_lock().context("数据目录正由另一个核心或备份任务使用")?;
    file.set_len(0)?;
    serde_json::to_writer(&mut file,&serde_json::json!({"core_version":env!("CARGO_PKG_VERSION")}))?;
    file.sync_all()?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    #[test]
    fn directory_lock_excludes_another_owner_and_releases_on_close(){
        let dir=tempfile::tempdir().unwrap();let first=super::acquire(dir.path()).unwrap();
        assert!(super::acquire(dir.path()).is_err());drop(first);
        assert!(super::acquire(dir.path()).is_ok());
    }
}
