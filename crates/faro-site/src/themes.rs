//! The theme chips under the demo panel (WEBSITE.md §4.5): the handful the page
//! features plus "all…", every name taken from `Theme::all` with no user themes,
//! so the page can't drift from the app or name a theme it doesn't have.

use faro_app::theme::{Theme, UserThemes};

use crate::placeholders::escape;

/// The themes the chips offer, in chip order (§4.5). Each one is looked up in
/// the app's own list, so a chip carries the resolved name and a preset the app
/// no longer has is left out rather than replaced by a name the visitor didn't
/// ask for. `every_featured_name_is_a_bundled_theme` is what fails loudly when
/// a name here goes stale.
const FEATURED: [&str; 7] = [
    "faro-dark",
    "faro-dusk",
    "faro-light",
    "catppuccin",
    "nord",
    "gruvbox",
    "tokyonight",
];

/// The chips for the template's `{{themes}}` (the row is hidden until the live
/// demo lands, §4.5): one per featured theme, then "all…" for the demo's palette.
pub fn chips() -> String {
    let user = UserThemes::default();
    let all = Theme::all(&user);
    let mut chips: Vec<String> = FEATURED
        .iter()
        .filter_map(|name| all.iter().find(|theme| theme.name == *name))
        .map(|theme| chip(&theme.name))
        .collect();
    chips.push(
        "<button type=\"button\" data-tool=\"themes\">all…\
         <span class=\"sr-only\"> themes, in faro's palette</span></button>"
            .to_owned(),
    );
    chips.join("\n")
}

/// One theme chip. `aria-pressed` says which one is showing; demo.js sets it
/// when the visitor picks a theme or faro's palette does.
fn chip(name: &str) -> String {
    let name = escape(name);
    format!("<button type=\"button\" data-theme=\"{name}\" aria-pressed=\"false\">{name}</button>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> UserThemes {
        UserThemes::default()
    }

    /// The theme names a chip row offers, in order.
    fn offered(chips: &str) -> Vec<String> {
        chips
            .split("data-theme=\"")
            .skip(1)
            .map(|rest| rest.split('"').next().unwrap_or_default().to_owned())
            .collect()
    }

    fn featured() -> Vec<String> {
        FEATURED.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The mechanical guard: a featured name that stops existing (renamed,
    /// dropped, a typo here) fails here instead of quietly changing the row.
    #[test]
    fn every_featured_name_is_a_bundled_theme() {
        let real: Vec<String> = Theme::all(&user())
            .into_iter()
            .map(|theme| theme.name.clone())
            .collect();
        for name in FEATURED {
            assert!(real.iter().any(|have| have == name), "{name} isn't a theme");
        }
    }

    #[test]
    fn chips_offer_the_featured_themes_once_each() {
        assert_eq!(offered(&chips()), featured());
    }

    #[test]
    fn chips_are_the_featured_seven_not_every_theme() {
        let chips = chips();
        assert_eq!(
            chips.matches("<button").count(),
            FEATURED.len() + 1,
            "seven + all…"
        );
        assert!(
            Theme::all(&user()).len() > FEATURED.len() + 1,
            "the app has more themes than the row shows"
        );
        for extra in ["system", "solarized", "zenburn"] {
            assert!(
                !chips.contains(&format!("data-theme=\"{extra}\"")),
                "{extra} is left to the palette"
            );
        }
    }

    #[test]
    fn the_all_chip_appears_once() {
        let chips = chips();
        assert_eq!(chips.matches("data-tool=\"themes\"").count(), 1);
        assert!(chips.contains(">all…<"), "the chip reads all…");
    }

    #[test]
    fn every_chip_starts_unpressed() {
        let chips = chips();
        assert_eq!(
            chips.matches("aria-pressed=\"false\"").count(),
            FEATURED.len()
        );
    }

    #[test]
    fn chip_names_are_escaped() {
        assert_eq!(
            chip("<b>&\"x"),
            "<button type=\"button\" data-theme=\"&lt;b&gt;&amp;&quot;x\" \
             aria-pressed=\"false\">&lt;b&gt;&amp;&quot;x</button>"
        );
    }
}
