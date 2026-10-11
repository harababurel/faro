#!/usr/bin/env python3
"""Preview candidate shapes for multi-line rounded bubbles (no dependencies).

Run it in the terminal you want to judge (kitty draws these glyphs itself),
then zoom in to look at the corners:

    python3 tools/bubble_shapes.py                 # all options, faro-dark
    python3 tools/bubble_shapes.py --theme dusk    # faro-dusk colours
    python3 tools/bubble_shapes.py --only B        # one option
    python3 tools/bubble_shapes.py --width 72      # wider pane

One-line bubbles are Powerline capsules in every option; only taller bubbles
change:

  A  current (D58): two-thirds edge rows with one diagonal per corner
     (Unicode 13 wedges and sextants).
  B  quarter circles (Unicode 16, U+1FBEC-1FBEF): radius half a cell wide,
     quarter-row edges, half-cell sides; same width as today.
  B+ as B, with a padding cell inside each side (text one cell further in,
     bubble two cells wider).
  C  two diagonals per corner (steep, then shallow) on full-row edges
     (Unicode 13 wedges only).
  B-compact, B+-compact
     compact density (no blank lines): consecutive bubbles from one sender
     touch and share one edge on their aligned side, rounded only at the
     run's top and bottom; on the other side a wider bubble's rounded corner
     is drawn in its neighbour's row (needs Unicode 16 octants too).

    python3 tools/bubble_shapes.py --only B-compact --only B+-compact
"""

import argparse
import os
import sys

THEMES = {
    # (pane background, bubble_other, bubble_own, text, muted)
    "dark": ((0x10, 0x12, 0x16), (0x1F, 0x23, 0x2B), (0x33, 0x29, 0x1A),
             (0xE6, 0xE6, 0xE6), (0x80, 0x84, 0x8C)),
    "dusk": ((0x16, 0x1B, 0x26), (0x23, 0x2A, 0x39), (0x1C, 0x3A, 0x3C),
             (0xDF, 0xE4, 0xEE), (0x85, 0x8D, 0x9E)),
}

CAP_L, CAP_R = "\uE0B6", "\uE0B4"

CONVERSATION = [
    (False, ["on my way, 5 min"]),
    (False, ["did you see the forecast for",
             "saturday? rain until noon, then",
             "clear. still up for the crag?"]),
    (True, ["yes! bringing the new rope"]),
    (True, ["let's meet at 1 then, the rock",
            "should be dry by the time we",
            "get there"]),
    (False, ["perfect", "see you there"]),
    (True, ["ok"]),
]


def capsule(text, fill, pane, fg):
    return [[(CAP_L, fill, pane), (text, fg, fill), (CAP_R, fill, pane)]]


def body_rows(lines, n, fill, fg, left, right):
    """Text rows: `left`/`right` are lists of (glyph, fg, bg) cells."""
    return [left + [(line.ljust(n), fg, fill)] + right for line in lines]


def shape_a(lines, n, fill, pane, fg):
    top = [("\U0001FB4A", fill, pane), ("\U0001FB39" * n, fill, pane),
           ("\U0001FB3F", fill, pane)]
    bottom = [("\U0001FB65", fill, pane), ("\U0001FB0E" * n, fill, pane),
              ("\U0001FB5A", fill, pane)]
    pad = [(" ", fg, fill)]
    return [top] + body_rows(lines, n, fill, fg, pad, pad) + [bottom]


def shape_b(lines, n, fill, pane, fg, roomy=False):
    inner = n + 2 if roomy else n
    top = [("\U0001FBEE", fill, pane), ("\u2582" * inner, fill, pane),
           ("\U0001FBED", fill, pane)]
    bottom = [("\U0001FBEC", fill, pane), ("\U0001FB82" * inner, fill, pane),
              ("\U0001FBEF", fill, pane)]
    pad = [(" ", fg, fill)] if roomy else []
    left = [("\u2590", fill, pane)] + pad
    right = pad + [("\u258C", fill, pane)]
    return [top] + body_rows(lines, n, fill, fg, left, right) + [bottom]


def shape_c(lines, n, fill, pane, fg):
    middle = max(n - 2, 0)
    top = [("\U0001FB4A\U0001FB42", fill, pane), (" " * middle, fg, fill),
           ("\U0001FB4D\U0001FB3F", fill, pane)]
    bottom = [("\U0001FB65\U0001FB53", fill, pane), (" " * middle, fg, fill),
              ("\U0001FB5E\U0001FB5A", fill, pane)]
    pad = [(" ", fg, fill)]
    return [top] + body_rows(lines, n, fill, fg, pad, pad) + [bottom]


