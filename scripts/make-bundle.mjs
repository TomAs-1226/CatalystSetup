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
//   nsis-dir   the newest setup exe in a Tauri build folder (Pit, Link)
// An app whose installer is not there is skipped with a line saying so; the bundle is still made,
// and Catalyst Setup shows that app as "not available" rather than pretending.
//
// It never builds another repo, never pushes, and writes only under dist/.

import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { buildManifest, newestSetup, sha256File, trimFileVersion, versionFromSetupName } from "./bundle-lib.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = new Set(process.argv.slice(2));
const suite = JSON.parse(readFileSync(join(root, "suite.json"), "utf8"));

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

/** Find one app's installer and put it in payload/. Returns { file, version } or null (skipped). */
function collect(app) {
  const how = app.bundle || {};
  if (how.from === "github") {
    const { repo, tagPrefix, assetSuffix } = app.source;
    if (args.has("--offline")) {
      // The last bundle's manifest says which file was this app's; reuse it if it is still there.
      const kept = previous.items?.find((item) => item.id === app.id);
      if (!kept || !existsSync(join(payload, kept.file))) { say(app.id, "skipped: --offline, and no installer for it is in dist/ yet"); return null; }
      say(app.id, `${kept.version}  kept from the last bundle (--offline)`);
      return { file: kept.file, version: kept.version };
    }
    let release;
    try {
      release = JSON.parse(gh(["release", "view", "-R", repo, "--json", "tagName,assets"]));
    } catch (e) {
      say(app.id, `skipped: gh could not read ${repo}'s latest release (${String(e.stderr || e.message).trim().split("\n")[0]})`);
      return null;
    }
    if (!release.tagName.startsWith(tagPrefix)) { say(app.id, `skipped: the latest release is ${release.tagName}, not a ${tagPrefix}* tag`); return null; }
    const asset = release.assets.find((a) => a.name.toLowerCase().endsWith(assetSuffix));
    if (!asset) { say(app.id, `skipped: ${release.tagName} has no *${assetSuffix}`); return null; }
    const version = release.tagName.slice(tagPrefix.length);
    const dest = join(payload, asset.name);
    if (!(existsSync(dest) && statSync(dest).size === asset.size)) {
      gh(["release", "download", release.tagName, "-R", repo, "-p", asset.name, "-D", payload, "--clobber"]);
    }
    if (statSync(dest).size !== asset.size) throw new Error(`${asset.name} is ${statSync(dest).size} bytes; GitHub says ${asset.size}`);
    say(app.id, `${version}  ${repo} ${release.tagName}`);
    return { file: asset.name, version, expect: asset.digest?.replace(/^sha256:/, "") };
  }

  if (how.from === "file") {
    const from = resolve(root, how.path);
    if (!existsSync(from)) { say(app.id, `skipped: ${from} does not exist`); return null; }
    // The Sim's exe has no version in its name; Windows knows it from the file itself.
    const raw = execFileSync("powershell", ["-NoProfile", "-Command",
      `[System.Diagnostics.FileVersionInfo]::GetVersionInfo('${from.replace(/'/g, "''")}').FileVersion`], { encoding: "utf8" });
    const version = trimFileVersion(raw);
    if (!/^\d/.test(version)) { say(app.id, `skipped: ${from} carries no file version`); return null; }
    const file = from.split(/[\\/]/).pop();
    copyFileSync(from, join(payload, file));
    say(app.id, `${version}  ${from}`);
    return { file, version };
  }

  if (how.from === "nsis-dir") {
    const dir = resolve(root, how.path);
    if (!existsSync(dir)) { say(app.id, `skipped: not built yet (no ${dir})`); return null; }
    const newest = newestSetup(readdirSync(dir));
    if (!newest) { say(app.id, `skipped: no *-setup.exe in ${dir}`); return null; }
    copyFileSync(join(dir, newest.name), join(payload, newest.name));
    say(app.id, `${newest.version}  ${join(dir, newest.name)}`);
    return { file: newest.name, version: versionFromSetupName(newest.name) };
  }

  say(app.id, "skipped: suite.json gives it no bundle source");
  return null;
}

console.log(`make-bundle: ${out}`);
const items = [];
const skipped = [];
for (const app of suite.apps) {
  const got = collect(app);
  if (!got) { skipped.push(app.name); continue; }
  const path = join(payload, got.file);
  const sha256 = await sha256File(path);
  if (got.expect && got.expect !== sha256) throw new Error(`${got.file} does not match the sha256 GitHub publishes for it`);
  items.push({ id: app.id, version: got.version, file: got.file, sha256, size: statSync(path).size });
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
  "Run it again later to update: it installs only what is newer.",
  "",
  `In this bundle: ${items.map((i) => `${suite.apps.find((a) => a.id === i.id).name} ${i.version}`).join(", ")}.`,
  skipped.length ? `Not in this bundle: ${skipped.join(", ")}.` : "",
  "",
].join("\r\n"));

// Anything in payload/ the manifest does not name is from an older bundle. It is reported, not
// removed: this script deletes nothing.
const named = new Set([...items.map((i) => i.file), "manifest.json"]);
const strays = readdirSync(payload).filter((n) => !named.has(n));
if (strays.length) console.log(`  note      payload/ also holds ${strays.join(", ")} (older; not in the manifest, and left alone)`);

if (!args.has("--no-zip")) {
  const zip = join(root, "dist", "CatalystSuite.zip");
  // The tar that ships with Windows (bsdtar) writes a real zip with -a. Named by its full path:
  // under Git Bash a bare `tar` is GNU tar, which reads "C:" as a remote host and writes no zips.
  const tar = join(process.env.SystemRoot || "C:\Windows", "System32", "tar.exe");
  const files = ["Catalyst Setup.exe", "README.txt", "payload/manifest.json", ...items.map((i) => `payload/${i.file}`)];
  execFileSync(tar, ["-a", "-c", "-f", zip, "-C", out, ...files], { stdio: "inherit" });
  console.log(`make-bundle: ${zip}  (${(statSync(zip).size / 1048576).toFixed(1)} MB)`);
}
console.log(`make-bundle: ${items.length} of ${suite.apps.length} apps in the payload${skipped.length ? `; skipped ${skipped.join(", ")}` : ""}`);
