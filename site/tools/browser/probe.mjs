// Watches the CDP event stream for things that must never happen on the
// page: console errors, uncaught exceptions, external requests, cookies,
// HTTP >= 400. Warnings are collected separately: scenarios decide which
// (if any) are expected — never a global ignore.
export function newProbeState() {
  return { problems: [], warnings: [], net: [] };
}

// Schemes the browser itself produces; never http(s), never flagged.
const INTERNAL = ["about:", "data:", "chrome:", "chrome-untrusted:", "devtools:", "blob:", "javascript:"];
export const isInternalScheme = (url) => INTERNAL.some((s) => url.startsWith(s));

// http(s) requests whose origin isn't one of ours: the one network rule.
export function foreignOrigins(urls, ours) {
  const bad = new Set();
  for (const url of urls) {
    if (isInternalScheme(url)) continue;
    let parsed;
    try { parsed = new URL(url); } catch { bad.add(`unparseable: ${url}`); continue; }
    if (parsed.protocol === "http:" || parsed.protocol === "https:") {
      if (!ours.has(parsed.origin)) bad.add(parsed.origin);
    }
  }
  return [...bad];
}

// Exact-origin test: parsed, never a bare prefix (a port could prefix-
// match another port). http(s) URLs only; internal schemes are separate.
export function isOurOrigin(url, origin) {
  try { return new URL(url).origin === origin; } catch { return false; }
}

export function attachProbe(cdp, state, origin) {
  cdp.on("Runtime.consoleAPICalled", ({ type, args }) => {
    const text = (args ?? []).map((a) => a.value ?? a.description ?? "").join(" ");
    if (type === "error") state.problems.push(`console.error: ${text}`);
    else if (type === "warning") state.warnings.push(text);
  });
  cdp.on("Runtime.exceptionThrown", (p) => {
    const d = p.exceptionDetails ?? {};
    const detail = (d.exception && (d.exception.description || d.exception.value)) || d.text;
    state.problems.push(`uncaught: ${detail}`);
  });
  cdp.on("Network.requestWillBeSent", (p) => {
    const url = p.request.url;
    state.net.push({ url, when: Date.now() });
    if (!isInternalScheme(url) && !isOurOrigin(url, origin)) {
      state.problems.push(`external request: ${url}`);
    }
  });
  cdp.on("Network.responseReceived", (p) => {
    const headers = p.response.headers ?? {};
    const cookie = headers["set-cookie"] ?? headers["Set-Cookie"];
    if (cookie) state.problems.push(`Set-Cookie from ${p.response.url}`);
    if (p.response.status >= 400) state.problems.push(`HTTP ${p.response.status}: ${p.response.url}`);
  });
}

// Take the problems recorded since the last drain (fail-fast per scenario);
// warnings accumulate for the audit scenario to judge as a whole.
export function drainProblems(state) {
  return state.problems.splice(0);
}
