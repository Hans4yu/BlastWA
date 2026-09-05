// local REST API server (U14): axum bound to 127.0.0.1 only.
// lets external systems trigger blasts without opening the UI.
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
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
    pub api_token: Arc<String>,
}

fn token_is_valid(headers: &HeaderMap, expected: &str) -> bool {
    let supplied = headers
        .get("x-blastwa-token")
        .and_then(|value| value.to_str().ok());
    supplied == Some(expected)
}

async fn authorize(State(state): State<AppState>, headers: HeaderMap, request: axum::http::Request<axum::body::Body>, next: Next) -> Response {
    if !token_is_valid(&headers, state.api_token.as_str()) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid X-BlastWA-Token").into_response();
    }
    next.run(request).await
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

fn write_accounts(app_dir: &Path, names: &[String]) -> std::io::Result<()> {
    let path = accounts_file(app_dir);
    let _lock = crate::config::settings::FileLock::acquire(&path)?;
    let temp = path.with_extension("json.tmp");
    let raw = serde_json::to_vec_pretty(&serde_json::json!({
        "schema_version": crate::config::settings::STORAGE_SCHEMA_VERSION,
        "accounts": names,
    }))?;
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&raw)?;
        file.sync_all()?;
    }
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    std::fs::rename(temp, path)
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
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(value) => value
            .as_array()
            .map(|names| names.iter().filter_map(|name| name.as_str().map(str::to_string)).collect())
            .or_else(|| value.get("accounts").and_then(|names| names.as_array()).map(|names| names.iter().filter_map(|name| name.as_str().map(str::to_string)).collect()))
            .unwrap_or_default(),
        Err(error) => {
            crate::config::settings::backup_corrupt_file(&path);
            log::warn!("accounts.json unreadable ({}), starting from empty list", error);
            Vec::new()
        }
    }
}

pub fn save_account_name(app_dir: &Path, name: &str) -> std::io::Result<()> {
    let mut names = load_saved_accounts(app_dir);
    if !names.iter().any(|n| n == name) {
        names.push(name.to_string());
    }
    write_accounts(app_dir, &names)
}

pub fn remove_saved_account(app_dir: &Path, name: &str) -> std::io::Result<()> {
    let mut names = load_saved_accounts(app_dir);
    names.retain(|n| n != name);
    write_accounts(app_dir, &names)
}

pub fn clear_saved_accounts(app_dir: &Path) -> std::io::Result<()> {
    write_accounts(app_dir, &[])
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

/// bind loopback ONLY, never 0.0.0.0: enforced here in code, not config.
/// walks up to 100 ports past the requested one when busy so multiple
/// profile instances never collide; returns the listener plus the
/// effective port for the caller to persist.
pub async fn bind_listener(requested: u16) -> Result<(tokio::net::TcpListener, u16)> {
    let last = requested.saturating_add(100);
    let mut port = requested;
    loop {
        match tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await {
            Ok(listener) => {
                if port != requested {
                    log::info!("local api landed on walked port {port} ({requested} was busy)");
                }
                return Ok((listener, port));
            }
            Err(e) => {
                if port >= last {
                    return Err(e).with_context(|| {
                        format!("no free api port in range {requested}-{last}")
                    });
                }
                port += 1;
            }
        }
    }
}

/// serve the api on an already-bound listener (loopback only by construction)
pub async fn serve(listener: tokio::net::TcpListener, state: AppState) -> Result<()> {
    let app = Router::new()
        .route("/api/blast", post(blast))
        .route("/api/status", get(status))
        .route("/api/accounts", get(accounts))
        .route("/api/stop", post(stop))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state);

    let addr = listener.local_addr()?;
    log::info!("local api listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_validation_rejects_missing_token() {
        assert!(!token_is_valid(&HeaderMap::new(), "test-token"));
    }

    #[test]
    fn token_validation_accepts_matching_token() {
        let mut headers = HeaderMap::new();
        headers.insert("x-blastwa-token", "test-token".parse().unwrap());
        assert!(token_is_valid(&headers, "test-token"));
    }

    #[tokio::test]
    async fn bind_walks_up_when_port_taken() {
        // occupy a port, then ask for it: walk must land on +1
        let (listener, _) = bind_listener(0).await.expect("ephemeral bind");
        let taken = listener.local_addr().unwrap().port();
        let (_next_listener, effective) = bind_listener(taken).await.expect("walk bind");
        assert_eq!(effective, taken.wrapping_add(1));
    }

    #[tokio::test]
    async fn bind_returns_requested_when_free() {
        // grab a free port first and release it: low collision odds
        let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let (_listener, effective) = bind_listener(port).await.unwrap();
        assert_eq!(effective, port);
    }

    #[test]
    fn legacy_account_array_is_still_readable() {
        let dir = std::env::temp_dir().join(format!("blastwa_legacy_accounts_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(accounts_file(&dir), r#"["legacy"]"#).unwrap();
        assert_eq!(load_saved_accounts(&dir), vec!["legacy"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_account_file_is_backed_up() {
        let dir = std::env::temp_dir().join(format!("blastwa_corrupt_accounts_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(accounts_file(&dir), "{broken").unwrap();
        assert!(load_saved_accounts(&dir).is_empty());
        assert!(std::fs::read_dir(&dir).unwrap().any(|entry| {
            entry.ok().and_then(|e| e.file_name().into_string().ok()).is_some_and(|name| name.contains("corrupt-"))
        }));
        let _ = std::fs::remove_dir_all(dir);
    }
}
