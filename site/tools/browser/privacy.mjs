// Privacy and opt-out scenarios (WEBSITE.md §4.2, §6.8, §6.9): the page
// without JS, Save-Data, the manifest's preload-off variant, and the
// footer opt-out. Absent/throwing trackers are already gated by the pure
// Node harness (site/tools/test_demo_analytics.mjs); here the opt-out is
// the browser-side proof. Each scenario gets its own fresh tab and, where
// storage was touched, its storage is cleared before the tab closes.
import { POINTER_FINE_SHIM } from "./cdp.mjs";
import {
  OPT_OUT_SHIM, SAVE_DATA_SHIM, idleProof, joinInit, sleep, waitLoad,
} from "./common.mjs";

export function privacyScenarios() {
  return [
    { name: "no JS: static page, gallery and FAQ, no /demo/* fetch",
      viewport: { width: 390, height: 844, mobile: true },
      scriptsDisabled: true, run: noJs },
    { name: "Save-Data: manifest read but no wasm, Start button offered",
      viewport: { width: 1440, height: 900 },
      init: joinInit(POINTER_FINE_SHIM, SAVE_DATA_SHIM), run: saveData },
    { name: "manifest preload off: no wasm, Start button offered",
      viewport: { width: 1440, height: 900 },
      init: POINTER_FINE_SHIM, manifestVariant: "no-preload", run: noPreload },
    { name: "opt-out: demo works, the tracker captured nothing",
      viewport: { width: 1440, height: 900 },
      init: joinInit(POINTER_FINE_SHIM, OPT_OUT_SHIM), run: optOut },
  ];
}

async function noJs(ctx, sc) {
  await ctx.page.goto(`${ctx.origin}/`);
  await waitLoad(ctx.page);
  await sleep(1500); // nothing can fetch with scripts off; a short window proves it
  const fetched = ctx.netNew((u) => u.includes("/demo/"));
  ctx.assert(sc, "no /demo/* request without JS", fetched.length === 0, fetched.join(", "));
  ctx.assert(sc, "html never got the js class",
    !(await ctx.page.evaluate("document.documentElement.classList.contains('js')")));
  ctx.assert(sc, "no canvas without JS",
    !(await ctx.page.evaluate("!!document.querySelector('canvas.demo-canvas')")));
  ctx.assert(sc, "gallery still there (3 figures)",
    (await ctx.page.evaluate("document.querySelectorAll('.phone-gallery figure').length")) === 3);
  ctx.assert(sc, "FAQ section present",
    await ctx.page.evaluate("!!document.querySelector('.faq')"));
  ctx.assert(sc, "essential copy present",
    await ctx.page.evaluate("document.body.textContent.includes('Never freezes')"));
}

// Shared by Save-Data and preload-off: the manifest is read (proving the
// loader ran and the idle window elapsed) but the module is never fetched,
// and the Start button is offered instead.
async function offeredNotLoaded(ctx, sc) {
  const fetched = await idleProof(ctx);
  ctx.assert(sc, "the manifest was read",
    fetched.some((u) => u.includes("/demo/manifest.json")), fetched.join(", "));
  ctx.assert(sc, "no wasm or module fetch",
    fetched.filter((u) => u.includes("faro_web")).length === 0, fetched.join(", "));
  const start = await ctx.page.evaluate(
    "(() => { const b = document.getElementById('demo-start');" +
    " return b && !b.hidden ? b.textContent : null; })()"
  );
  ctx.assert(sc, "Start button offered", start !== null && start.includes("Start the demo"), start);
  ctx.assert(sc, "no canvas", !(await ctx.page.evaluate("!!document.querySelector('canvas.demo-canvas')")));
}

async function saveData(ctx, sc) {
  ctx.note(sc, "navigator.connection.saveData via init shim (emulation), pointer fine via shim");
  await ctx.page.goto(`${ctx.origin}/`);
  await offeredNotLoaded(ctx, sc);
}

async function noPreload(ctx, sc) {
  ctx.note(sc, "server served the built manifest.no-preload.json at /demo/manifest.json (test-only switch)");
  await ctx.page.goto(`${ctx.origin}/`);
  await offeredNotLoaded(ctx, sc);
}

async function optOut(ctx, sc) {
  await ctx.page.goto(`${ctx.origin}/`);
  await ctx.page.waitFor("window.done === true", (v) => v === true, {
    what: "demo still starts for opted-out visitors", timeout: 25000,
  });
  ctx.assert(sc, "demo canvas live despite opt-out",
    await ctx.page.evaluate("document.querySelector('.panel .frame').classList.contains('live')"));
  const tracks = await ctx.tracker();
  ctx.assert(sc, "tracker captured nothing (opt-out honoured)",
    tracks.length === 0, JSON.stringify(tracks).slice(0, 120));
  const api = ctx.netNew((u) => u.includes("/api/event"));
  ctx.assert(sc, "no telemetry endpoint request", api.length === 0, api.join(", "));
}
