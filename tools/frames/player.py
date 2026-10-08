#!/usr/bin/env python3
"""Player for replay.py: runs inside a tmux pane, writes frame N of a recording
to the pane when asked on a FIFO, then acknowledges on another FIFO.

usage: player.py <rec.bin> <cmd-fifo> <ack-fifo>
"""
import os
import sys
import tty

rec, cmd_path, ack_path = sys.argv[1:4]
data = open(rec, "rb").read()
END = b"\x1b[?2026l"
frames = []
start = 0
while True:
    i = data.find(END, start)
    if i < 0:
        break
    frames.append(data[start:i + len(END)])
    start = i + len(END)

# faro runs in raw mode: no newline translation, no echo of what tmux sends
# (focus events, mouse) into the picture.
tty.setraw(0)
cmd = open(cmd_path, "r")
ack = open(ack_path, "w")
for line in cmd:
    n = int(line)
    os.write(1, frames[n - 1])
    ack.write("ok\n")
    ack.flush()
