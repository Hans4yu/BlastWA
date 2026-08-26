// BlastWA GUI entrypoint — Tauri v2.
// all frontend commands land here; pipeline + api server run in background.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use blastwa_core::api::server::{self, AppState};
use blastwa_core::autoreply::rules::{self, Rule};
use blastwa_core::campaign::checker::{check_numbers, CheckOutcome};
use blastwa_core::campaign::contact_list::ContactList;
use blastwa_core::campaign::group_grabber;
use blastwa_core::campaign::human_behavior::{HumanBehaviorConfig, Preset};
use blastwa_core::campaign::import as csv_import;
use blastwa_core::campaign::log_exporter::{
    append_campaign_record, export_csv, export_xlsx, finalize_last_campaign_record,
    interrupt_stale_running_records, load_campaign_records, CampaignRecord, LogEntry,
};
use blastwa_core::campaign::pipeline::Pipeline;
use blastwa_core::campaign::sender::{
    run_campaign, split_message_variants, CampaignConfig, ProgressEvent,
};
use blastwa_core::config::settings::{AppConfig, DataPaths};
use blastwa_core::message::spintax;
use blastwa_core::message::template_library::{MessageTemplate, TemplateLibrary};
use blastwa_core::updater::wpp_updater;

use tauri::{Emitter, Manager, State};

struct AppCtx {
    cfg: Mutex<AppConfig>,
    paths: DataPaths,
    state: AppState,
    pipeline: Pipeline,
    contacts: Mutex<ContactList>,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    templates: TemplateLibrary,
    /// short-lived wa-auth probe cache: name -> (probed_at, authenticated, number)
    auth_cache: Arc<Mutex<HashMap<String, (std::time::Instant, bool, Option<String>)>>>,
    /// accounts for which a one-shot wpp bootstrap was already kicked off
    wpp_bootstrapped: Arc<Mutex<HashMap<String, bool>>>,
}

// ---------- accounts ----------

fn validate_account_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Account name is required".into());
    }
    if name.len() > 64 {
        return Err("Account name is too long (max 64 characters)".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(
            "Account name may only contain letters, numbers, underscore and dash".into(),
        );
    }
    Ok(())
}

/// port of the live chrome session for this account, if any
async fn live_session_port(name: &str) -> Option<u16> {
    let reg = server::sessions_registry();
    let list = reg.lock().await;
    list.iter()
        .find(|(n, _): &&(String, u16)| n == name)
        .map(|(_, p)| *p)
}

/// cheap liveness probe: is the cdp port still accepting tcp connections?
async fn session_alive(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok()
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
    let handle = tauri::async_runtime::spawn(async move {
        let sm = blastwa_core::browser::cdp_client::SessionManager::new(accounts_dir, chrome_path);
        let port = blastwa_core::browser::cdp_client::find_free_port(9222).await;
        // ok(None) = spawn attached to an already-running chrome instance, so
        // the port we passed never opened. discovery below finds the real one.
        sm.launch(&owned, port).await.map(|_| port).ok()
    });
    let spawned_port: Option<u16> = handle.await.map_err(|e| e.to_string())?;
    // ports already owned by other accounts must not be handed to this one:
    // discovery prefers "any endpoint hosting whatsapp", which would bind a
    // second account to the first account's live session.
    let taken: Vec<u16> = {
        let reg = server::sessions_registry();
        let list = reg.lock().await;
        list.iter()
            .filter(|(n, _): &&(String, u16)| n != name)
            .map(|(_, p)| *p)
            .collect()
    };
    blastwa_core::browser::cdp_client::discover_wa_port_excluding(spawned_port, &taken)
        .await
        .ok_or_else(|| "chrome cdp endpoint not found after launch".to_string())
}

async fn register_live_session(name: &str, port: u16) {
    let reg = server::sessions_registry();
    let mut list = reg.lock().await;
    if !list.iter().any(|(n, _): &(String, u16)| n == name) {
        list.push((name.to_string(), port));
    }
}

