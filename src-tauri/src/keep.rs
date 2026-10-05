//! "Keep these apps up to date": the installed copy of Catalyst Setup, its scheduled task, its
//! entry in Windows' Apps list, and the real `--auto` run that the task starts.
//!
//! Everything here is per user. Nothing asks for an administrator: the copy lives under
//! %LOCALAPPDATA%, the task runs as the signed-in user with least privilege, and the registry
//! entry is in HKCU.
//!
//! Turning the switch on:   copy this exe to a place a USB stick cannot take away, add the Apps
//!                          entry, register the task.
//! Turning it off:          remove the task. The copy and its entry stay until it is uninstalled.
//! `--uninstall`:           remove the task, the entry and Setup's own files. Never an app.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::auto::{self, Change, Report};
use crate::detect;
use crate::github;
use crate::install::{self, Job, Machine, Origin};
use crate::suite::{App, Setup, Source, Suite};
use crate::version;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const PUBLISHER: &str = "frccatalyst";

/// Where the installed copy, its log and its settings live.
pub fn home(setup: &Setup) -> PathBuf {
    if let Some(dir) = std::env::var_os("CATALYST_SETUP_HOME") {
        return PathBuf::from(dir);
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    local.join(&setup.install_folder)
}

// ---------------------------------------------------------------- the scheduled task

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The task, as Task Scheduler's own XML: at this user's sign-in, and every few hours after.
/// It runs as that user with no elevation, on battery too (it is a laptop), one at a time.
pub fn task_xml(exe: &Path, user: &str, every_hours: u32) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    let user = xml_escape(user);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Keeps the installed Catalyst apps up to date. Turn it off in Catalyst Setup.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
      <Delay>PT2M</Delay>
    </LogonTrigger>
    <TimeTrigger>
      <Repetition>
        <Interval>PT{every_hours}H</Interval>
        <StopAtDurationEnd>false</StopAtDurationEnd>
      </Repetition>
      <StartBoundary>2026-01-01T00:30:00</StartBoundary>
      <Enabled>true</Enabled>
    </TimeTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <ExecutionTimeLimit>PT1H</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--auto</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// The plain schtasks form, for a laptop that refuses the XML: every few hours, no sign-in trigger
/// (schtasks' own ONLOGON asks for an administrator, which is why the XML is tried first).
pub fn fallback_task_args(task: &str, exe: &Path, every_hours: u32) -> Vec<String> {
    vec![
        "/Create".into(), "/F".into(),
        "/TN".into(), task.into(),
        "/SC".into(), "HOURLY".into(),
        "/MO".into(), every_hours.to_string(),
        "/TR".into(), format!("\"{}\" --auto", exe.display()),
    ]
}

fn schtasks(args: &[String]) -> Result<(), String> {
    let out = Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Task Scheduler could not be reached: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let words = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if words.is_empty() { format!("Task Scheduler refused (exit code {}).", out.status.code().unwrap_or(-1)) } else { words })
    }
}

pub fn task_registered(task: &str) -> bool {
    schtasks(&["/Query".into(), "/TN".into(), task.into()]).is_ok()
}

fn current_user() -> String {
    let name = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{name}"),
        _ => name,
    }
}

fn register_task(setup: &Setup, exe: &Path) -> Result<(), String> {
    let xml = task_xml(exe, &current_user(), setup.every_hours);
    let file = auto::work_dir().join("task.xml");
    std::fs::create_dir_all(auto::work_dir()).map_err(|e| e.to_string())?;
    // Task Scheduler reads UTF-16 with a byte-order mark, as the declaration says.
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&file, bytes).map_err(|e| format!("The task could not be written out: {e}"))?;
    let from_xml = schtasks(&[
        "/Create".into(), "/F".into(), "/TN".into(), setup.task.clone(), "/XML".into(), file.to_string_lossy().into_owned(),
    ]);
    let _ = std::fs::remove_file(&file);
    match from_xml {
        Ok(()) => Ok(()),
        Err(first) => schtasks(&fallback_task_args(&setup.task, exe, setup.every_hours))
            .map_err(|second| format!("Windows would not register the task. {first} {second}")),
    }
}

