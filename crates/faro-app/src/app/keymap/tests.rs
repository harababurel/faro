//! Key map tests: the defaults behave as DESIGN.md §7.6 says, and rebinding
//! works the way the config file (M5) will use it.

use std::collections::HashSet;

use super::*;

fn press(code: KeyCode, modifiers: KeyModifiers, focus: Focus) -> Option<Action> {
    Keymap::default().lookup(KeyEvent::new(code, modifiers), focus)
}

fn key(text: &str) -> KeyEvent {
    let chord = KeyChord::parse(text).unwrap();
    KeyEvent::new(chord.code, chord.modifiers)
}

#[test]
fn every_default_parses_and_every_name_is_unique() {
    for (context, text, action) in defaults::BINDINGS {
        let chord = KeyChord::parse(text);
        assert!(chord.is_some(), "{text} in {context:?} doesn't parse");
        assert!(action.name().is_some(), "{action:?} has no name");
    }
    let mut names = HashSet::new();
    let mut actions = Vec::new();
    for (name, title, action) in defaults::NAMES {
        assert!(names.insert(*name), "{name} twice");
        assert_ne!(*title, "");
        assert!(!actions.contains(action), "{action:?} named twice");
        actions.push(*action);
        assert_eq!(Action::parse(name), Some(*action));
    }
}

#[test]
fn every_named_action_has_a_default_key() {
    let keymap = Keymap::default();
    for (name, _, action) in defaults::NAMES {
        let bound = [Focus::Input, Focus::Conversations, Focus::Messages]
            .into_iter()
            .any(|focus| !keymap.keys_for(*action, focus).is_empty());
        assert!(bound, "{name} has no key");
    }
}

#[test]
fn no_default_key_is_bound_twice_in_a_context() {
    let mut seen = HashSet::new();
    for (context, text, _) in defaults::BINDINGS {
        let chord = KeyChord::parse(text).unwrap();
        assert!(
            seen.insert((context.name(), chord)),
            "{text} twice in {context:?}"
        );
    }
}

#[test]
fn letters_type_in_the_input_and_navigate_elsewhere() {
    let j = |focus| press(KeyCode::Char('j'), KeyModifiers::NONE, focus);
    assert_eq!(j(Focus::Input), Some(Action::Edit(Edit::Insert('j'))));
    assert_eq!(j(Focus::Conversations), Some(Action::NextConversation));
    assert_eq!(j(Focus::Messages), Some(Action::SelectNewer));
    assert_eq!(
        press(KeyCode::Char('x'), KeyModifiers::NONE, Focus::Messages),
        Some(Action::PlayVoiceNote)
    );
    assert_eq!(
        press(KeyCode::Char('t'), KeyModifiers::NONE, Focus::Messages),
        Some(Action::TranscribeVoiceNote)
    );
    // Unbound letters do nothing outside the input.
    assert_eq!(
        press(KeyCode::Char('z'), KeyModifiers::NONE, Focus::Messages),
        None
    );
}

#[test]
fn shift_tab_and_capitals_work() {
    assert_eq!(
        press(KeyCode::BackTab, KeyModifiers::SHIFT, Focus::Input),
        Some(Action::FocusPrevious)
    );
    assert_eq!(
        press(KeyCode::Char('G'), KeyModifiers::SHIFT, Focus::Messages),
        Some(Action::SelectNewest)
    );
    assert_eq!(
        press(KeyCode::Char('H'), KeyModifiers::SHIFT, Focus::Input),
        Some(Action::Edit(Edit::Insert('H')))
    );
    assert_eq!(
        press(
            KeyCode::Char('J'),
            KeyModifiers::ALT | KeyModifiers::SHIFT,
            Focus::Input
        ),
        Some(Action::NextConversation)
    );
}

