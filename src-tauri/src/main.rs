// BlastWA GUI entrypoint — Tauri v2.
// all frontend commands land here; pipeline + api server run in background.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use blastwa_core::api::server::{self, AppState};
use blastwa_core::campaign::contact_list::ContactList;
use blastwa_core::campaign::log_exporter::{interrupt_stale_running_records, LogEntry};
use blastwa_core::campaign::pipeline::Pipeline;
use blastwa_core::account::{registry, service::{AccountService, AccountStatus}};
use commands::accounts as account_commands;

mod commands;
use blastwa_core::config::settings::{AppConfig, DataPaths};
use blastwa_core::message::template_library::TemplateLibrary;

use tauri::{Manager, State};

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

// ---------- accounts ----------

/// port of the live chrome session for this account, if any
async fn live_session_port(name: &str) -> Option<u16> {
    account_commands::live_session_port(name).await
}

/// cheap liveness probe: is the cdp port still accepting tcp connections?
async fn session_alive(port: u16) -> bool {
    account_commands::session_alive(port).await
}

fn format_launch_failure(launch_error: Option<&str>) -> String {
    account_commands::format_launch_failure(launch_error)
}

/// merge the chrome spawn result with cdp endpoint discovery: a discovered
/// endpoint wins; otherwise the actionable spawn error (or the generic
/// discovery failure) is returned so the launch context is never lost.
fn combine_launch_and_discovery(
    launch_result: Result<u16, String>,
    discovered_port: Option<u16>,
) -> Result<u16, String> {
    match discovered_port {
        Some(port) => Ok(port),
        None => Err(format_launch_failure(launch_result.err().as_deref())),
    }
}


/// reuse a live session when one exists; otherwise spawn an isolated chrome
/// instance with a dedicated per-account user-data-dir. never touches the
/// user's personal chrome profile. no automation-warning suppression flags.
async fn launch_session(ctx: &AppCtx, name: &str) -> Result<u16, String> {
    if let Some(port) = live_session_port(name).await {
        if session_alive(port).await {
            return Ok(port); // reuse, no duplicate spawn
        }
        // stale registry entry, drop it before relaunching
        let reg = server::sessions_registry();
        let mut list = reg.lock().await;
        list.retain(|(n, _): &(String, u16)| n != name);
    }

    let chrome_path = ctx.cfg.lock().unwrap().chrome_path.clone();
    let accounts_dir = ctx.paths.accounts.clone();
    let owned = name.to_string();
    let port = blastwa_core::browser::cdp_client::find_free_port(9222).await;
    register_live_session(name, port).await;
    let handle = tauri::async_runtime::spawn(async move {
        let sm = blastwa_core::browser::cdp_client::SessionManager::new(accounts_dir, chrome_path);
        sm.launch(&owned, port).await.map(|_| port)
    });
    let launch_result = handle.await.map_err(|e| e.to_string())?;
    match launch_result {
        Ok(port) => Ok(port),
        Err(e) => {
            {
                let reg = server::sessions_registry();
                let mut list = reg.lock().await;
                list.retain(|(n, _): &(String, u16)| n != name);
            }
            let taken: Vec<u16> = {
                let reg = server::sessions_registry();
                let list = reg.lock().await;
                list.iter()
                    .filter(|(n, _): &&(String, u16)| n != name)
                    .map(|(_, p)| *p)
                    .collect()
            };
            let discovered =
                blastwa_core::browser::cdp_client::discover_wa_port_excluding(None, &taken).await;
            combine_launch_and_discovery(Err(e.to_string()), discovered)
        }
    }
}

async fn register_live_session(name: &str, port: u16) {
    account_commands::register_live_session(name, port).await
}

