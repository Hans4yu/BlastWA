// auto-reply watcher: polls live account sessions, drains the in-page inbox
// buffered by the WPP `chat.new_message` listener, matches each message
// against the saved rules, and fires the first match as a reply.
//
// design constraints:
// - never launches chrome or waits for a QR scan; an account is only watched
//   while a live session (cdp port) exists in the sessions registry
// - per-account work runs in its own task with a hard timeout so one dead
//   port can never stall the loop for the other accounts
// - message ids are deduped per account: wa-js can redeliver events after a
//   reconnect, and a double reply to the same message looks broken
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::browser::js_injector::{InboxMessage, JsInjector};
use crate::campaign::pipeline::Pipeline;
use crate::message::spintax;
use crate::message::variables::{apply_variables, ContactRow};

use super::rules;

/// counters surfaced to the Auto Reply page via the `autoreply_status` command
pub struct WatcherStats {
    pub replies_sent: AtomicU32,
    pub last_reply_epoch: AtomicI64,
    /// account names with a watcher currently attached
    pub watching: StdMutex<Vec<String>>,
}

static STATS: std::sync::OnceLock<WatcherStats> = std::sync::OnceLock::new();

pub fn stats() -> &'static WatcherStats {
    STATS.get_or_init(|| WatcherStats {
        replies_sent: AtomicU32::new(0),
        last_reply_epoch: AtomicI64::new(0),
        watching: StdMutex::new(Vec::new()),
    })
}

fn set_watching(account: &str, on: bool) {
    let mut list = stats().watching.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        if !list.iter().any(|n| n == account) {
            list.push(account.to_string());
        }
    } else {
        list.retain(|n| n != account);
    }
}

/// watcher main loop. runs forever; safe to spawn once at app start.
pub async fn run(pipeline: Pipeline, data_dir: PathBuf) {
    log::info!("auto-reply watcher started (poll every 3s)");
    let in_flight: Arc<tokio::sync::Mutex<HashSet<String>>> =
        Arc::new(tokio::sync::Mutex::new(HashSet::new()));
    let processed: Arc<tokio::sync::Mutex<HashMap<String, HashSet<String>>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    let mut ticker = tokio::time::interval(Duration::from_secs(3));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;

        let path = data_dir.join("autoreply.json");
        let rules = match tokio::task::spawn_blocking(move || rules::load_rules(&path)).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                log::warn!("auto-reply: skipping cycle, rules unreadable: {e:#}");
                continue;
            }
            Err(_) => continue,
        };
        if !rules.iter().any(rules::Rule::is_armed) {
            for account in stats()
                .watching
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain(..)
            {
                set_watching(&account, false);
            }
            continue;
        }

        let sessions = crate::api::server::sessions_registry();
        let snapshot: Vec<(String, u16)> = sessions.lock().await.clone();
        for (account, port) in snapshot {
            {
                let mut flight = in_flight.lock().await;
                if flight.contains(&account) {
                    continue;
                }
                flight.insert(account.clone());
            }
            let pipeline = pipeline.clone();
            let rules = rules.clone();
            let processed = processed.clone();
            let in_flight = in_flight.clone();
            tokio::spawn(async move {
                let result = tokio::time::timeout(
                    // generous enough for a first-time wpp bootstrap (cdp +
                    // bundle execution has its own 90s readiness deadline);
                    // once memoized in wpp_ready, cycles finish in seconds
                    Duration::from_secs(120),
                    watch_account(&pipeline, &account, port, &rules, &processed),
                )
                .await;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        set_watching(&account, false);
                        log::debug!("auto-reply watch {account}: {e:#}");
                    }
                    Err(_) => {
                        set_watching(&account, false);
                        log::debug!("auto-reply watch {account}: cycle timed out");
                    }
                }
                in_flight.lock().await.remove(&account);
            });
        }
    }
}

/// one cycle for one account: attach, arm the listener, drain, reply.
/// every failure mode is a soft error — the next tick retries.
async fn watch_account(
    pipeline: &Pipeline,
    account: &str,
    port: u16,
    rules: &[rules::Rule],
    processed: &tokio::sync::Mutex<HashMap<String, HashSet<String>>>,
) -> anyhow::Result<()> {
    let page = pipeline
        .attach(account, port)
        .await
        .map_err(|e| anyhow::anyhow!("attach failed: {e:#}"))?;

    if !JsInjector::new(&page)
        .is_logged_in()
        .await
        .unwrap_or(false)
    {
        // qr still pending or logged out: nothing to watch yet, stay quiet
        set_watching(account, false);
        return Ok(());
    }

    pipeline
        .ensure_wpp_for(account, page.clone())
        .await
        .map_err(|e| anyhow::anyhow!("wpp bootstrap failed: {e:#}"))?;

    let injector = JsInjector::new(&page);
    injector.install_inbox_listener().await?;
    let messages = injector.drain_inbox().await.unwrap_or_default();
    if messages.is_empty() {
        set_watching(account, true);
        return Ok(());
    }

    let to_process: Vec<InboxMessage> = {
        let mut map = processed.lock().await;
        let seen = map.entry(account.to_string()).or_default();
        let mut fresh = Vec::new();
        for msg in messages {
            if seen.contains(&msg.id) {
                continue;
            }
            seen.insert(msg.id.clone());
            // bound the dedupe set; a redelivered id older than this window
            // would reply twice, but 1000 ids only cover a reconnect burst
            if seen.len() > 1000 {
                let keep = msg.id.clone();
                seen.clear();
                seen.insert(keep);
            }
            fresh.push(msg);
        }
        fresh
    };

    for msg in to_process {
        if let Err(e) = reply_to(account, &injector, msg, rules).await {
            log::warn!("auto-reply {account}: reply failed: {e:#}");
        }
    }

    set_watching(account, true);
    Ok(())
}

/// match one incoming message and send the configured reply with a small
/// humanized pause (seen -> typing -> delay -> send)
async fn reply_to(
    account: &str,
    injector: &JsInjector,
    msg: InboxMessage,
    rules: &[rules::Rule],
) -> anyhow::Result<()> {
    if msg.body.trim().is_empty() {
        return Ok(());
    }
    let Some(rule) = rules::match_rule(&msg.body, rules) else {
        return Ok(());
    };
    let Some(reply_raw) = rule.reply_message.as_deref().map(str::to_string) else {
        return Ok(());
    };

    // sender profile for [[firstname]]-style variables; the chat id doubles
    // as the phone number (e.g. 6281234567890@c.us)
    let number = msg.from.split('@').next().unwrap_or("").to_string();
    let reply = apply_variables(
        &spintax::spin(&reply_raw),
        &ContactRow::from_fullname(&number, &msg.name),
    );

    let _ = injector.mark_seen(&msg.from).await;
    let _ = injector.send_typing_state(&msg.from).await;
    // ThreadRng is !Send: drop it before the next await point
    let pause: u64 = {
        let mut rng = rand::thread_rng();
        rand::Rng::gen_range(&mut rng, 700..=1800)
    };
    tokio::time::sleep(Duration::from_millis(pause)).await;

    let sent = injector.send_message(&msg.from, &reply, false).await?;
    if sent.ok() {
        let n = stats().replies_sent.fetch_add(1, Ordering::Relaxed) + 1;
        stats().last_reply_epoch.store(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            Ordering::Relaxed,
        );
        log::info!("[auto-reply:{account}] {n} total — replied to {} with rule \"{}\"", msg.from, rule.name);
    } else {
        anyhow::bail!(
            "send rejected: {}",
            sent.error.unwrap_or_else(|| "unknown".into())
        );
    }
    Ok(())
}
