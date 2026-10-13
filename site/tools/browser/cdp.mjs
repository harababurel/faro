// Minimal CDP client over Node's built-in WebSocket (Node >= 22): every
// call's reply error or timeout rejects, events fan out to handlers.
// Page wraps it with the bounded-wait and input helpers the scenarios use
// (the key/mouse encoding follows crates/faro-web/tools/shot.mjs).
const sleep = (ms) => new Promise((go) => setTimeout(go, ms));
export const bound = (value) => {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return (text ?? "").slice(0, 200);
};

export class Cdp {
  #id = 0;
  #pending = new Map();
  #handlers = new Map();
  #dead = null;

  static async connect(wsUrl, timeoutMs = 15000) {
    const ws = new WebSocket(wsUrl);
    await new Promise((open, fail) => {
      const timer = setTimeout(() => fail(new Error("CDP websocket connect timed out")), timeoutMs);
      ws.addEventListener("open", () => { clearTimeout(timer); open(); }, { once: true });
      ws.addEventListener("error", () => { clearTimeout(timer); fail(new Error("CDP websocket error")); }, { once: true });
    });
    const cdp = new Cdp(ws);
    // Node's WebSocket can sit through a killed browser without noticing,
    // so close/error (and our own close()) must fail everything pending.
    const die = (why) => {
      cdp.#dead = why;
      for (const [, reply] of cdp.#pending) reply({ error: { message: why } });
      cdp.#pending.clear();
    };
    ws.addEventListener("message", (event) => {
      let msg;
      try {
        msg = JSON.parse(event.data);
      } catch {
        die("malformed CDP frame"); // a broken stream can't be trusted further
        return;
      }
      if (msg.id && cdp.#pending.has(msg.id)) {
        cdp.#pending.get(msg.id)(msg);
        cdp.#pending.delete(msg.id);
      } else if (msg.method && cdp.#handlers.has(msg.method)) {
        for (const fn of cdp.#handlers.get(msg.method)) fn(msg.params ?? {});
      }
    });
    ws.addEventListener("close", () => die("CDP connection closed"));
    ws.addEventListener("error", () => die("CDP connection error"));
    return cdp;
  }

  constructor(ws) { this.ws = ws; }
  on(method, fn) {
    if (!this.#handlers.has(method)) this.#handlers.set(method, []);
    this.#handlers.get(method).push(fn);
  }
  send(method, params = {}, timeoutMs = 15000) {
    if (this.#dead) return Promise.reject(new Error(this.#dead));
    return new Promise((resolve, reject) => {
      const id = ++this.#id;
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error(`CDP ${method} timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      this.#pending.set(id, (msg) => {
        clearTimeout(timer);
        if (msg.error) reject(new Error(`CDP ${method}: ${msg.error.message}`));
        else resolve(msg.result);
      });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
  close() {
    this.#dead = this.#dead ?? "CDP connection closed by the harness";
    try { this.ws.close(); } catch { /* already gone */ }
  }
}

const KEYS = {
  Enter: [13, "\r"], Escape: [27], Tab: [9], Backspace: [8], Delete: [46], Space: [32, " "],
  ArrowUp: [38], ArrowDown: [40], ArrowLeft: [37], ArrowRight: [39],
  PageUp: [33], PageDown: [34], Home: [36], End: [35],
};
const MODS = { alt: 1, ctrl: 2, meta: 4, shift: 8 };

export class Page {
  constructor(cdp) { this.cdp = cdp; }

  async enable(initScript) {
    await this.cdp.send("Page.enable");
    await this.cdp.send("Runtime.enable");
    await this.cdp.send("Network.enable");
    if (initScript) {
      await this.cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: initScript });
    }
  }
  async goto(url) { await this.cdp.send("Page.navigate", { url }); }

  // Viewport and media emulation (the phone widths, coarse pointer,
  // prefers-color-scheme / prefers-reduced-motion).
  async setViewport(width, height, { mobile = false, touch = false } = {}) {
    await this.cdp.send("Emulation.setDeviceMetricsOverride", {
      width, height, deviceScaleFactor: 1, mobile,
    });
    await this.cdp.send("Emulation.setTouchEmulationEnabled", touch
      ? { enabled: true, maxTouchPoints: 5 }
      : { enabled: false });
  }
  async setMedia(features) {
    await this.cdp.send("Emulation.setEmulatedMedia", {
      features: Object.entries(features).map(([name, value]) => ({ name, value })),
    });
  }
  async setScriptsDisabled(disabled) {
    await this.cdp.send("Emulation.setScriptExecutionDisabled", { value: disabled });
  }
  async clearStorage(origin) {
    try {
      await this.cdp.send("Storage.clearDataForOrigin", {
        origin, storageTypes: "local_storage,session_storage,cookies",
      });
    } catch { /* best effort between scenarios */ }
  }

  async evaluate(expression) {
    const reply = await this.cdp.send("Runtime.evaluate", {
      expression, returnByValue: true, awaitPromise: true,
    });
    if (reply.exceptionDetails) {
      const d = reply.exceptionDetails;
      throw new Error(`page eval threw: ${bound((d.exception && d.exception.description) || d.text)}`);
    }
    return reply.result ? reply.result.value : undefined;
  }

  // Bounded poll: never a fixed sleep for a positive wait.
  async waitFor(expression, predicate, { what = expression, timeout = 10000, every = 100 } = {}) {
    const until = Date.now() + timeout;
    for (;;) {
      const value = await this.evaluate(expression);
      if (predicate(value)) return value;
      if (Date.now() > until) {
        throw new Error(`wait failed (${what}): last=${bound(JSON.stringify(value))}`);
      }
      await sleep(every);
    }
  }

  async click(selector) {
    await this.evaluate(
      `(() => { const e = document.querySelector(${JSON.stringify(selector)});` +
      ` if (e) e.scrollIntoView({ block: "center" }); })()`
    );
    const at = await this.evaluate(
      `JSON.stringify((() => { const e = document.querySelector(${JSON.stringify(selector)});` +
      ` if (!e) return null; const r = e.getBoundingClientRect();` +
      ` return [Math.round(r.x + r.width / 2), Math.round(r.y + r.height / 2)]; })())`
    );
    if (at === null) throw new Error(`click: no element for ${selector}`);
    const [x, y] = JSON.parse(at);
    for (const type of ["mousePressed", "mouseReleased"]) {
      await this.cdp.send("Input.dispatchMouseEvent", { type, x, y, button: "left", clickCount: 1 });
    }
  }

  async key(name, mods = []) {
    const modifiers = mods.reduce((m, k) => m | MODS[k], 0);
    const [code, text] = KEYS[name] ?? [name.toUpperCase().charCodeAt(0), name];
    const char = text && !(modifiers & (MODS.ctrl | MODS.alt | MODS.meta)) ? text : undefined;
    const codeName = name.length === 1
      ? (/[a-z]/i.test(name) ? `Key${name.toUpperCase()}` : /[0-9]/.test(name) ? `Digit${name}` : "")
      : name;
    const base = { modifiers, key: name === "Space" ? " " : name, code: codeName, windowsVirtualKeyCode: code };
    await this.cdp.send("Input.dispatchKeyEvent", { type: char ? "keyDown" : "rawKeyDown", text: char, ...base });
    await this.cdp.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  }

  async screenshot(file) {
    const shot = await this.cdp.send("Page.captureScreenshot", { format: "png" });
    const { writeFileSync } = await import("node:fs");
    writeFileSync(file, Buffer.from(shot.data, "base64"));
  }
}

// Headless Chrome reports no fine pointer, so demo.js won't autostart.
// This shim answers only the two pointer queries and delegates every other
// query to the real matchMedia; it records what the browser really said.
// The report labels this as emulation, never as a native pointer check.
export const POINTER_FINE_SHIM = `(() => {
  var real = window.matchMedia.bind(window);
  window.__pointerNative = real("(pointer: fine)").matches;
  var fake = function (q) {
    return {
      matches: q.indexOf("fine") >= 0, media: q, onchange: null,
      addEventListener: function () {}, removeEventListener: function () {},
      addListener: function () {}, removeListener: function () {},
      dispatchEvent: function () { return false; },
    };
  };
  window.matchMedia = function (q) {
    if (/\\(\\s*pointer\\s*:\\s*fine\\s*\\)/.test(q)) return fake("(pointer: fine)");
    if (/\\(\\s*pointer\\s*:\\s*coarse\\s*\\)/.test(q)) return fake("(pointer: coarse)");
    return real(q);
  };
})();`;
