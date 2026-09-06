// BlastWA GUI entrypoint — Tauri v2.
// all frontend commands land here; pipeline + api server run in background.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use blastwa_core::api::server::{self, AppState};
use blastwa_core::campaign::contact_list::ContactList;
use blastwa_core::campaign::log_exporter::{interrupt_stale_running_records, LogEntry};
use blastwa_core::campaign::pipeline::Pipeline;
use blastwa_core::account::{registry, service::AccountService};
use commands::accounts as account_commands;

mod commands;
use blastwa_core::config::settings::{AppConfig, DataPaths};
use blastwa_core::message::template_library::TemplateLibrary;

use tauri::Manager;

struct AppCtx {
    cfg: Mutex<AppConfig>,
    paths: DataPaths,
    state: AppState,
    pipeline: Pipeline,
    contacts: Mutex<ContactList>,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    templates: TemplateLibrary,
    account_service: AccountService,
    /// short-lived wa-auth probe cache: name -> (probed_at, authenticated, number)
    auth_cache: Arc<Mutex<HashMap<String, (std::time::Instant, bool, Option<String>)>>>,
    /// accounts for which a one-shot wpp bootstrap was already kicked off
    wpp_bootstrapped: Arc<Mutex<HashMap<String, bool>>>,
}

fn parse_cli_profile() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if let Some(rest) = a.strip_prefix("--profile=") {
            return Some(rest.to_string());
        }
        if a == "--profile" {
            return args.next();
        }
    }
    None
}

fn main() {
    env_logger::init();

    // profile isolation must be decided before any config load: app_dir(),
    // config_path(), and every derived storage path hang off it
    let cli_profile = parse_cli_profile()
        .or_else(|| std::env::var("BLASTWA_PROFILE").ok().filter(|s| !s.is_empty()));
    if let Some(name) = cli_profile.as_deref() {
        if let Err(e) = AppConfig::init_profile(name) {
            eprintln!("blastwa: {e}");
            std::process::exit(2);
        }
        log::info!("launcher profile active: {}", AppConfig::active_profile().unwrap_or("?"));
        let _ = registry::register(
            &AppConfig::classic_root(),
            registry::ProfileProcess { name: AppConfig::active_profile().unwrap_or(name).to_string(), pid: std::process::id() },
        );
    }

    let cfg = AppConfig::load_or_default();
    let paths = cfg.init_data_dirs().expect("init data dirs");

    // campaigns left "running" by a dead process are now interrupted (U6)
    if let Err(e) = interrupt_stale_running_records(&AppConfig::app_dir()) {
        log::warn!("campaign history sweep failed: {e:#}");
    }

    let (tx, rx) = tokio::sync::mpsc::channel::<server::BlastRequest>(16);
    let state = AppState {
        running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        paused: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        sent: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        failed: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        total: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        blast_requested: Arc::new(tx),
        stop_flag: Arc::new(tokio::sync::Mutex::new(
            tokio_util::sync::CancellationToken::new(),
        )),
        api_token: Arc::new(cfg.api_token.clone()),
    };

    let pipeline = Pipeline::new(state.clone(), cfg.chrome_path.clone(), paths.accounts.clone());

    // headless rest api alongside the gui when enabled.
    // bind walks up on port collision; the effective port is persisted back
    // into this profile's config so the Settings page always tells the truth
    if cfg.api_enabled {
        let desired_port = cfg.api_port;
        let api_state = state.clone();
        let cfg_for_port = cfg.clone();
        tauri::async_runtime::spawn(async move {
            match server::bind_listener(desired_port).await {
                Ok((listener, effective)) => {
                    if effective != desired_port {
                        log::warn!("api port {desired_port} busy, using {effective}");
                        let mut c = cfg_for_port;
                        c.api_port = effective;
                        if let Err(e) = c.save() {
                            log::warn!("failed to persist effective api port: {e}");
                        }
                    }
                    if let Err(e) = server::serve(listener, api_state).await {
                        log::error!("api server died: {e}");
                    }
                }
                Err(e) => log::error!("api server bind failed: {e:#}"),
            }
        });
    }
    // REST API blast channel consumer: dispatch incoming requests to the campaign pipeline
    let pipeline_for_api = pipeline.clone();
    tauri::async_runtime::spawn(async move {
        pipeline_for_api.serve(rx).await;
    });

    let templates = TemplateLibrary::new(&paths.templates);

    let account_service = AccountService::new(AppConfig::app_dir(), paths.accounts.clone());
    let ctx = AppCtx {
        cfg: Mutex::new(cfg),
        paths,
        state,
        pipeline,
        contacts: Mutex::new(ContactList::default()),
        logs: Arc::new(Mutex::new(Vec::new())),
        templates,
        auth_cache: Arc::new(Mutex::new(HashMap::new())),
        wpp_bootstrapped: Arc::new(Mutex::new(HashMap::new())),
        account_service,
    };

    let active_profile = AppConfig::active_profile().map(str::to_string);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // profile instances announce themselves in the title bar; the
            // default instance stays untitled
            if let Some(p) = AppConfig::active_profile() {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_title(&format!("BlastWA - WhatsApp Bulk Sender [Profile: {p}]"));
                }
            }
            Ok(())
        })
        .manage(ctx)
        .invoke_handler(tauri::generate_handler![
            account_commands::list_accounts,
            account_commands::add_account,
            account_commands::remove_account,
            account_commands::remove_all_accounts,
            account_commands::rename_account,
            account_commands::open_browser,
            commands::campaigns::start_campaign,
            commands::campaigns::pause_campaign,
            commands::campaigns::resume_campaign,
            commands::campaigns::stop_campaign,
            commands::campaigns::get_status,
            commands::contacts::get_contacts,
            commands::contacts::clear_contacts,
            commands::contacts::remove_contacts,
            commands::contacts::import_contacts,
            commands::contacts::check_numbers_cmd,
            commands::contacts::keep_contacts_only,
            commands::contacts::add_generated_contacts,
            commands::contacts::export_valid_numbers,
            commands::contacts::export_contacts_csv,
            commands::contacts::import_wa_contacts,
            commands::contacts::list_catalog_products,
            commands::groups::list_groups,
            commands::groups::grab_participants,
            commands::groups::export_groups,
            commands::groups::export_groups_xlsx,
            commands::autoreply::load_rules,
            commands::autoreply::save_rules,
            commands::templates::list_templates,
            commands::templates::search_templates,
            commands::templates::save_template,
            commands::templates::delete_template,
            commands::templates::preview_spintax,
            commands::logs::get_logs,
            commands::logs::export_log,
            commands::logs::list_sent_campaigns,
            commands::config::get_config,
            commands::config::save_config,
            commands::config::get_health_diagnostics,
            commands::updater::get_wpp_version,
            commands::updater::check_wpp_update,
            commands::updater::update_wpp,
            commands::profiles::open_profile_window,
            commands::profiles::list_profiles,
        ])
        .run(tauri::generate_context!())
        .expect("error while running blastwa");

    if let Some(name) = active_profile {
        let _ = registry::unregister(&AppConfig::classic_root(), &name, std::process::id());
    }
}
