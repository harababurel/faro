//! Terminal checks: emoji mode and mosh (D47), kitty for the bubble style
//! (D58), Ctrl+L.

use crate::app::term_event::Event;

use super::*;
use crate::app::state::Terminal;
use crate::app::state::{BlockGlyphs, EmojiMode, EmojiSetting, GraphicsSupport};
use faro_core::config::{BubbleStyle, InlineImages};

fn checked(mosh: bool) -> AppEvent {
    AppEvent::TerminalChecked(Terminal {
        mosh,
        glyphs: BlockGlyphs::Font,
        graphics: if mosh {
            GraphicsSupport::HalfblocksOnly
        } else {
            GraphicsSupport::Direct
        },
        kitty: false,
    })
}

fn auto(mode: EmojiMode) -> EmojiSetting {
    EmojiSetting {
        mode,
        auto: true,
        checked_at: None,
        from_cli: false,
        mosh: false,
        exact: false,
    }
}

#[test]
fn finding_mosh_switches_to_compatible_and_back() {
    let mut state = loaded_state();
    state.env.emoji = auto(EmojiMode::Full);

    update(&mut state, checked(true));
    assert_eq!(state.env.emoji.mode, EmojiMode::Compatible);
    assert!(state.env.repaint);
    assert_eq!(
        state.notice.as_ref().map(|n| n.level),
        Some(NoticeLevel::Info)
    );

    state.env.repaint = false;
    state.notice = None;
    update(&mut state, checked(true));
    assert!(!state.env.repaint, "nothing changed, nothing to repaint");
    assert!(state.notice.is_none());

    update(&mut state, checked(false));
    assert_eq!(state.env.emoji.mode, EmojiMode::Full);
    assert!(state.env.repaint);
}

#[test]
fn a_chosen_mode_is_kept() {
    let mut state = loaded_state();
    state.env.emoji = EmojiSetting {
        mode: EmojiMode::Full,
        auto: false,
        checked_at: None,
        from_cli: true,
        mosh: false,
        exact: false,
    };
    state.config.messages.bubble_style = BubbleStyle::Bar;
    state.config.media.inline_images = InlineImages::Off;
    state.env.graphics = GraphicsSupport::HalfblocksOnly;
    update(&mut state, checked(true));
    assert_eq!(state.env.emoji.mode, EmojiMode::Full);
    assert!(!state.env.repaint);

    // No checks either.
    assert_eq!(
        update(&mut state, AppEvent::Terminal(Event::FocusGained)),
        vec![]
    );
    assert_eq!(
        update(&mut state, AppEvent::Terminal(Event::Resize(80, 24))),
        vec![]
    );
}

#[test]
fn focus_and_resize_recheck_at_most_every_two_seconds() {
    let mut state = loaded_state();
    state.env.emoji = auto(EmojiMode::Full);
    assert_eq!(
        update(&mut state, AppEvent::Terminal(Event::FocusGained)),
        vec![Effect::CheckTerminal]
    );
    assert_eq!(
        update(&mut state, AppEvent::Terminal(Event::Resize(80, 24))),
        vec![]
    );
    assert_eq!(
        update(&mut state, AppEvent::Terminal(Event::FocusGained)),
        vec![]
    );
}

#[test]
fn an_auto_bubble_style_follows_kitty() {
    let mut state = loaded_state();
    state.env.emoji.auto = false;
    state.config.messages.bubble_style = BubbleStyle::Auto;
    assert_eq!(
        update(&mut state, AppEvent::Terminal(Event::FocusGained)),
        vec![Effect::CheckTerminal]
    );
    let kitty = Terminal {
        mosh: false,
        glyphs: BlockGlyphs::Native,
        graphics: GraphicsSupport::Direct,
        kitty: true,
    };
    update(&mut state, AppEvent::TerminalChecked(kitty));
    assert_eq!(state.env.glyphs, BlockGlyphs::Native);
    assert_eq!(
        state.env.emoji.mode,
        EmojiMode::Full,
        "emoji mode untouched"
    );
    update(&mut state, checked(false));
    assert_eq!(state.env.glyphs, BlockGlyphs::Font);
}

#[test]
fn ctrl_l_repaints() {
    let mut state = loaded_state();
    assert!(!state.env.repaint);
    update(&mut state, key(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(state.env.repaint);
}
