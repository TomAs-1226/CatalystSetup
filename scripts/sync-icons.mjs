// Copy an app's icon in from its own repo, when that repo is beside this one.
//
//   node scripts/sync-icons.mjs
//
// suite.json gives an app `icon` (where the window loads it from, under src/) and, for an app
// whose artwork lives elsewhere, `iconFrom` (the 128 px PNG in that app's Tauri icons folder).
// With the source there, the copy is refreshed. Without it nothing changes: a copy already
// checked in keeps being used, and with no copy at all the window draws the app's initial.

import { copyFileSync, existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const suite = JSON.parse(readFileSync(join(root, "suite.json"), "utf8"));

for (const app of suite.apps) {
  if (!app.icon || !app.iconFrom) continue;
  const from = resolve(root, app.iconFrom);
  const to = join(root, "src", app.icon);
  if (!existsSync(from)) {
    console.log(`sync-icons: ${app.name}: ${from} is not here; ${existsSync(to) ? "keeping the copy already in src/" : "the window will show its initial"}`);
    continue;
  }
  if (existsSync(to) && readFileSync(from).equals(readFileSync(to))) continue;
  copyFileSync(from, to);
  console.log(`sync-icons: ${app.name}: copied ${from}`);
}
