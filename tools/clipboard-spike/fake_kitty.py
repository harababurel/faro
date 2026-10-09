#!/usr/bin/env python3
"""Fake kitty for the D89 phase 0 spike.

Runs clipboard-spike in a pty (directly, or inside a private tmux attached to
our pty), answers OSC 5522 read requests like kitty does, types keys, and
checks what the spike reports and saves.

    python3 fake_kitty.py target/debug/clipboard-spike IMAGE SCRATCH [--tmux]
        [--split-max N] [--prompt-delay S] [--late]
"""

import argparse
import base64
import os
import pty
import random
import re
import select
import shutil
import struct
import subprocess
import sys
import termios
import fcntl
import time

REQUEST = re.compile(rb"\x1b\]5522;([^\x1b\x07]*)(?:\x1b\\|\x07)")
TYPES = b"image/png text/html"


def packet(meta, payload=None):
    body = b"type=read:" + meta
    if payload is not None:
        body += b";" + payload
    return b"\x1b]5522;" + body + b"\x1b\\"


def reply_for(payload_b64, image):
    want = base64.b64decode(payload_b64).decode()
    out = [packet(b"status=OK")]
    if want == ".":
        out.append(packet(b"status=DATA:mime=" + base64.b64encode(b"."), base64.b64encode(TYPES)))
    else:
        mime = base64.b64encode(want.split()[0].encode())
        for i in range(0, len(image), 4096):
            out.append(packet(b"status=DATA:mime=" + mime, base64.b64encode(image[i:i + 4096])))
    out.append(packet(b"status=DONE"))
    return want, b"".join(out)


class Harness:
    def __init__(self, args):
        self.args = args
        self.image = open(args.image, "rb").read()
        self.out = b""
        self.seen = 0  # offset in self.out already scanned for requests
        self.requests = []
        self.rng = random.Random(args.seed)

    def spawn(self):
        spike = os.path.abspath(self.args.spike)
        log = os.path.join(self.args.scratch, "spike.log")
        if os.path.exists(log):
            os.remove(log)
        self.log = log
        cmd = [spike, "--log", log, "--out", self.args.scratch] + self.args.spike_args.split()
        env = {k: v for k, v in os.environ.items() if k != "TMUX"}
        env["TERM"] = "xterm-256color"
        if self.args.tmux:
            conf = os.path.join(self.args.scratch, "tmux.conf")
            with open(conf, "w") as f:
                f.write("set -g allow-passthrough on\nset -g focus-events on\nset -g status off\n")
            self.tmux = ["tmux", "-L", "fake-kitty", "-f", conf]
            subprocess.run(self.tmux + ["kill-server"], stderr=subprocess.DEVNULL)
            subprocess.run(self.tmux + ["new-session", "-d", "-x", "100", "-y", "30", "-s", "s"], env=env, check=True)
            subprocess.run(self.tmux + ["set", "-g", "remain-on-exit", "on"], check=True)
            subprocess.run(self.tmux + ["respawn-pane", "-k", "-t", "s", " ".join(cmd)], check=True)
            cmd = self.tmux + ["attach", "-t", "s"]
        pid, fd = pty.fork()
        if pid == 0:
            os.execvpe(cmd[0], cmd, env)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        self.pid, self.fd = pid, fd

    def pump(self, seconds):
        """Read output for `seconds`, answering requests."""
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], max(0.0, min(0.05, end - time.time())))
            if r:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return
                if not data:
                    return
                self.out += data
                self.answer()

    def answer(self):
        for m in REQUEST.finditer(self.out, self.seen):
            self.seen = m.end()
            meta, _, payload = m.group(1).partition(b";")
            want, reply = reply_for(payload, self.image)
            self.requests.append((meta.decode(), want))
            if want != "." and self.args.prompt_delay:
                self.pump_quiet(self.args.prompt_delay)
            self.send_split(reply)

    def pump_quiet(self, seconds):
        # Keep draining output while "the user answers the prompt".
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if r:
                self.out += os.read(self.fd, 65536)

    def send_split(self, data):
        i = 0
        while i < len(data):
            n = self.rng.randint(1, self.args.split_max)
            # Sometimes end a write right after an ESC.
            esc = data.find(b"\x1b", i + 1, i + n)
            if esc != -1 and self.rng.random() < 0.3:
                n = esc + 1 - i
            os.write(self.fd, data[i:i + n])
            i += n
            if self.args.pace:
                time.sleep(self.args.pace)
            elif self.rng.random() < 0.05:
                time.sleep(0.002)

    def type(self, text):
        os.write(self.fd, text.encode())

    def log_lines(self):
        try:
            return open(self.log).read().splitlines()
        except FileNotFoundError:
            return []

    def wait_for(self, pattern, count, timeout):
        end = time.time() + timeout
        while time.time() < end:
            hits = [l for l in self.log_lines() if re.search(pattern, l)]
            if len(hits) >= count:
                return hits
            self.pump(0.1)
        return [l for l in self.log_lines() if re.search(pattern, l)]

    def stop(self):
        self.type("q")
        self.pump(1)
        if self.args.tmux:
            subprocess.run(self.tmux + ["kill-server"], stderr=subprocess.DEVNULL)
        try:
            os.kill(self.pid, 9)
        except ProcessLookupError:
            pass


