// BlastWA entrypoint — headless API mode.
// config load, data dirs init, local rest api + campaign pipeline.
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Result;
use blastwa_core::api::server::{self, AppState};
use blastwa_core::campaign::pipeline::Pipeline;
use blastwa_core::config::settings::{AppConfig, DataPaths};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    println!("BlastWA v{} (headless api mode)", env!("CARGO_PKG_VERSION"));

    let cfg = AppConfig::load_or_default();
    let paths: DataPaths = cfg.init_data_dirs()?;

    if cfg.chrome_path.is_empty() {
        log::warn!("chrome path not set — run blastwa-init.exe first");
    }

    // channel: api server -> pipeline
    let (tx, rx) = tokio::sync::mpsc::channel::<blastwa_core::api::server::BlastRequest>(16);

    let state = AppState {
        running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        sent: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        failed: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        blast_requested: Arc::new(tx),
        stop_flag: Arc::new(tokio::sync::Mutex::new(
            tokio_util::sync::CancellationToken::new(),
        )),
    };

    // campaign pipeline owns the chrome sessions
    let pipeline = Pipeline::new(state.clone(), cfg.chrome_path.clone(), paths.accounts.clone());
    tokio::spawn(async move {
        pipeline.serve(rx).await;
    });

    // local rest api
    if cfg.api_enabled {
        let port = cfg.api_port;
        let api_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = server::serve(port, api_state).await {
                log::error!("api server died: {e}");
            }
        });
        println!("api listening on http://127.0.0.1:{} (see /api/status)", cfg.api_port);
    } else {
        println!(
            "api disabled — enable in {} (\"api_enabled\": true)",
            AppConfig::config_path().display()
        );
    }

    println!("ready. profiles: {}", paths.profiles.display());

    // keep the runtime alive; work happens on spawned tasks
    tokio::signal::ctrl_c().await.ok();
    state.running.store(false, Ordering::Relaxed);
    Ok(())
}
