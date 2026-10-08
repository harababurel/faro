//! The built-in themes (D50). All colours are faro's own; none is copied
//! from another project, so there's nothing to license. Every theme must
//! pass the contrast checks in `tests.rs`.

use ratatui::style::Color;

use super::{Source, Theme};

impl Theme {
    /// faro's own dark theme: warm lighthouse amber on near-black.
    pub fn faro_dark() -> Self {
        Self {
            name: "faro-dark".to_owned(),
            source: Source::Faro,
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
            terminal_palette: false,
        }
    }

    /// A cooler dark theme: sea-glass teal and lamp yellow on blue slate.
    pub fn faro_dusk() -> Self {
        Self {
            name: "faro-dusk".to_owned(),
            source: Source::Faro,
            background: Color::Rgb(0x16, 0x1b, 0x26),
            background_panel: Color::Rgb(0x1d, 0x23, 0x31),
            background_element: Color::Rgb(0x2a, 0x32, 0x43),
            text: Color::Rgb(0xdf, 0xe4, 0xee),
            text_muted: Color::Rgb(0x85, 0x90, 0xa6),
            primary: Color::Rgb(0x5c, 0xcf, 0xc0),
            secondary: Color::Rgb(0x7f, 0xa7, 0xf5),
            accent: Color::Rgb(0xd3, 0x9c, 0xf0),
            border: Color::Rgb(0x34, 0x3d, 0x50),
            border_active: Color::Rgb(0x5c, 0xcf, 0xc0),
            success: Color::Rgb(0x8f, 0xd6, 0x94),
            warning: Color::Rgb(0xf5, 0x9f, 0x5b),
            error: Color::Rgb(0xf0, 0x7a, 0x7a),
            unread: Color::Rgb(0xf5, 0xc2, 0x6b),
            bubble_own: Color::Rgb(0x1c, 0x3a, 0x3c),
            bubble_other: Color::Rgb(0x23, 0x2a, 0x39),
            senders: [
                Color::Rgb(0xf0, 0x8c, 0x8c),
                Color::Rgb(0x93, 0xd6, 0x8f),
                Color::Rgb(0xf2, 0xc4, 0x6d),
                Color::Rgb(0x7f, 0xb0, 0xf5),
                Color::Rgb(0xd5, 0x9c, 0xf0),
                Color::Rgb(0x6f, 0xd6, 0xcc),
                Color::Rgb(0xf5, 0xa4, 0x6b),
                Color::Rgb(0xaa, 0xb6, 0xfb),
            ],
            terminal_palette: false,
        }
    }

    /// A light theme: ink and deep amber on warm paper.
    pub fn faro_light() -> Self {
        Self {
            name: "faro-light".to_owned(),
            source: Source::Faro,
            background: Color::Rgb(0xf7, 0xf4, 0xee),
            background_panel: Color::Rgb(0xef, 0xeb, 0xe3),
            background_element: Color::Rgb(0xe2, 0xdd, 0xd2),
            text: Color::Rgb(0x24, 0x27, 0x2e),
            text_muted: Color::Rgb(0x65, 0x6a, 0x73),
            primary: Color::Rgb(0x9a, 0x5c, 0x06),
            secondary: Color::Rgb(0x2a, 0x63, 0xa8),
            accent: Color::Rgb(0x74, 0x44, 0xb5),
            border: Color::Rgb(0xc9, 0xc3, 0xb6),
            border_active: Color::Rgb(0xa8, 0x65, 0x0a),
            success: Color::Rgb(0x2b, 0x74, 0x38),
            warning: Color::Rgb(0x9c, 0x4f, 0x08),
            error: Color::Rgb(0xb8, 0x35, 0x2a),
            unread: Color::Rgb(0x0b, 0x6f, 0x8c),
            bubble_own: Color::Rgb(0xf5, 0xe2, 0xbd),
            bubble_other: Color::Rgb(0xe9, 0xe6, 0xdf),
            senders: [
                Color::Rgb(0xa8, 0x36, 0x36),
                Color::Rgb(0x2b, 0x75, 0x2f),
                Color::Rgb(0x8c, 0x60, 0x00),
                Color::Rgb(0x2c, 0x5e, 0xa8),
                Color::Rgb(0x85, 0x39, 0x9e),
                Color::Rgb(0x0f, 0x75, 0x70),
                Color::Rgb(0xa8, 0x4c, 0x18),
                Color::Rgb(0x46, 0x55, 0xad),
            ],
            terminal_palette: false,
        }
    }

    /// The terminal's own colours: its default background and text, and the
    /// 16 ANSI colours, so faro follows the terminal's palette (light or
    /// dark). Bubbles have no fill here: no ANSI colour is sure to differ
    /// from the background while keeping the text readable.
    pub fn system() -> Self {
        Self {
            name: "system".to_owned(),
            source: Source::Faro,
            background: Color::Reset,
            background_panel: Color::Reset,
            background_element: Color::DarkGray,
            text: Color::Reset,
            text_muted: Color::Reset,
            primary: Color::Yellow,
            secondary: Color::Blue,
            accent: Color::Magenta,
            border: Color::DarkGray,
            border_active: Color::Yellow,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            unread: Color::Cyan,
            bubble_own: Color::Reset,
            bubble_other: Color::Reset,
            senders: [
                Color::Red,
                Color::Green,
                Color::Yellow,
                Color::Blue,
                Color::Magenta,
                Color::Cyan,
                Color::LightRed,
                Color::LightBlue,
            ],
            terminal_palette: true,
        }
    }
}
