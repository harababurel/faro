//! The poster (WEBSITE.md §3.2, §6.4): a real faro frame, built the way the
//! web demo builds its first state — the mock's backlog in the memory store,
//! through the real reducer — then opened onto the cast's liveliest group
//! chat and exported as SVG in both faro themes.
//!
//! The frame is byte-identical across runs and days: the mock runs with
//! [`MockConfig::deterministic`] and seed [`faro_web::demo::SEED`], and
//! `demo::draw` pins the clock to `DETERMINISTIC_EPOCH` (so the relative
//! timestamps are fixed), uses UTC and draws the still logo frame
//! (`logo_time: None`).

use faro_app::app::event::AppEvent;
use faro_app::app::state::{BlockGlyphs, State};
use faro_app::app::term_event::{Event, KeyCode, KeyEvent, KeyModifiers};
use faro_app::app::update;
use faro_app::theme::Theme;
use faro_core::domain::ConversationId;
use faro_core::mock::MockConfig;
use faro_core::store::Profile;
use faro_web::color::{self, Rgb};
use faro_web::demo;
use faro_web::svg::{self, SvgOptions};

/// The chat the poster shows: the cast's biggest group, 15 members and the
/// most scenes (`mock/cast.rs`), named in the template's alt-text comment.
const CHAT: &str = "Scranton Branch";

/// The poster grid: the demo's size (WEBSITE.md §4.4).
const COLS: u16 = 120;
const ROWS: u16 = 34;

/// The poster's backlog: bigger than the demo's 60 on purpose. The mock
/// plays whole scenes into the backlog and reshuffles when it runs out of
/// them, and the branch group's scenes sit late in the deck — with 60
/// messages (seed 42) the branch has no backlog at all. 400 covers all 30
/// scenes (≈ 180 lines) at least once, so the branch's liveliness shows.
const BACKLOG: usize = 400;

/// Cell width in SVG user units: `JetBrains Mono`'s advance width is 0.6 em,
/// so at a 16 px font a cell is 9.6 wide.
const CELL_W: f32 = 9.6;
/// Cell height: the web painter's line height, 1.2 × the font size
/// (`paint.rs::LINE_HEIGHT`), so the poster's cells have exactly the shape
/// the live demo lays out and the handover matches (§3.2). `svg.rs` puts
/// the baseline at 0.8 of the cell height = 0.96 of the font size, which is
/// what the canvas measures for this font. The SVG scales to the panel
/// (`width: 100%`), so only these ratios matter.
const CELL_H: f32 = 19.2;
/// The poster's font size in the same user units.
const FONT_SIZE: f32 = 16.0;
/// The page's own mono stack (`style.css`'s `--mono`), so the poster draws
/// in the `JetBrains Mono` webfont like the rest of the page.
const FONT_FAMILY: &str =
    r#""JetBrains Mono", ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace"#;

/// What the poster shows, for screen readers (the SVG `<title>`).
const TITLE: &str = "faro running in a terminal: the conversation list on the \
                     left, the Scranton Branch group chat open on the right, \
                     an input box below, and a footer reading connected · demo · unread";

/// The same frame as SVG, once per page scheme.
pub struct Posters {
    pub dark: String,
    pub light: String,
}

/// Build both posters. faro-site has no async runtime of its own, but the
/// demo's mock and memory store run as tasks; tokio is already in the tree
/// through faro-core, so a current-thread runtime with the real clock costs
/// nothing new. The mock connects after 400 ms and the backlog poll waits
/// three 100 ms beats, so this takes about a second.
#[expect(
    clippy::disallowed_methods,
    reason = "the site generator is a one-shot CLI: it blocks on a current-thread runtime to drive the demo's tasks"
)]
pub fn render() -> Result<Posters, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| format!("the poster's runtime won't start: {error}"))?;
    runtime.block_on(frame())
}

/// The state, opened onto the branch chat with the message pane focused,
/// then rendered in both themes.
async fn frame() -> Result<Posters, String> {
    let mock = MockConfig {
        activity: None,
        disconnects: false,
        errors: false,
        backlog: BACKLOG,
        ..MockConfig::deterministic(demo::SEED)
    };
    let (mut state, store) = demo::stored_state(&mock)
        .await
        .map_err(|error| format!("the demo's first state failed: {error}"))?;
    // The poster is a frame of `faro demo`: the footer says so (D87), and
    // the environment matches the live demo's (`session::Session::start`):
    // native block glyphs, so `bubble_style = "auto"` gives the rounded
    // bubbles the modern terminals show.
    state.profile = Profile::Demo;
    state.env.glyphs = BlockGlyphs::Native;

    let target = branch(&state)?;
    // Walk the list cursor onto it with Alt+Down (which saturates), as a
    // visitor would, then open it with Enter.
    let steps = state.visible_conversations().count();
    for _ in 0..=steps {
        if state.highlighted() == Some(target) {
            break;
        }
        let alt_down = Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT));
        update::update(&mut state, AppEvent::Terminal(alt_down));
    }
    if state.highlighted() != Some(target) {
        return Err(format!("the demo's chat list has no {CHAT} chat to open"));
    }
    demo::open(&mut state, &store, target)
        .await
        .map_err(|error| format!("opening the {CHAT} chat failed: {error}"))?;
    // Tab: focus the message pane, so the poster shows the chat's bubbles
    // and a selected message rather than a blinking input caret. (Before
    // the first draw the Tab order is messages ↔ input.)
    update::update(
        &mut state,
        AppEvent::Terminal(Event::Key(KeyEvent::from(KeyCode::Tab))),
    );
    store
        .close()
        .await
        .map_err(|error| format!("the memory store failed to close: {error}"))?;

    let dark = poster(&mut state, "faro-dark", "dark")?;
    let light = poster(&mut state, "faro-light", "light")?;
    Ok(Posters { dark, light })
}

