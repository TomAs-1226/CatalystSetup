// Catalyst Setup: one portable exe that puts every Catalyst app on a driver-station laptop.
//
// It never asks for an administrator, never deletes anything, and never installs anything that
// is not in suite.json. The window only ever shows what one of the modules below measured.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod detect;
mod github;
mod install;
mod manifest;
mod suite;
mod version;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as Atomic};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use detect::{Installed, Registry};
use github::{Offer, Problem};
use install::{Job, Machine, Origin};
use suite::{App, Source, Suite};

struct Ctx {
    suite: Suite,
    payload_dir: PathBuf,
    dry_run: bool,
    /// What GitHub said, per app id, the last time it was asked.
    online: Mutex<HashMap<String, Result<Offer, Problem>>>,
    busy: AtomicBool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OfferView {
    version: String,
    /// payload · github
    source: &'static str,
    size: Option<u64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: String,
    name: String,
    blurb: String,
    icon: Option<String>,
    default: bool,
    installed: Option<InstalledView>,
    offer: Option<OfferView>,
    /// not-installed · update · current · ahead · installed · unavailable
    status: &'static str,
    /// Why there is no offer, or what was wrong with the payload copy.
    note: Option<String>,
    /// It has a GitHub release to ask about.
    downloadable: bool,
    /// GitHub has been asked about it, whatever the answer.
    asked: bool,
}

#[derive(Serialize, Clone)]
struct InstalledView {
    version: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PayloadView {
    present: bool,
    dir: String,
    problem: Option<String>,
    count: usize,
    built: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SuiteState {
    apps: Vec<Row>,
    payload: PayloadView,
    dry_run: bool,
    version: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OnlineState {
    apps: Vec<Row>,
    /// GitHub answered for at least one app.
    reached: bool,
    problem: Option<String>,
}

#[cfg(windows)]
fn registry() -> impl Registry {
    detect::WindowsRegistry
}

fn detect_one(suite: &Suite, app: &App) -> Option<Installed> {
    detect::detect_app(&registry(), &suite.uninstall_root, &app.detect, &|p: &Path| p.is_file())
}

/// Where this app would come from right now, and the row that says so.
fn plan(ctx: &Ctx, app: &App, payload: &manifest::Payload) -> (Row, Option<Origin>, Option<String>) {
    let installed = detect_one(&ctx.suite, app);
    let mut note = None;

    let mut from_payload: Option<(String, Origin, u64)> = None;
    if let Some(item) = payload.manifest.as_ref().and_then(|m| m.items.iter().find(|i| i.id == app.id)) {
        match manifest::quick_check(&payload.dir, item) {
            Ok(path) => {
                let origin = Origin::Payload { path, sha256: item.sha256.clone(), size: item.size };
                from_payload = Some((item.version.clone(), origin, item.size));
            }
            Err(problem) => note = Some(problem),
        }
    }

    let downloadable = matches!(app.source, Source::Github { .. });
    let answer = ctx.online.lock().unwrap().get(&app.id).cloned();
    let asked = answer.is_some();
    let from_github = match answer {
        Some(Ok(offer)) => Some(offer),
        Some(Err(problem)) => {
            if from_payload.is_none() && note.is_none() {
                note = Some(problem.words());
            }
            None
        }
        None => None,
    };

    // The payload wins a tie: it is already here and already hashed. GitHub wins only when newer.
    let (offer, origin, version) = match (from_payload, from_github) {
        (Some((pv, _, _)), Some(g)) if version::compare(&g.version, &pv) == Ordering::Greater => {
            let view = OfferView { version: version::display(&g.version), source: "github", size: g.size };
            let v = g.version.clone();
            (Some(view), Some(Origin::Download { offer: g }), Some(v))
        }
        (Some((pv, origin, size)), _) => {
            (Some(OfferView { version: version::display(&pv), source: "payload", size: Some(size) }), Some(origin), Some(pv))
        }
        (None, Some(g)) => {
            let view = OfferView { version: version::display(&g.version), source: "github", size: g.size };
            let v = g.version.clone();
            (Some(view), Some(Origin::Download { offer: g }), Some(v))
        }
        (None, None) => (None, None, None),
    };

    if offer.is_none() && note.is_none() && !downloadable {
        note = Some(if payload.manifest.is_some() {
            "Not in this payload folder, and it has no download yet.".to_string()
        } else {
            "It has no download yet. It installs only from a bundle with a payload folder.".to_string()
        });
    }

    let have = installed.as_ref().and_then(|i| i.version.as_deref());
    let status = match (&installed, have) {
        // Installed, but its entry carries no version: it is there, and that is all we know.
        (Some(_), None) => "installed",
        _ => version::status(have, version.as_deref()),
    };

    let row = Row {
        id: app.id.clone(),
        name: app.name.clone(),
        blurb: app.blurb.clone(),
        icon: app.icon.clone(),
        default: app.default,
        installed: installed.map(|i| InstalledView { version: i.version.as_deref().map(version::display) }),
        offer,
        status,
        note,
        downloadable,
        asked,
    };
    (row, origin, version)
}

fn rows(ctx: &Ctx) -> Vec<Row> {
    let payload = manifest::load(&ctx.payload_dir);
    ctx.suite.apps.iter().map(|app| plan(ctx, app, &payload).0).collect()
}

#[tauri::command]
fn suite_state(ctx: State<Ctx>) -> SuiteState {
    let payload = manifest::load(&ctx.payload_dir);
    SuiteState {
        apps: ctx.suite.apps.iter().map(|app| plan(&ctx, app, &payload).0).collect(),
        payload: PayloadView {
            present: payload.present,
            dir: payload.dir.to_string_lossy().into_owned(),
            problem: payload.problem.clone(),
            count: payload.manifest.as_ref().map_or(0, |m| m.items.len()),
            built: payload.manifest.as_ref().and_then(|m| m.built.clone()),
        },
        dry_run: ctx.dry_run,
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// Ask GitHub about every app that has a release there. Slow by nature, so it is its own command
/// and the window is already drawn from the payload by the time it answers.
#[tauri::command]
async fn check_online(app: AppHandle) -> OnlineState {
    tauri::async_runtime::spawn_blocking(move || {
        let ctx = app.state::<Ctx>();
        let mut reached = false;
        let mut problem = None;
        match github::agent() {
            Err(text) => problem = Some(text),
            Ok(agent) => {
                for item in &ctx.suite.apps {
                    let Source::Github { repo, tag_prefix, asset_suffix } = &item.source else { continue };
                    let answer = github::fetch_offer(&agent, repo, tag_prefix, asset_suffix);
                    let offline = matches!(answer, Err(Problem::Offline));
                    match &answer {
                        Ok(_) => reached = true,
                        Err(p) => problem = Some(p.words()),
                    }
                    ctx.online.lock().unwrap().insert(item.id.clone(), answer);
                    if offline {
                        // No route: the rest would each wait out the same timeout to say the same.
                        for other in &ctx.suite.apps {
                            if matches!(other.source, Source::Github { .. }) {
                                ctx.online.lock().unwrap().entry(other.id.clone()).or_insert(Err(Problem::Offline));
                            }
                        }
                        break;
                    }
                }
            }
        }
        OnlineState { apps: rows(&ctx), reached, problem: if reached { None } else { problem } }
    })
    .await
    .unwrap_or_else(|_| OnlineState { apps: Vec::new(), reached: false, problem: Some("The check stopped unexpectedly.".into()) })
}

#[tauri::command]
fn laptop_state(ctx: State<Ctx>) -> Vec<detect::LaptopFinding> {
    detect::detect_laptop(&registry(), &ctx.suite.laptop, &detect::driver_station_paths(), &|p: &Path| p.is_file())
}

/// Install the chosen apps, one after another, on a thread of their own. Progress arrives as
/// `setup://progress` events and the end as `setup://finished`, carrying the rows as they now are.
#[tauri::command]
fn start_install(app: AppHandle, ctx: State<Ctx>, ids: Vec<String>) -> Result<(), String> {
    if ctx.busy.swap(true, Atomic::SeqCst) {
        return Err("An install is already running.".into());
    }
    let payload = manifest::load(&ctx.payload_dir);
    let mut jobs = Vec::new();
    let mut refused = Vec::new();
    for id in &ids {
        let Some(item) = ctx.suite.apps.iter().find(|a| &a.id == id) else { continue };
        match plan(&ctx, item, &payload) {
            (_, Some(origin), Some(version)) => jobs.push(Job {
                id: item.id.clone(),
                name: item.name.clone(),
                version,
                origin,
                installer_kind: item.installer.kind.clone(),
                silent_args: item.installer.silent_args.clone(),
                exe_name: item.detect.exe.clone(),
            }),
            (row, _, _) => refused.push((item.id.clone(), row.note)),
        }
    }

    std::thread::spawn(move || {
        let ctx = app.state::<Ctx>();
        let emit = |progress: install::Progress| {
            let _ = app.emit("setup://progress", progress);
        };
        for (id, note) in refused {
            emit(install::Progress {
                id,
                phase: "failed",
                received: 0,
                total: None,
                message: Some(note.unwrap_or_else(|| "There is no installer for it here.".into())),
                version: None,
                dry: ctx.dry_run,
            });
        }
        let detect = |id: &str| ctx.suite.apps.iter().find(|a| a.id == id).and_then(|a| detect_one(&ctx.suite, a));
        let machine = Machine {
            dry_run: ctx.dry_run,
            download_dir: std::env::temp_dir().join("CatalystSetup"),
            is_running: &install::is_running,
            detect: &detect,
            agent: &github::agent,
        };
        for job in &jobs {
            install::run_job(job, &machine, &emit);
        }
        ctx.busy.store(false, Atomic::SeqCst);
        let _ = app.emit("setup://finished", rows(&ctx));
    });
    Ok(())
}

#[tauri::command]
fn launch_app(ctx: State<Ctx>, id: String) -> Result<(), String> {
    let item = ctx.suite.apps.iter().find(|a| a.id == id).ok_or("That app is not in the suite.")?;
    let found = detect_one(&ctx.suite, item).ok_or_else(|| format!("{} is not installed.", item.name))?;
    std::process::Command::new(&found.exe)
        .current_dir(&found.location)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{} could not be started: {e}", item.name))
}

/// Open one of the download pages named in suite.json in the laptop's browser, and nothing else.
#[tauri::command]
fn open_link(ctx: State<Ctx>, url: String) -> Result<(), String> {
    if !ctx.suite.laptop.iter().any(|item| item.url == url) {
        return Err("That address is not one of the suite's links.".into());
    }
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("The browser could not be opened: {e}"))
}

fn payload_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CATALYST_SETUP_PAYLOAD") {
        return PathBuf::from(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("payload")))
        .unwrap_or_else(|| PathBuf::from("payload"))
}

fn main() {
    let dry_run = std::env::args().any(|a| a == "--dry-run")
        || std::env::var("CATALYST_SETUP_DRY_RUN").is_ok_and(|v| v == "1");
    tauri::Builder::default()
        .manage(Ctx {
            suite: suite::load(),
            payload_dir: payload_dir(),
            dry_run,
            online: Mutex::new(HashMap::new()),
            busy: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            suite_state,
            check_online,
            laptop_state,
            start_install,
            launch_app,
            open_link
        ])
        .run(tauri::generate_context!())
        .expect("error while running Catalyst Setup");
}
