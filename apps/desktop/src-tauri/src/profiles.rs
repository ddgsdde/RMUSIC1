use anyhow::{bail, Result};
use rmusic_bridge::connection::ConnectionProfile;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Serialize, Deserialize)]
struct Profiles {
    version: u32,
    items: Vec<ConnectionProfile>,
}
pub struct ProfileStore {
    path: PathBuf,
    items: Vec<ConnectionProfile>,
}
impl ProfileStore {
    pub fn open(directory: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(directory.as_ref())?;
        let path = directory.as_ref().join("profiles.json");
        let document: Profiles = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            Profiles {
                version: 1,
                items: vec![],
            }
        };
        if document.version != 1 {
            bail!("unsupported connection profile version")
        }
        for profile in &document.items {
            profile.validate()?;
        }
        Ok(Self {
            path,
            items: document.items,
        })
    }
    pub fn list(&self) -> Vec<ConnectionProfile> {
        self.items.clone()
    }
    pub fn get(&self, id: &str) -> Result<ConnectionProfile> {
        self.items
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown core profile"))
    }
    pub fn save(&mut self, profile: ConnectionProfile) -> Result<()> {
        if profile.id.is_empty() || profile.id.len() > 128 || profile.name.len() > 200 {
            bail!("invalid profile")
        }
        profile.validate()?;
        match self.items.iter().position(|p| p.id == profile.id) {
            Some(index) => self.items[index] = profile,
            None => self.items.push(profile),
        }
        self.persist()
    }
    pub fn remove(&mut self, id: &str) -> Result<()> {
        self.items.retain(|p| p.id != id);
        self.persist()
    }
    fn persist(&self) -> Result<()> {
        let temporary = self.path.with_extension("next");
        fs::write(
            &temporary,
            serde_json::to_vec(&Profiles {
                version: 1,
                items: self.items.clone(),
            })?,
        )?;
        fs::rename(temporary, &self.path)?;
        Ok(())
    }
}