async fn list_accounts_impl(ctx: State<'_, AppCtx>) -> Result<Vec<AccountStatus>, String> {
    // identities come from disk; live state is probed per account:
    //   browser_running  = chrome/cdp session alive (page handle + evaluate ok)
    //   wa_authenticated = whatsapp web auth confirmed via DOM (JsInjector)
    //   connected        = browser_running && wa_authenticated
    //   number           = whatsapp identity, only when authenticated
    let saved = ctx.account_service.load_names();
    let reg = server::sessions_registry();
    let live = reg.lock().await;

    let mut names: Vec<String> = saved.clone();
    for (n, _) in live.iter() {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    drop(live);

    let mut out = Vec::new();
    for name in &names {
        let (browser_running, wa_auth, number) = probe_account_state(&ctx, name).await;
        let port = live_session_port(name).await;
        let connected = browser_running && wa_auth;
        out.push(AccountStatus {
            name: name.clone(),
            port: if browser_running { port } else { None },
            browser_running,
            wa_authenticated: wa_auth,
            connected,
            number,
        });
    }
    Ok(out)
}

/// probe one account's live page for whatsapp auth state.
/// results are cached briefly so dashboard + status bar polling do not
/// re-evaluate the same page back to back.
async fn probe_account_state(
    ctx: &AppCtx,
    name: &str,
) -> (bool, bool, Option<String>) {
    const TTL: std::time::Duration = std::time::Duration::from_millis(2000);

    {
        let cache = ctx.auth_cache.lock().unwrap();
        if let Some((at, auth, number)) = cache.get(name) {
            if at.elapsed() < TTL {
                return (true, *auth, number.clone());
            }
        }
    }

    let page = match ctx.pipeline.page_handle(name).await {
        Some(p) => p,
        None => {
            // no handle this run (app restarted, or a failed probe evicted
            // it) — but the registry may still point at a living chrome.
            // attach on demand instead of reporting saved forever.
            let Some(port) = live_session_port(name).await else {
                return (false, false, None);
            };
            if !session_alive(port).await {
                return (false, false, None);
            }
            match ctx.pipeline.attach(name, port).await {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("probe attach for {name} failed: {e:#}");
                    return (false, false, None);
                }
            }
        }
    };

    let injector = blastwa_core::browser::js_injector::JsInjector::new(&page);
    match injector.is_logged_in().await {
        Ok(auth) => {
            let number = if auth {
                injector
                    .my_user_id()
                    .await
                    .ok()
                    .filter(|s| !s.is_empty())
            } else {
                None
            };
            // authenticated but no identity yet: modern wa web keeps the wid
            // out of localStorage, so kick off a one-shot wpp bootstrap and
            // let the next poll pick the number up via WPP.conn.getMyUserId().
            // routed through the pipeline gate so it can never race with a
            // groups/autoreply injection of the same page.
            if auth && number.is_none() {
                let mut boot = ctx.wpp_bootstrapped.lock().unwrap();
                if !boot.get(name).copied().unwrap_or(false) {
                    boot.insert(name.to_string(), true);
                    drop(boot);
                    let pipeline = ctx.pipeline.clone();
                    let account = name.to_string();
                    let page = page.clone();
                    let boot_flag = ctx.wpp_bootstrapped.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = pipeline.ensure_wpp_for(&account, page).await {
                            log::warn!("wpp bootstrap for number identity failed: {e:#}");
                            // allow a retry on the next poll
                            boot_flag.lock().unwrap().remove(&account);
                        }
                    });
                }
            }
            let mut cache = ctx.auth_cache.lock().unwrap();
            cache.insert(
                name.to_string(),
                (std::time::Instant::now(), auth, number.clone()),
            );
            (true, auth, number)
        }
        Err(_) => {
            // evaluate failed: the cached handle's cdp socket died. evict and
            // try one fresh attach before concluding the browser is gone.
            ctx.auth_cache.lock().unwrap().remove(name);
            ctx.pipeline.evict_page(name).await;
            if let Some(port) = live_session_port(name).await {
                if let Ok(fresh) = ctx.pipeline.attach(name, port).await {
                    let inj = blastwa_core::browser::js_injector::JsInjector::new(&fresh);
                    if let Ok(auth) = inj.is_logged_in().await {
                        let number = if auth {
                            inj.my_user_id().await.ok().filter(|s| !s.is_empty())
                        } else {
                            None
                        };
                        ctx.auth_cache.lock().unwrap().insert(
                            name.to_string(),
                            (std::time::Instant::now(), auth, number.clone()),
                        );
                        if auth && number.is_none() {
                            let pipeline = ctx.pipeline.clone();
                            let account = name.to_string();
                            let page = fresh.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) =
                                    pipeline.ensure_wpp_for(&account, page).await
                                {
                                    log::warn!("wpp bootstrap after reconnect failed: {e:#}");
                                }
                            });
                        }
                        return (true, auth, number);
                    }
                }
            }
            (false, false, None)
        }
    }
}

