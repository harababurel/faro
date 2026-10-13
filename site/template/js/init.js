// Plausible's init, out of the page for the CSP
// (`script-src 'self'`); runs before /js/s.js (both `defer`).

// Opt-out link (§6.9): the footer's /?plausible_ignore=true link is handled
// here (set/clear the flag, drop the parameter) before the tracker loads,
// so this pageview is already ignored.
try {
  var url = new URL(window.location.href);
  var want = url.searchParams.get("plausible_ignore");
  if (want === "true") {
    localStorage.setItem("plausible_ignore", "true");
  } else if (want === "false") {
    localStorage.removeItem("plausible_ignore");
  }
  if (want !== null) {
    url.searchParams.delete("plausible_ignore");
    window.history.replaceState(null, "", url);
  }
} catch (e) { /* no localStorage: the choice just won't stick */ }

window.plausible = window.plausible || function () { (plausible.q = plausible.q || []).push(arguments); };
plausible.init = plausible.init || function (options) { plausible.o = options || {}; };
plausible.init({ endpoint: "/api/event" });
