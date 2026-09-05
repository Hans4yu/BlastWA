use blastwa_core::autoreply::rules::{self, Rule};

use tauri::State;

use super::super::AppCtx;

#[tauri::command]
pub(crate) fn load_rules(ctx: State<'_, AppCtx>) -> Result<Vec<Rule>, String> {
    let path = ctx.paths.data.join("autoreply.json");
    rules::load_rules(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn save_rules(rules: Vec<Rule>, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let path = ctx.paths.data.join("autoreply.json");
    rules::save_rules(&rules, &path).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true }))
}