async fn add_account_impl(name: String, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let _mutation = ctx.account_service.lock().await;
    let name = name.trim().to_string();
    account_commands::validate_name(&name)?;

    if ctx.account_service.load_names().iter().any(|n| n == &name) {
        return Err(format!("Account \"{name}\" already exists"));
    }

    ctx.account_service.save_name(&name)
        .map_err(|e| format!("saving account: {e}"))?;

    // session launch is best effort; failure is reported as a warning,
    // the saved account can be opened later via Open Browser
    let mut warning = None;
    let mut port = None;
    match live_session_port(&name).await {
        Some(p) if session_alive(p).await => port = Some(p),
        _ => match launch_session(&ctx, &name).await {
            Ok(p) => port = Some(p),
            Err(e) => {
                log::warn!("add_account {name}: session launch failed: {e}");
                warning = Some(format!(
                    "Account saved, but its Chrome session could not start: {e}. Use Open Browser to retry."
                ));
            }
        },
    }

    if let Some(p) = port {
        register_live_session(&name, p).await;
        // the user may have removed this account while its session was still
        // launching — a finished launch must not resurrect a removed identity
        if !ctx.account_service.load_names().iter().any(|n| n == &name) {
            let reg = server::sessions_registry();
            let mut list = reg.lock().await;
            list.retain(|(n, _): &(String, u16)| n != &name);
        }
    }

    Ok(serde_json::json!({
        "ok": true,
        "name": name,
        "port": port,
        "connected": port.is_some(),
        "warning": warning,
    }))
}

/// rename an account: identity in accounts.json plus its chrome profile dir.
/// refuses while the browser is running — windows locks the user-data-dir
/// of a live chrome, so the rename would half-fail and orphan the session.
async fn rename_account_impl(
    old_name: String,
    new_name: String,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let _mutation = ctx.account_service.lock().await;
    let old_name = old_name.trim().to_string();
    let new_name = new_name.trim().to_string();
    account_commands::validate_name(&new_name)?;
    if new_name == old_name {
        return Ok(serde_json::json!({ "ok": true, "name": new_name }));
    }

    let saved = ctx.account_service.load_names();
    if !saved.iter().any(|n| n == &old_name) {
        return Err(format!("account {old_name} does not exist"));
    }
    if saved.iter().any(|n| n == &new_name) {
        return Err(format!("account {new_name} already exists"));
    }
    if let Some(p) = live_session_port(&old_name).await {
        if session_alive(p).await {
            return Err(
                "close this account's browser before renaming (Open Browser keeps a lock on the profile)".into(),
            );
        }
    }

    // move the chrome profile dir so the whatsapp login survives the rename
    let old_dir = ctx.paths.accounts.join(&old_name);
    let new_dir = ctx.paths.accounts.join(&new_name);
    if old_dir.exists() {
        let mut renamed = false;
        let mut last_err = None;
        for attempt in 0..5 {
            match std::fs::rename(&old_dir, &new_dir) {
                Ok(_) => {
                    renamed = true;
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(200 * (attempt + 1))).await;
                }
            }
        }
        if !renamed {
            if let Some(e) = last_err {
                return Err(format!("moving profile dir: {e} (profile directory locked by Windows)"));
            }
        }
    }

    ctx.account_service.remove_name(&old_name)
        .map_err(|e| format!("updating accounts: {e}"))?;
    ctx.account_service.save_name(&new_name)
        .map_err(|e| format!("updating accounts: {e}"))?;
    ctx.auth_cache.lock().unwrap().remove(&old_name);
    ctx.wpp_bootstrapped.lock().unwrap().remove(&old_name);
    Ok(serde_json::json!({ "ok": true, "name": new_name }))
}

