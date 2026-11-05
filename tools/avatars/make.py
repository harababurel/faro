#!/usr/bin/env python3
"""Stand-in profile pictures for the avatar spike (PLAN.md "Profile pictures").

Writes N distinct 256x256 PNGs (soft gradient backgrounds, a head-and-shoulders
silhouette, some grain so they compress like photos) to DIR, for
`FARO_SPIKE_AVATARS=DIR faro demo`. Python 3, no dependencies.

    python3 tools/avatars/make.py /tmp/faro-avatars [N]
"""

import colorsys
import math
import os
import random
import struct
import sys
import zlib

SIZE = 256


def png(path, pixels):
    """Write `pixels` (rows of (r, g, b) tuples) as an 8-bit RGB PNG."""
    raw = b"".join(b"\x00" + bytes(c for px in row for c in px) for row in pixels)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    with open(path, "wb") as out:
        out.write(b"\x89PNG\r\n\x1a\n")
        out.write(chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 2, 0, 0, 0)))
        out.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        out.write(chunk(b"IEND", b""))


def rgb(h, s, v):
    return tuple(int(c * 255) for c in colorsys.hsv_to_rgb(h % 1.0, s, v))


def picture(n, total):
    rnd = random.Random(n)
    hue = n / total
    top = rgb(hue, 0.55, 0.95)
    bottom = rgb(hue + 0.12, 0.75, 0.55)
    figure = rgb(hue + 0.5, rnd.uniform(0.25, 0.6), rnd.uniform(0.2, 0.45))
    head_y = rnd.uniform(0.36, 0.44) * SIZE
    head_r = rnd.uniform(0.15, 0.2) * SIZE
    body_y = head_y + head_r * 2.4
    body_rx = rnd.uniform(0.32, 0.42) * SIZE
    body_ry = rnd.uniform(0.25, 0.32) * SIZE
    cx = SIZE / 2 + rnd.uniform(-0.06, 0.06) * SIZE
    rows = []
    for y in range(SIZE):
        t = y / (SIZE - 1)
        bg = tuple(int(a + (b - a) * t) for a, b in zip(top, bottom))
        row = []
        for x in range(SIZE):
            in_head = math.hypot(x - cx, y - head_y) < head_r
            in_body = ((x - cx) / body_rx) ** 2 + ((y - body_y) / body_ry) ** 2 < 1
            base = figure if in_head or in_body else bg
            grain = rnd.randint(-10, 10)
            row.append(tuple(max(0, min(255, c + grain)) for c in base))
        rows.append(row)
    return rows


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    out = sys.argv[1]
    total = int(sys.argv[2]) if len(sys.argv) > 2 else 20
    os.makedirs(out, exist_ok=True)
    for n in range(total):
        png(os.path.join(out, f"avatar-{n:02}.png"), picture(n, total))
    print(f"{total} pictures in {out}")


if __name__ == "__main__":
    main()
