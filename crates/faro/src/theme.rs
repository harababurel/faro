//! Colours by meaning (DESIGN.md §10). Widgets never hard-code colours; they ask
//! the theme for a role. Theme files (OpenCode's JSON format) and the `system`
//! theme come with configuration (M5); for now there is one built-in theme.

use faro_core::domain::ServiceId;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
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
}

impl Default for Theme {
    fn default() -> Self {
        Self::faro_dark()
    }
}

impl Theme {
    /// faro's own dark theme: warm lighthouse amber on near-black.
    pub const fn faro_dark() -> Self {
        Self {
            name: "faro-dark",
            background: Color::Rgb(0x10, 0x12, 0x16),
            background_panel: Color::Rgb(0x1a, 0x1d, 0x23),
            background_element: Color::Rgb(0x26, 0x2a, 0x33),
            text: Color::Rgb(0xe6, 0xe6, 0xe6),
            text_muted: Color::Rgb(0x7c, 0x83, 0x8f),
            primary: Color::Rgb(0xf5, 0xb9, 0x4a),
            secondary: Color::Rgb(0x6c, 0xa8, 0xf0),
            accent: Color::Rgb(0xb4, 0x8e, 0xf0),
            border: Color::Rgb(0x3a, 0x3f, 0x4a),
            border_active: Color::Rgb(0xf5, 0xb9, 0x4a),
            success: Color::Rgb(0x7f, 0xd8, 0x8f),
            warning: Color::Rgb(0xf5, 0xa7, 0x42),
            error: Color::Rgb(0xe8, 0x6b, 0x6b),
            unread: Color::Rgb(0x5f, 0xc8, 0xe8),
            bubble_own: Color::Rgb(0x33, 0x29, 0x1a),
            bubble_other: Color::Rgb(0x1f, 0x23, 0x2b),
            senders: [
                Color::Rgb(0xe8, 0x8a, 0x8a),
                Color::Rgb(0x8a, 0xd0, 0x8a),
                Color::Rgb(0xe8, 0xc0, 0x6a),
                Color::Rgb(0x7a, 0xb0, 0xf0),
                Color::Rgb(0xd0, 0x8a, 0xe0),
                Color::Rgb(0x6a, 0xd0, 0xc8),
                Color::Rgb(0xf0, 0xa0, 0x70),
                Color::Rgb(0xa8, 0xb8, 0xf8),
            ],
        }
    }

    pub fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.background)
    }

    pub fn muted(&self) -> Style {
        Style::new().fg(self.text_muted)
    }

    pub fn panel(&self) -> Style {
        Style::new().fg(self.text).bg(self.background_panel)
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
