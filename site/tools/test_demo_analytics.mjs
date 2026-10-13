// §6.9 demo analytics: runs site/template/demo.js in a fake DOM (Node
// builtins only) and checks the four Plausible events: started once per
// page with how=auto|button, focused once, message sent once (from the
// faro-sent signal), Theme changed only on committed visitor choices —
// never on previews, scheme following, restart restores or same-theme
// no-ops; opt-out, and an absent or throwing tracker, stay inert.
// Run: node site/tools/test_demo_analytics.mjs. Not in CI (that's 4.6).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const SRC = readFileSync(
  path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "template", "demo.js"),
  "utf8"
);
const KNOWN = new Set(["faro-dark", "faro-light", "nord", "gruvbox"]);
const NAMES = new Set(["Demo started", "Demo focused", "Demo message sent", "Theme changed"]);

let failures = 0;
const ok = (cond, what) => {
  console.log((cond ? "  ok   " : "  FAIL ") + what);
  if (!cond) failures += 1;
};
const tick = () => new Promise((go) => setTimeout(go, 0));

function matches(node, sel) {
  if (sel.startsWith(".")) return node.classList.contains(sel.slice(1));
  const attr = sel.match(/^\[([\w-]+)(?:="([^"]*)")?\]$/);
  if (attr) {
    const v = node._attrs[attr[1]];
    return v !== undefined && (attr[2] === undefined || v === attr[2]);
  }
  return node.tagName === sel;
}

function el(tag) {
  const listeners = {};
  const attrs = {};
  const classes = new Set();
  const node = {
    tagName: tag, parentElement: null, children: [], hidden: false,
    textContent: "", disabled: false, offsetHeight: 30, clientWidth: 1200,
    style: {}, _attrs: attrs, _listeners: listeners,
    classList: {
      add: (c) => classes.add(c), remove: (c) => classes.delete(c),
      contains: (c) => classes.has(c),
      toggle: (c, on) => (on ? classes.add(c) : classes.delete(c)),
    },
    addEventListener: (t, f) => (listeners[t] ||= []).push(f),
    dispatch(t, detail) {
      (listeners[t] || []).forEach((f) => f({ type: t, detail, preventDefault() {} }));
      return true;
    },
    setAttribute: (k, v) => { attrs[k] = String(v); },
    getAttribute: (k) => (k in attrs ? attrs[k] : null),
    appendChild(c) { c.parentElement = node; node.children.push(c); return c; },
    remove() {
      const p = node.parentElement;
      if (p) p.children = p.children.filter((c) => c !== node);
    },
    focus() { node.doc.activeElement = node; },
    scrollIntoView() {},
    querySelector: (sel) => node.children.find((c) => matches(c, sel)) || null,
    querySelectorAll: (sel) => node.children.filter((c) => matches(c, sel)),
  };
  return node;
}

function boot({ preload, plausible, storage, failWasmOnce }) {
  const store = storage || {};
  const blocked = store === "blocked"; // localStorage throws (private mode)
  const byId = {};
  const frame = el("div");
  const panel = el("div");
  panel.appendChild(frame);
  frame.appendChild(el("figcaption"));
  const tools = el("div");
  const chips = ["faro-dark", "nord", "gruvbox"].map((name) => {
    const b = el("button");
    b.setAttribute("data-theme", name);
    tools.appendChild(b);
    return b;
  });
  ["themes", "motion", "restart", "fullscreen"].forEach((name) => {
    const b = el("button");
    b.setAttribute("data-tool", name);
    const label = el("span");
    label.classList.add("sr-only");
    b.appendChild(label);
    tools.appendChild(b);
  });
  ["demo-status", "demo-start", "demo-exited", "demo-again", "demo-cta",
    "demo-focus", "demo-controls-status"].forEach((id) => { byId[id] = el("div"); });
  const root = el("html");
  const doc = {
    querySelector: (sel) => (sel === ".panel .frame" ? frame : sel === ".panel-tools" ? tools : null),
    getElementById: (id) => byId[id] || null,
    createElement: (tag) => { const c = el(tag); c.doc = doc; return c; },
    documentElement: root, fonts: { load: async () => {} },
    hidden: false, hasFocus: () => true, activeElement: null,
    addEventListener() {},
  };
  const mqs = {};
  const mq = (q, m) => (mqs[q] ||= { matches: m, addEventListener() {} });
  const win = {
    matchMedia: (q) => mq(q, q === "(pointer: fine)"),
    _cbs: {},
    addEventListener(t, f) { (win._cbs[t] ||= []).push(f); },
    requestIdleCallback: (f) => f(),
    requestAnimationFrame: (f) => f(),
    setTimeout: (...a) => setTimeout(...a),
    getComputedStyle: () => new Proxy({}, { get: () => "0" }),
    devicePixelRatio: 1, innerWidth: 1400, innerHeight: 900,
    MutationObserver: class { constructor(cb) { win._schemeCb = cb; } observe() {} },
    plausible,
  };
  const light = mq("(prefers-color-scheme: light)", false);
  const fake = {
    cfg: "faro-dark", eff: "faro-dark", canvas: null,
    settings() {
      if (fake.canvas) {
        fake.canvas.dispatch("faro-settings",
          { theme: fake.eff, configuredTheme: fake.cfg, reduceMotion: false });
      }
    },
    // faro draws on the next frame: faro-settings arrives after the click.
    pick(n) {
      if (!KNOWN.has(n)) return false;
      fake.cfg = n; fake.eff = n;
      queueMicrotask(fake.settings);
      return true;
    },
    // a palette commit: faro changed it; the page only sees the event.
    palette(n) { fake.cfg = n; fake.eff = n; fake.settings(); },
    // a palette preview: only the effective theme moves, the setting stays.
    preview(n) { fake.eff = n; fake.settings(); },
    sent() { if (fake.canvas) fake.canvas.dispatch("faro-sent"); },
    start(canvas) {
      fake.canvas = canvas;
      return Promise.resolve({
        ready: () => true,
        theme: () => fake.eff,
        configured_theme: () => fake.cfg,
        reduce_motion: () => false,
        pick_theme: (n) => fake.pick(n),
        set_theme: (n) => fake.pick(n),
        configure_reduce_motion() {}, set_reduce_motion() {},
        set_font_size: () => true, open_themes() {}, free() {},
      });
    },
  };
  globalThis.window = win;
  globalThis.document = doc;
  Object.defineProperty(globalThis, "navigator", { value: {}, configurable: true });
  globalThis.localStorage = blocked
    ? { getItem: () => { throw new Error("blocked"); }, setItem: () => { throw new Error("blocked"); } }
    : {
        getItem: (k) => (k in store ? store[k] : null),
        setItem: (k, v) => { store[k] = String(v); },
      };
  globalThis.__fake = fake;
  const manifest = {
    preload, bytes: 1000, size: 1200, wasm: "/demo/faro.wasm",
    js: "data:text/javascript," + encodeURIComponent(
      "export default function(){return Promise.resolve();}\n" +
      "export function start(...a){return globalThis.__fake.start(...a);}"
    ),
  };
  globalThis.__fetchN = 0;
  globalThis.fetch = async (url) => {
    if (String(url).endsWith("manifest.json")) return { ok: true, json: async () => manifest };
    globalThis.__fetchN += 1;
    if (failWasmOnce && globalThis.__fetchN === 1) return { ok: false, status: 500 };
    return { ok: true, body: { getReader: () => ({ read: async () => ({ done: true }) }) } };
  };
  new Function(SRC)();
  return {
    fireLoad: () => (win._cbs.load || []).forEach((f) => f()),
    click: (node) => (node._listeners.click || []).forEach((f) => f({})),
    focusCanvas: () => { doc.activeElement = fake.canvas; fake.canvas.dispatch("focus"); },
    blurCanvas: () => { doc.activeElement = null; fake.canvas.dispatch("blur"); },
    fake, byId, chips, root, win, light, store,
  };
}

async function run() {
  // --- A: preload (auto); one page across restarts ---
  const calls = [];
  const h = boot({ preload: true, storage: {}, plausible: (n, o) => calls.push([n, o && o.props]) });
  h.fireLoad();
  for (let i = 0; i < 4; i += 1) await tick();
  const started = calls.filter(([n]) => n === "Demo started");
  ok(started.length === 1 && started[0][1] && started[0][1].how === "auto",
    "auto preload: Demo started {how:auto} once");
  ok(calls.filter(([n]) => n === "Demo focused").length === 0, "autostart: no Demo focused");

  h.fake.preview("gruvbox"); await tick(); // palette preview: only eff moves
  ok(calls.filter(([n]) => n === "Theme changed").length === 0, "preview: no Theme changed");

  h.light.matches = true; h.win._schemeCb(); await tick(); // fresh, unpinned
  ok(calls.filter(([n]) => n === "Theme changed").length === 0, "scheme follow: no Theme changed");
  ok(h.fake.cfg === "faro-light", "scheme follow actually applied (applyScheme ran)");

  h.fake.sent(); h.fake.sent();
  ok(calls.filter(([n]) => n === "Demo message sent").length === 1,
    "faro-sent twice: one Demo message sent");

  h.focusCanvas();
  ok(calls.filter(([n]) => n === "Demo focused").length === 1, "focus: Demo focused once");
  h.blurCanvas(); h.focusCanvas();
  ok(calls.filter(([n]) => n === "Demo focused").length === 1, "refocus: no second Demo focused");

  h.click(h.chips[1]); await tick(); // nord, prev faro-light
  let th = calls.filter(([n]) => n === "Theme changed");
  ok(th.length === 1 && th[0][1].theme === "nord", "chip nord: Theme changed {theme:nord} once");
  h.click(h.chips[1]); await tick();
  ok(calls.filter(([n]) => n === "Theme changed").length === 1,
    "chip nord again: no event (same theme)");

  h.fake.palette("gruvbox");
  th = calls.filter(([n]) => n === "Theme changed");
  ok(th.length === 2 && th[1][1].theme === "gruvbox", "palette commit: Theme changed {theme:gruvbox}");

  h.click(h.byId["demo-again"]); // restart: same page
  for (let i = 0; i < 4; i += 1) await tick();
  ok(calls.filter(([n]) => n === "Demo started").length === 1, "restart: no second Demo started");
  ok(calls.filter(([n]) => n === "Theme changed").length === 2, "restart restore: no Theme changed");
  h.fake.sent(); h.focusCanvas();
  ok(calls.filter(([n]) => n === "Demo message sent").length === 1,
    "restart: no second Demo message sent");
  ok(calls.filter(([n]) => n === "Demo focused").length === 1, "restart: no second Demo focused");

  h.store.plausible_ignore = "true";
  h.click(h.chips[0]); await tick(); // would count if not opted out
  ok(calls.filter(([n]) => n === "Theme changed").length === 2, "opt-out: nothing sent");
  delete h.store.plausible_ignore;

  h.win.plausible = () => { throw new Error("tracker down"); };
  h.click(h.chips[1]); await tick();
  ok(true, "throwing tracker: no crash");
  h.win.plausible = undefined;
  h.click(h.chips[0]); await tick();
  ok(true, "absent tracker: no crash");

  ok(calls.every(([n, p]) => NAMES.has(n) &&
    (p === undefined || (Object.keys(p).length === 1 &&
      ((n === "Demo started" && p.how === "auto") ||
       (n === "Theme changed" && KNOWN.has(p.theme)))))),
    "every call: whitelisted name, only how/theme props");

  // --- B: no preload: the start button itself starts by button ---
  const callsB = [];
  const hb = boot({ preload: false, storage: {}, plausible: (n, o) => callsB.push([n, o && o.props]) });
  hb.fireLoad(); await tick();
  ok(callsB.length === 0, "no preload: nothing sent yet");
  hb.click(hb.byId["demo-start"]);
  for (let i = 0; i < 4; i += 1) await tick();
  const stB = callsB.filter(([n]) => n === "Demo started");
  ok(stB.length === 1 && stB[0][1] && stB[0][1].how === "button",
    "start button: Demo started {how:button} once");
  ok(stB.every(([, p]) => p && Object.keys(p).length === 1 && p.how === "button"),
    "button start: props only how=button (no DOM event leaked)");

  // --- C: no preload: the hero CTA starts by button, start adds nothing ---
  const callsC = [];
  const hc = boot({ preload: false, storage: {}, plausible: (n, o) => callsC.push([n, o && o.props]) });
  hc.fireLoad(); await tick();
  hc.click(hc.byId["demo-cta"]);
  for (let i = 0; i < 4; i += 1) await tick();
  const stC = callsC.filter(([n]) => n === "Demo started");
  ok(stC.length === 1 && stC[0][1] && stC[0][1].how === "button", "CTA: Demo started {how:button} once");
  hc.click(hc.byId["demo-start"]); await tick();
  ok(callsC.filter(([n]) => n === "Demo started").length === 1, "start after CTA: no second started");

  // --- D: opted out from the start; removing the flag counts again ---
  const callsD = [];
  const hd = boot({
    preload: true, storage: { plausible_ignore: "true" },
    plausible: (n, o) => callsD.push([n, o && o.props]),
  });
  hd.fireLoad();
  for (let i = 0; i < 4; i += 1) await tick();
  ok(callsD.length === 0, "initial opt-out: nothing sent");
  delete hd.store.plausible_ignore;
  hd.click(hd.chips[1]); await tick();
  ok(callsD.filter(([n]) => n === "Theme changed").length === 1, "opt-out lifted: counts again");

  // --- E: failed auto start, then the visitor starts by button ---
  const callsE = [];
  const he = boot({
    preload: true, storage: {}, failWasmOnce: true,
    plausible: (n, o) => callsE.push([n, o && o.props]),
  });
  he.fireLoad();
  for (let i = 0; i < 4; i += 1) await tick();
  ok(callsE.length === 0, "failed auto start: nothing sent");
  he.click(he.byId["demo-start"]);
  for (let i = 0; i < 4; i += 1) await tick();
  const stE = callsE.filter(([n]) => n === "Demo started");
  ok(stE.length === 1 && stE[0][1] && stE[0][1].how === "button",
    "button retry after failed auto: {how:button} once");

  // --- F: localStorage throws (private mode): events still fire ---
  const callsF = [];
  const hf = boot({ preload: true, storage: "blocked", plausible: (n, o) => callsF.push([n, o && o.props]) });
  hf.fireLoad();
  for (let i = 0; i < 4; i += 1) await tick();
  ok(callsF.filter(([n]) => n === "Demo started").length === 1, "blocked storage: still counts");
}

run().then(() => {
  console.log(failures ? `FAIL: ${failures} check(s)` : "all analytics checks pass");
  process.exit(failures ? 1 : 0);
}).catch((e) => {
  console.error("harness error:", e);
  process.exit(1);
});
