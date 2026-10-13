// Headless browser tests for the built getfaro.dev page (WEBSITE.md §6.8,
// M13 4.6). Node >= 22 builtins only: no npm packages, no Playwright.
//
//   node site/tools/browser_test.mjs <built site dir> [--out DIR] [--budget SECS]
//
// CHROME names the browser (CI: $(command -v google-chrome)); missing →
// exit 2. The built root is validated first (index.html, js/init.js, the
// manifest's js+wasm). A local server serves it with the nginx headers
// parsed from site/deploy/nginx; /js/s.js and /api/event are recording
// stubs. The desktop core suite runs as one page flow; every other
// scenario gets its own fresh tab (new CDP target, storage cleared after)
// so emulation, init scripts and localStorage can't leak between them.
// The report (JSON, per assert) and up to 8 screenshots land in --out
// (default: a scratch dir). Exit: 0 all green, 1 any failure or no
// asserts ran, 2 timeout or harness/browser trouble.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { startServer, nginxHeaders } from "./browser/server.mjs";
import { launchChrome, pageTarget, httpBase, createTarget, closeTarget } from "./browser/chrome.mjs";
import { Cdp, Page, POINTER_FINE_SHIM, bound } from "./browser/cdp.mjs";
import { newProbeState, attachProbe, drainProblems, foreignOrigins } from "./browser/probe.mjs";
import { parseArgs } from "./browser/args.mjs";
import { TRACKER } from "./browser/common.mjs";
import { desktopCore } from "./browser/scenarios.mjs";
import { mobileScenarios } from "./browser/mobile.mjs";
import { privacyScenarios } from "./browser/privacy.mjs";
import { controlScenarios } from "./browser/controls.mjs";

const HERE = fileURLToPath(new URL(".", import.meta.url));
let parsed;
try {
  parsed = parseArgs(process.argv.slice(2));
} catch (error) {
  console.log(`args: ${error.message}`);
  process.exit(2);
}
const BUILT = join(parsed.built);
const BUDGET = parsed.budgetSec * 1000;

const startedAt = Date.now();
const report = {
  ok: false, exit: 2, durationMs: 0, timedOut: false,
  asserts: 0, failed: 0, scenarios: [], skips: [], warnings: [], problems: [], screenshots: [],
};
try {
  report.outDir = parsed.out ?? mkdtempSync(join(tmpdir(), "faro-browser-"));
  mkdirSync(report.outDir, { recursive: true });
} catch (error) { // before finish() exists: report where, then fail
  console.log(`out dir: ${error.message}`);
  process.exit(2);
}
const children = new Set(); // chrome procs we own, killed on any exit path
let profileDir = null; // our scratch chrome profile, removed on any exit path
const cleanup = () => {
  for (const child of children) { try { child.kill("SIGKILL"); } catch { /* gone */ } }
  // Full Chrome's child procs outlive the kill briefly and keep writing
  // into the profile; retry, and never let cleanup itself fail the run:
  // main's finally calls cleanup again once chrome has really exited,
  // and process.on("exit") covers every other path. Owned path only.
  if (profileDir) {
    try {
      rmSync(profileDir, { recursive: true, force: true, maxRetries: 25, retryDelay: 200 });
    } catch { /* retried from the finally / exit hook */ }
  }
};
const finish = (code, why) => {
  report.exit = code;
  report.ok = code === 0;
  report.durationMs = Date.now() - startedAt;
  if (why) report.error = why;
  cleanup();
  try {
    writeFileSync(join(report.outDir, "report.json"), JSON.stringify(report, null, 1));
  } catch (e) { console.log("report write failed:", e.message); }
  console.log(`\n${report.asserts} asserts, ${report.failed} failed, ` +
    `${report.scenarios.filter((s) => s.status === "pass").length} pass, ` +
    `${report.scenarios.filter((s) => s.status === "skip").length} skip of ` +
    `${report.scenarios.length} scenarios in ${(report.durationMs / 1000).toFixed(1)}s — ` +
    `exit ${code}` + (why ? ` (${why})` : ""));
  console.log(`report: ${join(report.outDir, "report.json")}`);
  process.exitCode = code;
};

