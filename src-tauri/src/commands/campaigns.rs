use std::sync::atomic::Ordering;
use std::sync::Arc;

use blastwa_core::campaign::contact_list::ContactList;
use blastwa_core::campaign::human_behavior::{HumanBehaviorConfig, Preset};
use blastwa_core::campaign::log_exporter::{
    append_campaign_record, finalize_last_campaign_record, CampaignRecord, LogEntry,
};
use blastwa_core::campaign::sender::{
    run_campaign, split_message_variants, CampaignConfig, ProgressEvent,
};
use blastwa_core::config::settings::AppConfig;

use tauri::{Emitter, State};

use super::super::AppCtx;

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_campaign(
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

    // pre-flight: every account must already be running AND logged in.
    // this used to go through get_injector, which LAUNCHES chrome and sits
    // in a 3-minute qr wait per account — the start button hung with no
    // feedback. a start attaches to live sessions only and fails fast.
    let mut injectors = Vec::with_capacity(account_names.len());
    for name in &account_names {
        let injector = ctx.pipeline.get_injector_attached(name).await.map_err(|e| {
            format!("account \"{name}\" is not running — open it on the Dashboard first ({e:#})")
        })?;
        if !injector.is_logged_in().await.unwrap_or(false) {
            return Err(format!(
                "account \"{name}\" is not logged in to WhatsApp — open it from the Dashboard, scan the QR, then start again"
            ));
        }
        injectors.push(injector);
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
        // account-level failures (dead session, cdp collapse) used to be
        // swallowed by `let _ = run_campaign` — the history then said
        // "completed" while nothing had been sent
        let mut account_errors: Vec<String> = Vec::new();

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
            let err_logs = logs.clone();
            let name = campaign_name.clone();
            let progress_app = app_for_progress.clone();
            let result = run_campaign(
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
            if let Err(e) = &result {
                let msg = format!("{e:#}");
                log::error!("campaign account {} failed: {msg}", account_names[i]);
                account_errors.push(format!("{}: {msg}", account_names[i]));
                // surface it on the Log page with the reason attached
                err_logs.lock().unwrap().push(LogEntry {
                    timestamp: chrono::Local::now(),
                    number: account_names[i].clone(),
                    fullname: String::new(),
                    status: "failed".into(),
                    error_reason: Some(msg),
                    campaign_name: campaign_name.clone(),
                });
            }
            acc_sent.store(counters.sent.load(Ordering::Relaxed), Ordering::Relaxed);
            acc_failed.store(counters.failed.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        // finalize the history record with the real counters (U6) and an
        // honest status: a run where every account died is "failed", not
        // "completed"
        let finished = CampaignRecord {
            started_at,
            account: account_label,
            message_preview: start_record.message_preview,
            total: state.total.load(Ordering::Relaxed),
            sent: state.sent.load(Ordering::Relaxed),
            failed: state.failed.load(Ordering::Relaxed),
            status: final_status(
                token_for_status.is_cancelled(),
                account_errors.len(),
                total_accounts,
            )
            .into(),
        };
        if let Err(e) = finalize_last_campaign_record(&AppConfig::app_dir(), &finished) {
            log::warn!("failed to finalize campaign history: {e:#}");
        }
        if !account_errors.is_empty() {
            log::error!(
                "campaign finished with {} failed account(s): {}",
                account_errors.len(),
                account_errors.join("; ")
            );
        }
        state.running.store(false, Ordering::Relaxed);
    });

    let mut payload = serde_json::json!({ "ok": true, "queued": queued, "variants": variant_count });
    if let Some(at) = scheduled {
        payload["scheduled_at"] = serde_json::json!(at.format("%Y-%m-%dT%H:%M").to_string());
    }
    Ok(payload)
}

/// final history status: user cancellation wins, then full/partial account
/// failure, then success
fn final_status(cancelled: bool, failed_accounts: usize, total_accounts: usize) -> &'static str {
    if cancelled {
        "stopped"
    } else if total_accounts > 0 && failed_accounts >= total_accounts {
        "failed"
    } else if failed_accounts > 0 {
        "partial"
    } else {
        "completed"
    }
}

#[cfg(test)]
mod tests {
    use super::final_status;

    #[test]
    fn status_classification() {
        assert_eq!(final_status(true, 0, 2), "stopped");
        assert_eq!(final_status(true, 2, 2), "stopped", "cancel outranks failure");
        assert_eq!(final_status(false, 2, 2), "failed");
        assert_eq!(final_status(false, 1, 2), "partial");
        assert_eq!(final_status(false, 0, 2), "completed");
        assert_eq!(final_status(false, 0, 1), "completed");
    }
}

#[tauri::command]
pub(crate) async fn pause_campaign(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.state.paused.store(true, Ordering::Relaxed);
    Ok(serde_json::json!({ "ok": true, "paused": true }))
}

#[tauri::command]
pub(crate) async fn resume_campaign(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.state.paused.store(false, Ordering::Relaxed);
    Ok(serde_json::json!({ "ok": true, "paused": false }))
}

#[tauri::command]
pub(crate) async fn stop_campaign(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    ctx.state.stop_flag.lock().await.cancel();
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
pub(crate) async fn get_status(ctx: State<'_, AppCtx>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "running": ctx.state.running.load(Ordering::Relaxed),
        "paused": ctx.state.paused.load(Ordering::Relaxed),
        "sent": ctx.state.sent.load(Ordering::Relaxed),
        "failed": ctx.state.failed.load(Ordering::Relaxed),
        "total": ctx.state.total.load(Ordering::Relaxed),
    }))
}
