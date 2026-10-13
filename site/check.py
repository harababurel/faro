#!/usr/bin/env python3
"""Checks for the built getfaro.dev site (WEBSITE.md §6.8, the CI site job).

Usage: site/check.py <built site dir>      — the output dir of site/build.sh.

Lists every failure and exits non-zero. Warnings (the owner placeholders
still to fill in, §5.5) are reported but don't fail the run. Stdlib only.
"""

import gzip
import html.parser
import os
import re
import subprocess
import sys

ROOT = None  # the built dir, absolute (set in main)

# nginx proxies these to Plausible: same origin, but not files in the build
# (site/deploy/nginx/getfaro.dev.conf).
PROXIED = {"js/s.js", "api/event"}
ORIGIN = "getfaro.dev"
TEXT_EXTS = {".html", ".css", ".js", ".svg", ".txt", ".json"}
OWNER_MARKS = ["{operator}", "{contact}", "{country}", "{law}", "TODO(owner)"]

# The §6.6 budgets, in bytes.
BUDGET_TOTAL = 200 * 1024
BUDGET_PAGE = 40 * 1024  # HTML + CSS + JS, compressed
BUDGET_FONT = 60 * 1024
BUDGET_POSTER = 80 * 1024

failures = []
warnings = []
external = set()


def fail(msg):
    failures.append(msg)


def warn(msg):
    warnings.append(msg)


def read(path):
    with open(path, encoding="utf-8", errors="replace") as handle:
        return handle.read()


def wire_size(path):
    """What nginx would send: the build's .gz twin, else gzip it here."""
    gz = path + ".gz"
    if os.path.isfile(gz):
        return os.path.getsize(gz)
    with open(path, "rb") as handle:
        return len(gzip.compress(handle.read(), 9))


ABS = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*://")


def resolve(path):
    """nginx's try_files ($uri, $uri.html, $uri/) plus the index directive."""
    for cand in (path, path + ".html", os.path.join(path, "index.html")):
        cand = os.path.normpath(cand)
        if not cand.startswith(ROOT + os.sep):
            continue  # a reference that would leave the site dir
        if os.path.isfile(cand):
            return cand
    return None


def target(url, base):
    """Classify one reference: ("skip", None), ("proxied", None),
    ("external", url) or ("file", resolved path or None)."""
    url = url.split("#", 1)[0].split("?", 1)[0]
    if not url or url.startswith(("data:", "mailto:", "tel:")):
        return ("skip", None)
    match = ABS.match(url)
    if match or url.startswith("//"):
        rest = url[match.end():] if match else url[2:]
        host = rest.split("/", 1)[0].split(":", 1)[0]
        if match and host == ORIGIN:
            return ("file", resolve(os.path.join(ROOT, rest[len(host):].lstrip("/") or "index.html")))
        return ("external", url)
    if url.lstrip("/") in PROXIED:
        return ("proxied", None)
    start = ROOT if url.startswith("/") else base
    return ("file", resolve(os.path.join(start, url.lstrip("/"))))


def check_ref(page, kind, url, offsite_ok, base):
    """One href/src/url()/fetch reference: off-site rules and resolution."""
    if url.lower().startswith("javascript:"):
        return  # already flagged as inline code
    what, found = target(url, base)
    if what == "external":
        if offsite_ok:
            external.add(url)
        else:
            fail(f"{page}: {kind} makes a third-party request: {url}")
    elif what == "file" and found is None:
        fail(f"{page}: {kind}=\"{url}\" doesn't resolve to a file in the site dir")


class Page(html.parser.HTMLParser):
    """Collects references; flags CSP-breaking inline code while parsing."""

    def __init__(self, name):
        super().__init__(convert_charrefs=True)
        self.name = name
        self.refs = []  # (kind, url)
        self.in_script = False

    def handle_starttag(self, tag, attrs):
        props = {name.lower(): (value or "") for name, value in attrs}
        for name, value in attrs:
            name = name.lower()
            if name.startswith("on"):
                fail(f"{self.name}: inline event handler {name}= (the CSP is script-src 'self')")
            if name == "style":
                fail(f"{self.name}: inline style attribute (the CSP is style-src 'self')")
            if name in ("href", "src") and value:
                if value.lower().startswith("javascript:"):
                    fail(f"{self.name}: javascript: URL in {name}= (the CSP is script-src 'self')")
                self.refs.append((f"<{tag} {name}>", value))
            if name in ("srcset", "imagesrcset") and value:
                for cand in value.split(","):
                    cand = cand.strip().split(" ")[0]
                    if cand:
                        self.refs.append((f"<{tag} {name}>", cand))
            if tag == "meta" and name == "http-equiv" and "cookie" in value.lower():
                fail(f"{self.name}: <meta http-equiv=\"{value}\"> sets a cookie")
        if tag == "meta" and props.get("property", props.get("name", "")) in ("og:image", "twitter:image"):
            if props.get("content"):
                self.refs.append(("<meta og:image content>", props["content"]))
        if tag == "style":
            fail(f"{self.name}: inline <style> (the CSP is style-src 'self')")
        if tag == "script" and "src" not in props:
            self.in_script = True

    def handle_data(self, data):
        if self.in_script and data.strip():
            fail(f"{self.name}: inline <script> body (the CSP is script-src 'self')")
            self.in_script = False

    def handle_endtag(self, tag):
        if tag == "script":
            self.in_script = False