#[test]
fn editing_keys() {
    let input = |code, modifiers| press(code, modifiers, Focus::Input);
    let edit = |e| Some(Action::Edit(e));
    assert_eq!(
        input(KeyCode::Enter, KeyModifiers::NONE),
        Some(Action::Send)
    );
    assert_eq!(
        input(KeyCode::Enter, KeyModifiers::ALT),
        edit(Edit::NewLine)
    );
    assert_eq!(
        input(KeyCode::Enter, KeyModifiers::SHIFT),
        edit(Edit::NewLine)
    );
    assert_eq!(
        input(KeyCode::Char('j'), KeyModifiers::CONTROL),
        edit(Edit::NewLine)
    );
    assert_eq!(
        input(KeyCode::Backspace, KeyModifiers::ALT),
        edit(Edit::DeleteWordBackward)
    );
    assert_eq!(
        input(KeyCode::Char('w'), KeyModifiers::CONTROL),
        edit(Edit::DeleteWordBackward)
    );
    assert_eq!(
        input(KeyCode::Char('b'), KeyModifiers::ALT),
        edit(Edit::WordLeft)
    );
    assert_eq!(
        input(KeyCode::Left, KeyModifiers::CONTROL),
        edit(Edit::WordLeft)
    );
    // Alt+J/K stay conversation switching, even in the input.
    assert_eq!(
        input(KeyCode::Char('k'), KeyModifiers::ALT),
        Some(Action::PreviousConversation)
    );
    // Ctrl+K is "delete to end of line" in the input.
    assert_eq!(
        input(KeyCode::Char('k'), KeyModifiers::CONTROL),
        edit(Edit::DeleteToLineEnd)
    );
    // Other Ctrl/Alt letters don't type.
    assert_eq!(input(KeyCode::Char('x'), KeyModifiers::CONTROL), None);
}

#[test]
fn popups_take_keys_but_not_quitting() {
    let popup = |code, modifiers| popup_action(KeyEvent::new(code, modifiers), false);
    assert_eq!(
        popup(KeyCode::Char('j'), KeyModifiers::NONE),
        Some(Action::Popup(PopupKey::Next))
    );
    assert_eq!(
        popup(KeyCode::Char('3'), KeyModifiers::NONE),
        Some(Action::Popup(PopupKey::Char('3')))
    );
    assert_eq!(
        popup(KeyCode::Char('+'), KeyModifiers::SHIFT),
        Some(Action::Popup(PopupKey::Char('+')))
    );
    assert_eq!(
        popup(KeyCode::Esc, KeyModifiers::NONE),
        Some(Action::Popup(PopupKey::Close))
    );
    assert_eq!(
        popup(KeyCode::Char('q'), KeyModifiers::CONTROL),
        Some(Action::Quit)
    );
    assert_eq!(popup(KeyCode::Char('j'), KeyModifiers::ALT), None);
    // h/l: shortcuts in menus, moves in the reaction picker.
    assert_eq!(
        popup(KeyCode::Char('h'), KeyModifiers::NONE),
        Some(Action::Popup(PopupKey::Char('h')))
    );
    assert_eq!(
        popup_action(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), true),
        Some(Action::Popup(PopupKey::Previous))
    );
}

#[test]
fn curation_keys_work_in_the_sidebar_only() {
    let sidebar = |c| press(KeyCode::Char(c), KeyModifiers::NONE, Focus::Conversations);
    assert_eq!(sidebar('p'), Some(Action::Curate(Curate::Pin)));
    assert_eq!(sidebar(' '), Some(Action::Curate(Curate::Menu)));
    // Capitals arrive with SHIFT, which is ignored.
    assert_eq!(
        press(
            KeyCode::Char('A'),
            KeyModifiers::SHIFT,
            Focus::Conversations
        ),
        Some(Action::Curate(Curate::SwitchView))
    );
    assert_eq!(
        press(
            KeyCode::Char('K'),
            KeyModifiers::SHIFT,
            Focus::Conversations
        ),
        Some(Action::Curate(Curate::MoveUp))
    );
    assert_eq!(
        press(KeyCode::Char('p'), KeyModifiers::NONE, Focus::Messages),
        None
    );
    // Alt+digit jumps to a pinned conversation from anywhere.
    assert_eq!(
        press(KeyCode::Char('3'), KeyModifiers::ALT, Focus::Input),
        Some(Action::JumpToPinned(3))
    );
    assert_eq!(
        press(KeyCode::Char('0'), KeyModifiers::ALT, Focus::Input),
        None
    );
    assert_eq!(
        press(KeyCode::Char('3'), KeyModifiers::NONE, Focus::Input),
        Some(Action::Edit(Edit::Insert('3')))
    );
}

