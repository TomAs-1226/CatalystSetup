// Assemble dist/CatalystSuite/: the portable exe plus a payload/ folder holding every installer
// it can find, with a manifest of versions and hashes — then zip it for a USB stick.
//
//   node scripts/make-bundle.mjs              build the folder and the zip
//   node scripts/make-bundle.mjs --no-zip     the folder only
//   node scripts/make-bundle.mjs --offline    do not ask GitHub; reuse installers already in dist/
//
// Where each installer comes from is in suite.json (`bundle`):
//   github     the latest release's setup exe, through `gh release download`
//   file       one file on this machine (the Sim, which has no release)
//   nsis-dir   the newest setup exe in a Tauri build folder (Pit, Link). When that folder is not
//              on this machine and the app has releases, its latest release is used instead —
//              which is what happens in CI, where no sibling checkout exists.
// An app whose installer is not there is skipped with a line saying so; the bundle is still made,
// and Catalyst Setup shows that app as "not available" rather than pretending.
//
// It also writes dist/release-notes.md, which says what the bundle holds and what it could not;
// the release workflow publishes that as the release's text.
//
// It never builds another repo, never pushes, and writes only under dist/ (and src/icons, through
// sync-icons, for an app whose icon lives in its own repo).

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { buildManifest, newestSetup, releaseNotes, sha256File, trimFileVersion, versionFromSetupName } from "./bundle-lib.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));
const suite = JSON.parse(readFileSync(join(root, "suite.json"), "utf8"));
const version = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")).version;

const exe = join(root, "src-tauri", "target", "release", "catalyst-setup.exe");
if (!existsSync(exe)) {
  console.error("make-bundle: no release exe. Run `npm run build` first.");
  process.exit(1);
}

const out = join(root, "dist", "CatalystSuite");
const payload = join(out, "payload");
mkdirSync(payload, { recursive: true });

const manifestPath = join(payload, "manifest.json");
const previous = existsSync(manifestPath) ? JSON.parse(readFileSync(manifestPath, "utf8")) : {};

