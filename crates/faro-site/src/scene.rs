//! The poster and the OG image (WEBSITE.md §3.2, §5.4, §6.4): a real faro
//! frame, built the way the web demo builds its first state — the mock's
//! backlog in the memory store, through the real reducer — then opened onto
//! the cast's liveliest group chat and exported as SVG in both faro themes,
//! plus a 1200×630 PNG for social previews.
//!
//! The frame is byte-identical across runs and days: the mock runs with
//! [`MockConfig::deterministic`] and seed [`faro_web::demo::SEED`], and
//! `demo::draw` pins the clock to `DETERMINISTIC_EPOCH` (so the relative
//! timestamps are fixed), uses UTC and draws the still logo frame
//! (`logo_time: None`). The PNG is deterministic too: resvg rasterises with
//! only the fonts under `fonts/` loaded, never system fonts.

use std::fmt::Write as _;

use faro_app::app::event::AppEvent;
use faro_app::app::state::{BlockGlyphs, State};
use faro_app::app::term_event::{Event, KeyCode, KeyEvent, KeyModifiers};
use faro_app::app::update;
use faro_app::theme::Theme;
use faro_app::ui::logo::TAGLINE;
use faro_core::domain::ConversationId;
use faro_core::mock::MockConfig;
use faro_core::store::Profile;
use faro_web::color::{self, Rgb};
use faro_web::demo;
use faro_web::glyphs;
use faro_web::svg::{self, SvgOptions};
use ratatui::buffer::Buffer;
use resvg::tiny_skia;
use resvg::usvg::{self, fontdb};

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

/// The poster's size in user units, from the grid and cell metrics.
fn poster_w() -> f32 {
    f32::from(COLS) * CELL_W
}
fn poster_h() -> f32 {
    f32::from(ROWS) * CELL_H
}

/// The OG image canvas: the size social platforms show (§5.4).
const OG_W: u32 = 1200;
const OG_H: u32 = 630;
/// The wordmark and tagline sit on one line in the top band; the frame is
/// centred below them, scaled to fit with a margin. 936 wide keeps the
/// frame's height at 530.4, so the bottom margin is ≈ 20.
const OG_FRAME_W: f32 = 936.0;
const OG_FRAME_X: f32 = (1200.0 - OG_FRAME_W) / 2.0;
const OG_FRAME_Y: f32 = 80.0;
const OG_WORDMARK_SIZE: f32 = 40.0;
const OG_TAGLINE_SIZE: f32 = 18.0;
const OG_TEXT_X: f32 = 40.0;
const OG_TEXT_BASELINE: f32 = 58.0;
/// The tagline starts after the 4-letter wordmark plus a gap.
const OG_TAGLINE_X: f32 = OG_TEXT_X + 4.0 * 0.6 * OG_WORDMARK_SIZE + 40.0;

/// The fonts resvg rasterises with — the web demo's own subsets (D107),
/// decompiled for resvg (see `fonts/README.md`). Loading only these, and
/// never system fonts, is what makes the PNG byte-identical everywhere.
const FONT_REGULAR: &[u8] = include_bytes!("../fonts/JetBrainsMono-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../fonts/JetBrainsMono-Bold.ttf");
/// The family name inside those files.
const FONT_NAME: &str = "JetBrains Mono";

/// The page's posters and the OG image, all from one frame.
pub struct Art {
    pub posters: Posters,
    pub og_png: Vec<u8>,
}

/// The same frame as SVG, once per page scheme.
pub struct Posters {
    pub dark: String,
    pub light: String,
}

/// Build the posters and the OG image. faro-site has no async runtime of
/// its own, but the demo's mock and memory store run as tasks; tokio is
/// already in the tree through faro-core, so a current-thread runtime with
/// the real clock costs nothing new. The mock connects after 400 ms and the
/// backlog poll waits three 100 ms beats, so this takes about a second.
#[expect(
    clippy::disallowed_methods,
    reason = "the site generator is a one-shot CLI: it blocks on a current-thread runtime to drive the demo's tasks"
)]
pub fn render() -> Result<Art, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| format!("the poster's runtime won't start: {error}"))?;
    runtime.block_on(frame())
}

/// The state, opened onto the branch chat with the message pane focused,
/// then rendered in both themes and rasterised for the OG image.
async fn frame() -> Result<Art, String> {
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
    let og_png = og(&mut state)?;
    Ok(Art {
        posters: Posters { dark, light },
        og_png,
    })
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
    Ok(named(
        &svg::buffer_to_svg(&buffer, &svg_options(theme)),
        variant,
    ))
}

