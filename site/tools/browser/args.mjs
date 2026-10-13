// CLI parsing for browser_test.mjs, separate so the self-test can check it
// without importing the runner (which would launch a browser).
export function parseArgs(argv) {
  const args = [...argv];
  const flag = (name, def) => {
    const i = args.indexOf(name);
    if (i < 0) return def;
    const [, value] = args.splice(i, 2);
    if (value === undefined || value.startsWith("--")) throw new Error(`${name} needs a value`);
    return value;
  };
  const out = flag("--out", null);
  const budgetRaw = flag("--budget", "100");
  const budgetSec = Number(budgetRaw);
  if (!Number.isFinite(budgetSec) || budgetSec <= 0) {
    throw new Error(`--budget must be a positive number of seconds, got ${budgetRaw}`);
  }
  const unknown = args.filter((a) => a.startsWith("--"));
  if (unknown.length) throw new Error(`unknown flag(s): ${unknown.join(" ")}`);
  const built = args[0] ?? null;
  if (!built) throw new Error("usage: node site/tools/browser_test.mjs <built site dir> [--out DIR] [--budget SECS]");
  return { built, out, budgetSec };
}
