use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::api::server;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStatus {
    pub name: String,
    pub port: Option<u16>,
    pub browser_running: bool,
    pub wa_authenticated: bool,
    pub connected: bool,
    pub number: Option<String>,
}

#[derive(Clone)]
pub struct AccountService {
    app_dir: PathBuf,
    accounts_dir: PathBuf,
    mutations: Arc<Mutex<()>>,
}

impl AccountService {
    pub fn new(app_dir: PathBuf, accounts_dir: PathBuf) -> Self {
        Self {
            app_dir,
            accounts_dir,
            mutations: Arc::new(Mutex::new(())),
        }
    }

    pub fn app_dir(&self) -> &Path {
        &self.app_dir
    }

    pub fn account_dir(&self, name: &str) -> PathBuf {
        self.accounts_dir.join(crate::config::settings::sanitize_name(name))
    }

    pub fn load_names(&self) -> Vec<String> {
        server::load_saved_accounts(&self.app_dir)
    }

    pub fn save_name(&self, name: &str) -> std::io::Result<()> {
        server::save_account_name(&self.app_dir, name)
    }

    pub fn remove_name(&self, name: &str) -> std::io::Result<()> {
        server::remove_saved_account(&self.app_dir, name)
    }

    pub fn clear_names(&self) -> std::io::Result<()> {
        server::clear_saved_accounts(&self.app_dir)
    }

    pub async fn lock(&self) -> OwnedMutexGuard<()> {
        self.mutations.clone().lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn mutation_lock_serializes_account_operations() {
        let service = AccountService::new(PathBuf::from("."), PathBuf::from("."));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..4 {
            let service = service.clone();
            let active = active.clone();
            let peak = peak.clone();
            tasks.push(tokio::spawn(async move {
                let _guard = service.lock().await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(2)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }
}
