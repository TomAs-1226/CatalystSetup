//! Installing one app: get its installer (payload or download), check it, run it, and check what
//! it left behind. Every step reports what is true at that moment and nothing else.
//!
//! One job never stops the next. A failure is a sentence about that app.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::detect::Installed;
use crate::github::{self, Offer};
use crate::manifest;
use crate::version;

#[derive(Debug, Clone)]
pub enum Origin {
    Payload { path: PathBuf, sha256: String, size: u64 },
    Download { offer: Offer },
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub version: String,
    pub origin: Origin,
    pub installer_kind: String,
    pub silent_args: Vec<String>,
    /// The app's own exe, to refuse while it is open.
    pub exe_name: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    /// verifying · downloading · installing · done · failed
    pub phase: &'static str,
    pub received: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
    pub version: Option<String>,
    /// The installer was not run: this was a dry run.
    pub dry: bool,
}

/// Everything a job needs from the machine, so a test can stand in for the machine.
pub struct Machine<'a> {
    pub dry_run: bool,
    pub download_dir: PathBuf,
    pub is_running: &'a dyn Fn(&str) -> bool,
    pub detect: &'a dyn Fn(&str) -> Option<Installed>,
    pub agent: &'a dyn Fn() -> Result<ureq::Agent, String>,
}

/// Bytes arrive thousands of times a second; the window needs them a dozen times a second.
struct Throttle {
    last: Instant,
}

impl Throttle {
    fn new() -> Self {
        Self { last: Instant::now() - Duration::from_secs(1) }
    }
    fn ready(&mut self, done: bool) -> bool {
        if done || self.last.elapsed() >= Duration::from_millis(80) {
            self.last = Instant::now();
            true
        } else {
            false
        }
    }
}

pub fn run_job(job: &Job, machine: &Machine, emit: &dyn Fn(Progress)) {
    let step = |phase: &'static str, received: u64, total: Option<u64>| Progress {
        id: job.id.clone(),
        phase,
        received,
        total,
        message: None,
        version: Some(job.version.clone()),
        dry: machine.dry_run,
    };
    let fail = |message: String| {
        emit(Progress { message: Some(message), ..step("failed", 0, None) });
    };

    // An installer that replaces a running program either fails halfway or closes the program
    // under the person using it. Neither is ours to choose for them.
    if (machine.is_running)(&job.exe_name) {
        return fail(format!("{} is open. Close it, then run setup again.", job.name));
    }

    let installer = match &job.origin {
        Origin::Payload { path, sha256, size } => {
            emit(step("verifying", 0, Some(*size)));
            let mut throttle = Throttle::new();
            let checked = manifest::verify(path, sha256, |n| {
                if throttle.ready(n == *size) {
                    emit(step("verifying", n, Some(*size)));
                }
            });
            if let Err(problem) = checked {
                return fail(problem);
            }
            path.clone()
        }
        Origin::Download { offer } => match fetch(offer, machine, &step, emit) {
            Ok(path) => path,
            Err(problem) => return fail(problem),
        },
    };

    emit(step("installing", 0, None));
    if machine.dry_run {
        let command = format!("{} {}", installer.display(), job.silent_args.join(" "));
        emit(Progress { message: Some(format!("Dry run. Would have run: {command}")), ..step("done", 0, None) });
        return;
    }

    match run_installer(&installer, &job.silent_args) {
        Ok(0) => {}
        Ok(code) => return fail(exit_words(&job.installer_kind, code)),
        Err(problem) => return fail(problem),
    }

    // The installer said yes. Believe the Apps list, not the installer.
    match (machine.detect)(&job.id) {
        Some(found) => match found.version.as_deref() {
            Some(have) if version::compare(have, &job.version) != std::cmp::Ordering::Equal => fail(format!(
                "The installer finished, but {} still reads {} and not {}.",
                job.name,
                version::display(have),
                version::display(&job.version)
            )),
            _ => emit(step("done", 0, None)),
        },
        None => fail(format!("The installer finished without an error, but {} is not on this laptop.", job.name)),
    }
}

