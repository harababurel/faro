//! Theme files (OpenCode's format, D54) turned into faro themes. OpenCode's
//! roles map onto faro's by name; what faro has and OpenCode doesn't (the
//! bubbles, the unread badge, sender names) is derived from the file's
//! colours, adjusted until it's readable. The file's own colours are kept
//! as they are, except coloured and muted text that's nearly invisible on
//! the background (`FLOOR`).

use faro_core::theme_file::{Mode, Rgba, ThemeError, ThemeFile};
use ratatui::style::Color;

use super::colors::{Rgb, blend, contrast, distance};
use super::{Source, Theme};

/// Where sender colours come from, in order of preference: the file's
/// status and accent colours, then its syntax and markdown colours.
const SENDER_ROLES: [&str; 17] = [
    "error",
    "success",
    "warning",
    "secondary",
    "accent",
    "info",
    "primary",
    "syntaxKeyword",
    "syntaxString",
    "syntaxFunction",
    "syntaxType",
    "syntaxNumber",
    "syntaxOperator",
    "markdownHeading",
    "markdownLink",
    "diffAdded",
    "diffRemoved",
];

/// The least contrast on the background for coloured and muted text taken
/// from a file (faro's own themes aim higher: D51).
pub const FLOOR: f64 = 3.0;

/// Sender colours closer than this (`colors::distance`) count as the same.
const SAME_SENDER: i32 = 2000;

/// Colours with less than this between their strongest and weakest channel
/// are greys: as sender names they'd read as muted text.
const GREY: u8 = 24;

/// The themes in a file: its dark variant called `name`, and its light one
/// called `name-light` if that looks different.
pub fn variants(name: &str, file: &ThemeFile, source: Source) -> Result<Vec<Theme>, ThemeError> {
    let dark = import(name.to_owned(), file, Mode::Dark, source)?;
    let light = match import(format!("{name}-light"), file, Mode::Light, source) {
        Ok(light) => light,
        Err(error) => {
            tracing::debug!(name, %error, "no light variant");
            return Ok(vec![dark]);
        }
    };
    let same = Theme {
        name: dark.name.clone(),
        ..light.clone()
    } == dark;
    Ok(if same { vec![dark] } else { vec![dark, light] })
}

/// One variant of a theme file. `background`, `text` and `primary` are
/// needed; every other role has a fallback.
pub fn import(
    name: String,
    file: &ThemeFile,
    mode: Mode,
    source: Source,
) -> Result<Theme, ThemeError> {
    // Transparent backgrounds show the terminal's; faro can't, so they're
    // drawn over black (dark) or white (light).
    let base = match mode {
        Mode::Dark => (0x00, 0x00, 0x00),
        Mode::Light => (0xff, 0xff, 0xff),
    };
    let background = over(
        required(file.colour("background", mode)?, "background")?,
        base,
    );
    let role = |role: &str| -> Result<Option<Rgb>, ThemeError> {
        Ok(file.colour(role, mode)?.map(|c| over(c, background)))
    };
    let text = required(role("text")?, "text")?;
    // Coloured text and muted text keep the file's colour unless it's
    // nearly invisible on the background; then it's moved towards the
    // text colour just enough.
    let coloured = |colour: Rgb| readable(colour, text, &[background], FLOOR);
    let primary = coloured(required(role("primary")?, "primary")?);
    let panel = role("backgroundPanel")?.unwrap_or(background);
    let panel = apart(panel, &[background], blend(background, text, 50));
    let element = role("backgroundElement")?.unwrap_or(panel);
    let element = apart(element, &[background, panel], blend(background, text, 100));
    let text_muted = coloured(role("textMuted")?.unwrap_or_else(|| blend(text, background, 450)));
    let secondary = role("secondary")?.map_or(primary, coloured);
    let accent = role("accent")?.map_or(secondary, coloured);
    let info = role("info")?.unwrap_or(secondary);
    let bubble_other = apart(
        blend(panel, element, 500),
        &[background],
        blend(background, text, 70),
    );
    let bubble_own = bubble_own(bubble_other, primary, text, text_muted);
    let senders = senders(file, mode, background, bubble_other, text);
    Ok(Theme {
        name,
        source,
        background: rgb(background),
        background_panel: rgb(panel),
        background_element: rgb(element),
        text: rgb(text),
        text_muted: rgb(text_muted),
        primary: rgb(primary),
        secondary: rgb(secondary),
        accent: rgb(accent),
        border: rgb(role("border")?.unwrap_or(text_muted)),
        border_active: rgb(role("borderActive")?.map_or(primary, coloured)),
        success: rgb(role("success")?.map_or(primary, coloured)),
        warning: rgb(role("warning")?.map_or(primary, coloured)),
        error: rgb(role("error")?.map_or(primary, coloured)),
        unread: rgb(readable(info, text, &[background, element], 3.0)),
        bubble_own: rgb(bubble_own),
        bubble_other: rgb(bubble_other),
        senders: senders.map(rgb),
        terminal_palette: false,
    })
}

