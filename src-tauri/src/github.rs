//! The latest release of a public repository, without signing in.
//!
//! Two doors. The releases API says everything (version, size, sha256) but allows 60 requests an
//! hour per address, and an event's whole pit shares one address. When it refuses, the release's
//! own `latest.json` — the file the apps' updaters read — is not rate limited and names the same
//! installer, only without a hash.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::manifest::{hex, safe_file_name};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub url: String,
    pub file: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// No route to GitHub at all: the ordinary state of a laptop at an event.
    Offline,
    RateLimited { reset_epoch: Option<u64> },
    Other(String),
}

impl Problem {
    pub fn words(&self) -> String {
        match self {
            Problem::Offline => "No connection to GitHub.".into(),
            Problem::RateLimited { .. } => {
                "GitHub's hourly limit for this network is used up. Try again later, or use a bundle with a payload folder.".into()
            }
            Problem::Other(text) => text.clone(),
        }
    }
}

pub fn agent() -> Result<ureq::Agent, String> {
    let tls = native_tls::TlsConnector::new().map_err(|e| format!("TLS is not available: {e}"))?;
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(tls))
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(30))
        .user_agent(concat!("CatalystSetup/", env!("CARGO_PKG_VERSION")))
        .build())
}

/// The installer in a `releases/latest` answer.
pub fn offer_from_release(release: &Value, tag_prefix: &str, asset_suffix: &str) -> Result<Offer, String> {
    let tag = release["tag_name"].as_str().ok_or("GitHub's answer names no release.")?;
    let version = tag.strip_prefix(tag_prefix).ok_or_else(|| {
        format!("The latest release is tagged {tag}, which is not a {tag_prefix}* release.")
    })?;
    let suffix = asset_suffix.to_ascii_lowercase();
    let asset = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find(|a| a["name"].as_str().is_some_and(|n| n.to_ascii_lowercase().ends_with(&suffix)))
        })
        .ok_or_else(|| format!("Release {tag} has no Windows installer (*{asset_suffix})."))?;
    let file = asset["name"].as_str().unwrap_or_default().to_string();
    if !safe_file_name(&file) {
        return Err(format!("Release {tag} names an installer this setup will not save: {file}"));
    }
    let url = asset["browser_download_url"].as_str().ok_or("The installer has no download address.")?;
    if !url.starts_with("https://github.com/") {
        return Err("The installer's download address is not on github.com.".into());
    }
    Ok(Offer {
        version: version.to_string(),
        url: url.to_string(),
        file,
        size: asset["size"].as_u64(),
        sha256: asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_string),
    })
}

/// The installer in a Tauri updater `latest.json`.
pub fn offer_from_latest_json(latest: &Value, asset_suffix: &str) -> Result<Offer, String> {
    let version = latest["version"].as_str().ok_or("latest.json names no version.")?;
    let url = latest["platforms"]["windows-x86_64"]["url"].as_str().ok_or("latest.json has no Windows build.")?;
    if !url.starts_with("https://github.com/") {
        return Err("latest.json points away from github.com.".into());
    }
    let raw = url.rsplit('/').next().unwrap_or_default();
    let file = raw.replace("%20", " ");
    if !file.to_ascii_lowercase().ends_with(&asset_suffix.to_ascii_lowercase()) || !safe_file_name(&file) {
        return Err("latest.json's Windows build is not an installer this setup can run.".into());
    }
    Ok(Offer { version: version.trim_start_matches('v').to_string(), url: url.to_string(), file, size: None, sha256: None })
}

/// Turn a refusal into what it means. 403 and 429 are GitHub's two ways of saying "limit".
pub fn classify_status(code: u16, remaining: Option<&str>, reset: Option<&str>) -> Problem {
    if code == 429 || (code == 403 && remaining == Some("0")) {
        return Problem::RateLimited { reset_epoch: reset.and_then(|r| r.parse().ok()) };
    }
    match code {
        404 => Problem::Other("GitHub has no release for this app yet.".into()),
        _ => Problem::Other(format!("GitHub answered {code}.")),
    }
}