fn fetch(
    offer: &Offer,
    machine: &Machine,
    step: &dyn Fn(&'static str, u64, Option<u64>) -> Progress,
    emit: &dyn Fn(Progress),
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&machine.download_dir)
        .map_err(|e| format!("Could not make {}: {e}", machine.download_dir.display()))?;
    let dest = machine.download_dir.join(&offer.file);

    // A file from an earlier run is reused only when GitHub published a hash to hold it to.
    if let (Some(expected), Ok(meta)) = (&offer.sha256, std::fs::metadata(&dest)) {
        emit(step("verifying", 0, Some(meta.len())));
        if manifest::verify(&dest, expected, |_| {}).is_ok() {
            return Ok(dest);
        }
    }

    emit(step("downloading", 0, offer.size));
    let agent = (machine.agent)()?;
    let mut throttle = Throttle::new();
    github::download(&agent, offer, &dest, |received, total| {
        if throttle.ready(Some(received) == total) {
            emit(step("downloading", received, total));
        }
    })?;
    Ok(dest)
}

/// Run an installer and wait for it. Never elevates: an installer that wants an administrator is
/// reported, not satisfied.
pub fn run_installer(path: &Path, args: &[String]) -> Result<i32, String> {
    let status = Command::new(path).args(args).status().map_err(|e| {
        if e.raw_os_error() == Some(740) {
            "This installer asks for an administrator, and Catalyst Setup never does.".to_string()
        } else {
            format!("The installer could not be started: {e}")
        }
    })?;
    status.code().ok_or_else(|| "The installer was stopped before it finished.".to_string())
}

pub fn exit_words(kind: &str, code: i32) -> String {
    match (kind, code) {
        ("tauri-nsis", 1) => "The installer was cancelled (exit code 1).".into(),
        ("tauri-nsis", 2) => "The installer stopped itself (exit code 2). Nothing else is known.".into(),
        ("catalyst-sim", _) => match sim_log_reason() {
            Some(reason) => format!("The Sim's installer failed: {reason}"),
            None => format!("The Sim's installer failed (exit code {code}). Its log is %TEMP%\\CatalystSimSetup.log."),
        },
        _ => format!("The installer failed (exit code {code})."),
    }
}

/// The Sim's silent installer says why only in its log. The last `failed:` line is the reason.
fn sim_log_reason() -> Option<String> {
    let text = std::fs::read_to_string(std::env::temp_dir().join("CatalystSimSetup.log")).ok()?;
    last_failure(&text)
}

pub fn last_failure(log: &str) -> Option<String> {
    let line = log.lines().rev().find(|l| l.contains("failed: "))?;
    let reason = line.split_once("failed: ")?.1.trim();
    // "System.InvalidOperationException: Catalyst Sim (MO) is running. Close it and try again."
    let reason = match reason.split_once("Exception: ") {
        Some((_, words)) => words,
        None => reason,
    };
    (!reason.is_empty()).then(|| reason.to_string())
}

/// Is a program with this exe name running? `tasklist` is on every Windows and needs no rights.
#[cfg(windows)]
pub fn is_running(exe_name: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {exe_name}"), "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|out| tasklist_names(&String::from_utf8_lossy(&out.stdout), exe_name))
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn is_running(_exe_name: &str) -> bool {
    false
}

