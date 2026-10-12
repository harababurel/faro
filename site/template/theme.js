// Apply the stored light/dark choice before the first paint (WEBSITE.md §5.1).
// Loaded non-deferred in <head> of every page; only sets data-theme on <html>.
// site.js owns the toggle and writes the same key.
try {
  var theme = localStorage.getItem("faro-site-theme");
  if (theme === "light" || theme === "dark") {
    document.documentElement.setAttribute("data-theme", theme);
  }
} catch (e) { /* private mode: follow the system scheme */ }
