//! The phone frames (WEBSITE.md §4.7, M13 4.5a): what a visitor on a phone
//! sees instead of the live demo — the open conversation at about 44 columns
//! (also the future mobile poster), the command palette, and the theme
//! picker, as three swipeable stills.
//!
//! Like the desktop poster these are real faro buffers, not hand-drawn
//! screens: the same `demo::first_state` the poster and the live demo start
//! from, opened further through the app's own reducer (`update::open_palette`
//! — the palette is opened the way the app opens it, not by drawing one),
//! drawn by `demo::draw` at the phone grid, and exported by the same
//! `faro_web::svg` writer. `demo::draw` pins the clock to the deterministic
//! epoch in UTC and draws the still logo frame, so every frame is
//! byte-identical across runs and days, exactly like the desktop poster.
//!
//! Each kind is opened *per scheme*, after the scheme's theme is set: the
//! theme picker marks the current theme and previews it when it opens
//! (`palette::themes` and `Palette::previewed_theme` feed `State::theme_name`,
//! which is what `demo::draw` resolves), so a picker opened under one scheme
//! and recoloured for the other would mark and draw the wrong theme.
//!
//! 44 columns is deliberate: below `faro_app::ui::LEFT_MIN_TERMINAL` (70)
//! the sidebars hide themselves, so the conversation pane fills the frame
//! and reads at phone width. The palette needs at least 20 columns
//! (`faro_app::ui::palette`), so it fits too.
//!
//! The SVGs embed no fonts and fetch none: an external SVG loaded as an
//! image is an isolated document and can't use the page's webfont, so
//! [`phone_font`] outlines the Latin text with the bundled subsets and
//! keeps only the emoji as text for the browser's system emoji font.

use faro_app::app::palette::Level;
use faro_app::app::state::State;
use faro_app::app::update::open_palette;
use faro_app::theme::Theme;
use faro_web::demo;
use faro_web::svg;

use super::{named, phone_font, svg_options};

/// The phone grid: one readable pane at about 44 columns (§4.7), tall
/// enough for the newest messages, the input box and the footer.
const COLS: u16 = 44;
const ROWS: u16 = 24;

/// The three frames, in the order the visitor swipes them.
const KINDS: [Kind; 3] = [Kind::Conversation, Kind::Palette, Kind::Themes];

/// The page's schemes and the theme each one uses, as for the posters.
const SCHEMES: [(&str, &str); 2] = [("dark", "faro-dark"), ("light", "faro-light")];

/// Which still this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The open conversation: also the future mobile poster.
    Conversation,
    /// The command palette at its root of categories.
    Palette,
    /// The theme picker under the palette.
    Themes,
}

impl Kind {
    /// The file stem, and the SVG title's id suffix, for this frame.
    const fn stem(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Palette => "palette",
            Self::Themes => "themes",
        }
    }

    /// What the frame shows, for screen readers (the SVG `<title>`). These
    /// are narrow-terminal screenshots of faro, not a mobile client.
    const fn title(self) -> &'static str {
        match self {
            Self::Conversation => {
                "a faro terminal conversation, narrow: the Scranton Branch \
                                     group chat open, its newest messages above an input box"
            }
            Self::Palette => {
                "faro's command palette open in a narrow terminal over the Scranton \
                              Branch chat, listing the categories Chats, Themes, Settings, \
                              Commands, Keys and Search"
            }
            Self::Themes => {
                "faro's theme picker open in a narrow terminal, listing faro's themes \
                             with the current one selected"
            }
        }
    }
}

/// One phone frame: a real faro buffer at the phone grid as a standalone
/// SVG document, plus the site-relative name the style sheet refers to it
/// by (`site::generate` hashes it like every other asset before rewriting
/// the CSS).
#[derive(Debug, Clone)]
pub struct Frame {
    /// The pre-hash site-relative name, as the template's CSS spells it.
    pub name: String,
    pub svg: String,
}

/// The six frames — three kinds, two schemes — from the poster's own state.
/// `state` is the desktop poster's state, already carrying the branch chat
/// open at the bottom; its theme and palette are restored before returning,
/// error or not, so generating frames is a read on the poster's state.
/// Each scheme opens its own palette under its own theme, so the picker's
/// current marker, preview and colours all agree.
pub(super) fn phones(state: &mut State) -> Result<Vec<Frame>, String> {
    let theme = state.config.theme.clone();
    let palette = state.palette.take();
    let frames = generate(state);
    theme.clone_into(&mut state.config.theme);
    state.palette = palette;
    frames
}

