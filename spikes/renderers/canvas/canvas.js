// M13 Phase 0 spike (throwaway): a Canvas 2D cell renderer for faro frames.
//
// Paints backgrounds, then glyphs. Block elements, octants, sextants, the
// Unicode 16 quarter circles, Powerline caps and box drawing are drawn as
// shapes, so they are exact whatever the font; everything else is text from the
// webfont, and emoji are drawn centred in exactly the cells faro gave them.
// The real renderer would be this logic behind a Rust ratatui `Backend`.

// ---- octants (Unicode 16) and older glyphs that are octant patterns --------
// bit i = octant i+1: column i % 2, row i >> 1 (4 rows, top to bottom).
const OCTANT = new Map();
const OLDER = [
  [0x2580, 0b00001111], [0x2584, 0b11110000], [0x258c, 0b01010101], [0x2590, 0b10101010],
  [0x2598, 0b00000101], [0x259d, 0b00001010], [0x2596, 0b01010000], [0x2597, 0b10100000],
  [0x259a, 0b10100101], [0x259e, 0b01011010], [0x2599, 0b11110101], [0x259b, 0b01011111],
  [0x259c, 0b10101111], [0x259f, 0b11111010], [0x1fb82, 0b00000011], [0x2582, 0b11000000],
  [0x2586, 0b11111100], [0x1fb85, 0b00111111], [0x1cea8, 0b00000001], [0x1ceab, 0b00000010],
  [0x1cea3, 0b01000000], [0x1cea0, 0b10000000], [0x1fbe6, 0b00010100], [0x1fbe7, 0b00101000],
];
{
  const older = new Set(OLDER.map(([, m]) => m));
  for (const [cp, m] of OLDER) OCTANT.set(cp, m);
  let cp = 0x1cd00;
  for (let m = 1; m < 255; m++) if (!older.has(m)) OCTANT.set(cp++, m);
  OCTANT.set(0x2588, 255);
}
// Sextants U+1FB00..1FB3B: masks 1..62 minus the two half blocks (21, 42).
const SEXTANT = new Map();
{
  let cp = 0x1fb00;
  for (let m = 1; m < 63; m++) if (m !== 21 && m !== 42) SEXTANT.set(cp++, m);
}

function blend(a, b, t) {
  const p = (c) => [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16));
  const [x, y] = [p(a), p(b)];
  return "#" + x.map((v, i) => Math.round(v * (1 - t) + y[i] * t).toString(16).padStart(2, "0")).join("");
}

export class CanvasRenderer {
  constructor(canvas, { font = "JetBrains Mono", size = 14, lineHeight = 1.2 } = {}) {
    this.canvas = canvas;
    this.font = font;
    this.size = size;
    this.lineHeight = lineHeight;
    this.ctx = canvas.getContext("2d", { alpha: false });
  }

  metrics() {
    const dpr = window.devicePixelRatio || 1;
    const px = this.size * dpr;
    const ctx = this.ctx;
    ctx.font = `${px}px "${this.font}"`;
    const m = ctx.measureText("M");
    const W = Math.round(m.width);
    const H = Math.round(px * this.lineHeight);
    const asc = m.fontBoundingBoxAscent, desc = m.fontBoundingBoxDescent;
    const base = Math.round((H - (asc + desc)) / 2 + asc);
    return { dpr, px, W, H, base };
  }