fn required<T>(colour: Option<T>, role: &str) -> Result<T, ThemeError> {
    colour.ok_or_else(|| ThemeError::Role {
        role: role.to_owned(),
        problem: "not set (faro needs it)".to_owned(),
    })
}

const fn rgb((r, g, b): Rgb) -> Color {
    Color::Rgb(r, g, b)
}

/// `colour` drawn over `under` (its opacity mixes them).
fn over(colour: Rgba, under: Rgb) -> Rgb {
    let opacity = u16::try_from(u32::from(colour.a) * 1000 / 255).unwrap_or(1000);
    blend(under, (colour.r, colour.g, colour.b), opacity)
}

/// `colour`, or `instead` if it's one of `taken` (a surface faro draws next
/// to the others must differ from them).
fn apart(colour: Rgb, taken: &[Rgb], instead: Rgb) -> Rgb {
    if taken.contains(&colour) {
        instead
    } else {
        colour
    }
}

/// `colour` moved towards `toward` (the text colour) just enough to reach
/// `min` contrast on every one of `backgrounds`.
fn readable(colour: Rgb, toward: Rgb, backgrounds: &[Rgb], min: f64) -> Rgb {
    (0..=1000)
        .step_by(50)
        .map(|t| blend(colour, toward, t))
        .find(|c| backgrounds.iter().all(|bg| contrast(*c, *bg) >= min))
        .unwrap_or(toward)
}

/// Own bubbles: the others' bubble tinted with the primary colour, as much
/// as text and muted text on it stay about as readable as on the others'.
fn bubble_own(other: Rgb, primary: Rgb, text: Rgb, muted: Rgb) -> Rgb {
    let text_min = (contrast(text, other) * 0.9).min(7.0);
    let muted_min = (contrast(muted, other) * 0.9).min(3.5);
    (20..=150)
        .rev()
        .step_by(10)
        .map(|t| blend(other, primary, t))
        .find(|c| contrast(text, *c) >= text_min && contrast(muted, *c) >= muted_min)
        .unwrap_or_else(|| blend(other, primary, 20))
}

/// Eight sender colours from the file's coloured roles: no greys, none too
/// alike, each readable on the background and on others' bubbles. Too few
/// distinct colours repeat.
fn senders(file: &ThemeFile, mode: Mode, background: Rgb, bubble: Rgb, text: Rgb) -> [Rgb; 8] {
    let mut picked: Vec<Rgb> = Vec::new();
    for role in SENDER_ROLES {
        // Roles faro uses only here: a mistake in one isn't worth a warning.
        let Ok(Some(colour)) = file.colour(role, mode) else {
            continue;
        };
        let colour = over(colour, background);
        if chroma(colour) < GREY {
            continue;
        }
        let colour = readable(colour, text, &[background, bubble], 3.0);
        if picked.iter().all(|p| distance(*p, colour) >= SAME_SENDER) {
            picked.push(colour);
        }
        if picked.len() == 8 {
            break;
        }
    }
    if picked.is_empty() {
        picked.push(text);
    }
    std::array::from_fn(|i| picked.get(i % picked.len()).copied().unwrap_or(text))
}

/// How colourful: the spread between the strongest and weakest channel.
fn chroma((r, g, b): Rgb) -> u8 {
    r.max(g).max(b) - r.min(g).min(b)
}
