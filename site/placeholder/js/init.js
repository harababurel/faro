// Plausible's snippet initialisation (CE 3.1+ tracker), moved out of the page so the
// CSP stays `script-src 'self'`. Runs before /js/s.js (both are `defer`, in order).
window.plausible = window.plausible || function () { (plausible.q = plausible.q || []).push(arguments); };
plausible.init = plausible.init || function (options) { plausible.o = options || {}; };
plausible.init({ endpoint: "/api/event" });