  draw(frame) {
    const t0 = performance.now();
    const { dpr, px, W, H, base } = (this.m ??= this.metrics());
    const { w, h, cells, theme } = frame;
    const c = this.canvas;
    if (c.width !== w * W || c.height !== h * H) {
      c.width = w * W;
      c.height = h * H;
      c.style.width = `${(w * W) / dpr}px`;
      c.style.height = `${(h * H) / dpr}px`;
    }
    const ctx = this.ctx;
    const dfg = theme.fg ?? "#e6e6e6", dbg = theme.bg ?? "#101216";
    const style = (cell) => {
      let fg = cell[2] ?? dfg, bg = cell[3] ?? dbg;
      const f = cell[4];
      if (f & 16) [fg, bg] = [bg, fg];
      if (f & 2) fg = blend(fg, bg, 0.45);
      return [fg, bg, f];
    };
    // Backgrounds, merged into runs per row. A wide glyph's background covers
    // every cell it spans (the cells it hides keep a default background).
    for (let y = 0; y < h; y++) {
      const row = [];
      for (let x = 0; x < w; ) {
        const cell = cells[y * w + x];
        const bg = style(cell)[1];
        const cw = Math.min(Math.max(cell[1], 1), w - x);
        for (let k = 0; k < cw; k++) row.push(bg);
        x += cw;
      }
      let x = 0;
      while (x < w) {
        const bg = row[x];
        let e = x + 1;
        while (e < w && row[e] === bg) e++;
        ctx.fillStyle = bg;
        ctx.fillRect(x * W, y * H, (e - x) * W, H);
        x = e;
      }
    }
    // Glyphs.
    ctx.textBaseline = "alphabetic";
    let lastFont = "";
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; ) {
        const cell = cells[y * w + x];
        const sym = cell[0];
        const cw = Math.max(cell[1], 1);
        const [fg, , f] = style(cell);
        const x0 = x * W, y0 = y * H;
        x += cw;
        if (sym === " " || sym === "" || f & 64) continue;
        ctx.fillStyle = fg;
        ctx.strokeStyle = fg;
        if (this.shape(sym, x0, y0, W, H, dpr)) continue;
        const font = `${f & 4 ? "italic " : ""}${f & 1 ? "bold " : ""}${px}px "${this.font}", "Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji", monospace`;
        if (font !== lastFont) { ctx.font = font; lastFont = font; }
        if (cw > 1 || sym.codePointAt(0) > 0x2fff) {
          // Emoji / wide: centred in exactly `cw` cells, shrunk if wider.
          const mw = ctx.measureText(sym).width;
          const room = cw * W;
          if (mw > room) {
            ctx.save();
            ctx.translate(x0 + room / 2, y0 + base);
            ctx.scale(room / mw, room / mw);
            ctx.fillText(sym, -mw / 2, 0);
            ctx.restore();
          } else {
            ctx.fillText(sym, x0 + (room - mw) / 2, y0 + base);
          }
        } else {
          ctx.fillText(sym, x0, y0 + base);
        }
        if (f & 8) ctx.fillRect(x0, y0 + base + Math.round(2 * dpr), cw * W, Math.max(1, Math.round(dpr)));
        if (f & 32) ctx.fillRect(x0, y0 + Math.round(H / 2), cw * W, Math.max(1, Math.round(dpr)));
      }
    }
    return performance.now() - t0;
  }

  /// Draw `sym` as a shape if it is one; false to fall back to text.
  shape(sym, x0, y0, W, H, dpr) {
    if (sym.length > 2) return false;
    const cp = sym.codePointAt(0);
    const ctx = this.ctx;
    const X = (f) => x0 + Math.round(W * f), Y = (f) => y0 + Math.round(H * f);
    const rect = (fx0, fy0, fx1, fy1) => ctx.fillRect(X(fx0), Y(fy0), X(fx1) - X(fx0), Y(fy1) - Y(fy0));
    const oct = OCTANT.get(cp);
    if (oct !== undefined) {
      for (let i = 0; i < 8; i++) if (oct & (1 << i)) rect((i % 2) / 2, (i >> 1) / 4, (i % 2) / 2 + 0.5, (i >> 1) / 4 + 0.25);
      return true;
    }
    const sx = SEXTANT.get(cp);
    if (sx !== undefined) {
      for (let i = 0; i < 6; i++) if (sx & (1 << i)) rect((i % 2) / 2, Math.floor(i / 2) / 3, (i % 2) / 2 + 0.5, Math.floor(i / 2) / 3 + 1 / 3);
      return true;
    }
    if (cp >= 0x2581 && cp <= 0x2587) { rect(0, 1 - (cp - 0x2580) / 8, 1, 1); return true; } // lower eighths
    if (cp >= 0x2589 && cp <= 0x258f) { rect(0, 0, (0x2590 - cp) / 8, 1); return true; } // left eighths
    if (cp === 0x2594) { rect(0, 0, 1, 1 / 8); return true; }
    if (cp === 0x2595) { rect(7 / 8, 0, 1, 1); return true; }
    if (cp >= 0x2591 && cp <= 0x2593) {
      ctx.globalAlpha = (cp - 0x2590) / 4; rect(0, 0, 1, 1); ctx.globalAlpha = 1; return true;
    }
    if (cp >= 0x1fb82 && cp <= 0x1fb86) { rect(0, 0, 1, [2, 3, 5, 6, 7][cp - 0x1fb82] / 8); return true; } // upper eighths
    if (cp >= 0x1fb87 && cp <= 0x1fb8b) { rect(1 - [2, 3, 5, 6, 7][cp - 0x1fb87] / 8, 0, 1, 1); return true; } // right eighths
    // Unicode 16 quarter circles: radius half a cell wide, centred on a corner.
    const quarter = { 0x1fbec: [1, 0], 0x1fbed: [0, 1], 0x1fbee: [1, 1], 0x1fbef: [0, 0] }[cp];
    if (quarter) {
      const cx = x0 + quarter[0] * W, cy = y0 + quarter[1] * H, r = W / 2;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.ellipse(cx, cy, r, r, 0, 0, Math.PI * 2);
      ctx.save();
      ctx.beginPath();
      ctx.rect(x0, y0, W, H);
      ctx.clip();
      ctx.beginPath();
      ctx.ellipse(cx, cy, r, r, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
      return true;
    }
    // Powerline: solid triangles and rounded caps.
    if (cp === 0xe0b0 || cp === 0xe0b2 || cp === 0xe0b4 || cp === 0xe0b6) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(x0, y0, W, H);
      ctx.clip();
      ctx.beginPath();
      if (cp === 0xe0b0) { ctx.moveTo(x0, y0); ctx.lineTo(x0 + W, y0 + H / 2); ctx.lineTo(x0, y0 + H); }
      if (cp === 0xe0b2) { ctx.moveTo(x0 + W, y0); ctx.lineTo(x0, y0 + H / 2); ctx.lineTo(x0 + W, y0 + H); }
      if (cp === 0xe0b4) ctx.ellipse(x0, y0 + H / 2, W, H / 2, 0, -Math.PI / 2, Math.PI / 2);
      if (cp === 0xe0b6) ctx.ellipse(x0 + W, y0 + H / 2, W, H / 2, 0, Math.PI / 2, (3 * Math.PI) / 2);
      ctx.fill();
      ctx.restore();
      return true;
    }
    return this.box(cp, x0, y0, W, H, dpr);
  }

  /// Light/heavy/double box drawing and rounded corners as lines.
  box(cp, x0, y0, W, H, dpr) {
    // [up, right, down, left]: 0 none, 1 light, 2 heavy, 3 double
    const B = {
      0x2500: [0, 1, 0, 1], 0x2502: [1, 0, 1, 0], 0x2501: [0, 2, 0, 2], 0x2503: [2, 0, 2, 0],
      0x250c: [0, 1, 1, 0], 0x2510: [0, 0, 1, 1], 0x2514: [1, 1, 0, 0], 0x2518: [1, 0, 0, 1],
      0x251c: [1, 1, 1, 0], 0x2524: [1, 0, 1, 1], 0x252c: [0, 1, 1, 1], 0x2534: [1, 1, 0, 1],
      0x253c: [1, 1, 1, 1], 0x250f: [0, 2, 2, 0], 0x2513: [0, 0, 2, 2], 0x2517: [2, 2, 0, 0],
      0x251b: [2, 0, 0, 2], 0x2550: [0, 3, 0, 3], 0x2551: [3, 0, 3, 0], 0x2574: [0, 0, 0, 1],
      0x2576: [0, 1, 0, 0], 0x2575: [1, 0, 0, 0], 0x2577: [0, 0, 1, 0],
    }[cp];
    const ctx = this.ctx;
    const t = Math.max(1, Math.round(dpr));
    const cx = x0 + Math.floor((W - t) / 2), cy = y0 + Math.floor((H - t) / 2);
    const round = { 0x256d: [1, 1], 0x256e: [-1, 1], 0x2570: [1, -1], 0x256f: [-1, -1] }[cp];
    if (round) {
      // Arc from the middle of one edge to the middle of the other.
      const [dx, dy] = round;
      const r = Math.min(W, H) / 2;
      const mx = cx + t / 2, my = cy + t / 2;
      ctx.lineWidth = t;
      ctx.beginPath();
      const ex = dx > 0 ? x0 + W : x0, ey = dy > 0 ? y0 + H : y0;
      ctx.moveTo(ex, my);
      ctx.lineTo(mx + dx * r, my);
      ctx.arcTo(mx, my, mx, my + dy * r, r);
      ctx.lineTo(mx, ey);
      ctx.stroke();
      return true;
    }
    if (!B) return false;
    const seg = (kind, x, y, w, h) => {
      if (!kind) return;
      if (kind === 3) {
        const g = t * 2;
        if (w > h) { ctx.fillRect(x, y - g, w, t); ctx.fillRect(x, y + g, w, t); }
        else { ctx.fillRect(x - g, y, t, h); ctx.fillRect(x + g, y, t, h); }
        return;
      }
      const k = kind === 2 ? t * 2 : t;
      if (w > h) ctx.fillRect(x, y - (k - t) / 2, w, k);
      else ctx.fillRect(x - (k - t) / 2, y, k, h);
    };
    const [u, r, d, l] = B;
    seg(u, cx, y0, t, cy - y0 + t);
    seg(d, cx, cy, t, y0 + H - cy);
    seg(l, x0, cy, cx - x0 + t, t);
    seg(r, cx, cy, x0 + W - cx, t);
    return true;
  }
}
