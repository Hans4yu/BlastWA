// AccountSession: one chrome process per wa account, connected via CDP.
// chrome stays the real browser (real fingerprint), we just drive it.
use std::path::PathBuf;

use anyhow::{Context, Result};
use chromiumoxide::browser::Browser;
use chromiumoxide::Page;

pub struct AccountSession {
    pub name: String,
    pub port: u16,
    pub page: Page,
    #[allow(dead_code)]
    user_data_dir: PathBuf,
    /// keeps the cdp event pump alive for this browser instance
    #[allow(dead_code)]
    handler_task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
pub struct SessionManager {
    pub accounts_dir: PathBuf,
    pub chrome_path: String,
}

impl SessionManager {
    pub fn new(accounts_dir: PathBuf, chrome_path: String) -> Self {
        Self {
            accounts_dir,
            chrome_path,
        }
    }

    /// launch chrome with an isolated profile + cdp port, then attach.
    /// retries because chrome takes a moment to open the debug socket.
    pub async fn launch(&self, name: &str, port: u16) -> Result<AccountSession> {
        let user_data_dir = self.accounts_dir.join(name);
        std::fs::create_dir_all(&user_data_dir)?;

        std::process::Command::new(&self.chrome_path)
            .arg(format!("--remote-debugging-port={port}"))
            .arg(format!(
                "--user-data-dir={}",
                user_data_dir.to_string_lossy()
            ))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-timer-throttling")
            .spawn()
            .context("spawning chrome")?;

        let mut last_err = None;
        // adaptive backoff: 500ms → 1000ms → 1500ms → 2000ms (cap), total
        // budget ~25 s. windows cold-starts routinely need 10-18 s before
        // the chrome debug socket is ready.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
        let mut delay = std::time::Duration::from_millis(500);
        loop {
            tokio::time::sleep(delay).await;
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            match Browser::connect(format!("http://127.0.0.1:{port}")).await {
                Ok((browser, mut handler)) => {
                    // cdp handler is a stream that must be polled or nothing works
                    let handler_task = tokio::spawn(async move {
                        use futures::StreamExt;
                        while let Some(event) = handler.next().await {
                            if event.is_err() {
                                break;
                            }
                        }
                    });
                    let page = browser
                        .new_page("https://web.whatsapp.com")
                        .await
                        .context("opening whatsapp web tab")?;
                    return Ok(AccountSession {
                        name: name.into(),
                        port,
                        page,
                        user_data_dir,
                        handler_task,
                    });
                }
                Err(e) => {
                    last_err = Some(e);
                    delay = (delay + std::time::Duration::from_millis(500))
                        .min(std::time::Duration::from_secs(2));
                }
            }
        }
        Err(anyhow::anyhow!(
            "could not connect to chrome cdp: {:?}",
            last_err.map(|e| e.to_string())
        ))
    }
}

/// discover the real listening cdp endpoint after a chrome launch.
/// needed because when chrome is already running with the same user-data-dir,
/// a new spawn attaches to the existing process and the port we passed is
/// never opened. scans the candidate port first, then the usual range,
/// preferring endpoints that currently host a web.whatsapp.com tab.
///
/// `taken` holds ports already claimed by other accounts: without it a
/// second account discovers the FIRST whatsapp endpoint in the range and
/// silently binds to another account's session, so a multi-account blast
/// sends every chunk from the same number.
pub async fn discover_wa_port_excluding(
    candidate: Option<u16>,
    taken: &[u16],
) -> Option<u16> {
    let mut ports = Vec::new();
    if let Some(c) = candidate {
        ports.push(c);
    }
    ports.extend(9222..=9241);

    let mut any_alive: Option<u16> = None;
    for p in ports {
        if taken.contains(&p) {
            continue;
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", p)).await.is_err() {
            continue;
        }
        if has_whatsapp_target(p).await {
            return Some(p);
        }
        if any_alive.is_none() {
            any_alive = Some(p);
        }
    }
    any_alive
}

pub async fn discover_wa_port(candidate: Option<u16>) -> Option<u16> {
    discover_wa_port_excluding(candidate, &[]).await
}

async fn has_whatsapp_target(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/json/list");
    let Ok(body) = reqwest::get(&url).await else {
        return false;
    };
    let Ok(text) = body.text().await else {
        return false;
    };
    text.contains("web.whatsapp.com")
}

/// find a free tcp port starting from `start`
pub async fn find_free_port(start: u16) -> u16 {
    for p in start..start + 100 {
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return p;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    start
}
