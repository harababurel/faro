//! The message being written: text plus a cursor, with the editing operations
//! the input box supports (M4 task 2). Positions are byte offsets that always
//! sit on grapheme boundaries, so an emoji or accented letter moves and
//! deletes as one character.

use faro_core::domain::{MessageBody, StyleSpan, TextStyle};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    text: String,
    cursor: usize,
    styles: Vec<StyleSpan>,
    active_styles: Vec<TextStyle>,
}

impl Input {
    /// Text with the cursor at the end (e.g. a restored draft).
    pub fn new(text: &str) -> Self {
        let mut input = Self::default();
        input.insert_str(text);
        input
    }

    /// Styled text with the cursor at the end (e.g. when editing a message).
    pub fn with_styles(text: &str, styles: Vec<StyleSpan>) -> Self {
        let mut input = Self::new(text);
        input.set_styles(styles);
        input
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Byte offset of the cursor in `text()`.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn styles(&self) -> &[StyleSpan] {
        &self.styles
    }

    pub fn active_styles(&self) -> &[TextStyle] {
        &self.active_styles
    }

    pub fn set_styles(&mut self, styles: Vec<StyleSpan>) {
        self.styles = MessageBody::with_styles(&self.text, styles)
            .styles()
            .to_vec();
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Insert at the cursor. Tabs become spaces, `\r\n` and bare `\r` (sent by
    /// some terminals for pasted line breaks) become `\n`, and other control
    /// characters (stray escape codes) are dropped, so the text only ever holds
    /// printable characters and `\n`.
    pub fn insert_str(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let clean: String = normalized
            .chars()
            .map(|c| if c == '\t' { ' ' } else { c })
            .filter(|c| *c == '\n' || !c.is_control())
            .collect();
        if clean.is_empty() {
            return;
        }
        let at = self.cursor;
        let len = clean.len();
        self.text.insert_str(at, &clean);
        self.cursor += len;
        for span in &mut self.styles {
            if span.start >= at {
                span.start += len;
            } else if span.start + span.len > at {
                span.len += len;
            }
        }
        for &style in &self.active_styles {
            self.styles.push(StyleSpan {
                start: at,
                len,
                style,
            });
        }
        self.normalize_styles();
    }

    /// Replace `range` (which must lie on char boundaries) with `replacement`
    /// and place the cursor at the end of the inserted text.
    pub fn replace_range(&mut self, range: std::ops::Range<usize>, replacement: &str) {
        if self.text.get(range.clone()).is_some() {
            self.remove(range.start, range.end);
            self.cursor = range.start;
            self.insert_str(replacement);
        }
    }

    /// Toggle `style` for the word touching the cursor, or toggle `style` in
    /// `active_styles` for upcoming characters when at whitespace / line start.
    pub fn toggle_style(&mut self, style: TextStyle) {
        if let Some(pos) = self.active_styles.iter().position(|&s| s == style) {
            self.active_styles.remove(pos);
            return;
        }
        if let Some(range) = self.word_around_cursor() {
            if self.range_has_style(range.clone(), style) {
                self.remove_style_in_range(range, style);
            } else {
                self.styles.push(StyleSpan {
                    start: range.start,
                    len: range.len(),
                    style,
                });
                self.normalize_styles();
            }
        } else {
            self.active_styles.push(style);
        }
    }

    pub fn insert(&mut self, c: char) {
        let mut buffer = [0; 4];
        self.insert_str(c.encode_utf8(&mut buffer));
    }

    pub fn newline(&mut self) {
        self.insert('\n');
    }

    pub fn backspace(&mut self) {
        let start = self.previous_boundary(self.cursor);
        self.remove(start, self.cursor);
    }

    pub fn delete(&mut self) {
        let end = self.next_boundary(self.cursor);
        self.remove(self.cursor, end);
    }

    pub fn left(&mut self) {
        self.cursor = self.previous_boundary(self.cursor);
        self.active_styles.clear();
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary(self.cursor);
        self.active_styles.clear();
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
        self.active_styles.clear();
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_end();
        self.active_styles.clear();
    }

    pub fn delete_word_back(&mut self) {
        let start = self.word_start();
        self.remove(start, self.cursor);
    }

    pub fn delete_word_forward(&mut self) {
        let end = self.word_end();
        self.remove(self.cursor, end);
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start(self.cursor);
        self.active_styles.clear();
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end(self.cursor);
        self.active_styles.clear();
    }

    /// Delete back to the start of the line (Alt+U).
    pub fn delete_to_line_start(&mut self) {
        let start = self.line_start(self.cursor);
        self.remove(start, self.cursor);
    }

    /// Delete to the end of the line, or the line break if already there
    /// (Ctrl+K, as in Emacs and shells).
    pub fn delete_to_line_end(&mut self) {
        let end = self.line_end(self.cursor);
        let end = if end == self.cursor {
            self.next_boundary(end)
        } else {
            end
        };
        self.remove(self.cursor, end);
    }

    /// Move to the previous line, keeping the column where possible. Returns
    /// `false` on the first line (the caller may use ↑ for something else).
    pub fn up(&mut self) -> bool {
        let start = self.line_start(self.cursor);
        let Some(previous_end) = start.checked_sub(1) else {
            return false;
        };
        let column = self.slice(start, self.cursor).width();
        let previous_start = self.line_start(previous_end);
        self.cursor = self.at_column(previous_start, previous_end, column);
        self.active_styles.clear();
        true
    }

    /// Move to the next line; `false` on the last line.
    pub fn down(&mut self) -> bool {
        let end = self.line_end(self.cursor);
        if end >= self.text.len() {
            return false;
        }
        let column = self
            .slice(self.line_start(self.cursor), self.cursor)
            .width();
        let next_start = end + 1;
        let next_end = self.line_end(next_start);
        self.cursor = self.at_column(next_start, next_end, column);
        self.active_styles.clear();
        true
    }

    // --- helpers ---

    fn slice(&self, start: usize, end: usize) -> &str {
        self.text.get(start..end).unwrap_or("")
    }

    fn remove(&mut self, start: usize, end: usize) {
        if start < end && end <= self.text.len() {
            let removed = end - start;
            self.text.replace_range(start..end, "");
            self.cursor = start;
            self.styles = self
                .styles
                .drain(..)
                .filter_map(|span| {
                    let s = span.start;
                    let e = span.start + span.len;
                    if e <= start {
                        Some(span)
                    } else if s >= end {
                        Some(StyleSpan {
                            start: s - removed,
                            len: span.len,
                            style: span.style,
                        })
                    } else {
                        let ns = s.min(start);
                        let ne = start + e.saturating_sub(end);
                        (ne > ns).then_some(StyleSpan {
                            start: ns,
                            len: ne - ns,
                            style: span.style,
                        })
                    }
                })
                .collect();
            self.normalize_styles();
        }
    }

    fn normalize_styles(&mut self) {
        if !self.styles.is_empty() {
            let spans = std::mem::take(&mut self.styles);
            self.styles = MessageBody::with_styles(&self.text, spans)
                .styles()
                .to_vec();
        }
    }

    fn word_around_cursor(&self) -> Option<std::ops::Range<usize>> {
        let before = self.slice(0, self.cursor);
        let after = self.slice(self.cursor, self.text.len());
        let touches_before = before
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace());
        let touches_after = after.chars().next().is_some_and(|c| !c.is_whitespace());
        if !touches_before && !touches_after {
            return None;
        }
        let start = before
            .char_indices()
            .rev()
            .take_while(|(_, c)| !c.is_whitespace())
            .last()
            .map_or(self.cursor, |(i, _)| i);
        let end = self.cursor
            + after
                .char_indices()
                .take_while(|(_, c)| !c.is_whitespace())
                .last()
                .map_or(0, |(i, c)| i + c.len_utf8());
        (start < end).then_some(start..end)
    }

    fn range_has_style(&self, range: std::ops::Range<usize>, style: TextStyle) -> bool {
        self.styles
            .iter()
            .any(|s| s.style == style && s.start <= range.start && s.start + s.len >= range.end)
    }

    fn remove_style_in_range(&mut self, range: std::ops::Range<usize>, style: TextStyle) {
        let mut next = Vec::with_capacity(self.styles.len());
        for span in self.styles.drain(..) {
            if span.style != style {
                next.push(span);
                continue;
            }
            let s = span.start;
            let e = span.start + span.len;
            if e <= range.start || s >= range.end {
                next.push(span);
                continue;
            }
            if s < range.start {
                next.push(StyleSpan {
                    start: s,
                    len: range.start - s,
                    style,
                });
            }
            if e > range.end {
                next.push(StyleSpan {
                    start: range.end,
                    len: e - range.end,
                    style,
                });
            }
        }
        self.styles = next;
        self.normalize_styles();
    }

    fn previous_boundary(&self, at: usize) -> usize {
        self.slice(0, at)
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self, at: usize) -> usize {
        at + self
            .slice(at, self.text.len())
            .graphemes(true)
            .next()
            .map_or(0, str::len)
    }

    /// Back over non-word characters, then over word characters.
    fn word_start(&self) -> usize {
        let mut graphemes = self.slice(0, self.cursor).grapheme_indices(true).rev();
        let mut at = self.cursor;
        let mut in_word = false;
        for (i, g) in graphemes.by_ref() {
            let word = is_word(g);
            if in_word && !word {
                break;
            }
            in_word |= word;
            at = i;
        }
        at
    }

    /// Forward over non-word characters, then over word characters.
    fn word_end(&self) -> usize {
        let rest = self.slice(self.cursor, self.text.len());
        let mut at = self.cursor;
        let mut in_word = false;
        for g in rest.graphemes(true) {
            let word = is_word(g);
            if in_word && !word {
                break;
            }
            in_word |= word;
            at += g.len();
        }
        at
    }

    fn line_start(&self, at: usize) -> usize {
        self.slice(0, at).rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self, at: usize) -> usize {
        self.slice(at, self.text.len())
            .find('\n')
            .map_or(self.text.len(), |i| at + i)
    }

    /// The position in `start..end` (one line) closest to display `column`.
    fn at_column(&self, start: usize, end: usize, column: usize) -> usize {
        let mut used = 0;
        for (i, g) in self.slice(start, end).grapheme_indices(true) {
            let w = g.width();
            if used + w > column {
                return start + i;
            }
            used += w;
        }
        end
    }
}

fn is_word(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text with `|` marking the cursor.
    fn at(marked: &str) -> Input {
        let cursor = marked.find('|').unwrap();
        Input {
            text: marked.replacen('|', "", 1),
            cursor,
            styles: Vec::new(),
            active_styles: Vec::new(),
        }
    }

    fn shown(input: &Input) -> String {
        let mut text = input.text.clone();
        text.insert(input.cursor, '|');
        text
    }

    #[test]
    fn inserts_and_deletes_whole_graphemes() {
        let mut input = Input::new("ok 👍🏽");
        input.backspace();
        assert_eq!(shown(&input), "ok |");
        input.left();
        input.insert('!');
        assert_eq!(shown(&input), "ok!| ");
        input.home();
        input.delete();
        assert_eq!(shown(&input), "|k! ");
        input.insert_str("a\tb\r\nc\rd\x1b[31m");
        assert_eq!(shown(&input), "a b\nc\nd[31m|k! ");
    }

    #[test]
    fn moves_and_deletes_by_word() {
        let mut input = at("see you there|");
        input.word_left();
        assert_eq!(shown(&input), "see you |there");
        input.word_left();
        input.word_left();
        assert_eq!(shown(&input), "|see you there");
        input.word_right();
        assert_eq!(shown(&input), "see| you there");

        let mut input = at("hello, wörld!|");
        input.delete_word_back();
        assert_eq!(shown(&input), "hello, |");
        input.delete_word_back();
        assert_eq!(shown(&input), "|");

        let mut input = at("|  foo bar");
        input.delete_word_forward();
        assert_eq!(shown(&input), "| bar");
    }

    #[test]
    fn line_editing_works_per_line() {
        let mut input = at("first\nsec|ond\nthird");
        input.home();
        assert_eq!(shown(&input), "first\n|second\nthird");
        input.end();
        assert_eq!(shown(&input), "first\nsecond|\nthird");
        input.delete_to_line_end();
        assert_eq!(shown(&input), "first\nsecond|third");
        input.delete_to_line_start();
        assert_eq!(shown(&input), "first\n|third");
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let mut input = at("a long line\nab\nanother li|ne");
        assert!(input.up());
        assert_eq!(shown(&input), "a long line\nab|\nanother line");
        assert!(input.up());
        assert_eq!(shown(&input), "a |long line\nab\nanother line");
        assert!(!input.up(), "already on the first line");
        assert!(input.down());
        assert!(input.down());
        assert_eq!(shown(&input), "a long line\nab\nan|other line");
        assert!(!input.down());
    }

    #[test]
    fn toggle_style_styles_current_word_or_upcoming_typing() {
        // 1. Toggle style on word touching cursor, then toggle off.
        let mut input = at("hello world|");
        input.toggle_style(TextStyle::Bold);
        assert_eq!(
            input.styles(),
            &[StyleSpan {
                start: 6,
                len: 5,
                style: TextStyle::Bold,
            }]
        );
        input.toggle_style(TextStyle::Bold);
        assert_eq!(input.styles(), []);

        // 2. Toggle style when at whitespace / start of input styles newly typed text.
        let mut typed = Input::default();
        typed.toggle_style(TextStyle::Monospace);
        typed.insert_str("let x = 1;");
        typed.toggle_style(TextStyle::Monospace);
        typed.insert_str(" ok");
        assert_eq!(typed.text(), "let x = 1; ok");
        assert_eq!(
            typed.styles(),
            &[StyleSpan {
                start: 0,
                len: 10,
                style: TextStyle::Monospace,
            }]
        );

        // 3. Inserting and deleting before a styled span shifts its byte offset.
        typed.home();
        typed.insert_str(">> ");
        assert_eq!(
            typed.styles(),
            &[StyleSpan {
                start: 3,
                len: 10,
                style: TextStyle::Monospace,
            }]
        );
        typed.backspace();
        assert_eq!(
            typed.styles(),
            &[StyleSpan {
                start: 2,
                len: 10,
                style: TextStyle::Monospace,
            }]
        );
    }
}
