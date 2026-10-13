// Serves a built getfaro.dev root like nginx does (WEBSITE.md §6.10): the
// security headers are parsed from site/deploy/nginx (never eval'd), hashed
// assets get immutable caching. /js/s.js (the proxied Plausible script) is
// replaced by a recording stub that never fetches, /api/event is recorded
// and answered with 204, so the run proves no real telemetry leaves.
// Every request is logged (url, cookie header, body) for the leak audit.
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { join, sep } from "node:path";

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json",
  ".wasm": "application/wasm",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".webp": "image/webp",
  ".woff2": "font/woff2",
  ".ico": "image/x-icon",
  ".txt": "text/plain; charset=utf-8",
};
const HASHED = /\.[0-9a-f]{8,}\.(js|css|woff2|wasm|svg|png|webp|jpg)$/;

// The stub stands in for Plausible's script. init.js (the built one, served
// from the built root) queues early calls in plausible.q; both are defer,
// in order init → s.js → demo.js, so the recorder is installed before any
// demo event and flushes whatever init already queued.
const STUB_S = `(function () {
  var cap = (window.__plausibleCaptured = window.__plausibleCaptured || []);
  function record(name, opts) {
    cap.push({ name: String(name), props: (opts && opts.props) || null });
  }
  var q = (window.plausible && window.plausible.q) || [];
  window.plausible = function (name, opts) { record(name, opts); };
  window.plausible.q = [];
  window.plausible.init = window.plausible.init || function () {};
  for (var i = 0; i < q.length; i++) record(q[i][0], q[i][1]);
})();`;

// Parse the exact CSP and the other always-on headers from the nginx files.
// Quoted values keep spaces and semicolons (HSTS, Permissions-Policy);
// every add_header line must parse — a silently dropped header would let
// the harness test weaker headers than nginx actually sends.
export function parseAddHeaders(snippet, cspValue) {
  const headers = [];
  for (const line of snippet.split("\n")) {
    const text = line.trim();
    if (!text.startsWith("add_header")) continue;
    const m = text.match(/^add_header\s+([A-Za-z-]+)\s+("(?:[^"\\]|\\.)*"|[^"\s;]+)\s+always\s*;$/);
    if (!m) throw new Error(`unparsed add_header line: ${text}`);
    let value = m[2];
    if (value.startsWith('"')) value = JSON.parse(value);
    else if (value === "$csp") value = cspValue;
    headers.push([m[1], value]);
  }
  if (headers.length === 0) throw new Error("the headers snippet has no add_header lines");
  for (const need of ["Content-Security-Policy", "Strict-Transport-Security", "Permissions-Policy"]) {
    if (!headers.some(([name]) => name === need)) {
      throw new Error(`the headers snippet is missing add_header ${need}`);
    }
  }
  return headers;
}

export function nginxHeaders(nginxDir) {
  const conf = readFileSync(join(nginxDir, "getfaro.dev.conf"), "utf8");
  const csp = conf.match(/^\s*set\s+\$csp\s+"([^"]+)"\s*;/m);
  if (!csp) throw new Error(`no set $csp "..." found in ${join(nginxDir, "getfaro.dev.conf")}`);
  const snippet = readFileSync(join(nginxDir, "snippets", "getfaro-headers.conf"), "utf8");
  return parseAddHeaders(snippet, csp[1]);
}

export async function startServer(root, headers, { manifestVariant = null } = {}) {
  // Test-only switch: serve the built manifest.no-preload.json (the file
  // nginx's emergency switch would serve, §6.8) at /demo/manifest.json.
  const manifestFile = manifestVariant === "no-preload"
    ? "/demo/manifest.no-preload.json"
    : "/demo/manifest.json";
  const requests = []; // {method, url, status, cookie, body, bodyTruncated}
  const server = createServer((req, res) => {
    const entry = {
      method: req.method,
      url: req.url,
      status: 0,
      cookie: req.headers.cookie ?? null,
      body: "",
      bodyTruncated: false,
    };
    requests.push(entry);
    // Record every request body, bounded: Buffer chunks, decoded once.
    // GET/HEAD have no body, so the listeners resolve empty without
    // delaying the response.
    const chunks = [];
    let seen = 0;
    req.on("data", (chunk) => {
      seen += chunk.length;
      if (seen <= 16384) chunks.push(chunk);
      else entry.bodyTruncated = true;
    });
    req.on("end", () => { entry.body = Buffer.concat(chunks).toString("utf8"); });
    const path = new URL(req.url, "http://127.0.0.1").pathname;
    const finish = (status, body, type, cache) => {
      entry.status = status;
      const out = { "Content-Type": type };
      for (const [name, value] of headers) out[name] = value;
      if (cache) out["Cache-Control"] = cache;
      res.writeHead(status, out);
      res.end(body);
    };
    if (path === "/api/event") {
      finish(204, "", "text/plain");
      return;
    }
    if (path === "/js/s.js") {
      finish(200, STUB_S, MIME[".js"], "no-cache");
      return;
    }
    if (path === "/favicon.ico") {
      finish(204, "", "text/plain", "no-cache");
      return;
    }
    let file = path === "/" ? "/index.html" : path;
    if (path === "/demo/manifest.json") file = manifestFile;
    const full = join(root, file);
    if (!full.startsWith(root + sep)) {
      finish(404, "not found", MIME[".txt"], "no-cache");
      return;
    }
    readFile(full)
      .then((data) => {
        const cache = HASHED.test(file)
          ? "public, max-age=31536000, immutable"
          : "no-cache";
        finish(200, data, MIME[file.slice(file.lastIndexOf("."))] ?? "application/octet-stream", cache);
      })
      .catch(() => finish(404, "not found", MIME[".txt"], "no-cache"));
  });
  await new Promise((ready) => server.listen(0, "127.0.0.1", ready));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const close = () => new Promise((gone) => server.close(gone));
  return { origin, requests, close };
}