def main():
    p = argparse.ArgumentParser()
    p.add_argument("spike")
    p.add_argument("image")
    p.add_argument("scratch")
    p.add_argument("--tmux", action="store_true")
    p.add_argument("--split-max", type=int, default=3000)
    p.add_argument("--prompt-delay", type=float, default=0.5)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--rounds", type=int, default=2)
    p.add_argument("--keys", default="1234")
    p.add_argument("--spike-args", default="", help="extra arguments for the spike")
    p.add_argument("--expect-images", type=int, default=None)
    p.add_argument("--pace", type=float, default=0.0, help="seconds to sleep after each write")
    args = p.parse_args()
    os.makedirs(args.scratch, exist_ok=True)
    for f in os.listdir(args.scratch):
        if f.startswith("clipboard-"):
            os.remove(os.path.join(args.scratch, f))

    h = Harness(args)
    h.spawn()
    h.pump(1.5)
    results = 0
    typed = 0
    ok = True
    for rnd in range(args.rounds):
        labels = {"1": "A list", "2": "A image", "3": "B list", "4": "B image"}
        for key, label in ((k, labels[k]) for k in args.keys):
            h.type(key)
            typed += 1
            # Image reads report the list first, then the image.
            results += 1 if key in "13" else 2
            hits = h.wait_for(r"^RESULT", results, 60)
            if len(hits) < results:
                print(f"round {rnd} {label}: timed out waiting for RESULT")
                ok = False
                break
            # Typing right after a read must arrive as keys, nothing else.
            h.type("ab")
            typed += 2
            keys = h.wait_for(r"^event: key", typed, 5)
            if len(keys) != typed:
                print(f"round {rnd} {label}: expected {typed} key events, got {len(keys)}")
                ok = False
        if not ok:
            break
    lines = h.log_lines()
    h.stop()

    for l in lines:
        if l.startswith(("RESULT", "A:", "B:", "LATE")):
            print(l[:400])
    unexpected = [l for l in lines if l.startswith("event: key") and not re.match(r"event: key Char\('[1234ab]'\)$", l)]
    if unexpected:
        ok = False
        print(f"UNEXPECTED key events: {len(unexpected)}; first: {unexpected[:5]}")
    saved = sorted(f for f in os.listdir(args.scratch) if f.startswith("clipboard-"))
    for f in saved:
        same = open(os.path.join(args.scratch, f), "rb").read() == h.image
        ok &= same
        print(f"{f}: {'identical' if same else 'DIFFERENT'}")
    images = sum(k in "24" for k in args.keys) * args.rounds
    if args.expect_images is not None:
        images = args.expect_images
    if len(saved) != images:
        ok = False
        print(f"expected {images} saved images, got {len(saved)}")
    print(f"requests seen by fake kitty: {len(h.requests)}")
    print("PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
