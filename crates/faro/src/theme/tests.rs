//! Theme checks (D50): every built-in theme loads under its name, stays
//! readable (contrast between each text role and the backgrounds it's drawn
//! on), and the colour fallbacks only use colours the terminal has.

use ratatui::style::Color;

use super::colors::{indexed_rgb, nearest_256};
use super::{ColorDepth, Theme, Themes};

fn rgb_themes() -> Vec<Theme> {
    Theme::NAMES
        .iter()
        .filter_map(|name| Theme::named(name))
        .filter(|t| !t.terminal_palette)
        .collect()
}

/// Every colour of a theme, with its role's name.
fn colours(theme: &Theme) -> Vec<(String, Color)> {
    let mut out = vec![
        ("background".to_owned(), theme.background),
        ("background_panel".to_owned(), theme.background_panel),
        ("background_element".to_owned(), theme.background_element),
        ("text".to_owned(), theme.text),
        ("text_muted".to_owned(), theme.text_muted),
        ("primary".to_owned(), theme.primary),
        ("secondary".to_owned(), theme.secondary),
        ("accent".to_owned(), theme.accent),
        ("border".to_owned(), theme.border),
        ("border_active".to_owned(), theme.border_active),
        ("success".to_owned(), theme.success),
        ("warning".to_owned(), theme.warning),
        ("error".to_owned(), theme.error),
        ("unread".to_owned(), theme.unread),
        ("bubble_own".to_owned(), theme.bubble_own),
        ("bubble_other".to_owned(), theme.bubble_other),
    ];
    for (i, colour) in theme.senders.iter().enumerate() {
        out.push((format!("senders[{i}]"), *colour));
    }
    out
}

fn rgb(colour: Color) -> (u8, u8, u8) {
    match colour {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) => indexed_rgb(i).unwrap(),
        Color::Reset
        | Color::Black
        | Color::Red
        | Color::Green
        | Color::Yellow
        | Color::Blue
        | Color::Magenta
        | Color::Cyan
        | Color::Gray
        | Color::DarkGray
        | Color::LightRed
        | Color::LightGreen
        | Color::LightYellow
        | Color::LightBlue
        | Color::LightMagenta
        | Color::LightCyan
        | Color::White => panic!("{colour:?} has no fixed value"),
    }
}

