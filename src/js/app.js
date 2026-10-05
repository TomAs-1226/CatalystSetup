// Catalyst Setup's window. Three steps on one page: choose the apps, watch them install, open them.
//
// Everything shown comes from a command in src-tauri/src/main.rs; the wording of it is in
// logic.js, which has tests. This file only puts those words on the page and moves them.

import { stateLayer } from "./motion.js";
import {
  actionLabel, defaultSelection, finished, fraction, mergeSelection, phaseText, reduceProgress,
  selectable, sourceSentence, sourceText, statusWord, summary, versionsText,
} from "./logic.js";

const tauri = window.__TAURI__;
const invoke = (cmd, args) => tauri.core.invoke(cmd, args);
const $ = (sel) => document.querySelector(sel);

const ICONS = {
  // The setup mark at titlebar size: the arrow in signal, the tray in ink.
  mark: '<svg viewBox="0 0 16 16" fill="none" stroke-linecap="round" stroke-linejoin="round" stroke-width="1.8"><path d="M3 9.5v2A1.5 1.5 0 0 0 4.5 13h7a1.5 1.5 0 0 0 1.5-1.5v-2" stroke="var(--cat-ink)"/><path d="M8 2.5v6.5M5.3 6.6 8 9.3l2.7-2.7" stroke="var(--cat-signal)"/></svg>',
  min: '<svg viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M3 7h8"/></svg>',
  close: '<svg viewBox="0 0 14 14" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"><path d="m3.5 3.5 7 7M10.5 3.5l-7 7"/></svg>',
  arrow: '<svg class="ver__arrow" viewBox="0 0 14 9" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M1 4.5h11M8.5 1 12 4.5 8.5 8"/></svg>',
  out: '<svg viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 1h6v6M9 1 1 9"/></svg>',
};

/** Build an element. Text goes in as text, so nothing read from a registry or a manifest is markup. */
function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value == null || value === false) continue;
    if (key === "class") node.className = value;
    else if (key === "html") node.innerHTML = value;       // only ever a constant from ICONS
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat()) {
    if (child == null || child === false) continue;
    node.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return node;
}

const state = {
  rows: [],
  payload: { present: false, problem: null, count: 0, built: null },
  dryRun: false,
  online: "checking",          // checking · reached · offline
  onlineProblem: null,
  selected: new Set(),
  touched: new Set(),
  run: null,                   // { ids, progress, complete }
};

// ---------------------------------------------------------------- motion

/** A role's duration and curve, read from the identity so script and CSS move the same way. */
function role(name) {
  const style = getComputedStyle(document.documentElement);
  return {
    duration: parseFloat(style.getPropertyValue(`--cat-dur-${name}`)) * 1000 || 1,
    easing: style.getPropertyValue(`--cat-ease-${name}`).trim() || "linear",
  };
}

/** One view leaves (smooth: leaving never bounces), the next arrives (release). */
async function showView(next, step) {
  const views = [$("#viewApps"), $("#viewRun")];
  const current = views.find((v) => !v.hidden);
  if (current && current !== next) {
    const leave = role("effect");
    const fade = current.animate([{ opacity: 1 }, { opacity: 0 }], { ...leave, fill: "forwards" });
    // A minimised or occluded window stops running animations, and `finished` then never settles.
    // The install must not wait on a fade nobody can see, so the clock can end it too.
    await Promise.race([fade.finished.catch(() => {}), new Promise((ok) => setTimeout(ok, leave.duration + 60))]);
    current.hidden = true;
    fade.cancel();
  }
  next.hidden = false;
  next.closest(".col-main").scrollTop = 0;
  next.animate(
    [{ opacity: 0, transform: "translateY(12px)" }, { opacity: 1, transform: "none" }],
    role("release"),
  );
  setStep(step);
}

function setStep(step) {
  for (const li of document.querySelectorAll("#tbSteps li")) {
    if (li.dataset.step === step) li.setAttribute("aria-current", "step");
    else li.removeAttribute("aria-current");
  }
}

// Bezel's press answer, for everything that is pressed: a state layer from the pointer.
document.addEventListener("pointerdown", (event) => {
  const target = event.target.closest(".cat-btn, .row:not(.row--off)");
  if (target && !target.disabled) stateLayer(target, event, { opacity: 0.08 });
});

// ---------------------------------------------------------------- step one

function icon(row) {
  if (row.icon) return el("img", { class: "icon", src: row.icon, alt: "", draggable: "false" });
  return el("span", { class: "icon icon--mono", "aria-hidden": "true" }, row.name.replace(/^Catalyst\s+/, "").charAt(0));
}

function versions(row) {
  const v = versionsText(row);
  const line = el("div", { class: "ver" });
  if (v.from && v.to) line.append(el("span", { class: "ver__from" }, v.from), el("span", { html: ICONS.arrow }), v.to);
  else if (v.to) line.append(v.to);
  return el("div", { class: "row__versions" }, line, el("div", { class: "row__source" }, sourceText(row, state.online)));
}

