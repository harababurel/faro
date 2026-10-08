//! The logo's pictures (D61), ported from the naming-phase prototype
//! (`tools/logos.py`, D12). A scene is a grid of cells holding half-block
//! pixels (two per cell, one above the other) over braille dots (2×4 per
//! cell); `Scene::picture` turns it into cells. The big lighthouse and the
//! wordmark use the logo's own colours (it's the brand), mapped to the
//! nearest xterm colour at 256 colours; the small lighthouse uses the
//! theme's, so it works with any palette, the terminal's own included.
//!
//! Only `▀ ▄ █`, braille and spaces are drawn: one cell wide in every
//! terminal, tmux and mosh measure (`tests::only_single_cell_symbols`).

use ratatui::style::Color;

use crate::theme::{ColorDepth, Rgb, Theme, indexed_rgb, luminance, nearest_256};

/// One cell of a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    pub symbol: char,
    pub fg: Color,
    /// `None`: the pane's own background shows.
    pub bg: Option<Color>,
}

/// A picture `width` × `height` cells, row by row; `None` is blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u16,
    pub height: u16,
    cells: Vec<Option<Glyph>>,
}

impl Picture {
    /// The cell at column `x`, row `y`.
    pub fn get(&self, x: u16, y: u16) -> Option<Glyph> {
        if x >= self.width {
            return None;
        }
        let index = usize::from(y) * usize::from(self.width) + usize::from(x);
        self.cells.get(index).copied().flatten()
    }

    /// Every cell drawn, with its position.
    pub fn glyphs(&self) -> impl Iterator<Item = (u16, u16, Glyph)> + '_ {
        (0..self.height).flat_map(move |y| {
            (0..self.width).filter_map(move |x| self.get(x, y).map(|g| (x, y, g)))
        })
    }
}

/// The big lighthouse, in cells.
pub const LIGHTHOUSE_WIDTH: u16 = 40;
pub const LIGHTHOUSE_HEIGHT: u16 = 13;
/// The small lighthouse, in cells.
pub const SMALL_WIDTH: u16 = 9;
pub const SMALL_HEIGHT: u16 = 8;

/// The wordmark's size at `scale` (each font pixel `scale` × `scale` half-
/// block pixels, letters `scale` apart).
pub const fn wordmark_size(scale: u16) -> (u16, u16) {
    (
        LETTERS * GLYPH_WIDTH * scale + (LETTERS - 1) * scale,
        (GLYPH_HEIGHT * scale).div_ceil(2),
    )
}

/// The letters of "faro", drawn with `FONT`.
const LETTERS: u16 = 4;
const GLYPH_WIDTH: u16 = 5;
const GLYPH_HEIGHT: u16 = 7;
/// F, A, R and O from the prototype's 5×7 font.
const FONT: [[&str; GLYPH_HEIGHT as usize]; LETTERS as usize] = [
    [
        "#####", "#....", "#....", "####.", "#....", "#....", "#....",
    ],
    [
        ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
    ],
    [
        "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
    ],
    [
        ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
    ],
];

/// Braille dot bits by row and column within a cell.
const BRAILLE: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Backgrounds brighter than this count as light.
const LIGHT: f64 = 0.4;

/// How the logo's colours are drawn on a theme: its background (light
/// glows fade towards it) and the terminal's colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Art {
    background: Rgb,
    light: bool,
    depth: ColorDepth,
}

/// The logo's colours on a dark and on a light background. On a light one
/// the stars would vanish (they're skipped), and the white stripes, the
/// lamp and the pale start of the wordmark's gradient are darkened.
struct Colours {
    white: Rgb,
    lamp: Rgb,
    word: [Rgb; 3],
    stars: bool,
}

const ON_DARK: Colours = Colours {
    white: (235, 235, 240),
    lamp: (255, 240, 170),
    word: [(255, 235, 170), (235, 90, 80), (120, 150, 230)],
    stars: true,
};
const ON_LIGHT: Colours = Colours {
    white: (172, 176, 188),
    lamp: (236, 160, 40),
    word: [(226, 146, 36), (210, 60, 55), (70, 100, 200)],
    stars: false,
};