const say = (id, text) => console.log(`  ${id.padEnd(9)} ${text}`);
const gh = (argv) => execFileSync("gh", argv, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
const skip = (app, why) => { say(app.id, `skipped: ${why}`); return { skipped: why }; };

function fromGithub(app) {
  const { repo, tagPrefix, assetSuffix } = app.source;
  if (args.has("--offline")) {
    // The last bundle's manifest says which file was this app's; reuse it if it is still there.
    const kept = previous.items?.find((item) => item.id === app.id);
    if (!kept || !existsSync(join(payload, kept.file))) return skip(app, "--offline, and no installer for it is in dist/ yet");
    say(app.id, `${kept.version}  kept from the last bundle (--offline)`);
    return { file: kept.file, version: kept.version };
  }
  let release;
  try {
    release = JSON.parse(gh(["release", "view", "-R", repo, "--json", "tagName,assets"]));
  } catch (e) {
    return skip(app, `${repo} has no release that gh can read (${String(e.stderr || e.message).trim().split("\n")[0]})`);
  }
  if (!release.tagName.startsWith(tagPrefix)) return skip(app, `the latest release of ${repo} is ${release.tagName}, not a ${tagPrefix}* tag`);
  const asset = release.assets.find((a) => a.name.toLowerCase().endsWith(assetSuffix));
  if (!asset) return skip(app, `${repo} ${release.tagName} has no *${assetSuffix}`);
  const dest = join(payload, asset.name);
  if (!(existsSync(dest) && statSync(dest).size === asset.size)) {
    gh(["release", "download", release.tagName, "-R", repo, "-p", asset.name, "-D", payload, "--clobber"]);
  }
  if (statSync(dest).size !== asset.size) throw new Error(`${asset.name} is ${statSync(dest).size} bytes; GitHub says ${asset.size}`);
  const got = release.tagName.slice(tagPrefix.length);
  say(app.id, `${got}  ${repo} ${release.tagName}`);
  return { file: asset.name, version: got, expect: asset.digest?.replace(/^sha256:/, "") };
}

/** Find one app's installer and put it in payload/. Returns { file, version } or { skipped }. */
function collect(app) {
  const how = app.bundle || {};
  const released = app.source?.kind === "github";
  if (how.from === "github") return fromGithub(app);

  if (how.from === "file") {
    const from = resolve(root, how.path);
    if (!existsSync(from)) return skip(app, "its installer is built on the owner's machine and has no release to fetch");
    // The Sim's exe has no version in its name; Windows knows it from the file itself.
    const raw = execFileSync("powershell", ["-NoProfile", "-Command",
      `[System.Diagnostics.FileVersionInfo]::GetVersionInfo('${from.replace(/'/g, "''")}').FileVersion`], { encoding: "utf8" });
    const got = trimFileVersion(raw);
    if (!/^\d/.test(got)) return skip(app, `${from} carries no file version`);
    const file = from.split(/[\\/]/).pop();
    copyFileSync(from, join(payload, file));
    say(app.id, `${got}  ${from}`);
    return { file, version: got };
  }

  if (how.from === "nsis-dir") {
    const dir = resolve(root, how.path);
    const newest = existsSync(dir) ? newestSetup(readdirSync(dir)) : null;
    if (newest) {
      copyFileSync(join(dir, newest.name), join(payload, newest.name));
      say(app.id, `${newest.version}  ${join(dir, newest.name)}`);
      return { file: newest.name, version: versionFromSetupName(newest.name) };
    }
    if (released) return fromGithub(app);
    return skip(app, "it has no release, and no local build of it was found");
  }

  return skip(app, "suite.json gives it no bundle source");
}

// An app's icon can live in its own repo; pick it up when that repo is beside this one.
execFileSync(process.execPath, [join(root, "scripts", "sync-icons.mjs")], { stdio: "inherit" });

console.log(`make-bundle: ${out}`);
const items = [];
const skipped = [];
for (const app of suite.apps) {
  const got = collect(app);
  if (got.skipped) { skipped.push({ name: app.name, why: got.skipped }); continue; }
  const path = join(payload, got.file);
  const sha256 = await sha256File(path);
  if (got.expect && got.expect !== sha256) throw new Error(`${got.file} does not match the sha256 GitHub publishes for it`);
  items.push({ id: app.id, name: app.name, version: got.version, file: got.file, sha256, size: statSync(path).size });
}

// Plain UTF-8, no BOM, LF: written by Node, never by PowerShell.
writeFileSync(manifestPath, JSON.stringify(buildManifest(items), null, 2) + "\n");
copyFileSync(exe, join(out, "Catalyst Setup.exe"));
writeFileSync(join(out, "README.txt"), [
  "Catalyst Suite",
  "",
  "Run \"Catalyst Setup.exe\". It installs the Catalyst apps from the payload folder beside it,",
  "so it works with no network. Keep the exe and the payload folder together.",
  "",
  "It installs for the signed-in user only and never asks for an administrator password.",
  "Leave \"Keep these apps up to date\" on and the laptop checks for new versions by itself;",
  "or run this program again later: it installs only what is newer.",
  "",
  `In this bundle: ${items.map((i) => `${i.name} ${i.version}`).join(", ") || "no apps"}.`,
  skipped.length ? `Not in this bundle: ${skipped.map((s) => s.name).join(", ")}.` : "",
  "",
].join("\r\n"));
writeFileSync(join(root, "dist", "release-notes.md"), releaseNotes(version, items, skipped));

// Anything in payload/ the manifest does not name is from an older bundle. It is reported, not
// removed: this script deletes nothing.
const named = new Set([...items.map((i) => i.file), "manifest.json"]);
const strays = readdirSync(payload).filter((n) => !named.has(n));
if (strays.length) console.log(`  note      payload/ also holds ${strays.join(", ")} (older; not in the manifest, and left alone)`);

if (!args.has("--no-zip")) {
  const zip = join(root, "dist", "CatalystSuite.zip");
  // The tar that ships with Windows (bsdtar) writes a real zip with -a. Named by its full path:
  // under Git Bash a bare `tar` is GNU tar, which reads "C:" as a remote host and writes no zips.
  const tar = join(process.env.SystemRoot || "C:\\Windows", "System32", "tar.exe");
  const files = ["Catalyst Setup.exe", "README.txt", "payload/manifest.json", ...items.map((i) => `payload/${i.file}`)];
  execFileSync(tar, ["-a", "-c", "-f", zip, "-C", out, ...files], { stdio: "inherit" });
  console.log(`make-bundle: ${zip}  (${(statSync(zip).size / 1048576).toFixed(1)} MB)`);
}
console.log(`make-bundle: ${items.length} of ${suite.apps.length} apps in the payload${skipped.length ? `; skipped ${skipped.map((s) => s.name).join(", ")}` : ""}`);
