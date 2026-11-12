//! Pasting an image from the clipboard (D91, D150): `Ctrl+V` asks the app
//! loop for a read, the result is staged (never sent), `Ctrl+O` shows it.

use std::path::PathBuf;

use faro_core::clipboard::ReadError;

use super::*;
use crate::app::event::{ClipboardError, PasteStage};
use crate::app::state::{
    ClipboardPaste, PastePhase, PasteState, Popup, StagedAttachment, TerminalName,
};

fn ctrl(c: char) -> AppEvent {
    key(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn image() -> StagedAttachment {
    StagedAttachment {
        path: PathBuf::from("/data/attachments/clipboard-20261009-142233.png"),
        name: "clipboard-20261009-142233.png".into(),
        content_type: "image/png".into(),
        size: 285_083,
        width: Some(600),
        height: Some(313),
    }
}

#[test]
fn ctrl_v_asks_for_the_clipboard_and_says_so() {
    let mut state = loaded_state();
    let open = state.open.as_ref().unwrap().id;
    let effects = update(&mut state, ctrl('v'));
    assert_eq!(
        effects,
        vec![Effect::ReadClipboardImage {
            conversation: open,
            system: true,
        }]
    );
    assert_eq!(
        state.pasting,
        PasteState::Active(ClipboardPaste {
            conversation: open,
            phase: PastePhase::Checking,
        })
    );
    let notice = state.notice.as_ref().expect("a notice");
    assert_eq!(notice.text, "Reading the clipboard…");
    // The terminal is about to prompt: name it (D150).
    update(&mut state, AppEvent::ClipboardPrompt(TerminalName::Ghostty));
    assert_eq!(
        state.pasting,
        PasteState::Active(ClipboardPaste {
            conversation: open,
            phase: PastePhase::WaitingForPrompt(TerminalName::Ghostty),
        })
    );
    let notice = state.notice.as_ref().expect("a notice");
    assert_eq!(
        notice.text,
        "Waiting for Ghostty… allow the clipboard prompt"
    );
    update(&mut state, AppEvent::ClipboardProgress { bytes: 3_565_158 });
    assert_eq!(
        state.pasting,
        PasteState::Active(ClipboardPaste {
            conversation: open,
            phase: PastePhase::Receiving { bytes: 3_565_158 },
        })
    );
    update(&mut state, AppEvent::ClipboardStage(PasteStage::Processing));
    assert_eq!(
        state.pasting,
        PasteState::Active(ClipboardPaste {
            conversation: open,
            phase: PastePhase::Processing,
        })
    );
}

/// D150: with `[media] paste_from_system = false` the app loop is told not
/// to read the computer's clipboard.
#[test]
fn the_system_clipboard_setting_reaches_the_app_loop() {
    let mut state = loaded_state();
    state.config.media.paste_from_system = false;
    let open = state.open.as_ref().unwrap().id;
    assert_eq!(
        update(&mut state, ctrl('v')),
        vec![Effect::ReadClipboardImage {
            conversation: open,
            system: false,
        }]
    );
}

/// A prompt notice that comes after the read is over changes nothing.
#[test]
fn a_late_prompt_notice_is_ignored() {
    let mut state = loaded_state();
    state.notice = None;
    update(&mut state, AppEvent::ClipboardPrompt(TerminalName::Kitty));
    assert_eq!(state.notice, None);
}

#[test]
fn ctrl_v_needs_an_open_conversation() {
    let mut state = loaded_state();
    state.open = None;
    assert_eq!(update(&mut state, ctrl('v')), vec![]);
    assert_eq!(state.pasting, PasteState::Idle);
    assert!(state.notice.is_some());
}

/// D91: a pasted image is staged like a pasted file and only Enter sends it.
#[test]
fn the_image_is_staged_and_not_sent() {
    let mut state = loaded_state();
    let open = state.open.as_ref().unwrap().id;
    update(&mut state, ctrl('v'));
    let effects = update(
        &mut state,
        AppEvent::ClipboardImage {
            conversation: open,
            result: Ok(image()),
        },
    );
    assert_eq!(effects, vec![], "nothing is sent");
    assert_eq!(state.pasting, PasteState::Idle);
    let staged = state.open.as_ref().unwrap().staged_attachment.clone();
    assert_eq!(staged, Some(image()));
    let notice = state.notice.as_ref().expect("a notice");
    assert!(notice.text.contains("Enter sends it"), "{}", notice.text);
    // Enter sends it, with the path of the saved file.
    let effects = update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE));
    let sent = effects.iter().any(|e| {
        matches!(e, Effect::Backend(Command::Send { message, .. })
            if message.attachments.first().map(|a| &a.path) == Some(&image().path))
    });
    assert!(sent, "{effects:?}");
}

