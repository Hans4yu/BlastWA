// campaign profile persistence (U12): template + config + contacts as one unit
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::campaign::contact_list::ContactList;
use crate::campaign::sender::CampaignConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignProfile {
    pub name: String,
    pub messages: Vec<String>,
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub attachment_path: Option<String>,
    pub config: CampaignConfig,
    #[serde(default)]
    pub contacts: ContactList,
}

impl CampaignProfile {
    fn dir(base: &Path, name: &str) -> PathBuf {
        base.join(crate::config::settings::sanitize_name(name))
    }

    pub fn save(&self, profiles_dir: &Path) -> Result<PathBuf> {
        let dir = Self::dir(profiles_dir, &self.name);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("profile.json");
        crate::config::settings::atomic_write(&path, serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(path)
    }

    pub fn load(profiles_dir: &Path, name: &str) -> Result<Self> {
        let path = Self::dir(profiles_dir, name).join("profile.json");
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn list_names(profiles_dir: &Path) -> Vec<String> {
        let mut names = Vec::new();
        if let Ok(entries) = std::fs::read_dir(profiles_dir) {
            for e in entries.flatten() {
                if e.path().join("profile.json").exists() {
                    if let Some(n) = e.file_name().to_str() {
                        names.push(n.to_string());
                    }
                }
            }
        }
        names.sort();
        names
    }
}
