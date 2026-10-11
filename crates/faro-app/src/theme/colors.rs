//! How many colours the terminal shows, and the nearest xterm-256 colour for
//! terminals without truecolor (D50).

use ratatui::style::Color;

/// What the terminal can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorDepth {
    /// 24-bit colour: themes are drawn as designed.
    #[default]
    Truecolor,
    /// The xterm-256 palette: each colour becomes the nearest one of the
    /// 240 with fixed values (16–255).
    Indexed256,
    /// Only the 16 ANSI colours, whose values the user picks: the `system`
    /// theme.
    Ansi16,
}

/// Terminals known to show truecolor whatever `COLORTERM` says (it doesn't
/// survive `ssh`, `sudo` or a tmux server started elsewhere; `TERM` does).
const TRUECOLOR_TERMS: &[&str] = &[
    "xterm-kitty",
    "xterm-ghostty",
    "alacritty",
    "foot",
    "wezterm",
    "contour",
    "rio",
];

/// `TERM_PROGRAM` values of terminals that show truecolor.
const TRUECOLOR_PROGRAMS: &[&str] = &["iTerm.app", "WezTerm", "ghostty", "vscode", "Hyper"];

/// `TERM` values of terminals with no more than 16 colours.
const ANSI_TERMS: &[&str] = &["linux", "vt100", "vt102", "vt220", "ansi", "cons25", "dumb"];

impl ColorDepth {
    /// Guess from the environment (`var` reads a variable):
    /// 1. `COLORTERM=truecolor` or `24bit` says so.
    /// 2. Inside tmux, truecolor: tmux turns RGB colours into the nearest of
    ///    the 256 (or 16) itself when the terminal it's attached to lacks
    ///    them, and does it per client, so this is right for every client.
    /// 3. Terminals known for truecolor, by `TERM` (`*-direct` too) or
    ///    `TERM_PROGRAM`.
    /// 4. 16 colours for the Linux console, old terminal types and
    ///    `*-16color`/`*-8color`; otherwise 256, which nearly every terminal
    ///    in use today shows.
    pub fn detect(var: impl Fn(&str) -> Option<String>) -> Self {
        let colorterm = var("COLORTERM").unwrap_or_default();
        if matches!(colorterm.as_str(), "truecolor" | "24bit") || var("TMUX").is_some() {
            return Self::Truecolor;
        }
        let term = var("TERM").unwrap_or_default();
        let program = var("TERM_PROGRAM").unwrap_or_default();
        if TRUECOLOR_TERMS.contains(&term.as_str())
            || term.ends_with("-direct")
            || TRUECOLOR_PROGRAMS.contains(&program.as_str())
        {
            return Self::Truecolor;
        }
        if ANSI_TERMS.contains(&term.as_str())
            || term.ends_with("-16color")
            || term.ends_with("-8color")
        {
            return Self::Ansi16;
        }
        Self::Indexed256
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Truecolor => "truecolor",
            Self::Indexed256 => "256 colours",
            Self::Ansi16 => "16 colours",
        }
    }
}

/// The six levels of each channel in the xterm colour cube (16–231).
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The RGB value of xterm-256 colour `index`, for the 240 colours whose value
/// is fixed (16–255); `None` for the 16 the user's palette decides.
pub fn indexed_rgb(index: u8) -> Option<(u8, u8, u8)> {
    match index {
        0..=15 => None,
        16..=231 => {
            let i = usize::from(index - 16);
            let level = |n: usize| CUBE.get(n % 6).copied();
            Some((level(i / 36)?, level(i / 6)?, level(i)?))
        }
        232..=255 => {
            let grey = 8 + (index - 232) * 10;
            Some((grey, grey, grey))
        }
    }
}

/// How different two colours look: the "redmean" weighted distance, a cheap
/// approximation of perceived difference that beats plain RGB distance.
pub fn distance(a: Rgb, b: Rgb) -> i32 {
    let (r1, g1, b1) = (i32::from(a.0), i32::from(a.1), i32::from(a.2));
    let (r2, g2, b2) = (i32::from(b.0), i32::from(b.1), i32::from(b.2));
    let mean = i32::midpoint(r1, r2);
    let (dr, dg, db) = (r1 - r2, g1 - g2, b1 - b2);
    (((512 + mean) * dr * dr) >> 8) + 4 * dg * dg + (((767 - mean) * db * db) >> 8)
}

/// `colour` as the nearest fixed xterm-256 colour; anything that isn't RGB
/// stays as it is.
pub fn nearest_256(colour: Color) -> Color {
    nearest_256_except(colour, &[])
}

/// The nearest fixed xterm-256 colour that isn't in `taken` (so surfaces
/// that differ in the theme don't merge in 256 colours).
pub fn nearest_256_except(colour: Color, taken: &[Color]) -> Color {
    let Color::Rgb(r, g, b) = colour else {
        return colour;
    };
    let mut best: Option<(i32, u8)> = None;
    for index in 16..=255 {
        let Some(value) = indexed_rgb(index) else {
            continue;
        };
        if taken.contains(&Color::Indexed(index)) {
            continue;
        }
        let d = distance((r, g, b), value);
        if best.is_none_or(|(least, _)| d < least) {
            best = Some((d, index));
        }
    }
    best.map_or(colour, |(_, index)| Color::Indexed(index))
}

/// A colour as red, green and blue.
pub type Rgb = (u8, u8, u8);

/// WCAG relative luminance, 0 (black) to 1 (white).
pub fn luminance((r, g, b): Rgb) -> f64 {
    let linear = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// WCAG contrast ratio, 1 (same) to 21 (black on white).
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `permille`/1000 of the way from `from` to `to`.
pub fn blend(from: Rgb, to: Rgb, permille: u16) -> Rgb {
    let t = u32::from(permille.min(1000));
    let mix = |a: u8, b: u8| {
        let value = (u32::from(a) * (1000 - t) + u32::from(b) * t + 500) / 1000;
        u8::try_from(value).unwrap_or(u8::MAX)
    };
    (mix(from.0, to.0), mix(from.1, to.1), mix(from.2, to.2))
}
