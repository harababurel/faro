// Phone and coarse-pointer scenarios (WEBSITE.md §4.7, 4.6 chunk 2):
// CSS owns the phone/desktop call, so at 320/390/768 (fine pointer, the
// container query) and at 1200 with a coarse pointer the gallery shows
// and demo.js must never fetch anything under /demo/ — proven over an
// idle window longer than demo.js's requestIdleCallback timeout.
// Each scenario runs on its own fresh tab (see browser_test.mjs).
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
  ctx.note(sc, `pointer coarse per matchMedia: ${await ctx.page.evaluate("matchMedia('(pointer: coarse)').matches")}`);
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
