import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { buildManifest, compareVersions, newestSetup, releaseNotes, sha256File, trimFileVersion, versionFromSetupName } from "./bundle-lib.mjs";

test("a version is read out of a Tauri setup name", () => {
  assert.equal(versionFromSetupName("Catalyst_2.8.0_x64-setup.exe"), "2.8.0");
  assert.equal(versionFromSetupName("Catalyst Console_2.0.0_x64-setup.exe"), "2.0.0");
  assert.equal(versionFromSetupName("Catalyst.Console_2.0.0_x64-setup.exe"), "2.0.0");
  assert.equal(versionFromSetupName("Catalyst Pit_0.1.0-beta.2_x64-setup.exe"), "0.1.0-beta.2");
  assert.equal(versionFromSetupName("CatalystSimSetup.exe"), null);
});

test("versions compare by number", () => {
  assert.equal(compareVersions("2.10.0", "2.9.0"), 1);
  assert.equal(compareVersions("1.4.3", "2.0.0"), -1);
  assert.equal(compareVersions("2.8.0", "v2.8.0"), 0);
  assert.equal(compareVersions("2.0.0", "2.0.0-beta.2"), 1);
  assert.equal(compareVersions("2.0.0-beta.2", "2.0.0-beta.10"), -1);
  assert.equal(compareVersions("2.0.0-beta.2", "2.0.0"), -1);
});

test("the newest installer is picked from a build folder that keeps every old one", () => {
  const names = [
    "Catalyst_1.4.3_x64-setup.exe", "Catalyst_2.0.0_x64-setup.exe", "Catalyst_2.0.0_x64-setup.exe.sig",
    "Catalyst_2.10.0_x64-setup.exe", "Catalyst_2.8.0_x64-setup.exe", "latest.json",
  ];
  assert.deepEqual(newestSetup(names), { name: "Catalyst_2.10.0_x64-setup.exe", version: "2.10.0" });
  assert.equal(newestSetup(["latest.json", "notes.txt"]), null);
  assert.equal(newestSetup([]), null);
});

test("a Windows file version loses its fourth zero", () => {
  assert.equal(trimFileVersion("1.4.0.0"), "1.4.0");
  assert.equal(trimFileVersion("1.4.0.7"), "1.4.0.7");
  assert.equal(trimFileVersion(" 2.0.0 \r\n"), "2.0.0");
});

test("the manifest has the shape the Rust side reads, and nothing else", () => {
  const m = buildManifest([{ id: "sim", version: "1.4.0", file: "CatalystSimSetup.exe", sha256: "ab".repeat(32), size: 3, extra: "dropped" }], "2026-10-05T00:00:00.000Z");
  assert.deepEqual(m, {
    schema: 1, built: "2026-10-05T00:00:00.000Z",
    items: [{ id: "sim", version: "1.4.0", file: "CatalystSimSetup.exe", sha256: "ab".repeat(32), size: 3 }],
  });
});

test("sha256 of a file", async () => {
  const dir = mkdtempSync(join(tmpdir(), "catalyst-setup-"));
  writeFileSync(join(dir, "abc"), "abc");
  assert.equal(await sha256File(join(dir, "abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
});

test("suite.json and the harness agree on the apps", () => {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const suite = JSON.parse(readFileSync(join(root, "suite.json"), "utf8"));
  const fake = readFileSync(join(root, "scripts", "fake-tauri.js"), "utf8");
  for (const app of suite.apps) {
    assert.ok(fake.includes(`name: ${JSON.stringify(app.name)}`), `the harness has no ${app.name}`);
    // An icon copied in from the app's own repo (iconFrom) may not have arrived yet; the window
    // then draws the app's initial. Every other icon has to be here.
    if (app.icon && !app.iconFrom) assert.ok(readFileSync(join(root, "src", app.icon)).length > 0, `${app.icon} is missing`);
    assert.ok(fake.includes(JSON.stringify(app.blurb)), `the harness has a different blurb for ${app.name}`);
  }
  // No BOM: the Rust side embeds this file as it is.
  assert.notEqual(readFileSync(join(root, "suite.json"))[0], 0xef);
});

test("release notes say what is missing from the zip, and why", () => {
  const notes = releaseNotes("1.0.0",
    [{ name: "Catalyst", version: "2.8.0" }, { name: "Catalyst Console", version: "2.0.0" }],
    [{ name: "Catalyst Sim (MO)", why: "its installer is built on the owner's machine and has no release to fetch" }]);
  assert.match(notes, /^Catalyst Setup 1\.0\.0/);
  assert.match(notes, /- Catalyst 2\.8\.0\n- Catalyst Console 2\.0\.0/);
  assert.match(notes, /Not in the zip:\n- Catalyst Sim \(MO\): its installer is built on the owner's machine/);
  assert.doesNotMatch(releaseNotes("1.0.0", [{ name: "Catalyst", version: "2.8.0" }], []), /Not in the zip/);
  assert.match(releaseNotes("1.0.0", [], []), /payload is empty/);
});

test("every file that carries the version agrees", () => {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const json = (...p) => JSON.parse(readFileSync(join(root, ...p), "utf8"));
  const text = (...p) => readFileSync(join(root, ...p), "utf8");
  const found = {
    "package.json": json("package.json").version,
    "package-lock.json": json("package-lock.json").version,
    'package-lock.json packages[""]': json("package-lock.json").packages?.[""]?.version,
    "src-tauri/tauri.conf.json": json("src-tauri", "tauri.conf.json").version,
    "src-tauri/Cargo.toml": text("src-tauri", "Cargo.toml").split(/^\[/m).find((s) => s.startsWith("package]"))?.match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1],
    "src-tauri/Cargo.lock": text("src-tauri", "Cargo.lock").match(/\[\[package\]\]\s*\nname = "catalyst-setup"\s*\nversion = "([^"]+)"/)?.[1],
  };
  // The release workflow publishes v<tauri.conf.json's version>; a file that disagrees would ship
  // an exe that calls itself something else, and its self-update would then never settle.
  assert.equal(new Set(Object.values(found)).size, 1, JSON.stringify(found, null, 2));
});

test("the workflows publish only a version that has no tag yet", () => {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  for (const file of [join(".github", "workflows", "release.yml"), join("docs", "pit-release.yml")]) {
    const yml = readFileSync(join(root, file), "utf8");
    assert.match(yml, /branches: \[main\]/, `${file} runs on a push to main`);
    assert.match(yml, /workflow_dispatch/, file);
    assert.match(yml, /needs\.version\.outputs\.exists == 'false'/, `${file} builds only when the tag does not exist`);
    assert.doesNotMatch(yml, /--clobber|TAURI_SIGNING|tauri-plugin-updater/, `${file}: never a second build under a version, no signing, no updater plugin`);
    assert.doesNotMatch(yml, /\t/, `${file} has a tab in it`);
  }
});
