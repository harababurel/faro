// Panel-control scenarios (WEBSITE.md §4.3/§4.5, 4.6 chunk 2): the OS's
// reduced-motion preference, the seamless swap (the poster already has the
// canvas's size and the caption its rows), the theme palette (previews
// silent, Enter commits once), the wide→narrow→wide lifecycle (freed, then restored
// from the cached module with the visitor's theme), and fullscreen —
// where headless Chrome's answer is asserted either way, and a denial
// is reported as a skip, never as a pass.
import { POINTER_FINE_SHIM } from "./cdp.mjs";
import { PICK, SAFE_INSPECT, TRACKER } from "./common.mjs";

export function controlScenarios() {
  return [
    { name: "reduced-motion: honoured from the OS",
      viewport: { width: 1440, height: 900 }, init: POINTER_FINE_SHIM,
      media: { "prefers-reduced-motion": "reduce" }, run: motion },
    { name: "seamless swap: the frame and the caption keep their size, the CTA its label",
      viewport: { width: 1440, height: 900 }, init: POINTER_FINE_SHIM, run: swap },
    { name: "theme palette: previews tracked nothing, Enter commits once",
      viewport: { width: 1440, height: 900 }, init: POINTER_FINE_SHIM, run: themePalette },
    { name: "wide→narrow→wide: demo freed then restored, one wasm fetch, theme kept",
      viewport: { width: 1440, height: 900 }, init: POINTER_FINE_SHIM, run: resize },
    { name: "fullscreen: granted end-to-end or a graceful refusal, never a hang",
      viewport: { width: 1440, height: 900 }, init: POINTER_FINE_SHIM, run: fullscreen },
  ];
}

async function liveOnDesktop(ctx) {
  await ctx.page.goto(`${ctx.origin}/`);
  await ctx.page.waitFor("window.done === true", (v) => v === true, {
    what: "demo live", timeout: 25000,
  });
}

async function motion(ctx, sc) {
  ctx.note(sc, "prefers-reduced-motion: reduce via CDP media emulation");
  await liveOnDesktop(ctx);
  ctx.assert(sc, "demo starts reduced",
    await ctx.page.evaluate("window.demo.reduce_motion()"));
}

// What the page shows before and after faro takes over: the frame's box,
// where the next section starts, and the CTA's label.
const LAYOUT = `(() => {
  const r = document.querySelector('.panel .frame').getBoundingClientRect();
  const next = document.querySelector('.panel-section').nextElementSibling.getBoundingClientRect();
  return JSON.stringify({ live: document.querySelector('.panel .frame').classList.contains('live'),
    w: r.width, h: r.height, x: r.left, next: next.top,
    cta: document.getElementById('demo-cta').textContent });
})()`;

async function swap(ctx, sc) {
  await ctx.page.goto(`${ctx.origin}/`);
  // parsed, so demo.js (deferred) has already sized the poster
  await ctx.page.waitFor("document.readyState", (v) => v !== "loading", {
    what: "page parsed", timeout: 10000,
  });
  const before = JSON.parse(await ctx.page.evaluate(LAYOUT));
  await ctx.page.waitFor("window.done === true", (v) => v === true, {
    what: "demo live", timeout: 25000,
  });
  // the fade's settle (250 ms) and a refit frame
  await ctx.page.waitFor(
    "document.querySelector('.panel .frame').classList.contains('settled')",
    (v) => v === true, { what: "poster gone after the fade", timeout: 4000 });
  const after = JSON.parse(await ctx.page.evaluate(LAYOUT));
  if (before.live) {
    ctx.skip(sc, "the demo was already live at the first look: the swap wasn't observed");
    return;
  }
  const near = (a, b) => Math.abs(a - b) <= 1;
  ctx.assert(sc, "the frame keeps its box",
    near(before.w, after.w) && near(before.h, after.h) && near(before.x, after.x),
    JSON.stringify({ before, after }));
  ctx.assert(sc, "the next section doesn't move", near(before.next, after.next),
    JSON.stringify({ before: before.next, after: after.next }));
  ctx.assert(sc, "the CTA says Try from the start, and still does",
    before.cta === after.cta && after.cta.includes("Try it here"), JSON.stringify([before.cta, after.cta]));
}

