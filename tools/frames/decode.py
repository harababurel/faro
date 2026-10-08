#!/usr/bin/env python3
"""Show what a recorded frame sent: runs of text after each cursor move.

usage: decode.py <rec-dir> <frame> [row]
"""
import re
import sys

d, n = sys.argv[1], int(sys.argv[2])
row = int(sys.argv[3]) if len(sys.argv) > 3 else None
data = open(f"{d}/rec.bin", "rb").read()
END = b"\x1b[?2026l"
chunks = []
start = 0
while True:
    i = data.find(END, start)
    if i < 0:
        break
    chunks.append(data[start:i + len(END)])
    start = i + len(END)
chunk = chunks[n - 1].decode("utf-8", "replace")
TOKEN = re.compile(r"\x1b\[(\d+);(\d+)H|\x1b\[[0-9;:?]*[A-Za-z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|([^\x1b]+)")
y = x = None
runs = []
cur = None
for m in TOKEN.finditer(chunk):
    if m.group(1):
        y, x = int(m.group(1)) - 1, int(m.group(2)) - 1
        cur = [y, x, ""]
        runs.append(cur)
    elif m.group(3) is not None and cur is not None:
        cur[2] += m.group(3)
    elif m.group(0).startswith("\x1b[") and m.group(0)[-1] not in "mH" and cur is not None:
        cur[2] += "<" + m.group(0)[1:] + ">"
for y, x, text in runs:
    if row is None or y == row:
        print(f"y={y:2} x={x:3} {text!r}")
