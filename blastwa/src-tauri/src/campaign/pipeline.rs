// campaign pipeline: consumes BlastRequest from the api server and drives
// the real blast loop — chrome session per account, qr wait, progress counters.
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::sync::{mpsc, Mutex};

use crate::api::server::{AppState, BlastRequest};
use crate::browser::cdp_client::{find_free_port, SessionManager};
use crate::browser::js_injector::JsInjector;
use crate::campaign::contact_list::{normalize_number, ContactList};
use crate::campaign::sender::{run_campaign, CampaignConfig};
use crate::message::variables::ContactRow;

#[derive(Clone)]
pub struct Pipeline {
    pub state: AppState,
    pub sessions: SessionManager,
    /// live wa web page handles keyed by account name
    pub pages: Arc<Mutex<HashMap<String, chromiumoxide::Page>>>,
}

impl Pipeline {
    pub fn new(state: AppState, chrome_path: String, accounts_dir: std::path::PathBuf) -> Self {
        Self {
            state,
            sessions: SessionManager::new(accounts_dir, chrome_path),
            pages: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// main loop: take requests off the channel, run each in its own task
    pub async fn serve(&self, mut rx: mpsc::Receiver<BlastRequest>) {
        while let Some(req) = rx.recv().await {
            let this = self.clone();
            tokio::spawn(async move {
                if let Err(e) = this.execute(req).await {
                    log::error!("blast failed: {e:#}");
                }
            });
        }
    }

    pub async fn get_page(&self, account: &str) -> Result<chromiumoxide::Page> {
        let mut pages = self.pages.lock().await;

        // reuse a live session when we have one
        if let Some(page) = pages.get(account) {
            let probe = JsInjector::new(page).is_logged_in().await;
            if probe.unwrap_or(false) {
                return Ok(page.clone());
            }
            log::warn!("session for {account} is dead, relaunching");
            pages.remove(account);
        }

        let port = find_free_port(9222).await;
        let session = self
            .sessions
            .launch(account, port)
            .await
            .with_context(|| format!("launching chrome for account {account}"))?;
        let page = session.page.clone();

        // first launch requires a manual QR scan; give the user 3 minutes.
        // session persists on disk after that (user-data-dir), so this is once ever.
        let mut injector = JsInjector::new(&page);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        let mut announced = false;
        let mut next_debug = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if injector.is_logged_in().await.unwrap_or(false) {
                match injector.my_user_id().await {
                    Ok(id) => log::info!("account {account}: logged in as {id}"),
                    Err(_) => log::info!("account {account}: logged in"),
                }
                break;
            }
            if !announced {
                log::warn!(
                    "account {account}: waiting for WhatsApp QR scan in the opened Chrome window (3 min timeout)"
                );
                announced = true;
            }
            // debug: peek what the page actually shows every ~15s
            if tokio::time::Instant::now() >= next_debug {
                next_debug = tokio::time::Instant::now() + Duration::from_secs(15);
                let probe = self
                    .eval_debug(&page)
                    .await
                    .unwrap_or_else(|| "probe failed".into());
                log::info!("account {account}: qr-wait probe: {probe}");
            }
            if tokio::time::Instant::now() >= deadline {
                bail!(
                    "account {account}: not logged in after 3 minutes — scan the QR in the opened Chrome window, then retry"
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        // inject WPP.js before any campaign work — from disk cache if the
        // updater downloaded one, else CDN
        let wpp_local = std::path::Path::new(&crate::config::settings::AppConfig::app_dir())
            .join("wpp")
            .join("wpp.js");
        let wpp_code = std::fs::read_to_string(&wpp_local).ok();
        if wpp_code.is_some() {
            log::info!("account {account}: injecting WPP.js from disk cache");
        }
        injector
            .ensure_wpp(wpp_code.as_deref())
            .await
            .with_context(|| format!("account {account}: WPP.js bootstrap"))?;

        pages.insert(account.to_string(), page.clone());

        // expose to /api/accounts
        let reg = crate::api::server::sessions_registry();
        let mut list = reg.lock().await;
        if !list.iter().any(|(n, _): &(String, u16)| n == account) {
            list.push((account.to_string(), port));
        }

        Ok(page)
    }

    async fn eval_debug(&self, page: &chromiumoxide::Page) -> Option<String> {
        let v = page
            .evaluate(
                r#"(function(){
                    try {
                        var wid = (window.localStorage && window.localStorage.getItem('last-wid')) || 'null';
                        var hasQr = !!document.querySelector('canvas');
                        var appEl = !!document.querySelector('#app');
                        var bodyLen = (document.body && document.body.innerText || '').length;
                        return 'wid=' + wid + ' qr=' + hasQr + ' text=' + (document.body.innerText || '').substring(0, 150).replace(/
/g, ' | ');
                    } catch(e) { return 'probe error: ' + String(e); }
                })()"#,
            )
            .await
            .ok()?;
        v.into_value::<String>().ok()
    }

    /// non-launching page accessor for status probes.
    /// returns None when no live session exists for the account.
    pub async fn page_handle(&self, name: &str) -> Option<chromiumoxide::Page> {
        let pages = self.pages.lock().await;
        pages.get(name).cloned()
    }

    /// public entry for gui commands: session + wpp bootstrap in one call
    pub async fn get_injector(&self, account: &str) -> Result<crate::browser::js_injector::JsInjector> {
        let page = self.get_page(account).await?;
        Ok(crate::browser::js_injector::JsInjector::new(&page))
    }

    async fn execute(&self, req: BlastRequest) -> Result<()> {
        // double-check despite the api guard (race between concurrent posts)
        if self.state.running.load(Ordering::Relaxed) {
            bail!("campaign already running");
        }

        let page = self.get_page(&req.account).await?;
        let injector = JsInjector::new(&page);

        // numbers come as "628123" or "628123|Budi Santoso"
        let mut contacts = ContactList::default();
        for raw in &req.contacts {
            let mut parts = raw.splitn(2, '|');
            let number = normalize_number(parts.next().unwrap_or(""));
            if number.is_empty() {
                continue;
            }
            contacts.contacts.push(ContactRow::from_fullname(
                &number,
                parts.next().unwrap_or("").trim(),
            ));
        }
        if contacts.is_empty() {
            bail!("no valid numbers in request");
        }

        let cfg = CampaignConfig {
            account_name: req.account.clone(),
            delay_min_s: req.delay_min_s.max(0.5), // hard floor 500ms
            delay_max_s: req.delay_max_s.max(req.delay_min_s),
            ..Default::default()
        };

        // reset counters, flip running flag
        self.state.sent.store(0, Ordering::Relaxed);
        self.state.failed.store(0, Ordering::Relaxed);
        self.state.running.store(true, Ordering::Relaxed);

        // fresh cancel token per campaign so previous stops don't leak
        let token = {
            let mut guard = self.state.stop_flag.lock().await;
            *guard = tokio_util::sync::CancellationToken::new();
            guard.clone()
        };

        let counters = self.state.clone();
        let result = run_campaign(
            injector,
            &contacts,
            &req.message,
            None,
            "",
            &cfg,
            token,
            move |progress| {
                counters.sent.store(progress.sent, Ordering::Relaxed);
                counters.failed.store(progress.failed, Ordering::Relaxed);
                log::info!(
                    "[{}] {}/{} sent, {} failed — {} ({})",
                    req.account,
                    progress.sent,
                    progress.sent + progress.pending as u32 + progress.failed,
                    progress.failed,
                    progress.current_number,
                    progress.status
                );
            },
        )
        .await;

        self.state.running.store(false, Ordering::Relaxed);

        match result {
            Ok(stats) => {
                log::info!(
                    "campaign done: {} sent, {} failed{}",
                    stats.sent.load(Ordering::Relaxed),
                    stats.failed.load(Ordering::Relaxed),
                    if stats.cancelled.load(Ordering::Relaxed) { " (cancelled)" } else { "" }
                );
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}