#[test]
fn releases_are_ignored() {
    let mut key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
    key.kind = KeyEventKind::Release;
    assert_eq!(Keymap::default().lookup(key, Focus::Input), None);
}

#[test]
fn rebinding_from_config_entries() {
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.apply("global", "ctrl+n", "conversation.next"),
        Ok(())
    );
    assert_eq!(keymap.apply("global", "alt+j", "none"), Ok(()));
    assert_eq!(
        keymap.lookup(key("ctrl+n"), Focus::Input),
        Some(Action::NextConversation)
    );
    // Unbound in the input: Alt+J now does nothing there.
    assert_eq!(keymap.lookup(key("alt+j"), Focus::Input), None);
    // The new key is the one hints show.
    assert_eq!(
        keymap.hint(&[Action::NextConversation], Focus::Input),
        Some("ctrl+n".into())
    );

    // A pane binding wins over a global one, only in that pane.
    assert_eq!(keymap.apply("messages", "ctrl+q", "message.copy"), Ok(()));
    assert_eq!(
        keymap.lookup(key("ctrl+q"), Focus::Messages),
        Some(Action::Copy)
    );
    assert_eq!(
        keymap.lookup(key("ctrl+q"), Focus::Input),
        Some(Action::Quit)
    );
    assert_eq!(keymap.keys_for(Action::Quit, Focus::Messages), vec![]);

    // Rebinding a key replaces what it did.
    assert_eq!(keymap.apply("messages", "r", "message.react"), Ok(()));
    assert_eq!(
        keymap.lookup(key("r"), Focus::Messages),
        Some(Action::ReactPicker)
    );
    assert_eq!(keymap.keys_for(Action::Reply, Focus::Messages), vec![]);
}

#[test]
fn config_mistakes_are_reported() {
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.apply("sidebar", "x", "app.quit"),
        Err(KeymapError::Context("sidebar".into()))
    );
    assert_eq!(
        keymap.apply("global", "hyper+x", "app.quit"),
        Err(KeymapError::Key("hyper+x".into()))
    );
    assert_eq!(
        keymap.apply("global", "ctrl+x", "app.explode"),
        Err(KeymapError::Action("app.explode".into()))
    );
    assert_eq!(keymap, Keymap::default(), "mistakes change nothing");
    assert_eq!(
        KeymapError::Key("hyper+x".into()).to_string(),
        "unknown key \"hyper+x\""
    );
}

#[test]
fn hints_name_the_preferred_keys() {
    let keymap = Keymap::default();
    let hint = |actions: &[Action], focus| keymap.hint(actions, focus);
    assert_eq!(
        hint(
            &[Action::NextConversation, Action::PreviousConversation],
            Focus::Input
        ),
        Some("alt+j/k".into())
    );
    assert_eq!(
        hint(
            &[Action::PreviousConversation, Action::NextConversation],
            Focus::Conversations
        ),
        Some("↑↓".into())
    );
    assert_eq!(hint(&[Action::Send], Focus::Input), Some("enter".into()));
    assert_eq!(
        hint(&[Action::Curate(Curate::SwitchView)], Focus::Conversations),
        Some("A".into())
    );
    assert_eq!(hint(&[Action::Send], Focus::Messages), None);
    assert_eq!(
        hint(&[Action::Escape, Action::Quit], Focus::Input),
        Some("esc/ctrl+q".into())
    );
}