#[test]
fn failures_become_notices() {
    let mut state = loaded_state();
    let open = state.open.as_ref().unwrap().id;
    for (error, text) in [
        (
            ClipboardError::OverSsh,
            "Pasting images over SSH needs kitty or a newer Ghostty",
        ),
        (
            ClipboardError::NeedsTool,
            "Install wl-clipboard (or xclip) to paste images",
        ),
        (
            ClipboardError::Read(ReadError::Unavailable),
            "The terminal can't share its clipboard here",
        ),
        (
            ClipboardError::Mosh,
            "Pasting images doesn't work over mosh",
        ),
        (ClipboardError::NoImage, "No image on the clipboard"),
        (
            ClipboardError::Read(ReadError::Denied),
            "Clipboard access denied",
        ),
    ] {
        update(&mut state, ctrl('v'));
        update(
            &mut state,
            AppEvent::ClipboardImage {
                conversation: open,
                result: Err(error),
            },
        );
        assert_eq!(state.pasting, PasteState::Idle);
        assert_eq!(state.notice.as_ref().map(|n| n.text.as_str()), Some(text));
        assert_eq!(state.open.as_ref().unwrap().staged_attachment, None);
    }
}

#[test]
fn an_image_for_another_conversation_is_dropped() {
    let mut state = loaded_state();
    let other = state
        .conversations
        .iter()
        .map(|c| c.id)
        .find(|id| Some(*id) != state.open.as_ref().map(|o| o.id))
        .expect("another conversation");
    update(
        &mut state,
        AppEvent::ClipboardImage {
            conversation: other,
            result: Ok(image()),
        },
    );
    assert_eq!(state.open.as_ref().unwrap().staged_attachment, None);
    assert!(state.notice.is_some());
}

#[test]
fn ctrl_o_shows_the_staged_image_and_esc_keeps_it() {
    let mut state = loaded_state();
    update(&mut state, ctrl('o'));
    assert_eq!(state.popup, None, "nothing staged");
    state.open.as_mut().unwrap().staged_attachment = Some(image());
    update(&mut state, ctrl('o'));
    assert_eq!(state.popup, Some(Popup::StagedImage));
    update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(state.popup, None);
    assert_eq!(
        state.open.as_ref().unwrap().staged_attachment,
        Some(image())
    );
}

#[test]
fn esc_and_ctrl_c_cancel_an_active_paste_and_discard_late_results() {
    let mut state = loaded_state();
    let open = state.open.as_ref().unwrap().id;
    update(&mut state, ctrl('v'));
    update(&mut state, key(KeyCode::Char('h'), KeyModifiers::NONE));
    update(&mut state, key(KeyCode::Char('i'), KeyModifiers::NONE));
    // Enter won't prematurely send the caption without the arriving image.
    assert_eq!(
        update(&mut state, key(KeyCode::Enter, KeyModifiers::NONE)),
        vec![]
    );
    assert_eq!(state.open.as_ref().unwrap().input.text(), "hi");

    // Esc cancels the paste, keeps the draft, and unlinks any raced file.
    assert_eq!(
        update(&mut state, key(KeyCode::Esc, KeyModifiers::NONE)),
        vec![Effect::CancelClipboardPaste]
    );
    assert_eq!(state.pasting, PasteState::Cancelled);
    assert_eq!(state.open.as_ref().unwrap().input.text(), "hi");
    assert_eq!(
        state.notice.as_ref().map(|n| n.text.as_str()),
        Some("Clipboard paste cancelled")
    );
    let expected_discard = image().path;
    assert_eq!(
        update(
            &mut state,
            AppEvent::ClipboardImage {
                conversation: open,
                result: Ok(image()),
            },
        ),
        vec![Effect::DiscardStagedImage(expected_discard)]
    );
    assert_eq!(state.pasting, PasteState::Idle);
    assert_eq!(state.open.as_ref().unwrap().staged_attachment, None);

    // Ctrl+C cancels an active paste instead of arming quit.
    update(&mut state, ctrl('v'));
    assert_eq!(
        update(&mut state, ctrl('c')),
        vec![Effect::CancelClipboardPaste]
    );
    assert!(!state.quit_armed);
    assert_eq!(state.pasting, PasteState::Cancelled);
}

#[test]
fn renders_the_live_pasting_bar_above_the_input() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut state = loaded_state();
    update(&mut state, ctrl('v'));
    update(&mut state, AppEvent::ClipboardProgress { bytes: 3_565_158 });
    let theme = crate::theme::Theme::faro_dark();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal
        .draw(|frame| {
            crate::ui::render(
                frame,
                &state,
                &crate::ui::View {
                    theme: &theme,
                    now: jiff::Timestamp::from_millisecond(1_760_000_000_000).expect("timestamp"),
                    tz: &jiff::tz::TimeZone::UTC,
                    logo_time: None,
                    images: &crate::app::image_cache::NoImages,
                },
            );
        })
        .expect("draw");
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .chunks(80)
        .map(|row| {
            row.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
                + "\n"
        })
        .collect();
    assert!(
        screen.contains("Pasting image · 3.4 MB"),
        "expected live pasting bar in:\n{screen}"
    );
    assert!(
        screen.contains("esc cancel"),
        "expected esc cancel hint in:\n{screen}"
    );
}