#[tauri::command]
async fn list_accounts(ctx: State<'_, AppCtx>) -> Result<Vec<serde_json::Value>, String> {
    // identities come from disk; live state is probed per account:
    //   browser_running  = chrome/cdp session alive (page handle + evaluate ok)
    //   wa_authenticated = whatsapp web auth confirmed via DOM (JsInjector)
    //   connected        = browser_running && wa_authenticated
    //   number           = whatsapp identity, only when authenticated
    let app_dir = AppConfig::app_dir();
    let saved = server::load_saved_accounts(&app_dir);
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
        out.push(serde_json::json!({
            "name": name,
            "port": if browser_running { port } else { None },
            "browser_running": browser_running,
            "wa_authenticated": wa_auth,
            "connected": connected,
            "number": number,
        }));
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
            let mut number = if auth {
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

#[tauri::command]
async fn add_account(name: String, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let name = name.trim().to_string();
    validate_account_name(&name)?;

    // identity is configuration: persist it first, independent of whether
    // the chrome session manages to start
    let app_dir = AppConfig::app_dir();
    server::save_account_name(&app_dir, &name)
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
    }

    Ok(serde_json::json!({
        "ok": true,
        "name": name,
        "port": port,
        "connected": port.is_some(),
        "warning": warning,
    }))
}

#[tauri::command]
async fn remove_account(name: String, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    // drop live session entry
    let reg = server::sessions_registry();
    {
        let mut list = reg.lock().await;
        list.retain(|(n, _): &(String, u16)| n != &name);
    }
    // drop saved identity
    let app_dir = AppConfig::app_dir();
    server::remove_saved_account(&app_dir, &name)
        .map_err(|e| format!("removing account: {e}"))?;
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
async fn open_browser(name: String, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    validate_account_name(&name)?;
    let port = launch_session(&ctx, &name).await?;
    register_live_session(&name, port).await;
    // attach the wa tab so the status probe can observe auth state
    ctx.pipeline
        .attach(&name, port)
        .await
        .map_err(|e| format!("browser launched but cdp attach failed: {e:#}"))?;
    Ok(serde_json::json!({ "ok": true, "port": port }))
}

// ---------- campaign ----------

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn start_campaign(
    message: String,
    account: String,
    delay_min_s: Option<f64>,
    delay_max_s: Option<f64>,
    human_preset: Option<String>,
    attachment_path: Option<String>,
    caption: Option<String>,
    schedule_at: Option<String>,
    is_blind_mode: Option<bool>,
    accounts: Option<Vec<String>>,
    list_message: Option<serde_json::Value>,
    catalog_product_id: Option<String>,
    ctx: State<'_, AppCtx>,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    if ctx.state.running.load(Ordering::Relaxed) {
        return Err("campaign already running".into());
    }

    let contacts = ctx.contacts.lock().unwrap().clone();
    if contacts.is_empty() {
        return Err("contact list is empty - import numbers first".into());
    }

    let cfg = ctx.cfg.lock().unwrap().clone();
    let preset = match human_preset.as_deref() {
        Some("off") => Preset::Off,
        Some("cautious") => Preset::Cautious,
        Some("custom") => Preset::Custom,
        _ => Preset::Natural,
    };
    let scheduled = match schedule_at.as_deref().filter(|s| !s.is_empty()) {
        Some(raw) => {
            // datetime-local submits "YYYY-MM-DDTHH:MM" in local wall time
            let naive = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
                .map_err(|_| format!("invalid schedule time: {raw}"))?;
            let local = naive
                .and_local_timezone(chrono::Local)
                .single()
                .ok_or_else(|| "invalid schedule time (ambiguous)".to_string())?;
            if local <= chrono::Local::now() {
                return Err("schedule time must be in the future".into());
            }
            Some(local)
        }
        None => None,
    };
    let human = HumanBehaviorConfig {
        preset,
        delay_min_s: delay_min_s.unwrap_or(cfg.default_delay_min as f64),
        delay_max_s: delay_max_s.unwrap_or(cfg.default_delay_max as f64),
        ..HumanBehaviorConfig::default()
    };
    let mut camp_cfg = CampaignConfig {
        account_name: account.clone(),
        delay_min_s: human.delay_min_s,
        delay_max_s: human.delay_max_s,
        human,
        is_blind_mode: is_blind_mode.unwrap_or(false),
        schedule_at: scheduled,
        ..Default::default()
    };
    // U15: interactive list composition
    if let Some(spec) = list_message.filter(|v| !v.is_null()) {
        let parsed: blastwa_core::campaign::sender::ListMessageSpec =
            serde_json::from_value(spec).map_err(|e| format!("invalid list message config: {e}"))?;
        camp_cfg.list_message = Some(parsed);
    }
    // U16: catalog product card
    camp_cfg.catalog_product_id = catalog_product_id.filter(|s| !s.is_empty());

    // rotation: the composed body splits into variants on --- separator lines
    let message_variants = split_message_variants(&message);
    let variant_count = message_variants.len();

    // resolve attachment to bytes now (ui passes a file path)
    let attachment: Option<(Vec<u8>, String)> = match attachment_path.filter(|p| !p.is_empty()) {
        Some(p) => {
            let bytes = std::fs::read(&p).map_err(|e| format!("reading attachment: {e}"))?;
            let fname = std::path::Path::new(&p)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            Some((bytes, fname))
        }
        None => None,
    };

    // multi-channel (U17): several accounts fan out one campaign without
    // duplicates. per OKESENDER parity, multi-account always runs blind.
    let mut account_names = accounts.unwrap_or_default();
    if !account.is_empty() && !account_names.iter().any(|a| a == &account) {
        account_names.insert(0, account.clone());
    }
    account_names.retain(|a| !a.is_empty());
    if account_names.is_empty() {
        return Err("account is required".into());
    }
    let multi = account_names.len() > 1;
    if multi {
        if !camp_cfg.is_blind_mode {
            log::info!("multi-channel mode forces blind mode");
        }
        camp_cfg.is_blind_mode = true;
    }

    // round-robin split: no contact receives the message twice
    let mut chunks: Vec<ContactList> =
        (0..account_names.len()).map(|_| ContactList::default()).collect();
    for (i, c) in contacts.contacts.iter().enumerate() {
        let slot = i % chunks.len();
        chunks[slot].contacts.push(c.clone());
    }

    // resolve every account session up front (qr waits happen sequentially)
    let mut injectors = Vec::with_capacity(account_names.len());
    for name in &account_names {
        injectors.push(
            ctx.pipeline
                .get_injector(name)
                .await
                .map_err(|e| e.to_string())?,
        );
    }

    let state = ctx.state.clone();
    state.running.store(true, Ordering::Relaxed);
    state.paused.store(false, Ordering::Relaxed);
    state.sent.store(0, Ordering::Relaxed);
    state.failed.store(0, Ordering::Relaxed);
    state.total.store(contacts.len() as u32, Ordering::Relaxed);

    let token = {
        let mut guard = state.stop_flag.lock().await;
        *guard = tokio_util::sync::CancellationToken::new();
        guard.clone()
    };

    let counters = state.clone();
    let logs = Arc::clone(&ctx.logs);
    let account_label = account_names.join("+");
    let campaign_name = format!(
        "{} {}",
        account_label,
        chrono::Local::now().format("%d-%m %H:%M")
    );
    let queued = contacts.len();

    // campaign history: append the record now, finalize it when the loop
    // returns (U6)
    let started_at = chrono::Local::now();
    let start_record = CampaignRecord {
        started_at,
        account: account_label.clone(),
        message_preview: message.chars().take(80).collect(),
        total: queued as u32,
        sent: 0,
        failed: 0,
        status: "running".into(),
    };
    if let Err(e) = append_campaign_record(&AppConfig::app_dir(), &start_record) {
        log::warn!("failed to write campaign history: {e:#}");
    }
    let token_for_status = token.clone();

    tauri::async_runtime::spawn(async move {
        let app_for_progress = app.clone();
        let acc_sent = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let acc_failed = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let total_accounts = injectors.len();

        for (i, inj) in injectors.into_iter().enumerate() {
            if token_for_status.is_cancelled() {
                break;
            }
            if total_accounts > 1 {
                log::info!(
                    "multi-channel: account {} of {} ({})",
                    i + 1,
                    total_accounts,
                    account_names[i]
                );
            }
            let mut per_account_cfg = camp_cfg.clone();
            per_account_cfg.account_name = account_names[i].clone();
            let base_sent = acc_sent.load(Ordering::Relaxed);
            let base_failed = acc_failed.load(Ordering::Relaxed);
            let closure_counters = counters.clone();
            let logs = logs.clone();
            let name = campaign_name.clone();
            let progress_app = app_for_progress.clone();
            let _ = run_campaign(
                inj,
                &chunks[i],
                &message_variants,
                attachment.as_ref(),
                caption.as_deref().unwrap_or(""),
                &per_account_cfg,
                token_for_status.clone(),
                state.paused.clone(),
                move |p: ProgressEvent| {
                    closure_counters.sent.store(base_sent + p.sent, Ordering::Relaxed);
                    closure_counters.failed.store(base_failed + p.failed, Ordering::Relaxed);
                    logs.lock().unwrap().push(LogEntry {
                        timestamp: chrono::Local::now(),
                        number: p.current_number.clone(),
                        fullname: String::new(),
                        status: p.status.clone(),
                        error_reason: None,
                        campaign_name: name.clone(),
                    });
                    // live progress: the sending page listens for this exact event
                    let _ = progress_app.emit(
                        "campaign_progress",
                        &ProgressEvent {
                            sent: base_sent + p.sent,
                            failed: base_failed + p.failed,
                            pending: p.pending,
                            current_number: p.current_number,
                            status: p.status,
                        },
                    );
                },
            )
            .await;
            acc_sent.store(counters.sent.load(Ordering::Relaxed), Ordering::Relaxed);
            acc_failed.store(counters.failed.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        // finalize the history record with the real counters (U6)
        let finished = CampaignRecord {
            started_at,
            account: account_label,
            message_preview: start_record.message_preview,
            total: state.total.load(Ordering::Relaxed),
            sent: state.sent.load(Ordering::Relaxed),
            failed: state.failed.load(Ordering::Relaxed),
            status: if token_for_status.is_cancelled() {
                "stopped".into()
            } else {
                "completed".into()
            },
        };
        if let Err(e) = finalize_last_campaign_record(&AppConfig::app_dir(), &finished) {
            log::warn!("failed to finalize campaign history: {e:#}");
        }
        state.running.store(false, Ordering::Relaxed);
    });

    let mut payload = serde_json::json!({ "ok": true, "queued": queued, "variants": variant_count });
    if let Some(at) = scheduled {
        payload["scheduled_at"] = serde_json::json!(at.format("%Y-%m-%dT%H:%M").to_string());
    }
    Ok(payload)
}

#[tauri::command]
async fn pause_campaign(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.state.paused.store(true, Ordering::Relaxed);
    Ok(serde_json::json!({ "ok": true, "paused": true }))
}

#[tauri::command]
async fn resume_campaign(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.state.paused.store(false, Ordering::Relaxed);
    Ok(serde_json::json!({ "ok": true, "paused": false }))
}

#[tauri::command]
async fn stop_campaign(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.state.stop_flag.lock().await.cancel();
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
async fn get_status(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "running": ctx.state.running.load(Ordering::Relaxed),
        "paused": ctx.state.paused.load(Ordering::Relaxed),
        "sent": ctx.state.sent.load(Ordering::Relaxed),
        "failed": ctx.state.failed.load(Ordering::Relaxed),
        "total": ctx.state.total.load(Ordering::Relaxed),
    }))
}

// ---------- contacts ----------

#[tauri::command]
fn get_contacts(ctx: State<'_, AppCtx>) -> Result<Vec<serde_json::Value>, String> {
    let list = ctx.contacts.lock().unwrap();
    Ok(list
        .contacts
        .iter()
        .map(|c| {
            serde_json::json!({
                "number": c.number, "fullname": c.fullname,
                "var1": c.var1, "var2": c.var2, "var3": c.var3,
                "var4": c.var4, "var5": c.var5,
            })
        })
        .collect())
}

#[tauri::command]
fn clear_contacts(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.contacts.lock().unwrap().contacts.clear();
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
fn import_contacts(
    path: String,
    remove_dupes: Option<bool>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let p = std::path::Path::new(&path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut list = match ext.as_str() {
        "txt" => ContactList::load_txt(p).map_err(|e| e.to_string())?,
        "csv" | "xlsx" | "xls" => {
            let headers = csv_import::read_table(p).map_err(|e| e.to_string())?.0;
            let mapping = csv_import::ColumnMapping::auto_suggest(&headers);
            csv_import::import_contacts(p, &mapping, true, remove_dupes.unwrap_or(true))
                .map_err(|e| e.to_string())?
                .1
        }
        other => return Err(format!("unsupported format: {other}")),
    };
    if remove_dupes.unwrap_or(true) {
        list.filter_duplicates();
    }
    let count = list.len();
    *ctx.contacts.lock().unwrap() = list;
    Ok(serde_json::json!({ "ok": true, "imported": count }))
}

#[tauri::command]
fn export_groups(
    path: String,
    groups: Vec<serde_json::Value>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let _ = &ctx;
    let mut wtr = csv::Writer::from_path(&path).map_err(|e| e.to_string())?;
    wtr.write_record(["Group Name", "Group ID"])
        .map_err(|e| e.to_string())?;
    let mut count = 0usize;
    for g in &groups {
        let name = g.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let id = g.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        wtr.write_record([name, id]).map_err(|e| e.to_string())?;
        count += 1;
    }
    wtr.flush().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "exported": count }))
}