async function themePalette(ctx, sc) {
  await liveOnDesktop(ctx);
  await ctx.page.click("canvas.demo-canvas");
  await ctx.page.waitFor(SAFE_INSPECT, (s) => s && /focused=true/.test(s), { timeout: 4000 });
  const before = await ctx.page.evaluate("window.demo.configured_theme()");
  await ctx.page.evaluate("window.demo.open_themes()"); // faro's palette at Themes
  await ctx.page.waitFor("window.demo.text()", (t) => t.includes("Themes"), {
    what: "palette open at Themes", timeout: 4000,
  });
  // One ArrowDown could land on a header or the same theme: step at
  // most 4 times, each with a short conditional wait (bounded overall).
  let previewed = before;
  for (let step = 0; step < 4 && previewed === before; step++) {
    await ctx.page.key("ArrowDown");
    try {
      previewed = await ctx.page.waitFor("window.demo.theme()", (t) => t !== before, {
        what: `preview changes the screen theme (ArrowDown ${step + 1})`, timeout: 1200,
      });
    } catch {
      previewed = await ctx.page.evaluate("window.demo.theme()");
    }
  }
  ctx.assert(sc, "ArrowDown previewed another theme", previewed !== before, previewed);
  ctx.assert(sc, "preview tracked nothing",
    ctx.count(await ctx.tracker(), "Theme changed").length === 0, previewed);
  ctx.assert(sc, "preview did not commit",
    (await ctx.page.evaluate("window.demo.configured_theme()")) === before);
  await ctx.page.key("Enter");
  // the commit reaches the page through faro's faro-settings event, one
  // draw later: wait for it instead of reading the tracker immediately.
  await ctx.page.waitFor(TRACKER, (s) => s.includes("Theme changed"), {
    what: "commit tracked via faro-settings", timeout: 4000,
  });
  const committed = await ctx.page.evaluate("window.demo.configured_theme()");
  const changed = ctx.count(await ctx.tracker(), "Theme changed");
  ctx.assert(sc, "commit tracked exactly once with the new name",
    changed.length === 1 && changed[0].props?.theme === committed, JSON.stringify(changed));
}

async function resize(ctx, sc) {
  await liveOnDesktop(ctx);
  await ctx.page.evaluate(PICK("nord"));
  await ctx.page.waitFor("window.demo.theme()", (t) => t === "nord", { timeout: 4000 });
  await ctx.page.setViewport(390, 844, { mobile: true });
  await ctx.page.waitFor(
    "!!document.querySelector('canvas.demo-canvas')", (v) => v === false,
    { what: "narrow frees the live demo", timeout: 6000 }
  );
  ctx.assert(sc, "narrow shows the gallery",
    (await ctx.page.evaluate("getComputedStyle(document.querySelector('.phone-gallery')).display")) === "flex");
  await ctx.page.setViewport(1440, 900, {});
  await ctx.page.waitFor("window.done === true", (v) => v === true, {
    what: "wide brings the live demo back", timeout: 25000,
  });
  const wasm = ctx.netNew((u) => u.endsWith(".wasm"));
  ctx.assert(sc, "regrowth reused the cached module (one wasm fetch total)",
    wasm.length === 1, `${wasm.length} fetches`);
  ctx.assert(sc, "the visitor's theme survived",
    (await ctx.page.evaluate("window.demo.theme()")) === "nord");
}

async function fullscreen(ctx, sc) {
  await liveOnDesktop(ctx);
  await ctx.page.click("canvas.demo-canvas");
  await ctx.page.click('[data-tool="fullscreen"]');
  let granted = false;
  try {
    await ctx.page.waitFor(
      "document.fullscreenElement === document.querySelector('.panel')",
      (v) => v === true, { what: "fullscreen granted", timeout: 4000 }
    );
    granted = true;
  } catch { /* headless commonly denies it */ }
  if (!granted) {
    const told = await ctx.page.waitFor(
      "document.getElementById('demo-controls-status').textContent",
      (t) => t.includes("Fullscreen isn't available"), {
        what: "graceful refusal shown", timeout: 4000,
      }
    );
    ctx.assert(sc, "refusal shown, no hang", told.includes("Fullscreen isn't available"), told);
    ctx.assert(sc, "button stays unpressed",
      (await ctx.page.evaluate(
        "document.querySelector('[data-tool=fullscreen]').getAttribute('aria-pressed')")) === "false");
    ctx.skip(sc, "headless Chrome denied native fullscreen: the graceful path is asserted, " +
      "but native fullscreen itself is NOT verified here (see MANUAL_TESTS)");
    return;
  }
  // fullscreenchange → syncFsBtn lands a beat after the grant: wait for it.
  await ctx.page.waitFor(
    "document.querySelector('[data-tool=fullscreen]').getAttribute('aria-pressed')",
    (v) => v === "true", { what: "button pressed while fullscreen", timeout: 4000 }
  );
  ctx.assert(sc, "button pressed while fullscreen", true);
  await ctx.page.evaluate("document.exitFullscreen()");
  await ctx.page.waitFor(
    "document.fullscreenElement === null", (v) => v === true, { timeout: 4000 }
  );
  // wait for the page's fullscreenchange handler to settle, not a fixed sleep
  await ctx.page.waitFor(
    "document.querySelector('.panel .frame').classList.contains('live')",
    (v) => v === true, { what: "demo still live after exiting fullscreen", timeout: 4000 }
  );
  ctx.assert(sc, "demo still live after exiting", true);
}
