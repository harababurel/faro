//! Theme checks (D50, D54): every theme loads under its name, faro's own stay
//! readable (contrast between each text role and the backgrounds it's drawn
//! on), and the colour fallbacks only use colours the terminal has.

use ratatui::style::Color;

use super::colors::{indexed_rgb, nearest_256};
use super::{ColorDepth, Theme, Themes, UserThemes, bundled};

mod files;

use files::MINIMAL;

fn rgb_themes() -> Vec<Theme> {
    bundled::faro()
        .iter()
        .filter(|t| !t.terminal_palette)
        .cloned()
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

fn contrast(a: Color, b: Color) -> f64 {
    super::colors::contrast(rgb(a), rgb(b))
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
    // Mentions (D72) are accent text, in bubbles too.
    for (name, bg) in [
        ("bubble_own", t.bubble_own),
        ("bubble_other", t.bubble_other),
    ] {
        rules.push((format!("accent on {name}"), t.accent, bg, 3.0));
    }
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

/// faro's own theme called `name`, at `depth`.
fn load(name: &str, depth: ColorDepth) -> Theme {
    Theme::find(name, &UserThemes::default())
        .cloned()
        .unwrap()
        .for_depth(depth)
}

#[test]
fn every_theme_is_found_by_its_name() {
    let user = UserThemes::default();
    let all = Theme::all(&user);
    for theme in &all {
        assert_eq!(Theme::find(&theme.name, &user), Some(*theme));
    }
    let mut names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), all.len(), "two themes share a name");
    assert_eq!(Theme::find("nope", &user), None);
    assert_eq!(Theme::default().name, faro_core::config::DEFAULT_THEME);
}

#[test]
fn the_config_help_names_every_faro_theme() {
    let help = faro_core::config::setting(faro_core::config::key::THEME)
        .unwrap()
        .help;
    for theme in bundled::faro() {
        assert!(
            help.contains(&format!("\"{}\"", theme.name)),
            "{} missing from {help:?}",
            theme.name
        );
    }
    for example in ["tokyonight", "nord", "github-light"] {
        assert!(
            Theme::find(example, &UserThemes::default()).is_some(),
            "the help names {example}, which doesn't exist"
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
        .map(|t| load(&t.name, ColorDepth::Indexed256))
        .flat_map(|t| check_contrast(&t, 0.9))
        .collect();
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn surfaces_stay_apart_in_256_colours() {
    let all: Vec<Theme> = rgb_themes()
        .into_iter()
        .chain(bundled::opencode().iter().cloned())
        .collect();
    for theme in all {
        let t = theme.clone().for_depth(ColorDepth::Indexed256);
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
    for theme in Theme::all(&UserThemes::default()) {
        let name = &theme.name;
        for (role, colour) in colours(&theme.clone().for_depth(ColorDepth::Indexed256)) {
            assert!(
                !matches!(colour, Color::Rgb(..)),
                "{name} {role} is still RGB in 256 colours"
            );
        }
        let ansi = theme.clone().for_depth(ColorDepth::Ansi16);
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
    let mut user = UserThemes::default();
    let first = themes
        .get("faro-light", ColorDepth::Indexed256, &user)
        .clone();
    assert_eq!(first, load("faro-light", ColorDepth::Indexed256));
    assert_eq!(
        themes.get("faro-light", ColorDepth::Indexed256, &user),
        &first
    );
    assert_eq!(
        themes.get("faro-light", ColorDepth::Truecolor, &user),
        &Theme::faro_light()
    );
    assert_eq!(
        themes.get("faro-light", ColorDepth::Ansi16, &user).name,
        "system"
    );
    assert_eq!(
        themes.get("unknown", ColorDepth::Truecolor, &user),
        &Theme::default()
    );
    // The user's themes changed: looked up again.
    let problems = user.load(vec![super::ThemeText {
        file: "unknown.json".to_owned(),
        text: Ok(MINIMAL.to_owned()),
    }]);
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(
        themes.get("unknown", ColorDepth::Truecolor, &user).name,
        "unknown"
    );
}

/// The example in the README and DESIGN §10: at 256 colours solarized's
/// dark blue background has no close match and becomes a neutral grey,
/// 235 (#262626), a step from Solarized's own 256-colour fallback (234).
#[test]
fn solarized_turns_grey_in_256_colours() {
    let truecolor = load("solarized", ColorDepth::Truecolor);
    assert_eq!(truecolor.background, Color::Rgb(0x00, 0x2b, 0x36));
    let approximated = load("solarized", ColorDepth::Indexed256);
    assert_eq!(approximated.background, Color::Indexed(235));
    assert_eq!(indexed_rgb(235), Some((0x26, 0x26, 0x26)));
}