function validateBuilt(dir) {
  if (!dir || !existsSync(dir)) return `built site dir not found: ${dir}`;
  for (const need of ["index.html", "js/init.js", "demo/manifest.json", "demo/manifest.no-preload.json"]) {
    if (!existsSync(join(dir, need))) return `built site is missing ${need}`;
  }
  let manifest;
  try {
    manifest = JSON.parse(readFileSync(join(dir, "demo/manifest.json"), "utf8"));
  } catch (error) {
    return `demo/manifest.json is not readable JSON: ${error.message}`;
  }
  if (!manifest || typeof manifest.js !== "string" || typeof manifest.wasm !== "string") {
    return "demo/manifest.json has no js/wasm asset paths";
  }
  for (const asset of [manifest.js, manifest.wasm]) {
    if (!existsSync(join(dir, asset.replace(/^\//, "")))) return `manifest asset missing on disk: ${asset}`;
  }
  return null;
}

function findChrome() {
  if (process.env.CHROME && existsSync(process.env.CHROME)) return process.env.CHROME;
  const which = spawnSync("which", ["google-chrome"], { encoding: "utf8" });
  if (which.status === 0) return which.stdout.trim();
  return null;
}

async function main() {
  if (Number(process.versions.node.split(".")[0]) < 22) {
    finish(2, `node >= 22 required, found ${process.versions.node}`);
    return;
  }
  const bad = validateBuilt(BUILT);
  if (bad) { finish(2, bad); return; }
  const chrome = findChrome();
  if (!chrome) { finish(2, "no browser: set CHROME to a Chrome binary (CI: google-chrome)"); return; }
  const profile = mkdtempSync(join(tmpdir(), "faro-chrome-")); // owned scratch only
  profileDir = profile;
  const state = newProbeState();
  const servers = [];
  const serverOrigins = new Set(); // every origin we ever served on, kept after closes
  let chromeProc = null, mainCdp = null, watchdog = null, shots = 0, nextN = 0;
  const headers = nginxHeaders(join(HERE, "..", "deploy", "nginx"));

  const makeCtx = (page, server) => {
    let mark = 0;
    return {
      page, state, server, origin: server.origin,
      markNet: () => { mark = state.net.length; },
      netNew: (pred) => state.net.slice(mark).map((r) => r.url).filter(pred ?? (() => true)),
      assert(sc, what, cond, actual) {
        report.asserts += 1;
        const pass = cond === true;
        if (!pass) { report.failed += 1; if (sc.status === "pass") sc.status = "fail"; }
        sc.asserts.push({ what, pass, actual: bound(actual ?? "") });
      },
      note(sc, msg) { sc.notes.push(msg); },
      skip(sc, why) {
        if (sc.status === "pass") sc.status = "skip";
        report.skips.push({ n: sc.n, name: sc.name, why });
      },
      tracker: async () => JSON.parse(await page.evaluate(TRACKER)),
      count: (tracks, name) => tracks.filter((t) => t.name === name),
      netAll: (pred) => state.net.map((r) => r.url).filter(pred ?? (() => true)),
    };
  };

  const reportScenario = async (sc, ctx) => {
    for (const a of sc.asserts) {
      if (!a.pass) console.log(`  FAIL ${a.what}${a.actual ? ` — ${a.actual}` : ""}`);
    }
    console.log(`  ${sc.status.toUpperCase()} (${sc.asserts.filter((a) => a.pass).length}/${sc.asserts.length} asserts)`);
    if (sc.status === "fail" || sc.status === "error") {
      if (shots < 8) {
        try {
          const file = join(report.outDir, `${String(sc.n).padStart(2, "0")}-${sc.name.replace(/[^\w]+/g, "_")}.png`);
          await ctx.page.screenshot(file);
          report.screenshots.push(file);
          shots += 1;
        } catch { /* browser may be gone */ }
      }
    }
  };

  const runInto = async (scenario, ctx, sc) => {
    if (!report.scenarios.includes(sc)) report.scenarios.push(sc);
    if (!sc.headerPrinted) {
      sc.headerPrinted = true;
      console.log(`\n[${sc.n}] ${sc.name}`);
    }
    ctx.markNet();
    try {
      await scenario.run(ctx, sc);
      const problems = drainProblems(state);
      if (problems.length) ctx.assert(sc, "no console/network problems during scenario", false, problems.join(" | "));
    } catch (error) {
      if (sc.status === "pass") sc.status = "error";
      sc.error = bound(error.message);
      report.failed += 1;
    }
    await reportScenario(sc, ctx);
  };

  try {
    const main = await startServer(BUILT, headers);
    servers.push(main);
    serverOrigins.add(main.origin);
    const launched = await launchChrome(chrome, profile);
    chromeProc = launched.proc;
    children.add(chromeProc);
    const target = await pageTarget(launched.wsUrl);
    mainCdp = await Cdp.connect(target.webSocketDebuggerUrl);
    attachProbe(mainCdp, state, main.origin);
    const corePage = new Page(mainCdp);
    await corePage.enable(POINTER_FINE_SHIM);
    // Full Chrome (headless=new) shrinks --window-size to a smaller
    // viewport (1440x900 → 780x437), which would flip the desktop core
    // flow into gallery mode. Pin the desktop viewport explicitly;
    // scenarios that set their own metrics override it per page.
    await corePage.setViewport(1440, 900);

    watchdog = setTimeout(() => {
      report.timedOut = true;
      try { chromeProc.kill("SIGKILL"); } catch { /* already gone */ }
      mainCdp.close(); // Node's WebSocket won't notice the killed browser itself
    }, BUDGET);

    // 1) the desktop core flow, one page session in order
    for (const scenario of desktopCore()) {
      const sc = { n: ++nextN, name: scenario.name, status: "pass", asserts: [], notes: [] };
      await runInto(scenario, makeCtx(corePage, main), sc);
      if (report.timedOut) break;
    }

    // 2) isolated scenarios: fresh tab, fresh CDP session, storage cleared after
    const base = httpBase(launched.wsUrl);
    for (const def of [...mobileScenarios(), ...privacyScenarios(), ...controlScenarios()]) {
      if (report.timedOut) break;
      const sc = { n: ++nextN, name: def.name, status: "pass", asserts: [], notes: [] };
      report.scenarios.push(sc);
      console.log(`\n[${sc.n}] ${sc.name}`);
      sc.headerPrinted = true;
      let tab = null, cdp = null, server = main;
      try {
        if (def.manifestVariant) {
          server = await startServer(BUILT, headers, { manifestVariant: def.manifestVariant });
          servers.push(server);
          serverOrigins.add(server.origin);
        }
        tab = await createTarget(base);
        cdp = await Cdp.connect(tab.webSocketDebuggerUrl);
        attachProbe(cdp, state, server.origin);
        const page = new Page(cdp);
        await page.enable(def.init ?? null);
        if (def.scriptsDisabled) await page.setScriptsDisabled(true);
        if (def.viewport) await page.setViewport(def.viewport.width, def.viewport.height, def.viewport);
        else await page.setViewport(1440, 900); // same desktop baseline as the core page
        if (def.media) await page.setMedia(def.media);
        await runInto(def, makeCtx(page, server), sc);
        await page.clearStorage(server.origin);
      } catch (error) { // target setup itself failed: record and move on
        if (sc.status === "pass") sc.status = "error";
        sc.error = bound(error.message);
        report.failed += 1;
        await reportScenario(sc, { page: { screenshot: async () => {} } });
      } finally {
        if (cdp) cdp.close();
        if (tab) await closeTarget(base, tab.id);
        if (server !== main) await server.close(); // kept in servers for the audit
      }
    }

    // 3) the global audit: everything seen across every tab and server
    if (!report.timedOut) {
      const audit = {
        n: ++nextN, name: "audit: same-origin everywhere, no cookies, no warnings",
        status: "pass", asserts: [], notes: [],
      };
      const ctx = makeCtx(corePage, main);
      const foreign = foreignOrigins(state.net.map((r) => r.url), serverOrigins);
      ctx.assert(audit, "every http(s) request in every scenario stayed on a harness server",
        foreign.length === 0, foreign.join(", "));
      const all = servers.flatMap((s) => s.requests);
      const cookieSent = all.filter((r) => r.cookie);
      ctx.assert(audit, "no cookie header ever reached a server",
        cookieSent.length === 0, cookieSent.map((r) => r.url).join(", "));
      // The site makes no writes except the Plausible endpoint: any other
      // POST/PUT is a leak channel on its own, whatever the body holds.
      const writes = all.filter((r) =>
        !["GET", "HEAD", "OPTIONS"].includes(r.method) && !r.url.startsWith("/api/event"));
      ctx.assert(audit, "no request wrote a body except /api/event",
        writes.length === 0, writes.map((r) => `${r.method} ${r.url}`).join(", "));
      ctx.assert(audit, "no console warnings anywhere",
        state.warnings.length === 0, state.warnings.join(" | "));
      ctx.assert(audit, "no problems left over",
        state.problems.length === 0, state.problems.join(" | "));
      report.scenarios.push(audit);
      console.log(`\n[${audit.n}] ${audit.name}`);
      await reportScenario(audit, ctx);
    }

    report.warnings = state.warnings;
    report.problems = state.problems;
    if (report.timedOut) { finish(2, `global budget of ${BUDGET / 1000}s hit`); return; }
    if (report.asserts === 0) { finish(1, "no asserts ran"); return; }
    finish(report.failed > 0 ? 1 : 0);
  } catch (error) {
    finish(2, `harness: ${error.message}`);
  } finally {
    clearTimeout(watchdog);
    if (mainCdp) mainCdp.close();
    if (chromeProc && chromeProc.exitCode === null) {
      try { chromeProc.kill("SIGKILL"); } catch { /* already gone */ }
      await new Promise((gone) => {
        chromeProc.once("exit", gone);
        setTimeout(gone, 3000).unref();
      });
    }
    for (const server of servers) await server.close();
    cleanup(); // chrome has exited by now: the profile delete can finish
  }
}

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    report.timedOut = true;
    finish(2, `interrupted by ${signal}`);
    process.exit(2);
  });
}
// last resort: even an unexpected crash must not orphan chrome or its profile
process.on("exit", cleanup);

// anything escaping main() still gets a report, a nonzero exit and cleanup
main().catch((error) => finish(2, `harness: ${error.message}`));
