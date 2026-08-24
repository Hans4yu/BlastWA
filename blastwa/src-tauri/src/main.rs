// BlastWA GUI entrypoint — Tauri v2.
// all frontend commands land here; pipeline + api server run in background.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use blastwa_core::api::server::{self, AppState};
use blastwa_core::autoreply::rules::{self, Rule};
use blastwa_core::campaign::checker::{check_numbers, CheckOutcome};
use blastwa_core::campaign::contact_list::ContactList;
use blastwa_core::campaign::group_grabber;
use blastwa_core::campaign::human_behavior::{HumanBehaviorConfig, Preset};
use blastwa_core::campaign::import as csv_import;
use blastwa_core::campaign::log_exporter::{export_csv, export_xlsx, LogEntry};
use blastwa_core::campaign::pipeline::Pipeline;
use blastwa_core::campaign::sender::{run_campaign, CampaignConfig, ProgressEvent};
use blastwa_core::config::settings::{AppConfig, DataPaths};
use blastwa_core::message::spintax;
use blastwa_core::message::template_library::{MessageTemplate, TemplateLibrary};
use blastwa_core::updater::wpp_updater;

use tauri::State;

struct AppCtx {
    cfg: Mutex<AppConfig>,
    paths: DataPaths,
    state: AppState,
    pipeline: Pipeline,
    contacts: Mutex<ContactList>,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    templates: TemplateLibrary,
}

// ---------- accounts ----------

#[tauri::command]
async fn list_accounts() -> Result<Vec<serde_json::Value>, String> {
    let reg = server::sessions_registry();
    let list = reg.lock().await;
    Ok(list
        .iter()
        .map(|(name, port)| serde_json::json!({ "name": name, "port": port, "connected": true }))
        .collect())
}

