//! Paints buffer rows onto a `<canvas>` with the Canvas 2D API (D106): cell
//! backgrounds first, merged into runs (a wide glyph's background covers every
//! cell it spans), then glyphs: shapes from [`glyphs`](crate::glyphs), text
//! from the webfont (D107), emoji from the system's colour font centred in
//! exactly the cells faro gave them. Works in device pixels, so it's crisp at
//! any device pixel ratio.

use std::f64::consts::{FRAC_PI_2, TAU};
use std::fmt::Write as _;

use ratatui::buffer::{Buffer, Cell, CellDiffOption};
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr as _;
use wasm_bindgen::{JsCast as _, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::glyphs::{self, Corner, Line, Powerline, Shape, Symbol};

/// Line height as a multiple of the font size (as the Phase 0 prototype).
const LINE_HEIGHT: f64 = 1.2;
/// How far `DIM` text fades towards its background.
const DIM: f64 = 0.45;
/// Fallback fonts for what the webfont lacks: emoji, then anything.
const FALLBACK: &str = r#""Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji", monospace"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn blend(self, to: Self, t: f64) -> Self {
        let mix = |a: u8, b: u8| {
            let v = f64::from(a).mul_add(1.0 - t, f64::from(b) * t);
            u8::try_from(clamp_u32(v, 255)).unwrap_or(u8::MAX)
        };
        Self(mix(self.0, to.0), mix(self.1, to.1), mix(self.2, to.2))
    }

    const fn key(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }
}

/// The 16 ANSI colours (xterm's defaults), for themes that use them (`system`).
const ANSI: [Rgb; 16] = [
    Rgb(0x00, 0x00, 0x00),
    Rgb(0xcd, 0x00, 0x00),
    Rgb(0x00, 0xcd, 0x00),
    Rgb(0xcd, 0xcd, 0x00),
    Rgb(0x00, 0x00, 0xee),
    Rgb(0xcd, 0x00, 0xcd),
    Rgb(0x00, 0xcd, 0xcd),
    Rgb(0xe5, 0xe5, 0xe5),
    Rgb(0x7f, 0x7f, 0x7f),
    Rgb(0xff, 0x00, 0x00),
    Rgb(0x00, 0xff, 0x00),
    Rgb(0xff, 0xff, 0x00),
    Rgb(0x5c, 0x5c, 0xff),
    Rgb(0xff, 0x00, 0xff),
    Rgb(0x00, 0xff, 0xff),
    Rgb(0xff, 0xff, 0xff),
];

fn ansi(index: usize) -> Rgb {
    ANSI.get(index).copied().unwrap_or(Rgb(0x80, 0x80, 0x80))
}

/// A cell colour as RGB; `Reset` is the theme's default.
fn rgb(color: Color, default: Rgb) -> Rgb {
    match color {
        Color::Reset => default,
        Color::Rgb(r, g, b) => Rgb(r, g, b),
        Color::Indexed(i) => match faro_app::theme::indexed_rgb(i) {
            Some((r, g, b)) => Rgb(r, g, b),
            None => ansi(usize::from(i)),
        },
        Color::Black => ansi(0),
        Color::Red => ansi(1),
        Color::Green => ansi(2),
        Color::Yellow => ansi(3),
        Color::Blue => ansi(4),
        Color::Magenta => ansi(5),
        Color::Cyan => ansi(6),
        Color::Gray => ansi(7),
        Color::DarkGray => ansi(8),
        Color::LightRed => ansi(9),
        Color::LightGreen => ansi(10),
        Color::LightYellow => ansi(11),
        Color::LightBlue => ansi(12),
        Color::LightMagenta => ansi(13),
        Color::LightCyan => ansi(14),
        Color::White => ansi(15),
    }
}

/// How many cells a cell's symbol covers: faro's forced width, else Unicode's.
pub fn cell_width(cell: &Cell) -> u16 {
    let width = match cell.diff_option {
        CellDiffOption::ForcedWidth(w) => usize::from(w.get()),
        CellDiffOption::None | CellDiffOption::Skip | CellDiffOption::AlwaysUpdate => {
            cell.symbol().width()
        }
    };
    u16::try_from(width.max(1)).unwrap_or(1)
}

