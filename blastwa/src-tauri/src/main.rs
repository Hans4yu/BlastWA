// BlastWA entrypoint.
// headless-first: config load, data dirs init, optional local api server.
// gui (tauri v2) mounts behind the `gui` feature.
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Result;
use blastwa_core::api::server::{self, AppState, BlastRequest};
use blastwa_core::config::settings::{AppConfig, DataPaths};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    println!("BlastWA v{}", env!("CARGO_PKG_VERSION"));

    let cfg = AppConfig::load_or_default();
    let paths: DataPaths = cfg.init_data_dirs()?;

    if cfg.chrome_path.is_empty() {
        log::warn!("chrome path not set — run blastwa-setup.exe first");
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<BlastRequest>(16);

    let state = AppState {
        running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        sent: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        failed: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        blast_requested: Arc::new(tx),
        stop_flag: Arc::new(tokio_util::sync::CancellationToken::new()),
    };

    // local rest api
    if cfg.api_enabled {
        let port = cfg.api_port;
        let api_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = server::serve(port, api_state).await {
                log::error!("api server died: {e}");
            }
        });
    }

    println!(
        "ready. profiles: {} | accounts dir: {}",
        paths.profiles.display(),
        paths.accounts.display()
    );

    // consume blast requests (campaign pipeline lands in the next iteration)
    while let Some(_req) = rx.recv().await {
        state.running.store(true, Ordering::Relaxed);
        log::info!("blast request received — pipeline pending wiring");
        state.running.store(false, Ordering::Relaxed);
    }

    Ok(())
}
