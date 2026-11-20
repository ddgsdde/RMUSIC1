//! One-time migration from the development client's old software-gain default.
//! Core keeps the chosen fixed/software mode thereafter; this file stores only migration keys.
use anyhow::{Context, Result};
use std::{collections::BTreeSet, path::PathBuf};
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    migrated: BTreeSet<String>,
    fixed: BTreeSet<String>,
}
pub struct VolumeDefaults {
    path: PathBuf,
    saved: Saved,
}
impl VolumeDefaults {
    pub fn open(path: PathBuf) -> Result<Self> {
        let saved = match std::fs::read(&path) {
            Ok(bytes) => {
                anyhow::ensure!(bytes.len() <= 256 * 1024, "输出默认值记录过大");
                serde_json::from_slice(&bytes)
                    .or_else(|_| {
                        serde_json::from_slice::<BTreeSet<String>>(&bytes).map(|migrated| Saved {
                            migrated,
                            ..Saved::default()
                        })
                    })
                    .context("无法读取输出默认值记录")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, saved })
    }
    pub fn contains(&self, key: &str) -> bool {
        self.saved.migrated.contains(key)
    }
    pub fn fixed(&self, key: &str) -> bool {
        self.saved.fixed.contains(key)
    }
    pub fn mark(&mut self, key: String) -> Result<()> {
        anyhow::ensure!(self.saved.migrated.len() < 4096, "输出默认值记录已达上限");
        self.saved.migrated.insert(key);
        self.save()
    }
    pub fn set_fixed(&mut self, key: String, fixed: bool) -> Result<()> {
        if fixed {
            self.saved.fixed.insert(key);
        } else {
            self.saved.fixed.remove(&key);
        }
        self.save()
    }
    fn save(&self) -> Result<()> {
        let temporary = self
            .path
            .with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, serde_json::to_vec(&self.saved)?)?;
        std::fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}
