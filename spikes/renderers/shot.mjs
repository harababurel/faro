// M13 Phase 0 spike (throwaway): screenshot a page in headless Chrome over CDP.
// node shot.mjs <url> <out.png> [width height dpr]   (prints window.stats)
import { spawn } from "node:child_process";
import { writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [url, out, width = "1040", height = "600", dpr = "2"] = process.argv.slice(2);
const port = 9300 + Math.floor(Math.random() * 500);
setTimeout(() => { console.log("TIMEOUT"); process.exit(2); }, 60000);
const chrome = spawn(process.env.CHROME ?? "/opt/google/chrome/chrome", [
  "--headless=new", `--remote-debugging-port=${port}`, "--no-first-run", "--no-default-browser-check",
  "--enable-unsafe-swiftshader", "--use-angle=swiftshader", "--hide-scrollbars", "--no-proxy-server", "--disable-features=LocalNetworkAccessChecks,PrivateNetworkAccessSendPreflights,PrivateNetworkAccessRespectPreflightResults", "--no-sandbox", `--window-size=${width},${height}`, `--force-device-scale-factor=${dpr}`,
  `--user-data-dir=${mkdtempSync(join(tmpdir(), "faro-shot-"))}`, "about:blank",
], { stdio: "ignore" });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let target;
for (let i = 0; i < 50 && !target; i++) {
  await sleep(200);
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    target = list.find((t) => t.type === "page");
  } catch {}
}
console.error("target", target?.url);
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let id = 0;
const pending = new Map();
const logs = [];
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  if (m.method === "Runtime.consoleAPICalled") logs.push(m.params.args.map((a) => a.value ?? a.description).join(" "));
  if (m.method === "Runtime.exceptionThrown") logs.push("EXCEPTION " + JSON.stringify(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text));
});
const send = (method, params = {}) => new Promise((r) => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); setTimeout(() => { if (pending.has(i)) { console.error("no reply to", method); pending.delete(i); r({}); } }, 10000); });
console.error("ws open");
await send("Runtime.enable");
console.error("runtime enabled");
await send("Page.navigate", { url });
console.error("navigated");
let stats = null;
for (let i = 0; i < 150; i++) {
  await sleep(200);
  const r = await send("Runtime.evaluate", { expression: "window.done ? JSON.stringify(window.stats ?? {}) : null", returnByValue: true });
  if (r.result?.result?.value) { stats = r.result.result.value; break; }
}
console.error("stats", stats, logs);
await sleep(300);
const shot = await send("Page.captureScreenshot", { format: "png" });
writeFileSync(out, Buffer.from(shot.result.data, "base64"));
console.log(JSON.stringify({ out, stats, logs: logs.slice(0, 10) }));
ws.close();
chrome.kill();
process.exit(0);
