//! Buffer → SVG (WEBSITE.md §6.4): the poster and OG image the site embeds.
//!
//! **The geometry mirrors `paint.rs` cell for cell** — the same [`metrics`]
//! rounding for the cell shape, the same fractions for every
//! [`glyphs::Shape`], the same run merging, the same line thickness. The
//! two must be changed together: a shape that moves in the Canvas painter has
//! to move here too, or the poster drifts from what the live demo draws.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::style::Modifier;

use crate::color::{Rgb, cell_colors, cell_width};
use crate::glyphs::{self, Corner, Line, Powerline, Shape, Symbol};
use crate::metrics;

/// The text baseline as a fraction of the cell height: what the Canvas painter
/// measures for `JetBrains Mono` at its line height (≈ 0.96 × the font size).
const BASELINE: f32 = 0.8;
/// A hairline as a fraction of the cell height: one device pixel at the row
/// height the demo draws at (`paint.rs` uses the device pixel ratio).
const HAIRLINE: f32 = 1.0 / 18.0;
/// A hairline never vanishes on a small poster.
const MIN_HAIRLINE: f32 = 0.1;

/// How a buffer becomes a picture: cell size, font, theme defaults, name.
/// The cell size and font size go together: [`metrics::cell`] gives the pair
/// the live demo draws with, and [`metrics::letter_spacing`] keeps the text
/// on the cell columns.
#[derive(Debug, Clone)]
pub struct SvgOptions {
    /// A cell's width in SVG user units.
    pub cell_width: f32,
    /// A cell's height in SVG user units.
    pub cell_height: f32,
    /// The `font-family` attribute for the text.
    pub font_family: String,
    /// The `font-size` attribute for the text.
    pub font_size: f32,
    /// The theme's default foreground, what `Color::Reset` means.
    pub foreground: Rgb,
    /// The theme's default background, and the picture's backdrop.
    pub background: Rgb,
    /// The accessible name (the `<title>`).
    pub title: String,
}

/// `buffer` as a standalone SVG document, deterministic down to the bytes.
#[must_use]
pub fn buffer_to_svg(buffer: &Buffer, options: &SvgOptions) -> String {
    let mut page = Page::new(options);
    page.document(buffer);
    page.out
}

/// One document being written.
struct Page<'a> {
    o: &'a SvgOptions,
    /// Cell width, cell height, baseline within a cell, hairline thickness.
    cw: f32,
    ch: f32,
    base: f32,
    line: f32,
    /// `letter-spacing` for the text group, so a glyph ends where the next
    /// cell column starts (`metrics::letter_spacing`); `None` when the cell
    /// is exactly one advance wide.
    spacing: Option<f32>,
    out: String,
    /// Scratch for colour attributes, so nothing allocates per element.
    css: String,
}

impl<'a> Page<'a> {
    fn new(o: &'a SvgOptions) -> Self {
        Self {
            o,
            cw: o.cell_width,
            ch: o.cell_height,
            base: o.cell_height * BASELINE,
            line: (o.cell_height * HAIRLINE).max(MIN_HAIRLINE),
            spacing: metrics::letter_spacing(o.cell_width, o.font_size),
            out: String::new(),
            css: String::new(),
        }
    }

    fn document(&mut self, buffer: &Buffer) {
        let (width, height) = (
            num(f32::from(buffer.area.width) * self.cw),
            num(f32::from(buffer.area.height) * self.ch),
        );
        let _ = write!(
            self.out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" role="img" aria-labelledby="poster-title">"#
        );
        self.title();
        self.backdrop(&width, &height);
        for y in 0..buffer.area.height {
            self.background_row(buffer, y);
        }
        for y in 0..buffer.area.height {
            self.shape_row(buffer, y);
        }
        self.text(buffer);
        self.out.push_str("</svg>");
    }

