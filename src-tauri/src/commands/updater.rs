use blastwa_core::config::settings::AppConfig;
use blastwa_core::updater::wpp_updater;

use tauri::State;

use super::super::AppCtx;

#[tauri::command]
pub(crate) fn get_wpp_version(_ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "local": wpp_updater::current_version(&AppConfig::app_dir()),
    }))
}

#[tauri::command]
pub(crate) async fn check_wpp_update() -> Result<serde_json::Value, String> {
    let latest = wpp_updater::check_latest().await.map_err(|e| e.to_string())?;
    let current = wpp_updater::current_version(&AppConfig::app_dir());
    Ok(serde_json::json!({
        "latest": latest.tag_name,
        "current": current,
        "up_to_date": current.as_deref() == Some(latest.tag_name.as_str()),
    }))
}

#[tauri::command]
pub(crate) async fn update_wpp() -> Result<serde_json::Value, String> {
    let latest = wpp_updater::check_latest().await.map_err(|e| e.to_string())?;
    let tag = wpp_updater::update(&AppConfig::app_dir(), &latest)
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "version": tag }))
}