async fn launch_session(chrome_path: String, accounts_dir: std::path::PathBuf, name: String) -> Result<u16, String> {
    let handle = tauri::async_runtime::spawn(async move {
        let sm = blastwa_core::browser::cdp_client::SessionManager::new(accounts_dir, chrome_path);
        let port = blastwa_core::browser::cdp_client::find_free_port(9222).await;
        sm.launch(&name, port).await.map(|_| port).map_err(|e| e.to_string())
    });
    handle.await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn add_account(name: String, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let chrome_path = ctx.cfg.lock().unwrap().chrome_path.clone();
    let accounts_dir = ctx.paths.accounts.clone();
    let port = launch_session(chrome_path, accounts_dir, name.clone()).await?;
    let reg = server::sessions_registry();
    let mut list = reg.lock().await;
    if !list.iter().any(|(n, _): &(String, u16)| n == &name) {
        list.push((name.clone(), port));
    }
    Ok(serde_json::json!({ "ok": true, "name": name, "port": port }))
}

#[tauri::command]
async fn remove_account(name: String) -> Result<serde_json::Value, String> {
    let reg = server::sessions_registry();
    let mut list = reg.lock().await;
    list.retain(|(n, _): &(String, u16)| n != &name);
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
async fn open_browser(name: String, ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    let chrome_path = ctx.cfg.lock().unwrap().chrome_path.clone();
    let accounts_dir = ctx.paths.accounts.clone();
    let port = launch_session(chrome_path, accounts_dir, name.clone()).await?;
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
    ctx: State<'_, AppCtx>,
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
    let human = HumanBehaviorConfig {
        preset,
        delay_min_s: delay_min_s.unwrap_or(cfg.default_delay_min as f64),
        delay_max_s: delay_max_s.unwrap_or(cfg.default_delay_max as f64),
        ..HumanBehaviorConfig::default()
    };
    let camp_cfg = CampaignConfig {
        account_name: account.clone(),
        delay_min_s: human.delay_min_s,
        delay_max_s: human.delay_max_s,
        human,
        ..Default::default()
    };

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

    // get or launch the account session (qr wait handled inside)
    let injector = ctx
        .pipeline
        .get_injector(&account)
        .await
        .map_err(|e| e.to_string())?;

    let state = ctx.state.clone();
    state.running.store(true, Ordering::Relaxed);
    state.sent.store(0, Ordering::Relaxed);
    state.failed.store(0, Ordering::Relaxed);

    let token = {
        let mut guard = state.stop_flag.lock().await;
        *guard = tokio_util::sync::CancellationToken::new();
        guard.clone()
    };

    let counters = state.clone();
    let logs = Arc::clone(&ctx.logs);
    let campaign_name = format!("{} {}", account, chrono::Local::now().format("%d-%m %H:%M"));
    let queued = contacts.len();

    tauri::async_runtime::spawn(async move {
        let _ = run_campaign(
            injector,
            &contacts,
            &message,
            attachment.as_ref(),
            caption.as_deref().unwrap_or(""),
            &camp_cfg,
            token,
            move |p: ProgressEvent| {
                counters.sent.store(p.sent, Ordering::Relaxed);
                counters.failed.store(p.failed, Ordering::Relaxed);
                logs.lock().unwrap().push(LogEntry {
                    timestamp: chrono::Local::now(),
                    number: p.current_number.clone(),
                    fullname: String::new(),
                    status: p.status.clone(),
                    error_reason: None,
                    campaign_name: campaign_name.clone(),
                });
            },
        )
        .await;
        state.running.store(false, Ordering::Relaxed);
    });

    Ok(serde_json::json!({ "ok": true, "queued": queued }))
}

#[tauri::command]
async fn pause_campaign() -> Result<serde_json::Value, String> {
    // pause == stop for v0.2; resume re-queues remaining contacts later
    Ok(serde_json::json!({ "ok": true, "note": "use stop" }))
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
        "sent": ctx.state.sent.load(Ordering::Relaxed),
        "failed": ctx.state.failed.load(Ordering::Relaxed),
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

// ---------- groups ----------

#[tauri::command]
async fn list_groups(
    account: String,
    ctx: State<'_, AppCtx>,
) -> Result<Vec<serde_json::Value>, String> {
    let injector = ctx.pipeline.get_injector(&account).await.map_err(|e| e.to_string())?;
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
    let injector = ctx.pipeline.get_injector(&account).await.map_err(|e| e.to_string())?;
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
    check_numbers(&injector, &numbers, |_, _, _| {})
        .await
        .map_err(|e| e.to_string())
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
fn preview_spintax(text: String) -> Result<Vec<String>, String> {
    Ok(spintax::preview_spins(&text, 3))
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

fn main() {
    env_logger::init();

    let cfg = AppConfig::load_or_default();
    let paths = cfg.init_data_dirs().expect("init data dirs");

    let (tx, rx) = tokio::sync::mpsc::channel::<server::BlastRequest>(16);
    let state = AppState {
        running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        sent: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        failed: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        blast_requested: Arc::new(tx),
        stop_flag: Arc::new(tokio::sync::Mutex::new(
            tokio_util::sync::CancellationToken::new(),
        )),
    };

    let pipeline = Pipeline::new(state.clone(), cfg.chrome_path.clone(), paths.accounts.clone());

    // headless rest api alongside the gui when enabled
    if cfg.api_enabled {
        let port = cfg.api_port;
        let api_state = state.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = server::serve(port, api_state).await {
                log::error!("api server died: {e}");
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
    };

    tauri::Builder::default()
        .manage(ctx)
        .invoke_handler(tauri::generate_handler![
            list_accounts, add_account, remove_account, open_browser,
            start_campaign, pause_campaign, stop_campaign, get_status,
            get_contacts, clear_contacts, import_contacts,
            list_groups, grab_participants, check_numbers_cmd,
            load_rules, save_rules,
            list_templates, search_templates, save_template, delete_template,
            get_logs, export_log,
            get_config, save_config, preview_spintax,
            get_wpp_version, check_wpp_update, update_wpp,
        ])
        .run(tauri::generate_context!())
        .expect("error while running blastwa");
}
