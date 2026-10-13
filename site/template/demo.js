// getfaro.dev demo loader (WEBSITE.md §4.2): the poster turns into the live
// faro. Same-origin only: the CSP is script-src 'self' 'wasm-unsafe-eval'.
// Phones (§4.7), Save-Data/2G and a manifest that says no keep the poster (§6.8).
(function () {
  "use strict";

  var frame = document.querySelector(".panel .frame");
  if (!frame) return; // privacy, terms

  var status = document.getElementById("demo-status");
  var startBtn = document.getElementById("demo-start");
  var exited = document.getElementById("demo-exited");
  var again = document.getElementById("demo-again");
  var cta = document.getElementById("demo-cta");

  // the poster's grid, so the first live frame is the poster's (§4.4, D123)
  var FONT = "JetBrains Mono", COLS = 120, ROWS = 34;

  var manifest = null, mod = null, demo = null, canvas = null;
  var loading = false, live = false;

  var query = function (q) { return window.matchMedia(q).matches; };

  // phones and small screens: the poster and the honest note stay (§4.7)
  var phone = query("(max-width: 640px)") ||
    (query("(pointer: coarse)") && !query("(pointer: fine)"));
  if (phone) return;

  // the room the frame has, not its width: once live it shrinks to the canvas
  var frameWidth = function () {
    var cs = window.getComputedStyle(frame);
    return frame.parentElement.clientWidth
      - (parseFloat(cs.paddingLeft) || 0) - (parseFloat(cs.paddingRight) || 0)
      - (parseFloat(cs.borderLeftWidth) || 0) - (parseFloat(cs.borderRightWidth) || 0);
  };

  // the largest size (16 down to 13) whose COLS cells fit the frame, rounded to
  // whole device pixels as faro_web::metrics::cell does, so the live cells are
  // the poster's (D122); null is the phone treatment however wide (§4.4)
  var fontSize = function () {
    var dpr = window.devicePixelRatio || 1;
    var room = frameWidth();
    for (var size = 16; size >= 13; size -= 1) {
      var cell = Math.round(Math.round(size * dpr) * 0.6);
      if (COLS * cell / dpr <= room) return size;
    }
    return null;
  };

  // no preload on a touch-only desktop, on Save-Data or 2G, or too narrow (§4.2)
  var conn = navigator.connection;
  var cheap = !!(conn && (conn.saveData || /^(slow-2g|2g)$/.test(conn.effectiveType || "")));
  var capable = query("(pointer: fine)") && !cheap && fontSize() !== null;

  // the page's scheme: the toggle's data-theme, else the system's (§5.1)
  var scheme = function () {
    var attr = document.documentElement.getAttribute("data-theme");
    if (attr === "light" || attr === "dark") return attr;
    return query("(prefers-color-scheme: light)") ? "light" : "dark";
  };

  // the hero's CTA (§3.1): it says there's a demo to see, and clicking focuses
  // the canvas or starts the load; without JS the link only scrolls
  var askCta = function () {
    if (!cta || cta.getAttribute("data-demo") === "on") return;
    cta.setAttribute("data-demo", "on");
    cta.textContent = "▶ Try it here, no install";
    cta.addEventListener("click", function (event) {
      if (!live || !canvas) { begin(); return; }
      // the fragment jump would take the focus back from the canvas
      event.preventDefault();
      frame.scrollIntoView({ block: "center" });
      canvas.focus({ preventScroll: true });
    });
  };

  // the poster plus an honest button (§4.2): the way in when the page didn't
  // preload, and the retry when a load failed
  var showStart = function () {
    // too narrow for 120 columns at 13 px: the poster stays (the grid is fixed)
    if (!startBtn || !manifest || fontSize() === null) return;
    startBtn.textContent = "▶ Start the demo (" + (manifest.bytes / 1e6).toFixed(1) + " MB)";
    startBtn.hidden = false;
    askCta();
  };

  // the manifest first, once idle, so it never competes with the page (§6.6);
  // a missing one leaves the poster, quietly
  window.addEventListener("load", function () {
    var read = function () {
      fetch("/demo/manifest.json").then(function (res) {
        if (!res.ok) throw new Error("manifest: HTTP " + res.status);
        return res.json();
      }).then(function (data) {
        manifest = data;
        if (manifest.preload && capable) begin();
        else showStart();
      }).catch(function (e) { console.warn("demo manifest:", e); });
    };
    if (window.requestIdleCallback) window.requestIdleCallback(read, { timeout: 3000 });
    else window.setTimeout(read, 1500);
  });

  // the wasm with an honest progress line: nginx sends it gzipped and the
  // browser decompresses as we read, so the reader counts the wasm as it is
  // (manifest.size) while the line shows the share of the wire (bytes, D124)
  var fetchWasm = async function () {
    var res = await fetch(manifest.wasm, { priority: "low" });
    if (!res.ok) throw new Error("wasm: HTTP " + res.status);
    var total = Math.round(manifest.bytes / 1024);
    var reader = res.body.getReader();
    var chunks = [], received = 0, chunk;
    for (;;) {
      chunk = await reader.read();
      if (chunk.done) break;
      chunks.push(chunk.value);
      received += chunk.value.length;
      if (status) {
        var done = Math.round(Math.min(received / manifest.size, 1) * manifest.bytes / 1024);
        status.textContent = "loading faro · " + done + " KB / " + total + " KB";
      }
    }
    var bytes = new Uint8Array(received), at = 0;
    chunks.forEach(function (c) { bytes.set(c, at); at += c.length; });
    return bytes;
  };

  // what the first frame needs: the wasm, the glue, and the font the painter
  // measures before it draws (D107)
  var load = async function () {
    var bytes = await fetchWasm();
    mod = await import(manifest.js);
    await mod.default({ module_or_path: bytes });
    await Promise.all([
      document.fonts.load('16px "JetBrains Mono"'),
      document.fonts.load('bold 16px "JetBrains Mono"'),
      document.fonts.load('italic 16px "JetBrains Mono"'),
    ]);
  };

  // until faro settles: the backlog is in and the poster's chat is open (D123);
  // a frame that hasn't settled still beats a poster that never moves
  var waitReady = async function () {
    var until = Date.now() + 4000;
    while (!demo.ready() && Date.now() < until) {
      await new Promise(function (go) { window.setTimeout(go, 50); });
    }
  };

  // the poster's place: a canvas at the size the frame fits, the poster's grid,
  // the page's scheme (§4.4, §4.5)
  var startLive = async function () {
    var size = fontSize();
    if (size === null) throw new Error("too narrow for " + COLS + " columns");
    canvas = document.createElement("canvas");
    canvas.className = "demo-canvas";
    canvas.tabIndex = 0;
    canvas.setAttribute("role", "application");
    canvas.setAttribute("aria-label", "faro demo: click to type");
    frame.appendChild(canvas);
    try {
      demo = await mod.start(canvas, FONT, size, COLS, ROWS);
    } catch (e) {
      canvas.remove();
      canvas = null;
      throw e;
    }
    if (scheme() === "light") demo.set_theme("faro-light");
    await waitReady();
    frame.classList.add("live"); // the CSS swaps the canvas for the posters
    if (status) status.hidden = true;
    live = true;
    loading = false;
    onCanvas();
    askCta();
    // the headless tests' hook: tools/shot.mjs waits for done, drives demo
    window.demo = demo;
    window.done = true;
  };

  var begin = function () {
    if (loading || live || !manifest) return;
    loading = true;
    if (startBtn) startBtn.hidden = true;
    if (status) {
      status.textContent = "loading faro";
      status.hidden = false;
    }
    load().then(startLive).catch(function (e) {
      console.warn("the demo didn't load:", e); // the button comes back: retry
      loading = false;
      if (status) status.hidden = true;
      showStart();
    });
  };

  if (startBtn) startBtn.addEventListener("click", begin);

  // faro quit (Ctrl+Q, Ctrl+C twice): the overlay offers a fresh one, same seed
  // (§4.5), as on faro-web's own test page
  var onCanvas = function () {
    canvas.addEventListener("faro-exit", function () {
      if (exited) exited.hidden = false;
      if (again) again.focus();
    });
  };

  if (again) {
    again.addEventListener("click", async function () {
      if (loading) return;
      loading = true;
      if (exited) exited.hidden = true;
      var old = demo;
      demo = null; // the scheme follower must not reach a freed demo
      if (old) old.free();
      if (canvas) canvas.remove();
      try {
        await startLive();
        canvas.focus();
      } catch (e) {
        // the poster comes back, and the button if the frame still fits
        console.warn("the demo didn't start again:", e);
        live = false;
        loading = false;
        frame.classList.remove("live");
        showStart();
      }
    });
  }

  // follow the page's switch while faro is on one of faro's own themes; one the
  // visitor picked in the palette stays theirs (§4.5)
  var followScheme = function () {
    var apply = function () {
      if (!demo) return;
      var name = demo.theme();
      if (name !== "faro-dark" && name !== "faro-light") return;
      demo.set_theme(scheme() === "light" ? "faro-light" : "faro-dark");
    };
    if (window.MutationObserver) {
      new window.MutationObserver(apply).observe(document.documentElement, {
        attributes: true, attributeFilter: ["data-theme"],
      });
    }
    var mq = window.matchMedia("(prefers-color-scheme: light)");
    if (mq.addEventListener) mq.addEventListener("change", apply);
    else mq.addListener(apply);
  };
  followScheme();
})();
