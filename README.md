# Catalyst Setup

One program that puts every Catalyst app on a driver-station laptop, and keeps them current.

| App | What it is | Where Setup gets it |
|---|---|---|
| Catalyst | The Catalyst tools and the library installer | GitHub release, or the stick |
| Catalyst Console | The driver station dashboard | GitHub release, or the stick |
| Catalyst Pit | Match day in the pit: checklist, batteries, queue, match log | GitHub release, or the stick |
| Catalyst Sim (MO) | Driver practice for Numbers | The stick only |
| Catalyst Link | The Tab5 companion (optional) | The stick only |

It installs for the signed-in user. It never asks for an administrator password, never closes an
app you have open, and never installs anything that is not in the table above.

## Using the stick

1. Copy the `CatalystSuite` folder from the stick to the laptop, or run it straight from the stick.
   Keep `Catalyst Setup.exe` and the `payload` folder together: the payload is what lets it work
   with no network.
2. Run `Catalyst Setup.exe`.
3. Each row shows the version on the laptop, the version on offer, and one of *not installed*,
   *update* or *up to date*. Everything worth installing is already ticked. Catalyst Link is
   optional and starts unticked.
4. Press the button. It says what it will do ("Install 3, update 1"). Each app's own installer runs
   in turn; if one fails, the row says why and the rest carry on.
5. On the last page, **Open** starts an app.

With a network, Setup also asks GitHub and uses a release when it is newer than the stick's copy.
With no stick at all, `catalyst-setup.exe` on its own downloads what it installs — but the Sim and
Link have no downloads, so they need a stick.

**If a row says an app is open:** close that app and press the button again. Setup will not close
it for you.

The panel on the right, *This laptop*, only looks: it reports whether the FRC Driver Station, the
NI FRC Game Tools and WebView2 are there, with a link to the official download for anything missing.
Setup never downloads or installs those.

## Keep these apps up to date

The switch under the app list. It is on by default and takes effect when you press the install
button (or straight away with *Turn on now*). The window always shows the laptop's real state: *On*
means the scheduled task exists.

Turning it on:

- copies Catalyst Setup to `%LOCALAPPDATA%\Catalyst Setup\`, so pulling the stick out changes
  nothing, and lists it under Windows Settings → Apps as "Catalyst Setup";
- adds a scheduled task for your account, "Catalyst Setup Auto Update", that runs when you sign in
  and every 4 hours. It shows no window.

Each run:

- **does nothing while the FRC Driver Station is open.** A robot laptop does not change under the
  drive team. It tries again on the next run;
- looks only at apps that are **already installed**. It never installs a new one;
- updates an app only to a newer GitHub release whose download matches the sha256 GitHub publishes
  for it;
- skips an app that is open, and tries again next run;
- stops quietly when there is no network;
- keeps Catalyst Setup itself current the same way.

The Sim and Link are marked *updates by hand*: they have no releases to check, so they change only
when you run Setup with a newer stick.

The window shows what the last run did ("Last checked Oct 5, 4:12 PM: updated Catalyst to 2.8.0").
The full history is `%LOCALAPPDATA%\Catalyst Setup\auto.log`.

**Before an event**, if you want the laptop frozen: turn the switch off. That removes the task.
Turn it back on afterwards.

**To remove Catalyst Setup**: Windows Settings → Apps → Catalyst Setup → Uninstall. That removes
the task and Setup's own files. The apps stay.

## Making a stick

On a machine with the repos side by side under one folder and `gh` signed in:

```
npm install
npm run build        # the portable exe: src-tauri/target/release/catalyst-setup.exe
npm run bundle       # dist/CatalystSuite/ and dist/CatalystSuite.zip
```

`npm run bundle` pulls the latest Catalyst and Console installers from their GitHub releases, takes
the Sim's from `../MoSimBuilder/Builds/`, and takes Pit's and Link's from their local Tauri build
folders (or Pit's release, when there is no local build). Anything it cannot find it skips, and
says so; the stick then shows that app as *not available*. Unzip `CatalystSuite.zip` onto the stick.

Each GitHub release of this repo also carries a `CatalystSuite.zip`, built in CI. That one has no
Sim and no Link: CI cannot reach installers that exist only on the owner's machine.

## Releasing

Push to `main`. `.github/workflows/release.yml` runs the tests on every push, and publishes
`v<version>` only when that tag does not exist yet. So a release is: bump the version in
`package.json`, `package-lock.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`
together, and push. A push without a new version only runs the checks. A version is never built
twice.

## Working on it

```
npm test             # identity drift check, node --test, cargo test
npm run harness      # the real window in a browser, over a scripted laptop
npm run shots        # screenshots of every step from the harness, into shots/
npm run shots:real   # drives the built exe in dry-run and screenshots it
```

`catalyst-setup.exe --dry-run` does everything except change the laptop: files are checked and
downloaded, no installer runs, no task is registered. `catalyst-setup.exe --auto --dry-run` does
the same for the background run and writes what it would do to `auto.log`.

`suite.json` is the one table of what the suite is: every app's id, name, source, silent switch and
how to tell it is installed. `AGENTS.md` has the rules for changing things.
