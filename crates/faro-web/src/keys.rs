//! The demo's settings and keys (D116): an embedded `demo.toml` in the
//! config file's format, and which key presses faro takes from the browser.

use faro_app::app::keymap::{palette_action, popup_action};
use faro_app::app::state::State;
use faro_app::app::term_event::{KeyEvent, KeyModifiers};
use faro_core::config::{self, Config, Problem};

/// The demo's config file.
pub const DEMO_TOML: &str = include_str!("../demo.toml");

/// The demo's settings, and anything wrong with them (a test keeps that
/// empty).
pub fn demo_config() -> (Config, Vec<Problem>) {
    let parsed = config::parse(DEMO_TOML, &Config::default());
    (parsed.config, parsed.problems)
}

/// Whether faro does something with a Ctrl or Alt chord right now (the
/// palette, a menu, or the focused pane's bindings). Chords it doesn't use
/// stay the browser's: reload, bookmarks, history and so on. Keys without
/// Ctrl or Alt are always faro's while it has the focus.
pub fn takes(state: &State, key: KeyEvent) -> bool {
    if !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return true;
    }
    if state.palette.is_some() {
        return palette_action(&state.keymap, key).is_some();
    }
    if state.popup.is_some() {
        return popup_action(key, true).is_some() || popup_action(key, false).is_some();
    }
    state.keymap.lookup(key, state.focus).is_some()
}

#[cfg(test)]
mod tests {
    use faro_app::app::state::Focus;
    use faro_app::app::term_event::KeyCode;
    use faro_app::app::update;
    use faro_app::fixtures::loaded_state;

    use super::*;

    fn chord(c: char, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), modifiers)
    }

    #[test]
    fn the_demo_config_is_clean_and_frees_the_browsers_chords() {
        let (config, problems) = demo_config();
        assert!(problems.is_empty(), "{problems:?}");
        let mut state = loaded_state();
        update::configure(&mut state, config, problems);
        state.focus = Focus::Input;
        let ctrl = KeyModifiers::CONTROL;
        assert!(!takes(&state, chord('w', ctrl)), "closes the tab");
        assert!(!takes(&state, chord('v', ctrl)), "paste");
        assert!(!takes(&state, chord('r', ctrl)), "reload: unbound");
        assert!(takes(&state, chord('p', ctrl)), "palette");
        assert!(takes(&state, chord('k', ctrl)), "readline");
        assert!(takes(&state, chord('j', KeyModifiers::ALT)), "next chat");
        assert!(takes(&state, chord('x', KeyModifiers::NONE)));
    }
}