const RED: Rgb = (210, 60, 60);
const ROOF: Rgb = (150, 40, 45);
const ROOF_EDGE: Rgb = (170, 45, 50);
const GALLERY: Rgb = (60, 60, 70);
const ROCK: Rgb = (95, 95, 105);
const WAVES: [Rgb; 2] = [(35, 95, 180), (60, 140, 225)];
const STAR: Rgb = (190, 190, 225);

impl Art {
    /// `None` where the logo's colours can't be drawn: the terminal's own
    /// palette (the `system` theme, 16 colours), whose values faro can't see.
    pub fn new(theme: &Theme, depth: ColorDepth) -> Option<Self> {
        if theme.terminal_palette || depth == ColorDepth::Ansi16 {
            return None;
        }
        let background = rgb(theme.background)?;
        // A theme already rounded to 256 colours gets the logo rounded too,
        // whatever `depth` says.
        let depth = if matches!(theme.background, Color::Indexed(_)) {
            ColorDepth::Indexed256
        } else {
            depth
        };
        Some(Self {
            background,
            light: luminance(background) > LIGHT,
            depth,
        })
    }

    fn colours(self) -> &'static Colours {
        if self.light { &ON_LIGHT } else { &ON_DARK }
    }

    /// `colour` as the terminal draws it.
    fn draw(self, (r, g, b): Rgb) -> Color {
        let colour = Color::Rgb(r, g, b);
        match self.depth {
            ColorDepth::Truecolor => colour,
            ColorDepth::Indexed256 | ColorDepth::Ansi16 => nearest_256(colour),
        }
    }

    /// `colour` at `k` of its strength over the background (glows).
    fn fade(self, colour: Rgb, k: f64) -> Rgb {
        lerp(self.background, colour, k)
    }

    /// The lighthouse `t` seconds into its animation: the beam turns, the
    /// lamp glows with it, the waves roll and the stars twinkle.
    pub fn lighthouse(self, t: f64) -> Picture {
        let colours = self.colours();
        let mut scene = Scene::new(LIGHTHOUSE_WIDTH, LIGHTHOUSE_HEIGHT);
        let cx = 20.0;
        scene.pixel(cx, 1.0, ROOF);
        for x in -1..=1 {
            scene.pixel(cx + f64::from(x), 2.0, ROOF);
        }
        for x in -2..=2 {
            scene.pixel(cx + f64::from(x), 3.0, ROOF_EDGE);
        }
        let theta = t * 1.4;
        let glow = 0.6 + 0.4 * theta.sin().max(0.0);
        for y in 4..=6 {
            for x in -1..=1 {
                let lamp = self.fade(colours.lamp, 0.55 + 0.45 * glow);
                scene.pixel(cx + f64::from(x), f64::from(y), lamp);
            }
        }
        for x in -3..=3 {
            scene.pixel(cx + f64::from(x), 7.0, GALLERY);
        }
        tower(&mut scene, cx, colours.white);
        for (y, half) in [(23, 7), (24, 9), (25, 11)] {
            for x in -half..=half {
                let x = 20 + x;
                let k = 0.6 + 0.4 * f64::from((x * 7 + y) % 3) / 2.0;
                scene.pixel(f64::from(x), f64::from(y), dim(ROCK, k));
            }
        }
        self.beam(&mut scene, cx, theta);
        scene.waves(t, 44.0, &WAVES);
        if colours.stars {
            self.stars(&mut scene, t);
        }
        scene.picture(|c| self.draw(c))
    }

    /// The beam: a fan of braille lines from the lamp, as long as the turn
    /// makes it (it points at the viewer, then away).
    fn beam(self, scene: &mut Scene, cx: f64, theta: f64) {
        let (bx, by) = (2.0 * cx + 1.0, 10.0);
        let length = theta.cos() * 38.0;
        if length.abs() <= 1.0 {
            return;
        }
        let spread = 2.0 + length.abs() / 7.0;
        let lamp = self.colours().lamp;
        let strength = 0.45 + 0.55 * theta.sin().max(0.0);
        let reach = floor(spread).unwrap_or(0);
        for s in -reach..=reach {
            let frac = f64::from(s.abs()) / (spread + 1.0);
            let near = self.fade(lamp, strength * (1.0 - 0.6 * frac));
            let far = self.fade(lamp, strength * 0.2);
            scene.line(
                (bx, by),
                (bx + length, by + f64::from(s)),
                (near, far),
                0.3 - frac * 0.1,
            );
        }
    }

    /// Fourteen stars in the sky, each twinkling at its own pace.
    fn stars(self, scene: &mut Scene, t: f64) {
        let mut random = SplitMix(7);
        for _ in 0..14 {
            let (x, y) = (random.uniform(0.0, 80.0), random.uniform(0.0, 22.0));
            let (phase, speed) = (random.uniform(0.0, 6.3), random.uniform(1.0, 3.0));
            let k = 0.25 + 0.75 * (0.5 + 0.5 * (t * speed + phase).sin());
            scene.dot(x, y, self.fade(STAR, k), 0.05);
        }
    }

    /// "faro" in the block font at `scale`, in the logo's gradient.
    pub fn wordmark(self, scale: u16) -> Picture {
        let (width, height) = wordmark_size(scale);
        let stops = &self.colours().word;
        let mut scene = Scene::new(width, height);
        let pixel_height = f64::from(GLYPH_HEIGHT * scale);
        let last = f64::from(width.saturating_sub(1).max(1));
        let mut left = 0;
        for glyph in &FONT {
            for (gy, row) in (0..).zip(glyph) {
                for (gx, _) in (0..).zip(row.chars()).filter(|(_, c)| *c == '#') {
                    for sx in 0..scale {
                        for sy in 0..scale {
                            let x = f64::from(left + gx * scale + sx);
                            let y = f64::from(gy * scale + sy);
                            let colour = gradient(stops, x / last);
                            scene.pixel(x, y, dim(colour, 1.0 - 0.3 * y / pixel_height));
                        }
                    }
                }
            }
            left += GLYPH_WIDTH * scale + scale;
        }
        scene.picture(|c| self.draw(c))
    }
}

