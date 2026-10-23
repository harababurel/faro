#!/usr/bin/env python3
"""End-to-end check of clipboard image paste (D89-D91, D150): a real `faro
demo` in a pty, with this script playing kitty (it answers OSC 5522 requests
with a PNG of your choice, and the status report that ends faro's startup image
query).

    cargo build
    python3 tools/clipboard/e2e.py target/debug/faro some.png <scratch dir> [--tmux]
        [--ghostty | --system]

--ghostty plays a Ghostty that says it knows OSC 5522 (`CSI ? 5522 ; 2 $ y`;
TERM=xterm-ghostty, or xterm-256color inside tmux, as Ghostty's ssh wrapper
sets it). --system plays a terminal that doesn't, with faro on the user's own
computer: a stand-in `wl-paste` serves the image and a stand-in `ps` hides any
SSH session around this script; no OSC 5522 request may be sent. A TIFF image
(by its magic bytes) is listed as image/tiff and must be staged as a PNG.

It opens the highlighted chat, presses Ctrl+V, checks that the image was saved
intact (`0600`) and, with --tmux (a private tmux server with allow-passthrough,
read through capture-pane), that it's staged, typing still works and Ctrl+O
shows it. Then it sends the image and checks the pasted file was removed once
copied under the message's name (D93). Prints PASS or FAIL. Needs Python 3 (and tmux for --tmux).
"""
import base64, fcntl, glob, os, pty, re, select, shutil, struct, subprocess, sys, termios, time

REQUEST = re.compile(rb"\x1b\]5522;([^\x1b\x07]*)(?:\x1b\\|\x07)")
SUPPORT = b"\x1b[?5522$p"


def packet(meta, payload=None):
    body = b"type=read:" + meta
    if payload is not None:
        body += b";" + payload
    return b"\x1b]5522;" + body + b"\x1b\\"


def reply_for(payload_b64, image):
    """kitty's answer to a read request: OK, 4096-byte chunks, DONE."""
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

faro, png, scratch = sys.argv[1:4]
use_tmux = "--tmux" in sys.argv
ghostty = "--ghostty" in sys.argv
system = "--system" in sys.argv
image = open(png, "rb").read()
tiff = image[:4] in (b"II*\x00", b"MM\x00*")
MIME = b"image/tiff" if tiff else b"image/png"
TYPES = MIME + b" text/html"
demo = os.path.join(scratch, "demo")
shutil.rmtree(scratch, ignore_errors=True)
os.makedirs(scratch)
cfg = os.path.join(scratch, "config.toml")
if os.environ.get("CFG_TEXT"):
    open(cfg, "w").write(os.environ["CFG_TEXT"])
elif "--no-images" in sys.argv:
    open(cfg, "w").write("[media]\ninline_images = \"off\"\n")
env = {k: v for k, v in os.environ.items() if k not in ("TMUX", "TMUX_PANE")}
env["TERM"] = "xterm-kitty"
if ghostty:
    env["TERM"] = "xterm-256color" if use_tmux else "xterm-ghostty"
if system:
    env["TERM"] = "xterm-256color"
    for var in ("SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY", "DISPLAY"):
        env.pop(var, None)
    env["WAYLAND_DISPLAY"] = "e2e"
    fake = os.path.join(scratch, "bin")
    os.makedirs(fake)
    with open(os.path.join(fake, "ps"), "w") as f:
        f.write("#!/bin/sh\nexit 0\n")
    with open(os.path.join(fake, "wl-paste"), "w") as f:
        f.write('#!/bin/sh\nif [ "$1" = --list-types ]; then printf "text/plain\\n%s\\n" "%s"; '
                'else cat "%s"; fi\n' % ("%s", MIME.decode(), os.path.abspath(png)))
    for tool in ("ps", "wl-paste"):
        os.chmod(os.path.join(fake, tool), 0o755)
    env["PATH"] = fake + os.pathsep + env["PATH"]
cmd = [os.path.abspath(faro), "--config", cfg, "demo", "--dir", demo, "--fresh", "--no-faults"]
if system:  # a tmux pane's login shell would reset PATH and lose the stand-ins
    cmd = ["env", "PATH=" + env["PATH"]] + cmd
