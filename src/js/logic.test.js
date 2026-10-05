import { test } from "node:test";
import assert from "node:assert/strict";

import {
  actionLabel, defaultSelection, finished, formatBytes, formatElapsed, fraction, mergeSelection,
  phaseText, reduceProgress, selectable, sourceSentence, sourceText, statusWord, summary, versionsText,
} from "./logic.js";

const row = (id, status, extra = {}) => ({
  id, name: id, default: true, status, downloadable: false, asked: false,
  installed: status === "not-installed" || status === "unavailable" ? null : { version: "1.0.0" },
  offer: status === "unavailable" || status === "installed" ? null : { version: "1.1.0", source: "payload", size: 2048 },
  ...extra,
});

test("bytes read the way Explorer shows them", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(812 * 1024), "812 KB");
  assert.equal(formatBytes(5597472), "5.3 MB");
  assert.equal(formatBytes(49832448), "47.5 MB");
  assert.equal(formatBytes(250 * 1024 * 1024), "250 MB");
  assert.equal(formatBytes(3 * 1024 ** 3), "3.00 GB");
  assert.equal(formatBytes(NaN), "");
});

test("elapsed time is minutes and seconds", () => {
  assert.equal(formatElapsed(0), "0:00");
  assert.equal(formatElapsed(7400), "0:07");
  assert.equal(formatElapsed(125000), "2:05");
  assert.equal(formatElapsed(-5), "0:00");
});

test("a fresh laptop ticks every default app that has an installer, and no optional one", () => {
  const rows = [
    row("catalyst", "not-installed"),
    row("console", "not-installed"),
    row("pit", "unavailable"),
    row("link", "not-installed", { default: false }),
  ];
  assert.deepEqual(defaultSelection(rows), ["catalyst", "console"]);
});

test("an up-to-date app is not reinstalled by default, and a newer one is never downgraded by default", () => {
  const rows = [row("a", "current"), row("b", "update"), row("c", "ahead")];
  assert.deepEqual(defaultSelection(rows), ["b"]);
  assert.ok(selectable(rows[0]), "it can still be chosen");
});

test("a late answer from GitHub ticks new rows but never undoes a person's choice", () => {
  const before = [row("catalyst", "unavailable"), row("sim", "not-installed")];
  const selected = new Set(defaultSelection(before));
  const touched = new Set(["sim"]);
  selected.delete("sim");                               // they unticked the Sim
  const after = [row("catalyst", "not-installed"), row("sim", "not-installed")];
  assert.deepEqual([...mergeSelection(selected, touched, after)], ["catalyst"]);
});

test("a row that lost its installer is dropped from the selection", () => {
  const selected = new Set(["a"]);
  assert.deepEqual([...mergeSelection(selected, new Set(["a"]), [row("a", "unavailable")])], []);
});

test("the button says what it will do", () => {
  const rows = [row("a", "not-installed"), row("b", "not-installed"), row("c", "update"), row("d", "current")];
  assert.equal(actionLabel(rows, new Set(["a", "b"])), "Install 2");
  assert.equal(actionLabel(rows, new Set(["c"])), "Update 1");
  assert.equal(actionLabel(rows, new Set(["a", "c", "d"])), "Install 1, update 1, reinstall 1");
  assert.equal(actionLabel(rows, new Set(["d"])), "Reinstall 1");
  assert.equal(actionLabel(rows, new Set()), "Nothing to install");
  assert.equal(actionLabel([row("x", "unavailable")], new Set(["x"])), "Nothing to install");
});

test("versions show a change only when there is one", () => {
  assert.deepEqual(versionsText(row("a", "update")), { from: "1.0.0", to: "1.1.0" });
  assert.deepEqual(versionsText(row("a", "current", { offer: { version: "1.0.0", source: "payload" } })), { from: null, to: "1.0.0" });
  assert.deepEqual(versionsText(row("a", "not-installed")), { from: "none", to: "1.1.0" });
  assert.deepEqual(versionsText(row("a", "installed")), { from: null, to: "1.0.0" });
  assert.deepEqual(versionsText(row("a", "unavailable")), { from: null, to: null });
  assert.deepEqual(versionsText(row("a", "installed", { installed: { version: null } })), { from: null, to: "installed" });
});

test("status words", () => {
  assert.equal(statusWord(row("a", "current")), "up to date");
  assert.equal(statusWord(row("a", "not-installed")), "not installed");
  assert.equal(statusWord(row("a", "ahead")), "newer than offered");
});

