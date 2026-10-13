// Apply the stored light/dark choice before first paint (§5.1).
try {
  var theme = localStorage.getItem("faro-site-theme");
  if (theme === "light" || theme === "dark") {
    document.documentElement.setAttribute("data-theme", theme);
  }
} catch (e) { /* private mode: follow the system scheme */ }
