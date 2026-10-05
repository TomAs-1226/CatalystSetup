//! `suite.json`, the one table of what the suite is. Embedded at build time so the portable exe
//! carries it and nothing beside the exe can change what gets installed.

use serde::{Deserialize, Serialize};

pub const SUITE_JSON: &str = include_str!("../../suite.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suite {
    pub schema: u32,
    pub uninstall_root: String,
    pub setup: Setup,
    pub apps: Vec<App>,
    pub laptop: Vec<LaptopItem>,
}

/// Catalyst Setup's own entry: its releases, its installed copy and its scheduled task.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    pub repo: String,
    pub tag_prefix: String,
    pub asset: String,
    pub install_folder: String,
    pub uninstall_key: String,
    pub task: String,
    pub every_hours: u32,
    pub driver_station_processes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct App {
    pub id: String,
    pub name: String,
    pub blurb: String,
    pub icon: Option<String>,
    /// Ticked without being asked. An optional app is offered, never assumed.
    pub default: bool,
    pub source: Source,
    pub installer: Installer,
    pub detect: Detect,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Source {
    /// The latest release of a public repository; the payload folder still wins when it is as new.
    #[serde(rename_all = "camelCase")]
    Github { repo: String, tag_prefix: String, asset_suffix: String },
    /// No download exists. The payload folder is the only place it comes from.
    PayloadOnly,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installer {
    pub kind: String,
    pub silent_args: Vec<String>,
    #[serde(default)]
    pub log: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Detect {
    pub hive: String,
    pub key: String,
    pub exe: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaptopItem {
    pub id: String,
    pub name: String,
    pub url: String,
    pub url_label: String,
}

pub fn parse(text: &str) -> Result<Suite, String> {
    let suite: Suite = serde_json::from_str(text).map_err(|e| format!("suite.json: {e}"))?;
    let mut seen = std::collections::HashSet::new();
    for app in &suite.apps {
        if !seen.insert(app.id.as_str()) {
            return Err(format!("suite.json: two apps share the id {}", app.id));
        }
        if app.detect.hive != "HKCU" && app.detect.hive != "HKLM" {
            return Err(format!("suite.json: {} has the hive {}", app.id, app.detect.hive));
        }
        if app.installer.silent_args.is_empty() {
            // Without one the installer would open its own window and wait for a person.
            return Err(format!("suite.json: {} has no silent switch", app.id));
        }
    }
    Ok(suite)
}

pub fn load() -> Suite {
    parse(SUITE_JSON).expect("the embedded suite.json is checked by the tests")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_suite_parses_and_holds_the_contract() {
        let suite = parse(SUITE_JSON).unwrap();
        let ids: Vec<&str> = suite.apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["catalyst", "console", "sim", "pit", "link"]);

        let by = |id: &str| suite.apps.iter().find(|a| a.id == id).unwrap();
        assert_eq!(by("catalyst").detect.key, "Catalyst");
        assert_eq!(by("catalyst").detect.exe, "catalyst-app.exe");
        assert_eq!(by("console").detect.key, "Catalyst Console");
        assert_eq!(by("console").detect.exe, "catalyst-console.exe");
        assert_eq!(by("sim").detect.key, "CatalystSimMO");
        assert_eq!(by("sim").detect.exe, "CatalystSim.exe");
        assert_eq!(by("sim").installer.silent_args, ["/silent"]);
        assert_eq!(by("pit").detect.key, "Catalyst Pit");
        assert_eq!(by("pit").detect.exe, "catalyst-pit.exe");
        assert_eq!(by("link").detect.exe, "catalyst-link-desktop.exe");
        assert!(!by("link").default, "Link is optional and starts unticked");
        assert!(matches!(by("sim").source, Source::PayloadOnly));
        assert!(matches!(by("link").source, Source::PayloadOnly));
        match &by("pit").source {
            Source::Github { repo, tag_prefix, asset_suffix } => {
                assert_eq!((repo.as_str(), tag_prefix.as_str(), asset_suffix.as_str()), ("TomAs-1226/CatalystPit", "v", "-setup.exe"));
            }
            other => panic!("pit should come from GitHub, not {other:?}"),
        }
        assert_eq!(suite.setup.repo, "TomAs-1226/CatalystSetup");
        assert_eq!(suite.setup.asset, "catalyst-setup.exe");
        assert_eq!(suite.setup.uninstall_key, "Catalyst Setup");
        assert!(suite.setup.driver_station_processes.iter().any(|p| p == "DriverStation.exe"));
        assert!((1..=23).contains(&suite.setup.every_hours), "schtasks takes 1 to 23 hours");
        match &by("catalyst").source {
            Source::Github { repo, tag_prefix, .. } => {
                assert_eq!(repo, "TomAs-1226/CatalystApp");
                assert_eq!(tag_prefix, "app-v");
            }
            other => panic!("catalyst should come from GitHub, not {other:?}"),
        }
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let mut value: serde_json::Value = serde_json::from_str(SUITE_JSON).unwrap();
        let first = value["apps"][0].clone();
        value["apps"].as_array_mut().unwrap().push(first);
        assert!(parse(&value.to_string()).unwrap_err().contains("share the id"));
    }
}
