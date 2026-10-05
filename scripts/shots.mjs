// Screenshot every step of the window, headlessly, from the harness.
//
//   node scripts/shots.mjs [name ...]      writes shots/harness-<name>.png
//
// Headless Edge with a virtual clock: the scripted run plays to its stopping point, the springs
// settle, and the picture is taken. These are pictures of the real frontend over scripted data;
// `scripts/shots-real.mjs` is the one that photographs the built exe.

import { execFile } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

import { serve } from "./harness.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const EDGE = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
].find(existsSync);
if (!EDGE) { console.error("shots: Microsoft Edge was not found"); process.exit(1); }

const SHOTS = {
  "1-apps-fresh": "scenario=fresh",
  "1-apps-mixed": "scenario=mixed",
  "1-apps-empty": "scenario=empty",
  "2-install-downloading": "scenario=mixed&run=downloading",
  "2-install-running": "scenario=fresh&run=installing",
  "3-done": "scenario=fresh&run=done",
  "3-done-failed": "scenario=mixed&run=failed&pick=all",
  "3-done-dry-run": "scenario=mixed&run=dry",
  "1-apps-small-window": ["scenario=fresh", "980,640"],
  "4-keep-on-updated": ["scenario=mixed&keep=on&last=updated", "1040,860"],
  "4-keep-on-driver-station": ["scenario=mixed&keep=on&last=ds", "1040,860"],
  "4-keep-off": ["scenario=mixed&keep=off", "1040,860"],
  "4-keep-not-set-up-yet": ["scenario=fresh", "1040,860"],
};

const wanted = process.argv.slice(2);
const port = 5311;
const server = await serve(port);
const out = join(root, "shots");
mkdirSync(out, { recursive: true });
const run = promisify(execFile);

for (const [name, spec] of Object.entries(SHOTS)) {
  if (wanted.length && !wanted.some((w) => name.includes(w))) continue;
  const [query, size] = Array.isArray(spec) ? spec : [spec, "1040,720"];
  const file = join(out, `harness-${name}.png`);
  await run(EDGE, [
    "--headless=new", "--disable-gpu", "--force-device-scale-factor=1", `--window-size=${size}`,
    "--virtual-time-budget=9000", `--user-data-dir=${join(tmpdir(), "catalyst-setup-shots")}`,
    `--screenshot=${file}`, `http://localhost:${port}/?${query}`,
  ]);
  console.log("shots:", file);
}
server.close();
