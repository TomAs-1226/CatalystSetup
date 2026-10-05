// Serve the REAL frontend with a scripted Tauri behind it (scripts/fake-tauri.js).
//
//   node scripts/harness.mjs [port]            then open http://localhost:5310/?scenario=fresh
//
// It serves `src/` itself, not a copy: a copy goes stale the moment the app is edited. Outside the
// desktop app every command would reject, so this is the way to look at the window in a browser.
// `scripts/shots.mjs` uses the same server to take the screenshots.

import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { dirname, extname, join, normalize, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const src = resolve(here, "..", "src");

const TYPES = {
  ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml", ".png": "image/png",
};

export function serve(port = 5310) {
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, "http://localhost");
    try {
      if (url.pathname === "/__fake-tauri.js") {
        res.writeHead(200, { "content-type": TYPES[".js"], "cache-control": "no-store" });
        return res.end(await readFile(join(here, "fake-tauri.js")));
      }
      let path = join(src, normalize(decodeURIComponent(url.pathname)).replace(/^[\\/]+/, ""));
      if (!resolve(path).startsWith(src)) { res.writeHead(403).end("outside src"); return; }
      let info = await stat(path).catch(() => null);
      if (info?.isDirectory()) { path = join(path, "index.html"); info = await stat(path).catch(() => null); }
      if (!info) { res.writeHead(404).end("not found: " + url.pathname); return; }
      if (path.endsWith(`${sep}index.html`)) {
        // The stub must exist before app.js evaluates, so it goes in the head as a classic script.
        const html = (await readFile(path, "utf8")).replace("</head>", '  <script src="/__fake-tauri.js"></script>\n</head>');
        res.writeHead(200, { "content-type": TYPES[".html"], "cache-control": "no-store" });
        return res.end(html);
      }
      res.writeHead(200, { "content-type": TYPES[extname(path).toLowerCase()] || "application/octet-stream", "cache-control": "no-store" });
      res.end(await readFile(path));
    } catch (e) {
      res.writeHead(500, { "content-type": "text/plain" }).end(String(e));
    }
  });
  return new Promise((ok) => server.listen(port, () => ok(server)));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const port = Number(process.argv[2] || 5310);
  await serve(port);
  console.log(`harness: ${src}`);
  console.log(`http://localhost:${port}/?scenario=mixed      (fresh | mixed | empty; &run=downloading | installing | done | failed | dry)`);
}