async fn remove_account_impl(
    name: String,
    delete_profile: Option<bool>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let _mutation = ctx.account_service.lock().await;
    let name = name.trim().to_string();
    account_commands::validate_name(&name)?;

    if delete_profile.unwrap_or(true) {
        let profile_dir = ctx.account_service.account_dir(&name);
        if profile_dir.exists() {
            std::fs::remove_dir_all(&profile_dir).map_err(|e| {
                format!(
                    "removing account profile failed: {e}; close the account browser and retry"
                )
            })?;
        }
    }
    remove_desktop_profile_shortcut(&name);
    // drop live session entry
    let reg = server::sessions_registry();
    {
        let mut list = reg.lock().await;
        list.retain(|(n, _): &(String, u16)| n != &name);
    }
    // drop saved identity
    ctx.account_service.remove_name(&name)
        .map_err(|e| format!("removing account: {e}"))?;
    ctx.auth_cache.lock().unwrap().remove(&name);
    ctx.wpp_bootstrapped.lock().unwrap().remove(&name);
    ctx.pipeline.evict_page(&name).await;
    Ok(serde_json::json!({ "ok": true }))
}

async fn remove_all_accounts_impl(delete_profiles: Option<bool>, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let _mutation = ctx.account_service.lock().await;
    let names = ctx.account_service.load_names();
    if delete_profiles.unwrap_or(true) {
        for name in &names {
            let profile_dir = ctx.account_service.account_dir(name);
            if profile_dir.exists() {
                std::fs::remove_dir_all(&profile_dir).map_err(|e| {
                    format!("removing account profile '{name}' failed: {e}; close its browser and retry")
                })?;
            }
        }
    }
    {
        let reg = server::sessions_registry();
        let mut list = reg.lock().await;
        list.retain(|(name, _): &(String, u16)| !names.iter().any(|saved| saved == name));
    }
    for name in &names {
        remove_desktop_profile_shortcut(name);
        ctx.auth_cache.lock().unwrap().remove(name);
        ctx.wpp_bootstrapped.lock().unwrap().remove(name);
        ctx.pipeline.evict_page(name).await;
    }
    ctx.account_service.clear_names()
        .map_err(|e| format!("clearing accounts: {e}"))?;
    Ok(serde_json::json!({ "ok": true, "removed": names.len() }))
}

async fn open_browser_impl(name: String, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    account_commands::validate_name(&name)?;
    let port = launch_session(&ctx, &name).await?;
    register_live_session(&name, port).await;
    // attach the wa tab so the status probe can observe auth state
    ctx.pipeline
        .attach(&name, port)
        .await
        .map_err(|e| format!("browser launched but cdp attach failed: {e:#}"))?;
    Ok(serde_json::json!({ "ok": true, "port": port }))
}

#[cfg(windows)]
fn remove_desktop_profile_shortcut(profile_name: &str) {
    if let Some(desktop_dir) = dirs::desktop_dir() {
        let _ = std::fs::remove_file(desktop_dir.join(format!("BlastWA - {profile_name}.lnk")));
    }
}

#[cfg(not(windows))]
fn remove_desktop_profile_shortcut(_profile_name: &str) {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_result_combiner_keeps_spawn_context_on_discovery_failure() {
        // Given: Chrome launch failed before endpoint discovery completed.
        let launch_error = "spawning chrome: The system cannot find the file specified.";

        // When: launch and discovery results are combined.
        let result = combine_launch_and_discovery(Err(launch_error.into()), None);

        // Then: the actionable launch detail is retained alongside discovery context.
        assert!(result.is_err());
        let message = result.err().unwrap_or_default();
        assert!(message.contains(launch_error));
        assert!(message.contains("chrome cdp endpoint not found after launch"));
    }

    #[test]
    fn launch_result_combiner_keeps_discovered_port() {
        // Given: launch succeeded and discovery found the WhatsApp endpoint.
        // When: launch and discovery results are combined.
        let result = combine_launch_and_discovery(Ok(9222), Some(9333));

        // Then: the discovered endpoint remains the result.
        assert_eq!(result, Ok(9333));
    }

    #[test]
    fn launch_result_combiner_preserves_discovery_only_context() {
        // Given: launch did not provide an error and endpoint discovery failed.
        // When: launch and discovery results are combined.
        let result = combine_launch_and_discovery(Ok(9222), None);

        // Then: the existing discovery message remains unchanged.
        assert_eq!(result, Err("chrome cdp endpoint not found after launch".into()));
    }
}