/// The frames themselves; [`phones`] wraps this so the caller's state is
/// always given back as it came.
fn generate(state: &mut State) -> Result<Vec<Frame>, String> {
    let mut frames = Vec::new();
    for kind in KINDS {
        for (scheme, theme_name) in SCHEMES {
            show(state, kind, theme_name);
            frames.push(frame(state, kind, scheme, theme_name)?);
        }
    }
    Ok(frames)
}

/// Put `state` into the picture for one kind under one theme: the theme set
/// through the config the way the app does, then the palette opened (or
/// closed) through the reducer.
fn show(state: &mut State, kind: Kind, theme_name: &str) {
    theme_name.clone_into(&mut state.config.theme);
    state.palette = None;
    match kind {
        // first_state already has the branch open with its history.
        Kind::Conversation => {}
        Kind::Palette => open_palette(state, Level::Root),
        Kind::Themes => open_palette(state, Level::Themes),
    }
}

/// `state` (already positioned by [`show`]) as one phone frame: drawn at the
/// phone grid and exported with the posters' options.
fn frame(state: &State, kind: Kind, scheme: &str, theme_name: &str) -> Result<Frame, String> {
    let buffer = demo::draw(state, COLS, ROWS);
    let theme = Theme::find(theme_name, &state.user_themes)
        .ok_or_else(|| format!("the built-in theme {theme_name} is missing"))?;
    let variant = format!("{}-{scheme}", kind.stem());
    let svg = named(
        &svg::buffer_to_svg(&buffer, &svg_options(theme, kind.title())),
        &variant,
    );
    let svg = phone_font::outlined(&svg, &variant)?;
    Ok(Frame {
        name: format!("phone/{variant}.svg"),
        svg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene;

    /// The names the site writes the frames under, in swipe order.
    const NAMES: [&str; 6] = [
        "phone/conversation-dark.svg",
        "phone/conversation-light.svg",
        "phone/palette-dark.svg",
        "phone/palette-light.svg",
        "phone/themes-dark.svg",
        "phone/themes-light.svg",
    ];

    fn svg_named<'a>(frames: &'a [Frame], name: &str) -> &'a str {
        frames
            .iter()
            .find(|frame| frame.name == name)
            .unwrap_or_else(|| panic!("no frame named {name}"))
            .svg
            .as_str()
    }

    #[test]
    fn the_phone_frames_are_deterministic_and_show_their_kinds() {
        let once = scene::render().expect("the site art renders");
        let twice = scene::render().expect("the site art renders again");
        assert_eq!(once.phones.len(), 6, "three frames in both schemes");
        let names: Vec<&str> = once
            .phones
            .iter()
            .map(|frame| frame.name.as_str())
            .collect();
        assert_eq!(names, NAMES, "stable names in swipe order");
        for name in NAMES {
            assert_eq!(
                svg_named(&once.phones, name),
                svg_named(&twice.phones, name),
                "{name} must be byte-identical"
            );
        }

        // The phone grid at the posters' cell metrics: 44 × 10 by 24 × 19.
        for frame in &once.phones {
            assert!(
                frame.svg.starts_with(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 440 456""#
                ),
                "{}: the 44 × 24 grid",
                frame.name
            );
            let variant = frame
                .name
                .trim_start_matches("phone/")
                .trim_end_matches(".svg");
            assert!(
                frame
                    .svg
                    .contains(&format!(r#"id="poster-title-{variant}""#)),
                "{}: its own accessible name",
                frame.name
            );
        }

        // Outlined for the isolated-image context: Latin text is paths,
        // the emoji the bundled subsets lack stay text, and nothing pulls
        // a font in.
        let conversation = svg_named(&once.phones, "phone/conversation-dark.svg");
        assert!(conversation.contains("<path"), "the text is outlined");
        assert!(
            !conversation.contains(">Scranton<"),
            "no Latin is left as text an image-mode SVG can't render"
        );
        assert_eq!(
            conversation.matches(">📄<").count(),
            2,
            "the chat title's and the file row's emoji stay text"
        );
        assert!(!conversation.contains("url("), "no font is fetched");
        assert!(!conversation.contains("@font-face"), "no font is embedded");

        // The scheme follows the theme, as for the posters: the first rect
        // is the theme's background.
        assert!(
            conversation.contains(r##"fill="#101216""##),
            "faro-dark's background"
        );
        let light = svg_named(&once.phones, "phone/conversation-light.svg");
        assert!(
            light.contains(r##"fill="#f7f4ee""##),
            "faro-light's background"
        );
        assert_ne!(conversation, light);
    }

    /// What the frames actually show, read from the real buffers behind
    /// them (the SVGs' text is outlines, so the content lives here).
    #[expect(
        clippy::disallowed_methods,
        reason = "the site generator is a one-shot CLI: the test blocks on a current-thread runtime to build the demo's first state"
    )]
    #[test]
    fn the_phone_buffers_show_their_kinds() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("the site's runtime starts");
        let mut state = runtime
            .block_on(demo::first_state())
            .expect("the demo's first state");
        let grid = |state: &mut State, kind: Kind, theme: &str| {
            show(state, kind, theme);
            faro_app::ui::text::grid(&demo::draw(state, COLS, ROWS))
        };

        // Conversation: the branch open with its backlog, and no sidebar —
        // at 44 columns the list hides itself, which is the whole point of
        // the crop.
        let conversation = grid(&mut state, Kind::Conversation, "faro-dark");
        assert!(conversation.contains("Scranton Branch"), "{conversation}");
        assert!(conversation.contains("drill"), "its backlog shows");
        assert!(
            !conversation.contains("Conversations"),
            "the sidebar hides itself at 44 columns"
        );
        assert!(!conversation.contains("Chats"), "no palette in this frame");

        // Palette: the root's categories, opened through the reducer, and
        // the Themes row's detail naming the *scheme's own* theme — the
        // palette was opened under it, not recoloured afterwards.
        let palette = grid(&mut state, Kind::Palette, "faro-dark");
        for category in ["Chats", "Themes", "Settings", "Commands", "Keys", "Search"] {
            assert!(
                palette.contains(category),
                "the palette root lists {category}"
            );
        }
        assert!(
            palette.contains("faro-dark"),
            "the dark palette's Themes row shows faro-dark"
        );
        let palette_light = grid(&mut state, Kind::Palette, "faro-light");
        assert!(
            palette_light.contains("faro-light"),
            "the light palette's Themes row shows faro-light"
        );
        assert!(
            !palette_light.contains("faro-dark"),
            "the light palette doesn't carry the dark scheme's marker"
        );

        // Themes: the picker under the palette, faro's own themes listed.
        let themes = grid(&mut state, Kind::Themes, "faro-dark");
        assert!(themes.contains("faro-dark"), "the picker lists faro-dark");
        assert!(themes.contains("faro-light"), "the picker lists faro-light");
    }

    /// The picker's own state under each scheme: the current marker, the
    /// selection and the previewed theme all name that scheme's theme, so
    /// `demo::draw` (which resolves `State::theme_name`) and the SVG's
    /// colours can't disagree.
    /// Generating frames is a read: the caller's theme and open palette come
    /// back as they were, so the poster's own state is untouched.
    #[expect(
        clippy::disallowed_methods,
        reason = "the site generator is a one-shot CLI: the test blocks on a current-thread runtime to build the demo's first state"
    )]
    #[test]
    fn generating_frames_restores_the_callers_theme_and_palette() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("the site's runtime starts");
        let mut state = runtime
            .block_on(demo::first_state())
            .expect("the demo's first state");
        "faro-dusk".clone_into(&mut state.config.theme);
        open_palette(&mut state, Level::Root);
        let frames = phones(&mut state).expect("the phone frames generate");
        assert_eq!(frames.len(), 6);
        assert_eq!(state.config.theme, "faro-dusk", "the theme comes back");
        assert_eq!(
            state.theme_name(),
            "faro-dusk",
            "so does the resolved theme"
        );
        let palette = state
            .palette
            .as_ref()
            .expect("the caller's palette comes back");
        assert_eq!(
            palette.level(),
            Level::Root,
            "and it is the one that was open"
        );
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the site generator is a one-shot CLI: the test blocks on a current-thread runtime to build the demo's first state"
    )]
    #[test]
    fn the_picker_marks_and_previews_the_schemes_own_theme() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("the site's runtime starts");
        let mut state = runtime
            .block_on(demo::first_state())
            .expect("the demo's first state");
        for (_, theme_name) in SCHEMES {
            show(&mut state, Kind::Themes, theme_name);
            assert_eq!(
                state.theme_name(),
                theme_name,
                "the preview follows the scheme"
            );
            let palette = state.palette.as_ref().expect("the picker is open");
            let selected = palette.selected_item().expect("a theme is selected");
            assert_eq!(
                selected.title, theme_name,
                "the picker opens on its own theme"
            );
            assert_eq!(selected.current, Some(true), "it is marked current");
            let marked: Vec<&str> = palette
                .items
                .iter()
                .filter(|item| item.current == Some(true))
                .map(|item| item.title.as_str())
                .collect();
            assert_eq!(marked, [theme_name], "exactly one theme is marked current");
        }
    }
}