function stateCell(row) {
  const quiet = row.status === "current" || row.status === "unavailable" || row.status === "installed";
  return el("div", { class: `state${quiet ? " state--quiet" : ""}` },
    row.status === "current" && el("span", { class: "cat-dot cat-dot--ok", "aria-hidden": "true" }),
    statusWord(row));
}

function appRow(row) {
  const can = selectable(row);
  const box = el("input", {
    type: "checkbox",
    checked: can && state.selected.has(row.id),
    disabled: !can,
    "aria-label": row.name,
    onchange: (event) => {
      state.touched.add(row.id);
      if (event.target.checked) state.selected.add(row.id);
      else state.selected.delete(row.id);
      renderAction();
    },
  });
  box.checked = can && state.selected.has(row.id);
  // The reason there is nothing to install takes the blurb's place: it is the more useful line.
  const second = !can && row.note ? row.note : row.blurb;
  return el("label", { class: `row${can ? "" : " row--off"}`, "data-id": row.id },
    box,
    icon(row),
    el("div", { class: "row__text" },
      el("div", { class: "row__name" }, row.name, !row.default && el("span", { class: "row__tag" }, "optional")),
      el("div", { class: "row__blurb" }, second)),
    versions(row),
    stateCell(row));
}

function renderAction() {
  const go = $("#go");
  go.textContent = actionLabel(state.rows, state.selected);
  go.disabled = ![...state.selected].some((id) => state.rows.some((r) => r.id === id && selectable(r)));
}

function renderApps() {
  $("#appList").replaceChildren(...state.rows.map(appRow));
  $("#sourceLine").textContent = sourceSentence(state.payload, state.online, state.rows);
  renderAction();
  renderSources();
}

function renderSources() {
  const p = state.payload;
  let payload = "none";
  if (p.present && p.problem) payload = "cannot be used";
  else if (p.present) {
    const built = p.built && !Number.isNaN(Date.parse(p.built))
      ? ` · ${new Date(p.built).toLocaleDateString(undefined, { day: "numeric", month: "short" })}`
      : "";
    payload = `${p.count} ${p.count === 1 ? "app" : "apps"}${built}`;
  }
  $("#srcPayload").textContent = payload;
  $("#srcGithub").textContent = { checking: "asking", reached: "reached", offline: "not reached" }[state.online];
  $("#srcGithub").title = state.onlineProblem || "";
}

function renderLaptop(items) {
  $("#laptopList").replaceChildren(...items.map((item) =>
    el("div", { class: "laptop__item" },
      el("span", { class: `cat-dot ${item.found ? "cat-dot--ok" : "cat-dot--warn"}`, "aria-hidden": "true" }),
      el("div", {},
        el("div", { class: "laptop__name" }, item.name),
        el("div", { class: "laptop__detail" }, item.found ? item.detail : "not found"),
        !item.found && el("button", {
          class: "out",
          title: item.url,
          onclick: () => invoke("open_link", { url: item.url }).catch(() => {}),
        }, item.urlLabel, el("span", { html: ICONS.out }))))));
}

// ---------------------------------------------------------------- steps two and three

function runRow(row) {
  return el("div", { class: "run", "data-id": row.id },
    icon(row),
    el("div", { class: "row__text" },
      el("div", { class: "row__name run__name" }, row.name),
      el("div", { class: "run__phase" }),
      el("div", { class: "run__detail", hidden: true })),
    el("div", { class: "run__side" }));
}

function updateRunRow(id) {
  const node = document.querySelector(`.run[data-id="${id}"]`);
  if (!node) return;
  const p = state.run.progress[id];
  if (p) node.dataset.phase = p.phase;

  const dot = p?.phase === "done" && !p.dry ? "cat-dot--ok" : p?.phase === "failed" ? "cat-dot--bad" : null;
  node.querySelector(".run__phase").replaceChildren(
    ...(dot ? [el("span", { class: `cat-dot ${dot}`, "aria-hidden": "true" })] : []),
    el("span", {}, phaseText(p, performance.now())),
  );

  const detail = node.querySelector(".run__detail");
  detail.hidden = !(p?.phase === "done" && p.dry && p.message);
  if (!detail.hidden) detail.textContent = p.message;

  const side = node.querySelector(".run__side");
  const f = fraction(p);
  if (f != null) {
    let fill = side.querySelector(".meter__fill");
    if (!fill) {
      side.replaceChildren(el("div", { class: "meter", role: "progressbar", "aria-valuemin": "0", "aria-valuemax": "100" }, el("div", { class: "meter__fill" })));
      fill = side.querySelector(".meter__fill");
    }
    fill.style.width = `${(f * 100).toFixed(1)}%`;
    fill.parentElement.setAttribute("aria-valuenow", String(Math.round(f * 100)));
  } else if (p?.phase === "done" && !p.dry) {
    if (!side.querySelector(".cat-btn")) {
      side.replaceChildren(el("button", {
        class: "cat-btn",
        onclick: (event) => {
          const button = event.currentTarget;
          invoke("launch_app", { id }).catch((error) => {
            button.replaceWith(el("span", { class: "state state--quiet" }, String(error)));
          });
        },
      }, "Open"));
    }
  } else {
    side.replaceChildren();
  }
}