OPTIONS = {
    "A": ("current: one diagonal per corner, 2/3-row edges",
          lambda l, n, f, p, t: shape_a(l, n, f, p, t)),
    "B": ("quarter circles (Unicode 16), same width as today",
          lambda l, n, f, p, t: shape_b(l, n, f, p, t)),
    "B+": ("quarter circles, plus a padding cell inside each side",
           lambda l, n, f, p, t: shape_b(l, n, f, p, t, roomy=True)),
    "C": ("two diagonals per corner, full-row edges",
          lambda l, n, f, p, t: shape_c(l, n, f, p, t)),
}


def sgr(fg, bg):
    return f"\x1b[38;2;{fg[0]};{fg[1]};{fg[2]};48;2;{bg[0]};{bg[1]};{bg[2]}m"


def cells(row):
    return sum(len(text) for text, _, _ in row)


def emit(row, width, pane, fg, align_right):
    """One pane line: the bubble row on the pane background, 1-cell margin."""
    used = cells(row)
    lead = width - used - 1 if align_right else 1
    lead = max(lead, 0)
    out = [sgr(fg, pane), " " * lead]
    out += [sgr(f, b) + text for text, f, b in row]
    out += [sgr(fg, pane), " " * max(width - lead - used, 0), "\x1b[0m"]
    return "".join(out)


def blank(width, pane, fg):
    return emit([], width, pane, fg, False)


def render(key, theme, width):
    if key in RUN_OPTIONS:
        return render_runs(key, theme, width)
    pane, other, own, fg, muted = THEMES[theme]
    title, build = OPTIONS[key]
    lines = [emit([(f"{key}  {title}", muted, pane)], width, pane, fg, False),
             blank(width, pane, fg)]
    for mine, text in CONVERSATION:
        fill = own if mine else other
        n = max(len(t) for t in text)
        if len(text) == 1:
            rows = capsule(text[0], fill, pane, fg)
        else:
            rows = build(text, n, fill, pane, fg)
        lines += [emit(r, width, pane, fg, mine) for r in rows]
        lines.append(blank(width, pane, fg))
    return lines


# Compact density: consecutive bubbles from one sender, no blank lines.
RUNS = [
    (False, [["on my way, 5 min"],
             ["did you see the forecast for",
              "saturday? rain until noon, then",
              "clear. still up for the crag?"],
             ["perfect"],
             ["see you there"]]),
    (True, [["yes!"],
            ["bringing the new rope and the", "quickdraws"],
            ["see you at 1"]]),
    (False, [["running late", "sorry, traffic"],
             ["10 min"],
             ["parking now", "where are you?"]]),
    (True, [["by the big boulder"]]),
    (False, [["found you"], ["coming"]]),
    (True, [["the left one, with the", "orange bag next to it"]]),
    (False, [["sounds good"], ["sounds great"], ["sounds good"]]),
    (True, [["see you"], ["see you!"], ["see you"]]),
]

def octant_glyphs():
    """Every 2x4 sub-cell pattern -> its glyph. Bit `y * 2 + x` is the octant
    at column x (0 left, 1 right) and row y (0 top .. 3 bottom). Unicode 16's
    octants (U+1CD00..) fill in, in ascending order, the patterns older block
    characters don't already cover."""
    older = {
        "\u2580": "1234", "\u2584": "5678", "\u258C": "1357", "\u2590": "2468",
        "\u2598": "13", "\u259D": "24", "\u2596": "57", "\u2597": "68",
        "\u259A": "1368", "\u259E": "2457", "\u2599": "135678",
        "\u259B": "123457", "\u259C": "123468", "\u259F": "245678",
        "\U0001FB82": "12", "\u2582": "78", "\u2586": "345678",
        "\U0001FB85": "123456", "\U0001CEA8": "1", "\U0001CEAB": "2",
        "\U0001CEA3": "7", "\U0001CEA0": "8", "\U0001FBE6": "35",
        "\U0001FBE7": "46",
    }
    table = {0: " ", 255: "\u2588"}
    for glyph, octants in older.items():
        table[sum(1 << (int(k) - 1) for k in octants)] = glyph
    code = 0x1CD00
    for mask in range(256):
        if mask not in table:
            table[mask] = chr(code)
            code += 1
    return table


OCTANT = octant_glyphs()


def bit(sx, sy):
    return 1 << ((sy % 4) * 2 + sx % 2)


# A rectangle's rounded corner, when it's alone in its cell: a quarter circle
# of radius half a cell wide, centred on the cell's corner (Unicode 16).
DISK = {
    ("TL", bit(1, 3)): "\U0001FBEE",
    ("TR", bit(0, 3)): "\U0001FBED",
    ("BL", bit(1, 0)): "\U0001FBEC",
    ("BR", bit(0, 0)): "\U0001FBEF",
}


