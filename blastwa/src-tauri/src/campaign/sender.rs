// campaign blast loop. async, cancellable, human-behavior aware.
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::browser::js_injector::JsInjector;
use crate::campaign::contact_list::ContactList;
use crate::campaign::human_behavior::{
    HumanBehaviorConfig, HumanBehaviorEngine,
};
use crate::message::spintax::spin;
use crate::message::variables::apply_variables;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignConfig {
    pub account_name: String,
    pub delay_min_s: f64,
    pub delay_max_s: f64,
    #[serde(default)]
    pub sleep_after: u32,
    #[serde(default)]
    pub sleep_for_s: u64,
    #[serde(default)]
    pub is_safe_mode: bool,
    /// blind mode: never pre-verify chat existence, accept silent failures
    #[serde(default)]
    pub is_blind_mode: bool,
    #[serde(default)]
    pub schedule_at: Option<chrono::DateTime<chrono::Local>>,
    #[serde(default)]
    pub human: HumanBehaviorConfig,
}

impl Default for CampaignConfig {
    fn default() -> Self {
        Self {
            account_name: "Default".into(),
            delay_min_s: 3.0,
            delay_max_s: 9.0,
            sleep_after: 0,
            sleep_for_s: 60,
            is_safe_mode: false,
            is_blind_mode: false,
            schedule_at: None,
            human: HumanBehaviorConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub sent: u32,
    pub failed: u32,
    pub pending: usize,
    pub current_number: String,
    pub status: String,
}

pub struct CampaignStats {
    pub sent: AtomicU32,
    pub failed: AtomicU32,
    pub cancelled: AtomicBool,
}

pub async fn run_campaign(
    injector: JsInjector,
    contacts: &ContactList,
    message_templates: &[String],
    attachment: Option<&(Vec<u8>, String)>, // (bytes, filename)
    caption_template: &str,
    cfg: &CampaignConfig,
    token: CancellationToken,
    paused: Arc<std::sync::atomic::AtomicBool>,
    on_progress: impl Fn(ProgressEvent),
) -> Result<CampaignStats> {
    // optional scheduled start
    if let Some(at) = cfg.schedule_at {
        let now = chrono::Local::now();
        if at > now {
            let wait = (at - now).to_std().unwrap_or_default();
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = token.cancelled() => {
                    return Ok(finished_stats(&token));
                }
            }
        }
    }

    let mut order: Vec<usize> = (0..contacts.len()).collect();

    let stats = Arc::new(CampaignStats {
        sent: AtomicU32::new(0),
        failed: AtomicU32::new(0),
        cancelled: AtomicBool::new(false),
    });

    let mut engine = HumanBehaviorEngine::new(
        &cfg.account_name,
        HumanBehaviorConfig {
            delay_min_s: cfg.delay_min_s,
            delay_max_s: cfg.delay_max_s,
            ..cfg.human.clone()
        },
    );
    // blind mode (U12) overrides the safe-mode pre-check entirely
    let effective_safe = cfg.is_safe_mode && !cfg.is_blind_mode;

    let total = order.len();
    for idx in 0..total {
        if token.is_cancelled() {
            break;
        }

        // contacts are messaged strictly in list order (user requirement);
        // human-mode randomization lives in the delays, not the order
        let contact = &contacts.contacts[order[idx]];

        // pause: hold before this contact until resumed or stopped
        while paused.load(std::sync::atomic::Ordering::Relaxed)
            && !token.is_cancelled()
        {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
        if token.is_cancelled() {
            break;
        }

        // per-message pipeline: rotate variants (contact i gets variant
        // i mod N), then spintax then variables
        let template = &message_templates[idx % message_templates.len()];
        let text_body = apply_variables(&spin(template), contact);

        // presence simulation before sending (best-effort, never fatal)
        if engine.config().enable_typing_sim {
            let _ = injector.mark_seen(&contact.wa_id()).await;
            let _ = injector.send_typing_state(&contact.wa_id()).await;
        }

        let decision = engine.next_wait(&text_body);
        tokio::select! {
            _ = tokio::time::sleep(decision.wait) => {}
            _ = token.cancelled() => break,
        }

        if token.is_cancelled() {
            break;
        }

        let result = match attachment {
            Some((bytes, filename)) => {
                let mime = guess_mime(filename);
                let data_uri = format!(
                    "data:{};base64,{}",
                    mime,
                    use_base64(bytes)
                );
                if mime == "audio/ogg" || filename.ends_with(".ogg") {
                    injector.send_ptt(&contact.wa_id(), &data_uri).await
                } else {
                    let caption = apply_variables(caption_template, contact);
                    injector
                        .send_file(
                            &contact.wa_id(),
                            &data_uri,
                            filename,
                            &caption,
                            effective_safe,
                        )
                        .await
                }
            }
            None => injector
                .send_message(&contact.wa_id(), &text_body, effective_safe)
                .await,
        };

        let success = matches!(&result, Ok(r) if r.ok());
        engine.record_result(success);

        if success {
            stats.sent.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.failed.fetch_add(1, Ordering::Relaxed);
        }

        on_progress(ProgressEvent {
            sent: stats.sent.load(Ordering::Relaxed),
            failed: stats.failed.load(Ordering::Relaxed),
            pending: total - idx - 1,
            current_number: contact.number.clone(),
            status: if success { "sent".into() } else { "failed".into() },
        });

        // periodic long sleep between batches
        if cfg.sleep_after > 0 && (idx + 1) % cfg.sleep_after as usize == 0 && idx + 1 < total {
            log::info!("batch rest for {}s", cfg.sleep_for_s);
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(cfg.sleep_for_s)) => {}
                _ = token.cancelled() => break,
            }
        }
    }

