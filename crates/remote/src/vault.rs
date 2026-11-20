use anyhow::{ensure, Context, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use rand::RngCore;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

/// Bound allocation while reading; checking length after fs::read is too late.
pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file=std::fs::File::open(path)?;
    let metadata=file.metadata()?;
    ensure!(metadata.is_file() && metadata.len()<=limit as u64,"文件超过大小限制或不是普通文件");
    let mut bytes=Vec::new();
    file.take(limit as u64+1).read_to_end(&mut bytes)?;
    ensure!(bytes.len()<=limit,"文件超过大小限制");
    Ok(bytes)
}

pub fn private_directory(path: &Path) -> Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        ensure!(
            !meta.file_type().is_symlink() && meta.is_dir(),
            "私有目录不可为链接或普通文件"
        );
    }
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn atomic_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("文件缺少目录")?;
    private_directory(parent)?;
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "私有文件不可为链接"
        );
    }
    let temp = parent.join(format!(".{}.next", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
pub fn private_key(path: &Path) -> Result<[u8; 32]> {
    private_directory(path.parent().context("缺少私钥目录")?)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(&key)?;
            file.sync_all()?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let m = std::fs::symlink_metadata(path)?;
            ensure!(
                m.is_file() && !m.file_type().is_symlink(),
                "私钥文件不可为链接"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                ensure!(m.permissions().mode() & 0o077 == 0, "私钥文件权限过宽");
            }
            let bytes = Zeroizing::new(std::fs::read(path)?);
            ensure!(bytes.len() == 32, "私钥文件无效");
            key.copy_from_slice(&bytes);
        }
        Err(e) => return Err(e.into()),
    };
    Ok(key)
}
pub struct Vault {
    root: PathBuf,
    key: Zeroizing<[u8; 32]>,
}
impl Vault {
    pub fn open(root: &Path) -> Result<Self> {
        private_directory(root)?;
        Ok(Self {
            root: root.into(),
            key: Zeroizing::new(private_key(&root.join("vault.key"))?),
        })
    }
    fn path(&self, name: &str) -> Result<PathBuf> {
        ensure!(
            !name.is_empty()
                && name.len() <= 100
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "密钥名称无效"
        );
        Ok(self.root.join(format!("{name}.sealed")))
    }
    pub fn put<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let path = self.path(name)?;
        let plain = Zeroizing::new(serde_json::to_vec(value)?);
        let mut nonce = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let cipher = ChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("密钥格式无效"))?;
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("密钥封装失败"))?;
        let mut result = b"RMUSICVAULT1".to_vec();
        result.extend(nonce);
        result.extend(encrypted);
        atomic_private(&path, &result)
    }
    pub fn get<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        let path = self.path(name)?;
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 8 * 1024 * 1024, "密钥文件类型或大小无效");
        let data = match std::fs::read(path) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            data.len() >= 40 && data.starts_with(b"RMUSICVAULT1"),
            "密钥存储格式无效"
        );
        let cipher = ChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("密钥格式无效"))?;
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&data[12..24]),
                    Payload {
                        msg: &data[24..],
                        aad: name.as_bytes(),
                    },
                )
                .map_err(|_| anyhow::anyhow!("密钥校验失败"))?,
        );
        Ok(Some(serde_json::from_slice(&plain)?))
    }
    pub fn remove(&self, name: &str) -> Result<()> {
        match std::fs::remove_file(self.path(name)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sealed_values_are_name_bound_and_tamper_evident() {
        let d = tempfile::tempdir().unwrap();
        let v = Vault::open(d.path()).unwrap();
        v.put("cloudflare", &"secret-example").unwrap();
        let raw = std::fs::read(d.path().join("cloudflare.sealed")).unwrap();
        assert!(!raw.windows(14).any(|b| b == b"secret-example"));
        assert_eq!(
            v.get::<String>("cloudflare").unwrap().unwrap(),
            "secret-example"
        );
        std::fs::write(d.path().join("other.sealed"), &raw).unwrap();
        assert!(v.get::<String>("other").is_err());
        assert!(v.put("../escape", &1).is_err());
    }
}
