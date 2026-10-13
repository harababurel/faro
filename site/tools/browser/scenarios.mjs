// The desktop core suite (WEBSITE.md §6.8 headless checks, 4.6 chunk 1),
// one page session in order: autostart unfocused → click to focus → type
// and send a canary → Esc releases → restart keeps one wasm fetch and
// dedups the funnel → a theme chip commits once. Every wait is bounded
// polling; the canary must never reach the tracker stub or any request.
import { CANARY, SAFE_INSPECT, TRACKER, parseInspect } from "./common.mjs";
import { foreignOrigins } from "./probe.mjs";

const sleep = (ms) => new Promise((go) => setTimeout(go, ms));

export function desktopCore() {
  return [
    { n: 1, name: "autostart: live canvas and grid, unfocused, Demo started once", run: autostart },
    { n: 2, name: "click focuses: faro focused, frame.focused, Demo focused once", run: focus },
    { n: 3, name: "canary send: message sent once, no payload leak, no cookies", run: sendCanary },
    { n: 4, name: "Esc releases focus with the funnel not refired", run: escReleases },
    { n: 5, name: "restart: one wasm fetch, fresh history, no funnel dupes", run: restart },
    { n: 6, name: "theme chip: committed change tracked once, same choice silent", run: themeChip },
    { n: 7, name: "audit: no console errors or warnings, no network violations", run: audit },
  ];
}

async function autostart(ctx, sc) {
  await ctx.page.goto(`${ctx.origin}/`);
  await ctx.page.waitFor("window.done === true", (v) => v === true, {
    what: "demo live (window.done)", timeout: 25000,
  });
  ctx.assert(sc, "canvas.demo-canvas exists",
    await ctx.page.evaluate("!!document.querySelector('canvas.demo-canvas')"));
  ctx.assert(sc, "frame has .live",
    await ctx.page.evaluate("document.querySelector('.panel .frame').classList.contains('live')"));
  const text = await ctx.page.evaluate("window.demo.text()");
  const rows = text.replace(/\n+$/, "").split("\n").length;
  ctx.assert(sc, "grid is 34 rows", rows === 34, `${rows} rows`);
  const ins = parseInspect(await ctx.page.evaluate(SAFE_INSPECT));
  ctx.assert(sc, "autostart leaves faro unfocused", ins && !ins.focused, ins?.raw);
  // the web demo opens a chat programmatically at startup (D125), never as user input
  ctx.assert(sc, "a chat is open at startup (programmatic)", ins && ins.open, ins?.raw);
  const tracks = await ctx.tracker();
  const started = ctx.count(tracks, "Demo started");
  ctx.assert(sc, "Demo started exactly once, how=auto",
    started.length === 1 && started[0].props?.how === "auto", JSON.stringify(started));
  ctx.assert(sc, "no Demo focused yet", ctx.count(tracks, "Demo focused").length === 0);
  const native = await ctx.page.evaluate("String(window.__pointerNative)");
  ctx.note(sc, `pointer:fine via JS shim (browser native said ${native}) — emulation, not a native pointer check`);
}

async function focus(ctx, sc) {
  await ctx.page.click("canvas.demo-canvas");
  const raw = await ctx.page.waitFor(SAFE_INSPECT, (s) => s && /focused=true/.test(s), {
    what: "faro focused after canvas click", timeout: 4000,
  });
  ctx.assert(sc, "frame has .focused",
    await ctx.page.evaluate("document.querySelector('.panel .frame').classList.contains('focused')"));
  ctx.assert(sc, "Demo focused exactly once", ctx.count(await ctx.tracker(), "Demo focused").length === 1, raw);
}

async function sendCanary(ctx, sc) {
  // The canvas click left faro on the Messages pane; Tab walks to the input.
  for (let i = 0; i < 3 && !/focus=Input/.test(await ctx.page.evaluate(SAFE_INSPECT) ?? ""); i++) {
    await ctx.page.key("Tab");
    await ctx.page.waitFor(SAFE_INSPECT, (s) => s && /focus=Input|focus=Messages/.test(s), {
      what: "Tab moved faro's pane focus", timeout: 3000,
    });
  }
  const focusedInput = /focus=Input/.test(await ctx.page.evaluate(SAFE_INSPECT) ?? "");
  ctx.assert(sc, "Tab reached the input pane", focusedInput, await ctx.page.evaluate(SAFE_INSPECT));
  for (const ch of CANARY) await ctx.page.key(ch === " " ? "Space" : ch);
  await ctx.page.waitFor("window.demo.text()", (t) => t.includes(CANARY), {
    what: "canary typed into faro", timeout: 5000,
  });
  await ctx.page.key("Enter");
  await ctx.page.waitFor(TRACKER, (s) => s.includes("Demo message sent"), {
    what: "Demo message sent event", timeout: 8000,
  });
  const tracks = await ctx.tracker();
  ctx.assert(sc, "Demo message sent exactly once", ctx.count(tracks, "Demo message sent").length === 1);
  const leaked = JSON.stringify(tracks).includes(CANARY);
  ctx.assert(sc, "canary never reaches the tracker", !leaked, leaked ? "found in tracker calls" : "clean");
  const reqLeak = ctx.server.requests.filter(
    (r) => r.url.includes(CANARY) || String(r.body).includes(CANARY)
  );
  ctx.assert(sc, "canary never reaches any request", reqLeak.length === 0,
    reqLeak.map((r) => r.url).join(", "));
  const apiHits = ctx.server.requests.filter((r) => r.url.startsWith("/api/event")).length;
  ctx.assert(sc, "no real telemetry POST (/api/event hit 0 times)", apiHits === 0, `${apiHits} hits`);
  ctx.assert(sc, "document.cookie is empty",
    (await ctx.page.evaluate("document.cookie")) === "");
}

