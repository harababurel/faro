#!/usr/bin/env python3
"""Record a scripted faro session in a private tmux server.

Writes <out>/rec.bin (faro's raw output, via pipe-pane) and <out>/frames.txt
(faro's intended frames, FARO_DEBUG_FRAMES).

usage: record.py <out-dir> <scenario> [--width W] [--height H] [--split N]
              [--activity MS] [--emoji full|compatible] [--seed S]
              [--layout bubbles|irc] [--steps N] [--binary PATH]

<scenario>: mixed (typing, switching, paging, palette, popups, resizes),
type (typing only), switch (switching and typing), logo (the logo screen
animating while nothing is open: idle spells, resizes across its sizes,
opening and closing a chat, the palette over it) or images (opening chats
with inline images, scrolling across image boxes, popups and resizes).
--split N puts a N-column pane to the left of faro (faro on the right).
<out-dir> is wiped first and holds the demo's directory and a scratch
config, so nothing touches your real ones. Run from the repository root (or set
FARO_REPO); the binary defaults to target/debug/faro. Then compare with
replay.py (AGENTS.md).
"""
import argparse
import os
import random
import shutil
import subprocess
import sys
import time

REPO = os.environ.get("FARO_REPO", os.getcwd())

ap = argparse.ArgumentParser()
ap.add_argument("out")
ap.add_argument("scenario")
ap.add_argument("--width", type=int, default=110)
ap.add_argument("--height", type=int, default=32)
ap.add_argument("--split", type=int, default=0)
ap.add_argument("--activity", type=int, default=300)
ap.add_argument("--emoji", default="full")
ap.add_argument("--seed", type=int, default=7)
ap.add_argument("--layout", default=None)
ap.add_argument("--bubbles", default=None, help="bubble_style: auto, rounded or bar")
ap.add_argument("--steps", type=int, default=300)
ap.add_argument("--steady", action="store_true",
                help="no resizes: a nested replay with --split can't mirror them")
ap.add_argument("--binary", default=os.path.join(REPO, "target/debug/faro"))
args = ap.parse_args()

out = os.path.abspath(args.out)
shutil.rmtree(out, ignore_errors=True)
os.makedirs(out)
# faro demo creates (and marks) its own directory.
data = os.path.join(out, "data")
config = os.path.join(out, "config.toml")
messages = []
if args.layout:
    messages.append(f'layout = "{args.layout}"')
if args.bubbles:
    messages.append(f'bubble_style = "{args.bubbles}"')
if messages:
    with open(config, "w") as f:
        f.write("[messages]\n" + "\n".join(messages) + "\n")

sock = "fb-rec-" + os.path.basename(out)


def tmux(*a, check=True):
    return subprocess.run(["tmux", "-L", sock, "-f", "/dev/null", *a],
                          capture_output=True, text=True, check=check).stdout


total_w = args.width + (args.split + 1 if args.split else 0)
tmux("kill-server", check=False)
tmux("new-session", "-d", "-x", str(total_w), "-y", str(args.height), "-s", "s")
tmux("set", "-g", "remain-on-exit", "on")
tmux("set", "-g", "status", "off")
target = "s"
if args.split:
    # Left pane: something static; faro goes in the new right pane.
    tmux("respawn-pane", "-k", "-t", "s", "sh -c 'while :; do date; sleep 3; done'")
    tmux("split-window", "-h", "-t", "s", "-l", str(args.width), "sleep 100000")
    target = tmux("display", "-p", "-t", "s", "#{pane_id}").strip()
rec = os.path.join(out, "rec.bin")
frames = os.path.join(out, "frames.txt")
cmd = (f"sleep 1; FARO_DEBUG_FRAMES={frames} exec {args.binary} demo "
       f"--dir {data} --config {config} --seed {args.seed} "
       f"--activity {args.activity} --no-faults --emoji {args.emoji}")
tmux("respawn-pane", "-k", "-t", target, cmd)
tmux("pipe-pane", "-o", "-t", target, f"cat > {rec}")
time.sleep(3)


def keys(*k, literal=False):
    a = ["send-keys", "-t", target]
    if literal:
        a.append("-l")
    tmux(*a, *k)


