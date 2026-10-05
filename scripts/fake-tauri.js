// A `window.__TAURI__` that plays a scripted laptop, so the real `src/` can be looked at in a
// browser and screenshotted headlessly. Nothing here installs, downloads or reads the machine.
//
//   ?scenario=fresh|mixed|empty|damaged      what the laptop and the payload look like
//   &run=downloading|installing|done|failed|dry   press Install and play the run up to that point
//
// The shapes are the ones src-tauri/src/main.rs returns; when a field is added there, add it here.

(() => {
  const params = new URLSearchParams(location.search);
  const scenario = params.get("scenario") || "mixed";
  const run = params.get("run");

  const APPS = {
    catalyst: { name: "Catalyst", blurb: "The Catalyst tools in one window, and the library installer for robot projects.", icon: "icons/catalyst.png", default: true, downloadable: true },
    console: { name: "Catalyst Console", blurb: "The driver station dashboard: read-only telemetry, field and robot views.", icon: "icons/console.png", default: true, downloadable: true },
    sim: { name: "Catalyst Sim (MO)", blurb: "Driver practice for Numbers, offline, with the real control map.", icon: "icons/sim.png", default: true, downloadable: false },
    pit: { name: "Catalyst Pit", blurb: "Match day in the pit: the pre-match checklist, batteries, the queue and the match log.", icon: "icons/pit.png", default: true, downloadable: true },
    link: { name: "Catalyst Link", blurb: "The Tab5 companion. Optional: only for laptops that pair with a Catalyst Tab.", icon: "icons/link.png", default: false, downloadable: false },
  };

  const row = (id, installed, offer, status, extra = {}) => ({
    id, ...APPS[id],
    installed: installed ? { version: installed } : null,
    offer: offer ? { version: offer[0], source: offer[1], size: offer[2] } : null,
    status, note: null, asked: APPS[id].downloadable, ...extra,
  });

  const NO_DOWNLOAD = "GitHub has no release for this app yet.";

  // &keep=on|off (default: never asked)   &last=updated|ds|offline|none   the switch and its last run
  const LAST = {
    updated: { time: 1790930400, outcome: "checked", dry: false, updated: [{ name: "Catalyst", from: "2.7.0", to: "2.8.0" }], failed: [], waiting: [{ name: "Catalyst Console", message: "is open, so its update waits for the next run." }], setup: null },
    current: { time: 1790930400, outcome: "checked", dry: false, updated: [], failed: [], waiting: [], setup: null },
    ds: { time: 1790930400, outcome: "driver-station", dry: false, updated: [], failed: [], waiting: [], setup: null },
    offline: { time: 1790930400, outcome: "offline", dry: false, updated: [], failed: [], waiting: [], setup: null },
  };
  const keep = {
    registered: params.get("keep") === "on",
    installedVersion: params.get("keep") === "on" ? "1.0.0" : null,
    preference: params.get("keep") === "on" ? true : params.get("keep") === "off" ? false : null,
    everyHours: 4,
    lastRun: LAST[params.get("last")] || null,
    note: null,
  };

  const SCENARIOS = {
    // A laptop out of the box, a full stick, no network.
    fresh: {
      payload: { present: true, dir: "E:\\CatalystSuite\\payload", problem: null, count: 5, built: "2026-10-05T19:40:00Z" },
      online: { reached: false, problem: "No connection to GitHub." },
      apps: [
        row("catalyst", null, ["2.8.0", "payload", 5597472], "not-installed"),
        row("console", null, ["2.0.0", "payload", 2367343], "not-installed"),
        row("sim", null, ["1.4.0", "payload", 49832448], "not-installed"),
        row("pit", null, ["0.1.0", "payload", 3145728], "not-installed"),
        row("link", null, ["1.0.0", "payload", 2936012], "not-installed"),
      ],
      laptop: [false, false, "154.0.4258.53"],
    },
    // A laptop that has been to an event: some things current, one behind, one not in the stick.
    mixed: {
      payload: { present: true, dir: "E:\\CatalystSuite\\payload", problem: null, count: 4, built: "2026-10-05T19:40:00Z" },
      online: { reached: true, problem: null },
      apps: [
        row("catalyst", "2.7.0", ["2.8.0", "github", 5597472], "update"),
        row("console", "2.0.0", ["2.0.0", "payload", 2367343], "current"),
        row("sim", null, ["1.4.0", "payload", 49832448], "not-installed"),
        row("pit", null, null, "unavailable", { note: NO_DOWNLOAD }),
        row("link", "1.0.0", ["1.0.0", "payload", 2936012], "current"),
      ],
      laptop: ["FIRST Driver Station 2027.0.0-alpha-6", "NI FIRST Robotics Utilities · 26.00.49166", "154.0.4258.53"],
    },
    // The bare exe, copied on its own, with no network: nothing to install, and it says so.
    empty: {
      payload: { present: false, dir: "C:\\Users\\driver\\Downloads\\payload", problem: null, count: 0, built: null },
      online: { reached: false, problem: "No connection to GitHub." },
      apps: [
        row("catalyst", null, null, "unavailable", { note: "No connection to GitHub." }),
        row("console", "1.4.3", null, "installed", { note: "No connection to GitHub." }),
        row("sim", null, null, "unavailable", { note: "It has no download yet. It installs only from a bundle with a payload folder." }),
        row("pit", null, null, "unavailable", { note: "It has no download yet. It installs only from a bundle with a payload folder." }),
        row("link", null, null, "unavailable", { note: "It has no download yet. It installs only from a bundle with a payload folder." }),
      ],
      laptop: [false, false, "154.0.4258.53"],
    },
  };

  const world = SCENARIOS[scenario] || SCENARIOS.mixed;
  const LAPTOP = [
    { id: "driver-station", name: "FRC Driver Station", url: "https://www.ni.com/en/support/downloads/drivers/download.frc-game-tools.html", urlLabel: "FRC Game Tools at ni.com" },
    { id: "game-tools", name: "NI FRC Game Tools", url: "https://www.ni.com/en/support/downloads/drivers/download.frc-game-tools.html", urlLabel: "FRC Game Tools at ni.com" },
    { id: "webview2", name: "WebView2 Runtime", url: "https://developer.microsoft.com/en-us/microsoft-edge/webview2/", urlLabel: "WebView2 at microsoft.com" },
  ];

  const listeners = new Map();
  const emit = (name, payload) => { for (const fn of listeners.get(name) || []) fn({ event: name, payload }); };

  // Before GitHub has answered, the rows it will fill are not yet "asked".
  const before = world.apps.map((r) => (r.offer?.source === "github" || (r.downloadable && !r.offer)
    ? { ...r, offer: null, status: r.installed ? "installed" : "unavailable", asked: false, note: null }
    : { ...r, asked: false }));

  /** Play one app's install as the backend would report it, stopping where the scenario says. */
  function script(ids) {
    const dry = run === "dry";
    const steps = [];
    const stopAt = { downloading: 0, installing: 2 }[run] ?? Infinity;   // index of the app to stop in
    ids.forEach((id, index) => {
      const app = world.apps.find((r) => r.id === id);
      const base = { id, version: app.offer.version, dry };
      const size = app.offer.size;
      if (index > stopAt) return;
      const phase = app.offer.source === "github" ? "downloading" : "verifying";
      steps.push({ ...base, phase, received: 0, total: size });
      if (index === stopAt && run === "downloading") { steps.push({ ...base, phase, received: Math.round(size * 0.58), total: size }); return; }
      steps.push({ ...base, phase, received: size, total: size });
      steps.push({ ...base, phase: "installing", received: 0, total: null });
      if (index === stopAt && run === "installing") return;
      if (run === "failed" && id === "catalyst") {
        steps.push({ ...base, phase: "failed", message: "Catalyst is open. Close it, then run setup again." });
      } else if (run === "failed" && id === "sim") {
        steps.push({ ...base, phase: "failed", message: "Catalyst_Sim_1.4.0-setup.exe is damaged: its sha256 is not the one recorded for it. Nothing was installed from it." });
      } else if (dry) {
        steps.push({ ...base, phase: "done", message: `Would have run: E:\\CatalystSuite\\payload\\${app.name.replace(/\W+/g, "_")}_${app.offer.version}_x64-setup.exe ${id === "sim" ? "/silent" : "/S"}` });
      } else {
        steps.push({ ...base, phase: "done" });
      }
    });
    return steps;
  }

  const COMMANDS = {
    suite_state: async () => ({ apps: before, payload: world.payload, dryRun: run === "dry", version: "1.0.0" }),
    check_online: async () => ({ apps: world.apps, ...world.online }),
    laptop_state: async () => LAPTOP.map((item, i) => ({ ...item, found: Boolean(world.laptop[i]), detail: world.laptop[i] || null })),
    start_install: async ({ ids }) => {
      const steps = script(ids);
      steps.forEach((step, i) => setTimeout(() => emit("setup://progress", step), 40 * (i + 1)));
      const whole = !["downloading", "installing"].includes(run);
      if (whole) {
        const after = world.apps.map((r) => (ids.includes(r.id) && run !== "failed" && run !== "dry"
          ? { ...r, installed: { version: r.offer.version }, status: "current" } : r));
        setTimeout(() => emit("setup://finished", after), 40 * (steps.length + 2));
      }
    },
    keep_state: async () => ({ ...keep }),
    keep_set: async ({ on }) => {
      if (run === "dry") return { ...keep, note: on ? "Dry run: nothing was copied and no task was registered." : "Dry run: the task was not removed." };
      Object.assign(keep, { registered: on, preference: on, installedVersion: on ? "1.0.0" : keep.installedVersion });
      return { ...keep };
    },
    launch_app: async ({ id }) => console.info("[harness] would open", id),
    open_link: async ({ url }) => console.info("[harness] would open", url),
  };

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        const fn = COMMANDS[cmd];
        if (!fn) throw new Error(`(harness) no stub for ${cmd}`);
        return fn(args || {});
      },
    },
    event: {
      listen: async (name, cb) => {
        if (!listeners.has(name)) listeners.set(name, new Set());
        listeners.get(name).add(cb);
        return () => listeners.get(name).delete(cb);
      },
    },
    window: { getCurrentWindow: () => ({ minimize: async () => {}, close: async () => {} }) },
  };

  // Step two and three are behind a press; make it once the page has settled.
  if (run) {
    window.addEventListener("load", () => setTimeout(() => {
      if (params.get("pick") === "all") {
        document.querySelectorAll("#appList input:not(:disabled):not(:checked)").forEach((box) => box.click());
      }
      document.querySelector("#go")?.click();
    }, 400));
  }

  console.info(`[harness] fake Tauri in place: scenario=${scenario}${run ? ` run=${run}` : ""}`);
})();
