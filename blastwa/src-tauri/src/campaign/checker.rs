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
}

/// check a batch of numbers with polite pacing between requests.
/// on_progress fires per checked number so the UI can stream results.
pub async fn check_numbers(
    injector: &JsInjector<'_>,
    numbers: &[String],
    on_progress: impl Fn(usize, usize, &CheckOutcome),
) -> Result<Vec<CheckOutcome>> {
    if !injector.is_logged_in().await.unwrap_or(false) {
        bail!("not logged in — scan the QR first");
    }

    let mut outcomes = Vec::with_capacity(numbers.len());
    for (i, num) in numbers.iter().enumerate() {
        let status = injector.check_number(num).await?;
        let outcome = CheckOutcome {
            number: num.clone(),
            exists: status.exists(),
            kind: status.kind().to_string(),
        };
        on_progress(i + 1, numbers.len(), &outcome);
        outcomes.push(outcome);
        // pacing: keep wa happy
        tokio::time::sleep(Duration::from_millis(600)).await;
    }
    Ok(outcomes)
}
