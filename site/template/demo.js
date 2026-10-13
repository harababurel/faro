// getfaro.dev demo loader (WEBSITE.md §4.2): the poster turns into live faro.
// Same-origin (CSP 'self' 'wasm-unsafe-eval'); phones, Save-Data/2G and a
// "no" manifest keep the poster (§4.7, §6.8).
(function () {
  "use strict";
  var frame = document.querySelector(".panel .frame");
  if (!frame) return; // privacy, terms
  var status = document.getElementById("demo-status");
  var startBtn = document.getElementById("demo-start");
  var exited = document.getElementById("demo-exited");
  var again = document.getElementById("demo-again");
  var cta = document.getElementById("demo-cta");
  var focusHint = document.getElementById("demo-focus");
  var tools = document.querySelector(".panel-tools");
  var ctlStatus = document.getElementById("demo-controls-status");
  var chipBtns = tools ? [].slice.call(tools.querySelectorAll("[data-theme]")) : [];
  var allBtn = tools && tools.querySelector('[data-tool="themes"]');
  var motionBtn = tools && tools.querySelector('[data-tool="motion"]');
  var restartBtn = tools && tools.querySelector('[data-tool="restart"]');
  var fsBtn = tools && tools.querySelector('[data-tool="fullscreen"]');
  var fsLabel = fsBtn && fsBtn.querySelector(".sr-only");
  var panel = frame.parentElement;
  var caption = panel.querySelector("figcaption");
  // poster's grid: first live frame matches it (D123)
  var FONT = "JetBrains Mono", COLS = 120, ROWS = 34;
  var manifest = null, mod = null, demo = null, canvas = null;
  var loading = false, live = false;
  var dead = false;
  var effTheme = "", cfgTheme = "", reduced = false;
  var lastCfg = null, lastReduced = null, seeded = false; // our own calls update last* up front
  var themePinned = false, motionOverridden = false;
  var visitTheme = null, visitMotion = null; // these survive a restart (D126)
  var fsOn = false, gen = 0, fontSet = null, starting = false;
  var lockTok = 0, lockPending = false;
  var refitQueued = false;
  var mqMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  var query = function (q) { return window.matchMedia(q).matches; };
  var pf = function (v) { return parseFloat(v) || 0; };
  var phone = query("(max-width: 640px)") ||
    (query("(pointer: coarse)") && !query("(pointer: fine)"));
  if (phone) return;
  var frameWidth = function () {
    var cs = window.getComputedStyle(frame);
    return frame.parentElement.clientWidth
      - pf(cs.paddingLeft) - pf(cs.paddingRight)
      - pf(cs.borderLeftWidth) - pf(cs.borderRightWidth);
  };
  // 16..13 fitting COLS cells, rounded like metrics::cell (D122)
  var fontSize = function () {
    var dpr = window.devicePixelRatio || 1;
    var room = frameWidth();
    for (var size = 16; size >= 13; size -= 1) {
      var cell = Math.round(Math.round(size * dpr) * 0.6);
      if (COLS * cell / dpr <= room) return size;
    }
    return null;
  };
  var conn = navigator.connection;
  var cheap = !!(conn && (conn.saveData || /^(slow-2g|2g)$/.test(conn.effectiveType || "")));
  var capable = query("(pointer: fine)") && !cheap && fontSize() !== null;
  var scheme = function () {
    var attr = document.documentElement.getAttribute("data-theme");
    if (attr === "light" || attr === "dark") return attr;
    return query("(prefers-color-scheme: light)") ? "light" : "dark";
  };
  var askCta = function () {
    if (!cta || cta.getAttribute("data-demo") === "on") return;
    cta.setAttribute("data-demo", "on");
    cta.textContent = "▶ Try it here, no install";
    cta.addEventListener("click", function (event) {
      if (!live || !canvas) { begin(); return; }
      event.preventDefault(); // the fragment jump steals the canvas's focus
      frame.scrollIntoView({ block: "center" });
      if (dead) { if (again) again.focus({ preventScroll: true }); return; } // §4.5
      canvas.focus({ preventScroll: true });
    });
  };
  var showStart = function () {
    if (!startBtn || !manifest || fontSize() === null) return;
    startBtn.textContent = "▶ Start the demo (" + (manifest.bytes / 1e6).toFixed(1) + " MB)";
    startBtn.hidden = false;
    askCta();
  };
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
  // progress: count decompressed (size), show wire (D124)
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
  var load = async function () {
    var bytes = await fetchWasm();
    var wasm = await import(manifest.js);
    await wasm.default({ module_or_path: bytes });
    await Promise.all([
      document.fonts.load('16px "JetBrains Mono"'),
      document.fonts.load('bold 16px "JetBrains Mono"'),
      document.fonts.load('italic 16px "JetBrains Mono"'),
    ]);
    mod = wasm; // set only after init+fonts: a failed load retries
  };
  var waitReady = async function () {
    var until = Date.now() + 4000;
    while (!demo.ready() && Date.now() < until) {
      await new Promise(function (go) { window.setTimeout(go, 50); });
    }
  };
  // caption controls (§4.5, §4.6, D126). faro-settings/focus/exit handlers
  // run with the shell borrowed: never call Rust (D125).
  var paintTools = function () {
    chipBtns.forEach(function (b) {
      b.setAttribute("aria-pressed", String(b.getAttribute("data-theme") === effTheme));
    });
    if (motionBtn) {
      motionBtn.setAttribute("aria-pressed", String(!reduced));
      motionBtn.textContent = reduced ? "motion: off" : "motion: on";
    }
  };
  var setTools = function (on) { if (tools) tools.hidden = !on; };
  var setActions = function (on) {
    chipBtns.concat([allBtn, motionBtn, restartBtn]).forEach(function (b) {
      if (b) b.disabled = !on;
    });
  };
  var tell = function (msg) { // tell("") hides
    if (!ctlStatus) return;
    ctlStatus.textContent = msg;
    ctlStatus.hidden = !msg;
    queueRefit();
  };
  var onSettings = function (event) {
    var d = event.detail;
    if (!d) return;
    effTheme = d.theme; cfgTheme = d.configuredTheme; reduced = d.reduceMotion;
    if (seeded) {
      if (cfgTheme !== lastCfg) { themePinned = true; visitTheme = cfgTheme; }
      if (reduced !== lastReduced) { motionOverridden = true; visitMotion = reduced; }
    }
    lastCfg = cfgTheme; lastReduced = reduced;
    paintTools();
    queueRefit();
  };
  // set_theme dispatches faro-settings sync: lastCfg first, undo on false
  var applyTheme = function (want) {
    var was = lastCfg;
    lastCfg = want;
    if (demo.set_theme(want)) return true;
    lastCfg = was;
    return false;
  };
  var applyScheme = function () {
    if (!demo || themePinned || !seeded || dead || (loading && !starting)) return;
    var want = scheme() === "light" ? "faro-light" : "faro-dark";
    if (cfgTheme === want) return;
    if (!applyTheme(want)) tell("that theme isn't in this demo");
  };
  var followMotion = function () {
    if (!demo || motionOverridden || !seeded || dead || (loading && !starting)) return;
    var want = mqMotion.matches;
    if (reduced === want) return;
    lastReduced = want;
    demo.configure_reduce_motion(want);
  };
  // chips/motion keep the keys; "all…" takes them + palette (D126)
  var noFocus = function (e) { e.preventDefault(); };
  chipBtns.forEach(function (btn) {
    btn.addEventListener("mousedown", noFocus);
    btn.addEventListener("click", function () {
      if (!demo || dead || loading) return;
      tell("");
      var name = btn.getAttribute("data-theme");
      if (demo.pick_theme(name)) { themePinned = true; visitTheme = lastCfg = name; }
      else tell("that theme isn't in this demo");
    });
  });
  if (allBtn) allBtn.addEventListener("click", function () {
    if (!demo || dead || loading) return;
    tell("");
    canvas.focus();
    demo.open_themes();
  });
  if (motionBtn) {
    motionBtn.addEventListener("mousedown", noFocus);
    motionBtn.addEventListener("click", function () {
      if (!demo || dead || loading) return;
      tell("");
      var want = !reduced; // motion on = reduce_motion off
      motionOverridden = true;
      visitMotion = lastReduced = want;
      demo.set_reduce_motion(want); // faro's own path and notice (D126)
    });
  }
  // one restart for toolbar+overlay: same seed; choices survive (D126)
  var restart = async function () {
    if (loading || !mod) return;
    loading = true;
    tell("");
    setActions(false);
    releaseLock(); // lock goes; fullscreen stays (D126)
    if (exited) exited.hidden = true;
    var old = demo;
    demo = null; // followers must not reach a freed demo
    if (old) old.free(); // Rust stops audio and tasks on drop
    clearFocus(); // removing the canvas doesn't fire blur
    canvas.remove();
    canvas = null;
    live = false; dead = false;
    frame.classList.remove("live");
    setTools(false);
    try {
      await startLive();
      canvas.focus(); // only restart lands keys; autostart doesn't
    } catch (e) {
      console.warn("the demo didn't start again:", e);
      loading = false;
      clearFocus(); setTools(false);
      showStart();
    }
  };
  [again, restartBtn].forEach(function (b) {
    if (b) b.addEventListener("click", restart);
  });
  // fullscreen (§4.3, D126): whole panel; Keyboard Lock best-effort, never
  // Escape. Listeners touch DOM/browser only, queue refit (D125).
  var syncFsBtn = function () {
    if (!fsBtn) return;
    fsBtn.setAttribute("aria-pressed", String(fsOn));
    if (fsLabel) fsLabel.textContent = fsOn ? "Exit fullscreen" : "Fullscreen";
  };
  var unlock = function () {
    try {
      if (navigator.keyboard && navigator.keyboard.unlock) navigator.keyboard.unlock(); // void per spec
    } catch (e) {}
  };
  // token bump: an in-flight lock resolves stale: unlock, never re-grant
  var releaseLock = function () {
    lockTok += 1;
    unlock();
  };
  var tryLock = function () {
    if (lockPending || !fsOn || !live || dead || loading || !keysTo()) return;
    if (!navigator.keyboard || !navigator.keyboard.lock) return;
    lockPending = true;
    var tok = lockTok;
    navigator.keyboard.lock(["KeyW", "KeyT", "KeyN", "KeyQ", "Tab"]).then(function () {
      lockPending = false;
      if (tok !== lockTok) unlock(); // superseded: hand the keys back
    }, function () {
      lockPending = false; // denial leaves ordinary fullscreen
      tell("Keyboard Lock isn't allowed here");
    });
  };
  var fsNo = "Fullscreen isn't available here";
  if (fsBtn) fsBtn.addEventListener("click", function () {
    if (!panel.requestFullscreen || !document.exitFullscreen) {
      tell(fsNo);
      return;
    }
    if (document.fullscreenElement) { // leaving ours works even dead/loading
      if (fsOn) document.exitFullscreen().catch(function () {});
      return;
    }
    if (loading || dead || !live) return;
    var c = canvas, g = gen; // a stale grant must not focus a new canvas
    panel.requestFullscreen().then(function () {
      if (live && !dead && !loading && canvas === c && gen === g) {
        canvas.focus();
        tryLock();
      }
    }).catch(function () { tell(fsNo); });
  });
  document.addEventListener("fullscreenchange", function () {
    var ours = document.fullscreenElement === panel;
    if (ours !== fsOn) {
      fsOn = ours;
      if (ours) tryLock(); // grant may land after the focus
      else releaseLock();
      syncFsBtn();
    }
    syncFocus();
    queueRefit();
  });
  // font refit (§4.4, D126): same grid, re-measured on resize/FS; 13 floor
  // (CSS scrolls), 16/24 FS cap; rAF-coalesced.
  var queueRefit = function () {
    if (refitQueued || !live) return;
    refitQueued = true;
    window.requestAnimationFrame(function () {
      refitQueued = false;
      if (!live || dead || !demo) return;
      var dpr = window.devicePixelRatio || 1;
      var pcs = window.getComputedStyle(panel);
      var cs = window.getComputedStyle(frame);
      var w = (fsOn ? window.innerWidth : panel.clientWidth)
        - pf(pcs.paddingLeft) - pf(pcs.paddingRight)
        - pf(cs.paddingLeft) - pf(cs.paddingRight)
        - pf(cs.borderLeftWidth) - pf(cs.borderRightWidth);
      var h = fsOn ? window.innerHeight
        - pf(pcs.paddingTop) - pf(pcs.paddingBottom)
        - pf(cs.paddingTop) - pf(cs.paddingBottom)
        - pf(cs.borderTopWidth) - pf(cs.borderBottomWidth)
        - (caption ? caption.offsetHeight : 0) : Infinity;
      var size = null;
      for (var s = fsOn ? 24 : 16; s >= 13; s -= 1) {
        var font = Math.round(s * dpr);
        if (COLS * Math.round(font * 0.6) / dpr <= w &&
            ROWS * Math.round(font * 1.2) / dpr <= h) { size = s; break; }
      }
      if (size === null) size = 13; // floor: the CSS scrolls (§4.4)
      if (size !== fontSet) { fontSet = size; demo.set_font_size(size); }
    });
  };
  window.addEventListener("resize", queueRefit);
  // the page's half of the focus model (§4.3, D125): Rust owns focus;
  // these listeners only touch hint and border.
  var keysTo = function () {
    return !!canvas && !dead && document.activeElement === canvas
      && !document.hidden && document.hasFocus();
  };
  var syncFocus = function (forceOff) {
    // forceOff is only ever our own true
    var on = live && !(forceOff === true) && keysTo();
    frame.classList.toggle("focused", on);
    if (!focusHint) return;
    if (!live || dead) { focusHint.hidden = true; return; }
    focusHint.textContent = on
      ? (fsOn ? "keys go to faro · click outside to leave · Esc exits fullscreen"
              : "keys go to faro · click outside or press Esc to leave")
      : (fsOn ? "live · click to type · Esc exits fullscreen" : "live · click to type");
    focusHint.hidden = false;
    queueRefit();
  };
  var clearFocus = function () {
    frame.classList.remove("focused");
    if (focusHint) focusHint.hidden = true;
  };
  document.addEventListener("visibilitychange", function () {
    if (document.hidden) releaseLock();
    syncFocus();
  });
  window.addEventListener("focus", syncFocus);
  // hasFocus() reads true mid-blur; window blur forces off
  window.addEventListener("blur", function () {
    releaseLock();
    syncFocus(true);
  });
  var startLive = async function () {
    var size = fontSize();
    if (size === null) throw new Error("too narrow for " + COLS + " columns");
    gen += 1;
    canvas = document.createElement("canvas");
    canvas.className = "demo-canvas";
    canvas.tabIndex = 0;
    canvas.setAttribute("role", "application");
    canvas.setAttribute("aria-label", "faro demo: click to type");
    canvas.setAttribute("aria-describedby", "demo-focus");
    // before start()'s first draw sends faro-settings
    seeded = false;
    canvas.addEventListener("faro-settings", onSettings);
    canvas.addEventListener("focus", function () { tryLock(); syncFocus(); });
    canvas.addEventListener("blur", function () {
      releaseLock();
      syncFocus();
    });
    canvas.addEventListener("faro-exit", function () {
      dead = true;
      releaseLock();
      clearFocus();
      if (exited) exited.hidden = false;
      setActions(false); // chips/all/motion off...
      if (restartBtn) restartBtn.disabled = false; // ...restart stays (D126)
      if (again) again.focus();
    });
    frame.appendChild(canvas);
    try {
      demo = await mod.start(canvas, FONT, size, COLS, ROWS);
      // seed the cache outside faro callbacks
      effTheme = demo.theme(); cfgTheme = demo.configured_theme();
      reduced = demo.reduce_motion();
      lastCfg = cfgTheme; lastReduced = reduced; seeded = true;
      paintTools();
      starting = true;
      if (!(themePinned && visitTheme && applyTheme(visitTheme))) applyScheme();
      if (motionOverridden && visitMotion !== null) {
        lastReduced = visitMotion;
        demo.configure_reduce_motion(visitMotion);
      } else followMotion(); // quiet, before the handover (§4.6)
      starting = false;
      await waitReady();
    } catch (e) {
      starting = false;
      releaseLock();
      if (demo) { demo.free(); demo = null; }
      seeded = false; live = false;
      frame.classList.remove("live");
      canvas.remove();
      canvas = null;
      throw e;
    }
    fontSet = size;
    frame.classList.add("live");
    if (status) status.hidden = true;
    live = true;
    dead = false;
    loading = false;
    setTools(true);
    setActions(true);
    askCta();
    syncFocus();
    queueRefit();
    // headless hook: shot.mjs waits for done
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
    (mod ? Promise.resolve() : load()).then(startLive).catch(function (e) { // no re-fetch once loaded
      console.warn("the demo didn't load:", e);
      loading = false;
      clearFocus();
      setTools(false); tell("");
      if (status) status.hidden = true;
      showStart();
    });
  };
  if (startBtn) startBtn.addEventListener("click", begin);
  if (window.MutationObserver) {
    new window.MutationObserver(applyScheme).observe(document.documentElement, {
      attributes: true, attributeFilter: ["data-theme"],
    });
  }
  window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", applyScheme);
  mqMotion.addEventListener("change", followMotion);
})();
