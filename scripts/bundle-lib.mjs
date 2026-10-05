// The parts of make-bundle that decide things, kept apart from the parts that touch the disk and
// the network so `node --test` can check them.

import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";

/** "Catalyst Pit_0.3.1_x64-setup.exe" -> "0.3.1". Null when the name carries no version. */
export function versionFromSetupName(name) {
  const m = /_(\d+(?:\.\d+)*(?:-[0-9A-Za-z.]+)?)_[^_]*-setup\.exe$/i.exec(name);
  return m ? m[1] : null;
}

/** Numeric compare, with a release above its own pre-releases. Mirrors src-tauri/src/version.rs. */
export function compareVersions(a, b) {
  const split = (v) => {
    const [core, pre = ""] = String(v).replace(/^[^\d]*/, "").split("+")[0].split(/-(.*)/s);
    return { nums: core.split(".").map(Number), pre: pre ? pre.split(".") : [] };
  };
  const x = split(a), y = split(b);
  for (let i = 0; i < Math.max(x.nums.length, y.nums.length); i++) {
    const d = (x.nums[i] || 0) - (y.nums[i] || 0);
    if (d) return Math.sign(d);
  }
  // A release is newer than any pre-release of the same numbers.
  if (!x.pre.length || !y.pre.length) return Math.sign(y.pre.length - x.pre.length);
  for (let i = 0; i < Math.min(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i], q = y.pre[i];
    const pn = /^\d+$/.test(p), qn = /^\d+$/.test(q);
    const d = pn && qn ? Number(p) - Number(q) : pn ? -1 : qn ? 1 : p < q ? -1 : p > q ? 1 : 0;
    if (d) return Math.sign(d);
  }
  return Math.sign(x.pre.length - y.pre.length);
}

/** The newest installer in a Tauri `bundle/nsis` folder listing, or null. */
export function newestSetup(names) {
  const found = names
    .filter((n) => /-setup\.exe$/i.test(n))
    .map((name) => ({ name, version: versionFromSetupName(name) }))
    .filter((f) => f.version);
  if (!found.length) return null;
  return found.sort((a, b) => compareVersions(b.version, a.version))[0];
}

/** "1.4.0.0" (a Windows file version) -> "1.4.0". */
export function trimFileVersion(v) {
  const parts = String(v).trim().split(".");
  while (parts.length > 3 && parts[parts.length - 1] === "0") parts.pop();
  return parts.join(".");
}

/** The manifest Catalyst Setup reads: src-tauri/src/manifest.rs is the other half of this shape. */
export function buildManifest(items, built = new Date().toISOString()) {
  return {
    schema: 1,
    built,
    items: items.map(({ id, version, file, sha256, size }) => ({ id, version, file, sha256, size })),
  };
}

export function sha256File(path) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    createReadStream(path).on("data", (d) => hash.update(d)).on("error", reject).on("end", () => resolve(hash.digest("hex")));
  });
}

/**
 * The text of a release: what the zip holds, and what it could not hold and why. A release must
 * not let anyone think the stick is complete when it is not.
 */
export function releaseNotes(version, items, skipped) {
  const lines = [
    `Catalyst Setup ${version}: one installer for every Catalyst app.`,
    "",
    "- `catalyst-setup.exe` is the program alone. It downloads what it installs, so it needs a network.",
    "- `CatalystSuite.zip` is the program with a `payload` folder beside it, for a USB stick. It installs with no network.",
    "",
    items.length ? "In the zip's payload:" : "The zip's payload is empty: no app's installer could be fetched when this was built.",
    ...items.map((i) => `- ${i.name} ${i.version}`),
  ];
  if (skipped.length) {
    lines.push("", "Not in the zip:", ...skipped.map((s) => `- ${s.name}: ${s.why}.`));
    lines.push("", "For a complete stick, build one on a machine that has those installers: `npm run build`, then `npm run bundle`.");
  }
  lines.push("", "Laptops with \"Keep these apps up to date\" turned on move to this version of Catalyst Setup by themselves.", "");
  return lines.join("\n");
}
