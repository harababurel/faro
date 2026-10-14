// §6.9 demo analytics: runs site/template/demo.js in a fake DOM (Node
// builtins only) and checks the four Plausible events: started once per
// page with how=auto|button, focused once, message sent once (from the
// faro-sent signal), Theme changed only on committed visitor choices —
// never on previews, scheme following, restart restores or same-theme
// no-ops; opt-out, and an absent or throwing tracker, stay inert.
// Plus the 4.5b responsive rule: while the phone gallery shows nothing is
// fetched; growing hands the panel to the live demo, shrinking frees it,
// and growing back reuses the cached manifest and module. Section H adds
// the lifecycle fences: stalled fetches and starts paused by a mode flip,
// no orphan canvases, and the fullscreen exemption. Section I adds the
// failure paths: one failed manifest read doesn't block the next wide
// flip, and a hung wasm download hits a 30 s abort deadline that returns
// the Start button for a fresh click.
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
    get firstChild() { return node.children[0] || null; },
    removeChild(c) { node.children = node.children.filter((x) => x !== c); return c; },
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

function boot({ preload, plausible, storage, failWasmOnce, failManifestOnce, hangWasm,
  galleryDisplay, idleQueue, stallFetch, stallStart }) {
  const store = storage || {};
  const blocked = store === "blocked"; // localStorage throws (private mode)
  const byId = {};
  const frame = el("div");
  const panel = el("div");
  panel.appendChild(frame);
  frame.appendChild(el("figcaption"));
  const gallery = el("div");
  gallery._display = galleryDisplay || "none"; // CSS's phone/desktop call
  panel.appendChild(gallery);
  const tools = el("div");
  ["restart", "fullscreen"].forEach((name) => {
    const b = el("button");
    b.setAttribute("data-tool", name);
    const label = el("span");
    label.classList.add("sr-only");
    b.appendChild(label);
    tools.appendChild(b);
  });
  ["demo-status", "demo-start", "demo-exited", "demo-again", "demo-cta",
    "demo-focus", "demo-controls-status"].forEach((id) => { byId[id] = el("div"); });
  byId["demo-theme"] = el("select"); // the theme menu (§4.5)
  byId["demo-theme"].value = "";
  byId["demo-start"].hidden = true; // the template starts it hidden
  const root = el("html");
  const doc = {
    querySelector: (sel) => (sel === ".panel .frame" ? frame
      : sel === ".panel-tools" ? tools : sel === ".phone-gallery" ? gallery : null),
    getElementById: (id) => byId[id] || null,
    createElement: (tag) => { const c = el(tag); c.doc = doc; return c; },
    documentElement: root, fonts: { load: async () => {} },
    hidden: false, hasFocus: () => true, activeElement: null, fullscreenElement: null,
    _cbs: {},
    addEventListener(t, f) { (doc._cbs[t] ||= []).push(f); },
    exitFullscreen: async () => {
      doc.fullscreenElement = null;
      (doc._cbs.fullscreenchange || []).forEach((f) => f({ type: "fullscreenchange" }));
    },
  };
  panel.requestFullscreen = async () => {
    doc.fullscreenElement = panel;
    (doc._cbs.fullscreenchange || []).forEach((f) => f({ type: "fullscreenchange" }));
  };
  const mqs = {};
  const mq = (q, m) => (mqs[q] ||= { matches: m, addEventListener() {} });
  const win = {
    matchMedia: (q) => mq(q, q === "(any-pointer: fine)"),
    _cbs: {},
    addEventListener(t, f) { (win._cbs[t] ||= []).push(f); },
    requestIdleCallback: (f) => { if (idleQueue) (win._idle ||= []).push(f); else f(); },
    requestAnimationFrame: (f) => f(),
    // the harness's fast deadline: only the wasm fetch's 30000 ms timer is
    // redirected (to a 0 ms tick), so a hung download aborts without sleeps.
    setTimeout: (f, ms, ...r) => setTimeout(f, hangWasm && ms === 30000 ? 0 : ms, ...r),
    clearTimeout: (...a) => clearTimeout(...a),
    getComputedStyle: (node) => ({
      paddingLeft: "0", paddingRight: "0", borderLeftWidth: "0", borderRightWidth: "0",
      paddingTop: "0", paddingBottom: "0", borderTopWidth: "0", borderBottomWidth: "0",
      display: node && node._display !== undefined ? node._display : "none",
    }),
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
    async start(canvas) {
      fake.canvas = canvas;
      if (stallStart && !fake._stalled) { fake._stalled = true; await globalThis.__goStartP; }
      return ({
        ready: () => true,
        theme: () => fake.eff,
        configured_theme: () => fake.cfg,
        reduce_motion: () => false,
        pick_theme: (n) => fake.pick(n),
        set_theme: (n) => fake.pick(n),
        configure_reduce_motion() {}, set_reduce_motion() {},
        set_font_size: () => true, open_themes() {}, free() {},
        themes: () => [["faro-dark", "faro"], ["faro-light", "faro"],
          ["nord", "OpenCode · dark"], ["gruvbox", "OpenCode · dark"]],
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
  globalThis.__manifestN = 0;
  globalThis.__sawSignal = false;
  if (stallFetch) globalThis.__goFetchP = new Promise((go) => { globalThis.__goFetch = go; });
  if (stallStart) globalThis.__goStartP = new Promise((go) => { globalThis.__goStart = go; });
  globalThis.fetch = async (url, init) => {
    if (String(url).endsWith("manifest.json")) {
      globalThis.__manifestN += 1;
      if (failManifestOnce && globalThis.__manifestN === 1) return { ok: false, status: 500 };
      return { ok: true, json: async () => manifest };
    }
    globalThis.__fetchN += 1;
    if (stallFetch && globalThis.__fetchN === 1) await globalThis.__goFetchP;
    if (failWasmOnce && globalThis.__fetchN === 1) return { ok: false, status: 500 };
    if (hangWasm && globalThis.__fetchN === 1) {
      // a download that never delivers a chunk: the reader only settles
      // when fetchWasm's signal aborts it (a real browser honours this too).
      globalThis.__sawSignal = !!(init && init.signal);
      if (!globalThis.__sawSignal) throw new Error("hung fetch: no abort signal");
      return { ok: true, body: { getReader: () => ({
        read: () => new Promise((_go, rej) => {
          init.signal.addEventListener("abort", () => rej(init.signal.reason));
        }),
      }) } };
    }
    return { ok: true, body: { getReader: () => ({ read: async () => ({ done: true }) }) } };
  };
  new Function(SRC)();
  const sel = byId["demo-theme"];
  return {
    // the visitor picks a theme in the menu
    pick: (name) => { sel.value = name; (sel._listeners.change || []).forEach((f) => f({})); },
    sel,
    fireLoad: () => (win._cbs.load || []).forEach((f) => f()),
    fireResize: () => (win._cbs.resize || []).forEach((f) => f()),
    click: (node) => (node._listeners.click || []).forEach((f) => f({})),
    focusCanvas: () => { doc.activeElement = fake.canvas; fake.canvas.dispatch("focus"); },
    blurCanvas: () => { doc.activeElement = null; fake.canvas.dispatch("blur"); },
    fake, byId, root, win, light, store, gallery, frame, doc,
    fsBtn: tools.querySelector('[data-tool="fullscreen"]'),
    runIdle: () => (win._idle || []).splice(0).forEach((f) => f()),
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
  ok(h.byId["demo-cta"].textContent === "▶ Try it here, no install",
    "a computer that runs the demo: the CTA says Try from the start");
  ok(h.sel.children.length === 2 && h.sel.children[1].label === "OpenCode · dark" &&
    h.sel.children[1].children.length === 2, "the theme menu lists the demo's themes in groups");
  ok(h.sel.value === "faro-dark", "the theme menu shows the theme faro draws");

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

  h.pick("nord"); await tick(); // nord, prev faro-light
  let th = calls.filter(([n]) => n === "Theme changed");
  ok(th.length === 1 && th[0][1].theme === "nord", "menu nord: Theme changed {theme:nord} once");
  h.pick("nord"); await tick();
  ok(calls.filter(([n]) => n === "Theme changed").length === 1,
    "menu nord again: no event (same theme)");

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
  h.pick("faro-dark"); await tick(); // would count if not opted out
  ok(calls.filter(([n]) => n === "Theme changed").length === 2, "opt-out: nothing sent");
  delete h.store.plausible_ignore;

  h.win.plausible = () => { throw new Error("tracker down"); };
  h.pick("nord"); await tick();
  ok(true, "throwing tracker: no crash");
  h.win.plausible = undefined;
  h.pick("faro-dark"); await tick();
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
  hd.pick("nord"); await tick();
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

  // --- G: phone gallery (4.5b): nothing fetched while it shows; growing
  // starts the demo, shrinking frees it, growing back reuses the cache ---
  const callsG = [];
  const hg = boot({
    preload: true, storage: {}, galleryDisplay: "flex",
    plausible: (n, o) => callsG.push([n, o && o.props]),
  });
  hg.fireLoad();
  for (let i = 0; i < 4; i += 1) await tick();
  ok(globalThis.__manifestN === 0 && globalThis.__fetchN === 0,
    "phone mode at load: no manifest and no wasm fetched");
  ok(hg.byId["demo-cta"].textContent === "", "phone mode at load: the CTA keeps its See label");
  hg.gallery._display = "none";
  hg.fireResize();
  for (let i = 0; i < 6; i += 1) await tick();
  const stG = callsG.filter(([n]) => n === "Demo started");
  ok(stG.length === 1 && stG[0][1] && stG[0][1].how === "auto",
    "growing wide: manifest read lazily, Demo started {how:auto} once");
  ok(hg.frame.classList.contains("live"), "growing wide: the frame went live");
  hg.gallery._display = "flex";
  hg.fireResize();
  await tick();
  ok(!hg.frame.classList.contains("live") && hg.win.demo === null,
    "shrinking: the live demo is freed and the gallery takes over");
  ok(hg.frame.children.filter((c) => c.tagName === "canvas").length === 0,
    "shrinking: the canvas is gone");
  hg.gallery._display = "none";
  hg.fireResize();
  for (let i = 0; i < 6; i += 1) await tick();
  ok(callsG.filter(([n]) => n === "Demo started").length === 1,
    "growing back: no second Demo started (visit choices survive)");
  ok(globalThis.__manifestN === 1 && globalThis.__fetchN === 1,
    "growing back: cached manifest and module, one fetch each");

  // --- H: loader lifecycle fences (4.5b) ---

  // H1: desktop at load, narrowed before the idle read: no manifest fetch.
  const h1 = boot({ preload: true, storage: {}, idleQueue: true });
  h1.fireLoad();
  h1.gallery._display = "flex"; // narrowed between load and idle
  h1.runIdle(); await tick();
  ok(globalThis.__manifestN === 0 && globalThis.__fetchN === 0,
    "narrowed before the idle read: the read skips, nothing fetched");
  h1.gallery._display = "none"; h1.fireResize();
  h1.runIdle();
  for (let i = 0; i < 6; i += 1) await tick();
  ok(globalThis.__manifestN === 1 && h1.frame.classList.contains("live"),
    "wide again: checkMode retries the read and the demo starts");

  // H2: the wasm fetch stalls; wide -> phone -> wide: quiet pause, no canvas.
  const h2 = boot({ preload: true, storage: {}, stallFetch: true });
  h2.fireLoad(); await tick(); // manifest read, begin: the wasm fetch stalls
  ok(globalThis.__fetchN === 1, "phone flip mid-fetch: the fetch had started");
  h2.gallery._display = "flex"; h2.fireResize(); await tick();
  globalThis.__goFetch();
  for (let i = 0; i < 6; i += 1) await tick();
  ok(!h2.frame.classList.contains("live") &&
    h2.frame.children.filter((c) => c.tagName === "canvas").length === 0,
    "phone while loading: completes quietly, no canvas, not live");
  h2.gallery._display = "none"; h2.fireResize();
  for (let i = 0; i < 6; i += 1) await tick();
  ok(h2.frame.classList.contains("live") &&
    h2.frame.children.filter((c) => c.tagName === "canvas").length === 1,
    "wide again: the cached module starts, exactly one canvas");
  ok(globalThis.__manifestN === 1 && globalThis.__fetchN === 1,
    "wide again: still one manifest read and one wasm fetch (no restart storm)");

  // H3: mod.start stalls; the phone fence frees the half-made canvas and a
  // stale attempt must not clear the new one's state.
  const h3 = boot({ preload: true, storage: {}, stallStart: true });
  h3.fireLoad();
  for (let i = 0; i < 3; i += 1) await tick(); // up to mod.start
  h3.gallery._display = "flex"; h3.fireResize(); await tick();
  globalThis.__goStart();
  for (let i = 0; i < 6; i += 1) await tick();
  ok(!h3.frame.classList.contains("live") &&
    h3.frame.children.filter((c) => c.tagName === "canvas").length === 0,
    "phone while starting: the canvas is gone, nothing is live");
  h3.gallery._display = "none"; h3.fireResize();
  for (let i = 0; i < 6; i += 1) await tick();
  ok(h3.frame.classList.contains("live") && globalThis.__fetchN === 1,
    "wide again: one wasm fetch, the new attempt goes live (no stale reset)");

  // H4: fullscreen exemption: narrow while fullscreen keeps the live demo;
  // exiting fullscreen into phone width frees it (checkMode after fsOn).
  const h4 = boot({ preload: true, storage: {} });
  h4.fireLoad();
  for (let i = 0; i < 6; i += 1) await tick(); // live, wide
  h4.click(h4.fsBtn);
  for (let i = 0; i < 3; i += 1) await tick(); // requestFullscreen resolves
  h4.gallery._display = "flex"; h4.fireResize(); await tick();
  ok(h4.frame.classList.contains("live") &&
    h4.frame.children.filter((c) => c.tagName === "canvas").length === 1,
    "fullscreen on a narrow viewport: exempt, not torn down");
  h4.click(h4.fsBtn);
  for (let i = 0; i < 3; i += 1) await tick(); // exitFullscreen + fschange
  ok(!h4.frame.classList.contains("live") &&
    h4.frame.children.filter((c) => c.tagName === "canvas").length === 0,
    "exiting fullscreen into phone width: checkMode frees the demo");

  // H5: the start button clicked while the gallery shows: begin refuses.
  const h5 = boot({ preload: false, storage: {}, galleryDisplay: "none" });
  h5.fireLoad(); await tick();
  ok(h5.byId["demo-start"].hidden === false, "wide, no preload: the start button shows");
  h5.gallery._display = "flex"; h5.fireResize(); await tick();
  h5.click(h5.byId["demo-start"]); await tick();
  ok(globalThis.__fetchN === 0, "start clicked on phone: begin refuses, nothing fetched");

  // --- I: failed manifest read and hung wasm download (4.5b review) ---

  // I1: the manifest read fails once: nothing else loads and the button
  // stays hidden; the next narrow→wide flip retries the read exactly once.
  const h6 = boot({ preload: true, storage: {}, failManifestOnce: true });
  h6.fireLoad(); await tick();
  ok(globalThis.__manifestN === 1 && globalThis.__fetchN === 0 &&
    h6.byId["demo-start"].hidden === true,
    "failed manifest: no wasm fetch, no start button");
  h6.gallery._display = "flex"; h6.fireResize(); await tick();
  h6.gallery._display = "none"; h6.fireResize();
  for (let i = 0; i < 6; i += 1) await tick();
  ok(globalThis.__manifestN === 2 && globalThis.__fetchN === 1 &&
    h6.frame.classList.contains("live"),
    "wide again: the read retries once and the demo starts");
  h6.fireResize(); h6.fireResize();
  for (let i = 0; i < 4; i += 1) await tick();
  ok(globalThis.__manifestN === 2 && globalThis.__fetchN === 1,
    "live again: further resizes fetch nothing new (no retry storm)");

  // I2: the wasm download hangs: the 30 s deadline (redirected to a fast
  // tick) aborts the body reader, the Start button comes back, and a click
  // fetches afresh and goes live.
  const h7 = boot({ preload: true, storage: {}, hangWasm: true });
  h7.fireLoad(); await tick();
  ok(globalThis.__fetchN === 1 && globalThis.__sawSignal,
    "hung wasm: the fetch started with an abort signal");
  for (let i = 0; i < 6; i += 1) await tick(); // the redirected deadline fires
  ok(h7.byId["demo-start"].hidden === false && h7.byId["demo-status"].hidden === true,
    "hung wasm: the deadline aborts the read and the Start button returns");
  h7.click(h7.byId["demo-start"]);
  for (let i = 0; i < 6; i += 1) await tick();
  ok(h7.frame.classList.contains("live") && globalThis.__fetchN === 2,
    "Start after the abort: a fresh fetch goes live");
}

run().then(() => {
  console.log(failures ? `FAIL: ${failures} check(s)` : "all analytics checks pass");
  process.exit(failures ? 1 : 0);
}).catch((e) => {
  console.error("harness error:", e);
  process.exit(1);
});