/// The striped tower, wider towards the bottom and shaded on the right.
fn tower(scene: &mut Scene, cx: f64, white: Rgb) {
    for y in 8..23 {
        let half = 2.0 + f64::from(y - 8) * 2.2 / 14.0;
        let stripe = if ((y - 8) / 3) % 2 == 0 { RED } else { white };
        let (Some(from), Some(to)) = (round(cx - half), round(cx + half)) else {
            continue;
        };
        for x in from..=to {
            let shade = 1.0 - 0.35 * ((f64::from(x) - cx) / half.max(1.0)).max(0.0);
            scene.pixel(f64::from(x), f64::from(y), dim(stripe, shade));
        }
    }
}

/// What each row of the small lighthouse is drawn with.
#[derive(Debug, Clone, Copy)]
enum Part {
    Red,
    White,
    Lamp,
    Stone,
}

/// The small lighthouse: every cell one colour (no cell needs two, so the
/// terminal's default colours work too).
const SMALL: [(&str, Part); SMALL_HEIGHT as usize] = [
    ("   ▄█▄   ", Part::Red),
    ("   ███   ", Part::Lamp),
    ("  ▀▀▀▀▀  ", Part::Stone),
    ("   ███   ", Part::Red),
    ("   ███   ", Part::White),
    ("  █████  ", Part::Red),
    ("  █████  ", Part::White),
    ("▄▄▄▄▄▄▄▄▄", Part::Stone),
];

/// The small lighthouse in `theme`'s colours: red and white are its error
/// and text colours, the lamp its warning colour, stone its border colour.
pub fn small(theme: &Theme) -> Picture {
    let mut cells = Vec::with_capacity(usize::from(SMALL_WIDTH * SMALL_HEIGHT));
    for (row, part) in SMALL {
        let fg = match part {
            Part::Red => theme.error,
            Part::White => theme.text,
            Part::Lamp => theme.warning,
            Part::Stone => theme.border,
        };
        cells.extend(row.chars().map(|symbol| {
            (symbol != ' ').then_some(Glyph {
                symbol,
                fg,
                bg: None,
            })
        }));
    }
    Picture {
        width: SMALL_WIDTH,
        height: SMALL_HEIGHT,
        cells,
    }
}

