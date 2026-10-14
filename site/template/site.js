// getfaro.dev (§6.1): page theme, install tabs, copy buttons, analytics.
// Vanilla, no cookies: only the theme choice and the opt-out flag.
(function () {
  "use strict";
  var root = document.documentElement;
  root.classList.add("js");

  function event(name, props) {
    if (window.plausible) window.plausible(name, { props: props });
  }

  // page theme (§5.1): system (the default, nothing stored), light or
  // dark; theme.js re-applies a stored choice before first paint.
  var THEME_KEY = "faro-site-theme";
  var scheme = document.getElementById("scheme");
  if (scheme) {
    var choices = Array.prototype.slice.call(scheme.querySelectorAll("[data-scheme]"));
    var stored = function () {
      try {
        var t = localStorage.getItem(THEME_KEY);
        return t === "light" || t === "dark" ? t : "system";
      } catch (e) { return "system"; }
    };
    var paint = function (choice) {
      choices.forEach(function (b) {
        var on = b.getAttribute("data-scheme") === choice;
        b.setAttribute("aria-checked", on ? "true" : "false");
        b.tabIndex = on ? 0 : -1;
      });
    };
    var choose = function (choice) {
      try {
        if (choice === "system") localStorage.removeItem(THEME_KEY);
        else localStorage.setItem(THEME_KEY, choice);
      } catch (e) { /* private mode: this page view only */ }
      if (choice === "system") root.removeAttribute("data-theme");
      else root.setAttribute("data-theme", choice);
      paint(choice);
    };
    choices.forEach(function (b, i) {
      b.addEventListener("click", function () { choose(b.getAttribute("data-scheme")); });
      b.addEventListener("keydown", function (e) {
        var step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1
          : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
        if (!step) return;
        e.preventDefault();
        var next = choices[(i + step + choices.length) % choices.length];
        next.focus();
        choose(next.getAttribute("data-scheme"));
      });
    });
    paint(stored());
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
