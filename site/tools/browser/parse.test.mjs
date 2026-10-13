// Pure self-tests for the harness helpers (no browser): run with
//   node --test site/tools/browser/parse.test.mjs
// They pin the nginx header parsing (quoted spaces/semicolons survive,
// malformed lines fail), the network audit's scheme filter, and the CLI.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { parseAddHeaders, nginxHeaders } from "./server.mjs";
import { foreignOrigins, isInternalScheme } from "./probe.mjs";
import { parseArgs } from "./args.mjs";

const NGINX = join(fileURLToPath(new URL(".", import.meta.url)), "..", "..", "deploy", "nginx");

test("add_header: quoted values keep spaces and semicolons", () => {
  const got = parseAddHeaders(
    `add_header Strict-Transport-Security "max-age=63072000; includeSubDomains" always;\n` +
    `add_header Content-Security-Policy $csp always;\n` +
    `add_header Permissions-Policy "camera=(), microphone=()" always;\n`,
    "default-src 'self'"
  );
  assert.equal(got[0][1], "max-age=63072000; includeSubDomains");
  assert.equal(got[1][1], "default-src 'self'"); // $csp substituted
  assert.equal(got[2][1], "camera=(), microphone=()");
});

test("add_header: malformed lines and missing headers fail loudly", () => {
  assert.throws(() => parseAddHeaders(`add_header X "unterminated always;\n`, "c"), /unparsed/);
  assert.throws(() => parseAddHeaders(`add_header X-Test value always;\n`, "c"),
    /missing add_header Content-Security-Policy/);
  assert.throws(() => parseAddHeaders(`nothing here\n`, "c"), /no add_header/);
});

test("the real nginx headers parse whole: CSP, HSTS and Permissions intact", () => {
  const headers = new Map(nginxHeaders(NGINX));
  assert.ok(headers.get("Content-Security-Policy").includes("wasm-unsafe-eval"));
  assert.equal(headers.get("Strict-Transport-Security"), "max-age=63072000; includeSubDomains");
  assert.ok(headers.get("Permissions-Policy").includes("camera=()"));
  assert.ok(headers.get("Referrer-Policy").includes("strict-origin"));
  assert.equal(headers.get("X-Content-Type-Options"), "nosniff");
  assert.equal(headers.size, 5);
});

test("network audit: internal schemes pass, foreign http(s) is caught", () => {
  const ours = new Set(["http://127.0.0.1:8080"]);
  assert.deepEqual(foreignOrigins(
    ["about:blank", "data:image/png;base64,x", "chrome://version", "devtools://d/",
     "http://127.0.0.1:8080/index.html", "blob:http://127.0.0.1:8080/abc"],
    ours
  ), []);
  assert.deepEqual(foreignOrigins(["https://plausible.io/api/event", "not a url"], ours),
    ["https://plausible.io", "unparseable: not a url"]);
  assert.ok(isInternalScheme("about:blank") && !isInternalScheme("http://x/"));
});

test("cli: flags anywhere, strict values, built dir required", () => {
  assert.deepEqual(parseArgs(["site", "--out", "o", "--budget", "30"]),
    { built: "site", out: "o", budgetSec: 30 });
  assert.deepEqual(parseArgs(["--budget", "30", "site"]),
    { built: "site", out: null, budgetSec: 30 });
  assert.throws(() => parseArgs(["site", "--budget"]), /needs a value/);
  assert.throws(() => parseArgs(["site", "--budget", "abc"]), /positive number/);
  assert.throws(() => parseArgs(["site", "--budget", "-5"]), /positive number/);
  assert.throws(() => parseArgs(["site", "--nope", "x"]), /unknown flag/);
  assert.throws(() => parseArgs([]), /usage/);
});