/// Half-block pixels over braille dots (the prototype's `Scene`).
struct Scene {
    width: i32,
    height: i32,
    /// `width` × `2 * height`.
    pixels: Vec<Option<Rgb>>,
    /// `2 * width` × `4 * height`, each with a priority: where dots of two
    /// colours share a cell, the higher one's colour is used.
    dots: Vec<Option<(Rgb, f64)>>,
}

impl Scene {
    fn new(width: u16, height: u16) -> Self {
        let cells = usize::from(width) * usize::from(height);
        Self {
            width: i32::from(width),
            height: i32::from(height),
            pixels: vec![None; cells * 2],
            dots: vec![None; cells * 8],
        }
    }

    fn pixel(&mut self, x: f64, y: f64, colour: Rgb) {
        if let Some(pixel) =
            index(x, y, self.width, 2 * self.height).and_then(|i| self.pixels.get_mut(i))
        {
            *pixel = Some(colour);
        }
    }

    fn dot(&mut self, x: f64, y: f64, colour: Rgb, priority: f64) {
        if let Some(dot) =
            index(x, y, 2 * self.width, 4 * self.height).and_then(|i| self.dots.get_mut(i))
            && dot.is_none_or(|(_, old)| priority >= old)
        {
            *dot = Some((colour, priority));
        }
    }

    /// A line of dots from `from` to `to`, its colour going from the first
    /// of `colours` to the second.
    fn line(&mut self, from: (f64, f64), to: (f64, f64), colours: (Rgb, Rgb), priority: f64) {
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let steps = floor(dx.abs().max(dy.abs())).unwrap_or(0) + 1;
        for k in 0..=steps {
            let t = f64::from(k) / f64::from(steps);
            let colour = lerp(colours.0, colours.1, t);
            self.dot(from.0 + dx * t, from.1 + dy * t, colour, priority);
        }
    }

    /// Rows of dots rolling along a sine, `base` dots from the top.
    fn waves(&mut self, t: f64, base: f64, colours: &[Rgb]) {
        let (amplitude, frequency, speed, step) = (1.2, 0.18, 2.0, 3.0);
        for (j, colour) in (0..).zip(colours) {
            let j = f64::from(j);
            for x in 0..2 * self.width {
                let x = f64::from(x);
                let phase = x * frequency + t * speed * (1.0 + 0.3 * j) + j * 1.7;
                self.dot(x, base + j * step + amplitude * phase.sin(), *colour, 0.5);
            }
        }
    }

    /// The cells: two pixels make `█` (same colour) or `▀` (fg on top, bg
    /// below), one makes `▀` or `▄` over the pane's background; cells
    /// without pixels show their dots in braille.
    fn picture(&self, draw: impl Fn(Rgb) -> Color) -> Picture {
        let mut cells = Vec::with_capacity(self.pixels.len() / 2);
        for cy in 0..self.height {
            for cx in 0..self.width {
                cells.push(self.cell(cx, cy, &draw));
            }
        }
        Picture {
            width: u16::try_from(self.width).unwrap_or(0),
            height: u16::try_from(self.height).unwrap_or(0),
            cells,
        }
    }

    fn cell(&self, cx: i32, cy: i32, draw: &impl Fn(Rgb) -> Color) -> Option<Glyph> {
        let pixel = |y: i32| {
            index(f64::from(cx), f64::from(y), self.width, 2 * self.height)
                .and_then(|i| self.pixels.get(i).copied().flatten())
                .map(draw)
        };
        let glyph = |symbol, fg, bg| Some(Glyph { symbol, fg, bg });
        match (pixel(2 * cy), pixel(2 * cy + 1)) {
            (Some(top), Some(bottom)) if top == bottom => glyph('█', top, None),
            (Some(top), Some(bottom)) => glyph('▀', top, Some(bottom)),
            (Some(top), None) => glyph('▀', top, None),
            (None, Some(bottom)) => glyph('▄', bottom, None),
            (None, None) => self.braille(cx, cy).map(|(symbol, colour)| Glyph {
                symbol,
                fg: draw(colour),
                bg: None,
            }),
        }
    }

