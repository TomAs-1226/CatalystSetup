// Photograph the BUILT exe, step by step, and write down what its window said.
//
//   node scripts/shots-real.mjs [path to exe] [--name bundle] [--all] [--no-install]
//
// The exe is started with --dry-run and this script refuses to press Install unless the window
// itself shows the dry-run chip: files are checked and downloaded for real, and no installer is
// ever started, so the apps on this machine are not touched.
//
// WebView2 is driven through its DevTools port, so the pictures are the window's own pixels and
// the script can press the window's own buttons. Writes shots/real-<name>-*.png and a .txt of the
// rows as read from the page.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const flag = (name) => argv.includes(name);
const value = (name, fallback) => (argv.includes(name) ? argv[argv.indexOf(name) + 1] : fallback);
const exe = resolve(argv.find((a, i) => !a.startsWith("--") && argv[i - 1] !== "--name") || join(root, "dist", "CatalystSuite", "Catalyst Setup.exe"));
const name = value("--name", "bundle");
if (!existsSync(exe)) { console.error(`shots-real: ${exe} does not exist`); process.exit(1); }

const port = 9333 + Math.floor(Math.random() * 500);
const out = join(root, "shots");
mkdirSync(out, { recursive: true });
const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

const child = spawn(exe, ["--dry-run"], {
  cwd: dirname(exe),
  env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, CATALYST_SETUP_DRY_RUN: "1" },
  stdio: "ignore",
});
const stop = () => { try { child.kill(); } catch { /* already gone */ } };
process.on("exit", stop);

async function target() {
  for (let i = 0; i < 100; i++) {
    try {
      const pages = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = pages.find((p) => p.type === "page" && p.webSocketDebuggerUrl);
      if (page) return page.webSocketDebuggerUrl;
    } catch { /* not up yet */ }
    await sleep(150);
  }
  throw new Error("the window's DevTools port never opened");
}

const ws = new WebSocket(await target());
await new Promise((ok, no) => { ws.onopen = ok; ws.onerror = no; });
let seq = 0;
const waiting = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && waiting.has(msg.id)) { waiting.get(msg.id)(msg); waiting.delete(msg.id); }
};
const cdp = (method, params = {}) => new Promise((ok, no) => {
  const id = ++seq;
  waiting.set(id, (msg) => (msg.error ? no(new Error(msg.error.message)) : ok(msg.result)));
  ws.send(JSON.stringify({ id, method, params }));
});
const js = async (expression) => {
  const r = await cdp("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
  return r.result.value;
};
const until = async (expression, what, ms = 180000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { if (await js(expression)) return; await sleep(120); }
  throw new Error(`timed out waiting for ${what}`);
};
const shot = async (step) => {
  await sleep(900);                                 // let the arriving spring settle
  const { data } = await cdp("Page.captureScreenshot", { format: "png" });
  const file = join(out, `real-${name}-${step}.png`);
  writeFileSync(file, Buffer.from(data, "base64"));
  console.log("shots-real:", file);
};
const READ = `[...document.querySelectorAll('#viewApps:not([hidden]) .row, #viewRun:not([hidden]) .run')].map(r => r.innerText.replace(/\\s*\\n\\s*/g, ' | ')).join('\\n')`;

const log = [];
try {
  await until(`document.querySelectorAll('#appList .row').length > 0`, "the app list");
  await until(`document.querySelector('#srcGithub').textContent !== 'asking'`, "GitHub's answer", 60000);
  await until(`document.querySelectorAll('#laptopList .laptop__item').length > 0`, "the laptop check");
  await until(`!document.querySelector('#keepSwitch').disabled`, "the keep-up-to-date state");
  await shot("1-apps");
  log.push("== apps ==", await js(READ), "", "lede: " + await js(`document.querySelector('#sourceLine').textContent`),
    "sources: payload=" + await js(`document.querySelector('#srcPayload').textContent`) + " github=" + await js(`document.querySelector('#srcGithub').textContent`),
    "keep: " + await js(`[document.querySelector('#keepSwitch').checked ? '[on]' : '[off]', document.querySelector('#keepLine').textContent, document.querySelector('#keepLast').hidden ? '' : document.querySelector('#keepLast').textContent].join(' ')`),
    "laptop: " + await js(`[...document.querySelectorAll('.laptop__item')].map(i => i.innerText.replace(/\\n+/g,' / ')).join(' ; ')`), "");

  if (!flag("--no-install")) {
    if (!(await js(`!document.querySelector('#dryChip').hidden`))) throw new Error("the window is NOT in dry-run mode; refusing to press Install");
    if (flag("--all")) await js(`document.querySelectorAll('#appList input:not(:disabled):not(:checked)').forEach(b => b.click())`);
    if (await js(`document.querySelector('#go').disabled`)) {
      log.push("Nothing was selectable; the install step was not reached.");
    } else {
      log.push("button: " + await js(`document.querySelector('#go').textContent`));
      await js(`document.querySelector('#go').click()`);
      await until(`!document.querySelector('#viewRun').hidden && document.querySelector('.run[data-phase]')`, "the first progress event");
      await shot("2-install");
      log.push("== mid-install ==", await js(READ), "");
      await until(`!document.querySelector('#runActions').hidden`, "the install to finish", 600000);
      await shot("3-done");
      log.push("== done ==", "title: " + await js(`document.querySelector('#runTitle').textContent`), await js(READ), "");
      // Back on the apps page, the switch shows what pressing Install did to it (in a dry run: nothing, and it says so).
      await js(`document.querySelector('#back').click()`);
      await until(`!document.querySelector('#viewApps').hidden`, "the apps page");
      await js(`document.querySelector('.col-main').scrollTop = 99999`);
      await shot("4-keep");
      log.push("keep after: " + await js(`[document.querySelector('#keepLine').textContent, document.querySelector('#keepNote').textContent].join(' | ')`));
    }
  }
} finally {
  writeFileSync(join(out, `real-${name}.txt`), log.join("\n") + "\n");
  console.log(log.join("\n"));
  ws.close();
  stop();
}