def run_rows(run, fill, pane, fg, roomy, right):
    """A run from one sender. Each bubble is a rectangle on a grid of 2x4
    sub-cells: half a cell in from its side cells, a quarter row above and
    below its text, with rounded corners. Bubbles touch; a wider bubble's
    edge and corner land in its neighbour's row, past the neighbour's end.
    Cells covered by several bubbles merge, so the aligned side is one edge."""
    if len(run) == 1 and len(run[0]) == 1:
        return capsule(run[0][0], fill, pane, fg)
    pad = 1 if roomy else 0
    box = max(max(len(t) for t in lines) + 2 + 2 * pad for lines in run)
    total = 2 + sum(len(lines) for lines in run)
    masks, corners, texts = {}, {}, {}
    row = 1
    for lines in run:
        n = max(len(t) for t in lines)
        w = n + 2 + 2 * pad
        x0 = box - w if right else 0
        sx0, sx1 = 2 * x0 + 1, 2 * (x0 + w) - 1
        sy0, sy1 = 4 * row - 1, 4 * (row + len(lines)) + 1
        for sy in range(sy0, sy1):
            for sx in range(sx0, sx1):
                cell = (sy // 4, sx // 2)
                masks[cell] = masks.get(cell, 0) | bit(sx, sy)
        for kind, sx, sy in (("TL", sx0, sy0), ("TR", sx1 - 1, sy0),
                             ("BL", sx0, sy1 - 1), ("BR", sx1 - 1, sy1 - 1)):
            corners.setdefault((sy // 4, sx // 2), []).append(
                (kind, bit(sx, sy)))
        for i, line in enumerate(lines):
            for j, ch in enumerate(line.ljust(n)):
                texts[(row + i, x0 + 1 + pad + j)] = ch
        row += len(lines)
    rows = []
    for y in range(total):
        cells_ = []
        for x in range(box):
            mask = masks.get((y, x), 0)
            if (y, x) in texts:
                cells_.append((texts[(y, x)], fg, fill))
            elif mask == 0:
                cells_.append((" ", fg, pane))
            elif mask == 255:
                cells_.append((" ", fg, fill))
            else:
                glyph = OCTANT[mask]
                for corner in corners.get((y, x), []):
                    if corner[1] == mask and corner in DISK:
                        glyph = DISK[corner]
                cells_.append((glyph, fill, pane))
        rows.append(cells_)
    return rows


RUN_OPTIONS = {
    "B-compact": ("B, compact: bubbles in a run touch", False),
    "B+-compact": ("B+, compact: bubbles in a run touch", True),
}


def render_runs(key, theme, width):
    pane, other, own, fg, muted = THEMES[theme]
    title, roomy = RUN_OPTIONS[key]
    lines = [emit([(f"{key}  {title}", muted, pane)], width, pane, fg, False),
             blank(width, pane, fg)]
    for mine, run in RUNS:
        rows = run_rows(run, own if mine else other, pane, fg, roomy, mine)
        lines += [emit(r, width, pane, fg, mine) for r in rows]
        lines.append(blank(width, pane, fg))
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--theme", choices=sorted(THEMES), default="dark")
    parser.add_argument("--only", choices=list(OPTIONS) + list(RUN_OPTIONS),
                        action="append",
                        help="show only these options (repeatable)")
    parser.add_argument("--width", type=int, default=60,
                        help="pane width in cells (default 60)")
    args = parser.parse_args()
    keys = args.only or list(OPTIONS) + list(RUN_OPTIONS)
    pane, _, _, fg, muted = THEMES[args.theme]

    term = os.environ.get("TERM", "?")
    kitty = "yes" if os.environ.get("KITTY_WINDOW_ID") else "no"
    tmux = "yes" if os.environ.get("TMUX") else "no"
    print(f"TERM={term}  kitty window={kitty}  tmux={tmux}  theme={args.theme}")
    print("Glyph check (should be shapes, not boxes or question marks):")
    print(emit([("Unicode 16: \U0001FBEC\U0001FBED\U0001FBEE\U0001FBEF   "
                 "Unicode 13: \U0001FB4A\U0001FB42\U0001FB4D\U0001FB3F "
                 "\U0001FB82\u2582   Powerline: \uE0B6\uE0B4",
                 fg, pane)], args.width, pane, fg, False))
    print("Zoom in on the corners: kitty cmd+= (macOS) / ctrl+shift+= (Linux).")
    print()
    for key in keys:
        sys.stdout.write("\n".join(render(key, args.theme, args.width)))
        sys.stdout.write("\n\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