fn get_json(agent: &ureq::Agent, url: &str) -> Result<Value, Problem> {
    match agent.get(url).set("Accept", "application/vnd.github+json").call() {
        Ok(response) => response.into_json().map_err(|e| Problem::Other(format!("GitHub's answer could not be read: {e}"))),
        Err(ureq::Error::Status(code, response)) => {
            Err(classify_status(code, response.header("x-ratelimit-remaining"), response.header("x-ratelimit-reset")))
        }
        Err(ureq::Error::Transport(_)) => Err(Problem::Offline),
    }
}

pub fn fetch_offer(agent: &ureq::Agent, repo: &str, tag_prefix: &str, asset_suffix: &str) -> Result<Offer, Problem> {
    match get_json(agent, &format!("https://api.github.com/repos/{repo}/releases/latest")) {
        Ok(release) => offer_from_release(&release, tag_prefix, asset_suffix).map_err(Problem::Other),
        Err(Problem::RateLimited { reset_epoch }) => {
            let fallback = get_json(agent, &format!("https://github.com/{repo}/releases/latest/download/latest.json"))
                .ok()
                .and_then(|latest| offer_from_latest_json(&latest, asset_suffix).ok());
            fallback.ok_or(Problem::RateLimited { reset_epoch })
        }
        Err(problem) => Err(problem),
    }
}

/// Download to `dest`, through a `.part` file so a half download is never mistaken for a whole one.
/// `progress(received, total)` is called as bytes arrive; `total` is what the server promised.
pub fn download(
    agent: &ureq::Agent,
    offer: &Offer,
    dest: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let response = match agent.get(&offer.url).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(code, _)) => return Err(format!("The download was refused ({code}).")),
        Err(ureq::Error::Transport(_)) => return Err("The connection to GitHub was lost before the download began.".into()),
    };
    let promised = response.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).or(offer.size);
    let part = dest.with_extension("part");
    let mut file = File::create(&part).map_err(|e| format!("Could not write to {}: {e}", part.display()))?;
    let received = copy_hashing(response.into_reader(), &mut file, |n| progress(n, promised));
    drop(file);
    let (received, sha) = received.map_err(|e| format!("The download stopped partway: {e}"))?;
    check_download(received, &sha, promised, offer)?;
    std::fs::rename(&part, dest).map_err(|e| format!("Could not save {}: {e}", dest.display()))
}

/// Copy, hashing on the way, so the bytes are checked without reading the file a second time.
pub fn copy_hashing(mut from: impl Read, to: &mut impl Write, mut progress: impl FnMut(u64)) -> std::io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut received = 0u64;
    loop {
        let read = from.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        to.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        received += read as u64;
        progress(received);
    }
    to.flush()?;
    Ok((received, hex(&hasher.finalize())))
}

