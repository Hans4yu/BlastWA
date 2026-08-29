// blastwa-setup: detects the user's Chrome install, records it in config,
// then hands off to the main app. no chrome -> native error dialog -> exit 1.
use std::path::PathBuf;
use std::process::Command;

use winreg::enums::*;
use winreg::RegKey;

const APP_DIR_NAME: &str = "BlastWA";
const MIN_CHROME_MAJOR: u32 = 115;

struct ChromeInstall {
    path: PathBuf,
    version: String,
}

fn find_chrome() -> Option<ChromeInstall> {
    let app_paths = |hive: RegKey| -> Option<ChromeInstall> {
        let key = hive
            .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe")
            .ok()?;
        let p: String = key.get_value("").ok()?;
        let pb = PathBuf::from(&p);
        pb.exists()
            .then(|| ChromeInstall { path: pb, version: read_version() })
    };

    // path 1: machine-wide App Paths registry
    if let Some(hit) = app_paths(RegKey::predef(HKEY_LOCAL_MACHINE)) {
        return Some(hit);
    }

    // path 2: per-user install under LOCALAPPDATA
    if let Some(local) = dirs_localappdata() {
        let candidate = local.join(r"Google\Chrome\Application\chrome.exe");
        if candidate.exists() {
            return Some(ChromeInstall {
                path: candidate,
                version: read_version(),
            });
        }
    }

    // path 3: per-user App Paths
    app_paths(RegKey::predef(HKEY_CURRENT_USER))
}

fn read_version() -> String {
    // BLBeacon holds the friendly version string; try HKLM then HKCU
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if let Ok(key) = RegKey::predef(hive).open_subkey(r"SOFTWARE\Google\Chrome\BLBeacon") {
            if let Ok(v) = key.get_value::<String, _>("version") {
                return v;
            }
        }
    }
    String::new()
}

fn chrome_major(version: &str) -> u32 {
    version
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0)
}

fn dirs_localappdata() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA").ok().map(PathBuf::from)
}

fn app_data_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join(APP_DIR_NAME)
}

#[cfg(windows)]
fn show_error_dialog(msg: &str, title: &str) {
    use std::ffi::OsStr;
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(once(0)).collect()
    }
    extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, utype: u32) -> i32;
    }
    unsafe {
        MessageBoxW(0, wide(msg).as_ptr(), wide(title).as_ptr(), 0x10);
    }
}

#[cfg(not(windows))]
fn show_error_dialog(msg: &str, _title: &str) {
    eprintln!("{msg}");
}

fn main() {
    let chrome = match find_chrome() {
        Some(c) => c,
        None => {
            show_error_dialog(
                "Google Chrome was not found on this system.\n\n\
                 BlastWA requires Google Chrome (version 115 or newer).\n\n\
                 Please install Chrome from https://www.google.com/chrome and run setup again.",
                "BlastWA Setup",
            );
            std::process::exit(1);
        }
    };

    let major = chrome_major(&chrome.version);
    if major < MIN_CHROME_MAJOR {
        eprintln!(
            "warning: Chrome version {} is below recommended minimum {}",
            chrome.version, MIN_CHROME_MAJOR
        );
    }

    let app_dir = app_data_dir();
    std::fs::create_dir_all(&app_dir).expect("failed to create app data dir");

    let config_path = app_dir.join("config.json");
    let mut config: serde_json::Value = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    config["chrome_path"] = serde_json::Value::String(chrome.path.to_string_lossy().to_string());
    config["chrome_version"] = serde_json::Value::String(chrome.version.clone());

    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap())
        .expect("failed to write config");

    println!("chrome found: {}", chrome.path.display());
    println!("version: {}", chrome.version);
    println!("config written: {}", config_path.display());

    // hand off to main app when it sits next to this binary
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));

    if let Some(dir) = exe_dir {
        let app_exe = dir.join("blastwa.exe");
        if app_exe.exists() {
            match Command::new(&app_exe).spawn() {
                Ok(_) => println!("launched {}", app_exe.display()),
                Err(e) => show_error_dialog(
                    &format!("Failed to launch BlastWA: {e}"),
                    "BlastWA Setup",
                ),
            }
        } else {
            println!("main app not found at {}, skipping launch", app_exe.display());
        }
    }
}