rng = random.Random(args.seed)
WORDS = ["hello", "flags", "🇷🇴", "🇩🇪", "🏳️‍🌈", "🏴‍☠️", "❤️", "👍🏽", "日本", "ok",
         "wrapping", "a", "longer", "sentence", "here", "🧗‍♀️", "x", "✌", "☝🏽", "🫶🏽"]


def type_text(lines):
    for i in range(lines):
        for _ in range(rng.randint(1, 8)):
            keys(rng.choice(WORDS) + " ", literal=True)
            time.sleep(0.02)
        if i < lines - 1:
            keys("M-Enter")
        time.sleep(0.03)


def step(kind):
    if kind == "type":
        type_text(rng.randint(1, 5))
        if rng.random() < 0.7:
            keys("Enter")
        else:
            keys("C-u")  # clear the line
            for _ in range(6):
                keys("C-k")
                keys("BSpace")
    elif kind == "switch":
        keys(rng.choice(["M-j", "M-k"]))
    elif kind == "page":
        keys(rng.choice(["PPage", "PPage", "NPage", "NPage", "NPage"]))
    elif kind == "palette":
        keys("C-p")
        time.sleep(0.1)
        keys(rng.choice(["fl", "a", "pin", ""]), literal=True)
        time.sleep(0.1)
        keys("Down")
        time.sleep(0.05)
        keys("Escape")
    elif kind == "popup":
        keys("Escape")
        keys("Up")
        time.sleep(0.05)
        for _ in range(rng.randint(0, 4)):
            keys("k")
            time.sleep(0.03)
        keys("Enter")
        time.sleep(0.1)
        keys("Escape")
        time.sleep(0.03)
        if rng.random() < 0.5:
            keys("+")
            time.sleep(0.1)
            keys("Escape")
        keys("Escape")
    elif kind in ("resize", "tall") and args.steady:
        time.sleep(rng.uniform(0.5, 1.5))
    elif kind == "tall" and args.split:
        # A height change resizes the whole window, left pane included.
        time.sleep(rng.uniform(0.5, 1.5))
    elif kind == "resize":
        w = rng.choice([args.width, args.width - 13, args.width + 7, 80, 97])
        if args.split:
            tmux("resize-pane", "-t", target, "-x", str(w))
        else:
            tmux("resize-window", "-t", "s", "-x", str(w))
        time.sleep(0.2)
        if rng.random() < 0.5:
            if args.split:
                tmux("resize-pane", "-t", target, "-x", str(args.width))
            else:
                tmux("resize-window", "-t", "s", "-x", str(args.width))
    elif kind == "end":
        keys("End")
    elif kind == "idle":
        time.sleep(rng.uniform(0.5, 1.5))
    elif kind == "openclose":
        keys("Enter")
        time.sleep(rng.uniform(0.2, 0.6))
        for _ in range(3):
            keys("Escape")
            time.sleep(0.05)
    elif kind == "tall":
        h = rng.choice([args.height, args.height, 14, 20, 40])
        tmux("resize-window", "-t", "s", "-y", str(h))
        time.sleep(rng.uniform(0.3, 0.8))
        tmux("resize-window", "-t", "s", "-y", str(args.height))
    time.sleep(rng.uniform(0.05, 0.3))


KINDS = {
    "mixed": ["type"] * 5 + ["switch"] * 3 + ["page"] * 2 + ["palette", "popup", "resize", "end"],
    "type": ["type"],
    "switch": ["switch", "type"],
    "logo": ["idle"] * 4 + ["openclose", "resize", "tall", "palette", "switch"],
    "images": ["switch"] * 3 + ["page"] * 3 + ["popup", "resize", "end", "type"],
}[args.scenario]
if args.scenario == "images":
    keys("Enter")
    time.sleep(0.5)
for n in range(args.steps):
    step(rng.choice(KINDS))
keys("C-q")
time.sleep(2)
tmux("kill-server", check=False)
data_bytes = open(rec, "rb").read()
n_end = data_bytes.count(b"\x1b[?2026l")
n_frames = sum(1 for line in open(frames, encoding="utf-8") if line.startswith("@@ frame "))
print(f"recorded {len(data_bytes)} bytes, {n_end} sync ends, {n_frames} logged frames")
