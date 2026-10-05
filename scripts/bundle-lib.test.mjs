import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { buildManifest, compareVersions, newestSetup, sha256File, trimFileVersion, versionFromSetupName } from "./bundle-lib.mjs";

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
    if (app.icon) assert.ok(readFileSync(join(root, "src", app.icon)).length > 0, `${app.icon} is missing`);
  }
  // No BOM: the Rust side embeds this file as it is.
  assert.notEqual(readFileSync(join(root, "suite.json"))[0], 0xef);
});
