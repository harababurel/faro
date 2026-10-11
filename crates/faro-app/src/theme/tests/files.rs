//! Theme files (D54): OpenCode's bundled themes all import and their derived
//! colours are readable; user files are named, layered and reported.

use faro_core::theme_file::{self, Mode};
use ratatui::style::Color;

use super::super::{Source, Theme, ThemeText, UserThemes, bundled, import};
use super::contrast;

/// The least a theme file needs.
pub const MINIMAL: &str =
    r##"{ "theme": { "background": "#101010", "text": "#e0e0e0", "primary": "#e0a040" } }"##;

fn text(file: &str, text: &str) -> ThemeText {
    ThemeText {
        file: file.to_owned(),
        text: Ok(text.to_owned()),
    }
}

fn names(user: &UserThemes) -> Vec<&str> {
    user.themes.iter().map(|t| t.name.as_str()).collect()
}

#[test]
fn every_opencode_theme_imports() {
    let dark = bundled::opencode()
        .iter()
        .filter(|t| !t.name.ends_with("-light"))
        .count();
    assert_eq!(dark, 33, "a bundled theme didn't import (see the log)");
    assert!(
        bundled::opencode()
            .iter()
            .all(|t| t.source == Source::OpenCode)
    );
    assert!(
        bundled::opencode().len() > dark,
        "the light variants are missing"
    );
}

/// What faro adds to a theme file must be readable however the file's own
/// colours turn out: the unread badge, sender names, and text on the
/// bubbles about as readable as on the others'.
#[test]
fn derived_colours_are_readable() {
    let mut problems = Vec::new();
    for t in bundled::opencode() {
        let mut check = |what: &str, ratio: f64, min: f64| {
            if ratio < min {
                problems.push(format!("{}: {what} is {ratio:.2}, below {min:.2}", t.name));
            }
        };
        check(
            "unread on background",
            contrast(t.unread, t.background),
            3.0,
        );
        check(
            "unread on background_element",
            contrast(t.unread, t.background_element),
            3.0,
        );
        for (i, sender) in t.senders.iter().enumerate() {
            check(
                &format!("senders[{i}] on background"),
                contrast(*sender, t.background),
                3.0,
            );
            check(
                &format!("senders[{i}] on bubble_other"),
                contrast(*sender, t.bubble_other),
                3.0,
            );
        }
        let text_on_other = contrast(t.text, t.bubble_other);
        check(
            "text on bubble_own",
            contrast(t.text, t.bubble_own),
            (text_on_other * 0.9).min(7.0),
        );
    }
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn senders_differ() {
    let few: Vec<String> = bundled::opencode()
        .iter()
        .filter_map(|t| {
            let mut distinct = t.senders.to_vec();
            distinct.sort_by_key(|c| format!("{c:?}"));
            distinct.dedup();
            (distinct.len() < 5).then(|| format!("{}: {}", t.name, distinct.len()))
        })
        .collect();
    // Vesper has three colours in all; every other theme has more.
    assert_eq!(few, vec!["vesper: 3", "vesper-light: 3"]);
}

/// Coloured and muted text from a file is at least faintly readable.
#[test]
fn file_colours_meet_the_floor() {
    let mut problems = Vec::new();
    for t in bundled::opencode() {
        let roles = [
            ("text_muted", t.text_muted),
            ("primary", t.primary),
            ("secondary", t.secondary),
            ("accent", t.accent),
            ("border_active", t.border_active),
            ("success", t.success),
            ("warning", t.warning),
            ("error", t.error),
        ];
        for (role, colour) in roles {
            let ratio = contrast(colour, t.background);
            if ratio < import::FLOOR {
                problems.push(format!("{}: {role} is {ratio:.2}", t.name));
            }
        }
    }
    assert_eq!(problems, Vec::<String>::new());
}

#[test]
fn missing_roles_fall_back() {
    let file = theme_file::parse(MINIMAL).unwrap();
    let t = import::import("min".to_owned(), &file, Mode::Dark, Source::User).unwrap();
    assert_eq!(t.background, Color::Rgb(0x10, 0x10, 0x10));
    assert_eq!(t.secondary, t.primary);
    assert_eq!(t.border_active, t.primary);
    assert_ne!(t.background_panel, t.background);
    assert_ne!(t.background_element, t.background_panel);
    assert_ne!(t.bubble_other, t.background);
    assert_ne!(t.bubble_own, t.bubble_other);
    // Plain colours: dark and light are the same, so there's one theme.
    let variants = import::variants("min", &file, Source::User).unwrap();
    assert_eq!(variants.len(), 1);
}

#[test]
fn a_theme_needs_background_text_and_primary() {
    let file =
        theme_file::parse(r##"{ "theme": { "background": "#000", "text": "#fff" } }"##).unwrap();
    let error = import::import("x".to_owned(), &file, Mode::Dark, Source::User).unwrap_err();
    assert_eq!(error.to_string(), "primary: not set (faro needs it)");
}

#[test]
fn transparency_is_drawn_over_the_background() {
    let file = theme_file::parse(
        r##"{ "theme": { "background": "transparent", "text": "#fff", "primary": "#ffff0080",
                        "backgroundPanel": "#ffffff20" } }"##,
    )
    .unwrap();
    let dark = import::import("x".to_owned(), &file, Mode::Dark, Source::User).unwrap();
    assert_eq!(dark.background, Color::Rgb(0, 0, 0));
    assert_eq!(dark.primary, Color::Rgb(0x80, 0x80, 0));
    assert_eq!(dark.background_panel, Color::Rgb(0x20, 0x20, 0x20));
    let light = import::import("x".to_owned(), &file, Mode::Light, Source::User).unwrap();
    assert_eq!(light.background, Color::Rgb(0xff, 0xff, 0xff));
}