CSS_URL = re.compile(r"""url\(\s*['"]?([^'")]+)""")
CSS_IMPORT = re.compile(r"""@import\s+(?:url\(\s*)?['"]([^'"]+)""")
JS_URL = re.compile(
    r"""(?:fetch|import)\s*\(\s*['"]([^'"]+)['"]|import\s+['"]([^'"]+)['"]|import\s+[\w{},*\s]*?\bfrom\s*['"]([^'"]+)['"]"""
)
# Other ways JS can reach out: only absolute URLs are third-party risks
# (relative ones are same-origin; .open also takes a method as its 1st arg).
JS_NET = re.compile(
    r"""sendBeacon\s*\(\s*['"]([^'"]*)['"]|new\s+(?:EventSource|WebSocket)\s*\(\s*['"]([^'"]*)['"]"""
    r"""|\.open\s*\(\s*['"]([^'"]*)['"](?:\s*,\s*['"]([^'"]*)['"])?|\bsrc\s*=\s*['"]([^'"]*)['"]"""
)


def check_css(rel, text):
    base = os.path.dirname(os.path.join(ROOT, rel))
    for kind, rx in (("css url()", CSS_URL), ("css @import", CSS_IMPORT)):
        for match in rx.finditer(text):
            check_ref(rel, kind, match.group(1).strip(), False, base)


def check_js(rel, text):
    if "document.cookie" in text:
        fail(f"{rel}: touches document.cookie")
    base = os.path.dirname(os.path.join(ROOT, rel))
    for match in JS_URL.finditer(text):
        check_ref(rel, "js fetch/import", next(g for g in match.groups() if g), False, base)
    for match in JS_NET.finditer(text):
        for url in (g for g in match.groups() if g and (ABS.match(g) or g.startswith("//"))):
            if target(url, base)[0] == "external":
                fail(f"{rel}: js network call makes a third-party request: {url}")


def check_placeholders(rel, text):
    if "{{" in text:
        fail(f"{rel}: unfilled placeholder \"{{\" left in the build")
    for mark in OWNER_MARKS:
        count = text.count(mark)
        if count:
            warn(f"{rel}: {count} x {mark} (owner to fill in, §5.5)")


def text_files():
    for dirpath, _, names in os.walk(ROOT):
        for name in sorted(names):
            if name.endswith((".gz", ".br")):
                continue  # the build's precompressed twins
            if os.path.splitext(name)[1] in TEXT_EXTS:
                yield os.path.relpath(os.path.join(dirpath, name), ROOT).replace(os.sep, "/")


LOADED = ("<link href>", "<script src>", "<img src>", "<img srcset>", "<link imagesrcset>", "css url()")