/// Cell metrics in device pixels.
#[derive(Debug, Clone, Copy)]
struct Metrics {
    w: f64,
    h: f64,
    /// Baseline, from the top of the cell.
    base: f64,
}

pub struct Painter {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    dpr: f64,
    metrics: Metrics,
    /// `[regular, bold, italic, bold italic]` font strings.
    fonts: [String; 4],
    /// Default foreground and background (the theme's), for `Color::Reset`.
    fg: Rgb,
    bg: Rgb,
    /// What the context has now, to skip redundant state changes.
    fill: Option<u32>,
    font: Option<usize>,
    css: String,
}

impl Painter {
    /// Paint into `canvas` with `family` at `size` CSS pixels. The font must
    /// already be loaded (`document.fonts.load`), or metrics come from a
    /// fallback.
    pub fn new(canvas: HtmlCanvasElement, family: &str, size: f64) -> Result<Self, JsValue> {
        let options = js_sys::Object::new();
        js_sys::Reflect::set(&options, &"alpha".into(), &JsValue::FALSE)?;
        let ctx = canvas
            .get_context_with_context_options("2d", &options)?
            .ok_or_else(|| JsValue::from_str("no 2D canvas context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        let dpr = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
        let px = (size * dpr).round();
        let fonts = [
            format!(r#"{px}px "{family}", {FALLBACK}"#),
            format!(r#"bold {px}px "{family}", {FALLBACK}"#),
            format!(r#"italic {px}px "{family}", {FALLBACK}"#),
            format!(r#"italic bold {px}px "{family}", {FALLBACK}"#),
        ];
        ctx.set_font(&format!(r#"{px}px "{family}""#));
        let m = ctx.measure_text("M")?;
        let h = (px * LINE_HEIGHT).round();
        let (ascent, descent) = (m.font_bounding_box_ascent(), m.font_bounding_box_descent());
        let metrics = Metrics {
            w: m.width().round().max(1.0),
            h,
            base: ((h - (ascent + descent)) / 2.0 + ascent).round(),
        };
        ctx.set_text_baseline("alphabetic");
        Ok(Self {
            canvas,
            ctx,
            dpr,
            metrics,
            fonts,
            fg: Rgb(0xe6, 0xe6, 0xe6),
            bg: Rgb(0x10, 0x12, 0x16),
            fill: None,
            font: None,
            css: String::new(),
        })
    }

    /// One cell in CSS pixels (for grid sizing and the backend's window size).
    pub fn cell_css(&self) -> (f64, f64) {
        (self.metrics.w / self.dpr, self.metrics.h / self.dpr)
    }

    /// The cell size rounded, for ratatui.
    pub fn cell_size(&self) -> Size {
        let (w, h) = self.cell_css();
        Size::new(round_u16(w), round_u16(h))
    }

    /// How many cells fit in `width × height` CSS pixels.
    pub fn grid_for(&self, width: f64, height: f64) -> (u16, u16) {
        let (w, h) = self.cell_css();
        (floor_u16(width / w), floor_u16(height / h))
    }

    /// The canvas's parent's content box in CSS pixels (`0 × 0` without one).
    pub fn parent_size(&self) -> (f64, f64) {
        self.canvas.parent_element().map_or((0.0, 0.0), |p| {
            (f64::from(p.client_width()), f64::from(p.client_height()))
        })
    }

    /// Theme defaults for `Color::Reset`; whether they changed (every row
    /// must then be painted again).
    pub fn set_defaults(&mut self, fg: Color, bg: Color) -> bool {
        let (fg, bg) = (rgb(fg, self.fg), rgb(bg, self.bg));
        let changed = (fg, bg) != (self.fg, self.bg);
        self.fg = fg;
        self.bg = bg;
        changed
    }

    /// The device pixel ratio the font was measured at.
    pub const fn ratio(&self) -> f64 {
        self.dpr
    }

    pub const fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }

    /// Size the canvas for a `columns × rows` grid (clears it).
    pub fn resize(&mut self, columns: u16, rows: u16) {
        let Metrics { w, h, .. } = self.metrics;
        let (width, height) = (f64::from(columns) * w, f64::from(rows) * h);
        self.canvas.set_width(to_u32(width));
        self.canvas.set_height(to_u32(height));
        let style = self.canvas.style();
        let _ = style.set_property("width", &format!("{}px", width / self.dpr));
        let _ = style.set_property("height", &format!("{}px", height / self.dpr));
        // Resizing resets the context's state.
        self.fill = None;
        self.font = None;
        self.ctx.set_text_baseline("alphabetic");
    }

    /// Repaint `rows` of `buffer`, with the cursor if it's on one of them.
    pub fn paint(&mut self, buffer: &Buffer, rows: &[u16], cursor: Option<Position>) {
        for &y in rows {
            self.paint_row(buffer, y, cursor.filter(|c| c.y == y).map(|c| c.x));
        }
    }

    fn paint_row(&mut self, buffer: &Buffer, y: u16, cursor: Option<u16>) {
        let Metrics { w, h, .. } = self.metrics;
        let width = buffer.area.width;
        let top = f64::from(y) * h;
        self.ctx.save();
        self.ctx.begin_path();
        self.ctx.rect(0.0, top, f64::from(width) * w, h);
        self.ctx.clip();
        // Backgrounds, in runs of one colour.
        let mut run: Option<(u16, Rgb)> = None;
        let mut x = 0;
        while x < width {
            let Some(cell) = buffer.cell((x, y)) else {
                break;
            };
            let span = cell_width(cell).min(width - x);
            let (fg, bg) = self.colors(cell);
            let bg = if cursor == Some(x) { fg } else { bg };
            match run {
                Some((_, colour)) if colour == bg => {}
                Some((start, colour)) => {
                    self.fill_cells(start, x, top, colour);
                    run = Some((x, bg));
                }
                None => run = Some((x, bg)),
            }
            x += span;
        }
        if let Some((start, colour)) = run {
            self.fill_cells(start, width, top, colour);
        }
        // Glyphs.
        let mut x = 0;
        while x < width {
            let Some(cell) = buffer.cell((x, y)) else {
                break;
            };
            let span = cell_width(cell).min(width - x);
            let (fg, bg) = self.colors(cell);
            let fg = if cursor == Some(x) { bg } else { fg };
            self.glyph(cell, f64::from(x) * w, top, span, fg);
            x += span;
        }
        self.ctx.restore();
        // `restore` brings back the fill and font from before `save`.
        self.fill = None;
        self.font = None;
    }

    /// A cell's foreground and background after `REVERSED` and `DIM`.
    fn colors(&self, cell: &Cell) -> (Rgb, Rgb) {
        let (mut fg, mut bg) = (rgb(cell.fg, self.fg), rgb(cell.bg, self.bg));
        if cell.modifier.contains(Modifier::REVERSED) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.modifier.contains(Modifier::DIM) {
            fg = fg.blend(bg, DIM);
        }
        (fg, bg)
    }

    fn fill_cells(&mut self, from: u16, to: u16, top: f64, colour: Rgb) {
        let Metrics { w, h, .. } = self.metrics;
        self.set_fill(colour);
        self.ctx
            .fill_rect(f64::from(from) * w, top, f64::from(to - from) * w, h);
    }

    fn set_fill(&mut self, colour: Rgb) {
        if self.fill == Some(colour.key()) {
            return;
        }
        self.css.clear();
        let _ = write!(
            self.css,
            "#{:02x}{:02x}{:02x}",
            colour.0, colour.1, colour.2
        );
        self.ctx.set_fill_style_str(&self.css);
        self.ctx.set_stroke_style_str(&self.css);
        self.fill = Some(colour.key());
    }

    fn set_font(&mut self, modifier: Modifier) {
        let index = usize::from(modifier.contains(Modifier::BOLD))
            | usize::from(modifier.contains(Modifier::ITALIC)) << 1;
        if self.font == Some(index) {
            return;
        }
        if let Some(font) = self.fonts.get(index) {
            self.ctx.set_font(font);
        }
        self.font = Some(index);
    }

    fn glyph(&mut self, cell: &Cell, x0: f64, y0: f64, span: u16, fg: Rgb) {
        let symbol = cell.symbol();
        if symbol.is_empty() || symbol == " " || cell.diff_option == CellDiffOption::Skip {
            return;
        }
        self.set_fill(fg);
        if let Some(shape) = glyphs::shape(symbol) {
            self.shape(shape, x0, y0);
        } else {
            self.text(symbol, cell.modifier, x0, y0, span);
        }
        let Metrics { w, h, base, .. } = self.metrics;
        let thickness = self.dpr.round().max(1.0);
        let wide = f64::from(span) * w;
        if cell.modifier.contains(Modifier::UNDERLINED) {
            self.ctx
                .fill_rect(x0, y0 + base + 2.0 * thickness, wide, thickness);
        }
        if cell.modifier.contains(Modifier::CROSSED_OUT) {
            self.ctx
                .fill_rect(x0, y0 + (h / 2.0).round(), wide, thickness);
        }
    }

    fn text(&mut self, symbol: &str, modifier: Modifier, x0: f64, y0: f64, span: u16) {
        self.set_font(modifier);
        let Metrics { w, base, .. } = self.metrics;
        let wide = span > 1 || symbol.chars().next().is_some_and(|c| u32::from(c) > 0x2fff);
        if !wide {
            let _ = self.ctx.fill_text(symbol, x0, y0 + base);
            return;
        }
        // Emoji and other wide text: centred in exactly `span` cells, shrunk
        // if it's wider than that.
        let room = f64::from(span) * w;
        let measured = self.ctx.measure_text(symbol).map_or(room, |m| m.width());
        if measured > room && measured > 0.0 {
            let scale = room / measured;
            self.ctx.save();
            let _ = self.ctx.translate(x0 + room / 2.0, y0 + base);
            let _ = self.ctx.scale(scale, scale);
            let _ = self.ctx.fill_text(symbol, -measured / 2.0, 0.0);
            self.ctx.restore();
            self.font = None;
        } else {
            let _ = self
                .ctx
                .fill_text(symbol, x0 + (room - measured) / 2.0, y0 + base);
        }
    }

    fn shape(&mut self, shape: Shape, x0: f64, y0: f64) {
        let Metrics { w, h, .. } = self.metrics;
        // Snapped to device pixels, so neighbouring cells meet exactly.
        let at_x = |f: f64| x0 + (w * f).round();
        let at_y = |f: f64| y0 + (h * f).round();
        let rect = |ctx: &CanvasRenderingContext2d, fx0: f64, fy0: f64, fx1: f64, fy1: f64| {
            ctx.fill_rect(
                at_x(fx0),
                at_y(fy0),
                at_x(fx1) - at_x(fx0),
                at_y(fy1) - at_y(fy0),
            );
        };
        let ctx = &self.ctx;
        match shape {
            Shape::Octant(mask) => {
                for i in 0..8 {
                    if mask & (1 << i) != 0 {
                        let (c, r) = (f64::from(i % 2), f64::from(i / 2));
                        rect(ctx, c / 2.0, r / 4.0, c / 2.0 + 0.5, r / 4.0 + 0.25);
                    }
                }
            }
            Shape::Sextant(mask) => {
                for i in 0..6 {
                    if mask & (1 << i) != 0 {
                        let (c, r) = (f64::from(i % 2), f64::from(i / 2));
                        rect(ctx, c / 2.0, r / 3.0, c / 2.0 + 0.5, (r + 1.0) / 3.0);
                    }
                }
            }
            Shape::Eighths(left, top, right, bottom) => {
                let eighth = |v: u8| f64::from(v) / 8.0;
                rect(
                    ctx,
                    eighth(left),
                    eighth(top),
                    eighth(right),
                    eighth(bottom),
                );
            }
            Shape::Shade(quarters) => {
                ctx.set_global_alpha(f64::from(quarters) / 4.0);
                rect(ctx, 0.0, 0.0, 1.0, 1.0);
                ctx.set_global_alpha(1.0);
            }
            Shape::QuarterCircle(corner) => {
                let (cx, cy) = corner_point(corner, x0, y0, w, h);
                clipped(ctx, x0, y0, w, h, |ctx| {
                    let _ = ctx.ellipse(cx, cy, w / 2.0, w / 2.0, 0.0, 0.0, TAU);
                });
            }
            Shape::Powerline(kind) => clipped(ctx, x0, y0, w, h, |ctx| match kind {
                Powerline::RightTriangle => {
                    ctx.move_to(x0, y0);
                    ctx.line_to(x0 + w, y0 + h / 2.0);
                    ctx.line_to(x0, y0 + h);
                }
                Powerline::LeftTriangle => {
                    ctx.move_to(x0 + w, y0);
                    ctx.line_to(x0, y0 + h / 2.0);
                    ctx.line_to(x0 + w, y0 + h);
                }
                Powerline::RightRound => {
                    let _ = ctx.ellipse(x0, y0 + h / 2.0, w, h / 2.0, 0.0, -FRAC_PI_2, FRAC_PI_2);
                }
                Powerline::LeftRound => {
                    let _ = ctx.ellipse(
                        x0 + w,
                        y0 + h / 2.0,
                        w,
                        h / 2.0,
                        0.0,
                        FRAC_PI_2,
                        3.0 * FRAC_PI_2,
                    );
                }
            }),
            Shape::Lines(lines) => self.lines(lines, x0, y0),
            Shape::Arc(corner) => self.arc(corner, x0, y0),
            Shape::Braille(dots) => {
                let r = (w / 2.0).min(h / 4.0) * 0.32;
                for bit in 0..8 {
                    if dots & (1 << bit) != 0 {
                        let (c, row) = glyphs::braille_dot(bit);
                        let cx = x0 + w * (f64::from(c) * 2.0 + 1.0) / 4.0;
                        let cy = y0 + h * (f64::from(row) * 2.0 + 1.0) / 8.0;
                        ctx.begin_path();
                        let _ = ctx.arc(cx, cy, r, 0.0, TAU);
                        ctx.fill();
                    }
                }
            }
            Shape::Symbol(symbol) => self.symbol(symbol, x0, y0),
        }
    }

    /// Box drawing as filled bars from the centre to the edges.
    fn lines(&self, [up, right, down, left]: [Line; 4], x0: f64, y0: f64) {
        let Metrics { w, h, .. } = self.metrics;
        let t = self.dpr.round().max(1.0);
        let cx = x0 + ((w - t) / 2.0).floor();
        let cy = y0 + ((h - t) / 2.0).floor();
        let ctx = &self.ctx;
        let seg = |kind: Line, x: f64, y: f64, sw: f64, sh: f64| {
            let horizontal = sw > sh;
            match kind {
                Line::None => {}
                Line::Light => ctx.fill_rect(x, y, sw, sh),
                Line::Heavy => {
                    if horizontal {
                        ctx.fill_rect(x, y - t / 2.0, sw, t * 2.0);
                    } else {
                        ctx.fill_rect(x - t / 2.0, y, t * 2.0, sh);
                    }
                }
                Line::Double => {
                    let gap = t * 2.0;
                    if horizontal {
                        ctx.fill_rect(x, y - gap, sw, t);
                        ctx.fill_rect(x, y + gap, sw, t);
                    } else {
                        ctx.fill_rect(x - gap, y, t, sh);
                        ctx.fill_rect(x + gap, y, t, sh);
                    }
                }
            }
        };
        seg(up, cx, y0, t, cy - y0 + t);
        seg(down, cx, cy, t, y0 + h - cy);
        seg(left, x0, cy, cx - x0 + t, t);
        seg(right, cx, cy, x0 + w - cx, t);
    }

    /// `╭ ╮ ╰ ╯`: a line from one edge's middle, round the centre, to the other's.
    fn arc(&self, corner: Corner, x0: f64, y0: f64) {
        let Metrics { w, h, .. } = self.metrics;
        let t = self.dpr.round().max(1.0);
        let (dx, dy) = Shape::arc_ends(corner);
        let (dx, dy) = (f64::from(dx), f64::from(dy));
        let r = w.min(h) / 2.0;
        let mx = x0 + ((w - t) / 2.0).floor() + t / 2.0;
        let my = y0 + ((h - t) / 2.0).floor() + t / 2.0;
        let ex = if dx > 0.0 { x0 + w } else { x0 };
        let ey = if dy > 0.0 { y0 + h } else { y0 };
        let ctx = &self.ctx;
        ctx.set_line_width(t);
        ctx.begin_path();
        ctx.move_to(ex, my);
        ctx.line_to(dx.mul_add(r, mx), my);
        let _ = ctx.arc_to(mx, my, mx, dy.mul_add(r, my), r);
        ctx.line_to(mx, ey);
        ctx.stroke();
    }

    fn symbol(&self, symbol: Symbol, x0: f64, y0: f64) {
        let Metrics { w, h, base, .. } = self.metrics;
        let t = self.dpr.round().max(1.0) * 1.2;
        let ctx = &self.ctx;
        ctx.set_line_width(t);
        ctx.set_line_cap("round");
        ctx.set_line_join("round");
        match symbol {
            Symbol::ReturnArrow => {
                // Down from the top third, then right, with an arrowhead.
                let (left, right) = (x0 + w * 0.25, x0 + w * 0.85);
                let (top, mid) = (y0 + h * 0.2, y0 + base - h * 0.2);
                let head = w * 0.22;
                ctx.begin_path();
                ctx.move_to(left, top);
                ctx.line_to(left, mid);
                ctx.line_to(right, mid);
                ctx.move_to(right - head, mid - head);
                ctx.line_to(right, mid);
                ctx.line_to(right - head, mid + head);
                ctx.stroke();
            }
            Symbol::Pencil => {
                // A thick diagonal body and a small tip, pointing down-left.
                let (tip_x, tip_y) = (x0 + w * 0.15, y0 + base);
                let (end_x, end_y) = (x0 + w * 0.9, y0 + base - h * 0.62);
                ctx.save();
                ctx.set_line_width(w * 0.28);
                ctx.set_line_cap("butt");
                ctx.begin_path();
                ctx.move_to(w.mul_add(0.18, tip_x), tip_y - h * 0.14);
                ctx.line_to(end_x, end_y);
                ctx.stroke();
                ctx.restore();
                ctx.begin_path();
                ctx.move_to(tip_x, tip_y);
                ctx.line_to(w.mul_add(0.3, tip_x), tip_y - h * 0.06);
                ctx.line_to(w.mul_add(0.08, tip_x), tip_y - h * 0.24);
                ctx.close_path();
                ctx.fill();
            }
            Symbol::Quote => {
                // Two heavy turned commas: a dot with a tail rising to the right.
                for offset in [0.08, 0.5] {
                    let cx = x0 + w * (offset + 0.17);
                    let cy = y0 + h * 0.42;
                    let r = w * 0.15;
                    ctx.begin_path();
                    let _ = ctx.arc(cx, cy, r, 0.0, TAU);
                    ctx.fill();
                    ctx.begin_path();
                    ctx.move_to(cx - r, cy);
                    ctx.quadratic_curve_to(cx - r, cy - r * 2.4, cx + r * 0.9, cy - r * 2.8);
                    ctx.line_to(cx + r * 0.6, cy - r * 2.1);
                    ctx.quadratic_curve_to(cx, cy - r * 1.6, cx + r, cy);
                    ctx.close_path();
                    ctx.fill();
                }
            }
        }
        ctx.set_line_cap("butt");
        ctx.set_line_join("miter");
    }
}

/// Fill a path built by `path`, clipped to the cell.
fn clipped(
    ctx: &CanvasRenderingContext2d,
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
    path: impl FnOnce(&CanvasRenderingContext2d),
) {
    ctx.save();
    ctx.begin_path();
    ctx.rect(x0, y0, w, h);
    ctx.clip();
    ctx.begin_path();
    path(ctx);
    ctx.fill();
    ctx.restore();
}

const fn corner_point(corner: Corner, x0: f64, y0: f64, w: f64, h: f64) -> (f64, f64) {
    match corner {
        Corner::TopLeft => (x0, y0),
        Corner::TopRight => (x0 + w, y0),
        Corner::BottomLeft => (x0, y0 + h),
        Corner::BottomRight => (x0 + w, y0 + h),
    }
}

fn round_u16(v: f64) -> u16 {
    u16::try_from(clamp_u32(v, u32::from(u16::MAX))).unwrap_or(u16::MAX)
}

fn floor_u16(v: f64) -> u16 {
    round_u16(v.floor())
}

fn to_u32(v: f64) -> u32 {
    clamp_u32(v, u32::MAX)
}

/// `v` rounded and clamped to `0..=max` (NaN is 0).
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 0..=max first"
)]
fn clamp_u32(v: f64, max: u32) -> u32 {
    let v = v.round();
    if v.is_nan() || v <= 0.0 {
        0
    } else if v >= f64::from(max) {
        max
    } else {
        v as u32
    }
}