    /// The cell's dots as a braille character, in the colour of the one
    /// with the highest priority.
    fn braille(&self, cx: i32, cy: i32) -> Option<(char, Rgb)> {
        let mut code = 0u32;
        let mut best: Option<(Rgb, f64)> = None;
        for (dy, bits) in (0..).zip(BRAILLE) {
            for (dx, bit) in (0..).zip(bits) {
                let (x, y) = (f64::from(2 * cx + dx), f64::from(4 * cy + dy));
                let Some((colour, priority)) = index(x, y, 2 * self.width, 4 * self.height)
                    .and_then(|i| self.dots.get(i).copied().flatten())
                else {
                    continue;
                };
                code |= u32::from(bit);
                if best.is_none_or(|(_, top)| priority > top) {
                    best = Some((colour, priority));
                }
            }
        }
        let (colour, _) = best?;
        Some((char::from_u32(0x2800 + code)?, colour))
    }
}

/// The position in a `width` × `height` grid nearest to (`x`, `y`).
fn index(x: f64, y: f64, width: i32, height: i32) -> Option<usize> {
    let (x, y) = (round(x)?, round(y)?);
    ((0..width).contains(&x) && (0..height).contains(&y))
        .then(|| usize::try_from(y * width + x).ok())
        .flatten()
}

/// `value` rounded, if it's a sensible coordinate.
fn round(value: f64) -> Option<i32> {
    to_int(value.round())
}

/// `value` rounded down, if it's a sensible coordinate.
fn floor(value: f64) -> Option<i32> {
    to_int(value.floor())
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "a whole number, range-checked first"
)]
fn to_int(whole: f64) -> Option<i32> {
    (whole.is_finite() && whole.abs() < 1e6).then_some(whole as i32)
}

/// A colour channel from a computed value.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 0–255 first; the fraction is dropped as in the prototype"
)]
fn channel(value: f64) -> u8 {
    value.clamp(0.0, 255.0) as u8
}

/// `t` of the way from `a` to `b`.
fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| channel(f64::from(a) + (f64::from(b) - f64::from(a)) * t);
    (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

/// `colour` darkened to `k` of its brightness (shading).
fn dim(colour: Rgb, k: f64) -> Rgb {
    lerp((0, 0, 0), colour, k)
}

/// The colour `t` of the way along `stops`.
fn gradient(stops: &[Rgb], t: f64) -> Rgb {
    let segments = stops.len().saturating_sub(1).max(1);
    let position = t.clamp(0.0, 1.0) * f64::from(u32::try_from(segments).unwrap_or(1));
    let i = floor(position)
        .and_then(|i| usize::try_from(i).ok())
        .unwrap_or(0)
        .min(segments - 1);
    let from = stops.get(i).copied().unwrap_or_default();
    let to = stops.get(i + 1).copied().unwrap_or(from);
    lerp(
        from,
        to,
        position - f64::from(u32::try_from(i).unwrap_or(0)),
    )
}

/// `colour` as red, green and blue, where it has fixed values.
fn rgb(colour: Color) -> Option<Rgb> {
    if let Color::Rgb(r, g, b) = colour {
        Some((r, g, b))
    } else if let Color::Indexed(index) = colour {
        indexed_rgb(index)
    } else {
        None
    }
}

/// A small deterministic random source (`SplitMix64`) for the stars: the
/// same sky every frame and every run.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Between `low` and `high`.
    fn uniform(&mut self, low: f64, high: f64) -> f64 {
        // The top 32 bits, as a fraction of 2^32.
        let fraction = f64::from(u32::try_from(self.next() >> 32).unwrap_or(0)) / 4_294_967_296.0;
        low + (high - low) * fraction
    }
}
