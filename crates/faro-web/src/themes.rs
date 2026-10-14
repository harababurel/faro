//! The page's theme menu (WEBSITE.md §4.5): every theme the demo has, in
//! the app's order, grouped the way a visitor scans them. Taken from
//! `Theme::all` with no user themes, so the menu can't name a theme the demo
//! doesn't have.

use faro_app::theme::{Source, Theme, UserThemes, luminance};
use ratatui::style::Color;

/// faro's own themes.
pub const FARO: &str = "faro";
/// OpenCode's themes with a dark background.
pub const OPENCODE_DARK: &str = "OpenCode · dark";
/// OpenCode's themes with a light background.
pub const OPENCODE_LIGHT: &str = "OpenCode · light";

/// `(name, group)` for every theme, faro's first, then OpenCode's dark ones,
/// then its light ones, each group in the app's order.
#[must_use]
pub fn menu() -> Vec<(String, &'static str)> {
    let user = UserThemes::default();
    let mut themes: Vec<(String, &'static str)> = Theme::all(&user)
        .into_iter()
        .map(|theme| (theme.name.clone(), group(theme)))
        .collect();
    let rank = |group: &str| match group {
        FARO => 0,
        OPENCODE_DARK => 1,
        _ => 2,
    };
    // Stable: each group keeps the app's order.
    themes.sort_by_key(|(_, group)| rank(group));
    themes
}

fn group(theme: &Theme) -> &'static str {
    let light = matches!(theme.background, Color::Rgb(r, g, b) if luminance((r, g, b)) > 0.5);
    match theme.source {
        Source::Faro | Source::User => FARO,
        Source::OpenCode if light => OPENCODE_LIGHT,
        Source::OpenCode => OPENCODE_DARK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_once_faro_first() {
        let menu = menu();
        let user = UserThemes::default();
        assert_eq!(menu.len(), Theme::all(&user).len());
        let names: Vec<&str> = menu.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names.get(..4),
            Some(["faro-dark", "faro-dusk", "faro-light", "system"].as_slice())
        );
        let groups: Vec<&str> = menu.iter().map(|&(_, group)| group).collect();
        assert!(groups.is_sorted_by_key(|g| {
            [FARO, OPENCODE_DARK, OPENCODE_LIGHT]
                .iter()
                .position(|x| x == g)
        }));
    }

    #[test]
    fn light_variants_are_light() {
        let menu = menu();
        let of = |name: &str| menu.iter().find(|(n, _)| n == name).map(|&(_, g)| g);
        assert_eq!(of("nord"), Some(OPENCODE_DARK));
        assert_eq!(of("catppuccin"), Some(OPENCODE_DARK));
        assert!(
            menu.iter()
                .filter(|(name, _)| name.ends_with("-light") && !name.starts_with("faro"))
                .all(|&(_, group)| group == OPENCODE_LIGHT),
            "{menu:?}"
        );
    }
}