def check_budget(pages):
    """First load of index.html: it plus every same-origin CSS/JS/font/image
    it pulls in, directly or through CSS url(), at wire sizes (§6.6)."""
    index = os.path.join(ROOT, "index.html")
    if not os.path.isfile(index):
        fail("index.html is missing")
        return
    items = [("index.html", wire_size(index))]
    seen = {index}

    def follow(refs, base):
        for kind, url in refs:
            if kind not in LOADED:
                continue  # <a href> and og:image aren't fetched on first load
            what, found = target(url, base)
            if what != "file" or found is None or found in seen:
                continue
            seen.add(found)
            items.append((os.path.relpath(found, ROOT).replace(os.sep, "/"), wire_size(found)))
            if found.endswith(".css"):
                css = [("css url()", m.group(1).strip()) for m in CSS_URL.finditer(read(found))]
                follow(css, os.path.dirname(found))

    follow(pages.get("index.html", []), ROOT)
    print("first load of index.html (wire sizes, .gz twins where present):")
    for name, size in items:
        print(f"  {name:<48} {size:>8}")
    total = sum(size for _, size in items)
    page = sum(size for name, size in items if os.path.splitext(name)[1] in (".html", ".css", ".js"))
    print(f"  {'total (budget ' + str(BUDGET_TOTAL) + ')':<48} {total:>8}")
    print(f"  {'html + css + js (budget ' + str(BUDGET_PAGE) + ')':<48} {page:>8}")
    if total > BUDGET_TOTAL:
        fail(f"first load is {total} bytes, over the {BUDGET_TOTAL} budget (§6.6)")
    if page > BUDGET_PAGE:
        fail(f"html+css+js is {page} bytes, over the {BUDGET_PAGE} budget (§6.6)")
    for name, size in items:
        if os.path.splitext(name)[1] in (".woff", ".woff2", ".ttf", ".otf") and size > BUDGET_FONT:
            fail(f"font {name} is {size} bytes, over the {BUDGET_FONT} budget (§6.6)")
    poster = re.search(r'<div class="poster-dark">(.*?)</div>', read(index), re.S)
    if poster is None:
        warn("index.html: no .poster-dark wrapper: the poster budget wasn't checked")
    else:
        size = len(gzip.compress(poster.group(1).encode("utf-8"), 9))
        print(f"  {'poster (inline, one scheme, budget ' + str(BUDGET_POSTER) + ')':<48} {size:>8}")
        if size > BUDGET_POSTER:
            fail(f"the poster SVG is {size} bytes gzipped, over the {BUDGET_POSTER} budget (§6.6)")
    print("  note: /js/s.js (Plausible, proxied by nginx) is not in the build and not counted")


NO_JS_TEXT = ("Signal, in your terminal.", "cargo build --release", "FAQ")


def dump_dom(chrome, path, extra_flags):
    cmd = [
        chrome, "--headless=new", "--disable-gpu", "--no-sandbox", *extra_flags,
        "--dump-dom", "file://" + path,
    ]
    done = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    return done.stdout


def check_no_js():
    """The page must still say its essentials with scripting off (§6.8)."""
    chrome = os.environ.get("CHROME")
    if not chrome:
        print("note: CHROME is not set: skipping the no-JS render check")
        return
    index = os.path.join(ROOT, "index.html")
    try:
        dom = dump_dom(chrome, index, ["--blink-settings=scriptEnabled=false"])
        if not dom.strip():
            # Chrome's --dump-dom serialises the DOM through scripting, so
            # scriptEnabled=false makes it print nothing (Chrome 14x). Block
            # the page's scripts with a CSP instead: they still never run.
            print("note: scriptEnabled=false left --dump-dom empty: blocking scripts via CSP")
            body = read(index)
            meta = "<meta http-equiv=Content-Security-Policy content=\"script-src 'none'\">"
            body = re.sub(r"<head[^>]*>", lambda m: m.group(0) + meta, body, count=1, flags=re.I)
            blocked = os.path.join(ROOT, "nojs-check.html")  # .html: Chrome must serve it as a page
            with open(blocked, "w", encoding="utf-8") as handle:
                handle.write(body)
            try:
                dom = dump_dom(chrome, blocked, [])
                if 'class="js"' in dom:
                    fail("no-JS check: the CSP fallback didn't stop site.js (it set class=\"js\")")
            finally:
                os.remove(blocked)
    except (OSError, subprocess.TimeoutExpired) as error:
        fail(f"no-JS check: {chrome} didn't run: {error}")
        return
    if not dom.strip():
        fail(f"no-JS check: {chrome} dumped nothing (exit unclear)")
        return
    for want in NO_JS_TEXT:
        if want not in dom:
            fail(f"no-JS render: the DOM is missing {want!r}")


def main():
    global ROOT
    if len(sys.argv) != 2 or not os.path.isdir(sys.argv[1]):
        print("usage: site/check.py <built site dir>", file=sys.stderr)
        return 2
    ROOT = os.path.abspath(sys.argv[1])
    pages = {}
    for rel in text_files():
        text = read(os.path.join(ROOT, rel))
        check_placeholders(rel, text)
        ext = os.path.splitext(rel)[1]
        if ext == ".html":
            page = Page(rel)
            page.feed(text)
            pages[rel] = page.refs
            base = os.path.dirname(os.path.join(ROOT, rel))
            for kind, url in page.refs:
                check_ref(rel, kind, url, kind == "<a href>", base)
        elif ext == ".css":
            check_css(rel, text)
        elif ext == ".js":
            check_js(rel, text)
    check_budget(pages)
    check_no_js()
    if external:
        print("external <a> links (listed, not fetched):")
        for url in sorted(external):
            print(f"  {url}")
    for message in warnings:
        print(f"warning: {message}")
    for message in failures:
        print(f"FAIL: {message}")
    print(f"{len(failures)} failure(s), {len(warnings)} warning(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
