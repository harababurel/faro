// Record faro's web demo as frames, on a virtual clock, from a storyboard.
//
//   node tools/demo_gif/record.mjs [--html crates/faro-web/www/dist/demo.html]
//        [--story tools/demo_gif/storyboard.jsonc] [--out target/demo-gif/frames]
//        [--cols 120] [--rows 34] [--font 14] [--dpr 2] [--fps 12] [--dry] [--verbose]
//
// `CHROME` names a chrome-headless-shell binary (full Chrome has no
// HeadlessExperimental.beginFrame). Nothing here depends on how fast the
// machine is: Chrome runs on virtual time (pinned start, UTC), the recorder
// advances it by exactly one frame at a time and asks for each frame with
// HeadlessExperimental.beginFrame, so the same build and storyboard give the
// same frames.
//
// Output: unique frames as PNGs plus `frames.ffconcat` (each frame with how
// long it stays on screen) for ffmpeg, and `still-<name>.png` for each
// `still` step. tools/demo_gif/README.md has the storyboard steps.
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const args = process.argv.slice(2);
const flag = (name, def) => {
  const i = args.indexOf(name);
  return i < 0 ? def : args[i + 1];
};
const html = resolve(flag("--html", "crates/faro-web/www/dist/demo.html"));
const storyPath = flag("--story", "tools/demo_gif/storyboard.jsonc");
const out = resolve(flag("--out", "target/demo-gif/frames"));
const cols = Number(flag("--cols", "120"));
const rows = Number(flag("--rows", "34"));
const font = Number(flag("--font", "14"));
const dpr = Number(flag("--dpr", "2"));
const fps = Number(flag("--fps", "12"));
const verbose = args.includes("--verbose");
// Run the storyboard without taking pictures: for timing it quickly.
const dry = args.includes("--dry");
const frameMs = 1000 / fps;
// 2026-01-01 09:41 UTC: the mock's deterministic epoch plus a morning hour.
const START = 1767225600 + 9 * 3600 + 41 * 60;

