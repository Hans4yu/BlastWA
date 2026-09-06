use blastwa_core::account::registry;
use blastwa_core::api::server;
use blastwa_core::config::settings::AppConfig;

use tauri::State;

use super::super::AppCtx;

#[tauri::command]
pub(crate) fn get_config(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let cfg = ctx.cfg.lock().unwrap();
    Ok(serde_json::json!({
        "chrome_path": cfg.chrome_path,
        "chrome_version": cfg.chrome_version,
        "default_delay_min": cfg.default_delay_min,
        "default_delay_max": cfg.default_delay_max,
        "human_mode_preset": cfg.human_mode_preset,
        "api_enabled": cfg.api_enabled,
        "api_port": cfg.api_port,
        "api_token": cfg.api_token,
        "wpp_last_check_at": cfg.wpp_last_check_at,
        "active_profile": AppConfig::active_profile(),
    }))
}

#[tauri::command]
pub(crate) fn save_config(
    default_delay_min: Option<u64>,
    default_delay_max: Option<u64>,
    human_mode_preset: Option<String>,
    api_enabled: Option<bool>,
    api_port: Option<u16>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let mut cfg = ctx.cfg.lock().unwrap();
    if let Some(v) = default_delay_min { cfg.default_delay_min = v; }
    if let Some(v) = default_delay_max { cfg.default_delay_max = v; }
    if let Some(v) = human_mode_preset { cfg.human_mode_preset = v; }
    if let Some(v) = api_enabled { cfg.api_enabled = v; }
    if let Some(v) = api_port { cfg.api_port = v; }
    cfg.save().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
pub(crate) async fn get_health_diagnostics(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let cfg = ctx.cfg.lock().unwrap().clone();
    let names = ctx.account_service.load_names();
    let sessions = server::sessions_registry();
    let live = sessions.lock().await;
    let pruned = registry::prune(&AppConfig::classic_root()).unwrap_or_default();
    Ok(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "profile": AppConfig::active_profile(),
        "storage_path": AppConfig::app_dir(),
        "storage_exists": AppConfig::app_dir().exists(),
        "chrome": {
            "path": cfg.chrome_path,
            "configured": !cfg.chrome_path.is_empty(),
            "exists": !cfg.chrome_path.is_empty() && std::path::Path::new(&cfg.chrome_path).exists(),
            "version": cfg.chrome_version,
        },
        "api": {
            "enabled": cfg.api_enabled,
            "port": cfg.api_port,
            "authenticated": !cfg.api_token.is_empty(),
        },
        "accounts": {
            "saved": names.len(),
            "live_sessions": live.len(),
        },
        "profile_processes": pruned.len(),
    }))
}