/// csv of only the checker rows that answered yes; the frontend filters,
/// this command re-filters anyway so the file contract cannot drift
#[tauri::command]
async fn export_valid_numbers(
    path: String,
    outcomes: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut wtr = csv::Writer::from_path(&path).map_err(|e| e.to_string())?;
    wtr.write_record(["Number", "Type"])
        .map_err(|e| e.to_string())?;
    let mut count = 0usize;
    for o in &outcomes {
        let exists = o.get("exists").and_then(|v| v.as_bool()).unwrap_or(false);
        if !exists {
            continue;
        }
        let number = o.get("number").and_then(|v| v.as_str()).unwrap_or("");
        if number.is_empty() {
            continue;
        }
        let kind = o.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        wtr.write_record([number, kind]).map_err(|e| e.to_string())?;
        count += 1;
    }
    wtr.flush().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "exported": count }))
}

#[tauri::command]
async fn export_groups_xlsx(
    account: String,
    path: String,
    groups: Vec<serde_json::Value>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    use rust_xlsxwriter::Workbook;

    let injector = ctx
        .pipeline
        .get_injector_attached(&account)
        .await
        .map_err(|e| e.to_string())?;

    let mut wb = Workbook::new();
    let mut used_sheet_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut exported_groups = 0usize;
    let mut exported_rows = 0usize;

    for g in &groups {
        let name = g.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let id = g.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let parts = injector
            .get_group_participants(id)
            .await
            .map_err(|e| format!("{}: {e}", name))?;

        // excel sheet names: max 31 chars, no : \ / ? * [ ], must be unique
        let base: String = {
            let cleaned: String = name
                .chars()
                .map(|c| match c {
                    ':' | '\\' | '/' | '?' | '*' | '[' | ']' => ' ',
                    c => c,
                })
                .collect();
            let trimmed = cleaned.trim();
            if trimmed.is_empty() {
                "Group".to_string()
            } else {
                trimmed.chars().take(28).collect()
            }
        };
        let mut sheet_name = base.clone();
        let mut n = 2;
        while !used_sheet_names.insert(sheet_name.clone()) {
            sheet_name = format!("{} ({})", base, n);
            n += 1;
        }

        let sheet = wb.add_worksheet();
        sheet
            .set_name(&sheet_name)
            .map_err(|e| e.to_string())?;
        sheet.write(0, 0, "#").map_err(|e| e.to_string())?;
        sheet.write(0, 1, "Number").map_err(|e| e.to_string())?;
        sheet.write(0, 2, "Name").map_err(|e| e.to_string())?;
        for (i, (wa_id, cname)) in parts.iter().enumerate() {
            let number = blastwa_core::campaign::contact_list::normalize_number(
                wa_id.trim_end_matches("@c.us"),
            );
            if number.is_empty() {
                continue;
            }
            let r = (i + 1) as u32;
            sheet.write(r, 0, (i + 1) as u32).map_err(|e| e.to_string())?;
            sheet.write(r, 1, &number).map_err(|e| e.to_string())?;
            sheet.write(r, 2, cname.as_deref().unwrap_or("")).map_err(|e| e.to_string())?;
            exported_rows += 1;
        }
        exported_groups += 1;
    }

    wb.save(&path).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "groups": exported_groups, "rows": exported_rows }))
}

