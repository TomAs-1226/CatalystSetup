//! `catalyst-setup.exe --auto`: the headless run a scheduled task makes to keep the apps current.
//!
//! What it may do is deliberately narrow, because nobody is watching it:
//!
//! - It updates only apps that are ALREADY installed. It never installs something new.
//! - It moves only to a strictly newer release, and only one GitHub publishes a sha256 for.
//!   A release without a digest is left for the window, where a person presses the button.
//! - It never closes a running app, and it does nothing at all while the FRC Driver Station is
//!   open: a robot laptop must not change under the drive team.
//! - Offline, or out of GitHub's hourly allowance, it stops quietly and tries again next time.
//!
//! The decisions are in `decide` and `run`, which touch nothing; `run_real` hands them the machine.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::detect::Installed;
use crate::github::{Offer, Problem};
use crate::suite::{App, Source};
use crate::version;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    /// Not on this laptop. Auto never installs something new.
    NotInstalled,
    /// It has no release to ask about (the Sim, Link): it updates from a stick, by hand.
    NoDownload,
    /// Installed, but its entry carries no version to compare.
    NoVersion,
    Offline,
    RateLimited,
    /// GitHub answered, but not with an installer (no release yet, a foreign tag).
    NoRelease(String),
    UpToDate,
    /// Newer, but GitHub publishes no sha256 for it, so nothing can vouch for the download.
    NoDigest,
    /// Newer, but the app is open. Next run.
    InUse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Update { from: String, to: String },
    Skip(Skip),
}

/// The whole decision for one app. `offer` is `None` when GitHub was not asked.
pub fn decide(
    installed: Option<&Installed>,
    downloadable: bool,
    offer: Option<&Result<Offer, Problem>>,
    running: bool,
) -> Decision {
    let Some(installed) = installed else { return Decision::Skip(Skip::NotInstalled) };
    if !downloadable {
        return Decision::Skip(Skip::NoDownload);
    }
    let Some(have) = installed.version.as_deref() else { return Decision::Skip(Skip::NoVersion) };
    let offer = match offer {
        None | Some(Err(Problem::Offline)) => return Decision::Skip(Skip::Offline),
        Some(Err(Problem::RateLimited { .. })) => return Decision::Skip(Skip::RateLimited),
        Some(Err(Problem::Other(words))) => return Decision::Skip(Skip::NoRelease(words.clone())),
        Some(Ok(offer)) => offer,
    };
    if version::compare(&offer.version, have) != Ordering::Greater {
        return Decision::Skip(Skip::UpToDate);
    }
    if offer.sha256.is_none() {
        return Decision::Skip(Skip::NoDigest);
    }
    if running {
        return Decision::Skip(Skip::InUse);
    }
    Decision::Update { from: version::display(have), to: version::display(&offer.version) }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub name: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub name: String,
    pub message: String,
}

/// What one run did. Written beside the log and shown in the window as "Last checked …".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Seconds since 1970, UTC.
    pub time: u64,
    /// checked · driver-station · offline · rate-limited
    pub outcome: String,
    /// A dry run: downloads were checked, no installer ran, nothing was swapped.
    pub dry: bool,
    pub updated: Vec<Change>,
    pub failed: Vec<Note>,
    /// Newer, and deliberately left: open, or without a digest.
    pub waiting: Vec<Note>,
    /// Catalyst Setup's own update, when it made one.
    pub setup: Option<Change>,
}

/// Everything a run needs from the machine, so a test can be the machine.
pub struct World<'a> {
    pub apps: &'a [App],
    pub dry_run: bool,
    pub now: u64,
    pub ds_running: &'a dyn Fn() -> bool,
    pub detect: &'a dyn Fn(&App) -> Option<Installed>,
    pub is_running: &'a dyn Fn(&str) -> bool,
    pub offer: &'a dyn Fn(&App) -> Result<Offer, Problem>,
    /// Download, verify, run the silent installer, confirm the version. `Err` says what happened.
    pub install: &'a dyn Fn(&App, &Offer) -> Result<(), String>,
    pub update_self: &'a dyn Fn() -> Result<Option<Change>, String>,
    pub log: &'a dyn Fn(String),
}