    stats.cancelled.store(token.is_cancelled(), Ordering::Relaxed);
    Ok(CampaignStats {
        sent: AtomicU32::new(stats.sent.load(Ordering::Relaxed)),
        failed: AtomicU32::new(stats.failed.load(Ordering::Relaxed)),
        cancelled: AtomicBool::new(stats.cancelled.load(Ordering::Relaxed)),
    })
}

fn finished_stats(token: &CancellationToken) -> CampaignStats {
    CampaignStats {
        sent: AtomicU32::new(0),
        failed: AtomicU32::new(0),
        cancelled: AtomicBool::new(token.is_cancelled()),
    }
}

/// split a composed body into rotation variants on separator lines made of
/// three or more dashes; single message stays a one-element vec
pub fn split_message_variants(message: &str) -> Vec<String> {
    let normalized = message.replace("\r\n", "\n");
    let mut variants: Vec<String> = normalized
        .split("\n---\n")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();
    if variants.is_empty() {
        variants.push(normalized.trim().to_string());
    }
    variants
}

fn guess_mime(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".mp4") {
        "video/mp4"
    } else if lower.ends_with(".ogg") || lower.ends_with(".opus") {
        "audio/ogg"
    } else if lower.ends_with(".txt") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

fn use_base64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn progress_event_payload_shape() {
        // the sending page reads ev.payload.sent/failed/pending/current_number/status
        // - field names are the IPC contract, renaming them breaks the ui silently
        let v = serde_json::to_value(ProgressEvent {
            sent: 1,
            failed: 2,
            pending: 3,
            current_number: "628123".into(),
            status: "sent".into(),
        })
        .unwrap();
        assert_eq!(v["sent"], 1);
        assert_eq!(v["failed"], 2);
        assert_eq!(v["pending"], 3);
        assert_eq!(v["current_number"], "628123");
        assert_eq!(v["status"], "sent");
    }

    #[test]
    fn variants_split_on_dash_separator() {
        let msg = "halo satu\n---\nhalo dua\n\n---\nhalo tiga";
        let v = split_message_variants(msg);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], "halo satu");
        assert_eq!(v[2], "halo tiga");
    }

    #[test]
    fn single_message_is_one_variant() {
        assert_eq!(split_message_variants("hanya ini").len(), 1);
        assert_eq!(split_message_variants("").len(), 1); // never empty
    }

    #[test]
    fn crlf_separator_still_splits() {
        let v = split_message_variants("a\r\n---\r\nb");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], "a");
        assert_eq!(v[1], "b");
    }
}