if use_tmux:
    conf = os.path.join(scratch, "tmux.conf")
    open(conf, "w").write("set -g allow-passthrough on\nset -g status off\n")
    tmux = ["tmux", "-L", "e2e-paste", "-f", conf]
    subprocess.run(tmux + ["kill-server"], stderr=subprocess.DEVNULL)
    subprocess.run(tmux + ["new-session", "-d", "-x", "120", "-y", "35", "-s", "s"], env=env, check=True)
    subprocess.run(tmux + ["set", "-g", "remain-on-exit", "on"], check=True)
    subprocess.run(tmux + ["respawn-pane", "-k", "-t", "s", " ".join(cmd)], check=True)
    cmd = tmux + ["attach", "-t", "s"]

pid, fd = pty.fork()
if pid == 0:
    os.execvpe(cmd[0], cmd, env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 120, 0, 0))
out = bytearray()
seen = 0
requests = []


def pump(seconds):
    global seen
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if not r:
            continue
        try:
            data = os.read(fd, 65536)
        except OSError:
            return
        if not data:
            return
        out.extend(data)
        # Ghostty knows OSC 5522; the --system terminal doesn't (D150).
        for _ in range(data.count(SUPPORT)):
            os.write(fd, b"\x1b[?5522;2$y" if ghostty else b"\x1b[?5522;0$y")
        # Like every terminal: answer the status report that ends faro's
        # startup image query (ratatui-image) and the D150 question.
        for _ in range(data.count(b"\x1b[5n")):
            os.write(fd, b"\x1b[0n")
        for m in REQUEST.finditer(out, seen):
            seen = m.end()
            meta, _, payload = m.group(1).partition(b";")
            want, reply = reply_for(payload, image)
            ident = re.search(rb"id=([^:;]*)", meta)
            if ident:  # kitty echoes the id in every packet
                reply = reply.replace(b"type=read:", b"type=read:id=" + ident.group(1) + b":")
            requests.append((meta.decode(), want))
            for i in range(0, len(reply), 3000):
                os.write(fd, reply[i:i + 3000])


def screen():
    if use_tmux:
        return subprocess.run(tmux + ["capture-pane", "-p", "-t", "s"], capture_output=True, text=True).stdout
    return ""


for _ in range(30):
    pump(0.5)
    if not use_tmux or "No conversations yet" not in screen():
        break
pump(1)
print("--- at start ---\n" + screen())
os.write(fd, b"\r")       # open the highlighted chat (D60)
pump(1.5)
print("--- before ctrl+v ---\n" + screen())
os.write(fd, b"\x16")     # Ctrl+V
pump(4)
print("--- after ctrl+v ---\n" + screen())
saved = sorted(glob.glob(os.path.join(demo, "attachments", "clipboard-*")))
data = open(saved[-1], "rb").read() if saved else b""
if tiff:
    ok = data.startswith(b"\x89PNG") and saved[-1].endswith(".png")
    image = data  # the sent copy is the PNG
else:
    ok = bool(saved) and data == image
mode = oct(os.stat(saved[-1]).st_mode & 0o777) if saved else "-"
ok &= mode == "0o600"
print("requests:", requests)
if system:
    ok &= requests == []
    print("no OSC 5522 request (system clipboard):", requests == [])
for word in [b"Pasting images", b"Waiting for", b"No image", b"Image pasted", b"Couldn"]:
    print(word, word in out)
print("saved:", saved, "mode", mode, "ok" if ok else "NOT OK (missing, different or not 0600)")
os.write(fd, b"hello")    # input works again
pump(1)
text = screen()
if use_tmux:
    staged = [l for l in text.splitlines() if "clipboard-" in l]
    print("staged bar:", staged)
    print("typed after paste:", "hello" in text)
    ok &= bool(staged) and "hello" in text
os.write(fd, b"\x0f")     # Ctrl+O
pump(1)
if use_tmux:
    print("viewer open:", "Not sent yet" in screen())
os.write(fd, b"\x1b")
pump(0.5)
os.write(fd, b"\r")       # send it, "hello" as the caption
pump(2)
left = glob.glob(os.path.join(demo, "attachments", "clipboard-*"))
copies = [p for p in glob.glob(os.path.join(demo, "attachments", "*")) if open(p, "rb").read() == image]
print("after sending: pasted file left:", left, "copies:", len(copies))
ok &= not left and len(copies) == 1
os.write(fd, b"\x11")     # Ctrl+Q
pump(2)
if use_tmux:
    subprocess.run(tmux + ["kill-server"], stderr=subprocess.DEVNULL)
try:
    os.kill(pid, 9)
except ProcessLookupError:
    pass
print("PASS" if ok else "FAIL")
