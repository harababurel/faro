//! The theme chips under the demo panel (WEBSITE.md §4.5): every bundled
//! theme, taken from `Theme::all` with no user themes, so the page can't
//! drift from the app.

use faro_app::theme::{Theme, UserThemes};

use crate::placeholders::escape;

/// One `<button>` per theme, for the template's `{{themes}}` (the row is
/// hidden until the live demo lands in Phase 4).
pub fn chips() -> String {
    Theme::all(&UserThemes::default())
        .iter()
        .map(|theme| {
            let name = escape(&theme.name);
            format!("<button type=\"button\" data-theme=\"{name}\">{name}</button>")
        })
        .collect::<Vec<String>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_list_every_bundled_theme_once() {
        let chips = chips();
        let count = Theme::all(&UserThemes::default()).len();
        assert_eq!(chips.matches("<button").count(), count);
        assert!(count >= 35);
        for name in [
            "faro-dark",
            "faro-dusk",
            "faro-light",
            "system",
            "nord",
            "catppuccin",
        ] {
            assert!(chips.contains(&format!("data-theme=\"{name}\"")));
        }
    }
}
