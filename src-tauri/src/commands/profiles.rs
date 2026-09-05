use blastwa_core::account::registry;
use blastwa_core::config::settings::AppConfig;

#[cfg(windows)]
fn create_desktop_profile_shortcut(profile_name: &str, exe_path: &std::path::Path) -> Result<(), String> {
    let desktop_dir = dirs::desktop_dir().ok_or_else(|| "cannot locate user desktop directory".to_string())?;
    let lnk_path = desktop_dir.join(format!("BlastWA - {profile_name}.lnk"));
    let exe_str = exe_path.to_str().ok_or_else(|| "invalid exe path".to_string())?;
    let lnk_str = lnk_path.to_str().ok_or_else(|| "invalid lnk path".to_string())?;

    let ps_script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{}'); \
         $s.TargetPath = '{}'; \
         $s.Arguments = '--profile \"{}\"'; \
         $s.WorkingDirectory = '{}'; \
         $s.IconLocation = '{},0'; \
         $s.Save()",
        lnk_str.replace('\'', "''"),
        exe_str.replace('\'', "''"),
        profile_name.replace('"', "`\""),
        exe_path.parent().and_then(|p| p.to_str()).unwrap_or("").replace('\'', "''"),
        exe_str.replace('\'', "''")
    );

    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", &ps_script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let out = cmd.output().map_err(|e| format!("failed to execute shortcut creator: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("powershell shortcut creation failed: {err}"));
    }
    log::info!("created desktop shortcut: {}", lnk_path.display());
    Ok(())
}

#[cfg(not(windows))]
fn create_desktop_profile_shortcut(_profile_name: &str, _exe_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

/// spawn a fully isolated second instance bound to its own data root.
/// the child re-enters main() which resolves --profile before any config
/// load, so every storage path isolates without further wiring.
#[tauri::command]
pub(crate) fn open_profile_window(profile: String, create_shortcut: Option<bool>) -> Result<(), String> {
    let safe = blastwa_core::config::settings::sanitize_name(&profile);
    if safe.is_empty() {
        return Err("Profile name is required".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate exe: {e}"))?;

    if create_shortcut.unwrap_or(false) {
        if let Err(e) = create_desktop_profile_shortcut(&profile, &exe) {
            log::warn!("failed to create desktop shortcut for profile '{profile}': {e}");
        }
    }

    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--profile").arg(&safe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn()
        .map_err(|e| format!("failed to spawn profile window: {e}"))
        .and_then(|child| {
            registry::register(
                &AppConfig::classic_root(),
                registry::ProfileProcess { name: safe.clone(), pid: child.id() },
            )
            .map_err(|e| format!("recording profile process: {e}"))
        })?;
    log::info!("spawned profile window: {safe}");
    Ok(())
}

/// existing profiles on disk (classic root scan, sorted); empty when none
#[tauri::command]
pub(crate) fn list_profiles() -> Vec<String> {
    let dir = AppConfig::classic_root().join("profiles");
    let _ = registry::prune(&AppConfig::classic_root());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}
