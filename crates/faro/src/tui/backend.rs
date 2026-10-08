//! The backend faro draws through: crossterm's, with each frame's cell
//! updates adjusted for how tmux builds emoji (D55).
//!
//! tmux receives a flag (🇷🇴) or an emoji made wide by a variation selector
//! (❤️) one code point at a time: it first stores the narrow first code
//! point, then widens that cell when the rest arrives. Widening marks the
//! next cell as the emoji's second half without clearing what was there, so
//! if that cell started another wide character, the character's own second
//! half is left behind. tmux's picture then differs from faro's, and the next
//! write near it erases the emoji (or a redraw shows garbage). Writing a
//! blank into the second cell first leaves tmux nothing to get wrong; every
//! terminal draws the same picture either way.

use std::io;
use std::num::NonZeroU16;

use ratatui::backend::{Backend as RatatuiBackend, ClearType, WindowSize};
use ratatui::buffer::{Cell, CellDiffOption};
use ratatui::layout::{Position, Size};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

/// Wraps a ratatui backend; see the module docs.
pub struct Backend<B> {
    inner: B,
    /// Reused for each frame's updates.
    updates: Vec<(u16, u16, Cell)>,
}

impl<B> Backend<B> {
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            updates: Vec::new(),
        }
    }
}

/// Add what the terminal needs before some cells (see the module docs) to a
/// frame's updates. `width` is the screen's.
pub fn prepare<'a>(
    content: impl Iterator<Item = (u16, u16, &'a Cell)>,
    width: u16,
    out: &mut Vec<(u16, u16, Cell)>,
) {
    let unit = CellDiffOption::ForcedWidth(NonZeroU16::MIN);
    for (x, y, cell) in content {
        if let Some(second) = x.checked_add(1)
            && second < width
            && cell.diff_option != unit
            && widened(cell.symbol())
        {
            let mut blank = cell.clone();
            blank.set_symbol(" ");
            blank.diff_option = CellDiffOption::None;
            out.push((second, y, blank));
        }
        out.push((x, y, cell.clone()));
    }
}

/// A wide grapheme whose first code point is narrow: tmux widens its cell
/// after the fact (flags, ❤️, keycaps).
fn widened(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    !symbol.contains('\x1b')
        && chars.next().is_some_and(|first| first.width() == Some(1))
        && chars.next().is_some()
        && symbol.width() > 1
}

impl<B: RatatuiBackend<Error = io::Error>> RatatuiBackend for Backend<B> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let width = self.inner.size()?.width;
        let mut updates = std::mem::take(&mut self.updates);
        updates.clear();
        prepare(content, width, &mut updates);
        let drawn = self
            .inner
            .draw(updates.iter().map(|(x, y, cell)| (*x, *y, cell)));
        self.updates = updates;
        drawn
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Style};

    use super::*;

    fn sent(prev: &Buffer, next: &Buffer) -> Vec<(u16, String)> {
        let mut out = Vec::new();
        prepare(prev.diff(next).into_iter(), next.area.width, &mut out);
        out.into_iter()
            .map(|(x, _, cell)| (x, cell.symbol().to_owned()))
            .collect()
    }

    #[test]
    fn widened_emoji_get_their_second_cell_cleared_first() {
        let prev = Buffer::empty(Rect::new(0, 0, 8, 1));
        let mut next = Buffer::empty(Rect::new(0, 0, 8, 1));
        next.set_string(0, 0, "a🇷🇴b❤️😂", Style::new().bg(Color::Red));
        crate::ui::finish_for_tests(&mut next);
        let sent = sent(&prev, &next);
        // The blank goes into the hidden cell before the emoji itself; a
        // plain wide emoji (😂) needs nothing.
        assert_eq!(
            sent,
            vec![
                (0, "a".to_owned()),
                (2, " ".to_owned()),
                (1, "🇷🇴".to_owned()),
                (3, "b".to_owned()),
                (5, " ".to_owned()),
                (4, "❤️".to_owned()),
                (6, "😂".to_owned()),
            ]
        );
    }

    #[test]
    fn the_blank_keeps_the_emoji_style() {
        let prev = Buffer::empty(Rect::new(0, 0, 4, 1));
        let mut next = Buffer::empty(Rect::new(0, 0, 4, 1));
        next.set_string(0, 0, "🇩🇪", Style::new().bg(Color::Blue));
        let mut out = Vec::new();
        prepare(prev.diff(&next).into_iter(), 4, &mut out);
        assert_eq!(out[0].2.bg, Color::Blue);
    }

    #[test]
    fn what_needs_widening() {
        assert!(widened("🇷🇴"));
        assert!(widened("❤️"));
        assert!(widened("🏳️‍🌈"));
        assert!(!widened("😂"));
        assert!(!widened("🧗‍♀️"));
        assert!(!widened("a"));
        assert!(!widened("é"));
        assert!(!widened("日"));
    }
}
