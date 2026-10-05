# AGENTS.md: Catalyst Setup

Instructions for AI coding agents working in this repo: Claude Code, and whatever model runs
when Claude isn't available. For humans, `README.md` is the guide.

## What this is

The master installer for the Catalyst suite, a Tauri 2 app shipped as one portable exe (no
installer of its own). It installs Catalyst, Catalyst Console, Catalyst Pit, Catalyst Sim (MO) and
Catalyst Link onto a Windows 11 driver-station laptop, from a `payload\` folder beside the exe or
from GitHub releases, and can keep the installed ones up to date from a scheduled task. One branch,
`main`.

| Path | What it is |
|---|---|
| `suite.json` | **The single source of truth**: every app's id, name, source, silent switch and detection, and Setup's own entry. Embedded into the exe at build time; read by the scripts |
| `src/` | Frontend: vanilla HTML/CSS/JS with no build step and no packages |
| `src/js/logic.js` | Every decision and every sentence the window shows, with no DOM. Tested by `logic.test.js` |
| `src/js/app.js` | Puts those words on the page. No logic of its own |
| `src/styles/identity.css`, `src/js/motion.js` | **Copies** of FrcCatalyst's `docs/assets/` originals. Don't edit them here |
| `src/styles/setup.css` | This window's layout, on the identity's tokens |
| `src-tauri/src/detect.rs` | What is installed (registry behind a trait) and the read-only "This laptop" checks |
| `src-tauri/src/manifest.rs` | The payload folder's `manifest.json` and the sha256 check |
| `src-tauri/src/github.rs` | Latest release, download, digest check |
| `src-tauri/src/install.rs` | One install job: get, verify, run silently, confirm in the registry |
| `src-tauri/src/auto.rs` | The headless `--auto` run's decisions, log, lock and exe swap. Touches nothing itself |
| `src-tauri/src/keep.rs` | The switch: installed copy, scheduled task, Apps-list entry, `--uninstall`, and the real `--auto` |
| `scripts/` | `make-bundle` (the stick), `harness` + `fake-tauri` + `shots` (the window in a browser), `shots-real` (the built exe), `check-identity`, `sync-icons`, `make-icon` |
| `.github/workflows/release.yml` | Tests on every push to main; a release when the version's tag does not exist yet |
| `docs/pit-release.yml` | The same workflow for Catalyst Pit, kept here as a hand-over. Not run from this repo |

## The rules

Every change must keep all of these.

1. **Never needs an administrator.** Per-user installs, HKCU, a least-privilege task. An installer
   that asks to elevate is reported as a failure, not satisfied.
2. **Never closes an app, never deletes a user's files.** An app that is open is skipped with a
   sentence saying so. The only things removed are Setup's own: its task, its Apps-list entry and
   the named files in its folder, on `--uninstall`.
3. **Show only true state.** No progress bar without bytes being counted against a known total. A
   dry run is never called an install. "On" for the switch means the task exists right now. If the
   backend did not measure it, the window does not say it.
4. **Believe the laptop, not the installer.** An install succeeded when the registry shows the new
   version and the exe is there, not when the installer exited 0.
5. **Nothing unverified runs unattended.** `--auto` installs only a strictly newer release with a
   sha256 digest published by GitHub, only for apps already installed, and not at all while a
   Driver Station process is running. Don't relax any of the three.
6. **One failure never stops the rest.** Per app, in the window and in `--auto`.
7. **Detection only for third-party software.** Driver Station, Game Tools, WebView2: found or not
   found, plus the official link. Never download or install them.

## Commands

- `npm install` once. There is nothing to vendor.
- `npm test` runs the identity check, `node --test` (frontend logic and the bundle scripts) and
  `cargo test`. The Rust tests are Windows tests.
- `npm run build` builds the portable exe with `tauri build --no-bundle`. The first build takes
  several minutes.
- `npm run bundle` assembles `dist/CatalystSuite/` and its zip. Needs `gh` signed in.
- `npm run harness`, then `http://localhost:5310/?scenario=fresh|mixed|empty`, with
  `&run=downloading|installing|done|failed|dry`, `&keep=on|off` and `&last=updated|current|ds|offline`.
- `npm run shots` photographs every step from the harness with headless Edge. `npm run shots:real`
  drives the built exe. **Look at the pictures** after a UI change.
- cargo is at `~/.cargo/bin`.

## Invariants

- **`suite.json` is a contract.** The Catalyst app's launcher reads the same ids, key names and exe
  names. Tauri's NSIS installer writes `InstallLocation` wrapped in double quotes and the Sim
  writes it bare; `clean_location` handles both. Changing an id or a key is a change in two repos.
- **The asset name `catalyst-setup.exe` and the tag form `v<version>`** are how installed copies
  find their own updates. Renaming either strands every laptop on its current version.
- **A version means one build.** Bump `package.json`, `package-lock.json`,
  `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` together; a test fails while they differ.
  The workflow publishes only when the tag is absent. Never add `--clobber` or a re-release path.
- **The identity.** `identity.css` and `motion.js` are copies; change them in FrcCatalyst
  (`docs/assets/`) and run `npm run identity`. A colour, corner or curve invented in `setup.css` is
  what the check exists to prevent. Sizes that are this window's own tuning say so in a comment.
- **House style, enforced by the owner:** no pulsing dots, no "LIVE" chips, no pinned call-to-action
  bars, no default browser scrollbars, nothing that reads as template decoration.
- **The harness mirrors the backend.** `scripts/fake-tauri.js` returns the shapes `main.rs` returns.
  Add a field in one, add it in the other; a test checks the app names and blurbs agree.
- **Nothing is fetched at run time by the frontend.** No CDN, no webfont. The CSP forbids it.

## Testing without touching the machine

- Never run a real installer to test. `install.rs` is tested against a stub exe (a copy of
  `cmd.exe`), and the built exe is driven with `--dry-run`.
- Never register the scheduled task or write the Apps-list entry to test. `keep.rs` builds the task
  XML and the registry values as plain data, and those are what the tests check. `keep::set`
  refuses to act in a dry run.
- `CATALYST_SETUP_HOME` points the installed-copy folder (log, last run, settings) somewhere else;
  `CATALYST_SETUP_PAYLOAD` points the payload folder somewhere else.

## Git and files

- Don't commit, push, tag or release unless the user asks for it in the current session. A push to
  `main` with a new version number **is** a release.
- `origin` is GitHub only for now; Forgejo mirroring hasn't been added to this repo yet.
- `dist/`, `shots/`, `src-tauri/target/` and `src-tauri/gen/` are generated and gitignored.
- Files are UTF-8 without a BOM, LF. In Windows PowerShell 5.1, don't round-trip them through
  `Get-Content` / `Set-Content`. Use an editor tool. `suite.json` is embedded as it is: a BOM in it
  is a build of an exe that cannot parse its own table.

## If you are a fallback model

Fine to take on: README and comment edits, wording in `logic.js` with its tests, harness scenarios,
CSS that stays on the identity's tokens.

Stop and leave notes for Claude on: anything in `src-tauri/`, `suite.json`, the workflows, the
auto-update rules, or anything that would run an installer or register a task.
