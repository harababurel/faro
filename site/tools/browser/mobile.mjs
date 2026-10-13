// Phone and coarse-pointer scenarios (WEBSITE.md §4.7, 4.6 chunk 2):
// CSS owns the phone/desktop call, so at 320/390/768 (the container
// query) and at 1200 with no fine pointer anywhere (any-pointer) the
// gallery shows and demo.js must never fetch anything under /demo/ —
// proven over an idle window longer than demo.js's requestIdleCallback
// timeout. A coarse PRIMARY pointer with a fine pointer somewhere (an
// iPad with a trackpad keyboard) is a laptop: the demo autostarts.
// Each scenario runs on its own fresh tab (see browser_test.mjs).
import { POINTER_IPAD_SHIM } from "./cdp.mjs";
import { assertGallery, idleProof, siteThemeShim } from "./common.mjs";

export function mobileScenarios() {
  return [
    { name: "phone 320: gallery only, no /demo/* fetch in the idle window",
      viewport: { width: 320, height: 700, mobile: true }, run: galleryOnly },
    { name: "phone 390: gallery only, no /demo/* fetch in the idle window",
      viewport: { width: 390, height: 844, mobile: true }, run: galleryOnly },
    { name: "tablet 768: gallery only, no /demo/* fetch in the idle window",
      viewport: { width: 768, height: 900 }, run: galleryOnly },
    { name: "coarse pointer 1200: gallery only, no /demo/* fetch",
      viewport: { width: 1200, height: 800, touch: true }, run: galleryOnly },
    { name: "coarse primary + fine any-pointer 1200 (iPad with trackpad): live demo autostarts",
      viewport: { width: 1200, height: 800 }, init: POINTER_IPAD_SHIM, run: ipadLive },
    { name: "gallery: stored dark wins while the OS says light",
      viewport: { width: 390, height: 844, mobile: true },
      media: { "prefers-color-scheme": "light" }, init: siteThemeShim("dark"), run: darkOverride },
    { name: "gallery: arrow keys pan the scroller natively",
      viewport: { width: 390, height: 844, mobile: true }, run: arrowPan },
  ];
}

async function galleryOnly(ctx, sc) {
  await ctx.page.goto(`${ctx.origin}/`);
  const fetched = await idleProof(ctx);
  ctx.assert(sc, "no /demo/* request through the idle deadline",
    fetched.length === 0, fetched.join(", "));
  await assertGallery(ctx, sc, ctx.page);
  // The query that drives both the CSS gallery rule and demo.js: no fine
  // pointer anywhere (natively emulated, no shim).
  ctx.assert(sc, "no fine pointer per matchMedia",
    await ctx.page.evaluate("!matchMedia('(any-pointer: fine)').matches"),
    `any-pointer fine: ${await ctx.page.evaluate("matchMedia('(any-pointer: fine)').matches")}`);
  ctx.note(sc, `pointer coarse per matchMedia: ${await ctx.page.evaluate("matchMedia('(pointer: coarse)').matches")}`);
}

// iPad with a Magic Keyboard: the shim answers the JS pointer queries as
// on an iPad (primary coarse, a fine trackpad somewhere). Headless can't
// emulate pointer at the CSS level (it reports none, which stays desktop
// like a laptop), so this proves demo.js's loader decision; the CSS rule
// for a real iPad is the MANUAL_TESTS item. No touch emulation: it would
// force CSS pointer:coarse true and the gallery, which the shim can't
// reach. A laptop width, so the demo autostarts and the gallery hides.
async function ipadLive(ctx, sc) {
  await ctx.page.goto(`${ctx.origin}/`);
  await ctx.page.waitFor("window.done === true", (v) => v === true, {
    what: "demo live (window.done)", timeout: 25000,
  });
  ctx.assert(sc, "primary pointer really coarse, any-pointer fine (shim)",
    await ctx.page.evaluate(
      "matchMedia('(pointer: coarse)').matches && matchMedia('(any-pointer: fine)').matches"
    ));
  ctx.assert(sc, "frame has .live",
    await ctx.page.evaluate("document.querySelector('.panel .frame').classList.contains('live')"));
  ctx.assert(sc, "gallery is not displayed",
    (await ctx.page.evaluate("getComputedStyle(document.querySelector('.phone-gallery')).display")) === "none");
  const tracks = await ctx.tracker();
  const started = ctx.count(tracks, "Demo started");
  ctx.assert(sc, "Demo started exactly once, how=auto",
    started.length === 1 && started[0].props?.how === "auto", JSON.stringify(started));
  ctx.note(sc, "pointer queries via iPad shim (primary coarse, any-pointer fine) — emulation");
}

async function darkOverride(ctx, sc) {
  await ctx.page.goto(`${ctx.origin}/`);
  await ctx.page.waitFor("document.readyState", (s) => s === "complete", { timeout: 10000 });
  ctx.assert(sc, "OS really says light",
    await ctx.page.evaluate("matchMedia('(prefers-color-scheme: light)').matches"));
  ctx.assert(sc, "stored dark override applied",
    (await ctx.page.evaluate("document.documentElement.getAttribute('data-theme')")) === "dark");
  await assertGallery(ctx, sc, ctx.page);
}

async function arrowPan(ctx, sc) {
  await ctx.page.goto(`${ctx.origin}/`);
  await ctx.page.waitFor("document.readyState", (s) => s === "complete", { timeout: 10000 });
  await ctx.page.click(".phone-gallery");
  const before = await ctx.page.evaluate("document.querySelector('.phone-gallery').scrollLeft");
  await ctx.page.key("ArrowRight");
  await ctx.page.key("ArrowRight");
  const after = await ctx.page.waitFor(
    "document.querySelector('.phone-gallery').scrollLeft",
    (v) => v > before, { what: "arrow keys panned the gallery", timeout: 3000 }
  );
  ctx.assert(sc, "gallery scrolled right", after > before, `${before} -> ${after}`);
}
