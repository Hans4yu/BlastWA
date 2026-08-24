// WPP.js auto-updater (U18): fetch latest wa-js release from github,
// hot-swap the injected bundle with automatic rollback on failure.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

const RELEASES_API: &str =
    "https://api.github.com/repos/wppconnect-team/wa-js/releases/latest";
const USER_AGENT: &str = "BlastWA/0.1";

#[derive(Debug, Clone, Deserialize)]
pub struct WppVersion {
    #[serde(rename = "tag_name")]
    pub tag_name: String,
    #[serde(skip)]
    pub download_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

fn wpp_dir(base: &Path) -> PathBuf {
    base.join("wpp")
}

fn wpp_js_path(base: &Path) -> PathBuf {
    wpp_dir(base).join("wpp.js")
}

fn version_path(base: &Path) -> PathBuf {
    wpp_dir(base).join("version.txt")
}

pub fn current_version(app_dir: &Path) -> Option<String> {
    std::fs::read_to_string(version_path(app_dir))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub async fn check_latest() -> Result<WppVersion> {
    let client = reqwest::Client::new();
    let release: GhRelease = client
        .get(RELEASES_API)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .context("github releases request failed")?
        .json()
        .await
        .context("parsing github release json")?;

    // prefer a bundled js asset if published; otherwise note that we inject
    // from source dist via npm cdn at runtime
    let download_url = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".js"))
        .map(|a| a.browser_download_url.clone());

    Ok(WppVersion {
        tag_name: release.tag_name,
        download_url,
    })
}

/// download + atomically swap wpp.js. rollback restores .bak on any failure.
pub async fn update(app_dir: &Path, version: &WppVersion) -> Result<String> {
    let dir = wpp_dir(app_dir);
    std::fs::create_dir_all(&dir)?;

    let js_path = wpp_js_path(app_dir);
    let bak_path = dir.join("wpp.js.bak");
    let new_path = dir.join("wpp.js.new");

    let body = match &version.download_url {
        Some(url) => download(url).await?,
        None => download_from_cdn(&version.tag_name).await?,
    };

    std::fs::write(&new_path, &body).context("writing wpp.js.new")?;

    // sanity check: must contain our entry symbol
    let body_text = String::from_utf8_lossy(&body);
    if !body_text.contains("WPP") {
        // corrupt download -> restore backup if we had one
        if bak_path.exists() {
            std::fs::copy(&bak_path, &js_path).ok();
        }
        anyhow::bail!("downloaded bundle looks invalid (no WPP symbol), rolled back");
    }

    // swap with backup
    if js_path.exists() {
        std::fs::copy(&js_path, &bak_path).context("backing up current wpp.js")?;
    }
    std::fs::rename(&new_path, &js_path).context("activating new wpp.js")?;
    std::fs::write(version_path(app_dir), &version.tag_name)?;

    Ok(version.tag_name.clone())
}

async fn download(url: &str) -> Result<Vec<u8>> {
    let resp = reqwest::Client::new()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .context("download failed")?
        .error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

async fn download_from_cdn(tag: &str) -> Result<Vec<u8>> {
    // wa-js publishes dist on unpkg per version
    let url = format!("https://unpkg.com/@wppconnect/wa-js@{tag}/dist/wpp.js");
    download(&url).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_missing_file_is_none() {
        assert_eq!(current_version(Path::new("/nonexistent")), None);
    }
}