// The storyboard is JSON with `//` line comments.
const story = JSON.parse(
  readFileSync(storyPath, "utf8")
    .split("\n")
    .filter((line) => !/^\s*\/\//.test(line))
    .join("\n"),
);

const fail = (message) => {
  console.error(`record: ${message}`);
  cleanup();
  process.exit(1);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// --- Chrome over CDP -------------------------------------------------------
const port = 9300 + Math.floor(Math.random() * 500);
const profile = mkdtempSync(join(tmpdir(), "faro-record-"));
if (!process.env.CHROME) {
  console.error("record: set CHROME to a chrome-headless-shell binary (see tools/demo_gif/README.md)");
  process.exit(1);
}
const chrome = spawn(process.env.CHROME, [
  "--headless", "--deterministic-mode", `--remote-debugging-port=${port}`,
  "--no-first-run", "--no-default-browser-check", "--no-sandbox", "--hide-scrollbars",
  "--allow-file-access-from-files", "--font-render-hinting=none", `--user-data-dir=${profile}`,
  // beginFrame screenshots ignore the emulated scale factor; this one counts.
  `--force-device-scale-factor=${dpr}`,
], { stdio: "ignore" });
let ws;
function cleanup() {
  ws?.close();
  chrome.kill();
  rmSync(profile, { recursive: true, force: true });
}

let browserUrl;
for (let i = 0; i < 50 && !browserUrl; i++) {
  await sleep(200);
  try {
    browserUrl = (await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()).webSocketDebuggerUrl;
  } catch {}
}
if (!browserUrl) fail("Chrome didn't start (set CHROME)");
ws = new WebSocket(browserUrl);
await new Promise((r) => ws.addEventListener("open", r, { once: true }));

let lastId = 0;
const pending = new Map();
const waiters = [];
const logs = [];
ws.addEventListener("message", (e) => {
  const m = JSON.parse(e.data);
  if (m.id && pending.has(m.id)) {
    pending.get(m.id)(m);
    pending.delete(m.id);
    return;
  }
  if (m.method === "Runtime.exceptionThrown") {
    logs.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text);
  }
  for (const w of [...waiters]) {
    if (w.method === m.method) {
      waiters.splice(waiters.indexOf(w), 1);
      w.resolve(m.params);
    }
  }
});
// Every call gets a deadline: a stalled Chrome fails the run instead of hanging.
const call = (method, params = {}, sessionId) => new Promise((ok, err) => {
  const id = ++lastId;
  const timer = setTimeout(() => err(new Error(`${method} timed out`)), 15000);
  pending.set(id, (m) => {
    clearTimeout(timer);
    m.error ? err(new Error(`${method}: ${m.error.message}`)) : ok(m.result);
  });
  ws.send(JSON.stringify({ id, method, params, sessionId }));
});
const next = (method) => new Promise((ok, err) => {
  const timer = setTimeout(() => err(new Error(`no ${method}`)), 15000);
  waiters.push({ method, resolve: (p) => { clearTimeout(timer); ok(p); } });
});

const { targetId } = await call("Target.createTarget", {
  url: "about:blank", width: 1600, height: 1000, enableBeginFrameControl: true,
});
const { sessionId } = await call("Target.attachToTarget", { targetId, flatten: true });
const page = (method, params) => call(method, params, sessionId);
const evaluate = async (expression) => {
  const reply = await page("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (reply.exceptionDetails) fail(`page: ${reply.exceptionDetails.exception?.description ?? reply.exceptionDetails.text}`);
  return reply.result?.value;
};

await page("Runtime.enable");
await page("HeadlessExperimental.enable").catch(() => fail("CHROME has no HeadlessExperimental domain: use chrome-headless-shell, not full Chrome"));
await page("Emulation.setTimezoneOverride", { timezoneId: "UTC" });
await page("Emulation.setFocusEmulationEnabled", { enabled: true });
await page("Emulation.setDeviceMetricsOverride", { width: 1600, height: 1000, deviceScaleFactor: dpr, mobile: false });
await page("Emulation.setVirtualTimePolicy", { policy: "pause", initialVirtualTime: START });
// Chrome jitters performance.now() a little (timer coarsening); virtual time
// is whole milliseconds, so rounding takes the jitter out. Without this the
// logo's animation, which reads it, differed by a colour level between runs.
await page("Page.enable");
await page("Page.addScriptToEvaluateOnNewDocument", {
  source: "{ const now = performance.now.bind(performance); performance.now = () => Math.round(now()); }",
});
await page("Page.navigate", { url: `file://${html}?record&cols=${cols}&rows=${rows}&size=${font}` });

// --- The clock and the frames -----------------------------------------------
let now = 0; // virtual milliseconds since the start
let ticks = 0;
let recording = false;
let last = null; // { data, file, ms }
const frames = []; // { file, ms }
rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

async function advance(ms) {
  const expired = next("Emulation.virtualTimeBudgetExpired");
  await page("Emulation.setVirtualTimePolicy", { policy: "pauseIfNetworkFetchesPending", budget: ms });
  await expired;
  now += ms;
}

// One frame: time moves on by a frame, Chrome draws, and (when recording) the
// picture is kept. frameTimeTicks must always increase, or Chrome waits.
async function tick() {
  await advance(frameMs);
  ticks += 1;
  const reply = await page("HeadlessExperimental.beginFrame", {
    frameTimeTicks: 1e6 + ticks * frameMs, interval: frameMs,
    ...(recording ? { screenshot: { format: "png" } } : {}),
  });
  if (!recording) return;
  const data = reply.screenshotData ?? last?.data;
  if (!data) fail("Chrome returned no picture");
  if (last && last.data === data) {
    last.ms += frameMs;
    return;
  }
  const file = `frame-${String(frames.length).padStart(5, "0")}.png`;
  writeFileSync(join(out, file), Buffer.from(data, "base64"));
  last = { data, file, ms: frameMs };
  frames.push(last);
}
const run = async (ms) => {
  for (let t = 0; t < ms - 1e-6; t += frameMs) await tick();
};
const screen = () => evaluate("window.demo ? window.demo.text() : ''");

// --- Keys (as tools/shot.mjs sends them) ------------------------------------
const KEYS = {
  Enter: [13, "\r"], Escape: [27], Tab: [9], Backspace: [8], Delete: [46], Space: [32, " "],
  ArrowUp: [38], ArrowDown: [40], ArrowLeft: [37], ArrowRight: [39],
  PageUp: [33], PageDown: [34], Home: [36], End: [35], F1: [112],
};
const MODS = { alt: 1, ctrl: 2, meta: 4, shift: 8 };
async function key(name, mods = []) {
  const modifiers = mods.reduce((m, k) => m | MODS[k], 0);
  const [code, text] = KEYS[name] ?? [name.toUpperCase().charCodeAt(0), name];
  const char = text && !(modifiers & (MODS.ctrl | MODS.alt | MODS.meta)) ? text : undefined;
  const codeName = [...name].length === 1
    ? (/[a-z]/i.test(name) ? `Key${name.toUpperCase()}` : /[0-9]/.test(name) ? `Digit${name}` : "")
    : name;
  const base = { modifiers, key: name === "Space" ? " " : name, code: codeName, windowsVirtualKeyCode: code };
  await page("Input.dispatchKeyEvent", { type: char ? "keyDown" : "rawKeyDown", text: char, ...base });
  await page("Input.dispatchKeyEvent", { type: "keyUp", ...base });
}

// --- Boot: until the demo has settled, then size the window to the canvas ----
// `ready()` turns true when the start has settled, a frame before it's drawn.
for (let i = 0; ; i++) {
  if (i > 600) fail("the demo never started");
  await tick();
  if (await evaluate("!!(window.done && window.demo && window.demo.ready() && window.demo.text().trim())")) break;
}
const rect = await evaluate("(() => { const r = document.getElementById('faro').getBoundingClientRect(); return [r.width, r.height]; })()");
await page("Emulation.setDeviceMetricsOverride", {
  width: Math.ceil(rect[0]), height: Math.ceil(rect[1]), deviceScaleFactor: dpr, mobile: false,
});
// The resize blanks the screen until the next draw.
for (let i = 0; ; i++) {
  if (i > 120) fail("the demo didn't redraw after the resize");
  await tick();
  if (await evaluate("!!window.demo.text().trim()")) break;
}
await run(500);

// --- The storyboard ----------------------------------------------------------
const started = Date.now();
let recordedAt = null; // virtual time when recording first started
for (const [index, step] of story.entries()) {
  const where = `step ${index + 1}${step.note ? ` (${step.note})` : ""}`;
  try {
    if (step.record !== undefined) recording = step.record && !dry;
    if (step.record && recordedAt === null) recordedAt = now;
    if (step.key) await key(step.key, step.mods);
    if (step.type !== undefined) {
      const gap = 1000 / (step.cps ?? 12);
      for (const c of step.type) {
        await key(c === " " ? "Space" : c);
        await run(gap);
      }
    }
    if (step.until !== undefined) {
      const deadline = now + (step.within ?? 15000);
      const want = (text) => (step.gone ? !text.includes(step.until) : text.includes(step.until));
      while (!want(await screen())) {
        if (now > deadline) fail(`${where}: "${step.until}" ${step.gone ? "never went away" : "never appeared"}\n${await screen()}`);
        await tick();
      }
    }
    if (step.expect !== undefined && !(await screen()).includes(step.expect)) {
      fail(`${where}: expected "${step.expect}" on screen\n${await screen()}`);
    }
    // Keys and `until` wait a beat afterwards so the change can be seen.
    const pause = step.wait ?? step.then ?? (step.key || step.until !== undefined ? 300 : 0);
    await run(pause);
    if (step.still) {
      const shot = await page("HeadlessExperimental.beginFrame", {
        frameTimeTicks: 1e6 + (ticks += 1) * frameMs, interval: frameMs, screenshot: { format: "png" },
      });
      const data = shot.screenshotData ?? last?.data;
      if (data) writeFileSync(join(out, `still-${step.still}.png`), Buffer.from(data, "base64"));
    }
    if (step.dump) console.log(`--- ${step.dump} at ${(now / 1000).toFixed(2)} s\n${await screen()}`);
    if (verbose) {
      const clip = recordedAt === null ? "" : `  (clip ${((now - recordedAt) / 1000).toFixed(2)} s)`;
      console.log(`${(now / 1000).toFixed(2).padStart(7)} s${clip}  ${where}`);
    }
  } catch (error) {
    fail(`${where}: ${error.message}`);
  }
}
if (logs.length) fail(`page errors:\n${logs.join("\n")}`);
if (dry) {
  console.log(`dry run: the storyboard played to the end at ${(now / 1000).toFixed(2)} s`);
  cleanup();
  process.exit(0);
}
if (!frames.length) fail("nothing was recorded (no step sets \"record\": true)");

// ffconcat: each frame with its duration; the last file is listed twice so
// ffmpeg honours its duration.
const concat = ["ffconcat version 1.0"];
for (const f of frames) concat.push(`file '${f.file}'`, `duration ${(f.ms / 1000).toFixed(4)}`);
concat.push(`file '${frames.at(-1).file}'`);
writeFileSync(join(out, "frames.ffconcat"), concat.join("\n") + "\n");
const total = frames.reduce((s, f) => s + f.ms, 0) / 1000;
const png = Buffer.from(last.data, "base64");
const [width, height] = [png.readUInt32BE(16), png.readUInt32BE(20)];
if (Math.abs(width - rect[0] * dpr) > dpr) fail(`frames are ${width} px wide, not ${rect[0]} × ${dpr}: the scale factor didn't apply`);
console.log(`recorded ${total.toFixed(1)} s in ${frames.length} unique frames (${fps} fps, ${width}×${height}) in ${((Date.now() - started) / 1000).toFixed(1)} s → ${out}`);
cleanup();
process.exit(0);
