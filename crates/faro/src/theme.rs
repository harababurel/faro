//! Colours by meaning (DESIGN.md §10). Widgets never hard-code colours; they ask
//! the theme for a role. faro's own themes are in `builtin.rs`; OpenCode's
//! are bundled (`bundled.rs`) and the user's own files are read from
//! `themes/` beside the config file (`user.rs`), both through `import.rs`
//! (D54). Terminals without truecolor get each colour's nearest xterm-256
//! colour, and those with only 16 colours get the `system` theme
//! (`colors.rs`); both happen once, when the theme is loaded, not per frame.

mod builtin;
mod bundled;
mod colors;
mod import;
#[cfg(test)]
mod tests;
mod user;

use faro_core::domain::ServiceId;
use ratatui::style::{Color, Modifier, Style};

pub use self::colors::ColorDepth;
pub use self::user::{ThemeText, UserThemes};

/// Where a theme comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// faro's own (`builtin.rs`).
    Faro,
    /// Bundled from OpenCode.
    OpenCode,
    /// A file in the user's `themes/` directory.
    User,
}

impl Source {
    /// How the palette labels it (faro's own need no label).
    pub const fn label(self) -> Option<&'static str> {
        match self {
            Self::Faro => None,
            Self::OpenCode => Some("OpenCode"),
            Self::User => Some("your file"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub name: String,
    pub source: Source,
    pub background: Color,
    /// Input box, popups.
    pub background_panel: Color,
    /// Selected rows.
    pub background_element: Color,
    pub text: Color,
    pub text_muted: Color,
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub border: Color,
    pub border_active: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub unread: Color,
    /// Own messages in the bubbles layout.
    pub bubble_own: Color,
    /// Others' messages in the bubbles layout.
    pub bubble_other: Color,
    /// Sender names are coloured by person, picked from here.
    pub senders: [Color; 8],
    /// The colours are the terminal's own (the `system` theme): faro doesn't
    /// know what they look like, so the background and text are the
    /// terminal's defaults, selections and badges are drawn in reverse video
    /// and muted text is dim instead of a darker colour.
    pub terminal_palette: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self::faro_dark()
    }
}

impl Theme {
    /// The theme called `name`: the user's own first (except `system`),
    /// then faro's, then OpenCode's.
    pub fn find<'a>(name: &str, user: &'a UserThemes) -> Option<&'a Self> {
        let own = (name != "system")
            .then(|| user.themes.iter().find(|t| t.name == name))
            .flatten();
        own.or_else(|| bundled::faro().iter().find(|t| t.name == name))
            .or_else(|| bundled::opencode().iter().find(|t| t.name == name))
    }

    /// Every theme, in the order the palette lists them: faro's, the
    /// user's, then OpenCode's dark and light ones. A user's theme hides a
    /// built-in one of the same name.
    pub fn all(user: &UserThemes) -> Vec<&Self> {
        let shown = |t: &&Self| t.name == "system" || !user.themes.iter().any(|u| u.name == t.name);
        bundled::faro()
            .iter()
            .filter(shown)
            .chain(user.themes.iter())
            .chain(bundled::opencode().iter().filter(shown))
            .collect()
    }

    /// This theme for a terminal showing `depth` colours.
    #[must_use]
    pub fn for_depth(self, depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::Truecolor => self,
            ColorDepth::Indexed256 => self.clone().map(colors::nearest_256).keep_apart(&self),
            // Picking 16 colours for a palette faro can't see would guess;
            // the system theme leaves them to the terminal.
            ColorDepth::Ansi16 => Self::system(),
        }
    }

    /// Every colour passed through `f`.
    fn map(self, f: impl Fn(Color) -> Color) -> Self {
        Self {
            background: f(self.background),
            background_panel: f(self.background_panel),
            background_element: f(self.background_element),
            text: f(self.text),
            text_muted: f(self.text_muted),
            primary: f(self.primary),
            secondary: f(self.secondary),
            accent: f(self.accent),
            border: f(self.border),
            border_active: f(self.border_active),
            success: f(self.success),
            warning: f(self.warning),
            error: f(self.error),
            unread: f(self.unread),
            bubble_own: f(self.bubble_own),
            bubble_other: f(self.bubble_other),
            senders: self.senders.map(&f),
            ..self
        }
    }

    /// After rounding to 256 colours: surfaces drawn next to each other
    /// that differ in the theme are kept different (the next nearest colour)
    /// even where they rounded to the same one, e.g. near-black greys.
    fn keep_apart(mut self, original: &Self) -> Self {
        let near = colors::nearest_256_except;
        self.background_panel = near(original.background_panel, &[self.background]);
        self.background_element = near(
            original.background_element,
            &[self.background, self.background_panel],
        );
        self.bubble_other = near(original.bubble_other, &[self.background]);
        self.bubble_own = near(original.bubble_own, &[self.background, self.bubble_other]);
        self
    }

    pub fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.background)
    }

    pub fn muted(&self) -> Style {
        let style = Style::new().fg(self.text_muted);
        if self.terminal_palette {
            style.add_modifier(Modifier::DIM)
        } else {
            style
        }
    }

    pub fn panel(&self) -> Style {
        Style::new().fg(self.text).bg(self.background_panel)
    }

    /// A selected row (sidebar, palette, menus, the selected message).
    pub fn selected(&self) -> Style {
        if self.terminal_palette {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new().bg(self.background_element)
        }
    }

    /// Text in the background colour on `colour` (badges).
    pub fn inverse(&self, colour: Color) -> Style {
        if self.terminal_palette {
            // The default background isn't a colour faro can name.
            Style::new()
                .fg(colour)
                .bg(Color::Reset)
                .add_modifier(Modifier::REVERSED)
        } else {
            Style::new().fg(self.background).bg(colour)
        }
    }

    pub fn border(&self, active: bool) -> Style {
        Style::new().fg(if active {
            self.border_active
        } else {
            self.border
        })
    }

    /// The same person always gets the same colour (stable across runs and
    /// versions: derived from the UUID, not a hasher).
    pub fn sender(&self, id: ServiceId) -> Style {
        let index = usize::try_from(id.uuid().as_u128() % self.senders.len() as u128).unwrap_or(0);
        let colour = self.senders.get(index).copied().unwrap_or(self.text);
        Style::new().fg(colour).add_modifier(Modifier::BOLD)
    }
}

/// The theme in use, loaded again only when its name, the colour depth or
/// the user's themes change (the app loop asks every frame).
#[derive(Debug, Default)]
pub struct Themes {
    loaded: Option<(String, ColorDepth, u64)>,
    theme: Theme,
}

impl Themes {
    /// The theme called `name` (the default one if there's none: that was
    /// reported when the config or the theme files were read).
    pub fn get(&mut self, name: &str, depth: ColorDepth, user: &UserThemes) -> &Theme {
        let fresh = self
            .loaded
            .as_ref()
            .is_some_and(|(n, d, g)| n == name && *d == depth && *g == user.generation);
        if !fresh {
            tracing::info!(theme = name, ?depth, "loading the theme");
            let theme = Theme::find(name, user).cloned().unwrap_or_default();
            self.theme = theme.for_depth(depth);
            self.loaded = Some((name.to_owned(), depth, user.generation));
        }
        &self.theme
    }
}
