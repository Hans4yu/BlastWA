use blastwa_core::autoreply::rules::{self, Rule};
use blastwa_core::autoreply::watcher;

use tauri::State;

use super::super::AppCtx;

#[tauri::command]
pub(crate) fn load_rules(ctx: State<'_, AppCtx>) -> Result<Vec<Rule>, String> {
    let path = ctx.paths.data.join("autoreply.json");
    rules::load_rules(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn save_rules(
    rules: Vec<Rule>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let path = ctx.paths.data.join("autoreply.json");
    // rows without keyword or reply are dropped on the rust side too (the
    // frontend filters as well) — both layers must agree or a half-written
    // row could later match every incoming message
    let saved = rules::save_rules(&rules, &path).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "ok": true,
        "saved": saved,
        "skipped": rules.len().saturating_sub(saved),
    }))
}

/// live watcher telemetry for the Auto Reply page status strip
#[tauri::command]
pub(crate) fn autoreply_status(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let path = ctx.paths.data.join("autoreply.json");
    let rules = rules::load_rules(&path).unwrap_or_default();
    let armed = rules.iter().filter(|r| r.is_armed()).count();
    let st = watcher::stats();
    let watching = st
        .watching
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    Ok(serde_json::json!({
        "total_rules": rules.len(),
        "armed_rules": armed,
        "watching": watching,
        "replies_sent": st.replies_sent.load(std::sync::atomic::Ordering::Relaxed),
        "last_reply_epoch": st.last_reply_epoch.load(std::sync::atomic::Ordering::Relaxed),
    }))
}
