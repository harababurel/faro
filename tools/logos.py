#!/usr/bin/env python3
"""Logo explorations from the naming phase of faro.

The project name is settled (faro, using the "lighthouse" logo); the other
candidates are kept for reference. This is a throwaway prototype in plain
Python with no dependencies; the real logo will be reimplemented in Rust.

Usage (from the repo root):
  python3 tools/logos.py                        # static gallery of everything
  python3 tools/logos.py faro                   # static gallery, filtered by name or group
  python3 tools/logos.py browse lighthouse      # interactive + animated (see key hints at the bottom)
  python3 tools/logos.py browse faro            # browse everything, starting at a name
  python3 tools/logos.py anim [names/groups]    # animated, auto-cycles every 8s

Groups: lighthouse, signet, signal lamp ("signal lamp" needs quotes), flock, new, earlier

Needs a truecolor terminal (kitty, iTerm2, WezTerm, Ghostty, tmux with RGB enabled).
"""
import math
import os
import random
import select
import shutil
import sys
import time

RESET = "\x1b[0m"


def fg(c):
    return f"\x1b[38;2;{c[0]};{c[1]};{c[2]}m"


def bg(c):
    return f"\x1b[48;2;{c[0]};{c[1]};{c[2]}m"


def lerp(a, b, t):
    t = max(0.0, min(1.0, t))
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def gradient(stops, t):
    t = max(0.0, min(1.0, t))
    seg = t * (len(stops) - 1)
    i = min(int(seg), len(stops) - 2)
    return lerp(stops[i], stops[i + 1], seg - i)


def dim(c, k):
    k = max(0.0, min(1.0, k))
    return tuple(int(x * k) for x in c)


def bright(c, k):
    return lerp(c, (255, 255, 255), k)


# --------------------------------------------------------------------------
# Scene: grid of terminal cells with three layers (priority high -> low):
#   text overrides, half-block pixels (1x2 per cell), braille dots (2x4 per cell)
# --------------------------------------------------------------------------
BRAILLE = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]]


class Scene:
    def __init__(self, w, h):
        self.w, self.h = w, h
        self.px = {}
        self.dots = {}
        self.text = {}

    def pixel(self, x, y, c):
        x, y = int(round(x)), int(round(y))
        if 0 <= x < self.w and 0 <= y < 2 * self.h:
            self.px[(x, y)] = c

    def dot(self, x, y, c, prio=1.0):
        x, y = int(round(x)), int(round(y))
        if 0 <= x < 2 * self.w and 0 <= y < 4 * self.h:
            old = self.dots.get((x, y))
            if old is None or prio >= old[1]:
                self.dots[(x, y)] = (c, prio)

    def line(self, x0, y0, x1, y1, c0, c1=None, prio=1.0):
        c1 = c1 or c0
        n = int(max(abs(x1 - x0), abs(y1 - y0))) + 1
        for k in range(n + 1):
            t = k / n
            self.dot(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, lerp(c0, c1, t), prio)

    def put(self, cx, cy, s, c, bgc=None):
        for k, ch in enumerate(s):
            if 0 <= cx + k < self.w and 0 <= cy < self.h:
                self.text[(cx + k, cy)] = (ch, c, bgc)

    def disk(self, cx, cy, r, colorfn):
        for y in range(int(cy - r - 1), int(cy + r + 2)):
            for x in range(int(cx - r - 1), int(cx + r + 2)):
                dx, dy = x + 0.5 - cx, y + 0.5 - cy
                if dx * dx + dy * dy <= r * r:
                    c = colorfn(x, y, dx, dy)
                    if c:
                        self.pixel(x, y, c)

    def render(self):
        lines = []
        for cy in range(self.h):
            out = []
            for cx in range(self.w):
                if (cx, cy) in self.text:
                    ch, c, bgc = self.text[(cx, cy)]
                    out.append((bg(bgc) if bgc else "") + fg(c) + ch + RESET)
                    continue
                top = self.px.get((cx, 2 * cy))
                bot = self.px.get((cx, 2 * cy + 1))
                if top and bot:
                    out.append(fg(top) + "█" + RESET if top == bot else fg(top) + bg(bot) + "▀" + RESET)
                    continue
                if top:
                    out.append(fg(top) + "▀" + RESET)
                    continue
                if bot:
                    out.append(fg(bot) + "▄" + RESET)
                    continue
                code, best = 0, None
                for dy in range(4):
                    for dx in range(2):
                        d = self.dots.get((2 * cx + dx, 4 * cy + dy))
                        if d:
                            code |= BRAILLE[dy][dx]
                            if best is None or d[1] > best[1]:
                                best = d
                out.append(fg(best[0]) + chr(0x2800 + code) + RESET if code else " ")
            lines.append("".join(out))
        return lines


def waves(sc, t, base, colors, amp=1.6, freq=0.18, speed=2.0, step=4):
    for j, col in enumerate(colors):
        for x in range(2 * sc.w):
            y = base + j * step + amp * math.sin(x * freq + t * speed * (1 + 0.3 * j) + j * 1.7)
            sc.dot(x, y, col, 0.5)


def starfield(sc, t, n, seed, region=None, color=(200, 200, 230)):
    rnd = random.Random(seed)
    x0, y0, x1, y1 = region or (0, 0, 2 * sc.w, 4 * sc.h)
    for i in range(n):
        x, y = rnd.uniform(x0, x1), rnd.uniform(y0, y1)
        ph, sp = rnd.uniform(0, 6.3), rnd.uniform(1, 3)
        k = 0.25 + 0.75 * (0.5 + 0.5 * math.sin(t * sp + ph))
        sc.dot(x, y, dim(color, k), 0.05)