/// The SVG export options shared by both posters and the OG's frame.
fn svg_options(theme: &Theme) -> SvgOptions {
    let (foreground, background) = theme_rgb(theme);
    SvgOptions {
        cell_width: CELL_W,
        cell_height: CELL_H,
        font_family: FONT_FAMILY.to_owned(),
        font_size: FONT_SIZE,
        foreground,
        background,
        title: TITLE.to_owned(),
    }
}

/// The OG image (WEBSITE.md §5.4): the dark poster's frame, scaled to fit
/// under the wordmark and tagline, rasterised to PNG at 1200×630.
///
/// Emoji: resvg has only `JetBrains Mono`, no emoji font, so the frame is
/// drawn a second time with the cells the font lacks blanked at the buffer
/// level (below). The page's posters keep their emoji — browsers have emoji
/// fonts; the OG image loses them and nothing else.
fn og(state: &mut State) -> Result<Vec<u8>, String> {
    "faro-dark".clone_into(&mut state.config.theme);
    let theme = Theme::find("faro-dark", &state.user_themes)
        .ok_or_else(|| "the built-in theme faro-dark is missing".to_owned())?;
    let mut buffer = demo::draw(state, COLS, ROWS);
    strip_unsupported(&mut buffer);
    let inner = svg::buffer_to_svg(&buffer, &svg_options(theme));
    let svg = og_svg(&inner, theme);
    rasterise(&svg)
}

/// Blank cells the bundled font can't draw and `glyphs::shape` doesn't draw
/// as a path either (emoji, flags, scripts outside the subset). Blanking at
/// the buffer level keeps the layout: a wide cell's continuation cells are
/// already empty, and `svg.rs` draws every cell's background on its own.
fn strip_unsupported(buffer: &mut Buffer) {
    let Ok(font) = skrifa::raw::FontRef::new(FONT_REGULAR) else {
        return;
    };
    let charmap = skrifa::charmap::Charmap::new(&font);
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let Some(cell) = buffer.cell_mut((x, y)) else {
                continue;
            };
            let symbol = cell.symbol();
            let keep = symbol.is_empty()
                || symbol == " "
                || glyphs::shape(symbol).is_some()
                || symbol.chars().all(|c| charmap.map(c).is_some());
            if !keep {
                cell.set_symbol(" ");
            }
        }
    }
}

/// The 1200×630 OG document: `inner` (a whole poster SVG) embedded as a
/// nested `<svg>` scaled to `OG_FRAME_*`, with the wordmark and tagline
/// above it in the theme's own colours.
fn og_svg(inner: &str, theme: &Theme) -> String {
    // Drop the poster's root tag and closing tag; keep everything inside.
    let body = inner
        .split_once('>')
        .map_or(inner, |(_, rest)| rest)
        .strip_suffix("</svg>")
        .unwrap_or(inner);
    let (foreground, background) = theme_rgb(theme);
    let muted = color::rgb(theme.text_muted, background);
    let frame_w = num(OG_FRAME_W);
    let frame_h = num(OG_FRAME_W * poster_h() / poster_w());
    let frame_x = num(OG_FRAME_X);
    let frame_y = num(OG_FRAME_Y);
    let text_x = num(OG_TEXT_X);
    let baseline = num(OG_TEXT_BASELINE);
    let tagline_x = num(OG_TAGLINE_X);
    let wordmark_size = num(OG_WORDMARK_SIZE);
    let tagline_size = num(OG_TAGLINE_SIZE);
    let view_w = num(poster_w());
    let view_h = num(poster_h());
    let bg = hex(background);
    let fg = hex(foreground);
    let dim = hex(muted);
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{OG_W}" height="{OG_H}" viewBox="0 0 {OG_W} {OG_H}">"#
    );
    let _ = write!(
        out,
        r#"<rect width="{OG_W}" height="{OG_H}" fill="{bg}" stroke="none"/>"#
    );
    let _ = write!(
        out,
        r#"<text x="{text_x}" y="{baseline}" font-family="{FONT_NAME}" font-size="{wordmark_size}" font-weight="bold" fill="{fg}">faro</text>"#
    );
    let _ = write!(
        out,
        r#"<text x="{tagline_x}" y="{baseline}" font-family="{FONT_NAME}" font-size="{tagline_size}" fill="{dim}">{TAGLINE}</text>"#
    );
    let _ = write!(
        out,
        r#"<svg x="{frame_x}" y="{frame_y}" width="{frame_w}" height="{frame_h}" viewBox="0 0 {view_w} {view_h}">{body}</svg>"#
    );
    out.push_str("</svg>");
    out
}