fn remove_task(setup: &Setup) -> Result<(), String> {
    if !task_registered(&setup.task) {
        return Ok(());
    }
    schtasks(&["/Delete".into(), "/F".into(), "/TN".into(), setup.task.clone()])
}

// ---------------------------------------------------------------- the Apps-list entry

/// What goes under HKCU\…\Uninstall\Catalyst Setup. `InstallLocation` is written bare (no quotes),
/// with `MainBinaryName` beside it, so the same lookup that finds the Tauri apps finds this.
pub fn uninstall_values(home: &Path, asset: &str, version: &str) -> Vec<(&'static str, String)> {
    let exe = home.join(asset);
    vec![
        ("DisplayName", "Catalyst Setup".to_string()),
        ("DisplayVersion", version.to_string()),
        ("Publisher", PUBLISHER.to_string()),
        ("InstallLocation", home.to_string_lossy().into_owned()),
        ("MainBinaryName", asset.to_string()),
        ("DisplayIcon", format!("\"{}\"", exe.display())),
        ("UninstallString", format!("\"{}\" --uninstall", exe.display())),
        ("QuietUninstallString", format!("\"{}\" --uninstall", exe.display())),
    ]
}

fn entry_path(suite: &Suite) -> String {
    format!("{}\\{}", suite.uninstall_root, suite.setup.uninstall_key)
}

