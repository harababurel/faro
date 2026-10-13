//! A ratatui backend that keeps the screen as a cell buffer and remembers which
//! rows changed, so the painter repaints only those (D106). Frames come from
//! ratatui's own diff, as in the terminal; nothing here touches the browser.

use std::convert::Infallible;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect, Size};

pub struct CanvasBackend {
    buffer: Buffer,
    /// Rows to repaint, by index.
    dirty: Vec<bool>,
    cursor: Position,
    cursor_shown: bool,
    /// Cell size in CSS pixels, for `window_size`.
    cell: Size,
}

impl CanvasBackend {
    pub fn new(columns: u16, rows: u16) -> Self {
        Self {
            buffer: Buffer::empty(Rect::new(0, 0, columns, rows)),
            dirty: vec![true; usize::from(rows)],
            cursor: Position::ORIGIN,
            cursor_shown: false,
            cell: Size::new(8, 16),
        }
    }

    /// A new grid size: everything is repainted.
    pub fn resize(&mut self, columns: u16, rows: u16) {
        self.buffer = Buffer::empty(Rect::new(0, 0, columns, rows));
        self.dirty = vec![true; usize::from(rows)];
    }

    pub fn set_cell_size(&mut self, cell: Size) {
        self.cell = cell;
    }

    pub const fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// The cursor's cell, if it's shown.
    pub fn cursor(&self) -> Option<Position> {
        self.cursor_shown.then_some(self.cursor)
    }

    /// Rows that changed since the last call, in order; they count as clean
    /// afterwards.
    pub fn take_dirty(&mut self) -> Vec<u16> {
        let rows = self
            .dirty
            .iter()
            .enumerate()
            .filter(|(_, dirty)| **dirty)
            .filter_map(|(row, _)| u16::try_from(row).ok())
            .collect();
        self.dirty.fill(false);
        rows
    }

    /// Repaint everything next time (the canvas was cleared or resized).
    pub fn mark_all(&mut self) {
        self.dirty.fill(true);
    }

    /// Repaint `rows` next time (a picture moved over them, D135).
    pub fn mark_rows(&mut self, rows: std::ops::Range<u16>) {
        for row in rows {
            self.mark(row);
        }
    }

    fn mark(&mut self, row: u16) {
        if let Some(dirty) = self.dirty.get_mut(usize::from(row)) {
            *dirty = true;
        }
    }

    fn mark_cursor(&mut self) {
        if self.cursor_shown {
            self.mark(self.cursor.y);
        }
    }
}

impl Backend for CanvasBackend {
    type Error = Infallible;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Infallible>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        for (x, y, cell) in content {
            if let Some(slot) = self.buffer.cell_mut((x, y)) {
                *slot = cell.clone();
                self.mark(y);
            }
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Infallible> {
        self.mark_cursor();
        self.cursor_shown = false;
        Ok(())
    }

    fn show_cursor(&mut self) -> Result<(), Infallible> {
        self.cursor_shown = true;
        self.mark_cursor();
        Ok(())
    }

    fn get_cursor_position(&mut self) -> Result<Position, Infallible> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Infallible> {
        let position = position.into();
        if position != self.cursor {
            self.mark_cursor();
            self.cursor = position;
            self.mark_cursor();
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Infallible> {
        self.buffer.reset();
        self.mark_all();
        Ok(())
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Infallible> {
        match clear_type {
            ClearType::All => self.clear(),
            ClearType::AfterCursor
            | ClearType::BeforeCursor
            | ClearType::CurrentLine
            | ClearType::UntilNewLine => {
                // faro never asks for these; repaint rather than guess.
                self.mark_all();
                Ok(())
            }
        }
    }

    fn size(&self) -> Result<Size, Infallible> {
        Ok(self.buffer.area.as_size())
    }

    fn window_size(&mut self) -> Result<WindowSize, Infallible> {
        let columns_rows = self.buffer.area.as_size();
        Ok(WindowSize {
            columns_rows,
            pixels: Size::new(
                columns_rows.width.saturating_mul(self.cell.width),
                columns_rows.height.saturating_mul(self.cell.height),
            ),
        })
    }

    fn flush(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::widgets::Paragraph;

    use super::*;

    #[test]
    fn only_changed_rows_are_dirty() {
        let mut terminal = Terminal::new(CanvasBackend::new(10, 4)).unwrap();
        terminal.backend_mut().take_dirty();
        let Ok(_) = terminal.draw(|f| f.render_widget(Paragraph::new("\nhi"), f.area()));
        assert_eq!(terminal.backend_mut().take_dirty(), vec![1]);
        let Ok(_) = terminal.draw(|f| f.render_widget(Paragraph::new("\nhi"), f.area()));
        assert_eq!(terminal.backend_mut().take_dirty(), Vec::<u16>::new());
    }

    #[test]
    fn the_cursor_marks_the_rows_it_leaves_and_enters() {
        let mut terminal = Terminal::new(CanvasBackend::new(10, 4)).unwrap();
        let Ok(_) = terminal.draw(|f| f.set_cursor_position((1, 2)));
        assert_eq!(terminal.backend().cursor(), Some(Position::new(1, 2)));
        terminal.backend_mut().take_dirty();
        let Ok(_) = terminal.draw(|f| f.set_cursor_position((1, 3)));
        assert_eq!(terminal.backend_mut().take_dirty(), vec![2, 3]);
        let Ok(_) = terminal.draw(|_| {});
        assert_eq!(terminal.backend().cursor(), None);
        assert_eq!(terminal.backend_mut().take_dirty(), vec![3]);
    }

    #[test]
    fn a_resize_repaints_everything() {
        let mut terminal = Terminal::new(CanvasBackend::new(10, 4)).unwrap();
        terminal.backend_mut().take_dirty();
        terminal.backend_mut().resize(12, 3);
        let Ok(_) = terminal.draw(|_| {});
        assert_eq!(terminal.backend_mut().take_dirty(), vec![0, 1, 2]);
    }
}
