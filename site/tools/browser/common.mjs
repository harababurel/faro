// Helpers shared by the scenario modules: page probes, bounded waits,
// the idle-proof window that proves demo.js never fetched, and the
// addScriptToEvaluateOnNewDocument shims (each runs before the page's
// own deferred scripts, in document order).
export const sleep = (ms) => new Promise((go) => setTimeout(go, ms));
export const CANARY = "ci-canary-7f2a9c";
export const SAFE_INSPECT =
  "(() => { try { return window.demo ? window.demo.inspect() : null; } catch (e) { return null; } })()";
export const TRACKER = "JSON.stringify(window.__plausibleCaptured || [])";

export const parseInspect = (raw) => {
  if (!raw) return null;
  const m = raw.match(/focused=(\w+) open=(\w+) focus=(\S+)/);
  if (!m) return null;
  return { focused: m[1] === "true", open: m[2] === "true", focus: m[3], raw };
};

export const events = (tracks, name) => tracks.filter((t) => t.name === name);

export async function waitLoad(page, timeout = 15000) {
  await page.waitFor("document.readyState", (s) => s === "complete", {
    what: "page load", timeout,
  });
}

// demo.js reads the manifest from requestIdleCallback(read, {timeout: 3000})
// on load, so a quiet window of load + 3.4 s with no /demo/* request proves
// the loader stayed off — shorter than that proves nothing.
export const IDLE_PROOF_MS = 3400;
export async function idleProof(ctx, ms = IDLE_PROOF_MS) {
  await waitLoad(ctx.page);
  await sleep(ms);
  return ctx.netNew((u) => u.includes("/demo/"));
}

// Shims. The pointer shim lives in cdp.mjs (it also records what the
// browser really reported); these complete the set.
export const SAVE_DATA_SHIM = `Object.defineProperty(navigator, "connection", {
  value: { saveData: true, effectiveType: "4g", addEventListener: function () {} },
  configurable: true,
});`;
export const OPT_OUT_SHIM = `try { localStorage.setItem("plausible_ignore", "true"); } catch (e) {}`;
export const siteThemeShim = (name) =>
  `try { localStorage.setItem("faro-site-theme", ${JSON.stringify(name)}); } catch (e) {}`;
export const joinInit = (...parts) => parts.filter(Boolean).join("\n");

// Gallery assertions shared by the phone widths.
export async function assertGallery(ctx, sc, page) {
  ctx.assert(sc, "gallery is displayed",
    (await page.evaluate("getComputedStyle(document.querySelector('.phone-gallery')).display")) === "flex");
  ctx.assert(sc, "phone note is displayed",
    (await page.evaluate("getComputedStyle(document.querySelector('.phone-note')).display")) !== "none");
  ctx.assert(sc, "three gallery figures",
    (await page.evaluate("document.querySelectorAll('.phone-gallery figure').length")) === 3);
  const figure = await page.evaluate(
    "Math.round(document.querySelector('.phone-gallery figure').getBoundingClientRect().width)"
  );
  ctx.assert(sc, "figure is at least the 358px clamp", figure >= 358, `${figure}px`);
  const overflow = await page.evaluate(
    "document.documentElement.scrollWidth - document.documentElement.clientWidth"
  );
  ctx.assert(sc, "no horizontal page overflow", overflow <= 2, `${overflow}px`);
  ctx.assert(sc, "no demo canvas",
    !(await page.evaluate("!!document.querySelector('canvas.demo-canvas')")));
}