pub fn run(world: &World) -> Report {
    let mut report = Report { time: world.now, outcome: "checked".into(), dry: world.dry_run, ..Default::default() };
    let log = world.log;

    if (world.ds_running)() {
        log("The FRC Driver Station is open. Nothing was checked; it will be tried again next run.".into());
        report.outcome = "driver-station".into();
        return report;
    }

    for app in world.apps {
        let installed = (world.detect)(app);
        let downloadable = matches!(app.source, Source::Github { .. });
        // Ask GitHub only about what could be updated: the hourly allowance is shared by the pit.
        let offer = match decide(installed.as_ref(), downloadable, None, false) {
            Decision::Skip(Skip::Offline) => Some((world.offer)(app)),
            _ => None,
        };
        let running = offer.as_ref().is_some_and(|o| o.is_ok()) && (world.is_running)(&app.detect.exe);
        match decide(installed.as_ref(), downloadable, offer.as_ref(), running) {
            Decision::Skip(Skip::NotInstalled) => log(format!("{}: not installed, left alone.", app.name)),
            Decision::Skip(Skip::NoDownload) => log(format!("{}: has no download; it updates from a bundle, by hand.", app.name)),
            Decision::Skip(Skip::NoVersion) => log(format!("{}: installed, but its version is not recorded.", app.name)),
            Decision::Skip(Skip::Offline) => {
                log("GitHub could not be reached. Stopping until next run.".into());
                report.outcome = "offline".into();
                return report;
            }
            Decision::Skip(Skip::RateLimited) => {
                log("GitHub's hourly limit for this network is used up. Stopping until next run.".into());
                report.outcome = "rate-limited".into();
                return report;
            }
            Decision::Skip(Skip::NoRelease(words)) => log(format!("{}: {words}", app.name)),
            Decision::Skip(Skip::UpToDate) => {
                let have = installed.and_then(|i| i.version).unwrap_or_default();
                log(format!("{}: {} is up to date.", app.name, version::display(&have)));
            }
            Decision::Skip(Skip::NoDigest) => {
                let message = "has a newer release with no published sha256. Update it from the Catalyst Setup window.".to_string();
                log(format!("{} {message}", app.name));
                report.waiting.push(Note { name: app.name.clone(), message });
            }
            Decision::Skip(Skip::InUse) => {
                let message = "is open, so its update waits for the next run.".to_string();
                log(format!("{} {message}", app.name));
                report.waiting.push(Note { name: app.name.clone(), message });
            }
            Decision::Update { from, to } => {
                let offer = offer.and_then(Result::ok).expect("an update is only decided from an offer");
                match (world.install)(app, &offer) {
                    Ok(()) => {
                        let verb = if world.dry_run { "would update" } else { "updated" };
                        log(format!("{}: {verb} {from} to {to}.", app.name));
                        report.updated.push(Change { name: app.name.clone(), from, to });
                    }
                    Err(message) => {
                        log(format!("{}: {from} to {to} failed. {message}", app.name));
                        report.failed.push(Note { name: app.name.clone(), message });
                    }
                }
            }
        }
    }

    // Last, and only after a run that reached GitHub: Setup's own copy.
    match (world.update_self)() {
        Ok(Some(change)) => {
            let verb = if world.dry_run { "would update itself" } else { "updated itself" };
            log(format!("Catalyst Setup {verb} from {} to {}.", change.from, change.to));
            report.setup = Some(change);
        }
        Ok(None) => {}
        Err(message) => {
            log(format!("Catalyst Setup could not update itself. {message}"));
            report.failed.push(Note { name: "Catalyst Setup".into(), message });
        }
    }
    report
}

// ---------------------------------------------------------------- the log

pub const LOG_MAX: usize = 128 * 1024;
pub const LOG_KEEP: usize = 64 * 1024;

