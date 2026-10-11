#!/usr/bin/env python3
"""Bundle the web demo's test page into one self-contained HTML file that opens
from file:// (no server): fonts as data URLs, the wasm-bindgen JS as a Blob
URL, the wasm module inlined. For headless screenshots (tools/shot.mjs) and
for passing the demo around.

    crates/faro-web/build.sh && python3 crates/faro-web/tools/bundle.py
    -> crates/faro-web/www/dist/demo.html
"""
import base64
import json
import pathlib
import re

WWW = pathlib.Path(__file__).resolve().parent.parent / "www"


def main() -> None:
    html = (WWW / "index.html").read_text()

    def font(m: re.Match) -> str:
        data = base64.b64encode((WWW / m.group(1)).read_bytes()).decode()
        return f"url(data:font/woff2;base64,{data})"

    html = re.sub(r"url\((fonts/[^)]+)\)", font, html)
    js = (WWW / "pkg/faro_web.js").read_text()
    wasm = base64.b64encode((WWW / "pkg/faro_web_bg.wasm").read_bytes()).decode()
    head = (
        "<script>\n"
        f"window.FARO_JS = URL.createObjectURL(new Blob([{json.dumps(js)}], {{type: 'text/javascript'}}));\n"
        f"window.FARO_WASM = Uint8Array.from(atob({json.dumps(wasm)}), (c) => c.charCodeAt(0));\n"
        "</script>\n"
    )
    html = html.replace('<script type="module">', head + '<script type="module">', 1)
    out = WWW / "dist/demo.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text(html)
    print(f"{out}: {out.stat().st_size} bytes")


if __name__ == "__main__":
    main()
