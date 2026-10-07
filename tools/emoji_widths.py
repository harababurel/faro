#!/usr/bin/env python3
"""Measure how wide this terminal draws emoji and other tricky text, and
compare with the widths faro assumes (unicode-width 0.2.2, per grapheme).

Run it in the terminal you want to check, from the repo root:

  python3 tools/emoji_widths.py

Run it twice: once inside the tmux where faro runs, and once directly in the
terminal you look at (kitty etc., outside tmux). Any "DIFFERS" line is a
grapheme faro and that terminal disagree on: it shifts or garbles the rest of
the line. Paste the whole output into the issue or chat.

Uses cursor position reports (ESC [6n); plain Python, no dependencies.
"""

import os
import platform
import subprocess
import sys
import termios
import tty

# (name, text, width faro assumes)
SAMPLES = [
    ("ascii", "abc", 3),
    ("CJK", "陈伟", 4),
    ("arabic", "مريم", 4),
    ("combining accent", "e\u0301", 1),
    ("check marks", "✓✓", 2),
    ("dotted circle", "◌", 1),
    ("wide emoji", "😂", 2),
    ("wide emoji + VS16", "😂\ufe0f", 2),
    ("paperclip", "📎", 2),
    ("heart, bare", "❤", 1),
    ("heart + VS16", "❤\ufe0f", 2),
    ("smile + VS16", "☺\ufe0f", 2),
    ("victory + VS16", "✌\ufe0f", 2),
    ("phone, bare", "☎", 1),
    ("phone + VS16", "☎\ufe0f", 2),
    ("keycap", "1\ufe0f\u20e3", 2),
    ("thumbs up", "👍", 2),
    ("thumbs up + skin tone", "👍\U0001f3fd", 2),
    ("flag DE", "🇩🇪", 2),
    ("lone regional indicator", "🇩", 1),
    ("rainbow flag (ZWJ)", "🏳\ufe0f\u200d🌈", 2),
    ("family (ZWJ)", "👨\u200d👩\u200d👧\u200d👦", 2),
    ("cyclist (ZWJ)", "🚴\u200d♂\ufe0f", 2),
    ("climber (ZWJ)", "🧗\u200d♀\ufe0f", 2),
]


def cursor_column(fd):
    os.write(fd, b"\x1b[6n")
    reply = b""
    while not reply.endswith(b"R"):
        chunk = os.read(fd, 1)
        if not chunk:
            raise RuntimeError("no cursor position report")
        reply += chunk
    _row, col = reply[reply.rindex(b"[") + 1 : -1].split(b";")
    return int(col)


def measure():
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    tty.setraw(fd)
    results = []
    try:
        for name, text, expected in SAMPLES:
            os.write(1, b"\r\x1b[2K")
            start = cursor_column(fd)
            os.write(1, text.encode())
            results.append((name, text, expected, cursor_column(fd) - start))
        os.write(1, b"\r\x1b[2K")
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)
    return results


def tmux_version():
    if not os.environ.get("TMUX"):
        return "not in tmux"
    try:
        return subprocess.run(
            ["tmux", "-V"], capture_output=True, text=True, timeout=5
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return "in tmux (version unknown)"


def main():
    if not sys.stdin.isatty():
        sys.exit("run this in a terminal")
    results = measure()
    print(f"system: {platform.system()} {platform.release()}")
    print(f"TERM={os.environ.get('TERM', '')} "
          f"TERM_PROGRAM={os.environ.get('TERM_PROGRAM', '')} "
          f"TERM_PROGRAM_VERSION={os.environ.get('TERM_PROGRAM_VERSION', '')}")
    print(f"tmux: {tmux_version()}")
    if os.environ.get("KITTY_WINDOW_ID"):
        print("kitty: yes (outside tmux)")
    print()
    differ = 0
    for name, _text, expected, got in results:
        mark = "ok" if got == expected else "DIFFERS"
        differ += got != expected
        print(f"{name:26} faro={expected} terminal={got}  {mark}")
    print()
    print(f"{differ} of {len(results)} differ")


main()
