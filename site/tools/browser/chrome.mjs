// Launch headless Chrome with --remote-debugging-port=0 and take the real
// port from the "DevTools listening on ws://…" stderr line, so parallel
// runs can't collide on a guessed port.
import { spawn } from "node:child_process";

export async function launchChrome(bin, profileDir, { size = "1440x900", timeoutMs = 20000 } = {}) {
  const proc = spawn(bin, [
    "--headless=new", "--remote-debugging-port=0", "--no-sandbox",
    "--no-first-run", "--no-default-browser-check", "--disable-gpu",
    "--hide-scrollbars", "--no-proxy-server",
    `--window-size=${size}`, "--force-device-scale-factor=1",
    `--user-data-dir=${profileDir}`, "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"] });

  const wsUrl = await new Promise((found, fail) => {
    let text = "";
    const timer = setTimeout(() => fail(new Error(`chrome printed no DevTools line in ${timeoutMs}ms: ${text.slice(0, 300)}`)), timeoutMs);
    proc.stderr.on("data", (chunk) => {
      text += chunk;
      const m = text.match(/DevTools listening on (ws:\/\/\S+)/);
      if (m) { clearTimeout(timer); found(m[1]); }
    });
    proc.on("exit", (code) => {
      clearTimeout(timer);
      fail(new Error(`chrome exited (${code}) before DevTools: ${text.slice(0, 300)}`));
    });
  });
  return { proc, wsUrl };
}

// The page target for the about:blank tab chrome opened.
export async function pageTarget(wsUrl) {
  const origin = wsUrl.match(/^(ws:\/\/[^/]+)/)[1].replace("ws://", "http://");
  const until = Date.now() + 15000;
  for (;;) {
    try {
      const list = await (await fetch(`${origin}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page;
    } catch { /* chrome not serving /json yet */ }
    if (Date.now() > until) throw new Error("no CDP page target");
    await new Promise((go) => setTimeout(go, 200));
  }
}

// A dedicated fresh tab per isolated scenario: new target, new CDP
// session, no leftover emulation; storage is cleared before it closes.
export function httpBase(wsUrl) {
  return wsUrl.match(/^(ws:\/\/[^/]+)/)[1].replace("ws://", "http://");
}
export async function createTarget(base) {
  const res = await fetch(`${base}/json/new?url=about:blank`, { method: "PUT" });
  if (!res.ok) throw new Error(`Target.createTarget: HTTP ${res.status}`);
  return res.json();
}
export async function closeTarget(base, id) {
  try { await fetch(`${base}/json/close/${id}`); } catch { /* already gone */ }
}
