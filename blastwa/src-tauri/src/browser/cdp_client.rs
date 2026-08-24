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
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
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
                Err(e) => last_err = Some(e),
            }
        }
        Err(anyhow::anyhow!(
            "could not connect to chrome cdp: {:?}",
            last_err.map(|e| e.to_string())
        ))
    }
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