/// WCAG relative luminance.
fn luminance(colour: Color) -> f64 {
    let (r, g, b) = rgb(colour);
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
fn contrast(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The contrast rules: (what, foreground, background, minimum ratio).
/// Text is ≥ 7 (WCAG AAA) everywhere it's drawn; muted text (times, hints)
/// ≥ 3.5; coloured text and the active border ≥ 3 (WCAG for large text and
/// UI parts) on the backgrounds they're drawn on; borders must be visible.
fn rules(t: &Theme) -> Vec<(String, Color, Color, f64)> {
    let mut rules = Vec::new();
    let backgrounds = [
        ("background", t.background),
        ("background_panel", t.background_panel),
        ("background_element", t.background_element),
        ("bubble_own", t.bubble_own),
        ("bubble_other", t.bubble_other),
    ];
    for (name, bg) in backgrounds {
        rules.push((format!("text on {name}"), t.text, bg, 7.0));
        rules.push((format!("text_muted on {name}"), t.text_muted, bg, 3.5));
    }
    let coloured = [
        ("primary", t.primary),
        ("secondary", t.secondary),
        ("accent", t.accent),
        ("success", t.success),
        ("warning", t.warning),
        ("error", t.error),
        ("unread", t.unread),
    ];
    for (name, fg) in coloured {
        rules.push((format!("{name} on background"), fg, t.background, 3.0));
        rules.push((
            format!("{name} on background_element"),
            fg,
            t.background_element,
            3.0,
        ));
    }
    for (i, fg) in t.senders.iter().enumerate() {
        rules.push((
            format!("senders[{i}] on background"),
            *fg,
            t.background,
            3.0,
        ));
        rules.push((
            format!("senders[{i}] on bubble_other"),
            *fg,
            t.bubble_other,
            3.0,
        ));
    }
    rules.push((
        "badge (background on unread)".to_owned(),
        t.background,
        t.unread,
        3.0,
    ));
    rules.push((
        "border_active on background".to_owned(),
        t.border_active,
        t.background,
        3.0,
    ));
    rules.push((
        "border on background".to_owned(),
        t.border,
        t.background,
        1.4,
    ));
    rules
}

fn check_contrast(theme: &Theme, slack: f64) -> Vec<String> {
    rules(theme)
        .into_iter()
        .filter_map(|(what, fg, bg, min)| {
            let ratio = contrast(fg, bg);
            (ratio < min * slack)
                .then(|| format!("{}: {what} is {ratio:.2}, below {min}", theme.name))
        })
        .collect()
}

#[test]
fn every_name_loads_that_theme() {
    for name in Theme::NAMES {
        let theme = Theme::named(name).unwrap();
        assert_eq!(theme.name, name);
        assert_eq!(Theme::load(name, ColorDepth::Truecolor), theme);
    }
    assert_eq!(Theme::named("nope"), None);
    assert_eq!(Theme::load("nope", ColorDepth::Truecolor), Theme::default());
    assert_eq!(Theme::default().name, faro_core::config::DEFAULT_THEME);
}

#[test]
fn the_config_help_names_every_theme() {
    let help = faro_core::config::setting(faro_core::config::key::THEME)
        .unwrap()
        .help;
    for name in Theme::NAMES {
        assert!(
            help.contains(&format!("\"{name}\"")),
            "{name} missing from {help:?}"
        );
    }
}

#[test]
fn every_theme_is_readable() {
    let problems: Vec<String> = rgb_themes()
        .iter()
        .flat_map(|t| check_contrast(t, 1.0))
        .collect();
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn themes_stay_readable_in_256_colours() {
    // Rounding to the palette may cost a little contrast, not much.
    let problems: Vec<String> = rgb_themes()
        .iter()
        .map(|t| Theme::load(t.name, ColorDepth::Indexed256))
        .flat_map(|t| check_contrast(&t, 0.9))
        .collect();
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn surfaces_stay_apart_in_256_colours() {
    for theme in rgb_themes() {
        let t = Theme::load(theme.name, ColorDepth::Indexed256);
        let pairs = [
            (
                "background_panel",
                t.background_panel,
                "background",
                t.background,
            ),
            (
                "background_element",
                t.background_element,
                "background",
                t.background,
            ),
            (
                "background_element",
                t.background_element,
                "background_panel",
                t.background_panel,
            ),
            ("bubble_own", t.bubble_own, "background", t.background),
            ("bubble_other", t.bubble_other, "background", t.background),
            ("bubble_own", t.bubble_own, "bubble_other", t.bubble_other),
        ];
        for (a, x, b, y) in pairs {
            assert_ne!(x, y, "{}: {a} and {b} merge", theme.name);
        }
    }
}

#[test]
fn fallbacks_use_only_what_the_terminal_has() {
    for name in Theme::NAMES {
        for (role, colour) in colours(&Theme::load(name, ColorDepth::Indexed256)) {
            assert!(
                !matches!(colour, Color::Rgb(..)),
                "{name} {role} is still RGB in 256 colours"
            );
        }
        let ansi = Theme::load(name, ColorDepth::Ansi16);
        assert_eq!(ansi.name, "system");
        for (role, colour) in colours(&ansi) {
            assert!(
                !matches!(colour, Color::Rgb(..) | Color::Indexed(_)),
                "{name} {role} is {colour:?} in 16 colours"
            );
        }
    }
}

#[test]
fn nearest_256_finds_exact_and_close_colours() {
    for index in 16..=255 {
        let (r, g, b) = indexed_rgb(index).unwrap();
        assert_eq!(nearest_256(Color::Rgb(r, g, b)), Color::Indexed(index));
    }
    assert_eq!(
        nearest_256(Color::Rgb(0xff, 0x00, 0x01)),
        Color::Indexed(196)
    );
    assert_eq!(
        nearest_256(Color::Rgb(0x10, 0x12, 0x16)),
        Color::Indexed(233)
    );
    assert_eq!(nearest_256(Color::Reset), Color::Reset);
    assert_eq!(nearest_256(Color::Yellow), Color::Yellow);
}

#[test]
fn colour_depth_detection() {
    use ColorDepth::{Ansi16, Indexed256, Truecolor};
    let detect = |vars: &[(&str, &str)]| {
        ColorDepth::detect(|key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        })
    };
    let cases: &[(&[(&str, &str)], ColorDepth)] = &[
        (
            &[("COLORTERM", "truecolor"), ("TERM", "xterm-256color")],
            Truecolor,
        ),
        (&[("COLORTERM", "24bit")], Truecolor),
        // tmux converts RGB itself where the outer terminal can't show it.
        (
            &[
                ("TMUX", "/tmp/tmux-1000/default,1,0"),
                ("TERM", "tmux-256color"),
            ],
            Truecolor,
        ),
        (&[("TMUX", "x"), ("TERM", "screen")], Truecolor),
        // kitty over ssh: COLORTERM is lost, TERM isn't.
        (&[("TERM", "xterm-kitty")], Truecolor),
        (&[("TERM", "xterm-direct")], Truecolor),
        (
            &[("TERM", "xterm-256color"), ("TERM_PROGRAM", "iTerm.app")],
            Truecolor,
        ),
        // Apple's Terminal, mosh, most terminals over ssh.
        (
            &[
                ("TERM", "xterm-256color"),
                ("TERM_PROGRAM", "Apple_Terminal"),
            ],
            Indexed256,
        ),
        (&[("TERM", "xterm-256color")], Indexed256),
        (&[("TERM", "xterm")], Indexed256),
        (&[], Indexed256),
        (&[("TERM", "linux")], Ansi16),
        (&[("TERM", "vt100")], Ansi16),
        (&[("TERM", "xterm-16color")], Ansi16),
        (&[("COLORTERM", "truecolor"), ("TERM", "linux")], Truecolor),
    ];
    for (vars, expected) in cases {
        assert_eq!(detect(vars), *expected, "{vars:?}");
    }
}

#[test]
fn themes_are_loaded_once() {
    let mut themes = Themes::default();
    let first = themes.get("faro-light", ColorDepth::Indexed256).clone();
    assert_eq!(first, Theme::load("faro-light", ColorDepth::Indexed256));
    assert_eq!(themes.get("faro-light", ColorDepth::Indexed256), &first);
    assert_eq!(
        themes.get("faro-light", ColorDepth::Truecolor),
        &Theme::faro_light()
    );
    assert_eq!(themes.get("faro-light", ColorDepth::Ansi16).name, "system");
    assert_eq!(
        themes.get("unknown", ColorDepth::Truecolor),
        &Theme::default()
    );
}
