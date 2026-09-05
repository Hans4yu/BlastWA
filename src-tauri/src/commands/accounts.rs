//! Account IPC commands, chrome launch/discovery, and live-session probing.

use blastwa_core::account::service::AccountStatus;
use blastwa_core::api::server;
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

/// port of the live chrome session for this account, if any
pub(crate) async fn live_session_port(name: &str) -> Option<u16> {
    let registry = server::sessions_registry();
    let sessions = registry.lock().await;
    sessions.iter().find(|(n, _)| n == name).map(|(_, port)| *port)
}

/// cheap liveness probe: is the cdp port still accepting tcp connections?
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
    validate_name(&name)?;

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
    validate_name(&new_name)?;
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
    validate_name(&name)?;

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
    validate_name(&name)?;
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

#[tauri::command]
pub(crate) async fn list_accounts(ctx: State<'_, AppCtx>) -> Result<Vec<AccountStatus>, AppError> {
    list_accounts_impl(ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn add_account(name: String, ctx: State<'_, AppCtx>) -> Result<Value, AppError> {
    add_account_impl(name, ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn rename_account(
    old_name: String,
    new_name: String,
    ctx: State<'_, AppCtx>,
) -> Result<Value, AppError> {
    rename_account_impl(old_name, new_name, ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn remove_account(
    name: String,
    delete_profile: Option<bool>,
    ctx: State<'_, AppCtx>,
) -> Result<Value, AppError> {
    remove_account_impl(name, delete_profile, ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn remove_all_accounts(
    delete_profiles: Option<bool>,
    ctx: State<'_, AppCtx>,
) -> Result<Value, AppError> {
    remove_all_accounts_impl(delete_profiles, ctx).await.map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn open_browser(name: String, ctx: State<'_, AppCtx>) -> Result<Value, AppError> {
    open_browser_impl(name, ctx).await.map_err(AppError::from)
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
