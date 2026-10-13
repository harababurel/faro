//! The key table (WEBSITE.md §3.7): ten rows taken from faro's own key
//! map, so the page can't drift from the app.

use faro_app::app::keymap::{Action, Curate, Keymap};
use faro_app::app::palette::Level;
use faro_app::app::state::Focus;

use crate::placeholders::escape;

/// One row of the table §3.7 asks for (palette, switch chats, select,
/// react, reply, search, mute, themes, help, quit).
struct Spec {
    /// The actions whose preferred keys fill the key cell, via
    /// [`Keymap::hint`] (two actions render as one compact hint,
    /// `alt+j/k`).
    actions: &'static [Action],
    /// The pane the key lookup is made from.
    focus: Focus,
    /// A palette category drawn after the key (`Ctrl+P › Themes`).
    path: Option<&'static str>,
    /// The description; `None` uses the action's own title.
    desc: Option<&'static str>,
}

const SPECS: &[Spec] = &[
    Spec {
        actions: &[Action::OpenPalette],
        focus: Focus::Messages,
        path: None,
        desc: None,
    },
    Spec {
        actions: &[Action::NextConversation, Action::PreviousConversation],
        focus: Focus::Messages,
        path: None,
        desc: Some("Next / previous conversation"),
    },
    Spec {
        actions: &[Action::SelectOlder, Action::SelectNewer],
        focus: Focus::Messages,
        path: None,
        desc: Some("Select older / newer message"),
    },
    Spec {
        actions: &[Action::ReactPicker],
        focus: Focus::Messages,
        path: None,
        desc: None,
    },
    Spec {
        actions: &[Action::Reply],
        focus: Focus::Messages,
        path: None,
        desc: None,
    },
    Spec {
        actions: &[Action::SearchMessages],
        focus: Focus::Conversations,
        path: None,
        desc: None,
    },
    Spec {
        actions: &[Action::Curate(Curate::Mute)],
        focus: Focus::Conversations,
        path: None,
        desc: None,
    },
    Spec {
        actions: &[Action::OpenPalette],
        focus: Focus::Messages,
        path: Some(Level::Themes.title()),
        desc: Some("Switch theme, with live preview"),
    },
    Spec {
        actions: &[Action::Help],
        focus: Focus::Messages,
        path: None,
        desc: None,
    },
    Spec {
        actions: &[Action::Quit],
        focus: Focus::Messages,
        path: None,
        desc: None,
    },
];

/// The `<tr>` rows for the template's `{{keys}}`: a key cell followed by the
/// action's description.
pub fn table(keymap: &Keymap) -> Result<String, String> {
    let mut rows: Vec<String> = Vec::new();
    for spec in SPECS {
        let Some(label) = keymap.hint(spec.actions, spec.focus) else {
            return Err(format!("no key is bound to {:?}", spec.actions));
        };
        let mut keys = format!("<kbd>{}</kbd>", escape(&pretty(&label)));
        if let Some(path) = spec.path {
            let path_cell = format!(" › <kbd>{}</kbd>", escape(path));
            keys.push_str(&path_cell);
        }
        let desc = description(keymap, spec)?;
        rows.push(format!(
            "    <tr><th scope=\"row\">{keys}</th><td>{desc}</td></tr>"
        ));
    }
    Ok(rows.join("\n"))
}

/// The description cell: the action's own title, unless the row spells one
/// out (the combined rows) — and `Quit` names the twice-press fallback.
fn description(keymap: &Keymap, spec: &Spec) -> Result<String, String> {
    if let Some(desc) = spec.desc {
        return Ok(escape(desc));
    }
    let Some(action) = spec.actions.first() else {
        return Err("a key row needs at least one action".to_owned());
    };
    let Some(title) = action.title() else {
        return Err(format!("action {action:?} has no title"));
    };
    let mut desc = escape(title);
    if *action == Action::Quit {
        let interrupt = keymap
            .hint(&[Action::Interrupt], spec.focus)
            .ok_or_else(|| "no key is bound to Action::Interrupt".to_owned())?;
        let twice = escape(&pretty(&interrupt));
        let note = format!(" (or <kbd>{twice}</kbd> twice)");
        desc.push_str(&note);
    }
    Ok(desc)
}

/// `Keymap::hint` writes config spellings (`ctrl+p`, `alt+j/k`, `↑↓`); the
/// page shows them the way the README does: `Ctrl+P`, `Alt+J/K`.
fn pretty(label: &str) -> String {
    let mut out = String::new();
    let mut last_modified = false;
    for part in label.split('/') {
        if !out.is_empty() {
            out.push('/');
        }
        let mut segments: Vec<&str> = part.split('+').collect();
        let key = segments.pop().unwrap_or("");
        for modifier in &segments {
            let spelled = format!("{}+", cap(modifier));
            out.push_str(&spelled);
        }
        // A bare part after a modified one (`alt+j/k`'s `k`) keeps the
        // modifier's effect without repeating it.
        let modified = !segments.is_empty() || last_modified;
        out.push_str(&pretty_key(key, modified));
        last_modified = !segments.is_empty();
    }
    out
}

/// A bare key in a chord: single characters only get uppercased together
/// with a modifier (`ctrl+p` → `Ctrl+P`), so a bare `m` still means `m`,
/// not `M`. Names and function keys are capitalised (`enter` → `Enter`);
/// arrows and symbols stay as they are.
fn pretty_key(key: &str, modified: bool) -> String {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii() => {
            if modified && c.is_ascii_alphabetic() {
                c.to_uppercase().collect()
            } else {
                c.to_string()
            }
        }
        _ => cap(key),
    }
}

/// `ctrl` → `Ctrl`, `f1` → `F1`, `enter` → `Enter`, `↑↓` unchanged.
fn cap(word: &str) -> String {
    let letters: String = word.chars().take_while(char::is_ascii_alphabetic).collect();
    let rest: String = word.chars().skip(letters.chars().count()).collect();
    if !letters.is_empty() && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
        return format!("{}{rest}", letters.to_uppercase());
    }
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => {
            let mut out: String = first.to_uppercase().collect();
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_has_ten_rows_and_the_palette_key() {
        let table = table(&Keymap::default()).unwrap();
        assert_eq!(table.lines().count(), 10);
        assert!(table.contains("<kbd>Ctrl+P</kbd>"));
        assert!(table.contains("<kbd>Alt+J/K</kbd>"));
        assert!(table.contains("Quit (or <kbd>Ctrl+C</kbd> twice)"));
        assert!(table.contains("Mute or unmute"));
    }

    #[test]
    fn pretty_shows_chords_the_pages_way() {
        assert_eq!(pretty("ctrl+p"), "Ctrl+P");
        assert_eq!(pretty("alt+j/k"), "Alt+J/K");
        assert_eq!(pretty("↑↓"), "↑↓");
        assert_eq!(pretty("+"), "+");
        assert_eq!(pretty("?"), "?");
        assert_eq!(pretty("m"), "m");
        assert_eq!(pretty("f1"), "F1");
        assert_eq!(pretty("shift+tab"), "Shift+Tab");
        assert_eq!(pretty("ctrl+f"), "Ctrl+F");
    }
}
