//! What is already on this laptop. Read-only: nothing here writes to the registry or the disk.
//!
//! The registry is behind a trait so the rules can be tested against a made-up one. The rules that
//! are easy to get wrong, all measured on a real install:
//!
//! - Tauri's NSIS installer writes `InstallLocation` wrapped in double quotes. The Sim's own
//!   installer writes it bare. Both have to resolve.
//! - An uninstall key can outlive the program. An app counts as installed only when its exe is
//!   really where the key says.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::suite::{Detect, LaptopItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

pub trait Registry {
    fn value(&self, hive: Hive, path: &str, name: &str) -> Option<String>;
    fn subkeys(&self, hive: Hive, path: &str) -> Vec<String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Installed {
    pub version: Option<String>,
    pub location: String,
    pub exe: String,
}

fn hive(name: &str) -> Hive {
    if name == "HKLM" { Hive::LocalMachine } else { Hive::CurrentUser }
}

/// `"C:\Users\a\AppData\Local\Catalyst"` and `C:\...\Catalyst\` both mean the same folder.
pub fn clean_location(raw: &str) -> String {
    raw.trim().trim_matches('"').trim().trim_end_matches(['\\', '/']).to_string()
}

pub fn detect_app(
    registry: &dyn Registry,
    uninstall_root: &str,
    detect: &Detect,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<Installed> {
    let hive = hive(&detect.hive);
    let key = format!("{uninstall_root}\\{}", detect.key);
    let location = clean_location(&registry.value(hive, &key, "InstallLocation")?);
    if location.is_empty() {
        return None;
    }
    // Tauri records the binary it installed; trust the suite table first and that second, so a
    // renamed binary is found rather than reported as missing.
    let mut names = vec![detect.exe.clone()];
    if let Some(recorded) = registry.value(hive, &key, "MainBinaryName") {
        if !recorded.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(&recorded)) {
            names.push(recorded);
        }
    }
    let exe = names.iter().map(|n| PathBuf::from(&location).join(n)).find(|p| exists(p))?;
    let version = registry
        .value(hive, &key, "DisplayVersion")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    Some(Installed { version, location, exe: exe.to_string_lossy().into_owned() })
}

// ---------------------------------------------------------------- this laptop

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaptopFinding {
    pub id: String,
    pub name: String,
    pub found: bool,
    /// What was found, in the words the laptop itself uses for it.
    pub detail: Option<String>,
    pub url: String,
    pub url_label: String,
}

const UNINSTALL_ROOTS: [&str; 2] = [
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
];

const WEBVIEW2_CLIENT: &str = "Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

/// Every machine-wide program as (name, version), from both registry views.
fn machine_programs(registry: &dyn Registry) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for root in UNINSTALL_ROOTS {
        for key in registry.subkeys(Hive::LocalMachine, root) {
            let path = format!("{root}\\{key}");
            if let Some(name) = registry.value(Hive::LocalMachine, &path, "DisplayName") {
                out.push((name, registry.value(Hive::LocalMachine, &path, "DisplayVersion")));
            }
        }
    }
    out
}

pub fn is_driver_station(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("driver station") && (n.contains("first") || n.contains("frc"))
}

pub fn is_game_tools(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("frc game tools")
        || n.contains("first robotics competition game tools")
        || n.starts_with("ni first robotics")
}

/// The season in a product name: "… 2026 Driver Station" is 2026, "… 2027.0.0-alpha-6" is 2027.
pub fn season(name: &str) -> u32 {
    name.split(|c: char| !c.is_ascii_digit())
        .filter(|part| part.len() == 4)
        .filter_map(|part| part.parse::<u32>().ok())
        .filter(|year| (2000..2100).contains(year))
        .max()
        .unwrap_or(0)
}

fn webview2_version(registry: &dyn Registry) -> Option<String> {
    let places = [
        (Hive::LocalMachine, format!("SOFTWARE\\WOW6432Node\\{WEBVIEW2_CLIENT}")),
        (Hive::LocalMachine, format!("SOFTWARE\\{WEBVIEW2_CLIENT}")),
        (Hive::CurrentUser, format!("Software\\{WEBVIEW2_CLIENT}")),
    ];
    places
        .iter()
        .filter_map(|(hive, path)| registry.value(*hive, path, "pv"))
        .map(|v| v.trim().to_string())
        // The runtime leaves 0.0.0.0 behind when it is removed.
        .find(|v| !v.is_empty() && v != "0.0.0.0")
}

