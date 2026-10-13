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
  // On failure, name up to 3 outermost elements past the right edge so
  // CI reports the culprit instead of just the pixel count. Elements
  // inside a horizontal scroller (overflow-x auto/scroll/hidden/clip,
  // other than html/body) are expected to extend past the viewport.
  const overflow = await page.evaluate(`(() => {
    const de = document.documentElement;
    const over = de.scrollWidth - de.clientWidth;
    if (over <= 2) return { over, bad: [] };
    const limit = de.clientWidth;
    const clipped = (e) => {
      for (let p = e.parentElement; p && p !== document.body; p = p.parentElement) {
        if (["auto", "scroll", "hidden", "clip"].includes(getComputedStyle(p).overflowX)) return true;
      }
      return false;
    };
    const bad = [];
    for (const el of document.querySelectorAll("body *")) {
      const r = el.getBoundingClientRect();
      if (r.right <= limit + 0.5 || r.width === 0 || r.height === 0) continue;
      if (clipped(el)) continue;
      if (bad.some((seen) => seen.contains(el))) continue; // inner: keep the outer
      for (let i = bad.length - 1; i >= 0; i--) if (el.contains(bad[i])) bad.splice(i, 1);
      bad.push(el);
      if (bad.length === 3) break;
    }
    const name = (el) => {
      const id = el.id ? "#" + el.id : "";
      const cls = typeof el.className === "string" && el.className
        ? "." + el.className.trim().split(/\\s+/)[0] : "";
      return el.tagName.toLowerCase() + id + cls + " right=" + Math.round(el.getBoundingClientRect().right);
    };
    return { over, bad: bad.map(name) };
  })()`);
  ctx.assert(sc, "no horizontal page overflow", overflow.over <= 2,
    `${overflow.over}px${overflow.bad.length ? " — " + overflow.bad.join(", ") : ""}`);
  ctx.assert(sc, "no demo canvas",
    !(await page.evaluate("!!document.querySelector('canvas.demo-canvas')")));
}
