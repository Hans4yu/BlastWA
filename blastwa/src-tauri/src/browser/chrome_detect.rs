// locate the user's Chrome install (registry first, then filesystem fallbacks).
use std::path::PathBuf;

use winreg::enums::*;
use winreg::{RegKey, RegKey as Hive};

pub fn find_chrome() -> Option<(PathBuf, String)> {
    let app_paths = |hive: Hive| -> Option<PathBuf> {
        let key = hive
            .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe")
            .ok()?;
        let p: String = key.get_value("").ok()?;
        let pb = PathBuf::from(p);
        pb.exists().then_some(pb)
    };

    if let Some(p) = app_paths(RegKey::predef(HKEY_LOCAL_MACHINE)) {
        return Some((p.clone(), read_version()));
    }

    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local).join(r"Google\Chrome\Application\chrome.exe");
        if p.exists() {
            return Some((p, read_version()));
        }
    }

    app_paths(RegKey::predef(HKEY_CURRENT_USER)).map(|p| (p, read_version()))
}

pub fn read_version() -> String {
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if let Ok(key) = RegKey::predef(hive).open_subkey(r"SOFTWARE\Google\Chrome\BLBeacon") {
            if let Ok(v) = key.get_value::<String, _>("version") {
                return v;
            }
        }
    }
    String::new()
}

pub fn chrome_major(version: &str) -> u32 {
    version
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