// ---------- groups ----------

#[tauri::command]
async fn list_groups(
    account: String,
    ctx: State<'_, AppCtx>,
) -> Result<Vec<serde_json::Value>, String> {
    let injector = ctx.pipeline.get_injector_attached(&account).await.map_err(|e| e.to_string())?;
    let groups = group_grabber::list_groups(&injector)
        .await
        .map_err(|e| e.to_string())?;
    Ok(groups
        .into_iter()
        .map(|g| serde_json::json!({ "id": g.id, "name": g.name }))
        .collect())
}

#[tauri::command]
async fn grab_participants(
    account: String,
    group_id: String,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let injector = ctx.pipeline.get_injector_attached(&account).await.map_err(|e| e.to_string())?;
    let rows = group_grabber::grab_participants(&injector, &group_id)
        .await
        .map_err(|e| e.to_string())?;
    let count = rows.len();
    ctx.contacts.lock().unwrap().contacts.extend(rows);
    Ok(serde_json::json!({ "ok": true, "grabbed": count }))
}

#[tauri::command]
async fn check_numbers_cmd(
    account: String,
    ctx: State<'_, AppCtx>,
    app: tauri::AppHandle,
) -> Result<Vec<CheckOutcome>, String> {
    let injector = ctx.pipeline.get_injector(&account).await.map_err(|e| e.to_string())?;
    let numbers: Vec<String> = ctx
        .contacts
        .lock()
        .unwrap()
        .contacts
        .iter()
        .map(|c| c.number.clone())
        .collect();
    let outcomes = check_numbers(&injector, &numbers, |checked, tot, outcome| {
        // stream each result so the contacts page can render live
        let _ = app.emit(
            "check_progress",
            serde_json::json!({
                "checked": checked,
                "total": tot,
                "number": outcome.number,
                "exists": outcome.exists,
                "kind": outcome.kind,
            }),
        );
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(outcomes)
}

/// keep only the listed (checker-validated) numbers in the send list (U9)
#[tauri::command]
fn keep_contacts_only(valid_numbers: Vec<String>, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let mut list = ctx.contacts.lock().unwrap();
    list.contacts.retain(|c| valid_numbers.contains(&c.number));
    let kept = list.len();
    Ok(serde_json::json!({ "ok": true, "kept": kept }))
}

/// generate candidate numbers under a prefix range into the send list (U18).
/// output feeds the checker, never straight into a campaign blast.
#[tauri::command]
fn add_generated_contacts(
    prefix: String,
    range_start: u64,
    range_end: u64,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    const MAX_RANGE: u64 = 1000;
    let digits: String = prefix.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 6 {
        return Err("prefix must carry at least 6 digits".into());
    }
    if range_end < range_start {
        return Err("range end is below range start".into());
    }
    if range_end - range_start + 1 > MAX_RANGE {
        return Err(format!("range too large (max {} per batch)", MAX_RANGE));
    }
    let mut list = ctx.contacts.lock().unwrap();
    let mut added = 0usize;
    for n in range_start..=range_end {
        let num = format!("{}{}", digits, n);
        if list.contacts.iter().any(|c| c.number == num) {
            continue;
        }
        list.contacts
            .push(blastwa_core::message::variables::ContactRow::from_fullname(&num, ""));
        added += 1;
    }
    Ok(serde_json::json!({ "ok": true, "added": added }))
}

/// U14: pull contacts saved in a whatsapp account's phonebook into the list
#[tauri::command]
async fn import_wa_contacts(
    account: String,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let injector = ctx
        .pipeline
        .get_injector_attached(&account)
        .await
        .map_err(|e| e.to_string())?;
    let wa_contacts = injector.list_wa_contacts().await.map_err(|e| e.to_string())?;
    let mut list = ctx.contacts.lock().unwrap();
    let mut added = 0usize;
    for (number, name) in wa_contacts {
        if number.is_empty() || list.contacts.iter().any(|c| c.number == number) {
            continue;
        }
        list.contacts
            .push(blastwa_core::message::variables::ContactRow::from_fullname(&number, &name));
        added += 1;
    }
    Ok(serde_json::json!({ "ok": true, "added": added }))
}

/// U16: products in an account's own whatsapp catalog
#[tauri::command]
async fn list_catalog_products(
    account: String,
    ctx: State<'_, AppCtx>,
) -> Result<Vec<serde_json::Value>, String> {
    let injector = ctx
        .pipeline
        .get_injector_attached(&account)
        .await
        .map_err(|e| e.to_string())?;
    let products = injector
        .get_catalog_products()
        .await
        .map_err(|e| e.to_string())?;
    Ok(products
        .into_iter()
        .map(|(id, name, description)| {
            serde_json::json!({ "id": id, "name": name, "description": description })
        })
        .collect())
}

// ---------- autoreply ----------

#[tauri::command]
fn load_rules(ctx: State<'_, AppCtx>) -> Result<Vec<Rule>, String> {
    let path = ctx.paths.data.join("autoreply.json");
    rules::load_rules(&path).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_rules(rules: Vec<Rule>, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let path = ctx.paths.data.join("autoreply.json");
    rules::save_rules(&rules, &path).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true }))
}

