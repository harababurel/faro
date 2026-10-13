// getfaro.dev (§6.1): theme toggle, install tabs, copy buttons, analytics.
// Vanilla, no cookies: only the theme choice and the opt-out flag.
(function () {
  "use strict";
  var root = document.documentElement;
  root.classList.add("js");

  function event(name, props) {
    if (window.plausible) window.plausible(name, { props: props });
  }

  // theme toggle (§5.1): theme.js re-applies it.
  var THEME_KEY = "faro-site-theme";
  var toggle = document.getElementById("theme-toggle");
  if (toggle) {
    var storedTheme = function () {
      try { return localStorage.getItem(THEME_KEY); } catch (e) { return null; }
    };
    var currentTheme = function () {
      var t = storedTheme();
      if (t === "light" || t === "dark") return t;
      return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
    };
    var paintToggle = function () {
      var next = currentTheme() === "light" ? "dark" : "light";
      toggle.setAttribute("aria-label", "Switch to the " + next + " theme");
    };
    toggle.addEventListener("click", function () {
      var next = currentTheme() === "light" ? "dark" : "light";
      root.setAttribute("data-theme", next);
      try { localStorage.setItem(THEME_KEY, next); } catch (e) { /* private mode */ }
      paintToggle();
    });
    paintToggle();
  }

  // install tabs (§3.4)
  var tabs = Array.prototype.slice.call(document.querySelectorAll("[role=tab]"));
  var panelOf = function (tab) {
    var id = tab.getAttribute("aria-controls");
    return id ? document.getElementById(id) : null;
  };
  var selectTab = function (index) {
    tabs.forEach(function (tab, i) {
      var on = i === index;
      tab.setAttribute("aria-selected", on ? "true" : "false");
      tab.tabIndex = on ? 0 : -1;
      var panel = panelOf(tab);
      if (panel) panel.hidden = !on;
    });
  };
  if (tabs.length > 1) {
    tabs.forEach(function (tab) {
      var panel = panelOf(tab);
      if (panel) panel.tabIndex = 0; // keyboard-scrollable
      tab.addEventListener("click", function () { selectTab(tabs.indexOf(tab)); });
      tab.addEventListener("keydown", function (e) {
        var step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        if (!step) return;
        e.preventDefault();
        var next = (tabs.indexOf(tab) + step + tabs.length) % tabs.length;
        tabs[next].focus();
        selectTab(next);
      });
    });
    selectTab(0);
  }

  // copy buttons
  document.querySelectorAll("button.copy").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var label = btn.firstChild;
      var block = btn.closest(".codeblock");
      block = block && block.querySelector("pre, code");
      var flash = function (text) {
        label.nodeValue = text;
        window.setTimeout(function () { label.nodeValue = "Copy"; }, 2000);
      };
      if (!block || !navigator.clipboard) { flash("Copy failed "); return; }
      navigator.clipboard.writeText(block.textContent.trim()).then(
        function () {
          flash("Copied ✓ ");
          var method = btn.getAttribute("data-method");
          if (method) event("Install copied", { method: method });
        },
        function () { flash("Copy failed "); }
      );
    });
  });

  // analytics opt-out (§6.9)
  var optLink = document.getElementById("opt-out");
  var optNote = document.getElementById("opt-out-note");
  var optedOut = false;
  try { optedOut = localStorage.getItem("plausible_ignore") === "true"; } catch (e) { /* no localStorage */ }
  if (optLink && optedOut) {
    optLink.href = "/?plausible_ignore=false";
    optLink.firstChild.nodeValue = "Count my visits again";
    if (optNote) optNote.hidden = false;
  }

  // outbound links (§6.9)
  document.querySelectorAll('a[href^="http"]').forEach(function (link) {
    var url = new URL(link.href);
    if (url.host === window.location.host) return;
    link.addEventListener("click", function () {
      event("Outbound Link: Click", { domain: url.host });
    });
  });
})();
