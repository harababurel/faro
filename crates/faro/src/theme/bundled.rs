//! The themes faro ships: its own (`builtin.rs`) and OpenCode's (D54),
//! whose files are in `crates/faro/themes/opencode/` (MIT, see the `LICENSE`
//! there and `THIRD_PARTY_NOTICES.md`). OpenCode's are read the first time a
//! theme is looked up past faro's own.

use std::sync::OnceLock;

use super::{Source, Theme, import};

/// OpenCode's theme files, by name.
const OPENCODE: [(&str, &str); 33] = [
    ("aura", include_str!("../../themes/opencode/aura.json")),
    ("ayu", include_str!("../../themes/opencode/ayu.json")),
    (
        "carbonfox",
        include_str!("../../themes/opencode/carbonfox.json"),
    ),
    (
        "catppuccin",
        include_str!("../../themes/opencode/catppuccin.json"),
    ),
    (
        "catppuccin-frappe",
        include_str!("../../themes/opencode/catppuccin-frappe.json"),
    ),
    (
        "catppuccin-macchiato",
        include_str!("../../themes/opencode/catppuccin-macchiato.json"),
    ),
    (
        "cobalt2",
        include_str!("../../themes/opencode/cobalt2.json"),
    ),
    ("cursor", include_str!("../../themes/opencode/cursor.json")),
    (
        "dracula",
        include_str!("../../themes/opencode/dracula.json"),
    ),
    (
        "everforest",
        include_str!("../../themes/opencode/everforest.json"),
    ),
    (
        "flexoki",
        include_str!("../../themes/opencode/flexoki.json"),
    ),
    ("github", include_str!("../../themes/opencode/github.json")),
    (
        "gruvbox",
        include_str!("../../themes/opencode/gruvbox.json"),
    ),
    (
        "kanagawa",
        include_str!("../../themes/opencode/kanagawa.json"),
    ),
    (
        "lucent-orng",
        include_str!("../../themes/opencode/lucent-orng.json"),
    ),
    (
        "material",
        include_str!("../../themes/opencode/material.json"),
    ),
    ("matrix", include_str!("../../themes/opencode/matrix.json")),
    (
        "mercury",
        include_str!("../../themes/opencode/mercury.json"),
    ),
    (
        "monokai",
        include_str!("../../themes/opencode/monokai.json"),
    ),
    (
        "nightowl",
        include_str!("../../themes/opencode/nightowl.json"),
    ),
    ("nord", include_str!("../../themes/opencode/nord.json")),
    (
        "one-dark",
        include_str!("../../themes/opencode/one-dark.json"),
    ),
    (
        "opencode",
        include_str!("../../themes/opencode/opencode.json"),
    ),
    ("orng", include_str!("../../themes/opencode/orng.json")),
    (
        "osaka-jade",
        include_str!("../../themes/opencode/osaka-jade.json"),
    ),
    (
        "palenight",
        include_str!("../../themes/opencode/palenight.json"),
    ),
    (
        "rosepine",
        include_str!("../../themes/opencode/rosepine.json"),
    ),
    (
        "solarized",
        include_str!("../../themes/opencode/solarized.json"),
    ),
    (
        "synthwave84",
        include_str!("../../themes/opencode/synthwave84.json"),
    ),
    (
        "tokyonight",
        include_str!("../../themes/opencode/tokyonight.json"),
    ),
    ("vercel", include_str!("../../themes/opencode/vercel.json")),
    ("vesper", include_str!("../../themes/opencode/vesper.json")),
    (
        "zenburn",
        include_str!("../../themes/opencode/zenburn.json"),
    ),
];

/// faro's own themes, in the order the palette lists them.
pub fn faro() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        vec![
            Theme::faro_dark(),
            Theme::faro_dusk(),
            Theme::faro_light(),
            Theme::system(),
        ]
    })
}

/// OpenCode's themes: every dark variant, then the light ones, each by name.
pub fn opencode() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        let (mut dark, mut light) = (Vec::new(), Vec::new());
        for (name, text) in OPENCODE {
            let themes = faro_core::theme_file::parse(text)
                .and_then(|file| import::variants(name, &file, Source::OpenCode));
            match themes {
                Ok(themes) => {
                    let mut themes = themes.into_iter();
                    dark.extend(themes.next());
                    light.extend(themes);
                }
                // Checked by the tests; never expected.
                Err(error) => tracing::warn!(name, %error, "skipping a bundled theme"),
            }
        }
        dark.append(&mut light);
        dark
    })
}