/// Keep a log small: past `max` bytes, drop the oldest whole lines until `keep` remain.
pub fn trim_log(text: &str, max: usize, keep: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - keep.min(text.len());
    while !text.is_char_boundary(start) {
        start += 1;
    }
    match text[start..].find('\n') {
        Some(newline) => &text[start + newline + 1..],
        None => &text[start..],
    }
}

/// 1790000000 -> "2026-09-21 14:13:20Z". No clock library: days-to-civil, as in Howard Hinnant's notes.
pub fn stamp(epoch: u64) -> String {
    let days = (epoch / 86_400) as i64;
    let secs = epoch % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z", secs / 3600, secs % 3600 / 60, secs % 60)
}

pub fn append_log(home: &Path, lines: &[String]) {
    let path = home.join("auto.log");
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    let _ = std::fs::write(&path, trim_log(&text, LOG_MAX, LOG_KEEP));
}

pub fn write_report(home: &Path, report: &Report) {
    if let Ok(json) = serde_json::to_string_pretty(report) {
        let _ = std::fs::write(home.join("last-run.json"), json + "\n");
    }
}

pub fn read_report(home: &Path) -> Option<Report> {
    serde_json::from_str(&std::fs::read_to_string(home.join("last-run.json")).ok()?).ok()
}

// ---------------------------------------------------------------- one at a time

/// One Catalyst Setup changes this laptop at a time: the window's install and the headless run
/// take the same lock. It is a file held open without sharing, so it cannot go stale — Windows
/// lets go of it when the process ends, however it ends.
pub struct Lock {
    _file: std::fs::File,
}

impl Lock {
    pub fn acquire(dir: &Path) -> Option<Lock> {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::create_dir_all(dir).ok()?;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(dir.join("setup.lock"))
            .ok()
            .map(|file| Lock { _file: file })
    }
}

pub fn work_dir() -> PathBuf {
    std::env::temp_dir().join("CatalystSetup")
}

// ---------------------------------------------------------------- Setup's own copy

