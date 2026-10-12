// Plausible's snippet initialisation (CE 3.1+ tracker), moved out of the page so the
// CSP stays `script-src 'self'`. Runs before /js/s.js (both are `defer`, in order).

// Opt-out link (WEBSITE.md §6.9): the tracker only honours
// localStorage.plausible_ignore === "true", so the footer's
// /?plausible_ignore=true link is handled here: set the flag (or clear it for
// ?plausible_ignore=false, the "count my visits again" link) and drop the
// parameter from the URL. Runs before the tracker loads, so the current
// pageview is already ignored.
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
