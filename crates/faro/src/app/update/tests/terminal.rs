//! Emoji mode and terminal checks (D47): mosh detection, Ctrl+L.

use crossterm::event::Event;

use super::*;
use crate::app::state::{EmojiMode, EmojiSetting};

fn auto(mode: EmojiMode) -> EmojiSetting {
    EmojiSetting {
        mode,
        auto: true,
        checked_at: None,
        from_cli: false,
    }
}

#[test]
fn finding_mosh_switches_to_compatible_and_back() {
    let mut state = loaded_state();
    state.env.emoji = auto(EmojiMode::Full);

    update(&mut state, AppEvent::TerminalChecked { mosh: true });
    assert_eq!(state.env.emoji.mode, EmojiMode::Compatible);
    assert!(state.env.repaint);
    assert_eq!(
        state.notice.as_ref().map(|n| n.level),
        Some(NoticeLevel::Info)
    );

    state.env.repaint = false;
    state.notice = None;
    update(&mut state, AppEvent::TerminalChecked { mosh: true });
    assert!(!state.env.repaint, "nothing changed, nothing to repaint");
    assert!(state.notice.is_none());

    update(&mut state, AppEvent::TerminalChecked { mosh: false });
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
    };
    update(&mut state, AppEvent::TerminalChecked { mosh: true });
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
fn ctrl_l_repaints() {
    let mut state = loaded_state();
    assert!(!state.env.repaint);
    update(&mut state, key(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert!(state.env.repaint);
}