test("the source says where the installer is, and admits when it is still asking", () => {
  assert.equal(sourceText(row("a", "update"), "reached"), "payload · 2 KB");
  assert.equal(sourceText(row("a", "update", { offer: { version: "1", source: "github", size: null } }), "reached"), "github");
  assert.equal(sourceText(row("a", "unavailable", { downloadable: true }), "checking"), "asking github");
  assert.equal(sourceText(row("a", "unavailable", { downloadable: true, asked: true }), "offline"), "");
  assert.equal(sourceText(row("a", "unavailable"), "checking"), "", "an app with no download is never 'asking'");
});

test("the sentence under the title counts real sources", () => {
  const payload = { present: true, problem: null };
  const rows = [row("a", "update"), row("b", "update"), row("c", "update", { offer: { version: "2", source: "github" } })];
  assert.equal(sourceSentence(payload, "reached", rows), "2 from the payload folder beside this program, 1 from GitHub.");
  assert.match(sourceSentence({ present: false, problem: null }, "offline", [row("a", "unavailable")]), /nothing to install from/);
  assert.match(sourceSentence({ present: false, problem: null }, "checking", [row("a", "unavailable")]), /Asking GitHub/);
  assert.equal(sourceSentence({ present: true, problem: "manifest.json could not be read" }, "reached", rows), "manifest.json could not be read");
});

test("progress folds event by event and keeps the start of a phase", () => {
  let s = {};
  s = reduceProgress(s, { id: "a", phase: "downloading", received: 10, total: 100, version: "2.0.0" }, 1000);
  s = reduceProgress(s, { id: "a", phase: "downloading", received: 60, total: 100 }, 1500);
  assert.equal(s.a.started, 1000);
  assert.equal(s.a.received, 60);
  assert.equal(s.a.version, "2.0.0", "the version is remembered from the first event");
  s = reduceProgress(s, { id: "a", phase: "installing" }, 2000);
  assert.equal(s.a.started, 2000);
  assert.equal(s.b, undefined);
});

test("a bar exists only while bytes are being counted against a known total", () => {
  assert.equal(fraction({ phase: "downloading", received: 50, total: 200 }), 0.25);
  assert.equal(fraction({ phase: "verifying", received: 300, total: 200 }), 1);
  assert.equal(fraction({ phase: "downloading", received: 50, total: null }), null);
  assert.equal(fraction({ phase: "installing", received: 0, total: null }), null, "an installer reports nothing, so nothing is drawn");
  assert.equal(fraction({ phase: "done", received: 0, total: null }), null);
  assert.equal(fraction(undefined), null);
});

test("phase text says only what is known", () => {
  assert.equal(phaseText(undefined, 0), "waiting");
  assert.equal(phaseText({ phase: "downloading", received: 0, total: null }, 0), "connecting to github");
  assert.equal(phaseText({ phase: "downloading", received: 1048576, total: 5597472 }, 0), "downloading · 1.0 MB of 5.3 MB");
  assert.equal(phaseText({ phase: "downloading", received: 1048576, total: null }, 0), "downloading · 1.0 MB");
  assert.equal(phaseText({ phase: "verifying", received: 0, total: 49832448 }, 0), "checking the file · 0 B of 47.5 MB");
  assert.equal(phaseText({ phase: "installing", started: 1000 }, 8400), "running its installer · 0:07");
  assert.equal(phaseText({ phase: "done", version: "2.8.0" }, 0), "installed 2.8.0");
  assert.equal(phaseText({ phase: "done", version: "2.8.0", dry: true }, 0), "dry run · nothing was installed");
  assert.equal(phaseText({ phase: "failed", message: "Catalyst is open. Close it, then run setup again." }, 0), "Catalyst is open. Close it, then run setup again.");
});

test("the summary counts, and a dry run is never called an install", () => {
  const ids = ["a", "b", "c"];
  assert.equal(summary(ids, { a: { phase: "done" } }).title, "Installing");
  assert.ok(!finished(undefined));
  const all = { a: { phase: "done" }, b: { phase: "done" }, c: { phase: "done" } };
  assert.equal(summary(ids, all).title, "3 apps installed");
  assert.equal(summary(["a"], all).title, "1 app installed");
  const mixed = { ...all, c: { phase: "failed", message: "x" } };
  assert.deepEqual(summary(ids, mixed), { done: 2, dry: 0, failed: 1, complete: true, title: "2 installed, 1 failed" });
  const none = { a: { phase: "failed" }, b: { phase: "failed" }, c: { phase: "failed" } };
  assert.equal(summary(ids, none).title, "Nothing was installed");
  assert.equal(summary(["a"], none).title, "It did not install");
  const dry = { a: { phase: "done", dry: true }, b: { phase: "done", dry: true }, c: { phase: "failed" } };
  assert.equal(summary(ids, dry).title, "Dry run finished, 1 failed");
  assert.equal(summary(["a", "b"], dry).title, "Dry run finished");
  assert.equal(summary(["a", "b"], dry).done, 0);
});