async function escReleases(ctx, sc) {
  let ins = parseInspect(await ctx.page.evaluate(SAFE_INSPECT));
  for (let i = 0; i < 4 && ins && ins.focused; i++) {
    await ctx.page.key("Escape");
    try {
      await ctx.page.waitFor(SAFE_INSPECT, (s) => s && /focused=false/.test(s), {
        what: `Esc released focus (press ${i + 1})`, timeout: 2000,
      });
    } catch { /* faro still working through its dismissal chain */ }
    ins = parseInspect(await ctx.page.evaluate(SAFE_INSPECT));
  }
  ctx.assert(sc, "faro unfocused after Esc chain", ins && !ins.focused, ins?.raw);
  ctx.assert(sc, "frame lost .focused",
    !(await ctx.page.evaluate("document.querySelector('.panel .frame').classList.contains('focused')")));
  ctx.assert(sc, "Demo focused still exactly once",
    ctx.count(await ctx.tracker(), "Demo focused").length === 1);
}

async function restart(ctx, sc) {
  const wasmBefore = ctx.netAll((u) => u.endsWith(".wasm")).length;
  await ctx.page.evaluate("window.__oldCanvas = document.querySelector('canvas.demo-canvas')");
  await ctx.page.click('[data-tool="restart"]');
  const raw = await ctx.page.waitFor(
    SAFE_INSPECT,
    (s) => s && /focused=true/.test(s),
    { what: "restarted demo live and focused (restart lands keys)", timeout: 25000 }
  );
  ctx.assert(sc, "a new canvas replaced the old one",
    await ctx.page.evaluate("document.querySelector('canvas.demo-canvas') !== window.__oldCanvas"));
  const text = await ctx.page.evaluate("window.demo.text()");
  ctx.assert(sc, "fresh history: the canary is gone", !text.includes(CANARY));
  const wasmAfter = ctx.netAll((u) => u.endsWith(".wasm")).length;
  ctx.assert(sc, "restart fetched no extra wasm (module cache)", wasmAfter === wasmBefore,
    `before=${wasmBefore} after=${wasmAfter}`);
  ctx.assert(sc, "exactly one wasm fetch in the page", wasmAfter === 1, `${wasmAfter} fetches`);
  const tracks = await ctx.tracker();
  ctx.assert(sc, "Demo started still once across restart",
    ctx.count(tracks, "Demo started").length === 1);
  ctx.assert(sc, "Demo focused still once across restart",
    ctx.count(tracks, "Demo focused").length === 1, raw);
}

async function themeChip(ctx, sc) {
  const current = await ctx.page.evaluate("window.demo.theme()");
  const target = current === "nord" ? "faro-dark" : "nord";
  await ctx.page.click(`[data-theme="${target}"]`);
  await ctx.page.waitFor("window.demo.theme()", (t) => t === target, {
    what: `chip applies theme ${target}`, timeout: 5000,
  });
  ctx.assert(sc, "chip aria-pressed true after commit",
    (await ctx.page.evaluate(
      `document.querySelector('[data-theme="${target}"]').getAttribute("aria-pressed")`
    )) === "true");
  const changed = ctx.count(await ctx.tracker(), "Theme changed");
  ctx.assert(sc, "Theme changed once with the committed name",
    changed.length === 1 && changed[0].props?.theme === target, JSON.stringify(changed));
  await ctx.page.click(`[data-theme="${target}"]`); // same choice again
  await sleep(600); // negative check: bounded, not a positive wait
  ctx.assert(sc, "same theme again stays silent",
    ctx.count(await ctx.tracker(), "Theme changed").length === 1);
}

async function audit(ctx, sc) {
  ctx.assert(sc, "no console warnings anywhere in the suite",
    ctx.state.warnings.length === 0, ctx.state.warnings.join(" | "));
  const foreign = foreignOrigins(ctx.state.net.map((r) => r.url), new Set([ctx.origin]));
  ctx.assert(sc, "every http(s) request stayed on our server",
    foreign.length === 0, foreign.join(", "));
  ctx.assert(sc, "no cookie header reached the server",
    ctx.server.requests.every((r) => !r.cookie));
}
