use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileProcess {
    pub name: String,
    pub pid: u32,
}

fn registry_path(root: &Path) -> PathBuf {
    root.join("profile-processes.json")
}

pub fn load(root: &Path) -> Vec<ProfileProcess> {
    let path = registry_path(root);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(value) => value
            .as_array()
            .or_else(|| value.get("processes").and_then(|v| v.as_array()))
            .and_then(|items| serde_json::from_value(serde_json::Value::Array(items.clone())).ok())
            .unwrap_or_default(),
        Err(error) => {
            crate::config::settings::backup_corrupt_file(&path);
            log::warn!("profile process registry unreadable: {error}");
            Vec::new()
        }
    }
}

fn save(root: &Path, entries: &[ProfileProcess]) -> std::io::Result<()> {
    let path = registry_path(root);
    let _lock = crate::config::settings::FileLock::acquire(&path)?;
    crate::config::settings::atomic_write(
        &path,
        &serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": crate::config::settings::STORAGE_SCHEMA_VERSION,
            "processes": entries,
        }))?,
    )
}

pub fn prune(root: &Path) -> std::io::Result<Vec<ProfileProcess>> {
    let live: Vec<_> = load(root).into_iter().filter(is_process_alive).collect();
    save(root, &live)?;
    Ok(live)
}

pub fn register(root: &Path, entry: ProfileProcess) -> std::io::Result<()> {
    let mut entries = prune(root)?;
    entries.retain(|old| old.name != entry.name && old.pid != entry.pid);
    entries.push(entry);
    save(root, &entries)
}

pub fn unregister(root: &Path, name: &str, pid: u32) -> std::io::Result<()> {
    let entries: Vec<_> = load(root)
        .into_iter()
        .filter(|entry| entry.name != name || entry.pid != pid)
        .collect();
    save(root, &entries)
}

#[cfg(windows)]
fn is_process_alive(entry: &ProfileProcess) -> bool {
    use std::process::Command;
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", entry.pid), "/NH"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).contains(&entry.pid.to_string()))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_process_alive(entry: &ProfileProcess) -> bool {
    std::path::Path::new(&format!("/proc/{}", entry.pid)).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_replaces_stale_name_entry() {
        let root = std::env::temp_dir().join(format!("blastwa_registry_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        save(&root, &[ProfileProcess { name: "work".into(), pid: u32::MAX }]).unwrap();
        register(&root, ProfileProcess { name: "work".into(), pid: std::process::id() }).unwrap();
        assert_eq!(load(&root), vec![ProfileProcess { name: "work".into(), pid: std::process::id() }]);
        let _ = std::fs::remove_dir_all(root);
    }
}
