// Panel-control scenarios (WEBSITE.md §4.3/§4.5, 4.6 chunk 2): motion
// preference and button, the theme palette (previews silent, Enter
// commits once), the wide→narrow→wide lifecycle (freed, then restored
// from the cached module with the visitor's theme), and fullscreen —
// where headless Chrome's answer is asserted either way, and a denial
// is reported as a skip, never as a pass.
import { POINTER_FINE_SHIM } from "./cdp.mjs";
import { SAFE_INSPECT, TRACKER } from "./common.mjs";

export function controlScenarios() {
  return [
    { name: "reduced-motion: honoured from the OS, the button overrides it",
      viewport: { width: 1440, height: 900 }, init: POINTER_FINE_SHIM,
      media: { "prefers-reduced-motion": "reduce" }, run: motion },
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
  ctx.assert(sc, "motion button reads off",
    (await ctx.page.evaluate(
      "document.querySelector('[data-tool=motion]').getAttribute('aria-pressed')")) === "false");
  await ctx.page.click("[data-tool=motion]");
  await ctx.page.waitFor("window.demo.reduce_motion()", (v) => v === false, {
    what: "button turns motion on", timeout: 4000,
  });
  // paintTools lands with the faro-settings event, a draw later
  const pressed = await ctx.page.waitFor(
    "document.querySelector('[data-tool=motion]').getAttribute('aria-pressed')",
    (v) => v === "true", { what: "motion button reads on", timeout: 4000 }
  );
  ctx.assert(sc, "motion button reads on", pressed === "true");
}

async function themePalette(ctx, sc) {
  await liveOnDesktop(ctx);
  await ctx.page.click("canvas.demo-canvas");
  await ctx.page.waitFor(SAFE_INSPECT, (s) => s && /focused=true/.test(s), { timeout: 4000 });
  const before = await ctx.page.evaluate("window.demo.configured_theme()");
  await ctx.page.click('[data-tool="themes"]'); // All…: faro's palette at Themes
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
  await ctx.page.click('[data-theme="nord"]');
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