/// A download is complete when it is as long as promised and, where GitHub published a hash, when
/// it has that hash.
pub fn check_download(received: u64, sha256: &str, promised: Option<u64>, offer: &Offer) -> Result<(), String> {
    for expected in [promised, offer.size].into_iter().flatten() {
        if received != expected {
            return Err(format!("The download is incomplete: {received} of {expected} bytes arrived."));
        }
    }
    if received == 0 {
        return Err("The download was empty.".into());
    }
    match &offer.sha256 {
        Some(expected) if !expected.eq_ignore_ascii_case(sha256) => {
            Err("The download does not match the sha256 GitHub publishes for it. Nothing was installed from it.".into())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release() -> Value {
        json!({
            "tag_name": "v2.0.0",
            "assets": [
                { "name": "catalyst-console.exe", "size": 5495808, "browser_download_url": "https://github.com/TomAs-1226/CatalystConsole/releases/download/v2.0.0/catalyst-console.exe" },
                { "name": "Catalyst.Console_2.0.0_universal.dmg", "size": 5594281, "browser_download_url": "https://github.com/x/y.dmg" },
                { "name": "Catalyst.Console_2.0.0_x64-setup.exe", "size": 2367343,
                  "digest": "sha256:cb22db8701746e98fdaa2f6cb1eb18a2942b7612ea1d8e1e66a70526b5d8397c",
                  "browser_download_url": "https://github.com/TomAs-1226/CatalystConsole/releases/download/v2.0.0/Catalyst.Console_2.0.0_x64-setup.exe" },
                { "name": "Catalyst.Console_2.0.0_x64-setup.exe.sig", "size": 428, "browser_download_url": "https://github.com/x/y.sig" }
            ]
        })
    }

    #[test]
    fn picks_the_setup_exe_and_not_the_bare_exe_or_its_signature() {
        let offer = offer_from_release(&release(), "v", "-setup.exe").unwrap();
        assert_eq!(offer.version, "2.0.0");
        assert_eq!(offer.file, "Catalyst.Console_2.0.0_x64-setup.exe");
        assert_eq!(offer.size, Some(2367343));
        assert_eq!(offer.sha256.as_deref(), Some("cb22db8701746e98fdaa2f6cb1eb18a2942b7612ea1d8e1e66a70526b5d8397c"));
    }

    #[test]
    fn the_app_tag_prefix_is_stripped_and_a_foreign_tag_is_refused() {
        let mut r = release();
        r["tag_name"] = json!("app-v2.8.0");
        assert_eq!(offer_from_release(&r, "app-v", "-setup.exe").unwrap().version, "2.8.0");
        r["tag_name"] = json!("nightly");
        assert!(offer_from_release(&r, "app-v", "-setup.exe").unwrap_err().contains("nightly"));
    }

    #[test]
    fn a_release_without_an_installer_says_so() {
        let r = json!({ "tag_name": "v1.0.0", "assets": [{ "name": "notes.txt" }] });
        assert!(offer_from_release(&r, "v", "-setup.exe").unwrap_err().contains("no Windows installer"));
    }

    #[test]
    fn a_download_address_off_github_is_refused() {
        let mut r = release();
        r["assets"][2]["browser_download_url"] = json!("https://example.com/setup.exe");
        assert!(offer_from_release(&r, "v", "-setup.exe").is_err());
    }

    #[test]
    fn latest_json_is_a_second_way_in() {
        let latest = json!({ "version": "2.0.0", "platforms": { "windows-x86_64": {
            "url": "https://github.com/TomAs-1226/CatalystConsole/releases/download/v2.0.0/Catalyst%20Console_2.0.0_x64-setup.exe" } } });
        let offer = offer_from_latest_json(&latest, "-setup.exe").unwrap();
        assert_eq!(offer.version, "2.0.0");
        assert_eq!(offer.file, "Catalyst Console_2.0.0_x64-setup.exe");
        assert_eq!(offer.sha256, None);

        let msi = json!({ "version": "2.0.0", "platforms": { "windows-x86_64": { "url": "https://github.com/a/b/releases/download/v2/App.msi" } } });
        assert!(offer_from_latest_json(&msi, "-setup.exe").is_err());
    }

    #[test]
    fn refusals_are_told_apart() {
        assert_eq!(classify_status(403, Some("0"), Some("1790000000")), Problem::RateLimited { reset_epoch: Some(1790000000) });
        assert_eq!(classify_status(429, None, None), Problem::RateLimited { reset_epoch: None });
        assert!(matches!(classify_status(403, Some("41"), None), Problem::Other(_)));
        assert!(classify_status(404, None, None).words().contains("no release"));
    }

    #[test]
    fn copying_hashes_what_it_copies() {
        let mut out = Vec::new();
        let mut last = 0;
        let (n, sha) = copy_hashing(&b"abc"[..], &mut out, |r| last = r).unwrap();
        assert_eq!((n, last, out.as_slice()), (3, 3, &b"abc"[..]));
        assert_eq!(sha, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn a_download_is_checked_for_length_and_hash() {
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let offer = |size, hash: Option<&str>| Offer {
            version: "1".into(), url: String::new(), file: "a.exe".into(), size, sha256: hash.map(str::to_string),
        };
        assert!(check_download(3, sha, Some(3), &offer(Some(3), Some(sha))).is_ok());
        assert!(check_download(2, sha, Some(3), &offer(None, None)).unwrap_err().contains("incomplete"));
        assert!(check_download(3, sha, None, &offer(Some(4), None)).unwrap_err().contains("incomplete"));
        assert!(check_download(3, "00", Some(3), &offer(Some(3), Some(sha))).unwrap_err().contains("sha256"));
        assert!(check_download(0, sha, None, &offer(None, None)).unwrap_err().contains("empty"));
        // No hash published (the latest.json door): length is all there is to check.
        assert!(check_download(3, "anything", Some(3), &offer(None, None)).is_ok());
    }
}
