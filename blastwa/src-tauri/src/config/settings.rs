use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub chrome_path: String,
    pub chrome_version: String,
    pub default_delay_min: u64,
    pub default_delay_max: u64,
    pub active_profile: String,
    pub human_mode_preset: String,
    #[serde(default = "default_api_enabled")]
    pub api_enabled: bool,
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    #[serde(default)]
    pub wpp_last_check_at: Option<String>,
}

fn default_api_enabled() -> bool {
    false
}

fn default_api_port() -> u16 {
    8765
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            chrome_path: String::new(),
            chrome_version: String::new(),
            default_delay_min: 5,
            default_delay_max: 12,
            active_profile: "Default".into(),
            human_mode_preset: "natural".into(),
            api_enabled: false,
            api_port: 8765,
            wpp_last_check_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataPaths {
    pub profiles: PathBuf,
    pub accounts: PathBuf,
    pub data: PathBuf,
    pub reports: PathBuf,
    pub templates: PathBuf,
    pub wpp: PathBuf,
}

impl DataPaths {
    /// create every managed directory under app_dir; idempotent
    pub fn init_all(base: &Path) -> Result<DataPaths> {
        let paths = DataPaths {
            profiles: base.join("Profiles"),
            accounts: base.join("accounts"),
            data: base.join("Data"),
            reports: base.join("Reports"),
            templates: base.join("templates"),
            wpp: base.join("wpp"),
        };
        for dir in [
            &paths.profiles,
            &paths.accounts,
            &paths.data,
            &paths.reports,
            &paths.templates,
            &paths.wpp,
        ] {
            fs::create_dir_all(dir)
                .with_context(|| format!("failed to create dir {}", dir.display()))?;
        }
        Ok(paths)
    }

    pub fn account_dir(&self, name: &str) -> PathBuf {
        self.accounts.join(sanitize_name(name))
    }
}

pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => c,
            _ => '_',
        })
        .collect()
}

impl AppConfig {
    pub fn app_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("BlastWA")
    }

    pub fn config_path() -> PathBuf {
        Self::app_dir().join("config.json")
    }

    pub fn load_or_default() -> Self {
        match Self::load() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("config load failed ({e}), using defaults");
                Self::default()
            }
        }
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(&path).context("reading config.json")?;
        serde_json::from_str(&raw).context("parsing config.json")
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn init_data_dirs(&self) -> Result<DataPaths> {
        DataPaths::init_all(&Self::app_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_alnum_only() {
        assert_eq!(sanitize_name("My Profile!"), "My_Profile_");
        assert_eq!(sanitize_name("akun-1"), "akun-1");
        assert_eq!(sanitize_name(""), "");
    }

    #[test]
    fn default_config_roundtrip() {
        let c = AppConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let back: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.api_port, 8765);
        assert_eq!(back.default_delay_min, 5);
    }
}
