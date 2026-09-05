use blastwa_core::campaign::log_exporter::{
    export_csv, export_xlsx, load_campaign_records, CampaignRecord, LogEntry,
};
use blastwa_core::config::settings::AppConfig;

use tauri::State;

use super::super::AppCtx;

#[tauri::command]
pub(crate) fn get_logs(ctx: State<'_, AppCtx>) -> Result<Vec<LogEntry>, String> {
    let logs = ctx.logs.lock().unwrap();
    Ok(logs.iter().rev().take(500).cloned().collect())
}

/// persistent per-campaign history, newest first (U6)
#[tauri::command]
pub(crate) fn list_sent_campaigns() -> Vec<CampaignRecord> {
    load_campaign_records(&AppConfig::app_dir())
}

#[tauri::command]
pub(crate) fn export_log(
    format: String,
    path: String,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let logs = ctx.logs.lock().unwrap();
    let out = std::path::Path::new(&path);
    match format.as_str() {
        "csv" => export_csv(&logs, out).map_err(|e| e.to_string())?,
        "xlsx" => export_xlsx(&logs, out).map_err(|e| e.to_string())?,
        other => return Err(format!("unknown export format: {other}")),
    }
    Ok(serde_json::json!({ "ok": true, "entries": logs.len() }))
}
