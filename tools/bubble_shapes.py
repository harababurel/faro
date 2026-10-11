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


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--theme", choices=sorted(THEMES), default="dark")
    parser.add_argument("--only", choices=sorted(OPTIONS), action="append",
                        help="show only these options (repeatable)")
    parser.add_argument("--width", type=int, default=60,
                        help="pane width in cells (default 60)")
    args = parser.parse_args()
    keys = args.only or list(OPTIONS)
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