function renderRunHead() {
  const s = summary(state.run.ids, state.run.progress);
  const done = state.run.complete && s.complete;
  $("#runTitle").textContent = done ? s.title : "Installing";
  $("#runEyebrow").textContent = done ? "FINISHED" : "INSTALLING";
  $("#runLine").textContent = !done
    ? "One app at a time. Each one's own installer does the work; this window waits for it."
    : s.failed
      ? "What failed says why beside it. Fix that, then go back and run it again; what installed stays installed."
      : s.dry
        ? "Files were checked and downloaded; no installer was run and nothing on this laptop changed."
        : "Run this program again any time to update: it installs only what is newer.";
  $("#runActions").hidden = !done;
  if (done) setStep("done");
}

async function startInstall() {
  const ids = state.rows.filter((r) => state.selected.has(r.id) && selectable(r)).map((r) => r.id);
  if (!ids.length) return;
  state.run = { ids, progress: {}, complete: false };
  $("#runList").replaceChildren(...state.rows.filter((r) => ids.includes(r.id)).map(runRow));
  ids.forEach(updateRunRow);
  renderRunHead();
  await showView($("#viewRun"), "install");
  try {
    await invoke("start_install", { ids });
  } catch (error) {
    // The install never began; say so against every app rather than leave them "waiting".
    for (const id of ids) {
      state.run.progress = reduceProgress(state.run.progress, { id, phase: "failed", message: String(error) }, performance.now());
      updateRunRow(id);
    }
    state.run.complete = true;
    renderRunHead();
  }
}

function onProgress(event) {
  if (!state.run) return;
  state.run.progress = reduceProgress(state.run.progress, event.payload, performance.now());
  updateRunRow(event.payload.id);
}

function onFinished(event) {
  if (!state.run) return;
  if (Array.isArray(event.payload) && event.payload.length) state.rows = event.payload;
  state.run.complete = true;
  // An app the backend never mentioned did not install; "waiting" forever would be a lie.
  for (const id of state.run.ids) {
    if (!finished(state.run.progress[id])) {
      state.run.progress = reduceProgress(state.run.progress, { id, phase: "failed", message: "Setup stopped before reaching it." }, performance.now());
    }
    updateRunRow(id);
  }
  renderRunHead();
}

// The one thing that changes with no event: how long an installer has been running.
setInterval(() => {
  if (!state.run || state.run.complete) return;
  for (const id of state.run.ids) {
    if (state.run.progress[id]?.phase === "installing") updateRunRow(id);
  }
}, 500);

async function backToApps() {
  state.run = null;
  state.touched.clear();
  state.selected = new Set(defaultSelection(state.rows));
  renderApps();
  await showView($("#viewApps"), "apps");
}

// ---------------------------------------------------------------- boot

async function boot() {
  $("#tbMark").innerHTML = ICONS.mark;
  $("#tbMin").innerHTML = ICONS.min;
  $("#tbClose").innerHTML = ICONS.close;

  if (!tauri?.core) {
    $("#sourceLine").textContent = "This page is Catalyst Setup's window. Open it by running Catalyst Setup.exe.";
    $("#go").textContent = "Nothing to install";
    return;
  }

  const appWindow = tauri.window?.getCurrentWindow?.();
  $("#tbMin").onclick = () => appWindow?.minimize();
  $("#tbClose").onclick = () => appWindow?.close();
  $("#finish").onclick = () => appWindow?.close();
  $("#back").onclick = backToApps;
  $("#go").onclick = startInstall;

  await tauri.event.listen("setup://progress", onProgress);
  await tauri.event.listen("setup://finished", onFinished);

  const first = await invoke("suite_state");
  state.rows = first.apps;
  state.payload = first.payload;
  state.dryRun = first.dryRun;
  state.selected = new Set(defaultSelection(state.rows));
  $("#tbVersion").textContent = first.version;
  $("#dryChip").hidden = !state.dryRun;
  renderApps();

  invoke("laptop_state").then(renderLaptop).catch(() => {});

  // GitHub is asked after the page is drawn, so a laptop with no network is never kept waiting.
  invoke("check_online").then((answer) => {
    state.online = answer.reached ? "reached" : "offline";
    state.onlineProblem = answer.problem;
    if (answer.apps?.length) state.rows = answer.apps;
    if (state.run) return;                                   // mid-install: the list is not on show
    state.selected = mergeSelection(state.selected, state.touched, state.rows);
    renderApps();
  }).catch(() => {
    state.online = "offline";
    if (!state.run) renderApps();
  });
}

boot();