// ---------- templates ----------

#[tauri::command]
fn list_templates(ctx: State<'_, AppCtx>) -> Result<Vec<MessageTemplate>, String> {
    ctx.templates.list().map_err(|e| e.to_string())
}

#[tauri::command]
fn search_templates(query: String, ctx: State<'_, AppCtx>) -> Result<Vec<MessageTemplate>, String> {
    ctx.templates.search(&query).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_template(
    name: String,
    tags: Option<Vec<String>>,
    body: String,
    attachment_path: Option<String>,
    ctx: State<'_, AppCtx>,
) -> Result<MessageTemplate, String> {
    ctx.templates
        .create(name, tags.unwrap_or_default(), body, attachment_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_template(id: uuid::Uuid, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.templates.delete(id).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true }))
}

// ---------- logs ----------

#[tauri::command]
fn get_logs(ctx: State<'_, AppCtx>) -> Result<Vec<LogEntry>, String> {
    let logs = ctx.logs.lock().unwrap();
    Ok(logs.iter().rev().take(500).cloned().collect())
}

/// persistent per-campaign history, newest first (U6)
#[tauri::command]
fn list_sent_campaigns() -> Vec<CampaignRecord> {
    load_campaign_records(&AppConfig::app_dir())
}

#[tauri::command]
fn export_log(
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

// ---------- config / spintax / updater ----------

#[tauri::command]
fn get_config(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let cfg = ctx.cfg.lock().unwrap();
    Ok(serde_json::json!({
        "chrome_path": cfg.chrome_path,
        "chrome_version": cfg.chrome_version,
        "default_delay_min": cfg.default_delay_min,
        "default_delay_max": cfg.default_delay_max,
        "human_mode_preset": cfg.human_mode_preset,
        "api_enabled": cfg.api_enabled,
        "api_port": cfg.api_port,
        "active_profile": AppConfig::active_profile(),
    }))
}

#[tauri::command]
fn save_config(
    default_delay_min: Option<u64>,
    default_delay_max: Option<u64>,
    human_mode_preset: Option<String>,
    api_enabled: Option<bool>,
    api_port: Option<u16>,
    ctx: State<'_, AppCtx>,
) -> Result<serde_json::Value, String> {
    let mut cfg = ctx.cfg.lock().unwrap();
    if let Some(v) = default_delay_min { cfg.default_delay_min = v; }
    if let Some(v) = default_delay_max { cfg.default_delay_max = v; }
    if let Some(v) = human_mode_preset { cfg.human_mode_preset = v; }
    if let Some(v) = api_enabled { cfg.api_enabled = v; }
    if let Some(v) = api_port { cfg.api_port = v; }
    cfg.save().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
fn preview_spintax(
    text: String,
    ctx: State<'_, AppCtx>,
) -> Result<Vec<String>, String> {
    // render against the first contacts so [[firstname]] etc. show real
    // sample values; falls back to spintax-only when the list is empty
    let samples = ctx.contacts.lock().unwrap();
    if samples.contacts.is_empty() {
        return Ok(spintax::preview_spins(&text, 3));
    }
    let mut out = Vec::new();
    for c in samples.contacts.iter().take(3) {
        out.push(blastwa_core::message::variables::apply_variables(
            &spintax::spin(&text),
            c,
        ));
    }
    Ok(out)
}

#[tauri::command]
fn get_wpp_version(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "local": wpp_updater::current_version(&AppConfig::app_dir()),
    }))
}

