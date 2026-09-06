// number checker: batch-validate which numbers have whatsapp before blasting
use std::time::Duration;

use anyhow::{bail, Result};
use serde::Serialize;

use crate::browser::js_injector::JsInjector;

#[derive(Debug, Clone, Serialize)]
pub struct CheckOutcome {
    pub number: String,
    pub exists: bool,
    pub kind: String,
    /// a per-number failure (page busy, cdp hiccup, api throw). the ui must
    /// render this as its own state — folding it into "not found" made a
    /// broken check look like a clean all-miss result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// live snapshot of a check run, pollable from the ui. progress events are
/// fire-and-forget broadcasts (listeners that navigated away never see them),
/// so the frontend needs an authoritative state to re-sync from instead of
/// trusting the event stream alone.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CheckJobState {
    pub running: bool,
    pub account: String,
    pub checked: usize,
    pub total: usize,
    pub outcomes: Vec<CheckOutcome>,
}

/// check a batch of numbers with polite pacing between requests.
/// on_progress fires per checked number so the ui can stream results.
/// `slow_mode` doubles the pacing while a campaign is sending on the same
/// page so the two cdp consumers don't starve each other.
pub async fn check_numbers(
    injector: &JsInjector,
    numbers: &[String],
    slow_mode: bool,
    on_progress: impl Fn(usize, usize, &CheckOutcome),
) -> Result<Vec<CheckOutcome>> {
    if !injector.is_logged_in().await.unwrap_or(false) {
        bail!("not logged in — scan the QR first");
    }

    let pacing = Duration::from_millis(if slow_mode { 1200 } else { 600 });
    let mut outcomes = Vec::with_capacity(numbers.len());
    for (i, num) in numbers.iter().enumerate() {
        let outcome = check_with_retry(injector, num).await;
        on_progress(i + 1, numbers.len(), &outcome);
        outcomes.push(outcome);
        tokio::time::sleep(pacing).await;
    }
    Ok(outcomes)
}

/// one number: retry once after a pause before giving up and reporting the
/// error as its own outcome
async fn check_with_retry(injector: &JsInjector, num: &str) -> CheckOutcome {
    let mut last_err = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        match injector.check_number(num).await {
            Ok(status) if status.error.is_none() => {
                return CheckOutcome {
                    number: num.to_string(),
                    exists: status.exists(),
                    kind: status.kind().to_string(),
                    error: None,
                };
            }
            Ok(status) => {
                // the page-side script threw; the message says why
                last_err = status.error.unwrap_or_else(|| "unknown page error".into());
            }
            Err(e) => {
                last_err = format!("{e:#}");
            }
        }
    }
    CheckOutcome {
        number: num.to_string(),
        exists: false,
        kind: "Error".into(),
        error: Some(last_err),
    }
}