/// The branch group's conversation, by title prefix (the full title carries
/// an emoji; "Scranton Business Park" doesn't match).
fn branch(state: &State) -> Result<ConversationId, String> {
    state
        .visible_conversations()
        .find(|summary| {
            summary
                .title
                .as_deref()
                .is_some_and(|title| title.starts_with(CHAT))
        })
        .map(|summary| summary.id)
        .ok_or_else(|| format!("the demo's chat list has no {CHAT} chat"))
}

/// `state` as one theme's poster SVG: the theme set through the config the
/// way the app does, drawn by `demo::draw` and exported by `faro_web::svg`.
fn poster(state: &mut State, name: &str, variant: &str) -> Result<String, String> {
    name.clone_into(&mut state.config.theme);
    let buffer = demo::draw(state, COLS, ROWS);
    let theme = Theme::find(name, &state.user_themes)
        .ok_or_else(|| format!("the built-in theme {name} is missing"))?;
    let (foreground, background) = theme_rgb(theme);
    let options = SvgOptions {
        cell_width: CELL_W,
        cell_height: CELL_H,
        font_family: FONT_FAMILY.to_owned(),
        font_size: FONT_SIZE,
        foreground,
        background,
        title: TITLE.to_owned(),
    };
    Ok(named(&svg::buffer_to_svg(&buffer, &options), variant))
}

/// The theme's text and background as RGB. Both faro themes are RGB; a
/// `Color::Reset` there would mean "the page's own colours", which an
/// inline SVG inherits anyway, so black is only a stand-in.
fn theme_rgb(theme: &Theme) -> (Rgb, Rgb) {
    let stand_in = Rgb(0, 0, 0);
    (
        color::rgb(theme.text, stand_in),
        color::rgb(theme.background, stand_in),
    )
}

/// `svg::buffer_to_svg` calls its `<title>` `poster-title`; two inline
/// posters in one page need distinct ids so each `aria-labelledby` points
/// at its own.
fn named(svg: &str, variant: &str) -> String {
    svg.replace("poster-title", &format!("poster-title-{variant}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::disallowed_methods,
        reason = "the site generator is a one-shot CLI, and FARO_POSTER_OUT is a test-only convenience for a human to look at"
    )]
    #[test]
    fn the_posters_are_deterministic_and_show_the_branch() {
        let once = render().expect("the posters render");
        let twice = render().expect("the posters render again");
        assert_eq!(
            once.dark, twice.dark,
            "the dark poster must be byte-identical"
        );
        assert_eq!(
            once.light, twice.light,
            "the light poster must be byte-identical"
        );

        // Cast text from the opened chat's backlog — not just its title,
        // which the sidebar lists too. (Text comes in per-word tspans, so
        // assert single words.)
        assert!(
            once.dark.contains(">drill<"),
            "the branch's fire-drill scene shows"
        );
        assert!(
            once.dark.contains(">connected<"),
            "the footer shows the connection"
        );
        assert!(once.dark.contains(">demo<"), "the footer says demo (D87)");

        // The two variants differ in their backdrop: the first rect is the
        // theme's background.
        assert!(
            once.dark.contains(r##"fill="#101216""##),
            "faro-dark's background"
        );
        assert!(
            once.light.contains(r##"fill="#f7f4ee""##),
            "faro-light's background"
        );
        assert_ne!(once.dark, once.light);

        // One accessible name per poster, with distinct ids.
        assert!(once.dark.contains(r#"id="poster-title-dark""#));
        assert!(!once.dark.contains(r#"id="poster-title">"#));
        assert!(once.light.contains(r#"id="poster-title-light""#));

        // `FARO_POSTER_OUT=<dir> cargo test -p faro-site` writes both out.
        if let Some(dir) = std::env::var_os("FARO_POSTER_OUT") {
            std::fs::create_dir_all(&dir).expect("the poster out dir");
            std::fs::write(
                std::path::Path::new(&dir).join("poster-dark.svg"),
                &once.dark,
            )
            .expect("poster-dark.svg can be written");
            std::fs::write(
                std::path::Path::new(&dir).join("poster-light.svg"),
                &once.light,
            )
            .expect("poster-light.svg can be written");
        }
    }
}