#[test]
fn user_files_name_their_themes() {
    let two_modes = r##"{ "theme": { "background": { "dark": "#000", "light": "#fff" },
                                     "text": { "dark": "#fff", "light": "#000" },
                                     "primary": "#c08000" } }"##;
    let mut user = UserThemes::default();
    let problems = user.load(vec![
        text("mine.json", two_modes),
        text("plain.json", MINIMAL),
        // An explicit file beats the generated light variant of `other`.
        text("other.json", two_modes),
        text("other-light.json", MINIMAL),
        text("notes.txt", "ignored"),
    ]);
    assert_eq!(problems, Vec::<String>::new());
    assert_eq!(
        names(&user),
        vec!["mine", "mine-light", "other", "other-light", "plain"]
    );
    let explicit = Theme::find("other-light", &user).unwrap();
    assert_eq!(explicit.background, Color::Rgb(0x10, 0x10, 0x10));
    assert!(user.themes.iter().all(|t| t.source == Source::User));
    assert_eq!(user.generation, 1);
}

#[test]
fn broken_user_files_are_reported_and_skipped() {
    let mut user = UserThemes::default();
    let problems = user.load(vec![
        text("broken.json", "{"),
        text("system.json", MINIMAL),
        text("partial.json", r##"{ "theme": { "background": "#000" } }"##),
        ThemeText {
            file: "unreadable.json".to_owned(),
            text: Err("couldn't read it: permission denied".to_owned()),
        },
        text("good.json", MINIMAL),
    ]);
    assert_eq!(names(&user), vec!["good"]);
    assert_eq!(problems.len(), 4, "{problems:?}");
    assert!(
        problems[0].starts_with("broken.json: not valid JSON"),
        "{problems:?}"
    );
    assert_eq!(problems[1], "partial.json: text: not set (faro needs it)");
    assert_eq!(
        problems[2],
        "system.json: the system theme can't be replaced; rename the file"
    );
    assert_eq!(
        problems[3],
        "unreadable.json: couldn't read it: permission denied"
    );
}

#[test]
fn user_themes_replace_built_in_ones_except_system() {
    let mut user = UserThemes::default();
    user.load(vec![
        text("nord.json", MINIMAL),
        text("faro-dark.json", MINIMAL),
    ]);
    for name in ["nord", "faro-dark"] {
        assert_eq!(Theme::find(name, &user).unwrap().source, Source::User);
    }
    let all = Theme::all(&user);
    let listed = |name: &str| all.iter().filter(|t| t.name == name).count();
    assert_eq!(listed("nord"), 1);
    assert_eq!(listed("faro-dark"), 1);
    assert_eq!(listed("system"), 1);
    // faro's, then the user's, then OpenCode's.
    let order: Vec<Source> = all.iter().map(|t| t.source).collect();
    let mut sorted = order.clone();
    sorted.sort_by_key(|s| match s {
        Source::Faro => 0,
        Source::User => 1,
        Source::OpenCode => 2,
    });
    assert_eq!(order, sorted);
}
