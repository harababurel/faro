//! Text helpers for the terminal: sanitising untrusted text, measuring it in
//! terminal cells (grapheme-aware, DESIGN.md §11), truncating and wrapping.

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// Make untrusted text (message bodies, names) safe to draw: control characters
/// could otherwise reach the terminal as escape sequences. Tabs become spaces;
/// other controls become `�`. Newlines are removed too: callers that keep line
/// breaks split on `'\n'` first.
pub fn sanitize(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| match c {
                '\t' => ' ',
                c if c.is_control() => '\u{FFFD}',
                c => c,
            })
            .collect(),
    )
}

/// Width in terminal cells.
pub fn width(text: &str) -> usize {
    text.graphemes(true).map(str::width).sum()
}

/// Cut `text` to at most `max` cells, ending with `…` if anything was cut.
pub fn truncate(text: &str, max: usize) -> Cow<'_, str> {
    if width(text) <= max {
        return Cow::Borrowed(text);
    }
    if max == 0 {
        return Cow::Borrowed("");
    }
    let mut out = String::new();
    let mut used = 0;
    for g in text.graphemes(true) {
        let w = g.width();
        if used + w > max - 1 {
            break;
        }
        out.push_str(g);
        used += w;
    }
    out.push('…');
    Cow::Owned(out)
}

/// Pad `text` on the left to `cells` cells (no-op if it's already wider).
pub fn pad_left(text: &str, cells: usize) -> String {
    let w = width(text);
    let mut out = " ".repeat(cells.saturating_sub(w));
    out.push_str(text);
    out
}

/// Word-wrap sanitised text into lines of at most `max` cells. Existing line
/// breaks are kept; words longer than a line are split between graphemes.
/// Always returns at least one line.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        wrap_paragraph(&sanitize(paragraph), max, &mut lines);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_paragraph(text: &str, max: usize, lines: &mut Vec<String>) {
    let mut line = String::new();
    let mut used = 0;
    // Leading spaces are kept on a paragraph's first line (indentation), but a
    // continuation line never starts with the space where the break happened.
    let mut first_line = true;
    let mut push_line = |line: &mut String, used: &mut usize, first_line: &mut bool| {
        lines.push(std::mem::take(line).trim_end().to_owned());
        *used = 0;
        *first_line = false;
    };
    for word in text.split_word_bounds() {
        let w = width(word);
        if word.chars().all(char::is_whitespace) {
            if used + w > max {
                push_line(&mut line, &mut used, &mut first_line);
            } else if used > 0 || first_line {
                line.push_str(word);
                used += w;
            }
            continue;
        }
        if used + w > max && used > 0 {
            push_line(&mut line, &mut used, &mut first_line);
        }
        if used + w <= max {
            line.push_str(word);
            used += w;
            continue;
        }
        // A word wider than a whole line: split it between graphemes.
        for g in word.graphemes(true) {
            let gw = g.width();
            if used + gw > max && used > 0 {
                push_line(&mut line, &mut used, &mut first_line);
            }
            line.push_str(g);
            used += gw;
        }
    }
    lines.push(line.trim_end().to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_neutralises_escape_sequences() {
        assert_eq!(sanitize("plain"), Cow::Borrowed("plain"));
        assert_eq!(sanitize("a\x1b[31mb\tc\u{9b}"), "a\u{FFFD}[31mb c\u{FFFD}");
    }

    #[test]
    fn widths_count_cells_not_chars() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("🧗"), 2);
        assert_eq!(width("👩‍👩‍👧"), 2, "a ZWJ family is one grapheme");
        assert_eq!(width("日本"), 4);
    }

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("Climbing club", 8), "Climbin…");
        assert_eq!(truncate("🧗🧗🧗", 4), "🧗…");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn wraps_on_words_and_keeps_breaks() {
        assert_eq!(
            wrap("the quick brown fox jumps", 10),
            vec!["the quick", "brown fox", "jumps"]
        );
        assert_eq!(wrap("one\ntwo", 10), vec!["one", "two"]);
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("hi 🧗🧗🧗", 4), vec!["hi", "🧗🧗", "🧗"]);
    }

    #[test]
    fn wrapped_lines_never_exceed_the_width() {
        let text = "Lorem ipsum 🌊🌊 dolor sit amet, consectetur adipiscing elit 日本語のテキスト";
        for max in 1..30 {
            for line in wrap(text, max) {
                assert!(width(&line) <= max.max(2), "{line:?} wider than {max}");
            }
        }
    }
}