# --------------------------------------------------------------------------
# Block-letter wordmark
# --------------------------------------------------------------------------
FONT = {
    "A": [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "B": ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."],
    "C": [".####", "#....", "#....", "#....", "#....", "#....", ".####"],
    "D": ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."],
    "E": ["#####", "#....", "#....", "####.", "#....", "#....", "#####"],
    "G": [".####", "#....", "#....", "#..##", "#...#", "#...#", ".###."],
    "H": ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "F": ["#####", "#....", "#....", "####.", "#....", "#....", "#...."],
    "I": ["###", ".#.", ".#.", ".#.", ".#.", ".#.", "###"],
    "K": ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"],
    "Q": [".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"],
    "V": ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."],
    "L": ["#....", "#....", "#....", "#....", "#....", "#....", "#####"],
    "M": ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"],
    "N": ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"],
    "O": [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "P": ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."],
    "R": ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
    "S": [".####", "#....", "#....", ".###.", "....#", "....#", "####."],
    "T": ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
    "U": ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "W": ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#"],
}


def wordmark(word, stops, scale):
    glyphs = [FONT[c] for c in word.upper()]
    gap = scale
    W = sum(len(g[0]) * scale for g in glyphs) + gap * (len(glyphs) - 1)
    H = 7 * scale
    sc = Scene(W, (H + 1) // 2)
    x0 = 0
    for g in glyphs:
        for gy, row in enumerate(g):
            for gx, ch in enumerate(row):
                if ch != "#":
                    continue
                for sx in range(scale):
                    for sy in range(scale):
                        X, Y = x0 + gx * scale + sx, gy * scale + sy
                        c = gradient(stops, X / max(W - 1, 1))
                        sc.pixel(X, Y, dim(c, 1 - 0.3 * Y / H))
        x0 += len(g[0]) * scale + gap
    return sc


# --------------------------------------------------------------------------
# helio: heliograph sun on the horizon
# --------------------------------------------------------------------------
def icon_helio(t, static):
    W, H, HY = 40, 12, 8
    sc = Scene(W, H)
    if static:
        t = 0.6
    cxp, cyp, r = 20, 2 * HY, 8
    for y in range(cyp - r, cyp):
        if y in (cyp - 2, cyp - 5):
            continue
        for x in range(cxp - r - 1, cxp + r + 1):
            if (x + 0.5 - cxp) ** 2 + (y + 0.5 - cyp) ** 2 <= r * r:
                sc.pixel(x, y, gradient([(255, 238, 120), (255, 165, 60), (255, 85, 95)], (y - (cyp - r)) / r))
    cx, cy = 2 * cxp, 4 * HY
    step = math.radians(15)
    a, k = (t * 0.35) % step, 0
    while a < math.pi:
        fade = max(0.0, min(1.0, math.sin(a) * 3))
        if fade > 0.05:
            L = 6 + 3 * math.sin(t * 3 + k * 1.3)
            s0, s1 = 2 * r + 4, 2 * r + 4 + L
            sc.line(cx + s0 * math.cos(a), cy - s0 * math.sin(a), cx + s1 * math.cos(a), cy - s1 * math.sin(a),
                    dim((255, 225, 110), fade), dim((255, 120, 60), fade * 0.5))
        a += step
        k += 1
    for x in range(2 * W):
        e = 1 - abs(x - cx) / cx
        sc.dot(x, cy, dim(lerp((200, 60, 140), (255, 140, 60), e), 0.35 + 0.65 * e))
    for i, row in enumerate(range(HY + 1, H)):
        half = 8 - 2 * i
        off = round(math.sin(t * 3.5 + i) * 1.2)
        c = dim(gradient([(255, 170, 70), (230, 80, 130)], i / 2), 1 - 0.25 * i)
        for x in range(cxp - half + off, cxp + half + off):
            if (x + int(t * 6) + 2 * i) % 4:
                sc.put(x, row, "─", c)
    return sc


# --------------------------------------------------------------------------
# lumen: a glowing orb radiating rings of light
# --------------------------------------------------------------------------
def icon_lumen(t, static):
    W, H = 34, 12
    sc = Scene(W, H)
    if static:
        t = 0.4
    cxp, cyp, r = 17, 12, 5.5
    pulse = 0.5 + 0.5 * math.sin(t * 2.5)

    def orb(x, y, dx, dy):
        d = math.sqrt(dx * dx + dy * dy) / r
        c = gradient([(255, 255, 240), (255, 236, 150), (255, 190, 80), (240, 130, 50)], d)
        return bright(c, 0.15 * pulse * (1 - d))

    sc.disk(cxp, cyp, r, orb)
    cx, cy = 2 * cxp, 2 * cyp
    phase = (t * 5) % 5
    for k in range(5):
        rad = 2 * r + 3 + k * 5 + phase
        fade = max(0.0, 1 - (rad - 2 * r) / 28)
        col = dim(gradient([(255, 230, 140), (255, 150, 70)], 1 - fade), fade)
        n = int(rad * 6)
        for i in range(n):
            if i % 2:
                continue
            th = 2 * math.pi * i / n + k * 0.3
            sc.dot(cx + rad * math.cos(th), cy + rad * math.sin(th), col, fade)
    return sc


# --------------------------------------------------------------------------
# morse: a signal lamp flashing the name in Morse code
# --------------------------------------------------------------------------
MORSE = {"A": ".-", "B": "-...", "C": "-.-.", "D": "-..", "E": ".", "F": "..-.", "G": "--.",
         "H": "....", "I": "..", "K": "-.-", "L": ".-..", "M": "--", "N": "-.", "O": "---",
         "P": ".--.", "Q": "--.-", "R": ".-.", "S": "...", "T": "-", "U": "..-", "V": "...-", "W": ".--"}
_TL_CACHE = {}


def morse_timeline(word):
    if word in _TL_CACHE:
        return _TL_CACHE[word]
    tl, starts = [], []
    for li, ch in enumerate(word):
        starts.append((len(tl), ch))
        for si, sym in enumerate(MORSE[ch]):
            tl += [1] if sym == "." else [2, 2, 2]
            if si < len(MORSE[ch]) - 1:
                tl.append(0)
        tl += [0, 0, 0] if li < len(word) - 1 else [0] * 7
    _TL_CACHE[word] = (tl, starts)
    return tl, starts


def icon_morse(t, static, name="morse"):
    W, H, LENS = 60, 9, 45
    sc = Scene(W, H)
    MORSE_TL, MORSE_STARTS = morse_timeline(name.upper())
    total = len(MORSE_TL) - 7
    f = max(44, total) if static else int(t / 0.07)
    lit = static or MORSE_TL[f % len(MORSE_TL)] > 0
    steel_hi, steel_lo = (150, 160, 175), (70, 78, 92)
    lens = (255, 248, 210) if lit else (95, 75, 35)

    def P(x, y, c):
        sc.pixel(44 + (15 - x), y, c)

    for x in range(3, 10):
        P(x, 1, steel_hi)
    for y in (2, 3):
        P(3, y, steel_hi)
        P(9, y, steel_hi)
    for y in range(4, 14):
        for x in range(0, 11):
            P(x, y, lerp(steel_hi, steel_lo, (y - 4) / 9 + (0.15 if x in (3, 7) else 0)))
    for y in range(3, 15):
        P(11, y, (185, 190, 200))
        P(12, y, (165, 170, 182))
    for y in range(5, 13):
        c = lens if y % 2 else dim(lens, 0.82)
        P(13, y, c)
        P(14, y, c)
    for x in (13, 14, 15):
        P(x, 3, (120, 128, 140))
        P(x, 14, (120, 128, 140))
    for y in range(14, 18):
        for x in (4, 5, 6):
            P(x, y, (55, 45, 40))
    cone = (150, 105, 40) if lit else (55, 40, 18)
    lx, ly = 2 * LENS - 2, 18
    sc.line(lx, ly - 3, 0, 1, cone, dim(cone, 0.35), prio=0.1)
    sc.line(lx, ly + 2, 0, 4 * H - 2, cone, dim(cone, 0.35), prio=0.1)
    for x in range(LENS):
        u = f - (LENS - 1 - x)
        if u < 0 or (static and u >= len(MORSE_TL)):
            continue
        s = MORSE_TL[u % len(MORSE_TL)]
        if not s:
            continue
        d = (LENS - 1 - x) / LENS
        c = dim(gradient([(255, 250, 225), (255, 200, 80), (255, 120, 50)], d), 1 - 0.45 * d)
        sc.put(x, 4, "•" if s == 1 else "━", c)
    if static:
        for u, ch in MORSE_STARTS:
            sc.put(LENS - 1 - (f - u), 5, ch, (130, 110, 80))
    return sc


# --------------------------------------------------------------------------
# pharos: lighthouse with a rotating beam
# --------------------------------------------------------------------------
def icon_pharos(t, static):
    W, H = 40, 13
    sc = Scene(W, H)
    if static:
        t = 0.35
    cx = 20
    red, white = (210, 60, 60), (235, 235, 240)
    sc.pixel(cx, 1, (150, 40, 45))
    for x in range(cx - 1, cx + 2):
        sc.pixel(x, 2, (150, 40, 45))
    for x in range(cx - 2, cx + 3):
        sc.pixel(x, 3, (170, 45, 50))
    theta = t * 1.4
    glow = 0.6 + 0.4 * max(0.0, math.sin(theta))
    for y in (4, 5, 6):
        for x in range(cx - 1, cx + 2):
            sc.pixel(x, y, dim((255, 240, 170), 0.55 + 0.45 * glow))
    for x in range(cx - 3, cx + 4):
        sc.pixel(x, 7, (60, 60, 70))
    for y in range(8, 23):
        half = 2 + (y - 8) * 2.2 / 14
        stripe = red if ((y - 8) // 3) % 2 == 0 else white
        for x in range(int(round(cx - half)), int(round(cx + half)) + 1):
            shade = 1 - 0.35 * max(0.0, (x - cx) / max(half, 1))
            sc.pixel(x, y, dim(stripe, shade))
    for y, half in ((23, 7), (24, 9), (25, 11)):
        for x in range(cx - half, cx + half + 1):
            sc.pixel(x, y, dim((95, 95, 105), 0.6 + 0.4 * ((x * 7 + y) % 3) / 2))
    # Beam
    bx, by = 2 * cx + 1, 10
    L = math.cos(theta) * 38
    if abs(L) > 1:
        spread = 2 + abs(L) / 7
        k0 = (255, 240, 170)
        inten = 0.45 + 0.55 * max(0.0, math.sin(theta))
        for s in range(-int(spread), int(spread) + 1):
            frac = abs(s) / (spread + 1)
            col0 = dim(k0, inten * (1 - 0.6 * frac))
            sc.line(bx, by, bx + L, by + s, col0, dim(k0, inten * 0.2), prio=0.3 - frac * 0.1)
    waves(sc, t, 44, [(35, 95, 180), (60, 140, 225)], amp=1.2, step=3)
    starfield(sc, t, 14, 7, (0, 0, 80, 22), (190, 190, 225))
    return sc


# --------------------------------------------------------------------------
# cairn: stones stacking up on a trail
# --------------------------------------------------------------------------
CAIRN_STONES = [(14, 4, 0, (125, 130, 140)), (11, 4, 1, (150, 140, 128)), (9, 3, -1, (112, 122, 134)),
                (6, 3, 1, (160, 152, 140)), (4, 2, 0, (138, 144, 155))]


def icon_cairn(t, static):
    W, H = 30, 12
    sc = Scene(W, H)
    cx, ground = 15, 21
    cycle = 6.0
    tc = 99 if static else t % cycle
    # Distant mountains
    for x in range(2 * W):
        y = 30 - 9 * abs(math.sin(x * 0.045 + 0.6)) - 4 * abs(math.sin(x * 0.11))
        sc.dot(x, y, (55, 70, 100), 0.05)
    for x in range(2 * W):
        sc.dot(x, 2 * ground + 2, (90, 120, 70) if x % 3 else (120, 150, 80), 0.2)
    base = ground
    for i, (w, h, off, col) in enumerate(CAIRN_STONES):
        appear = i * 0.6
        if tc < appear:
            break
        fall = max(0.0, 1 - (tc - appear) / 0.35)
        top = base - h + 1 - int(fall * fall * 14)
        for j in range(h):
            v = abs(2 * (j + 0.5) / h - 1)
            half = (w / 2) * (1 - v ** 3) ** (1 / 3)
            for x in range(int(round(cx + off - half)), int(round(cx + off + half))):
                shade = 1.1 - 0.35 * j / h - 0.15 * ((x - cx - off) / max(half, 1))
                sc.pixel(x, top + j, lerp(dim(col, shade), (110, 150, 90), 0.25 if (j == h - 1 and i == 0) else 0))
        base -= h
    return sc


# --------------------------------------------------------------------------
# glint: a twinkling four-pointed sparkle
# --------------------------------------------------------------------------
def sparkle(sc, cx, cy, Lh, Lv, cols, prio=1.0):
    if Lh < 1 or Lv < 1:
        return
    for y in range(int(cy - Lv), int(cy + Lv) + 1):
        for x in range(int(cx - Lh), int(cx + Lh) + 1):
            u, v = abs(x - cx) / Lh, abs(y - cy) / Lv
            s = math.sqrt(u) + math.sqrt(v)
            if s <= 1:
                sc.dot(x, y, gradient(cols, s), prio)


def icon_glint(t, static):
    W, H = 30, 12
    sc = Scene(W, H)
    if static:
        t = 0.3
    cols = [(255, 255, 255), (200, 240, 255), (150, 160, 255), (120, 90, 220)]
    s = 0.8 + 0.2 * math.sin(t * 3)
    sparkle(sc, 30, 24, 26 * s, 21 * s, cols, 2)
    d = 7 * s
    for k in range(int(d)):
        c = dim((200, 220, 255), 1 - k / d)
        for sx, sy in ((1, 1), (1, -1), (-1, 1), (-1, -1)):
            sc.dot(30 + sx * k, 24 + sy * k, c, 1.5)
    for i, (x, y, ph) in enumerate(((8, 8, 0.0), (52, 10, 2.1), (48, 40, 4.0), (12, 38, 1.2))):
        k = max(0.0, math.sin(t * 2.2 + ph))
        sparkle(sc, x, y, 6 * k, 5 * k, cols, 1)
    return sc


# --------------------------------------------------------------------------
# wisp: a will-o'-the-wisp flame with drifting embers
# --------------------------------------------------------------------------
def icon_wisp(t, static):
    W, H = 30, 12
    sc = Scene(W, H)
    if static:
        t = 0.5
    cx, base = 15, 20
    height = 15 * (1 + 0.06 * math.sin(t * 7) + 0.04 * math.sin(t * 13))
    for y in range(base - int(height), base + 1):
        u = (base - y) / height
        if u < 0.35:
            w = 5.2 * math.sqrt(max(0.0, 1 - ((0.35 - u) / 0.35) ** 2))
        else:
            w = 5.2 * (1 - (u - 0.35) / 0.65) ** 1.4
        sway = math.sin(t * 3 + y * 0.45) * u * 2.2
        for x in range(int(cx - w - 1), int(cx + w + 2)):
            dx = abs(x + 0.5 - cx - sway)
            if dx <= w:
                core = 1 - min(1.0, (dx / max(w, 0.5)) * 0.7 + u * 0.6)
                c = gradient([(40, 140, 120), (80, 220, 200), (190, 255, 245), (255, 255, 255)], core)
                sc.pixel(x, y, c)
    rnd = random.Random(3)
    for i in range(14):
        ph, sp, xo = rnd.uniform(0, 1), rnd.uniform(0.6, 1.2), rnd.uniform(-6, 6)
        p = (t * 0.35 * sp + ph) % 1
        y = 2 * (base - height * 0.7) - p * 30
        x = 2 * cx + xo + math.sin(t * 2 + i) * 3
        sc.dot(x, y, dim((130, 255, 230), 1 - p), 0.5)
    for x in range(2 * W):
        y = 44 + 1.2 * math.sin(x * 0.12 + t * 0.8)
        sc.dot(x, y, dim((70, 110, 120), 0.5 + 0.5 * math.sin(x * 0.3)), 0.1)
    return sc


# --------------------------------------------------------------------------
# sotto: a speech bubble with a typing indicator
# --------------------------------------------------------------------------
def icon_sotto(t, static):
    W, H = 32, 10
    sc = Scene(W, H)
    if static:
        t = 0.25
    x0, x1, y0, y1 = 2, 29, 2, 13
    cols = [(150, 125, 230), (110, 165, 240)]

    def fill(x):
        return gradient(cols, (x - x0) / (x1 - x0))

    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            if x in (x0, x1) and y in (y0, y1):
                continue  # rounded corners
            sc.pixel(x, y, fill(x))
    for i, y in enumerate(range(y1 + 1, y1 + 4)):
        for x in range(5, 9 - i - (1 if i == 2 else 0)):
            sc.pixel(x, y, fill(x))
    cy = 4
    for i in range(3):
        cxx = 10 + i * 5
        k = max(0.0, math.sin(t * 5 - i * 0.9))
        sc.put(cxx, cy, "●" if k > 0.5 else "•", lerp((200, 205, 245), (255, 255, 255), k), fill(cxx))
    for i in range(3):
        rad = 6 + i * 4
        k = 0.5 + 0.5 * math.sin(t * 3 - i)
        for a in range(-40, 41, 8):
            th = math.radians(a)
            sc.dot(2 * x1 + 2 + rad * math.cos(th), 15 + rad * math.sin(th) * 1.1,
                   dim((150, 160, 230), (0.2 + 0.5 * k) * (1 - i * 0.25)), 0.2)
    return sc


# --------------------------------------------------------------------------
# tightbeam: two nodes linked by a narrow laser, messages flowing both ways
# --------------------------------------------------------------------------
def icon_tightbeam(t, static):
    W, H = 40, 9
    sc = Scene(W, H)
    if static:
        t = 0.55
    starfield(sc, t, 24, 11)
    sc.disk(5, 9, 4.5, lambda x, y, dx, dy: lerp((120, 170, 255), (30, 60, 140), (dx + dy + 6) / 10))
    for x in range(-1, 12):
        y = 9 + (x - 5) * 0.25
        if not (abs(x - 5) < 3):
            sc.pixel(x, round(y), (170, 150, 220))
    for y in range(8, 11):
        for x in range(33, 36):
            sc.pixel(x, y, (180, 185, 195))
    for x in list(range(29, 32)) + list(range(37, 40)):
        sc.pixel(x, 9, (70, 120, 210))
    sc.pixel(34, 7, (220, 220, 230))
    x0, x1, row = 11, 28, 4
    n = x1 - x0 + 1
    for x in range(x0, x1 + 1):
        sc.put(x, row, "─", (110, 35, 60))
    pr = (t * 14) % (n + 6) - 3
    pl = n - 1 - ((t * 11 + 7) % (n + 6) - 3)
    for k in range(4):
        xa = int(pr) - k
        if 0 <= xa < n:
            sc.put(x0 + xa, row, "━", dim((255, 110, 170), 1 - k * 0.25))
        xb = int(pl) + k
        if 0 <= xb < n:
            sc.put(x0 + xb, row, "━", dim((120, 230, 255), 1 - k * 0.25))
    return sc


# --------------------------------------------------------------------------
# signet: an envelope closed with a wax seal
# --------------------------------------------------------------------------
def icon_signet(t, static):
    W, H = 30, 12
    sc = Scene(W, H)
    if static:
        t = 0.9
    paper = (225, 205, 165)
    for x in range(2, 28):
        sc.put(x, 0, "─", paper)
        sc.put(x, 11, "─", paper)
    for y in range(1, 11):
        sc.put(1, y, "│", paper)
        sc.put(28, y, "│", paper)
    sc.put(1, 0, "╭", paper)
    sc.put(28, 0, "╮", paper)
    sc.put(1, 11, "╰", paper)
    sc.put(28, 11, "╯", paper)
    sc.line(5, 4, 30, 24, dim(paper, 0.8), dim(paper, 0.5))
    sc.line(55, 4, 30, 24, dim(paper, 0.8), dim(paper, 0.5))
    cxp, cyp = 15, 13

    def wax(x, y, dx, dy):
        r = math.hypot(dx, dy)
        th = math.atan2(dy, dx)
        if r > 5.6 + 0.7 * math.sin(5 * th + 1):
            return None
        c = lerp((205, 50, 55), (120, 20, 30), (dx + dy + 6) / 12)
        band = (x + y - t * 14) % 26
        if band < 2.5:
            c = bright(c, 0.35)
        return c

    sc.disk(cxp, cyp, 7, wax)
    glyph = FONT["S"]
    for gy, row in enumerate(glyph):
        for gx, ch in enumerate(row):
            if ch == "#":
                x, y = cxp - 2 + gx, cyp - 4 + gy
                c = sc.px.get((x, y))
                if c:
                    sc.pixel(x, y, dim(c, 0.6))
    return sc


# --------------------------------------------------------------------------
# starling: a swirling murmuration
# --------------------------------------------------------------------------
STARLINGS = [(random.Random(i).uniform(0, 2 * math.pi), random.Random(i * 7 + 1).random() ** 0.6,
              random.Random(i * 13 + 5).uniform(-1, 1)) for i in range(220)]


def icon_starling(t, static):
    W, H = 36, 12
    sc = Scene(W, H)
    if static:
        t = 1.3
    cols = [(150, 90, 220), (70, 190, 200), (120, 220, 110), (150, 90, 220)]
    for th0, f, j in STARLINGS:
        tt = t - f * 0.5
        cx = 36 + 14 * math.sin(tt * 0.55)
        cy = 22 + 6 * math.sin(tt * 0.83 + 1)
        rx = 17 + 6 * math.sin(tt * 0.7)
        ry = 8 + 3 * math.cos(tt * 0.5)
        phi = 0.5 * math.sin(tt * 0.4)
        th = th0 + tt * 0.9 * (1.2 - f)
        lx, ly = rx * f * math.cos(th), ry * f * math.sin(th) + j * 1.5
        x = cx + lx * math.cos(phi) - ly * math.sin(phi)
        y = cy + lx * math.sin(phi) + ly * math.cos(phi)
        c = gradient(cols, ((th0 + t * 0.3) / (2 * math.pi)) % 1)
        sc.dot(x, y, c)
    for x in range(2 * W):
        y = 46 - 1.5 * abs(math.sin(x * 0.2)) - (2 if x % 9 == 0 else 0)
        sc.dot(x, y, (60, 75, 70), 0.1)
    return sc


# --------------------------------------------------------------------------
# hush: crescent moon, twinkling stars, drifting clouds
# --------------------------------------------------------------------------
def icon_hush(t, static):
    W, H = 30, 12
    sc = Scene(W, H)
    if static:
        t = 0.8
    cxp, cyp, r = 12, 11, 8

    def moon(x, y, dx, dy):
        if (x + 0.5 - (cxp + 4)) ** 2 + (y + 0.5 - (cyp - 3)) ** 2 <= (r - 1) ** 2:
            return None
        return gradient([(250, 245, 255), (210, 200, 250), (160, 140, 230)], (dy + r) / (2 * r))

    sc.disk(cxp, cyp, r, moon)
    for i, (x, y, ch, ph) in enumerate(((22, 1, "✦", 0), (27, 4, "·", 1.5), (19, 6, "⋆", 3.1),
                                       (25, 8, "✧", 2.2), (3, 1, "·", 4.0), (28, 0, "⋆", 5))):
        k = 0.3 + 0.7 * (0.5 + 0.5 * math.sin(t * 1.8 + ph))
        sc.put(x, y, ch, dim((230, 220, 255), k))
    for j, (yb, sp, col) in enumerate(((40, 0.8, (90, 85, 140)), (44, 1.3, (120, 110, 170)))):
        off = t * sp * 3
        for x in range(2 * W):
            y = yb - 2.5 * abs(math.sin((x + off) * 0.15)) - 1.5 * abs(math.sin((x + off) * 0.33))
            sc.dot(x, y, col, 0.3 + j)
    return sc


# --------------------------------------------------------------------------
# coo: a carrier pigeon, bobbing its head
# --------------------------------------------------------------------------
PIGEON_COLORS = {
    "g": (155, 163, 178), "d": (115, 122, 140), "b": (70, 75, 92), "n": (80, 190, 140),
    "p": (155, 100, 195), "h": (170, 177, 190), "e": (255, 140, 40), "k": (235, 215, 195),
    "l": (230, 110, 120), "t": (90, 95, 112), "m": (240, 230, 200), "r": (200, 60, 60),
}
PIGEON_HEAD = [
    "..............hhh.....",
    ".............hhhhh....",
    ".............hhehhk...",
    ".............hhhhhkk..",
    "..............hhhh....",
]
PIGEON_BODY = [
    ".............nnnn.....",
    "............nppnn.....",
    "...........nnppng.....",
    ".....ggggggggnnggg....",
    "...dddddgggggggggg....",
    ".tddbbdddggggggggg....",
    "ttdddddbbdggggggg.....",
    "tt.dddddddgggggg......",
    ".....ddddgggggg.......",
    ".........ggggg........",
]
PIGEON_LEGS = [
    ["..........l.l.........", "..........l.l.........", ".........ll.ll........"],
    [".........l...l........", ".........l...l........", "........ll..ll........"],
]


def icon_coo(t, static):
    W, H = 30, 11
    sc = Scene(W, H)
    if static:
        t = 0.0
    ox, oy = 3, 1
    step = int(t * 3.3) % 2
    bob = [0, 2][step]
    for y, row in enumerate(PIGEON_HEAD):
        for x, ch in enumerate(row):
            if ch != ".":
                sc.pixel(ox + x + bob, oy + y, PIGEON_COLORS[ch])
    for y, row in enumerate(PIGEON_BODY):
        for x, ch in enumerate(row):
            if ch != ".":
                sc.pixel(ox + x, oy + 5 + y, PIGEON_COLORS[ch])
    for y, row in enumerate(PIGEON_LEGS[step]):
        for x, ch in enumerate(row):
            if ch != ".":
                sc.pixel(ox + x, oy + 15 + y, PIGEON_COLORS[ch])
    # tiny scroll (the message) tied to the leg
    for x in range(ox + 13, ox + 16):
        sc.pixel(x, oy + 16, PIGEON_COLORS["m"])
    sc.pixel(ox + 13, oy + 16, PIGEON_COLORS["r"])
    off = 0 if static else int(t * 10)
    for x in range(2 * W):
        if (x + off) % 6:
            sc.dot(x, 4 * (oy + 18) // 2 + 1, (110, 100, 90), 0.2)
    return sc


# --------------------------------------------------------------------------
# Shared: an embossed wax seal
# --------------------------------------------------------------------------
def glyph_set(rows, scale=1):
    gw, gh = len(rows[0]) * scale, len(rows) * scale
    s = set()
    for gy, row in enumerate(rows):
        for gx, ch in enumerate(row):
            if ch == "#":
                for a in range(scale):
                    for b in range(scale):
                        s.add((gx * scale + a - gw // 2, gy * scale + b - gh // 2))
    return s


def wax_seal(sc, cx, cy, r, t, gscale=1, sy=1.0, base=(190, 35, 45), shine=True, emblem=None, ecol=None):
    gset = glyph_set(emblem or FONT["S"], gscale)
    ry = r * sy
    pos = (t * 9) % (5 * r) - 2.5 * r
    for y in range(int(cy - ry - 2), int(cy + ry + 3)):
        for x in range(int(cx - r - 2), int(cx + r + 3)):
            dx = x + 0.5 - cx
            dy = (y + 0.5 - cy) / sy
            rr = math.hypot(dx, dy)
            th = math.atan2(dy, dx)
            if rr > r + 0.08 * r * math.sin(5 * th + 1) + 0.04 * r * math.sin(11 * th):
                continue
            diag = (dx + dy) / max(rr, 0.01) / 1.414
            if rr > 0.92 * r:
                c = lerp(bright(base, 0.1), dim(base, 0.55), 0.5 + 0.5 * diag)
            elif rr > 0.78 * r:
                c = lerp(bright(base, 0.4), dim(base, 0.45), 0.5 + 0.5 * diag)
            else:
                c = dim(base, 0.82)
                lx, ly = math.floor(dx), math.floor(dy)
                if (lx, ly) in gset and ecol:
                    c = ecol[0] if (lx - 1, ly - 1) not in gset else ecol[1]
                elif (lx, ly) in gset:
                    c = bright(base, 0.35) if (lx - 1, ly - 1) not in gset else bright(base, 0.1)
                elif (lx - 1, ly - 1) in gset:
                    c = dim(base, 0.5)
            if shine and abs((dx + dy) - pos) < 1.3:
                c = bright(c, 0.35)
            sc.pixel(x, y, c)


# --------------------------------------------------------------------------
# signet variant A: wax seal close-up with ribbon
# --------------------------------------------------------------------------
def icon_signet_seal(t, static, name=None):
    W, H = 34, 13
    sc = Scene(W, H)
    if static:
        t = 1.0
    cx, cy = 17, 11
    for side in (-1, 1):
        for k in range(15):
            x0 = cx + side * (3 + k * 0.65)
            y = cy + 4 + k
            notch = k >= 13
            for w in range(-1, 2):
                if notch and w == 0 and k == 14:
                    continue  # V-notch at the ribbon tail
                c = (225, 195, 120) if w != side else (185, 150, 85)
                sc.pixel(x0 + w, y, dim(c, 1 - 0.02 * k))
    wax_seal(sc, cx, cy, 11, t, gscale=2)
    if not static:
        k = max(0.0, math.sin(t * 2.2))
        if k > 0.6:
            sc.dot(2 * cx - 13, 2 * cy - 15, (255, 240, 240), 5)
    return sc


# --------------------------------------------------------------------------
# signet variant B: ring presses into wax, lifts, leaves the seal
# --------------------------------------------------------------------------
def draw_ring_side(sc, cx, cyc, gold=(235, 190, 80)):
    for y in range(int(cyc - 6), int(cyc + 7)):
        for x in range(cx - 8, cx + 9):
            d = ((x + 0.5 - cx) / 6.5) ** 2 + ((y + 0.5 - cyc) / 4.5) ** 2
            if 0.55 <= d <= 1.0:
                sc.pixel(x, y, lerp(bright(gold, 0.3), dim(gold, 0.55), (x - cx + 8) / 16))
    for y in range(int(cyc + 4), int(cyc + 7)):
        half = 5 if y < cyc + 6 else 4
        for x in range(cx - half, cx + half + 1):
            sc.pixel(x, y, lerp(bright(gold, 0.2), dim(gold, 0.6), (x - cx + half) / (2 * half)))


def icon_signet_stamp(t, static, name=None):
    W, H = 34, 13
    sc = Scene(W, H)
    cx, sy_ = 17, 20
    cyc_rest = 13.5
    p = 3.6 if static else t % 5.0
    for y in range(16, 26):
        for x in range(1, 33):
            if y == 16 or x in (1, 32):
                sc.pixel(x, y, (200, 180, 140))
            else:
                n = ((x * 31 + y * 17) % 7) / 60
                sc.pixel(x, y, dim((238, 222, 186), 0.92 + n))
    if p < 1.4:
        ring_y = -8 + (cyc_rest + 8) * min(1.0, p / 1.2) ** 2
        wax_r = 5
    elif p < 1.9:
        ring_y = cyc_rest
        wax_r = 5 + 4 * min(1.0, (p - 1.4) / 0.25)
    elif p < 3.0:
        ring_y = cyc_rest - (cyc_rest + 10) * ((p - 1.9) / 1.1) ** 1.5
        wax_r = 9
    else:
        ring_y = None if not static else 3.5
        wax_r = 9
    if wax_r < 9:
        for y in range(sy_ - 3, sy_ + 3):
            for x in range(cx - 7, cx + 8):
                dx, dy = (x + 0.5 - cx) / wax_r, (y + 0.5 - sy_) / (wax_r * 0.45)
                if dx * dx + dy * dy <= 1:
                    hl = dx < -0.2 and dy < -0.1
                    sc.pixel(x, y, bright((190, 35, 45), 0.35) if hl else lerp((200, 40, 50), (120, 20, 30), (dx + 1) / 2))
    else:
        wax_seal(sc, cx, sy_, 9, t if p >= 3.0 else 0, gscale=1, sy=0.45, shine=p >= 3.0)
    if 1.35 < p < 2.1:
        k = 1 - abs(p - 1.7) / 0.4
        for a in range(0, 360, 30):
            th = math.radians(a)
            rr = 22 + 6 * (p - 1.35)
            sc.dot(2 * cx + rr * math.cos(th), 2 * sy_ + rr * 0.45 * math.sin(th), dim((255, 230, 180), k), 4)
    if ring_y is not None:
        draw_ring_side(sc, cx, ring_y)
    return sc


# --------------------------------------------------------------------------
# signet variant C: shaded envelope sealed at the flap tip
# --------------------------------------------------------------------------
def icon_signet_envelope(t, static, name=None):
    W, H = 34, 13
    sc = Scene(W, H)
    if static:
        t = 1.0
    x0, x1, y0, y1 = 2, 31, 2, 23
    ax, ay = (x0 + x1) / 2, 14
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            base = lerp((245, 232, 200), (215, 195, 155), (y - y0) / (y1 - y0))
            if x in (x0, x1) or y in (y0, y1):
                c = (175, 155, 115)
            else:
                flap_y = y0 + (ay - y0) * (1 - abs(x + 0.5 - ax) / (ax - x0))
                if y <= flap_y:
                    c = dim(base, 0.93)
                    if y >= flap_y - 1:
                        c = (185, 160, 120)
                else:
                    side = (y1 - y) < (y1 - ay) * (1 - abs(x + 0.5 - ax) / (ax - x0))
                    c = base if side else dim(base, 0.97)
                    if abs((y1 - y) - (y1 - ay) * (1 - abs(x + 0.5 - ax) / (ax - x0))) < 0.8:
                        c = dim(base, 0.88)
                n = ((x * 13 + y * 7) % 5) / 120
                c = dim(c, 0.97 + n)
            sc.pixel(x, y, c)
    for y in range(y1 + 1, y1 + 2):
        for x in range(x0 + 1, x1 + 2):
            sc.pixel(x, y, (60, 50, 45))
    wax_seal(sc, ax, ay, 5.2, t, gscale=1)
    return sc


# --------------------------------------------------------------------------
# signet variant D: a ducal signet ring on Arrakis (Dune homage)
# --------------------------------------------------------------------------
HAWK = [
    "#.............#",
    "##...........##",
    ".###...#...###.",
    "..####.#.####..",
    "...#########...",
    ".....#####.....",
    "......###......",
    ".....#.#.#.....",
]


KEYHOLE = [
    "..###..",
    ".#####.",
    ".#####.",
    "..###..",
    "..###..",
    ".#####.",
    ".#####.",
]
GOLD = (240, 195, 85)
HAWK_RED = ((255, 110, 80), (220, 60, 45))
GREEN_ENAMEL = ((40, 85, 55), (15, 40, 25))


def draw_signet_ring(sc, cx, fy, t, emblem=HAWK, enamel=GREEN_ENAMEL, ecol=HAWK_RED, kx=1.0, gold=GOLD):
    """Front-on signet ring: oval face at (cx, fy), band below. kx = horizontal squash (cos of spin)."""
    ak = max(abs(kx), 0.04)
    pos = (t * 8) % 40 - 20
    gset = glyph_set(emblem)

    def band():
        bcy = fy + 8.5
        for y in range(int(fy), int(fy + 18)):
            for x in range(cx - 10, cx + 11):
                dx, dy = x + 0.5 - cx, y + 0.5 - bcy
                if (dx / (7.5 * ak + 0.6)) ** 2 + (dy / 7.5) ** 2 > 1:
                    continue
                rxi = 7.5 * ak - 1.8
                if rxi > 0 and (dx / rxi) ** 2 + (dy / 5.8) ** 2 < 1:
                    continue
                sc.pixel(x, y, lerp(bright(gold, 0.25), dim(gold, 0.5), 0.5 + dx / 18 + 0.2 * (1 - ak)))

    def face():
        for y in range(int(fy - 9), int(fy + 9)):
            for x in range(cx - 11, cx + 12):
                dx, dy = x + 0.5 - cx, y + 0.5 - fy
                d = (dx / (9.5 * ak + 0.5)) ** 2 + (dy / 8) ** 2
                if d > 1:
                    continue
                if kx < 0:
                    c = lerp(bright(gold, 0.15), dim(gold, 0.45), 0.5 + 0.5 * dy / 8)
                elif d > 0.72:
                    c = lerp(bright(gold, 0.45), dim(gold, 0.5), 0.5 + 0.5 * (dx + dy) / 12)
                else:
                    c = lerp(enamel[0], enamel[1], (dy + 8) / 16)
                    lx, ly = math.floor(dx / ak), math.floor(dy)
                    if (lx, ly) in gset:
                        c = ecol[1] if (lx - 1, ly - 1) in gset else ecol[0]
                if abs((dx + dy) - pos) < 1.2:
                    c = bright(c, 0.3)
                sc.pixel(x, y, c)

    if kx >= 0:
        band()
        face()
    else:
        face()
        band()


def arrakis_sky(sc, t, W):
    starfield(sc, t, 22, 5, (0, 0, 2 * W, 30), (220, 200, 180))
    sc.disk(6, 4, 3, lambda x, y, dx, dy: lerp((245, 235, 215), (190, 170, 140), (dx + dy + 3) / 6))
    sc.disk(11, 2.5, 1.5, lambda x, y, dx, dy: (210, 195, 170))
    for x in range(W):
        top2 = 17 + 2 * math.sin(x * 0.13 + 3)
        for y in range(int(top2), 2 * sc.h):
            sc.pixel(x, y, dim((170, 110, 70), 0.75))


def arrakis_front(sc, t, W):
    for x in range(W):
        top = 21 + 2.2 * math.sin(x * 0.21 + 1.3) + 1.2 * math.sin(x * 0.53)
        for y in range(int(top), 2 * sc.h):
            sc.pixel(x, y, gradient([(235, 170, 95), (200, 120, 60), (140, 75, 40)], (y - top) / 8))


def blowing_sand(sc, t, W, ylo=36, yhi=54, n=40):
    rnd = random.Random(9)
    for i in range(n):
        ph, sp, yb = rnd.random(), rnd.uniform(0.5, 1.2), rnd.uniform(ylo, yhi)
        x = 2 * W - ((t * 18 * sp + ph * 2 * W) % (2 * W))
        y = yb + 2 * math.sin(x * 0.1 + i)
        sc.dot(x, y, dim((250, 200, 140), 0.4 + 0.6 * sp / 1.2), 0.3)


def icon_signet_dune(t, static, name=None):
    W, H = 40, 14
    sc = Scene(W, H)
    if static:
        t = 1.2
    arrakis_sky(sc, t, W)
    arrakis_front(sc, t, W)
    draw_signet_ring(sc, 20, 10, t)
    blowing_sand(sc, t, W)
    return sc


# --------------------------------------------------------------------------
# signet variant E: the ring spinning in space (original keyhole crest)
# --------------------------------------------------------------------------
def icon_signet_spin(t, static, name=None):
    W, H = 36, 14
    sc = Scene(W, H)
    a = 0.5 if static else t * 1.4
    starfield(sc, t, 30, 17, (0, 0, 72, 56), (190, 200, 240))
    draw_signet_ring(sc, 18, 10, t, emblem=KEYHOLE, enamel=((45, 70, 150), (15, 25, 70)),
                     ecol=((255, 230, 140), (205, 160, 60)), kx=math.cos(a))
    k = max(0.0, math.cos(a)) ** 8
    if k > 0.3:
        sparkle(sc, 2 * 18 + 12, 2 * 4, 5 * k, 4 * k, [(255, 255, 255), (255, 230, 150)], 5)
    return sc


# --------------------------------------------------------------------------
# signet variant F: Dune ring with a sandworm breaching behind it
# --------------------------------------------------------------------------
def icon_signet_worm(t, static, name=None):
    W, H = 44, 14
    sc = Scene(W, H)
    if static:
        t = 2.2
    arrakis_sky(sc, t, W)
    p = 0.5 + 0.5 * math.sin(t * 0.7)
    L = 6 + 15 * p
    u = 0.0
    seg = 0
    pts = []
    while u <= L:
        x = 35 + 4 * math.sin(u / 7)
        y = 27 - u * 0.95
        pts.append((x, y, 4.3 - u * 0.05, seg))
        u += 0.5
        seg += 1
    for x, y, r, sg in pts:
        ridge = (sg // 3) % 2 == 0
        sc.disk(x, y, r, lambda px, py, dx, dy, ridge=ridge:
                lerp((190, 140, 95), (110, 70, 45), 0.5 + dx / 8) if not ridge else
                lerp((165, 115, 75), (90, 55, 35), 0.5 + dx / 8))
    hx, hy, hr, _ = pts[-1]
    sc.disk(hx, hy, hr + 0.4, lambda px, py, dx, dy: (235, 215, 180)
            if (math.atan2(dy, dx) * 6 / math.pi) % 1 < 0.5 else (150, 110, 80))
    sc.disk(hx, hy, hr - 1.2, lambda px, py, dx, dy: lerp((90, 35, 30), (30, 10, 10), math.hypot(dx, dy) / hr))
    arrakis_front(sc, t, W)
    draw_signet_ring(sc, 13, 10, t)
    blowing_sand(sc, t, W)
    for i in range(10):
        ph = i * 0.37
        q = (t * 0.9 + ph) % 1
        sc.dot(2 * 35 + 6 * math.sin(i * 2.1) + q * 8 * math.cos(i), 54 - q * 14 - L * 0.4 * p,
               dim((240, 200, 150), 1 - q), 1)
    return sc


# --------------------------------------------------------------------------
# signet variant G: black wax seal, gold hawk, green ribbon (Dune-flavoured)
# --------------------------------------------------------------------------
def icon_signet_blackwax(t, static, name=None):
    W, H = 34, 13
    sc = Scene(W, H)
    if static:
        t = 1.0
    blowing_sand(sc, t, W, 4, 50, 50)
    cx, cy = 17, 11
    for side in (-1, 1):
        for k in range(15):
            x0, y = cx + side * (3 + k * 0.65), cy + 4 + k
            for w in range(-1, 2):
                if k == 14 and w == 0:
                    continue
                c = (45, 110, 70) if w != side else (25, 70, 45)
                sc.pixel(x0 + w, y, dim(c, 1 - 0.02 * k))
    wax_seal(sc, cx, cy, 11, t, base=(70, 60, 70), emblem=HAWK, ecol=((255, 220, 120), (200, 150, 55)))
    return sc


# --------------------------------------------------------------------------
# signet variant H: minimal line-art ring with a travelling glint
# --------------------------------------------------------------------------
def icon_signet_line(t, static, name=None):
    W, H = 26, 12
    sc = Scene(W, H)
    if static:
        t = 0.8
    cx, fy, by = 26, 15, 31

    def col(y):
        return gradient([(255, 225, 130), (240, 170, 70), (220, 110, 60)], y / 48)

    def in_face(x, y):
        return ((x - cx) / 18) ** 2 + ((y - fy) / 13) ** 2 <= 1.05

    for rx, ry in ((15, 15), (11, 11)):
        n = int(rx * 8)
        for i in range(n):
            th = 2 * math.pi * i / n
            x, y = cx + rx * math.cos(th), by + ry * math.sin(th)
            if not in_face(x, y):
                sc.dot(x, y, col(y))
    for rx, ry in ((18, 13), (14, 10)):
        n = int(rx * 8)
        for i in range(n):
            th = 2 * math.pi * i / n
            x, y = cx + rx * math.cos(th), fy + ry * math.sin(th)
            sc.dot(x, y, col(y))
    for (gx, gy) in glyph_set(HAWK):
        sc.dot(cx + gx, fy + gy, (255, 120, 90), 2)
    th = t * 1.6
    for k in range(6):
        a = th - k * 0.06
        x, y = cx + 15 * math.cos(a), by + 15 * math.sin(a)
        if not in_face(x, y):
            sc.dot(x, y, bright(col(y), 1 - k / 6), 3)
    return sc


# --------------------------------------------------------------------------
# tincan: two tin cans on a string, messages travelling along it
# --------------------------------------------------------------------------
def draw_can(sc, x0, y0, facing, label):
    w, h = 7, 9
    for y in range(y0, y0 + h):
        for x in range(x0, x0 + w):
            rel = (y - y0) / (h - 1)
            c = lerp((215, 220, 230), (110, 115, 128), abs(rel - 0.3) * 1.4)
            if 3 <= y - y0 <= 5:
                c = lerp(label, dim(label, 0.6), abs(rel - 0.3) * 1.4)
            if (x - x0) % 3 == 1 and not (3 <= y - y0 <= 5):
                c = dim(c, 0.85)
            sc.pixel(x, y, c)
    ox = x0 + w if facing > 0 else x0 - 1
    for y in range(y0, y0 + h):
        sc.pixel(ox, y, (60, 62, 70) if y0 < y < y0 + h - 1 else (230, 232, 240))
    bx = x0 - 1 if facing > 0 else x0 + w
    for y in range(y0 + 1, y0 + h - 1):
        sc.pixel(bx, y, (235, 238, 245))


def icon_tincan(t, static, name=None):
    W, H = 40, 10
    sc = Scene(W, H)
    if static:
        t = 0.9
    draw_can(sc, 1, 5, 1, (210, 70, 70))
    draw_can(sc, 32, 5, -1, (70, 130, 210))
    xa, xb, ya = 18, 62, 19
    cycle = 4.0
    p = (t % cycle) / (cycle / 2)
    forward = p < 1
    p = p if forward else p - 1
    pos = xa + (xb - xa) * (p if forward else 1 - p)
    col = (255, 130, 160) if forward else (120, 220, 255)
    for x in range(xa, xb + 1):
        u = (x - xa) / (xb - xa)
        sag = 6 * 4 * u * (1 - u)
        env = math.exp(-((x - pos) / 4) ** 2)
        disp = 2.2 * math.sin((x - pos) * 1.3) * env
        c = lerp((200, 185, 150), col, env * 1.5)
        sc.dot(x, ya + sag + disp, c, 1 + env)
    recv = (xb + 6, ya) if forward else (xa - 6, ya)
    if p > 0.85:
        k = (p - 0.85) / 0.15
        for i in range(3):
            rad = 3 + i * 3 + k * 2
            for a in range(-50, 51, 10):
                th = math.radians(a) + (0 if forward else math.pi)
                sc.dot(recv[0] + rad * math.cos(th) * (1 if forward else 1), recv[1] + rad * math.sin(th),
                       dim(col, 1 - i * 0.3), 0.5)
    return sc


# --------------------------------------------------------------------------
# porch: a lantern on a porch at night, fireflies in the yard
# --------------------------------------------------------------------------
def icon_porch(t, static, name=None):
    W, H = 36, 12
    sc = Scene(W, H)
    if static:
        t = 1.7
    starfield(sc, t, 12, 21, (12, 6, 72, 26), (200, 200, 230))
    wood, wood_d = (125, 85, 55), (85, 55, 35)
    for x in range(W):
        sc.pixel(x, 0, wood_d)
        sc.pixel(x, 1, wood)
    for y in range(2, 22):
        sc.pixel(3, y, wood)
        sc.pixel(4, y, wood_d)
    for x in range(3, W):
        sc.pixel(x, 15, wood)
        sc.pixel(x, 21, wood_d)
        if x % 3 == 0:
            for y in range(16, 21):
                sc.pixel(x, y, dim(wood, 0.85))
    for x in range(W):
        sc.pixel(x, 22, dim(wood, 0.7 + 0.1 * (x % 4 == 0)))
        sc.pixel(x, 23, dim(wood_d, 0.8))
    lx = 22
    flick = 0.8 + 0.12 * math.sin(t * 9) + 0.08 * math.sin(t * 23)
    sc.line(2 * lx + 1, 8, 2 * lx + 1, 23, (110, 110, 120), prio=2)
    for x in range(lx - 1, lx + 2):
        sc.pixel(x, 6, (50, 50, 55))
        sc.pixel(x, 11, (50, 50, 55))
        for y in range(7, 11):
            sc.pixel(x, y, dim((255, 215, 120), flick) if x == lx else dim((230, 160, 70), flick))
    for k in range(3):
        rad = 8 + k * 4
        for a in range(0, 360, 12):
            th = math.radians(a + k * 6)
            sc.dot(2 * lx + 1 + rad * math.cos(th), 18 + rad * math.sin(th) * 0.9,
                   dim((255, 190, 90), flick * (0.45 - k * 0.12)), 0.2)
    for i in range(2):
        th = t * (3 + i) + i * 3
        sc.dot(2 * lx + 1 + 6 * math.cos(th), 18 + 4 * math.sin(th * 1.3), (230, 220, 200), 3)
    rnd = random.Random(4)
    for i in range(8):
        ax, ay = rnd.uniform(14, 70), rnd.uniform(30, 58)
        sp, ph = rnd.uniform(0.3, 0.7), rnd.uniform(0, 6.3)
        x = ax + 5 * math.sin(t * sp + ph)
        y = ay + 3 * math.sin(t * sp * 1.4 + ph * 2)
        on = math.sin(t * 1.3 + ph * 3)
        if on > 0.2:
            sc.dot(x, y, dim((210, 255, 120), on), 3)
    return sc


# --------------------------------------------------------------------------
# nook: rain on the window, a warm mug on the sill
# --------------------------------------------------------------------------
def icon_nook(t, static, name=None):
    W, H = 34, 12
    sc = Scene(W, H)
    if static:
        t = 0.7
    fx0, fx1, fy0, fy1 = 3, 30, 0, 19
    rnd = random.Random(8)
    for i in range(26):
        x0 = rnd.uniform(2 * fx0 + 4, 2 * fx1 - 2)
        ph, sp = rnd.random(), rnd.uniform(0.8, 1.3)
        y = 4 * fy0 + 3 + ((t * 28 * sp + ph * 70) % 70)
        x = x0 - (y / 10)
        sc.line(x, y, x - 1, y + 3, (110, 150, 210), (60, 90, 150), prio=0.5)
    for (lx, ly) in ((14, 30), (40, 22), (52, 34), (26, 14)):
        sc.dot(lx, ly, (240, 200, 120), 0.3)
    frame, inner = (150, 100, 60), (190, 135, 80)
    for y in range(fy0, fy1 + 1):
        for x in range(fx0, fx1 + 1):
            edge = x in (fx0, fx0 + 1, fx1, fx1 - 1) or y in (fy0, fy1)
            mull = x in (16, 17) or y == 10
            if edge or mull:
                sc.pixel(x, y, inner if (x in (fx0 + 1, fx1 - 1) or mull) else frame)
    for y in (20, 21):
        for x in range(1, 33):
            sc.pixel(x, y, (170, 115, 70) if y == 20 else (120, 80, 50))
    for y in (22, 23):
        for x in range(W):
            sc.pixel(x, y, dim((90, 60, 45), 0.8 + 0.2 * (y == 22)))
    mug = (90, 175, 165)
    for y in range(15, 20):
        for x in range(21, 26):
            sc.pixel(x, y, mug if y > 15 else (60, 40, 30))
    sc.pixel(26, 16, mug)
    sc.pixel(27, 17, mug)
    sc.pixel(26, 18, mug)
    for s in range(2):
        for k in range(12):
            y = 58 - k * 2 - (t * 6 % 2)
            x = 2 * 23 + 2 * s + 1.5 * math.sin(k * 0.6 + t * 2.5 + s * 2)
            sc.dot(x, y, dim((230, 230, 235), 0.8 - k * 0.06), 3)
    return sc


# --------------------------------------------------------------------------
# cove: a small boat with a lantern in a moonlit bay
# --------------------------------------------------------------------------
def icon_cove(t, static, name=None):
    W, H = 40, 12
    sc = Scene(W, H)
    if static:
        t = 0.4
    starfield(sc, t, 18, 31, (0, 0, 80, 30), (200, 205, 235))
    sc.disk(27, 4, 3, lambda x, y, dx, dy: lerp((250, 250, 235), (200, 200, 190), (dx + dy + 3) / 6))
    for x in range(W):
        lh = 5 + x * 1.5 + 1.2 * math.sin(x * 0.9) if x < 11 else None
        rh = 8 + (W - 1 - x) * 1.3 + 1.0 * math.sin(x * 1.1) if x > 29 else None
        for top in (lh, rh):
            if top is None:
                continue
            for y in range(max(2, int(top)), 21):
                lit = y == max(2, int(top))
                sc.pixel(x, y, (95, 100, 125) if lit else lerp((55, 60, 80), (30, 32, 45), (y - top) / 10))
    for i, row in enumerate(range(10, 12)):
        half = 4 - i
        off = round(math.sin(t * 3 + i))
        for x in range(27 - half + off, 27 + half + off):
            if (x + int(t * 5) + i) % 3:
                sc.put(x, row, "─", dim((235, 235, 220), 0.8 - i * 0.25))
    waves(sc, t, 41, [(35, 85, 160), (55, 120, 200)], amp=1.0, step=3, speed=1.5)
    bob = round(math.sin(t * 2.2))
    bx = 17
    for x in range(bx - 4, bx + 5):
        sc.pixel(x, 18 + bob, (140, 90, 55))
    for x in range(bx - 3, bx + 4):
        sc.pixel(x, 19 + bob, (100, 62, 40))
    for y in range(11, 18):
        sc.pixel(bx, y + bob, (200, 195, 185))
    for y in range(12, 17):
        w = (y - 11)
        for x in range(bx + 1, bx + 1 + w):
            sc.pixel(x, y + bob, (225, 220, 205) if x < bx + w else (190, 185, 175))
    flick = 0.75 + 0.25 * math.sin(t * 8)
    sc.pixel(bx - 3, 17 + bob, dim((255, 200, 90), flick))
    for a in range(0, 360, 30):
        th = math.radians(a)
        sc.dot(2 * (bx - 3) + 4 * math.cos(th), 2 * (17 + bob) + 3 * math.sin(th), dim((255, 190, 90), flick * 0.5), 0.3)
    return sc


# --------------------------------------------------------------------------
# Catalogue
# --------------------------------------------------------------------------
def plain(fn):
    return lambda t, static, name: fn(t, static)


LIGHT = [(255, 235, 170), (235, 90, 80), (120, 150, 230)]
LAMP = [(255, 236, 150), (255, 180, 60), (255, 110, 50)]
FLOCK = [(170, 110, 235), (70, 190, 200), (130, 225, 110)]
WAX = [(240, 215, 170), (215, 80, 80), (150, 30, 45)]

ENTRIES = [
    # Lighthouse family
    dict(name="pharos", group="lighthouse", fn=plain(icon_pharos), stops=LIGHT,
         meaning="The Lighthouse of Alexandria (an ancient Wonder); became the word for lighthouse in several languages. Said FAIR-oss.",
         avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="faro", group="lighthouse", fn=plain(icon_pharos), stops=LIGHT,
         meaning="Italian/Spanish/Portuguese for 'lighthouse' (descends from Pharos). Said FAH-ro.",
         avail="Free on Homebrew, Arch, AUR, Debian. Also a city in Portugal and an old card game."),
    dict(name="keeper", group="lighthouse", fn=plain(icon_pharos), stops=LIGHT,
         meaning="As in lighthouse keeper: tends the light and keeps watch. Plain English.",
         avail="Free on Homebrew, Arch, AUR, Debian. 'Keeper' is also a well-known password manager (Keeper Security)."),
    dict(name="lightkeep", group="lighthouse", fn=plain(icon_pharos), stops=LIGHT,
         meaning="Made-up compound: the job of keeping the light burning. Plain English, easy to say.",
         avail="Free on Homebrew, Arch, AUR, Debian. Long (9 letters)."),
    # Signet variants
    dict(name="signet", group="signet", variant="wax seal close-up", fn=icon_signet_seal, stops=WAX,
         meaning="A signet ring pressed into wax to seal a letter: proves the sender and shows if it was opened.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="signet", group="signet", variant="ring stamping", fn=icon_signet_stamp, stops=WAX,
         meaning="A signet ring pressed into wax to seal a letter: proves the sender and shows if it was opened.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="signet", group="signet", variant="sealed envelope", fn=icon_signet_envelope, stops=WAX,
         meaning="A signet ring pressed into wax to seal a letter: proves the sender and shows if it was opened.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="signet", group="signet", variant="Dune: ducal ring on Arrakis", fn=icon_signet_dune,
         stops=[(250, 215, 120), (235, 150, 70), (200, 70, 50)],
         meaning="Homage to Duke Leto's signet ring (Dune): hawk crest, two moons, drifting sand. Fan nod only; the real crest is Dune IP.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="signet", group="signet", variant="Dune: sandworm breaching", fn=icon_signet_worm,
         stops=[(250, 215, 120), (235, 150, 70), (200, 70, 50)],
         meaning="The ducal ring on Arrakis while a sandworm rises and sinks behind the dunes. Fan nod only (Dune IP).",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="signet", group="signet", variant="Dune: black wax, gold hawk", fn=icon_signet_blackwax,
         stops=[(255, 220, 120), (90, 160, 110), (45, 110, 70)],
         meaning="Wax-seal take on the Dune theme: dark wax, gilded hawk, Atreides-green ribbon, sand blowing past. Fan nod only (Dune IP).",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="signet", group="signet", variant="spinning ring, original crest", fn=icon_signet_spin,
         stops=[(255, 230, 140), (240, 190, 80), (90, 120, 220)],
         meaning="The Dune-style ring rotating in 3D, with an original keyhole crest on blue enamel: no IP concerns.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="signet", group="signet", variant="minimal line art", fn=icon_signet_line,
         stops=[(255, 225, 130), (240, 170, 70), (220, 110, 60)],
         meaning="Outline-only ring in braille dots with a glint running around the band. Closest to a small app icon.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    # Signal-lamp family
    dict(name="qsl", group="signal lamp", fn=icon_morse, stops=LAMP,
         meaning="Radio Q-code for 'I confirm receipt of your message'. Said Q-S-L. The lamp flashes Q-S-L.",
         avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="didah", group="signal lamp", fn=icon_morse, stops=LAMP,
         meaning="How Morse is spoken aloud: 'di-dah' is the letter A. Said dih-DAH.",
         avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="dit", group="signal lamp", fn=icon_morse, stops=LAMP,
         meaning="The short signal in Morse code (a spoken 'dot'). Tiny command to type.",
         avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="blinker", group="signal lamp", fn=icon_morse, stops=LAMP,
         meaning="Navy slang for a signal lamp ('send it by blinker').",
         avail="Free on Homebrew, Arch, AUR. TAKEN on Debian (a Python library)."),
    dict(name="morse", group="signal lamp", fn=icon_morse, stops=LAMP,
         meaning="Morse code.",
         avail="TAKEN on Homebrew and Debian; would need a different package name."),
    # Flock family
    dict(name="skein", group="flock", fn=plain(icon_starling), stops=FLOCK,
         meaning="A flock of geese in flight (also a loop of yarn). Said SKAYN.",
         avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="murmur", group="flock", fn=plain(icon_starling), stops=FLOCK,
         meaning="From 'murmuration' (a swirling flock of starlings); also a soft, low voice.",
         avail="Free on Homebrew, Arch, AUR, Debian. Mumble's voice-chat server used to be called 'murmur'."),
    dict(name="roost", group="flock", fn=plain(icon_starling), stops=FLOCK,
         meaning="Where a flock settles for the night: home, among your own.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="starling", group="flock", fn=plain(icon_starling), stops=FLOCK,
         meaning="Songbird famous for murmurations.",
         avail="Free everywhere, but sounds like Starlink; also a UK bank."),
    # New batch
    dict(name="tincan", group="new", fn=icon_tincan, stops=[(235, 235, 245), (220, 110, 120), (110, 160, 230)],
         meaning="Tin-can telephone: a private line strung between two friends.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    dict(name="porch", group="new", fn=icon_porch, stops=[(255, 215, 120), (220, 140, 70), (150, 200, 90)],
         meaning="Front-porch talk: unhurried conversation with people you know.",
         avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="nook", group="new", fn=icon_nook, stops=[(240, 200, 140), (190, 130, 80), (110, 160, 210)],
         meaning="A cosy, tucked-away corner; somewhere private.",
         avail="Free on Homebrew, Arch, AUR, Debian. Barnes & Noble's 'Nook' e-reader is well known."),
    dict(name="cove", group="new", fn=icon_cove, stops=[(240, 240, 230), (110, 150, 220), (60, 90, 170)],
         meaning="A small sheltered bay: calm water, out of the wind.",
         avail="Free on Homebrew, Arch, Debian. 1 AUR package."),
    # Earlier candidates
    dict(name="helio", group="earlier", fn=plain(icon_helio), stops=[(255, 220, 80), (255, 130, 50), (230, 60, 120)],
         meaning="Short for heliograph: a mirror that flashed sunlight in Morse code.",
         avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="lumen", group="earlier", fn=plain(icon_lumen), stops=[(255, 245, 190), (255, 190, 80), (240, 120, 60)],
         meaning="Latin for 'light'; the unit of how much light a source emits.",
         avail="1 AUR package; Laravel 'Lumen' (PHP) is well known."),
    dict(name="cairn", group="earlier", fn=plain(icon_cairn), stops=[(190, 195, 205), (150, 160, 175), (120, 160, 110)],
         meaning="A stack of stones marking a trail. Said like 'care-n'.",
         avail="1 AUR package."),
    dict(name="glint", group="earlier", fn=plain(icon_glint), stops=[(255, 255, 255), (170, 220, 255), (140, 110, 240)],
         meaning="A brief flash of reflected light.", avail="1 AUR package."),
    dict(name="wisp", group="earlier", fn=plain(icon_wisp), stops=[(200, 255, 245), (80, 220, 200), (40, 140, 130)],
         meaning="Will-o'-the-wisp: a ghostly light from folklore.", avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="sotto", group="earlier", fn=plain(icon_sotto), stops=[(190, 170, 250), (130, 150, 240), (100, 190, 240)],
         meaning="From 'sotto voce' ('under the voice'): speaking quietly.", avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="tightbeam", group="earlier", fn=plain(icon_tightbeam), stops=[(255, 120, 180), (200, 110, 230), (120, 220, 255)],
         meaning="Sci-fi: a narrow, hard-to-intercept laser link between two ships.", avail="Free on Homebrew, Arch, AUR, Debian."),
    dict(name="hush", group="earlier", fn=plain(icon_hush), stops=[(240, 235, 255), (180, 165, 245), (120, 110, 200)],
         meaning="Quiet, private.", avail="1 AUR package."),
    dict(name="coo", group="earlier", fn=plain(icon_coo), stops=[(180, 188, 200), (90, 190, 145), (160, 105, 200)],
         meaning="The sound a pigeon makes: carrier pigeons.", avail="1 AUR package. Reads like 'COO' (exec title)."),
]
TAGLINE = "unofficial Signal client for the terminal"
GREY = (130, 130, 145)


def compose(e, t, static, termw):
    icon = e["fn"](t, static, e["name"])
    ilines = icon.render()
    for scale in (2, 1):
        word = wordmark(e["name"], e["stops"], scale)
        rw = max(word.w, len(TAGLINE))
        if icon.w + 4 + rw <= termw:
            break
    right = [l + " " * (rw - word.w) for l in word.render()] + \
            [" " * rw, fg(GREY) + TAGLINE + RESET + " " * (rw - len(TAGLINE))]
    if icon.w + 4 + rw <= termw:
        h = max(len(ilines), len(right))
        ip, rp = (h - len(ilines)) // 2, (h - len(right)) // 2
        Li = [" " * icon.w] * ip + ilines + [" " * icon.w] * (h - ip - len(ilines))
        Rr = [" " * rw] * rp + right + [" " * rw] * (h - rp - len(right))
        return ["  " + a + "    " + b for a, b in zip(Li, Rr)]
    return ["  " + l for l in ilines] + [""] + ["  " + l for l in right]


def header(e):
    members = [x for x in ENTRIES if x["group"] == e["group"]]
    pos = members.index(e) + 1
    tag = f"[{e['group']} {pos}/{len(members)}]"
    variant = f"  {fg((235, 180, 120))}variant: {e['variant']}{RESET}" if e.get("variant") else ""
    return [f"  {fg((110, 110, 125))}{tag}{RESET}  {fg((240, 240, 250))}\x1b[1m{e['name']}{RESET}{variant}",
            f"  {fg((175, 175, 190))}{e['meaning']}{RESET}",
            f"  {fg((110, 110, 125))}{e['avail']}{RESET}"]


def static_gallery(entries):
    termw = shutil.get_terminal_size((120, 40)).columns
    for e in entries:
        print()
        print("\n".join(header(e)))
        print()
        print("\n".join(compose(e, 0, True, termw)))
    print()


def run(entries, interactive, start_idx=0):
    out = sys.stdout
    fd = sys.stdin.fileno()
    old = None
    if interactive:
        import termios
        import tty
        old = termios.tcgetattr(fd)
        tty.setcbreak(fd)
    out.write("\x1b[?1049h\x1b[?25l\x1b[2J")
    idx, auto = start_idx, not interactive
    t0 = time.time()
    last_switch = t0
    groups = []
    for e in entries:
        if e["group"] not in groups:
            groups.append(e["group"])
    try:
        while True:
            now = time.time()
            t = now - t0
            if auto and now - last_switch >= 8:
                idx, last_switch = (idx + 1) % len(entries), now
                out.write("\x1b[2J")
            if interactive:
                r, _, _ = select.select([fd], [], [], 0)
                if r:
                    buf = os.read(fd, 64)
                    keys, i = [], 0
                    while i < len(buf):
                        if buf[i:i + 2] == b"\x1b[" and i + 2 < len(buf):
                            keys.append(buf[i:i + 3])
                            i += 3
                        else:
                            keys.append(buf[i:i + 1])
                            i += 1
                    new, quit_ = idx, False
                    for key in keys:
                        if key in (b"q", b"Q", b"\x1b", b"\x03"):
                            quit_ = True
                        elif key in (b"\x1b[C", b"l", b"n", b" "):
                            new = (new + 1) % len(entries)
                        elif key in (b"\x1b[D", b"h", b"p"):
                            new = (new - 1) % len(entries)
                        elif key in (b"\x1b[B", b"j", b"\x1b[A", b"k"):
                            g = groups.index(entries[new]["group"])
                            g = (g + (1 if key in (b"\x1b[B", b"j") else -1)) % len(groups)
                            new = next(i for i, e in enumerate(entries) if e["group"] == groups[g])
                        elif key == b"a":
                            auto, last_switch = not auto, now
                    if quit_:
                        break
                    if new != idx:
                        idx, last_switch = new, now
                        out.write("\x1b[2J")
            e = entries[idx]
            termw = shutil.get_terminal_size((120, 40)).columns
            frame = [""] + header(e) + [""] + compose(e, t, False, termw)
            if interactive:
                hint = (f"←/→ h/l: prev/next · ↑/↓ k/j: prev/next group · "
                        f"a: auto-cycle {'ON' if auto else 'off'} · q: quit")
            else:
                hint = "ctrl+c: exit"
            frame += ["", f"  {fg((100, 100, 115))}{idx + 1}/{len(entries)} · {hint}{RESET}"]
            out.write("\x1b[H" + "\n".join(l + "\x1b[K" for l in frame) + "\x1b[J")
            out.flush()
            time.sleep(0.05)
    except KeyboardInterrupt:
        pass
    finally:
        out.write(RESET + "\x1b[?25h\x1b[?1049l")
        out.flush()
        if old is not None:
            import termios
            termios.tcsetattr(fd, termios.TCSADRAIN, old)


def select_entries(filters):
    if not filters:
        return ENTRIES
    picked = [e for e in ENTRIES if e["name"] in filters or e["group"] in filters]
    if not picked:
        names = sorted({e["name"] for e in ENTRIES})
        groups = sorted({e["group"] for e in ENTRIES})
        sys.exit(f"nothing matches {filters}.\nnames: {', '.join(names)}\ngroups: {', '.join(groups)}")
    return picked


if __name__ == "__main__":
    args = sys.argv[1:]
    mode = args[0] if args and args[0] in ("anim", "browse", "static") else "static"
    rest = args[1:] if args and args[0] in ("anim", "browse", "static") else args
    if mode == "anim":
        run(select_entries(rest), interactive=False)
    elif mode == "browse":
        entries = select_entries([r for r in rest if r in {e["group"] for e in ENTRIES}])
        start = next((i for i, e in enumerate(entries) if e["name"] in rest), 0)
        run(entries, interactive=True, start_idx=start)
    else:
        static_gallery(select_entries(rest))