fn describe(name: &str, version: &Option<String>) -> String {
    match version.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        // "FIRST Driver Station 2027.0.0-alpha-6" already says its version.
        // (Windows pads it to 2027.0.0.0; compare without the padding.)
        Some(v) if !name.contains(crate::version::display(v).as_str()) => format!("{name} · {v}"),
        _ => name.to_string(),
    }
}

pub fn detect_laptop(
    registry: &dyn Registry,
    items: &[LaptopItem],
    ds_paths: &[PathBuf],
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<LaptopFinding> {
    let programs = machine_programs(registry);
    items
        .iter()
        .map(|item| {
            let detail = match item.id.as_str() {
                "driver-station" => programs
                    .iter()
                    .filter(|(name, _)| is_driver_station(name))
                    // Newest season first when a laptop carries two.
                    .max_by_key(|(name, _)| season(name))
                    .map(|(name, version)| describe(name, version))
                    .or_else(|| {
                        ds_paths.iter().find(|p| exists(p)).map(|p| p.to_string_lossy().into_owned())
                    }),
                "game-tools" => programs
                    .iter()
                    .filter(|(name, _)| is_game_tools(name))
                    .max_by(|a, b| a.0.cmp(&b.0))
                    .map(|(name, version)| describe(name, version)),
                "webview2" => webview2_version(registry),
                _ => None,
            };
            LaptopFinding {
                id: item.id.clone(),
                name: item.name.clone(),
                found: detail.is_some(),
                detail,
                url: item.url.clone(),
                url_label: item.url_label.clone(),
            }
        })
        .collect()
}

/// Where the NI Driver Station has lived for years, for a laptop whose registry does not name it.
pub fn driver_station_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(root) = std::env::var(var) {
            out.push(PathBuf::from(&root).join("FRC Driver Station").join("DriverStation.exe"));
            out.push(PathBuf::from(&root).join("FIRST Driver Station").join("DriverStation.exe"));
        }
    }
    out
}

// ---------------------------------------------------------------- the real registry

#[cfg(windows)]
pub struct WindowsRegistry;

#[cfg(windows)]
impl Registry for WindowsRegistry {
    fn value(&self, hive: Hive, path: &str, name: &str) -> Option<String> {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
        let root = winreg::RegKey::predef(match hive {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        });
        root.open_subkey_with_flags(path, KEY_READ).ok()?.get_value::<String, _>(name).ok()
    }

