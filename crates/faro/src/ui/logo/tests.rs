//! The logo's pictures and the choice of form by pane size (D61).

use ratatui::layout::Rect;
use ratatui::style::Color;
use unicode_width::UnicodeWidthChar as _;

use super::art::{self, Art, Picture};
use super::{Note, STILL, TAGLINE, Tier, choose, width_of};
use crate::theme::{ColorDepth, Theme};

fn on(theme: &Theme, depth: ColorDepth) -> Art {
    Art::new(&theme.clone().for_depth(depth), depth).unwrap()
}

fn dark() -> Art {
    on(&Theme::faro_dark(), ColorDepth::Truecolor)
}

fn light() -> Art {
    on(&Theme::faro_light(), ColorDepth::Truecolor)
}

/// Every picture the logo draws, at a few moments of the animation.
fn pictures() -> Vec<Picture> {
    let mut all = vec![
        dark().wordmark(1),
        dark().wordmark(2),
        light().wordmark(2),
        art::small(&Theme::system()),
        art::small(&Theme::faro_dark()),
    ];
    for art in [
        dark(),
        light(),
        on(&Theme::faro_dark(), ColorDepth::Indexed256),
    ] {
        for step in 0..40 {
            all.push(art.lighthouse(STILL + f64::from(step) * 0.17));
        }
    }
    all
}

#[test]
fn only_single_cell_symbols() {
    for picture in pictures() {
        for (x, y, glyph) in picture.glyphs() {
            let c = glyph.symbol;
            assert!(
                matches!(c, '▀' | '▄' | '█' | '\u{2801}'..='\u{28FF}'),
                "{c:?} at {x},{y}"
            );
            // One cell everywhere: unicode-width (and so tmux and mosh,
            // which measure single code points the same way, D47).
            assert_eq!(c.width(), Some(1), "{c:?}");
        }
    }
    assert_eq!(width_of(TAGLINE), 45);
}

#[test]
fn sizes_match_the_prototype() {
    let lighthouse = dark().lighthouse(STILL);
    assert_eq!((lighthouse.width, lighthouse.height), (40, 13));
    assert_eq!(art::wordmark_size(2), (46, 7));
    assert_eq!(art::wordmark_size(1), (23, 4));
    let word = dark().wordmark(2);
    assert_eq!((word.width, word.height), (46, 7));
}

#[test]
fn the_still_frame_is_always_the_same_and_the_animation_moves() {
    assert_eq!(dark().lighthouse(STILL), dark().lighthouse(STILL));
    for later in [0.1, 0.5, 2.0] {
        assert_ne!(dark().lighthouse(STILL), dark().lighthouse(STILL + later));
    }
}

/// Braille cells in the sky left of the tower, where only stars are at the
/// still frame (the beam points right).
fn stars(picture: &Picture) -> usize {
    picture
        .glyphs()
        .filter(|&(x, y, g)| x < 15 && y < 6 && ('\u{2801}'..='\u{28FF}').contains(&g.symbol))
        .count()
}

#[test]
fn stars_shine_only_on_dark_backgrounds() {
    assert!(stars(&dark().lighthouse(STILL)) > 0);
    assert_eq!(stars(&light().lighthouse(STILL)), 0);
}

#[test]
fn the_logo_s_own_colours_at_truecolor_and_their_nearest_at_256() {
    let red = Color::Rgb(210, 60, 60);
    let colours = |p: &Picture| -> Vec<Color> {
        p.glyphs()
            .flat_map(|(_, _, g)| [Some(g.fg), g.bg])
            .flatten()
            .collect()
    };
    assert!(colours(&dark().lighthouse(STILL)).contains(&red));
    let indexed = on(&Theme::faro_dark(), ColorDepth::Indexed256);
    for picture in [indexed.lighthouse(STILL), indexed.wordmark(2)] {
        let all = colours(&picture);
        assert_ne!(all, vec![]);
        assert!(
            all.iter().all(|c| matches!(c, Color::Indexed(16..))),
            "{all:?}"
        );
    }
}

#[test]
fn no_art_where_the_colours_are_the_terminal_s() {
    assert_eq!(Art::new(&Theme::system(), ColorDepth::Truecolor), None);
    assert_eq!(Art::new(&Theme::faro_dark(), ColorDepth::Ansi16), None);
    // The small one is drawn in the theme's roles instead.
    let theme = Theme::system();
    let small = art::small(&theme);
    let fgs: Vec<Color> = small.glyphs().map(|(_, _, g)| g.fg).collect();
    for role in [theme.error, theme.text, theme.warning, theme.border] {
        assert!(fgs.contains(&role), "{role:?}");
    }
    assert!(small.glyphs().all(|(_, _, g)| g.bg.is_none()));
}

fn hints(n: usize) -> Vec<Note> {
    vec![Note::Hint("key".to_owned(), "does"); n]
}

fn tier(width: u16, height: u16, art: bool) -> (Tier, usize) {
    let mut notes = hints(4);
    let tier = choose(Rect::new(0, 0, width, height), art, 45, &mut notes);
    (tier, notes.len())
}

#[test]
fn the_largest_form_that_fits_with_its_hints() {
    // Full: 90 × 13, then a blank line and the hints.
    assert_eq!(tier(90, 18, true), (Tier::Full, 4));
    assert_eq!(tier(200, 60, true), (Tier::Full, 4));
    // A short pane drops the last hint rather than the big picture.
    assert_eq!(tier(90, 17, true), (Tier::Full, 3));
    // Stacked at scale 2: 46 wide, 13 + 1 + 7 + 2 (tagline) tall.
    assert_eq!(tier(89, 28, true), (Tier::Stacked(2), 4));
    assert_eq!(tier(60, 25, true), (Tier::Stacked(1), 4));
    // Without the tagline (narrower than it), stacked needs only 40 columns.
    assert_eq!(tier(42, 23, true), (Tier::Stacked(1), 4));
    assert_eq!(tier(60, 13, true), (Tier::Small, 4));
    assert_eq!(tier(60, 12, true), (Tier::Small, 3));
    assert_eq!(tier(60, 11, true), (Tier::Word, 4));
    assert_eq!(tier(3, 3, true), (Tier::Word, 1));
    assert_eq!(tier(3, 1, true), (Tier::Word, 0));
    // The terminal's own colours: never the big picture.
    assert_eq!(tier(200, 60, false), (Tier::Small, 4));
}
