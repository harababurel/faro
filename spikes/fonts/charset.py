#!/usr/bin/env python3
"""M13 Phase 0 spike (throwaway): which characters must the site's webfont draw?

Non-ASCII characters in string/char literals of faro's UI and demo sources, plus
every symbol in the exported frames, split into:
  shapes  - drawn by the canvas renderer itself (blocks, octants, box drawing, Powerline)
  emoji   - drawn by the colour emoji font (wide, regional indicators, joiners, VS16)
  script  - other scripts (CJK, Arabic, ...): system fallback, at faro's width
  font    - everything else: the webfont must have it
Writes charset.json.
"""
import json
import pathlib
import re
import unicodedata

REPO = pathlib.Path(__file__).resolve().parents[2]
SOURCES = [
    *REPO.glob("crates/faro/src/**/*.rs"),
    *REPO.glob("crates/faro-core/src/mock/*.rs"),
    *REPO.glob("crates/faro-core/src/config*.rs"),
]
SKIP = re.compile(r"(tests?(\.rs|/)|fixtures\.rs|snapshots/|shortcodes/table\.rs|web_spike\.rs)")
LITERAL = re.compile(r'"(?:[^"\\]|\\.)*"|\'(?:[^\'\\]|\\.)\'')


def literal_chars(text):
    for m in LITERAL.finditer(text):
        yield from m.group(0)


def escapes(text):
    for m in re.finditer(r"\\u\{([0-9A-Fa-f]{2,6})\}", text):
        yield chr(int(m.group(1), 16))


chars = set()
for path in SOURCES:
    rel = str(path.relative_to(REPO))
    if SKIP.search(rel):
        continue
    text = path.read_text()
    chars.update(literal_chars(text))
    chars.update(escapes(text))
for frame in (REPO / "spikes/renderers/frames").glob("*.json"):
    if frame.name == "index.json":
        continue
    for cell in json.loads(frame.read_text())["cells"]:
        chars.update(cell[0])
chars = {c for c in chars if ord(c) > 0x7E}


def kind(c):
    cp = ord(c)
    if 0x2500 <= cp <= 0x259F or 0x1CD00 <= cp <= 0x1CDE5 or 0x1CEA0 <= cp <= 0x1CEAF \
            or 0x1FB00 <= cp <= 0x1FBFF or 0xE0B0 <= cp <= 0xE0BF:
        return "shapes"
    if cp in (0x200D, 0xFE0F, 0xFE0E, 0x20E3) or 0x1F1E6 <= cp <= 0x1F1FF \
            or 0x1F3FB <= cp <= 0x1F3FF or 0xE0020 <= cp <= 0xE007F or cp >= 0x1F000:
        return "emoji"
    if unicodedata.east_asian_width(c) == "W":
        name = unicodedata.name(c, "")
        return "script" if ("CJK" in name or "HIRAGANA" in name or "KATAKANA" in name
                            or "HANGUL" in name) else "emoji"
    if unicodedata.category(c).startswith("M") and cp >= 0x300 and cp < 0x370:
        return "font"
    script = unicodedata.name(c, "").split(" ")[0]
    if script in ("ARABIC", "DEVANAGARI", "HEBREW", "THAI", "BENGALI", "TAMIL"):
        return "script"
    if unicodedata.category(c) in ("Cc", "Cf", "Co", "Cn"):
        return "control"
    return "font"


groups = {}
for c in sorted(chars):
    groups.setdefault(kind(c), []).append(c)
for k, v in groups.items():
    print(f"{k:8} {len(v):4}  {''.join(v)[:160]}")
font = groups.get("font", [])
(pathlib.Path(__file__).parent / "charset.json").write_text(
    json.dumps({"font": "".join(font), "all": groups}, ensure_ascii=False, indent=1))
