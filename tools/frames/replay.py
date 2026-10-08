#!/usr/bin/env python3
"""Replay a record.py recording frame by frame in a private tmux server and
compare tmux's grid with faro's intended frame after every frame.

usage: replay.py <rec-dir> [--nested] [--split N] [--max-report K] [--from N]

--nested: attach to the replay server from an outer tmux server and compare
the outer pane too (what tmux sends to its client, as another emulator draws
it). --split N: a N-column pane left of the player (nested only matters).
Prints a summary and writes <rec-dir>/replay-report[-nested].txt.
"""
import argparse
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser()
ap.add_argument("dir")
ap.add_argument("--nested", action="store_true")
ap.add_argument("--split", type=int, default=0)
ap.add_argument("--max-report", type=int, default=5)
ap.add_argument("--stop-after", type=int, default=0, help="stop after K divergent frames")
args = ap.parse_args()

d = os.path.abspath(args.dir)
rec = os.path.join(d, "rec.bin")
data = open(rec, "rb").read()
END = b"\x1b[?2026l"
chunks = []
start = 0
while True:
    i = data.find(END, start)
    if i < 0:
        break
    chunks.append(data[start:i + len(END)])
    start = i + len(END)

intended = []
with open(os.path.join(d, "frames.txt"), encoding="utf-8") as f:
    lines = f.read().split("\n")
i = 0
while i < len(lines):
    line = lines[i]
    if line.startswith("@@ frame "):
        _, _, n, size = line.split(" ")
        w, h = map(int, size.split("x"))
        intended.append((w, h, lines[i + 1:i + 1 + h]))
        i += 1 + h
    else:
        i += 1
if len(intended) != len(chunks):
    print(f"WARNING: {len(chunks)} chunks vs {len(intended)} logged frames")
n_frames = min(len(intended), len(chunks))

tag = os.path.basename(d) + ("-n" if args.nested else "")
inner_sock = "fb-rp-" + tag
outer_sock = "fb-ro-" + tag


def tmux(sock, *a, check=True):
    return subprocess.run(["tmux", "-L", sock, "-f", "/dev/null", *a],
                          capture_output=True, text=True, check=check).stdout


tmpdir = tempfile.mkdtemp(prefix="fbreplay")
cmd_fifo = os.path.join(tmpdir, "cmd")
ack_fifo = os.path.join(tmpdir, "ack")
os.mkfifo(cmd_fifo)
os.mkfifo(ack_fifo)

w0, h0, _ = intended[0]
split_w = args.split + 1 if args.split else 0
for s in (inner_sock, outer_sock):
    tmux(s, "kill-server", check=False)
tmux(inner_sock, "new-session", "-d", "-x", str(w0 + split_w), "-y", str(h0), "-s", "s",
     "sleep 100000")
tmux(inner_sock, "set", "-g", "status", "off")
tmux(inner_sock, "set", "-g", "remain-on-exit", "on")
target = "s"
if args.split:
    tmux(inner_sock, "respawn-pane", "-k", "-t", "s",
         "sh -c 'printf \"left pane\"; sleep 100000'")
    tmux(inner_sock, "split-window", "-h", "-t", "s", "-l", str(w0), "sleep 100000")
target = tmux(inner_sock, "display", "-p", "-t", "s", "#{pane_id}").strip()
if args.nested:
    tmux(inner_sock, "set", "-g", "window-size", "latest")
    tmux(outer_sock, "new-session", "-d", "-x", str(w0 + split_w), "-y", str(h0), "-s", "o",
         f"env -u TMUX tmux -L {inner_sock} -f /dev/null attach -t s")
    tmux(outer_sock, "set", "-g", "status", "off")
    tmux(outer_sock, "set", "-g", "remain-on-exit", "on")
    time.sleep(0.5)
tmux(inner_sock, "respawn-pane", "-k", "-t", target,
     f"python3 {HERE}/player.py {rec} {cmd_fifo} {ack_fifo}")
cmd = open(cmd_fifo, "w")
ack = open(ack_fifo, "r")


def norm(rows):
    return [r.rstrip() for r in rows]


def inner_rows(h):
    return tmux(inner_sock, "capture-pane", "-p", "-t", target).split("\n")[:h]


def outer_rows(h):
    rows = tmux(outer_sock, "capture-pane", "-p", "-t", "o").split("\n")[:h]
    return [r[split_w:] for r in rows]


def wait_match(get, want, timeout):
    deadline = time.time() + timeout
    got = get()
    while norm(got) != want and time.time() < deadline:
        time.sleep(0.01)
        got = get()
    return got


def pane_size():
    out = tmux(inner_sock, "display", "-p", "-t", target, "#{pane_width}x#{pane_height}")
    return tuple(map(int, out.strip().split("x")))


report = []
seen = set()
bad_inner = 0
bad_outer = 0
first_inner = None
first_outer = None
cur = (w0, h0)
for n in range(1, n_frames + 1):
    w, h, rows = intended[n - 1]
    if (w, h) != cur:
        if args.nested:
            tmux(outer_sock, "resize-window", "-t", "o", "-x", str(w + split_w), "-y", str(h))
        else:
            tmux(inner_sock, "resize-window", "-t", "s", "-x", str(w + split_w), "-y", str(h))
        if args.split:
            tmux(inner_sock, "resize-pane", "-t", target, "-x", str(w))
        deadline = time.time() + 2
        while pane_size() != (w, h) and time.time() < deadline:
            time.sleep(0.02)
        cur = (w, h)
    cmd.write(f"{n}\n")
    cmd.flush()
    ack.readline()
    want = norm(rows)
    got = wait_match(lambda: inner_rows(h), want, 0.4)
    inner_ok = norm(got) == want
    outer_ok = True
    if args.nested:
        got_o = wait_match(lambda: outer_rows(h), want, 0.8)
        outer_ok = norm(got_o) == want
    if not inner_ok:
        bad_inner += 1
        first_inner = first_inner or n
    if not outer_ok:
        bad_outer += 1
        first_outer = first_outer or n
    if (not inner_ok or not outer_ok) and len(report) < args.max_report:
        entry = [f"=== frame {n} ({w}x{h}) inner_ok={inner_ok} outer_ok={outer_ok}"]
        for layer, ok, g in (("inner", inner_ok, got),) + ((("outer", outer_ok, got_o),) if args.nested else ()):
            if ok:
                continue
            for r, (a, b) in enumerate(zip(want, norm(g))):
                if a != b and (layer, a, b) not in seen:
                    seen.add((layer, a, b))
                    entry.append(f"[{layer}] row {r}\n  want {a!r}\n  got  {b!r}")
        if len(entry) > 1:
            report.append("\n".join(entry))
    if args.stop_after and (bad_inner + bad_outer) >= args.stop_after:
        break
    if n % 500 == 0:
        print(f"... frame {n}: bad inner {bad_inner}, bad outer {bad_outer}", flush=True)

summary = (f"{os.path.basename(d)}{' nested' if args.nested else ''}: {n} frames replayed, "
           f"divergent inner {bad_inner} (first {first_inner}), "
           f"outer {bad_outer} (first {first_outer})")
print(summary)
with open(os.path.join(d, "replay-report" + ("-nested" if args.nested else "") + ".txt"), "w") as f:
    f.write(summary + "\n\n" + "\n\n".join(report) + "\n")
for s in (inner_sock, outer_sock):
    tmux(s, "kill-server", check=False)