    fn subkeys(&self, hive: Hive, path: &str) -> Vec<String> {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
        let root = winreg::RegKey::predef(match hive {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        });
        match root.open_subkey_with_flags(path, KEY_READ) {
            Ok(key) => key.enum_keys().filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A registry made of a map. Paths compare without case, as the real one does.
    #[derive(Default)]
    pub struct FakeRegistry {
        values: HashMap<(Hive, String, String), String>,
    }

    impl FakeRegistry {
        pub fn set(&mut self, hive: Hive, path: &str, name: &str, value: &str) -> &mut Self {
            self.values.insert((hive, path.to_ascii_lowercase(), name.to_ascii_lowercase()), value.to_string());
            self
        }
    }

    impl Registry for FakeRegistry {
        fn value(&self, hive: Hive, path: &str, name: &str) -> Option<String> {
            self.values.get(&(hive, path.to_ascii_lowercase(), name.to_ascii_lowercase())).cloned()
        }

        fn subkeys(&self, hive: Hive, path: &str) -> Vec<String> {
            let prefix = format!("{}\\", path.to_ascii_lowercase());
            let mut keys: Vec<String> = self
                .values
                .keys()
                .filter(|(h, p, _)| *h == hive && p.starts_with(&prefix))
                .map(|(_, p, _)| p[prefix.len()..].split('\\').next().unwrap().to_string())
                .collect();
            keys.sort();
            keys.dedup();
            keys
        }
    }

    const ROOT: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";

    fn detect(key: &str, exe: &str) -> Detect {
        Detect { hive: "HKCU".into(), key: key.into(), exe: exe.into() }
    }

    fn on_disk(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| files.iter().any(|f| Path::new(f) == p)
    }

    #[test]
    fn tauri_writes_the_install_location_in_quotes() {
        let mut reg = FakeRegistry::default();
        let key = format!("{ROOT}\\Catalyst");
        reg.set(Hive::CurrentUser, &key, "InstallLocation", "\"C:\\Users\\d\\AppData\\Local\\Catalyst\"")
            .set(Hive::CurrentUser, &key, "DisplayVersion", "2.8.0");
        let exists = on_disk(&["C:\\Users\\d\\AppData\\Local\\Catalyst\\catalyst-app.exe"]);
        let found = detect_app(&reg, ROOT, &detect("Catalyst", "catalyst-app.exe"), &exists).unwrap();
        assert_eq!(found.version.as_deref(), Some("2.8.0"));
        assert_eq!(found.location, "C:\\Users\\d\\AppData\\Local\\Catalyst");
        assert_eq!(found.exe, "C:\\Users\\d\\AppData\\Local\\Catalyst\\catalyst-app.exe");
    }

    #[test]
    fn the_sim_writes_it_bare() {
        let mut reg = FakeRegistry::default();
        let key = format!("{ROOT}\\CatalystSimMO");
        reg.set(Hive::CurrentUser, &key, "InstallLocation", "C:\\Users\\d\\AppData\\Local\\Programs\\Catalyst Sim (MO)")
            .set(Hive::CurrentUser, &key, "DisplayVersion", "1.4.0");
        let exists = on_disk(&["C:\\Users\\d\\AppData\\Local\\Programs\\Catalyst Sim (MO)\\CatalystSim.exe"]);
        let found = detect_app(&reg, ROOT, &detect("CatalystSimMO", "CatalystSim.exe"), &exists).unwrap();
        assert_eq!(found.version.as_deref(), Some("1.4.0"));
        assert!(found.exe.ends_with("Catalyst Sim (MO)\\CatalystSim.exe"));
    }

    #[test]
    fn no_key_means_not_installed() {
        let reg = FakeRegistry::default();
        assert!(detect_app(&reg, ROOT, &detect("Catalyst Pit", "catalyst-pit.exe"), &|_| true).is_none());
    }

    #[test]
    fn a_key_left_behind_without_its_exe_is_not_installed() {
        let mut reg = FakeRegistry::default();
        let key = format!("{ROOT}\\Catalyst Console");
        reg.set(Hive::CurrentUser, &key, "InstallLocation", "\"C:\\gone\"")
            .set(Hive::CurrentUser, &key, "DisplayVersion", "2.0.0");
        assert!(detect_app(&reg, ROOT, &detect("Catalyst Console", "catalyst-console.exe"), &|_| false).is_none());
    }

    #[test]
    fn a_renamed_binary_is_found_through_the_name_tauri_recorded() {
        let mut reg = FakeRegistry::default();
        let key = format!("{ROOT}\\Catalyst Pit");
        reg.set(Hive::CurrentUser, &key, "InstallLocation", "\"C:\\apps\\Catalyst Pit\"")
            .set(Hive::CurrentUser, &key, "MainBinaryName", "pit.exe");
        let exists = on_disk(&["C:\\apps\\Catalyst Pit\\pit.exe"]);
        let found = detect_app(&reg, ROOT, &detect("Catalyst Pit", "catalyst-pit.exe"), &exists).unwrap();
        assert_eq!(found.exe, "C:\\apps\\Catalyst Pit\\pit.exe");
        assert_eq!(found.version, None);
    }

    #[test]
    fn the_wrong_hive_is_not_read() {
        let mut reg = FakeRegistry::default();
        let key = format!("{ROOT}\\Catalyst");
        reg.set(Hive::LocalMachine, &key, "InstallLocation", "C:\\x");
        assert!(detect_app(&reg, ROOT, &detect("Catalyst", "catalyst-app.exe"), &|_| true).is_none());
    }

    fn laptop_items() -> Vec<LaptopItem> {
        crate::suite::load().laptop
    }

    #[test]
    fn an_empty_laptop_finds_nothing() {
        let reg = FakeRegistry::default();
        let found = detect_laptop(&reg, &laptop_items(), &[], &|_| false);
        assert_eq!(found.len(), 3);
        assert!(found.iter().all(|f| !f.found && f.detail.is_none() && f.url.starts_with("https://")));
    }

    #[test]
    fn finds_the_ni_tools_the_way_this_machine_lists_them() {
        let mut reg = FakeRegistry::default();
        let wow = UNINSTALL_ROOTS[1];
        reg.set(Hive::LocalMachine, &format!("{wow}\\{{7840E449}}"), "DisplayName", "NI FIRST Robotics Competition 2026 Driver Station")
            .set(Hive::LocalMachine, &format!("{wow}\\{{7840E449}}"), "DisplayVersion", "26.00.49166")
            .set(Hive::LocalMachine, &format!("{wow}\\{{6D63386A}}"), "DisplayName", "NI FIRST Robotics Utilities")
            .set(Hive::LocalMachine, &format!("{wow}\\{{0000}}"), "DisplayName", "NI Package Manager")
            .set(Hive::LocalMachine, &format!("SOFTWARE\\WOW6432Node\\{WEBVIEW2_CLIENT}"), "pv", "154.0.4258.53");
        let found = detect_laptop(&reg, &laptop_items(), &[], &|_| false);
        let by = |id: &str| found.iter().find(|f| f.id == id).unwrap();
        assert_eq!(by("driver-station").detail.as_deref(), Some("NI FIRST Robotics Competition 2026 Driver Station · 26.00.49166"));
        assert!(by("game-tools").found);
        assert_eq!(by("webview2").detail.as_deref(), Some("154.0.4258.53"));
    }

    #[test]
    fn the_2027_driver_station_counts_and_package_manager_alone_does_not() {
        let mut reg = FakeRegistry::default();
        let root = UNINSTALL_ROOTS[0];
        reg.set(Hive::LocalMachine, &format!("{root}\\{{CD84}}_is1"), "DisplayName", "FIRST Driver Station 2027.0.0-alpha-6")
            .set(Hive::LocalMachine, &format!("{root}\\{{CD84}}_is1"), "DisplayVersion", "2027.0.0.0")
            .set(Hive::LocalMachine, &format!("{root}\\NI Package Manager"), "DisplayName", "NI Package Manager");
        let found = detect_laptop(&reg, &laptop_items(), &[], &|_| false);
        let by = |id: &str| found.iter().find(|f| f.id == id).unwrap();
        assert_eq!(by("driver-station").detail.as_deref(), Some("FIRST Driver Station 2027.0.0-alpha-6"));
        assert!(!by("game-tools").found);
        assert!(!by("webview2").found);
    }

    #[test]
    fn two_driver_stations_show_the_newer_season() {
        let mut reg = FakeRegistry::default();
        reg.set(Hive::LocalMachine, &format!("{}\\ni", UNINSTALL_ROOTS[1]), "DisplayName", "NI FIRST Robotics Competition 2026 Driver Station")
            .set(Hive::LocalMachine, &format!("{}\\first", UNINSTALL_ROOTS[0]), "DisplayName", "FIRST Driver Station 2027.0.0-alpha-6");
        let found = detect_laptop(&reg, &laptop_items(), &[], &|_| false);
        assert_eq!(found[0].detail.as_deref(), Some("FIRST Driver Station 2027.0.0-alpha-6"));
        assert_eq!(season("NI FIRST Robotics Utilities"), 0);
    }

    #[test]
    fn a_driver_station_on_disk_counts_without_a_registry_entry() {
        let reg = FakeRegistry::default();
        let path = PathBuf::from("C:\\Program Files (x86)\\FRC Driver Station\\DriverStation.exe");
        let found = detect_laptop(&reg, &laptop_items(), &[path.clone()], &|p| p == path);
        assert!(found[0].found);
    }

    #[test]
    fn a_removed_webview2_leaves_zeros_and_is_not_found() {
        let mut reg = FakeRegistry::default();
        reg.set(Hive::LocalMachine, &format!("SOFTWARE\\WOW6432Node\\{WEBVIEW2_CLIENT}"), "pv", "0.0.0.0");
        let found = detect_laptop(&reg, &laptop_items(), &[], &|_| false);
        assert!(!found.iter().find(|f| f.id == "webview2").unwrap().found);
    }
}
