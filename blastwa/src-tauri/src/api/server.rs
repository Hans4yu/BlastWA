// local REST API server (U14): axum bound to 127.0.0.1 only.
// lets external systems trigger blasts without opening the UI.
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct AppState {
    pub running: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
    pub sent: Arc<std::sync::atomic::AtomicU32>,
    pub failed: Arc<std::sync::atomic::AtomicU32>,
    /// contacts queued for the current campaign (for pending restore)
    pub total: Arc<std::sync::atomic::AtomicU32>,
    /// set by the gui layer: triggers campaign start through the normal pipeline
    pub blast_requested: Arc<tokio::sync::mpsc::Sender<BlastRequest>>,
    /// current campaign cancel token; overwritten at each campaign start
    pub stop_flag: Arc<tokio::sync::Mutex<tokio_util::sync::CancellationToken>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BlastRequest {
    pub account: String,
    pub contacts: Vec<String>,
    pub message: String,
    #[serde(default = "default_delay_min")]
    pub delay_min_s: f64,
    #[serde(default = "default_delay_max")]
    pub delay_max_s: f64,
}

fn default_delay_min() -> f64 {
    3.0
}

fn default_delay_max() -> f64 {
    9.0
}

#[derive(Serialize)]
struct ApiResponse<T> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn ok<T>(data: T) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::OK,
        Json(ApiResponse { ok: true, data: Some(data), error: None }),
    )
}

fn err<T>(msg: &str, code: StatusCode) -> (StatusCode, Json<ApiResponse<T>>) {
    (code, Json(ApiResponse { ok: false, data: None, error: Some(msg.into()) }))
}

async fn blast(
    State(state): State<AppState>,
    Json(req): Json<BlastRequest>,
) -> (StatusCode, Json<ApiResponse<String>>) {
    if state.running.load(Ordering::Relaxed) {
        return err::<String>("campaign already running", StatusCode::CONFLICT);
    }
    if req.contacts.is_empty() || req.message.is_empty() {
        return err::<String>(
            "contacts and message are required",
            StatusCode::BAD_REQUEST,
        );
    }
    match state.blast_requested.clone().send(req).await {
        Ok(_) => ok("campaign queued".into()),
        Err(_) => err::<String>("internal dispatch failed", StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn status(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse<StatusData>>) {
    let data = StatusData {
        running: state.running.load(Ordering::Relaxed),
        sent: state.sent.load(Ordering::Relaxed),
        failed: state.failed.load(Ordering::Relaxed),
    };
    ok(data)
}

#[derive(Serialize)]
struct StatusData {
    running: bool,
    sent: u32,
    failed: u32,
}

async fn stop(State(state): State<AppState>) -> (StatusCode, Json<ApiResponse<String>>) {
    state.stop_flag.lock().await.cancel();
    ok("stop requested".into())
}

#[derive(Serialize)]
struct AccountInfo {
    name: String,
    port: u16,
}

/// live session registry — pipeline pushes/removes, this just reads
static LIVE_SESSIONS: std::sync::OnceLock<
    Arc<tokio::sync::Mutex<Vec<(String, u16)>>>,
> = std::sync::OnceLock::new();

pub fn sessions_registry() -> Arc<tokio::sync::Mutex<Vec<(String, u16)>>> {
    LIVE_SESSIONS
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(Vec::new())))
        .clone()
}

// ---------- persistent account identity store ----------
// only account names live on disk. cdp ports, connected flags, and browser
// sessions are runtime state and stay in the in-memory registry above.

pub fn accounts_file(app_dir: &Path) -> PathBuf {
    app_dir.join("accounts.json")
}

/// account identities saved on disk; empty when missing or corrupt
pub fn load_saved_accounts(app_dir: &Path) -> Vec<String> {
    let path = accounts_file(app_dir);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        log::warn!("accounts.json unreadable ({}), starting from empty list", e);
        Vec::new()
    })
}

pub fn save_account_name(app_dir: &Path, name: &str) -> std::io::Result<()> {
    let mut names = load_saved_accounts(app_dir);
    if !names.iter().any(|n| n == name) {
        names.push(name.to_string());
    }
    std::fs::create_dir_all(app_dir)?;
    std::fs::write(accounts_file(app_dir), serde_json::to_string_pretty(&names)?)
}

pub fn remove_saved_account(app_dir: &Path, name: &str) -> std::io::Result<()> {
    let mut names = load_saved_accounts(app_dir);
    names.retain(|n| n != name);
    std::fs::create_dir_all(app_dir)?;
    std::fs::write(accounts_file(app_dir), serde_json::to_string_pretty(&names)?)
}

async fn accounts(
    State(_state): State<AppState>,
) -> (StatusCode, Json<ApiResponse<Vec<AccountInfo>>>) {
    let reg = sessions_registry();
    let list = reg.lock().await;
    let data = list
        .iter()
        .map(|(name, port)| AccountInfo { name: name.clone(), port: *port })
        .collect();
    ok(data)
}

/// bind loopback ONLY — never 0.0.0.0. enforced here in code, not config.
pub async fn serve(port: u16, state: AppState) -> Result<()> {
    let app = Router::new()
        .route("/api/blast", post(blast))
        .route("/api/status", get(status))
        .route("/api/accounts", get(accounts))
        .route("/api/stop", post(stop))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    log::info!("local api listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