/// `"catalyst-app.exe","1234","Console","1","52,112 K"` is a hit; "INFO: No tasks…" is not.
pub fn tasklist_names(output: &str, exe_name: &str) -> bool {
    let wanted = format!("\"{}\"", exe_name.to_ascii_lowercase());
    output.lines().any(|line| line.trim().to_ascii_lowercase().starts_with(&wanted))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("catalyst-setup-test-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// A stand-in installer: a copy of cmd.exe, so "installing" is whatever its arguments say.
    fn stub(dir: &Path) -> (PathBuf, String, u64) {
        let comspec = std::env::var("ComSpec").unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".into());
        let path = dir.join("Stub_1.0.0_x64-setup.exe");
        std::fs::copy(comspec, &path).unwrap();
        let sha = manifest::sha256_file(&path, |_| {}).unwrap();
        let size = std::fs::metadata(&path).unwrap().len();
        (path, sha, size)
    }

    fn job(path: &Path, sha: &str, size: u64, args: &[&str]) -> Job {
        Job {
            id: "stub".into(),
            name: "Stub".into(),
            version: "1.0.0".into(),
            origin: Origin::Payload { path: path.to_path_buf(), sha256: sha.into(), size },
            installer_kind: "tauri-nsis".into(),
            silent_args: args.iter().map(|s| s.to_string()).collect(),
            exe_name: "stub-app.exe".into(),
        }
    }

    fn run(job: &Job, dry_run: bool, running: bool, installed: Option<&str>) -> Vec<Progress> {
        let events = RefCell::new(Vec::new());
        let detect = |_: &str| {
            installed.map(|v| Installed { version: Some(v.into()), location: "C:\\x".into(), exe: "C:\\x\\stub-app.exe".into() })
        };
        let machine = Machine {
            dry_run,
            download_dir: scratch("downloads"),
            is_running: &|_| running,
            detect: &detect,
            agent: &|| Err("no network in tests".into()),
        };
        run_job(job, &machine, &|p| events.borrow_mut().push(p));
        events.into_inner()
    }

    fn phases(events: &[Progress]) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = events.iter().map(|e| e.phase).collect();
        out.dedup();
        out
    }

    #[test]
    fn a_good_payload_is_verified_installed_and_confirmed() {
        let dir = scratch("good");
        let (path, sha, size) = stub(&dir);
        let events = run(&job(&path, &sha, size, &["/C", "exit", "0"]), false, false, Some("1.0.0"));
        assert_eq!(phases(&events), ["verifying", "installing", "done"]);
        let last_verify = events.iter().filter(|e| e.phase == "verifying").last().unwrap();
        assert_eq!((last_verify.received, last_verify.total), (size, Some(size)), "the last verify event is the whole file");
        assert!(!events.last().unwrap().dry);
    }

    #[test]
    fn a_dry_run_checks_the_file_but_never_runs_it() {
        let dir = scratch("dry");
        let (path, sha, size) = stub(&dir);
        let marker = dir.join("ran.txt");
        let _ = std::fs::remove_file(&marker);
        // No spaces, so nothing is quoted on the way to cmd: `cmd /C echo ran>C:\...\ran.txt`.
        let touch = format!("ran>{}", marker.display());
        assert!(!touch.contains(' '), "the temp folder has a space in it; this test needs one without");
        let events = run(&job(&path, &sha, size, &["/C", "echo", &touch]), true, false, None);
        assert_eq!(phases(&events), ["verifying", "installing", "done"]);
        let done = events.last().unwrap();
        assert!(done.dry && done.message.as_deref().unwrap().starts_with("Dry run. Would have run:"));
        assert!(!marker.exists(), "a dry run must not start the installer");

        // The same job for real does run it, which proves the marker would have appeared.
        let events = run(&job(&path, &sha, size, &["/C", "echo", &touch]), false, false, Some("1.0.0"));
        assert_eq!(events.last().unwrap().phase, "done");
        assert!(marker.exists());
    }

    #[test]
    fn a_damaged_payload_never_reaches_the_installer() {
        let dir = scratch("damaged");
        let (path, _, size) = stub(&dir);
        let events = run(&job(&path, &"0".repeat(64), size, &["/C", "exit", "0"]), false, false, Some("1.0.0"));
        assert_eq!(phases(&events), ["verifying", "failed"]);
        assert!(events.last().unwrap().message.as_deref().unwrap().contains("damaged"));
    }

    #[test]
    fn a_failing_installer_reports_its_exit_code() {
        let dir = scratch("exit");
        let (path, sha, size) = stub(&dir);
        let events = run(&job(&path, &sha, size, &["/C", "exit", "7"]), false, false, None);
        assert_eq!(events.last().unwrap().phase, "failed");
        assert_eq!(events.last().unwrap().message.as_deref(), Some("The installer failed (exit code 7)."));
    }

    #[test]
    fn an_open_app_is_left_alone() {
        let dir = scratch("open");
        let (path, sha, size) = stub(&dir);
        let events = run(&job(&path, &sha, size, &["/C", "exit", "0"]), false, true, Some("0.9.0"));
        assert_eq!(phases(&events), ["failed"]);
        assert_eq!(events[0].message.as_deref(), Some("Stub is open. Close it, then run setup again."));
    }

    #[test]
    fn success_is_what_the_laptop_says_not_what_the_installer_says() {
        let dir = scratch("confirm");
        let (path, sha, size) = stub(&dir);
        let ok = job(&path, &sha, size, &["/C", "exit", "0"]);
        let nothing = run(&ok, false, false, None);
        assert!(nothing.last().unwrap().message.as_deref().unwrap().contains("is not on this laptop"));
        let stale = run(&ok, false, false, Some("0.9.0"));
        assert!(stale.last().unwrap().message.as_deref().unwrap().contains("still reads 0.9.0 and not 1.0.0"));
    }

    #[test]
    fn a_download_with_no_network_fails_plainly() {
        let offer = Offer { version: "1.0.0".into(), url: "https://github.com/x".into(), file: "never.exe".into(), size: Some(10), sha256: None };
        let mut j = job(Path::new("unused"), "", 0, &["/S"]);
        j.origin = Origin::Download { offer };
        let events = run(&j, false, false, None);
        assert_eq!(phases(&events), ["downloading", "failed"]);
        assert_eq!(events.last().unwrap().message.as_deref(), Some("no network in tests"));
    }

    #[test]
    fn a_missing_installer_cannot_be_started() {
        assert!(run_installer(Path::new("C:\\no\\such\\setup.exe"), &[]).unwrap_err().contains("could not be started"));
    }

    #[test]
    fn exit_codes_have_words() {
        assert!(exit_words("tauri-nsis", 1).contains("cancelled"));
        assert!(exit_words("tauri-nsis", 2).contains("stopped itself"));
        assert_eq!(exit_words("other", 9), "The installer failed (exit code 9).");
    }

    #[test]
    fn the_sims_log_gives_the_reason() {
        let log = "2026-10-05T14:00:00  installed to C:\\x\n2026-10-05T14:02:11  failed: System.InvalidOperationException: Catalyst Sim (MO) is running. Close it and try again.\n   at CatalystSimSetup.Work.Install\n";
        assert_eq!(last_failure(log).as_deref(), Some("Catalyst Sim (MO) is running. Close it and try again."));
        assert_eq!(last_failure("2026  installed to C:\\x\n"), None);
    }

    #[test]
    fn tasklist_output_is_read_by_image_name() {
        let hit = "\"catalyst-app.exe\",\"1234\",\"Console\",\"1\",\"52,112 K\"\r\n";
        assert!(tasklist_names(hit, "Catalyst-App.exe"));
        assert!(!tasklist_names("INFO: No tasks are running which match the specified criteria.\r\n", "catalyst-app.exe"));
        assert!(!tasklist_names(hit, "catalyst-console.exe"));
    }

    #[test]
    fn the_real_tasklist_sees_a_running_program_and_not_an_imaginary_one() {
        assert!(is_running("tasklist.exe") || is_running("cargo.exe") || is_running("explorer.exe"));
        assert!(!is_running("no-such-program-zzz.exe"));
    }
}
