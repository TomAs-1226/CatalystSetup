// The window's decisions, with no DOM in them, so `node --test` can hold them to account.
//
// The Rust side measures; this file only words what it measured. Nothing here may invent a state:
// a row with no number shows no number, and a phase with no total shows no bar.

/** 5597472 -> "5.3 MB". Binary units, as Explorer shows them. */
export function formatBytes(n) {
  if (!Number.isFinite(n) || n < 0) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  const mb = n / (1024 * 1024);
  if (mb < 1024) return `${mb < 100 ? mb.toFixed(1) : Math.round(mb)} MB`;
  return `${(mb / 1024).toFixed(2)} GB`;
}

/** 7400 ms -> "0:07". */
export function formatElapsed(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

const STATUS_WORDS = {
  "not-installed": "not installed",
  update: "update",
  current: "up to date",
  ahead: "newer than offered",
  installed: "installed",
  unavailable: "not available",
};

export function statusWord(row) {
  return STATUS_WORDS[row.status] || row.status;
}

/** Can this row be installed at all right now? Only with an installer to run. */
export function selectable(row) {
  return Boolean(row.offer);
}

/**
 * What a first run ticks: every default app that would change something. An app that is already
 * up to date is left alone — reinstalling it is a choice, not a default — and an optional app is
 * never assumed.
 */
export function defaultSelection(rows) {
  return rows
    .filter((r) => r.default && selectable(r) && (r.status === "not-installed" || r.status === "update"))
    .map((r) => r.id);
}

/**
 * Keep a person's ticks when the rows are refreshed (GitHub answering late), and tick what the
 * refresh newly made worth installing — but only rows they have not touched.
 */
export function mergeSelection(selected, touched, rows) {
  const out = new Set();
  const wanted = new Set(defaultSelection(rows));
  for (const row of rows) {
    if (!selectable(row)) continue;
    if (touched.has(row.id) ? selected.has(row.id) : wanted.has(row.id)) out.add(row.id);
  }
  return out;
}

/** The primary button's label: what pressing it will do, counted. */
export function actionLabel(rows, selected) {
  const chosen = rows.filter((r) => selected.has(r.id) && selectable(r));
  if (!chosen.length) return "Nothing to install";
  const installs = chosen.filter((r) => r.status === "not-installed").length;
  const updates = chosen.filter((r) => r.status === "update").length;
  const again = chosen.length - installs - updates;
  const parts = [];
  if (installs) parts.push(`Install ${installs}`);
  if (updates) parts.push(`${parts.length ? "update" : "Update"} ${updates}`);
  if (again) parts.push(`${parts.length ? "reinstall" : "Reinstall"} ${again}`);
  return parts.join(", ");
}

/** "2.7.0 → 2.8.0", "2.8.0", "— → 2.8.0": the two versions a row is about. */
export function versionsText(row) {
  const have = row.installed ? row.installed.version || "installed" : null;
  const offer = row.offer?.version || null;
  if (have && offer && row.status !== "current") return { from: have, to: offer };
  if (have) return { from: null, to: have };
  if (offer) return { from: "none", to: offer };
  return { from: null, to: null };
}

/** Where the installer would come from, in three words or fewer. */
export function sourceText(row, online) {
  if (row.offer) {
    const size = row.offer.size != null ? ` · ${formatBytes(row.offer.size)}` : "";
    return `${row.offer.source}${size}`;
  }
  if (row.downloadable && !row.asked && online === "checking") return "asking github";
  return "";
}

/** One sentence under the title about where installers are coming from. True, or not said. */
export function sourceSentence(payload, online, rows) {
  const fromPayload = rows.filter((r) => r.offer?.source === "payload").length;
  const fromGithub = rows.filter((r) => r.offer?.source === "github").length;
  if (payload.problem) return payload.problem;
  const parts = [];
  if (fromPayload) parts.push(`${fromPayload} from the payload folder beside this program`);
  if (fromGithub) parts.push(`${fromGithub} from GitHub`);
  if (parts.length) return `${parts.join(", ")}.`;
  if (online === "checking") return payload.present ? "Reading the payload folder." : "No payload folder beside this program. Asking GitHub.";
  if (!payload.present) return "No payload folder beside this program, and GitHub cannot be reached. There is nothing to install from.";
  return "Nothing in the payload folder can be installed.";
}

// ---------------------------------------------------------------- keep up to date

/**
 * The switch, from what the laptop really has. "On" means the scheduled task exists; a laptop
 * that has never been asked shows the switch set, and says plainly that nothing is set up yet.
 */
export function keepView(keep, canInstall = true) {
  if (!keep) return { checked: false, pending: false, line: "" };
  if (keep.registered) {
    return { checked: true, pending: false, line: `On. Checks when you sign in, and every ${keep.everyHours} hours after.` };
  }
  if (keep.preference === false) {
    return { checked: false, pending: false, line: "Off. Apps change only when you run this program." };
  }
  // With nothing to install there is no button to press, so the line must not point at one.
  const how = canInstall ? "It turns on when you press the button above." : "Nothing needs installing, so turn it on here.";
  return { checked: true, pending: true, line: `Not set up yet. ${how}` };
}

const list = (parts) => (parts.length <= 1 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`);

/** What the last background run did, as the end of "Last checked <when>: ...". Null if there was none. */
export function lastRunText(report) {
  if (!report) return null;
  if (report.outcome === "driver-station") return "the Driver Station was open, so nothing was checked";
  if (report.outcome === "offline") return "GitHub could not be reached";
  if (report.outcome === "rate-limited") return "GitHub's hourly limit for this network was used up";
  const verb = report.dry ? "would have updated" : "updated";
  const parts = [];
  const changes = [...(report.updated || []), ...(report.setup ? [{ ...report.setup, name: "itself" }] : [])];
  if (changes.length) parts.push(`${verb} ${list(changes.map((c) => `${c.name} to ${c.to}`))}`);
  for (const n of report.waiting || []) parts.push(`${n.name} ${n.message.replace(/\.$/, "")}`);
  for (const n of report.failed || []) parts.push(`${n.name} failed: ${n.message.replace(/\.$/, "")}`);
  return parts.length ? parts.join("; ") : "everything installed was up to date";
}

// ---------------------------------------------------------------- the install

/** Fold one progress event into what is known about each app. `now` stamps the phase's start. */
export function reduceProgress(state, event, now) {
  const before = state[event.id];
  const started = before && before.phase === event.phase ? before.started : now;
  return {
    ...state,
    [event.id]: {
      phase: event.phase,
      received: event.received || 0,
      total: event.total ?? null,
      message: event.message || null,
      version: event.version || before?.version || null,
      dry: Boolean(event.dry),
      started,
    },
  };
}

/** 0..1 while there is something to measure, otherwise null: no bar is better than a made-up one. */
export function fraction(p) {
  if (!p || (p.phase !== "downloading" && p.phase !== "verifying")) return null;
  if (!p.total) return null;
  return Math.max(0, Math.min(1, p.received / p.total));
}

/** The line under an app's name while it installs. */
export function phaseText(p, now) {
  if (!p) return "waiting";
  const bytes = p.total ? `${formatBytes(p.received)} of ${formatBytes(p.total)}` : p.received ? formatBytes(p.received) : "";
  switch (p.phase) {
    case "verifying": return bytes ? `checking the file · ${bytes}` : "checking the file";
    case "downloading": return bytes ? `downloading · ${bytes}` : "connecting to github";
    case "installing": return `running its installer · ${formatElapsed(now - p.started)}`;
    case "done": return p.dry ? "dry run · nothing was installed" : p.version ? `installed ${p.version}` : "installed";
    case "failed": return p.message || "failed";
    default: return p.phase;
  }
}

export function finished(p) {
  return Boolean(p) && (p.phase === "done" || p.phase === "failed");
}

/** The finish page's title and counts. */
export function summary(ids, progress) {
  const done = ids.filter((id) => progress[id]?.phase === "done" && !progress[id].dry).length;
  const dry = ids.filter((id) => progress[id]?.phase === "done" && progress[id].dry).length;
  const failed = ids.filter((id) => progress[id]?.phase === "failed").length;
  const complete = ids.every((id) => finished(progress[id]));
  let title;
  if (!complete) title = "Installing";
  else if (dry && !done) title = failed ? `Dry run finished, ${failed} failed` : "Dry run finished";
  else if (!failed) title = done === 1 ? "1 app installed" : `${done} apps installed`;
  else if (!done) title = failed === 1 ? "It did not install" : "Nothing was installed";
  else title = `${done} installed, ${failed} failed`;
  return { done, dry, failed, complete, title };
}
