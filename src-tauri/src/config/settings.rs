use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const STORAGE_SCHEMA_VERSION: u32 = 1;

pub struct FileLock {
    path: PathBuf,
}

impl FileLock {
    pub fn acquire(target: &Path) -> std::io::Result<Self> {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let path = target.with_extension("lock");
        for _ in 0..40 {
            match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "storage lock timeout"))
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|ext| ext.to_str()).unwrap_or("data")
    ));
    {
        use std::io::Write;
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temp, path)
}

pub fn backup_corrupt_file(path: &Path) {
    if !path.exists() {
        return;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup = path.with_file_name(format!(
        "{}.corrupt-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("data"),
        stamp
    ));
    let _ = fs::copy(path, backup);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
    #[serde(default = "default_api_token")]
    pub api_token: String,
    #[serde(default)]
    pub wpp_last_check_at: Option<String>,
}

fn default_api_enabled() -> bool {
    false
}

fn default_api_port() -> u16 {
    8765
}

fn default_api_token() -> String {
    uuid::Uuid::new_v4().to_string()
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
            api_token: default_api_token(),
            wpp_last_check_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataPaths {
    pub profiles: PathBuf,
    pub accounts: PathBuf,
    pub data: PathBuf,
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
            templates: base.join("templates"),
            wpp: base.join("wpp"),
        };
        for dir in [
            &paths.profiles,
            &paths.accounts,
            &paths.data,
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

// process-global launcher profile, set exactly once at startup before any
// config load. absent means classic root (no profile isolation).
static ACTIVE_PROFILE: OnceLock<String> = OnceLock::new();

/// pure resolution logic so tests can exercise it without touching the
/// process-global profile state
fn resolve_app_dir(base: &Path, profile: Option<&str>) -> PathBuf {
    match profile {
        // re-sanitize here so the returned path is always a single safe
        // segment even if a caller skipped init validation
        Some(p) => base.join("profiles").join(sanitize_name(p)),
        None => base.to_path_buf(),
    }
}

fn merge_chrome_fallback(profile: &AppConfig, classic: &AppConfig) -> AppConfig {
    let mut merged = profile.clone();
    if merged.chrome_path.is_empty() {
        merged.chrome_path = classic.chrome_path.clone();
    }
    if merged.chrome_version.is_empty() {
        merged.chrome_version = classic.chrome_version.clone();
    }
    merged
}

fn resolve_profile_config(profile: AppConfig, classic: Result<AppConfig>) -> AppConfig {
    let classic = classic.unwrap_or_default();
    merge_chrome_fallback(&profile, &classic)
}

fn load_profile_config(profile_path: &Path, classic_path: &Path) -> AppConfig {
    let profile = match AppConfig::load_from_path(profile_path) {
        Ok(config) => config,
        Err(error) => {
            log::warn!(
                "profile config load failed ({}), using classic Chrome fallback",
                error
            );
            AppConfig::default()
        }
    };
    resolve_profile_config(profile, AppConfig::load_from_path(classic_path))
}

impl AppConfig {
    /// classic data root, ignoring any active profile (used by the launcher
    /// to scan profiles/ across all instances)
    pub fn classic_root() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("BlastWA")
    }

    /// activate launcher profile isolation for this process; fails when the
    /// sanitized name collapses to an empty directory segment
    pub fn init_profile(name: &str) -> Result<(), String> {
        let safe = sanitize_name(name);
        if safe.is_empty() {
            return Err("profile name resolves to an empty directory segment".into());
        }
        let _ = ACTIVE_PROFILE.set(safe);
        Ok(())
    }

    /// active profile name, if launcher isolation was activated in main()
    pub fn active_profile() -> Option<&'static str> {
        ACTIVE_PROFILE.get().map(|s| s.as_str())
    }

    pub fn app_dir() -> PathBuf {
        resolve_app_dir(&Self::classic_root(), Self::active_profile())
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
        if Self::active_profile().is_some() {
            let classic_path = Self::classic_root().join("config.json");
            return Ok(load_profile_config(&path, &classic_path));
        }
        Self::load_from_path(&path)
    }

    fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path).context("reading config.json")?;
        match serde_json::from_str(&raw) {
            Ok(config) => Ok(config),
            Err(error) => {
                backup_corrupt_file(path);
                Err(error).context("parsing config.json")
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, serde_json::to_string_pretty(self)?.as_bytes())?;
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

    #[test]
    fn atomic_write_replaces_file_without_temp_artifact() {
        let dir = std::env::temp_dir().join(format!("blastwa_atomic_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("value.json");
        atomic_write(&path, br#"{"ok":true}"#).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"ok":true}"#);
        assert!(!path.with_extension("json.tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_without_profile_is_classic_root() {
        let base = Path::new("C:\\Users\\x\\AppData\\Roaming\\BlastWA");
        assert_eq!(resolve_app_dir(base, None), base);
    }

    #[test]
    fn resolve_with_profile_lands_under_profiles() {
        let base = Path::new("C:\\data\\BlastWA");
        let dir = resolve_app_dir(base, Some("work"));
        assert_eq!(dir, Path::new("C:\\data\\BlastWA\\profiles\\work"));
    }

    #[test]
    fn resolve_sanitizes_illegal_profile_names() {
        let base = Path::new("C:\\data\\BlastWA");
        // path separators and dots collapse into one underscore-joined
        // segment: no traversal possible
        let dir = resolve_app_dir(base, Some("..\\evil"));
        assert_eq!(dir, Path::new("C:\\data\\BlastWA\\profiles\\___evil"));
        assert!(dir.components().count() == base.join("profiles").components().count() + 1);
    }

    #[test]
    fn init_profile_rejects_only_truly_empty_names() {
        // sanitize maps every illegal char to underscore, so " /// " becomes
        // "___" (ugly but safe); only the empty string collapses to an empty
        // segment and must be rejected at startup
        assert!(AppConfig::init_profile("").is_err());
        assert!(AppConfig::init_profile(" /// ").is_ok());
    }

    #[test]
    fn chrome_fallback_fills_missing_profile_values_without_mutating_profile() {
        // Given: a profile has no Chrome values and the classic config has the
        // installer-detected runtime plus unrelated settings.
        let profile = AppConfig::default();
        let classic = AppConfig { chrome_path: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe".into(), chrome_version: "128.0.6613.120".into(), default_delay_min: 21, default_delay_max: 34, active_profile: "Classic".into(), human_mode_preset: "cautious".into(), api_enabled: true, api_port: 9876, ..AppConfig::default() };

        // When: the profile is resolved against the classic fallback.
        let merged = merge_chrome_fallback(&profile, &classic);
        let expected = AppConfig {
            chrome_path: classic.chrome_path.clone(),
            chrome_version: classic.chrome_version.clone(),
            ..profile.clone()
        };

        // Then: only the missing Chrome fields come from classic, and the
        // input profile remains unchanged for read-time-only fallback.
        assert_eq!(merged, expected);
        assert!(profile.chrome_path.is_empty() && profile.chrome_version.is_empty());
    }

    #[test]
    fn chrome_fallback_preserves_non_empty_profile_values_even_when_invalid() {
        // Given: a profile contains explicit, non-empty Chrome values that do
        // not point to a valid executable.
        let profile = AppConfig {
            chrome_path: "C:\\missing\\profile-chrome.exe".into(),
            chrome_version: "profile-version".into(),
            ..AppConfig::default()
        };
        let classic = AppConfig {
            chrome_path: "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe".into(),
            chrome_version: "classic-version".into(),
            ..AppConfig::default()
        };

        // When: the profile is resolved against the classic fallback.
        let merged = merge_chrome_fallback(&profile, &classic);

        // Then: explicit profile values remain authoritative.
        assert_eq!(merged.chrome_path, profile.chrome_path);
        assert_eq!(merged.chrome_version, profile.chrome_version);
    }

    #[test]
    fn chrome_fallback_inherits_each_missing_value_independently() {
        // Given: only the profile path is missing while its version is explicit.
        let profile = AppConfig {
            chrome_version: "profile-version".into(),
            ..AppConfig::default()
        };
        let classic = AppConfig {
            chrome_path: "classic-path".into(),
            chrome_version: "classic-version".into(),
            ..AppConfig::default()
        };

        // When: the profile is resolved against the classic fallback.
        let merged = merge_chrome_fallback(&profile, &classic);

        // Then: the missing path is inherited but the explicit version wins.
        assert_eq!(merged.chrome_path, classic.chrome_path);
        assert_eq!(merged.chrome_version, profile.chrome_version);
    }

    #[test]
    fn malformed_classic_config_does_not_break_profile_resolution() {
        // Given: a usable profile config and an unreadable classic fallback.
        let profile = AppConfig::default();
        let classic = Err(anyhow::anyhow!("malformed classic config"));

        // When: the profile is resolved against the failed classic fallback.
        let merged = resolve_profile_config(profile, classic);

        // Then: the profile remains usable with its own default Chrome values.
        assert!(merged.chrome_path.is_empty());
        assert!(merged.chrome_version.is_empty());
    }

    #[test]
    fn profile_file_loading_falls_back_without_rewriting_profile_data() {
        // Given: a valid classic config and a temporary profile config path.
        let root = std::env::temp_dir().join(format!(
            "blastwa-profile-config-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        assert!(fs::create_dir_all(&root).is_ok());
        let profile_path = root.join("profile.json");
        let classic_path = root.join("classic.json");
        let classic_json = r#"{
            "chrome_path": "classic-path",
            "chrome_version": "classic-version",
            "default_delay_min": 31,
            "default_delay_max": 32,
            "active_profile": "classic",
            "human_mode_preset": "cautious",
            "api_enabled": true,
            "api_port": 9876
        }"#;
        assert!(fs::write(&classic_path, classic_json).is_ok());

        // When: the profile file is missing, then malformed, then explicit.
        let missing = load_profile_config(&profile_path, &classic_path);
        assert_eq!(missing.chrome_path, "classic-path");
        assert_eq!(missing.chrome_version, "classic-version");
        assert!(!profile_path.exists());

        let malformed_json = "{ malformed profile";
        assert!(fs::write(&profile_path, malformed_json).is_ok());
        let malformed_before = fs::read_to_string(&profile_path).unwrap_or_default();
        assert!(AppConfig::load_from_path(&profile_path).is_err());
        let malformed = load_profile_config(&profile_path, &classic_path);
        assert_eq!(malformed.chrome_path, "classic-path");
        assert_eq!(malformed.chrome_version, "classic-version");
        assert_eq!(fs::read_to_string(&profile_path).unwrap_or_default(), malformed_before);

        let explicit_json = r#"{
            "chrome_path": "profile-path",
            "chrome_version": "profile-version",
            "default_delay_min": 3,
            "default_delay_max": 4,
            "active_profile": "profile",
            "human_mode_preset": "natural",
            "api_enabled": false,
            "api_port": 7654
        }"#;
        assert!(fs::write(&profile_path, explicit_json).is_ok());
        let explicit_before = fs::read_to_string(&profile_path).unwrap_or_default();
        let explicit = load_profile_config(&profile_path, &classic_path);
        assert_eq!(explicit.chrome_path, "profile-path");
        assert_eq!(explicit.chrome_version, "profile-version");
        assert_eq!(explicit.default_delay_min, 3);
        assert_eq!(explicit.default_delay_max, 4);
        assert_eq!(explicit.active_profile, "profile");
        assert_eq!(explicit.human_mode_preset, "natural");
        assert!(!explicit.api_enabled);
        assert_eq!(explicit.api_port, 7654);
        assert_eq!(fs::read_to_string(&profile_path).unwrap_or_default(), explicit_before);

        // Then: the temporary files are removed after the file-loading checks.
        assert!(fs::remove_dir_all(&root).is_ok());
    }
}
