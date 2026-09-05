use blastwa_core::api::server;
use blastwa_core::account::service::AccountStatus;
use blastwa_core::error::AppError;
use serde_json::Value;
use tauri::State;

use super::super::AppCtx;

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Account name is required".into());
    }
    if name.len() > 64 {
        return Err("Account name is too long (max 64 characters)".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("Account name may only contain letters, numbers, underscore and dash".into());
    }
    Ok(())
}

pub(crate) async fn live_session_port(name: &str) -> Option<u16> {
    let registry = server::sessions_registry();
    let sessions = registry.lock().await;
    sessions.iter().find(|(n, _)| n == name).map(|(_, port)| *port)
}

pub(crate) async fn session_alive(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok()
}

pub(crate) fn format_launch_failure(error: Option<&str>) -> String {
    match error {
        Some(error) => format!(
            "chrome launch failed: {error}; chrome cdp endpoint not found after launch"
        ),
        None => "chrome cdp endpoint not found after launch".to_string(),
    }
}

pub(crate) async fn register_live_session(name: &str, port: u16) {
    let registry = server::sessions_registry();
    let mut sessions = registry.lock().await;
    if !sessions.iter().any(|(n, _)| n == name) {
        sessions.push((name.to_string(), port));
    }
}

#[tauri::command]
pub(crate) async fn list_accounts(ctx: State<'_, AppCtx>) -> Result<Vec<AccountStatus>, AppError> {
    super::super::list_accounts_impl(ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn add_account(name: String, ctx: State<'_, AppCtx>) -> Result<Value, AppError> {
    super::super::add_account_impl(name, ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn rename_account(
    old_name: String,
    new_name: String,
    ctx: State<'_, AppCtx>,
) -> Result<Value, AppError> {
    super::super::rename_account_impl(old_name, new_name, ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn remove_account(
    name: String,
    delete_profile: Option<bool>,
    ctx: State<'_, AppCtx>,
) -> Result<Value, AppError> {
    super::super::remove_account_impl(name, delete_profile, ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn remove_all_accounts(
    delete_profiles: Option<bool>,
    ctx: State<'_, AppCtx>,
) -> Result<Value, AppError> {
    super::super::remove_all_accounts_impl(delete_profiles, ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn open_browser(name: String, ctx: State<'_, AppCtx>) -> Result<Value, AppError> {
    super::super::open_browser_impl(name, ctx).await.map_err(AppError::from)
}
