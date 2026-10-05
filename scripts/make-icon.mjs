// Turn assets/setup-icon.svg into the icon set Tauri wants.
//
//   node scripts/make-icon.mjs
//
// Headless Edge draws the SVG (it is the one rasteriser on these machines that works: cairosvg and
// ImageMagick's `convert` both look available and both fail), then `tauri icon` cuts the sizes.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const EDGE = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
].find(existsSync);
if (!EDGE) {
  console.error("make-icon: Microsoft Edge was not found");
  process.exit(1);
}

const work = join(tmpdir(), "catalyst-setup-icon");
mkdirSync(work, { recursive: true });
const svg = readFileSync(join(root, "assets", "setup-icon.svg"), "utf8");
const page = join(work, "icon.html");
writeFileSync(page, `<!doctype html><style>html,body{margin:0;background:transparent}svg{display:block;width:1024px;height:1024px}</style>${svg}`);
const png = join(root, "assets", "setup-icon.png");
execFileSync(EDGE, [
  "--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
  "--default-background-color=00000000", "--window-size=1024,1024",
  `--user-data-dir=${join(work, "profile")}`, `--screenshot=${png}`, pathToFileURL(page).href,
], { stdio: "ignore" });
console.log("make-icon: drew", png);

execFileSync(process.execPath, [join(root, "node_modules", "@tauri-apps", "cli", "tauri.js"), "icon", png, "-o", join(root, "src-tauri", "icons")], { stdio: "inherit", cwd: root });