/// Put `new` in place of `dir\name`. A running exe cannot be overwritten but can be renamed, so
/// the current one is moved aside to `*.old.exe` (replacing the one before it) and the new one is
/// copied in. If the copy fails the old one is moved back, so there is always a working exe.
pub fn swap_exe(dir: &Path, name: &str, new: &Path) -> Result<(), String> {
    let exe = dir.join(name);
    let aside = dir.join(name.replace(".exe", ".old.exe"));
    let had = exe.exists();
    if had {
        std::fs::rename(&exe, &aside).map_err(|e| format!("The installed copy could not be moved aside: {e}"))?;
    }
    if let Err(e) = std::fs::copy(new, &exe) {
        let _ = std::fs::remove_file(&exe);
        if had {
            let _ = std::fs::rename(&aside, &exe);
        }
        return Err(format!("The new copy could not be written: {e}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite;
    use std::cell::RefCell;

    fn installed(version: Option<&str>) -> Installed {
        Installed { version: version.map(str::to_string), location: "C:\\x".into(), exe: "C:\\x\\app.exe".into() }
    }

    fn offer(version: &str, digest: bool) -> Result<Offer, Problem> {
        Ok(Offer {
            version: version.into(),
            url: "https://github.com/x".into(),
            file: "App_x64-setup.exe".into(),
            size: Some(10),
            sha256: digest.then(|| "ab".repeat(32)),
        })
    }

    #[test]
    fn the_decision_table() {
        use Decision::Skip as S;
        let have = installed(Some("2.7.0"));
        let update = Decision::Update { from: "2.7.0".into(), to: "2.8.0".into() };
        // installed, newer, digest, closed: the only row that updates
        assert_eq!(decide(Some(&have), true, Some(&offer("2.8.0", true)), false), update);
        assert_eq!(decide(Some(&have), true, Some(&offer("app-v2.8.0", true)), false), update);
        // not installed: never, whatever is on offer
        assert_eq!(decide(None, true, Some(&offer("2.8.0", true)), false), S(Skip::NotInstalled));
        // payload-only: never
        assert_eq!(decide(Some(&have), false, None, false), S(Skip::NoDownload));
        // same, or older than installed: nothing
        assert_eq!(decide(Some(&have), true, Some(&offer("2.7.0", true)), false), S(Skip::UpToDate));
        assert_eq!(decide(Some(&have), true, Some(&offer("2.6.0", true)), false), S(Skip::UpToDate));
        assert_eq!(decide(Some(&installed(Some("2.8.0"))), true, Some(&offer("2.8.0-beta.1", true)), false), S(Skip::UpToDate));
        // newer but open: wait, never close it
        assert_eq!(decide(Some(&have), true, Some(&offer("2.8.0", true)), true), S(Skip::InUse));
        // newer but no digest: not without a person, open or closed
        assert_eq!(decide(Some(&have), true, Some(&offer("2.8.0", false)), false), S(Skip::NoDigest));
        assert_eq!(decide(Some(&have), true, Some(&offer("2.8.0", false)), true), S(Skip::NoDigest));
        // offline, rate limited, no release
        assert_eq!(decide(Some(&have), true, Some(&Err(Problem::Offline)), false), S(Skip::Offline));
        assert_eq!(decide(Some(&have), true, None, false), S(Skip::Offline));
        assert_eq!(decide(Some(&have), true, Some(&Err(Problem::RateLimited { reset_epoch: None })), false), S(Skip::RateLimited));
        assert_eq!(decide(Some(&have), true, Some(&Err(Problem::Other("no release".into()))), false), S(Skip::NoRelease("no release".into())));
        // installed with no recorded version: nothing to compare, so nothing is done
        assert_eq!(decide(Some(&installed(None)), true, Some(&offer("2.8.0", true)), false), S(Skip::NoVersion));
    }

    /// A made-up laptop: which apps are installed at what version, what GitHub says, what is open.
    #[derive(Default)]
    struct Laptop {
        installed: Vec<(&'static str, &'static str)>,
        offers: Vec<(&'static str, Result<Offer, Problem>)>,
        open: Vec<&'static str>,
        ds: bool,
        dry: bool,
        install_fails: Option<&'static str>,
        own: Option<Change>,
    }

    struct Seen {
        report: Report,
        asked: Vec<String>,
        installs: Vec<String>,
        own_checked: bool,
        log: Vec<String>,
    }

    fn play(laptop: Laptop) -> Seen {
        let apps = suite::load().apps;
        let asked = RefCell::new(Vec::new());
        let installs = RefCell::new(Vec::new());
        let own_checked = RefCell::new(false);
        let log = RefCell::new(Vec::new());
        let report = run(&World {
            apps: &apps,
            dry_run: laptop.dry,
            now: 1_790_000_000,
            ds_running: &|| laptop.ds,
            detect: &|app| laptop.installed.iter().find(|(id, _)| *id == app.id).map(|(_, v)| installed(Some(v))),
            is_running: &|exe| laptop.open.contains(&exe),
            offer: &|app| {
                asked.borrow_mut().push(app.id.clone());
                laptop.offers.iter().find(|(id, _)| *id == app.id).map(|(_, o)| o.clone()).unwrap_or(Err(Problem::Other("GitHub has no release for this app yet.".into())))
            },
            install: &|app, offer| {
                installs.borrow_mut().push(format!("{} {}", app.id, offer.version));
                match laptop.install_fails {
                    Some(words) => Err(words.to_string()),
                    None => Ok(()),
                }
            },
            update_self: &|| {
                *own_checked.borrow_mut() = true;
                Ok(laptop.own.clone())
            },
            log: &|line| log.borrow_mut().push(line),
        });
        Seen { report, asked: asked.into_inner(), installs: installs.into_inner(), own_checked: own_checked.into_inner(), log: log.into_inner() }
    }

    #[test]
    fn only_installed_apps_with_a_newer_release_are_updated() {
        let seen = play(Laptop {
            installed: vec![("catalyst", "2.7.0"), ("console", "2.0.0"), ("sim", "1.4.0")],
            offers: vec![("catalyst", offer("2.8.0", true)), ("console", offer("2.0.0", true)), ("pit", offer("0.2.0", true))],
            ..Default::default()
        });
        assert_eq!(seen.installs, ["catalyst 2.8.0"]);
        assert_eq!(seen.report.updated, [Change { name: "Catalyst".into(), from: "2.7.0".into(), to: "2.8.0".into() }]);
        assert_eq!(seen.report.outcome, "checked");
        // Pit is not installed and Link is optional and not installed: GitHub is not even asked.
        assert_eq!(seen.asked, ["catalyst", "console"]);
        assert!(seen.own_checked);
        assert!(seen.log.iter().any(|l| l.contains("Catalyst Sim (MO): has no download")));
        assert!(seen.log.iter().any(|l| l == "Catalyst Pit: not installed, left alone."));
    }

    #[test]
    fn the_driver_station_stops_everything_before_it_starts() {
        let seen = play(Laptop {
            installed: vec![("catalyst", "2.7.0")],
            offers: vec![("catalyst", offer("2.8.0", true))],
            ds: true,
            ..Default::default()
        });
        assert_eq!(seen.report.outcome, "driver-station");
        assert!(seen.asked.is_empty() && seen.installs.is_empty() && !seen.own_checked);
    }

    #[test]
    fn an_open_app_waits_and_the_others_carry_on() {
        let seen = play(Laptop {
            installed: vec![("catalyst", "2.7.0"), ("console", "1.4.3")],
            offers: vec![("catalyst", offer("2.8.0", true)), ("console", offer("2.0.0", true))],
            open: vec!["catalyst-app.exe"],
            ..Default::default()
        });
        assert_eq!(seen.installs, ["console 2.0.0"]);
        assert_eq!(seen.report.waiting.len(), 1);
        assert_eq!(seen.report.waiting[0].name, "Catalyst");
    }

    #[test]
    fn no_digest_means_no_unattended_install() {
        let seen = play(Laptop {
            installed: vec![("catalyst", "2.7.0")],
            offers: vec![("catalyst", offer("2.8.0", false))],
            ..Default::default()
        });
        assert!(seen.installs.is_empty());
        assert!(seen.report.waiting[0].message.contains("no published sha256"));
    }

    #[test]
    fn offline_and_rate_limited_stop_quietly() {
        for (problem, outcome) in [(Problem::Offline, "offline"), (Problem::RateLimited { reset_epoch: None }, "rate-limited")] {
            let seen = play(Laptop {
                installed: vec![("catalyst", "2.7.0"), ("console", "1.0.0")],
                offers: vec![("catalyst", Err(problem)), ("console", offer("2.0.0", true))],
                ..Default::default()
            });
            assert_eq!(seen.report.outcome, outcome);
            assert_eq!(seen.asked, ["catalyst"], "it does not keep asking once GitHub is out of reach");
            assert!(seen.installs.is_empty() && seen.report.failed.is_empty() && !seen.own_checked);
        }
    }

    #[test]
    fn a_failed_install_is_reported_and_does_not_stop_the_next() {
        let seen = play(Laptop {
            installed: vec![("catalyst", "2.7.0"), ("console", "1.0.0")],
            offers: vec![("catalyst", offer("2.8.0", true)), ("console", offer("2.0.0", true))],
            install_fails: Some("The download does not match the sha256 GitHub publishes for it."),
            ..Default::default()
        });
        assert_eq!(seen.installs.len(), 2);
        assert_eq!(seen.report.failed.len(), 2);
        assert!(seen.report.updated.is_empty());
    }

    #[test]
    fn an_app_with_no_release_yet_is_skipped_without_a_fuss() {
        let seen = play(Laptop { installed: vec![("pit", "0.1.0")], ..Default::default() });
        assert_eq!(seen.report.outcome, "checked");
        assert!(seen.report.failed.is_empty() && seen.report.waiting.is_empty());
        assert!(seen.log.iter().any(|l| l.contains("Catalyst Pit: GitHub has no release")));
    }

    #[test]
    fn setups_own_update_is_last_and_is_reported() {
        let own = Change { name: "Catalyst Setup".into(), from: "1.0.0".into(), to: "1.1.0".into() };
        let seen = play(Laptop { own: Some(own.clone()), dry: true, ..Default::default() });
        assert_eq!(seen.report.setup, Some(own));
        assert!(seen.report.dry);
        assert!(seen.log.last().unwrap().contains("would update itself from 1.0.0 to 1.1.0"));
    }

    #[test]
    fn the_log_is_kept_small_by_dropping_whole_old_lines() {
        let text: String = (0..100).map(|i| format!("line {i:03}\n")).collect();
        assert_eq!(trim_log(&text, 2000, 50), text, "under the limit it is left alone");
        let trimmed = trim_log(&text, 500, 50);
        assert!(trimmed.len() <= 50 && trimmed.starts_with("line 09") && trimmed.ends_with("line 099\n"));
        assert_eq!(trim_log("é".repeat(40).as_str(), 10, 5).chars().count(), 2);
    }

    #[test]
    fn timestamps_are_utc_dates() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00Z");
        assert_eq!(stamp(951_782_400), "2000-02-29 00:00:00Z");
        assert_eq!(stamp(1_790_000_000), "2026-09-21 14:13:20Z");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("catalyst-setup-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_log_and_the_report_round_trip_through_the_home_folder() {
        let home = scratch("auto-home");
        append_log(&home, &["one".into(), "two".into()]);
        append_log(&home, &["three".into()]);
        assert_eq!(std::fs::read_to_string(home.join("auto.log")).unwrap(), "one\ntwo\nthree\n");
        assert_eq!(read_report(&home), None);
        let report = Report { time: 5, outcome: "checked".into(), updated: vec![Change { name: "Catalyst".into(), from: "1".into(), to: "2".into() }], ..Default::default() };
        write_report(&home, &report);
        assert_eq!(read_report(&home), Some(report));
    }

    #[test]
    fn only_one_holds_the_lock_and_it_is_free_again_afterwards() {
        let dir = scratch("lock");
        let first = Lock::acquire(&dir).expect("the first one gets it");
        assert!(Lock::acquire(&dir).is_none(), "the second is refused while the first holds it");
        drop(first);
        assert!(Lock::acquire(&dir).is_some());
    }

    #[test]
    fn swapping_the_exe_keeps_the_old_one_aside() {
        let dir = scratch("swap");
        std::fs::write(dir.join("catalyst-setup.exe"), "v1").unwrap();
        let new = dir.join("download.exe");
        std::fs::write(&new, "v2").unwrap();
        swap_exe(&dir, "catalyst-setup.exe", &new).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("catalyst-setup.exe")).unwrap(), "v2");
        assert_eq!(std::fs::read_to_string(dir.join("catalyst-setup.old.exe")).unwrap(), "v1");
        // Again: the copy set aside before is replaced, not piled up.
        std::fs::write(&new, "v3").unwrap();
        swap_exe(&dir, "catalyst-setup.exe", &new).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("catalyst-setup.old.exe")).unwrap(), "v2");
    }

    #[test]
    fn a_swap_that_cannot_finish_puts_the_old_exe_back() {
        let dir = scratch("swap-back");
        std::fs::write(dir.join("catalyst-setup.exe"), "v1").unwrap();
        let err = swap_exe(&dir, "catalyst-setup.exe", &dir.join("missing.exe")).unwrap_err();
        assert!(err.contains("could not be written"));
        assert_eq!(std::fs::read_to_string(dir.join("catalyst-setup.exe")).unwrap(), "v1");
    }

    #[test]
    fn a_first_install_has_nothing_to_move_aside() {
        let dir = scratch("swap-first");
        let new = dir.join("download.exe");
        std::fs::write(&new, "v1").unwrap();
        swap_exe(&dir, "catalyst-setup.exe", &new).unwrap();
        assert!(dir.join("catalyst-setup.exe").exists() && !dir.join("catalyst-setup.old.exe").exists());
    }
}