fn write_entry(suite: &Suite, home: &Path, version: &str) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    let (key, _) = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(entry_path(suite))
        .map_err(|e| format!("The Apps-list entry could not be written: {e}"))?;
    for (name, value) in uninstall_values(home, &suite.setup.asset, version) {
        key.set_value(name, &value).map_err(|e| format!("The Apps-list entry could not be written: {e}"))?;
    }
    for name in ["NoModify", "NoRepair"] {
        key.set_value(name, &1u32).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn installed_version(suite: &Suite, home: &Path) -> Option<String> {
    use detect::Registry;
    if !home.join(&suite.setup.asset).is_file() {
        return None;
    }
    detect::WindowsRegistry.value(detect::Hive::CurrentUser, &entry_path(suite), "DisplayVersion")
}

// ---------------------------------------------------------------- the switch

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepState {
    /// The task exists right now. This, and nothing remembered, is what "on" means.
    pub registered: bool,
    /// The version of the copy under %LOCALAPPDATA%, when there is one.
    pub installed_version: Option<String>,
    /// What the person last chose, if they ever chose. `None` on a laptop that has not been asked.
    pub preference: Option<bool>,
    pub every_hours: u32,
    pub last_run: Option<Report>,
    /// Said once, after a change: why it did not happen (a dry run, a refusal).
    pub note: Option<String>,
}

fn read_preference(home: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(home.join("settings.json")).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()?["keepUpdated"].as_bool()
}

fn write_preference(home: &Path, on: bool) {
    if std::fs::create_dir_all(home).is_ok() {
        let _ = std::fs::write(home.join("settings.json"), format!("{{\n  \"keepUpdated\": {on}\n}}\n"));
    }
}

pub fn state(suite: &Suite) -> KeepState {
    let home = home(&suite.setup);
    KeepState {
        registered: task_registered(&suite.setup.task),
        installed_version: installed_version(suite, &home),
        preference: read_preference(&home),
        every_hours: suite.setup.every_hours,
        last_run: auto::read_report(&home),
        note: None,
    }
}

/// Should the copy under %LOCALAPPDATA% be replaced by the exe that is running? Not by an older
/// one: a stick from last month must not undo an update the laptop already made.
pub fn should_replace_copy(running: &str, installed: Option<&str>) -> bool {
    match installed {
        None => true,
        Some(have) => version::compare(running, have) != std::cmp::Ordering::Less,
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn enable(suite: &Suite) -> Result<(), String> {
    let setup = &suite.setup;
    let home = home(setup);
    std::fs::create_dir_all(&home).map_err(|e| format!("{} could not be made: {e}", home.display()))?;
    let dest = home.join(&setup.asset);
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mine = env!("CARGO_PKG_VERSION");
    let mut version = installed_version(suite, &home);
    if !same_file(&me, &dest) && (!dest.is_file() || should_replace_copy(mine, version.as_deref())) {
        auto::swap_exe(&home, &setup.asset, &me)?;
        version = Some(mine.to_string());
    }
    write_entry(suite, &home, version.as_deref().unwrap_or(mine))?;
    register_task(setup, &dest)
}

pub fn set(suite: &Suite, on: bool, dry_run: bool) -> KeepState {
    if dry_run {
        let what = if on { "nothing was copied and no task was registered" } else { "the task was not removed" };
        return KeepState { note: Some(format!("Dry run: {what}.")), ..state(suite) };
    }
    let result = if on { enable(suite) } else { remove_task(&suite.setup) };
    if result.is_ok() {
        write_preference(&home(&suite.setup), on);
    }
    KeepState { note: result.err(), ..state(suite) }
}

// ---------------------------------------------------------------- uninstall

/// The command that clears Setup's own folder once this process has gone: named files only, then
/// the folder if that left it empty. Never a wildcard, never anything of an app's.
pub fn cleanup_command(home: &Path, asset: &str) -> String {
    let files = [asset.to_string(), asset.replace(".exe", ".old.exe"), "auto.log".into(), "last-run.json".into(), "settings.json".into()];
    let dels: Vec<String> = files.iter().map(|f| format!("del /q \"{}\"", home.join(f).display())).collect();
    format!("/S /C \"ping -n 3 127.0.0.1 >nul & {} & rmdir \"{}\"\"", dels.join(" & "), home.display())
}

/// `catalyst-setup.exe --uninstall`, which is what the Apps list runs.
pub fn uninstall(suite: &Suite) -> i32 {
    use winreg::enums::HKEY_CURRENT_USER;
    let home = home(&suite.setup);
    let _ = remove_task(&suite.setup);
    let _ = winreg::RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(entry_path(suite));
    // Only when this really is the installed copy: run from a stick, there is no folder to clear.
    let installed_here = std::env::current_exe().is_ok_and(|me| same_file(&me, &home.join(&suite.setup.asset)));
    if installed_here {
        let _ = Command::new("cmd")
            .raw_arg(cleanup_command(&home, &suite.setup.asset))
            .current_dir(std::env::temp_dir())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    0
}

// ---------------------------------------------------------------- the real --auto

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn install_update(suite: &Suite, app: &App, offer: &github::Offer, dry_run: bool) -> Result<(), String> {
    let job = Job {
        id: app.id.clone(),
        name: app.name.clone(),
        version: offer.version.clone(),
        origin: Origin::Download { offer: offer.clone() },
        installer_kind: app.installer.kind.clone(),
        silent_args: app.installer.silent_args.clone(),
        exe_name: app.detect.exe.clone(),
    };
    let detect = |id: &str| suite.apps.iter().find(|a| a.id == id).and_then(|a| detect_app(suite, a));
    let machine = Machine {
        dry_run,
        download_dir: auto::work_dir(),
        is_running: &install::is_running,
        detect: &detect,
        agent: &github::agent,
    };
    let last = std::cell::RefCell::new(None);
    install::run_job(&job, &machine, &|progress| *last.borrow_mut() = Some(progress));
    match last.into_inner() {
        Some(p) if p.phase == "done" => Ok(()),
        Some(p) => Err(p.message.unwrap_or_else(|| "It did not finish.".into())),
        None => Err("It did not start.".into()),
    }
}

fn detect_app(suite: &Suite, app: &App) -> Option<detect::Installed> {
    detect::detect_app(&detect::WindowsRegistry, &suite.uninstall_root, &app.detect, &|p: &Path| p.is_file())
}

/// Setup's own update: only the installed copy updates itself, only to a strictly newer release
/// with a published digest, and by moving the running exe aside rather than writing over it.
fn update_self(suite: &Suite, home: &Path, dry_run: bool) -> Result<Option<Change>, String> {
    let setup = &suite.setup;
    let dest = home.join(&setup.asset);
    let Ok(me) = std::env::current_exe() else { return Ok(None) };
    if !same_file(&me, &dest) {
        return Ok(None);
    }
    let agent = github::agent()?;
    // No release yet, offline, rate limited: none of these is a failure of this run.
    let Ok(offer) = github::fetch_offer(&agent, &setup.repo, &setup.tag_prefix, &setup.asset) else { return Ok(None) };
    let mine = env!("CARGO_PKG_VERSION");
    if version::compare(&offer.version, mine) != std::cmp::Ordering::Greater || offer.sha256.is_none() {
        return Ok(None);
    }
    let work = auto::work_dir();
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let download = work.join(format!("catalyst-setup-{}.exe", version::display(&offer.version)));
    github::download(&agent, &offer, &download, |_, _| {})?;
    let change = Change { name: "Catalyst Setup".into(), from: mine.into(), to: version::display(&offer.version) };
    if dry_run {
        return Ok(Some(change));
    }
    auto::swap_exe(home, &setup.asset, &download)?;
    write_entry(suite, home, &change.to)?;
    Ok(Some(change))
}

/// `catalyst-setup.exe --auto [--dry-run]`. No window. Returns the process's exit code, which is
/// 0 whenever the run did what it should — including "nothing to do" and "offline".
pub fn run_auto(suite: &Suite, dry_run: bool) -> i32 {
    let home = home(&suite.setup);
    if std::fs::create_dir_all(&home).is_err() {
        return 1;
    }
    let started = now();
    let lines = std::cell::RefCell::new(vec![format!(
        "{}  catalyst-setup {} --auto{}",
        auto::stamp(started),
        env!("CARGO_PKG_VERSION"),
        if dry_run { " --dry-run" } else { "" }
    )]);
    let log = |line: String| lines.borrow_mut().push(format!("  {line}"));

    let Some(_lock) = auto::Lock::acquire(&auto::work_dir()) else {
        log("Another Catalyst Setup is installing. Nothing was checked.".into());
        auto::append_log(&home, &lines.into_inner());
        return 0;
    };

    let agent = std::cell::OnceCell::new();
    let report = auto::run(&auto::World {
        apps: &suite.apps,
        dry_run,
        now: started,
        ds_running: &|| suite.setup.driver_station_processes.iter().any(|p| install::is_running(p)),
        detect: &|app| detect_app(suite, app),
        is_running: &install::is_running,
        offer: &|app| {
            let Source::Github { repo, tag_prefix, asset_suffix } = &app.source else {
                return Err(github::Problem::Other("It has no download.".into()));
            };
            match agent.get_or_init(github::agent) {
                Ok(agent) => github::fetch_offer(agent, repo, tag_prefix, asset_suffix),
                Err(words) => Err(github::Problem::Other(words.clone())),
            }
        },
        install: &|app, offer| install_update(suite, app, offer, dry_run),
        update_self: &|| update_self(suite, &home, dry_run),
        log: &log,
    });

    auto::write_report(&home, &report);
    auto::append_log(&home, &lines.into_inner());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_runs_this_exe_headless_as_this_user_without_elevation() {
        let xml = task_xml(Path::new("C:\\Users\\d\\AppData\\Local\\Catalyst Setup\\catalyst-setup.exe"), "PIT-LAPTOP\\driver", 4);
        assert!(xml.contains("<Command>C:\\Users\\d\\AppData\\Local\\Catalyst Setup\\catalyst-setup.exe</Command>"));
        assert!(xml.contains("<Arguments>--auto</Arguments>"));
        assert_eq!(xml.matches("<UserId>PIT-LAPTOP\\driver</UserId>").count(), 2, "the sign-in trigger and the principal");
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(xml.contains("<LogonTrigger>") && xml.contains("<Interval>PT4H</Interval>"));
        // A laptop on battery in the pit still gets checked, and two runs never overlap.
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        // A laptop that was asleep at the hour catches up when it wakes.
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(!xml.contains("HighestAvailable"));
    }

    #[test]
    fn names_with_markup_in_them_cannot_break_the_task() {
        let xml = task_xml(Path::new("C:\\Users\\R&D <team>\\catalyst-setup.exe"), "LAB\\R&D", 6);
        assert!(xml.contains("C:\\Users\\R&amp;D &lt;team&gt;\\catalyst-setup.exe"));
        assert!(xml.contains("<UserId>LAB\\R&amp;D</UserId>"));
        assert!(xml.contains("PT6H"));
    }

    #[test]
    fn the_fallback_is_the_plain_hourly_form() {
        let args = fallback_task_args("Catalyst Setup Auto Update", Path::new("C:\\a b\\catalyst-setup.exe"), 4);
        assert_eq!(args, ["/Create", "/F", "/TN", "Catalyst Setup Auto Update", "/SC", "HOURLY", "/MO", "4", "/TR", "\"C:\\a b\\catalyst-setup.exe\" --auto"]);
    }

    #[test]
    fn the_apps_list_entry_can_be_found_the_way_the_other_apps_are() {
        let values = uninstall_values(Path::new("C:\\Users\\d\\AppData\\Local\\Catalyst Setup"), "catalyst-setup.exe", "1.0.0");
        let get = |name: &str| values.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str()).unwrap();
        assert_eq!(get("DisplayName"), "Catalyst Setup");
        assert_eq!(get("DisplayVersion"), "1.0.0");
        assert_eq!(get("InstallLocation"), "C:\\Users\\d\\AppData\\Local\\Catalyst Setup");
        assert_eq!(get("MainBinaryName"), "catalyst-setup.exe");
        assert_eq!(get("UninstallString"), "\"C:\\Users\\d\\AppData\\Local\\Catalyst Setup\\catalyst-setup.exe\" --uninstall");

        // The same detection the suite uses reads it back.
        let mut reg = detect::tests::FakeRegistry::default();
        let key = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Catalyst Setup";
        for (name, value) in &values {
            reg.set(detect::Hive::CurrentUser, key, name, value);
        }
        let probe = crate::suite::Detect { hive: "HKCU".into(), key: "Catalyst Setup".into(), exe: "catalyst-setup.exe".into() };
        let found = detect::detect_app(&reg, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall", &probe, &|_| true).unwrap();
        assert_eq!(found.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn an_older_stick_does_not_replace_a_newer_installed_copy() {
        assert!(should_replace_copy("1.0.0", None));
        assert!(should_replace_copy("1.1.0", Some("1.0.0")));
        assert!(should_replace_copy("1.0.0", Some("1.0.0")));
        assert!(!should_replace_copy("1.0.0", Some("1.1.0")));
    }

    #[test]
    fn uninstall_clears_named_files_only() {
        let command = cleanup_command(Path::new("C:\\Users\\d\\AppData\\Local\\Catalyst Setup"), "catalyst-setup.exe");
        assert!(command.starts_with("/S /C \""));
        for file in ["catalyst-setup.exe", "catalyst-setup.old.exe", "auto.log", "last-run.json", "settings.json"] {
            assert!(command.contains(&format!("del /q \"C:\\Users\\d\\AppData\\Local\\Catalyst Setup\\{file}\"")), "{file}");
        }
        assert!(command.ends_with("rmdir \"C:\\Users\\d\\AppData\\Local\\Catalyst Setup\"\""));
        assert!(!command.contains('*') && !command.contains("/s ") && !command.contains("rmdir /"), "no wildcards, nothing recursive");
    }

    #[test]
    fn the_preference_is_remembered_in_the_home_folder() {
        let home = std::env::temp_dir().join(format!("catalyst-setup-test-{}-pref", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(read_preference(&home), None, "a laptop that was never asked has no preference");
        write_preference(&home, false);
        assert_eq!(read_preference(&home), Some(false));
        write_preference(&home, true);
        assert_eq!(read_preference(&home), Some(true));
    }

    #[test]
    fn a_task_that_was_never_registered_reads_as_off() {
        // Read-only: asks Task Scheduler about a name nothing uses.
        assert!(!task_registered("Catalyst Setup Test - No Such Task"));
    }
}