    fn title(&mut self) {
        self.out.push_str(r#"<title id="poster-title">"#);
        escape(&self.o.title, &mut self.out);
        self.out.push_str("</title>");
    }

    fn backdrop(&mut self, width: &str, height: &str) {
        self.o.background.write_hex(&mut self.css);
        let _ = write!(
            self.out,
            r#"<rect width="{width}" height="{height}" fill="{css}" stroke="none"/>"#,
            css = self.css
        );
    }

    /// One rect per run of the same non-default background, as `paint.rs` fills.
    fn background_row(&mut self, buffer: &Buffer, y: u16) {
        let width = buffer.area.width;
        let mut colour = self.o.background;
        let (mut start, mut end) = (0, 0);
        let mut open = false;
        let mut x = 0;
        while x < width {
            let Some(cell) = buffer.cell((x, y)) else {
                break;
            };
            let span = cell_width(cell).min(width - x);
            // A continuation cell's background comes from the cell that owns
            // it, whose span already covers this one.
            if cell.diff_option != CellDiffOption::Skip {
                let (_, bg) = cell_colors(cell, self.o.foreground, self.o.background);
                if bg != colour {
                    if open {
                        self.bg_rect(start, end, y, colour);
                    }
                    colour = bg;
                    start = x;
                    open = bg != self.o.background;
                }
                end = x + span;
            }
            x += span;
        }
        if open {
            self.bg_rect(start, end, y, colour);
        }
    }

    fn bg_rect(&mut self, from: u16, to: u16, y: u16, colour: Rgb) {
        let (x, width) = (
            num(f32::from(from) * self.cw),
            num(f32::from(to - from) * self.cw),
        );
        let (top, height) = (num(f32::from(y) * self.ch), num(self.ch));
        colour.write_hex(&mut self.css);
        let _ = write!(
            self.out,
            r#"<rect x="{x}" y="{top}" width="{width}" height="{height}" fill="{css}" stroke="none"/>"#,
            css = self.css
        );
    }

    /// The cells `glyphs` draws as shapes, grouped per row by colour.
    fn shape_row(&mut self, buffer: &Buffer, y: u16) {
        let width = buffer.area.width;
        let mut groups: BTreeMap<u32, (Rgb, String)> = BTreeMap::new();
        let mut x = 0;
        while x < width {
            let Some(cell) = buffer.cell((x, y)) else {
                break;
            };
            let span = cell_width(cell).min(width - x);
            if cell.diff_option != CellDiffOption::Skip
                && let Some(shape) = glyphs::shape(cell.symbol())
            {
                let (fg, _) = cell_colors(cell, self.o.foreground, self.o.background);
                let body = &mut groups
                    .entry(fg.key())
                    .or_insert_with(|| (fg, String::new()))
                    .1;
                self.shape(shape, f32::from(x) * self.cw, f32::from(y) * self.ch, body);
            }
            x += span;
        }
        for (colour, body) in groups.values() {
            if body.is_empty() {
                continue;
            }
            colour.write_hex(&mut self.css);
            let _ = write!(
                self.out,
                r#"<g fill="{css}" stroke="{css}">"#,
                css = self.css
            );
            self.out.push_str(body);
            self.out.push_str("</g>");
        }
    }

    /// Every row's text in one group, so the font is written once; no group at
    /// all when the buffer has no text.
    fn text(&mut self, buffer: &Buffer) {
        let first = self.out.len();
        for y in 0..buffer.area.height {
            self.text_row(buffer, y);
        }
        if self.out.len() == first {
            return;
        }
        let family = escaped(&self.o.font_family);
        let size = num(self.o.font_size);
        let spacing = self.spacing.map_or(String::new(), |room| {
            format!(r#" letter-spacing="{}""#, num(room))
        });
        let body = self.out.split_off(first);
        let _ = write!(
            self.out,
            r#"<g font-family="{family}" font-size="{size}"{spacing}>"#
        );
        self.out.push_str(&body);
        self.out.push_str("</g>");
    }

    /// One `<text>` per row: runs of the same colour and font as `<tspan>`s,
    /// each at an exact `x` so nothing drifts after a wide cell.
    fn text_row(&mut self, buffer: &Buffer, y: u16) {
        let width = buffer.area.width;
        let mut runs: Vec<Run> = Vec::new();
        let mut open: Option<Run> = None;
        let mut x = 0;
        while x < width {
            let Some(cell) = buffer.cell((x, y)) else {
                break;
            };
            let span = cell_width(cell).min(width - x);
            let symbol = cell.symbol();
            let drawn = cell.diff_option != CellDiffOption::Skip
                && !symbol.is_empty()
                && symbol != " "
                && glyphs::shape(symbol).is_none();
            if !drawn {
                close(&mut runs, open.take());
                x += span;
                continue;
            }
            // As in `paint.rs`: centred in its own cells, so it stands alone.
            let wide = span > 1 || symbol.chars().next().is_some_and(|c| u32::from(c) > 0x2fff);
            let (fg, _) = cell_colors(cell, self.o.foreground, self.o.background);
            let font = cell.modifier;
            match &mut open {
                Some(run) if !wide && run.fg == fg && run.font == font => {
                    run.text.push_str(symbol);
                }
                _ => {
                    close(&mut runs, open.take());
                    open = Some(Run {
                        x,
                        fg,
                        font,
                        text: symbol.to_owned(),
                    });
                }
            }
            if wide {
                close(&mut runs, open.take());
            }
            x += span;
        }
        close(&mut runs, open.take());
        if runs.is_empty() {
            return;
        }
        let top = num(f32::from(y) * self.ch + self.base);
        let _ = write!(self.out, r#"<text y="{top}" xml:space="preserve">"#);
        for run in &runs {
            run.fg.write_hex(&mut self.css);
            run.write(&mut self.out, self.cw, &self.css);
        }
        self.out.push_str("</text>");
    }

    fn shape(&self, shape: Shape, x0: f32, y0: f32, out: &mut String) {
        match shape {
            Shape::Octant(mask) => self.grid(mask, 2, 4, x0, y0, out),
            Shape::Sextant(mask) => self.grid(mask, 2, 3, x0, y0, out),
            Shape::Eighths(left, top, right, bottom) => {
                self.eighths((left, top, right, bottom), x0, y0, out);
            }
            Shape::Shade(quarters) => self.shade(quarters, x0, y0, out),
            Shape::QuarterCircle(corner) => self.quarter_circle(corner, x0, y0, out),
            Shape::Powerline(kind) => self.powerline(kind, x0, y0, out),
            Shape::Lines(lines) => self.lines(lines, x0, y0, out),
            Shape::Arc(corner) => self.arc(corner, x0, y0, out),
            Shape::Braille(dots) => self.braille(dots, x0, y0, out),
            Shape::Symbol(symbol) => self.symbol(symbol, x0, y0, out),
        }
    }

    /// The octant (2×4) and sextant (2×3) grids, bit `i` at column `i % 2` and
    /// row `i / 2`, as `paint.rs` fills them.
    fn grid(&self, mask: u8, columns: u8, rows: u8, x0: f32, y0: f32, out: &mut String) {
        let (across, down) = (f32::from(columns), f32::from(rows));
        for i in 0..columns * rows {
            if mask & (1u8 << i) == 0 {
                continue;
            }
            let (column, row) = (i % columns, i / columns);
            rect(
                out,
                x0 + self.cw * f32::from(column) / across,
                y0 + self.ch * f32::from(row) / down,
                self.cw / across,
                self.ch / down,
            );
        }
    }

    /// Partial blocks: one rectangle, in eighths of the cell.
    fn eighths(
        &self,
        (left, top, right, bottom): (u8, u8, u8, u8),
        x0: f32,
        y0: f32,
        out: &mut String,
    ) {
        let eighth = |v: u8| f32::from(v) / 8.0;
        rect(
            out,
            x0 + self.cw * eighth(left),
            y0 + self.ch * eighth(top),
            self.cw * (eighth(right) - eighth(left)),
            self.ch * (eighth(bottom) - eighth(top)),
        );
    }

    /// `░ ▒ ▓`: the whole cell at 1, 2 or 3 quarters' opacity.
    fn shade(&self, quarters: u8, x0: f32, y0: f32, out: &mut String) {
        let opacity = num(f32::from(quarters) / 4.0);
        let (x, y, w, h) = (num(x0), num(y0), num(self.cw), num(self.ch));
        let _ = write!(
            out,
            r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill-opacity="{opacity}" stroke="none"/>"#
        );
    }

    /// A quarter disc at the corner with a radius of half the cell's width:
    /// what `paint.rs` gets by clipping that circle to the cell, which is
    /// exactly the quarter as long as the cell is at least as tall as it is wide.
    fn quarter_circle(&self, corner: Corner, x0: f32, y0: f32, out: &mut String) {
        let (cx, cy) = corner_point(corner, x0, y0, self.cw, self.ch);
        let (dx, dy) = Shape::arc_ends(corner);
        let (fdx, fdy) = (f32::from(dx), f32::from(dy));
        let radius = self.cw / 2.0;
        let mut path = String::new();
        point(&mut path, 'M', cx, cy);
        point(&mut path, 'L', cx + fdx * radius, cy);
        arc_to(
            &mut path,
            radius,
            radius,
            u8::from(dx == dy),
            cx,
            cy + fdy * radius,
        );
        path.push_str(" Z");
        filled(out, &path);
    }

    fn powerline(&self, kind: Powerline, x0: f32, y0: f32, out: &mut String) {
        let (cw, ch) = (self.cw, self.ch);
        let mut path = String::new();
        match kind {
            Powerline::RightTriangle => {
                point(&mut path, 'M', x0, y0);
                point(&mut path, 'L', x0 + cw, y0 + ch / 2.0);
                point(&mut path, 'L', x0, y0 + ch);
            }
            Powerline::LeftTriangle => {
                point(&mut path, 'M', x0 + cw, y0);
                point(&mut path, 'L', x0, y0 + ch / 2.0);
                point(&mut path, 'L', x0 + cw, y0 + ch);
            }
            Powerline::RightRound => {
                point(&mut path, 'M', x0, y0);
                arc_to(&mut path, cw, ch / 2.0, 1, x0, y0 + ch);
            }
            Powerline::LeftRound => {
                point(&mut path, 'M', x0 + cw, y0 + ch);
                arc_to(&mut path, cw, ch / 2.0, 1, x0 + cw, y0);
            }
        }
        path.push_str(" Z");
        filled(out, &path);
    }

    /// Box drawing: `paint.rs` fills bars from the centre out; here they're
    /// stroked paths of the same thickness along the same lines.
    fn lines(&self, [up, right, down, left]: [Line; 4], x0: f32, y0: f32, out: &mut String) {
        let (cw, ch, line) = (self.cw, self.ch, self.line);
        let (cx, cy) = (x0 + cw / 2.0, y0 + ch / 2.0);
        let mut light = String::new();
        let mut heavy = String::new();
        let mut near = String::new();
        let mut far = String::new();
        for (kind, vertical, x1, y1, x2, y2) in [
            (up, true, cx, y0, cx, cy),
            (right, false, cx, cy, x0 + cw, cy),
            (down, true, cx, cy, cx, y0 + ch),
            (left, false, x0, cy, cx, cy),
        ] {
            match kind {
                Line::None => {}
                Line::Light => segment(&mut light, x1, y1, x2, y2),
                Line::Heavy => segment(&mut heavy, x1, y1, x2, y2),
                Line::Double => {
                    let gap = line * 2.0;
                    if vertical {
                        segment(&mut near, x1 - gap, y1, x2 - gap, y2);
                        segment(&mut far, x1 + gap, y1, x2 + gap, y2);
                    } else {
                        segment(&mut near, x1, y1 - gap, x2, y2 - gap);
                        segment(&mut far, x1, y1 + gap, x2, y2 + gap);
                    }
                }
            }
        }
        if !light.is_empty() {
            stroked(out, &light, line);
        }
        if !heavy.is_empty() {
            stroked(out, &heavy, line * 2.0);
        }
        if !near.is_empty() {
            stroked(out, &near, line);
            stroked(out, &far, line);
        }
    }

    /// `╭ ╮ ╰ ╯`: edge middle to centre, round the corner, centre to edge.
    fn arc(&self, corner: Corner, x0: f32, y0: f32, out: &mut String) {
        let (cw, ch, line) = (self.cw, self.ch, self.line);
        let (dx, dy) = Shape::arc_ends(corner);
        let (fdx, fdy) = (f32::from(dx), f32::from(dy));
        let radius = cw.min(ch) / 2.0;
        let (mx, my) = (x0 + cw / 2.0, y0 + ch / 2.0);
        let ex = if dx > 0 { x0 + cw } else { x0 };
        let ey = if dy > 0 { y0 + ch } else { y0 };
        let mut path = String::new();
        point(&mut path, 'M', ex, my);
        point(&mut path, 'L', mx + fdx * radius, my);
        arc_to(
            &mut path,
            radius,
            radius,
            u8::from(dx != dy),
            mx,
            my + fdy * radius,
        );
        point(&mut path, 'L', mx, ey);
        stroked(out, &path, line);
    }

    fn braille(&self, dots: u8, x0: f32, y0: f32, out: &mut String) {
        let radius = (self.cw / 2.0).min(self.ch / 4.0) * 0.32;
        for bit in 0..8u8 {
            if dots & (1 << bit) == 0 {
                continue;
            }
            let (column, row) = glyphs::braille_dot(bit);
            let cx = x0 + self.cw * (f32::from(column) * 2.0 + 1.0) / 4.0;
            let cy = y0 + self.ch * (f32::from(row) * 2.0 + 1.0) / 8.0;
            circle(out, cx, cy, radius);
        }
    }

    /// The symbols the webfont lacks (D107, D145), as in `paint.rs`.
    fn symbol(&self, symbol: Symbol, x0: f32, y0: f32, out: &mut String) {
        let (cw, ch, base) = (self.cw, self.ch, self.base);
        match symbol {
            Symbol::ReturnArrow => {
                let (left, right) = (x0 + cw * 0.25, x0 + cw * 0.85);
                let (top, mid) = (y0 + ch * 0.2, y0 + base - ch * 0.2);
                let head = cw * 0.22;
                let mut path = String::new();
                point(&mut path, 'M', left, top);
                point(&mut path, 'L', left, mid);
                point(&mut path, 'L', right, mid);
                point(&mut path, 'M', right - head, mid - head);
                point(&mut path, 'L', right, mid);
                point(&mut path, 'L', right - head, mid + head);
                rounded(out, &path, self.line * 1.2);
            }
            Symbol::Pencil => {
                let (tip_x, tip_y) = (x0 + cw * 0.15, y0 + base);
                let (end_x, end_y) = (x0 + cw * 0.9, y0 + base - ch * 0.62);
                let mut body = String::new();
                point(&mut body, 'M', tip_x + cw * 0.18, tip_y - ch * 0.14);
                point(&mut body, 'L', end_x, end_y);
                stroked(out, &body, cw * 0.28);
                let mut tip = String::new();
                point(&mut tip, 'M', tip_x, tip_y);
                point(&mut tip, 'L', tip_x + cw * 0.3, tip_y - ch * 0.06);
                point(&mut tip, 'L', tip_x + cw * 0.08, tip_y - ch * 0.24);
                tip.push_str(" Z");
                filled(out, &tip);
            }
            Symbol::Quote | Symbol::CloseQuote => {
                // `❞` is `❝` turned half a circle about the pair's middle.
                let radius = cw * 0.15;
                let (mid_x, mid_y) = (x0 + cw * 0.46, ch.mul_add(0.42, y0) - radius * 0.9);
                let turned = symbol == Symbol::CloseQuote;
                let at = |x: f32, y: f32| {
                    if turned {
                        (2.0 * mid_x - x, 2.0 * mid_y - y)
                    } else {
                        (x, y)
                    }
                };
                for offset in [0.08, 0.5] {
                    let cx = x0 + cw * (offset + 0.17);
                    let cy = y0 + ch * 0.42;
                    let (dot_x, dot_y) = at(cx, cy);
                    circle(out, dot_x, dot_y, radius);
                    let start = at(cx - radius, cy);
                    let c1 = at(cx - radius, cy - radius * 2.4);
                    let p1 = at(cx + radius * 0.9, cy - radius * 2.8);
                    let p2 = at(cx + radius * 0.6, cy - radius * 2.1);
                    let (c3, p3) = (at(cx, cy - radius * 1.6), at(cx + radius, cy));
                    let mut tail = String::new();
                    point(&mut tail, 'M', start.0, start.1);
                    curve(&mut tail, c1.0, c1.1, p1.0, p1.1);
                    point(&mut tail, 'L', p2.0, p2.1);
                    curve(&mut tail, c3.0, c3.1, p3.0, p3.1);
                    tail.push_str(" Z");
                    filled(out, &tail);
                }
            }
        }
    }
}

/// A run of text drawn in one colour and font, starting at cell `x`.
struct Run {
    x: u16,
    fg: Rgb,
    font: Modifier,
    text: String,
}

impl Run {
    fn write(&self, out: &mut String, cell: f32, css: &str) {
        let x = num(f32::from(self.x) * cell);
        let mut style = String::new();
        if self.font.contains(Modifier::BOLD) {
            style.push_str(r#" font-weight="bold""#);
        }
        if self.font.contains(Modifier::ITALIC) {
            style.push_str(r#" font-style="italic""#);
        }
        if let Some(decoration) = self.decoration() {
            let _ = write!(style, r#" text-decoration="{decoration}""#);
        }
        let _ = write!(out, r#"<tspan x="{x}" fill="{css}"{style}>"#);
        escape(&self.text, out);
        out.push_str("</tspan>");
    }

    fn decoration(&self) -> Option<&'static str> {
        let under = self.font.contains(Modifier::UNDERLINED);
        let through = self.font.contains(Modifier::CROSSED_OUT);
        match (under, through) {
            (true, true) => Some("underline line-through"),
            (true, false) => Some("underline"),
            (false, true) => Some("line-through"),
            (false, false) => None,
        }
    }
}

/// Ends the open run: spaces at its edge are dropped, and a run left with
/// nothing in it is omitted.
fn close(runs: &mut Vec<Run>, run: Option<Run>) {
    let Some(mut run) = run else {
        return;
    };
    while run.text.ends_with(' ') {
        run.text.pop();
    }
    if !run.text.is_empty() {
        runs.push(run);
    }
}

/// The corner of a cell, as `paint.rs` names it.
const fn corner_point(corner: Corner, x0: f32, y0: f32, w: f32, h: f32) -> (f32, f32) {
    match corner {
        Corner::TopLeft => (x0, y0),
        Corner::TopRight => (x0 + w, y0),
        Corner::BottomLeft => (x0, y0 + h),
        Corner::BottomRight => (x0 + w, y0 + h),
    }
}

/// A path command: move to, or a line to, a point.
fn point(path: &mut String, command: char, x: f32, y: f32) {
    let _ = write!(path, " {command} {} {}", num(x), num(y));
}

/// A path command: an elliptical arc to `x y`.
fn arc_to(path: &mut String, rx: f32, ry: f32, sweep: u8, x: f32, y: f32) {
    let _ = write!(
        path,
        " A {} {} 0 0 {} {} {}",
        num(rx),
        num(ry),
        sweep,
        num(x),
        num(y)
    );
}

/// A path command: a quadratic curve to `x y`.
fn curve(path: &mut String, cx: f32, cy: f32, x: f32, y: f32) {
    let _ = write!(path, " Q {} {} {} {}", num(cx), num(cy), num(x), num(y));
}

/// A path: a straight line from one point to another.
fn segment(path: &mut String, x1: f32, y1: f32, x2: f32, y2: f32) {
    point(path, 'M', x1, y1);
    point(path, 'L', x2, y2);
}

fn rect(out: &mut String, x: f32, y: f32, w: f32, h: f32) {
    let (x, y, w, h) = (num(x), num(y), num(w), num(h));
    let _ = write!(
        out,
        r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" stroke="none"/>"#
    );
}

fn circle(out: &mut String, cx: f32, cy: f32, r: f32) {
    let (cx, cy, r) = (num(cx), num(cy), num(r));
    let _ = write!(
        out,
        r#"<circle cx="{cx}" cy="{cy}" r="{r}" stroke="none"/>"#
    );
}

/// A closed path in the group's colour.
fn filled(out: &mut String, path: &str) {
    let _ = write!(out, r#"<path d="{path}" stroke="none"/>"#);
}

/// A path stroked in the group's colour at `width`.
fn stroked(out: &mut String, path: &str, width: f32) {
    let width = num(width);
    let _ = write!(
        out,
        r#"<path d="{path}" fill="none" stroke-width="{width}"/>"#
    );
}

/// A stroked path with round caps and joins.
fn rounded(out: &mut String, path: &str, width: f32) {
    let width = num(width);
    let _ = write!(
        out,
        r#"<path d="{path}" fill="none" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round"/>"#
    );
}

/// At most two decimals, no trailing zeros: the same bytes every run.
fn num(value: f32) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded == 0.0 {
        return "0".to_string();
    }
    let mut text = format!("{rounded:.2}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// `text` XML-escaped into `out`; control characters are dropped.
fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
}

/// [`escape`] as a fresh string.
fn escaped(text: &str) -> String {
    let mut out = String::new();
    escape(text, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};

    fn options() -> SvgOptions {
        SvgOptions {
            cell_width: 10.0,
            cell_height: 20.0,
            font_family: "JetBrains Mono".to_string(),
            font_size: 16.0,
            foreground: Rgb(0xe6, 0xe6, 0xe6),
            background: Rgb(0x10, 0x12, 0x16),
            title: "faro".to_string(),
        }
    }

    /// The poster's settings: 120 × 34 cells at 9 × 16.
    fn poster() -> SvgOptions {
        let mut o = options();
        o.cell_width = 9.0;
        o.cell_height = 16.0;
        o.font_size = 13.0;
        o.title = "faro in the browser".to_string();
        o
    }

    fn row(symbols: &[&str], style: Style) -> Buffer {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 1));
        let mut x = 0;
        for symbol in symbols {
            let (next, _) = buffer.set_stringn(x, 0, *symbol, usize::MAX, style);
            x = next;
        }
        buffer
    }

    #[test]
    fn backgrounds_merge_into_one_rect_per_run() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
        buffer.set_string(0, 0, "aaa", Style::default().bg(Color::Red));
        buffer.set_string(3, 0, "bbb", Style::default());
        let svg = buffer_to_svg(&buffer, &options());
        assert_eq!(svg.matches(r##"fill="#cd0000""##).count(), 1, "{svg}");
        assert!(
            svg.contains(
                r##"<rect x="0" y="0" width="30" height="20" fill="#cd0000" stroke="none"/>"##
            ),
            "{svg}"
        );
    }

    #[test]
    fn tspans_split_when_the_style_changes() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer.set_string(0, 0, "ab", Style::default().fg(Color::Red));
        buffer.set_string(
            2,
            0,
            "cd",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
        let svg = buffer_to_svg(&buffer, &options());
        assert!(
            svg.contains(r##"<tspan x="0" fill="#cd0000">ab</tspan>"##),
            "{svg}"
        );
        assert!(
            svg.contains(r##"<tspan x="20" fill="#00cd00" font-weight="bold">cd</tspan>"##),
            "{svg}"
        );
    }

    #[test]
    fn text_and_title_are_escaped() {
        let mut o = options();
        o.title = "Q&A <faro> \"x\" 'y'".to_string();
        let svg = buffer_to_svg(&row(&["<&>"], Style::default()), &o);
        assert!(
            svg.contains(
                r#"<title id="poster-title">Q&amp;A &lt;faro&gt; &quot;x&quot; &apos;y&apos;</title>"#
            ),
            "{svg}"
        );
        assert!(svg.contains(">&lt;&amp;&gt;</tspan>"), "{svg}");

        let mut out = String::new();
        escape("a\u{7}b\u{0}c", &mut out);
        assert_eq!(out, "abc");
    }

    #[test]
    fn block_glyphs_are_rects_not_text() {
        // The same grid `paint.rs` fills: eight quarters for `█`, the top four
        // for `▀` — never a glyph taken from the font.
        let full = buffer_to_svg(&row(&["█"], Style::default()), &options());
        assert_eq!(
            full.matches(r#"<rect x=""#).count(),
            8,
            "eight quarters, no text: {full}"
        );
        assert!(!full.contains("<tspan"), "{full}");

        let upper = buffer_to_svg(&row(&["▀"], Style::default()), &options());
        assert!(
            upper.contains(r#"<rect x="0" y="0" width="5" height="5" stroke="none"/>"#),
            "{upper}"
        );
        assert!(
            upper.contains(r#"<rect x="5" y="5" width="5" height="5" stroke="none"/>"#),
            "{upper}"
        );
        assert!(!upper.contains(r#"<rect x="0" y="10" "#), "{upper}");
        assert!(!upper.contains("<tspan"), "{upper}");
    }

    #[test]
    fn a_wide_char_keeps_the_next_tspan_exact() {
        let svg = buffer_to_svg(&row(&["語a"], Style::default()), &options());
        assert!(
            svg.contains(r##"<tspan x="0" fill="#e6e6e6">語</tspan>"##),
            "{svg}"
        );
        assert!(
            svg.contains(r##"<tspan x="20" fill="#e6e6e6">a</tspan>"##),
            "{svg}"
        );
    }

    #[test]
    fn reverse_swaps_the_colours() {
        let style = Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::REVERSED);
        let svg = buffer_to_svg(&row(&["a"], style), &options());
        assert!(
            svg.contains(r##"<tspan x="0" fill="#101216">a</tspan>"##),
            "{svg}"
        );
        assert!(
            svg.contains(
                r##"<rect x="0" y="0" width="10" height="20" fill="#cd0000" stroke="none"/>"##
            ),
            "{svg}"
        );
    }

    /// The cell shape and the text go together: a cell wider than the font's
    /// advance gets the difference back as `letter-spacing`, so a run of
    /// glyphs lands on cell columns instead of drifting (`metrics`).
    #[test]
    fn text_gets_letter_spacing_when_the_cell_is_wider_than_the_advance() {
        let svg = buffer_to_svg(&row(&["ab"], Style::default()), &options());
        assert!(
            svg.contains(r#"<g font-family="JetBrains Mono" font-size="16" letter-spacing="0.4">"#),
            "{svg}"
        );
        let mut o = options();
        o.cell_width = 9.6;
        let svg = buffer_to_svg(&row(&["ab"], Style::default()), &o);
        assert!(
            svg.contains(r#"<g font-family="JetBrains Mono" font-size="16">"#),
            "{svg}"
        );
        assert!(!svg.contains("letter-spacing"), "{svg}");
    }

    #[test]
    fn the_same_buffer_gives_the_same_svg() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 2));
        buffer.set_string(0, 0, "█a▀語b", Style::default().fg(Color::Cyan));
        buffer.set_string(
            0,
            1,
            "╭──╮",
            Style::default().fg(Color::Yellow).bg(Color::DarkGray),
        );
        let once = buffer_to_svg(&buffer, &options());
        let twice = buffer_to_svg(&buffer, &options());
        assert_eq!(once, twice);
        assert!(once.contains(r#"stroke-width=""#), "{once}");
    }

    #[test]
    fn numbers_have_two_decimals_at_most_and_no_trailing_zeros() {
        assert_eq!(num(10.0), "10");
        assert_eq!(num(2.5), "2.5");
        assert_eq!(num(2.537), "2.54");
        assert_eq!(num(0.0), "0");
        assert_eq!(num(-0.004), "0");
    }

    /// A crude XML check: every tag closes, in order, and none is left over.
    fn tags_balanced(svg: &str) -> bool {
        let mut stack: Vec<&str> = Vec::new();
        let mut rest = svg;
        while let Some(start) = rest.find('<') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('>') else {
                return false;
            };
            let tag = after[..end].trim_start();
            rest = &after[end + 1..];
            if tag.starts_with('?') || tag.starts_with('!') {
                continue;
            }
            let closing = tag.starts_with('/');
            let name = tag
                .trim_start_matches('/')
                .split([' ', '/', '\n'])
                .next()
                .unwrap_or_default();
            if name.is_empty() {
                return false;
            }
            if closing {
                if stack.pop() != Some(name) {
                    return false;
                }
            } else if !tag.ends_with('/') {
                stack.push(name);
            }
        }
        stack.is_empty()
    }

    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on; tests may block"
    )]
    async fn the_first_frame_is_well_formed_svg() {
        let buffer = crate::demo::first_frame(120, 34).await.unwrap();
        let svg = buffer_to_svg(&buffer, &poster());
        assert!(tags_balanced(&svg), "{svg}");
        assert!(
            svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1080 544""#),
            "{svg}"
        );
        assert!(svg.contains("Conversations"), "{svg}");
        assert!(svg.contains("Michael"), "{svg}");
    }

    /// `FARO_SVG_OUT=<path> cargo test -p faro-web svg` writes the poster out
    /// for a human to look at.
    #[tokio::test(start_paused = true)]
    #[expect(
        clippy::disallowed_methods,
        reason = "#[tokio::test] expands to block_on, and writing the poster to a path is a test-only convenience"
    )]
    async fn the_demo_poster_can_be_written_out() {
        let Some(path) = std::env::var_os("FARO_SVG_OUT") else {
            return;
        };
        let buffer = crate::demo::first_frame(120, 34).await.unwrap();
        std::fs::write(path, buffer_to_svg(&buffer, &poster())).unwrap();
    }
}
