// Drive the web demo in headless Chrome over CDP: open a page, wait for
// `window.done`, optionally type, click and wait, and take screenshots.
//
//   node crates/faro-web/tools/shot.mjs <url> <out.png> [script.json] \
//        [--size 1280x760] [--dpr 2]
//
// `CHROME` names the browser (default /opt/google/chrome/chrome); headless
// shell builds work too. The script is a list of steps:
//   {"keys": "hello"}                 text, one key per character
//   {"key": "Enter", "mods": ["ctrl"]} one key ("Enter", "Escape", "Tab",
//                                      "ArrowUp", "a", …), with modifiers
//   {"click": [x, y]}                 a click at CSS pixels
//   {"wheel": [x, y, dy]}             a wheel turn
//   {"wait": 500}                     milliseconds
//   {"shot": "name"}                  a screenshot to <out>-name.png
//   {"eval": "expression"}            run JS; its JSON result is printed
// It prints a JSON report: window.stats, console output and eval results.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
const flag = (name, def) => {
  const i = args.indexOf(name);
  if (i < 0) return def;
  const [v] = args.splice(i, 2).slice(1);
  return v;
};
const [width, height] = flag("--size", "1280x760").split("x").map(Number);
const dpr = Number(flag("--dpr", "2"));
const [url, out, scriptPath] = args;
const script = scriptPath ? JSON.parse(readFileSync(scriptPath, "utf8")) : [];

const port = 9300 + Math.floor(Math.random() * 500);
const timer = setTimeout(() => { console.log(JSON.stringify({ error: "timeout" })); process.exit(2); }, 120000);
const chrome = spawn(process.env.CHROME ?? "/opt/google/chrome/chrome", [
  "--headless=new", `--remote-debugging-port=${port}`, "--no-first-run", "--no-default-browser-check",
  "--hide-scrollbars", "--no-proxy-server", "--no-sandbox", "--allow-file-access-from-files",
  `--window-size=${width},${height}`, `--force-device-scale-factor=${dpr}`,
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
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let id = 0;
const pending = new Map();
const logs = [];
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
  if (m.method === "Runtime.consoleAPICalled") logs.push(m.params.args.map((a) => a.value ?? a.description).join(" "));
  if (m.method === "Runtime.exceptionThrown") logs.push("EXCEPTION " + (m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text));
});
const send = (method, params = {}) => new Promise((r) => {
  const i = ++id;
  pending.set(i, r);
  ws.send(JSON.stringify({ id: i, method, params }));
});
const evaluate = async (expression) =>
  (await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true })).result?.result?.value;

await send("Runtime.enable");
await send("Page.navigate", { url });
let stats = null;
for (let i = 0; i < 300 && stats === null; i++) {
  await sleep(200);
  stats = await evaluate("window.done ? JSON.stringify(window.stats ?? {}) : null") ?? null;
}

const KEYS = {
  Enter: [13, "\r"], Escape: [27], Tab: [9], Backspace: [8], Delete: [46], Space: [32, " "],
  ArrowUp: [38], ArrowDown: [40], ArrowLeft: [37], ArrowRight: [39],
  PageUp: [33], PageDown: [34], Home: [36], End: [35],
};
const MODS = { alt: 1, ctrl: 2, meta: 4, shift: 8 };
async function key(name, mods = []) {
  const modifiers = mods.reduce((m, k) => m | MODS[k], 0);
  const [code, text] = KEYS[name] ?? [name.toUpperCase().charCodeAt(0), name];
  const char = text && !(modifiers & (MODS.ctrl | MODS.alt | MODS.meta)) ? text : undefined;
  const codeName = name.length === 1 ? (/[a-z]/i.test(name) ? `Key${name.toUpperCase()}` : /[0-9]/.test(name) ? `Digit${name}` : "") : name;
  const base = { modifiers, key: name === "Space" ? " " : name, code: codeName, windowsVirtualKeyCode: code };
  await send("Input.dispatchKeyEvent", { type: char ? "keyDown" : "rawKeyDown", text: char, ...base });
  await send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}

const results = [];
for (const step of script) {
  if (step.keys !== undefined) for (const c of step.keys) await key(c === " " ? "Space" : c);
  if (step.key) await key(step.key, step.mods);
  if (step.click) {
    const [x, y] = step.click;
    for (const type of ["mousePressed", "mouseReleased"]) {
      await send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
    }
  }
  if (step.wheel) {
    const [x, y, dy] = step.wheel;
    await send("Input.dispatchMouseEvent", { type: "mouseWheel", x, y, deltaX: 0, deltaY: dy });
  }
  if (step.wait) await sleep(step.wait);
  if (step.eval) results.push(await evaluate(step.eval));
  if (step.shot) {
    await sleep(150);
    const shot = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(out.replace(/\.png$/, `-${step.shot}.png`), Buffer.from(shot.result.data, "base64"));
  }
}
await sleep(300);
const shot = await send("Page.captureScreenshot", { format: "png" });
writeFileSync(out, Buffer.from(shot.result.data, "base64"));
console.log(JSON.stringify({ out, stats: stats && JSON.parse(stats), results, logs: logs.slice(0, 20) }));
clearTimeout(timer);
ws.close();
chrome.kill();
process.exit(0);