#[tauri::command]
async fn check_wpp_update() -> Result<serde_json::Value, String> {
    let latest = wpp_updater::check_latest().await.map_err(|e| e.to_string())?;
    let current = wpp_updater::current_version(&AppConfig::app_dir());
    Ok(serde_json::json!({
        "latest": latest.tag_name,
        "current": current,
        "up_to_date": current.as_deref() == Some(latest.tag_name.as_str()),
    }))
}

#[tauri::command]
async fn update_wpp() -> Result<serde_json::Value, String> {
    let latest = wpp_updater::check_latest().await.map_err(|e| e.to_string())?;
    let tag = wpp_updater::update(&AppConfig::app_dir(), &latest)
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "ok": true, "version": tag }))
}

// ---------- multi-profile launcher ----------

/// spawn a fully isolated second instance bound to its own data root.
/// the child re-enters main() which resolves --profile before any config
/// load, so every storage path isolates without further wiring.
#[tauri::command]
fn open_profile_window(profile: String) -> Result<(), String> {
    let safe = blastwa_core::config::settings::sanitize_name(&profile);
    if safe.is_empty() {
        return Err("Profile name is required".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate exe: {e}"))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--profile").arg(&safe);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    cmd.spawn()
        .map_err(|e| format!("failed to spawn profile window: {e}"))?;
    log::info!("spawned profile window: {safe}");
    Ok(())
}

/// existing profiles on disk (classic root scan, sorted); empty when none
#[tauri::command]
fn list_profiles() -> Vec<String> {
    let dir = AppConfig::classic_root().join("profiles");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
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
    // drain the legacy channel (gui commands talk to pipeline directly)
    tauri::async_runtime::spawn(async move {
        let mut rx = rx;
        while rx.recv().await.is_some() {}
    });

    let templates = TemplateLibrary::new(&paths.templates);

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
    };

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
            list_accounts, add_account, remove_account, open_browser,
            start_campaign, pause_campaign, resume_campaign, stop_campaign, get_status,
            get_contacts, clear_contacts, import_contacts,
            list_groups, grab_participants, export_groups, export_groups_xlsx, check_numbers_cmd,
            keep_contacts_only, add_generated_contacts, export_valid_numbers,
            import_wa_contacts, list_catalog_products,
            load_rules, save_rules,
            list_templates, search_templates, save_template, delete_template,
            get_logs, export_log,
            list_sent_campaigns,
            get_config, save_config, preview_spintax,
            get_wpp_version, check_wpp_update, update_wpp,
            open_profile_window, list_profiles,
        ])
        .run(tauri::generate_context!())
        .expect("error while running blastwa");
}