/// An RGB as the `#rrggbb` an SVG attribute wants.
fn hex(rgb: Rgb) -> String {
    let mut text = String::new();
    rgb.write_hex(&mut text);
    text
}

/// The same number style `svg.rs` uses: two decimals at most, no trailing
/// zeros, so the OG document's geometry reads cleanly and never carries
/// f32 noise.
fn num(value: f32) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded == 0.0 {
        return "0".to_owned();
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

/// Rasterise an SVG with only the bundled fonts: an empty font database
/// plus the two subsets, and the generic monospace family pointed at them
/// so nothing on the build machine leaks into the pixels.
fn rasterise(svg: &str) -> Result<Vec<u8>, String> {
    let mut db = fontdb::Database::new();
    db.load_font_data(FONT_REGULAR.to_vec());
    db.load_font_data(FONT_BOLD.to_vec());
    db.set_monospace_family(FONT_NAME);
    let options = usvg::Options {
        fontdb: std::sync::Arc::new(db),
        font_family: FONT_NAME.to_owned(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(svg.as_bytes(), &options)
        .map_err(|error| format!("the OG SVG won't parse: {error}"))?;
    let mut pixmap =
        tiny_skia::Pixmap::new(OG_W, OG_H).ok_or_else(|| "the OG pixmap won't start".to_owned())?;
    let mut surface = pixmap.as_mut();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(1.0, 1.0),
        &mut surface,
    );
    pixmap
        .encode_png()
        .map_err(|error| format!("the OG PNG won't encode: {error}"))
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
            once.posters.dark, twice.posters.dark,
            "the dark poster must be byte-identical"
        );
        assert_eq!(
            once.posters.light, twice.posters.light,
            "the light poster must be byte-identical"
        );
        assert_eq!(
            once.og_png, twice.og_png,
            "the OG PNG must be byte-identical"
        );

        // Cast text from the opened chat's backlog — not just its title,
        // which the sidebar lists too. (Text comes in per-word tspans, so
        // assert single words.)
        assert!(
            once.posters.dark.contains(">drill<"),
            "the branch's fire-drill scene shows"
        );
        assert!(
            once.posters.dark.contains(">connected<"),
            "the footer shows the connection"
        );
        assert!(
            once.posters.dark.contains(">demo<"),
            "the footer says demo (D87)"
        );

        // The two variants differ in their backdrop: the first rect is the
        // theme's background.
        assert!(
            once.posters.dark.contains(r##"fill="#101216""##),
            "faro-dark's background"
        );
        assert!(
            once.posters.light.contains(r##"fill="#f7f4ee""##),
            "faro-light's background"
        );
        assert_ne!(once.posters.dark, once.posters.light);

        // One accessible name per poster, with distinct ids.
        assert!(once.posters.dark.contains(r#"id="poster-title-dark""#));
        assert!(!once.posters.dark.contains(r#"id="poster-title">"#));
        assert!(once.posters.light.contains(r#"id="poster-title-light""#));

        // `FARO_POSTER_OUT=<dir> cargo test -p faro-site` writes all three
        // out.
        if let Some(dir) = std::env::var_os("FARO_POSTER_OUT") {
            std::fs::create_dir_all(&dir).expect("the poster out dir");
            let out = std::path::Path::new(&dir);
            std::fs::write(out.join("poster-dark.svg"), &once.posters.dark)
                .expect("poster-dark.svg can be written");
            std::fs::write(out.join("poster-light.svg"), &once.posters.light)
                .expect("poster-light.svg can be written");
            std::fs::write(out.join("og.png"), &once.og_png).expect("og.png can be written");
        }
    }

    /// The OG PNG: the right size and small enough for social platforms
    /// (§5.4: under 300 KB; the budget keeps it a fast unfurl).
    #[test]
    fn the_og_image_is_the_right_size_and_weight() {
        let art = render().expect("the art renders");
        let png = &art.og_png;
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "a PNG");
        // IHDR: 4-byte length, "IHDR", then width and height, big-endian.
        assert_eq!(&png[12..16], b"IHDR");
        let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!((width, height), (OG_W, OG_H));
        assert!(
            png.len() <= 300 * 1024,
            "the OG PNG is {} KB: over the 300 KB budget",
            png.len() / 1024
        );
    }
}
