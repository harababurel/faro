// getfaro.dev demo loader (§4.2): the poster turns into live faro.
// Same-origin; gallery mode (§4.7), Save-Data/2G and a "no" manifest (§6.8)
// keep the stills.
(function () {
  "use strict";
  var frame = document.querySelector(".panel .frame");
  if (!frame) return;
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
  var caption = panel.querySelector(":scope > figcaption");
  var gallery = document.querySelector(".phone-gallery");
  var FONT = "JetBrains Mono", COLS = 120, ROWS = 34;
  var manifest = null, mod = null, demo = null, canvas = null;
  var loading = false, live = false;
  var dead = false;
  var effTheme = "", cfgTheme = "", reduced = false;
  var lastCfg = null, lastReduced = null, seeded = false;
  var themePinned = false, motionOverridden = false;
  var visitTheme = null, visitMotion = null; // these survive a restart (D126)
  var fsOn = false, gen = 0, fontSet = null, starting = false;
  var lockTok = 0, lockPending = false;
  var refitQueued = false;
  // §6.9: opt-out aware; props only how/theme
  var track = function (name, props) {
    try {
      if (localStorage.getItem("plausible_ignore") === "true") return;
    } catch (e) {}
    if (typeof window.plausible !== "function") return;
    try { window.plausible(name, { props: props }); } catch (e) {}
  };
  var startHow = null, startSent = false, focusSent = false, sentSent = false;
  var mqMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  var query = function (q) { return window.matchMedia(q).matches; };
  var pf = function (v) { return parseFloat(v) || 0; };
  // CSS owns the phone/desktop call (§4.7): gallery shows ⇔ no demo
  var phoneMode = function () {
    // fullscreen is never phone (§4.3)
    return !fsOn && !!gallery && window.getComputedStyle(gallery).display !== "none";
  };
  var frameWidth = function () {
    var cs = window.getComputedStyle(frame);
    return frame.parentElement.clientWidth
      - pf(cs.paddingLeft) - pf(cs.paddingRight)
      - pf(cs.borderLeftWidth) - pf(cs.borderRightWidth);
  };
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
  var capableNow = function () {
    return query("(pointer: fine)") && !cheap && fontSize() !== null;
  };
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
      if (!live || !canvas) { begin("button"); return; }
      event.preventDefault(); // the fragment jump steals the canvas's focus
      frame.scrollIntoView({ block: "center" });
      if (dead) { if (again) again.focus({ preventScroll: true }); return; }
      canvas.focus({ preventScroll: true });
    });
  };
  var showStart = function () {
    if (!startBtn || !manifest || phoneMode() || fontSize() === null) return;
    startBtn.textContent = "▶ Start the demo (" + (manifest.bytes / 1e6).toFixed(1) + " MB)";
    startBtn.hidden = false;
    askCta();
  };
  // read once, lazily (§4.7)
  var manifestRead = false, readQueued = false;
  var scheduleRead = function () {
    if (manifestRead || readQueued || phoneMode()) return;
    readQueued = true;
    var read = function () {
      readQueued = false;
      if (phoneMode()) { // narrowed since the queue: record the
        wasPhone = true; // flip; checkMode retries when wide
        return;
      }
      manifestRead = true;
      fetch("/demo/manifest.json").then(function (res) {
        if (!res.ok) throw new Error("manifest: HTTP " + res.status);
        return res.json();
      }).then(function (data) {
        manifest = data;
        if (manifest.preload && capableNow()) begin("auto");
        else if (!live) showStart();
      }).catch(function (e) {
        manifestRead = false; // one failure must not block every later grow
        console.warn("demo manifest:", e);
      });
    };
    if (window.requestIdleCallback) window.requestIdleCallback(read, { timeout: 3000 });
    else window.setTimeout(read, 1500);
  };
  window.addEventListener("load", function () {
    if (!phoneMode()) scheduleRead();
  });
  // a stalled download can't hang the panel: 30 s deadline, the signal
  // covers the body reader too; begin's catch brings the Start button back.
  var fetchWasm = async function () {
    var ctl = new AbortController();
    var timer = window.setTimeout(function () { ctl.abort(); }, 30000);
    try {
      var res = await fetch(manifest.wasm, { priority: "low", signal: ctl.signal });
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
    } finally {
      window.clearTimeout(timer);
    }
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
    mod = wasm; // only after init+fonts: a failed load retries
  };
  var waitReady = async function (d) {
    var until = Date.now() + 4000;
    while (!d.ready() && Date.now() < until) {
      await new Promise(function (go) { window.setTimeout(go, 50); });
    }
  };
  // caption controls (§4.5): never Rust in handlers (D125).
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
  var tell = function (msg) {
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
      if (cfgTheme !== lastCfg) {
        themePinned = true; visitTheme = cfgTheme;
        track("Theme changed", { theme: cfgTheme });
      }
      if (reduced !== lastReduced) { motionOverridden = true; visitMotion = reduced; }
    }
    lastCfg = cfgTheme; lastReduced = reduced;
    paintTools();
    queueRefit();
  };
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
  var noFocus = function (e) { e.preventDefault(); };
  chipBtns.forEach(function (btn) {
    btn.addEventListener("mousedown", noFocus);
    btn.addEventListener("click", function () {
      if (!demo || dead || loading) return;
      tell("");
      var name = btn.getAttribute("data-theme");
      var prev = cfgTheme;
      if (demo.pick_theme(name)) {
        themePinned = true; visitTheme = lastCfg = name;
        if (name !== prev) track("Theme changed", { theme: name });
      } else tell("that theme isn't in this demo");
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
      var want = !reduced;
      motionOverridden = true;
      visitMotion = lastReduced = want;
      demo.set_reduce_motion(want);
    });
  }
  // restart: same seed, choices survive
  var restart = async function () {
    if (loading || !mod || phoneMode()) return;
    loading = true;
    var tok = ++gen; // fence: newest attempt owns the panel
    tell("");
    setActions(false);
    releaseLock(); // lock goes; fullscreen stays (D126)
    if (exited) exited.hidden = true;
    var old = demo;
    demo = null; // no follower reaches a freed demo
    if (old) old.free();
    clearFocus(); // remove fires no blur
    canvas.remove();
    canvas = null;
    live = false; dead = false;
    frame.classList.remove("live");
    setTools(false);
    try {
      await startLive(tok);
      canvas.focus(); // only restart lands keys; autostart doesn't
    } catch (e) {
      if (tok !== gen) return; // newer attempt cleans up
      console.warn("the demo didn't start again:", e);
      loading = false;
      clearFocus(); setTools(false);
      showStart();
    }
  };
  [again, restartBtn].forEach(function (b) {
    if (b) b.addEventListener("click", restart);
  });
  // fullscreen (§4.3): Lock best-effort, never Rust (D125).
  var syncFsBtn = function () {
    if (!fsBtn) return;
    fsBtn.setAttribute("aria-pressed", String(fsOn));
    if (fsLabel) fsLabel.textContent = fsOn ? "Exit fullscreen" : "Fullscreen";
  };
  var unlock = function () {
    try {
      if (navigator.keyboard && navigator.keyboard.unlock) navigator.keyboard.unlock();
    } catch (e) {}
  };
  // stale lock grant → unlock
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
      if (tok !== lockTok) unlock();
    }, function () {
      lockPending = false;
      tell("Keyboard Lock isn't allowed here");
    });
  };
  var fsNo = "Fullscreen isn't available here";
  if (fsBtn) fsBtn.addEventListener("click", function () {
    if (!panel.requestFullscreen || !document.exitFullscreen) {
      tell(fsNo);
      return;
    }
    if (document.fullscreenElement) {
      if (fsOn) document.exitFullscreen().catch(function () {});
      return;
    }
    if (loading || dead || !live) return;
    var c = canvas, g = gen; // stale grant: not a new canvas
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
      if (ours) tryLock();
      else releaseLock();
      syncFsBtn();
      checkMode(); // FS moved the phone call
    }
    syncFocus();
    queueRefit();
  });
  // font refit (§4.4, D126)
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
      if (size === null) size = 13;
      if (size !== fontSet) { fontSet = size; demo.set_font_size(size); }
    });
  };
  window.addEventListener("resize", queueRefit);
  // narrow ⇄ wide (4.5b): a live demo that no longer fits is freed, a load
  // in flight pauses at its next fence and keeps the module (D125: async,
  // never in a faro borrow). Growing back reuses the cache; choices survive.
  // Fullscreen is never torn down, never phone.
  var teardown = function () {
    gen += 1;
    releaseLock();
    if (exited) exited.hidden = true;
    if (startBtn) startBtn.hidden = true;
    var old = demo;
    demo = null; // no follower reaches a freed demo
    if (old) old.free();
    clearFocus(); // remove fires no blur
    if (canvas) {
      canvas.remove();
      canvas = null;
    }
    live = false;
    dead = false;
    loading = false;
    seeded = false;
    frame.classList.remove("live");
    setTools(false);
    tell("");
    if (status) status.hidden = true;
    window.demo = null;
    window.done = false;
  };
  var wasPhone = phoneMode();
  var checkMode = function () {
    var now = phoneMode();
    if (now === wasPhone) return;
    wasPhone = now;
    if (now) {
      if (live && !fsOn) teardown();
    } else if (!live && !loading && !dead) {
      if (manifest) {
        if (manifest.preload && capableNow()) begin("auto");
        else showStart();
      } else {
        scheduleRead();
      }
    }
  };
  window.addEventListener("resize", checkMode);
  window.matchMedia("(pointer: coarse)").addEventListener("change", checkMode);
  // focus model (§4.3): Rust owns focus
  var keysTo = function () {
    return !!canvas && !dead && document.activeElement === canvas
      && !document.hidden && document.hasFocus();
  };
  var syncFocus = function (forceOff) {
    var on = live && !(forceOff === true) && keysTo();
    if (on && !focusSent) { focusSent = true; track("Demo focused"); }
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
  // hasFocus() lies mid-blur; blur forces off
  window.addEventListener("blur", function () {
    releaseLock();
    syncFocus(true);
  });
  var startLive = async function (tok) {
    var size = fontSize();
    if (size === null) throw new Error("too narrow for " + COLS + " columns");
    var c = document.createElement("canvas");
    c.className = "demo-canvas";
    c.tabIndex = 0;
    c.setAttribute("role", "application");
    c.setAttribute("aria-label", "faro demo: click to type");
    c.setAttribute("aria-describedby", "demo-focus");
    seeded = false; // before start()'s first draw
    c.addEventListener("faro-settings", onSettings);
    c.addEventListener("focus", function () { tryLock(); syncFocus(); });
    c.addEventListener("blur", function () {
      releaseLock();
      syncFocus();
    });
    c.addEventListener("faro-exit", function () {
      dead = true;
      releaseLock();
      clearFocus();
      if (exited) exited.hidden = false;
      setActions(false);
      if (restartBtn) restartBtn.disabled = false;
      if (again) again.focus();
    });
    c.addEventListener("faro-sent", function () { // Rust: send accepted (§6.9)
      if (!sentSent) { sentSent = true; track("Demo message sent"); }
    });
    frame.appendChild(c);
    var d = null;
    try {
      d = await mod.start(c, FONT, size, COLS, ROWS);
      // a mode flip mid-await: leave no canvas
      if (tok !== gen || phoneMode()) throw new Error("the panel moved on");
      demo = d;
      // seed outside faro callbacks
      effTheme = d.theme(); cfgTheme = d.configured_theme();
      reduced = d.reduce_motion();
      lastCfg = cfgTheme; lastReduced = reduced; seeded = true;
      paintTools();
      starting = true;
      if (!(themePinned && visitTheme && applyTheme(visitTheme))) applyScheme();
      if (motionOverridden && visitMotion !== null) {
        lastReduced = visitMotion;
        d.configure_reduce_motion(visitMotion);
      } else followMotion();
      starting = false;
      await waitReady(d);
      if (tok !== gen || phoneMode()) throw new Error("the panel moved on");
    } catch (e) {
      starting = false;
      releaseLock();
      if (d) d.free();
      if (demo === d) demo = null;
      seeded = false; live = false;
      frame.classList.remove("live");
      c.remove();
      if (canvas === c) canvas = null;
      throw e;
    }
    canvas = c;
    fontSet = size;
    frame.classList.add("live");
    if (status) status.hidden = true;
    live = true;
    dead = false;
    loading = false;
    if (!startSent && startHow) { // first handover only
      startSent = true;
      track("Demo started", { how: startHow });
    }
    setTools(true);
    setActions(true);
    askCta();
    syncFocus();
    queueRefit();
    window.demo = demo;
    window.done = true;
  };
  var begin = function (how) { // explicit "auto"/"button": never a DOM event
    if (loading || live || !manifest || phoneMode()) return;
    startHow = how;
    loading = true;
    var tok = ++gen; // fence: newest attempt owns the panel
    if (startBtn) startBtn.hidden = true;
    if (status) {
      status.textContent = "loading faro";
      status.hidden = false;
    }
    (mod ? Promise.resolve() : load()).then(function () {
      if (tok !== gen) return; // superseded: module stays cached
      if (phoneMode()) { // paused: checkMode restarts when wide
        loading = false;
        if (status) status.hidden = true;
        return;
      }
      return startLive(tok);
    }).catch(function (e) {
      if (tok !== gen) return; // newer attempt cleans up
      console.warn("the demo didn't load:", e);
      loading = false;
      clearFocus();
      setTools(false); tell("");
      if (status) status.hidden = true;
      showStart();
    });
  };
  if (startBtn) startBtn.addEventListener("click", function () { begin("button"); });
  if (window.MutationObserver) {
    new window.MutationObserver(applyScheme).observe(document.documentElement, {
      attributes: true, attributeFilter: ["data-theme"],
    });
  }
  window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", applyScheme);
  mqMotion.addEventListener("change", followMotion);
})();
