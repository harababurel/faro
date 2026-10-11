#!/usr/bin/env python3
"""M13 Phase 0 spike (throwaway): bundle each prototype page into one
self-contained HTML file (fonts as data URLs, frames inlined, JS and wasm loaded
from Blob URLs), so it opens from file:// with no server.

python3 bundle.py   ->  dist/canvas.html, dist/ratzilla.html
Open with ?frame=<name> or ?flood.
"""
import base64
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).parent
DIST = ROOT / "dist"


def frames():
    names = json.loads((ROOT / "frames/index.json").read_text())
    return {n: json.loads((ROOT / f"frames/{n}.json").read_text()) for n in names}


def fonts_inline(html: str) -> str:
    def data(m):
        path = (ROOT / "canvas" / m.group(1)).resolve()
        if not path.exists():
            return m.group(0)
        return "url(data:font/woff2;base64," + base64.b64encode(path.read_bytes()).decode() + ")"
    return re.sub(r"url\((\.\./fonts/[^)]+)\)", data, html)


def blob(var: str, source: str) -> str:
    return (f"<script>window.{var} = URL.createObjectURL(new Blob([{json.dumps(source)}], "
            "{type: 'text/javascript'}));</script>\n")


def bundle(page: pathlib.Path, extra: str, out: str):
    html = fonts_inline(page.read_text())
    head = "<script>window.FRAMES = " + json.dumps(frames(), ensure_ascii=False) + ";</script>\n" + extra
    html = html.replace("<script type=\"module\">", head + "<script type=\"module\">", 1)
    DIST.mkdir(exist_ok=True)
    (DIST / out).write_text(html)
    print(out, f"{len(html) / 1e6:.1f} MB")


bundle(ROOT / "canvas/index.html", blob("CANVAS_JS", (ROOT / "canvas/canvas.js").read_text()), "canvas.html")
wasm = (ROOT / "ratzilla/pkg/ratzilla_spike_bg.wasm").read_bytes()
glue = (ROOT / "ratzilla/pkg/ratzilla_spike.js").read_text()
bundle(
    ROOT / "ratzilla/index.html",
    blob("GLUE_JS", glue)
    + "<script>window.WASM_BYTES = Uint8Array.from(atob("
    + json.dumps(base64.b64encode(wasm).decode())
    + "), (c) => c.charCodeAt(0));</script>\n",
    "ratzilla.html",
)
