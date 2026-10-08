//! Text helpers for the terminal: sanitising untrusted text, measuring it in
//! terminal cells (grapheme-aware, DESIGN.md §11), truncating and wrapping.

use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// What Signal puts in a body where someone is mentioned.
const MENTION_PLACEHOLDER: char = '\u{FFFC}';

/// Make untrusted text (message bodies, names) safe to draw: control characters
/// could otherwise reach the terminal as escape sequences. Tabs become spaces;
/// other controls become `�`. Newlines are removed too: callers that keep line
/// breaks split on `'\n'` first. A mention's placeholder (U+FFFC) left
/// without its mention becomes `@` (D72). Emoji are drawn as the emoji mode
/// says (`emoji::display`, D47), so measure text after sanitising it.
pub fn sanitize(text: &str) -> Cow<'_, str> {
    let replaced = |c: char| c.is_control() || c == MENTION_PLACEHOLDER;
    if !text.chars().any(replaced) {
        return super::emoji::display(text);
    }
    let clean: String = text
        .chars()
        .map(|c| match c {
            '\t' => ' ',
            MENTION_PLACEHOLDER => '@',
            c if c.is_control() => '\u{FFFD}',
            c => c,
        })
        .collect();
    Cow::Owned(super::emoji::display(&clean).into_owned())
}

/// [`sanitize`]d text on one line: line breaks become spaces (previews of
/// multi-line messages, e.g. quotes).
pub fn one_line(text: &str) -> String {
    let joined: String = text
        .split(['\r', '\n'])
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    sanitize(&joined).into_owned()
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

/// Word-wrap text made of pieces that each carry a value (a style: mentions
/// are drawn apart, D72) into lines of at most `max` cells, sanitised.
/// Existing line breaks are kept; words longer than a line are split between
/// graphemes. Each line comes back as `(text, value)` pieces, so a piece
/// keeps its value even where it's wrapped. Always at least one line.
pub fn wrap_pieces<T: Copy>(pieces: &[(&str, T)], max: usize) -> Vec<Vec<(String, T)>> {
    let max = max.max(1);
    let mut lines = Vec::new();
    // One paragraph at a time: the sanitised text and its pieces' byte ranges.
    let mut paragraph = String::new();
    let mut ranges: Vec<(Range<usize>, T)> = Vec::new();
    let mut finish = |paragraph: &mut String, ranges: &mut Vec<(Range<usize>, T)>| {
        for line in wrap_ranges(paragraph, max) {
            lines.push(cut_pieces(paragraph, line, ranges));
        }
        paragraph.clear();
        ranges.clear();
    };
    for &(text, value) in pieces {
        let mut rest = Some(text);
        while let Some(piece) = rest {
            let (part, next) = piece
                .split_once('\n')
                .map_or((piece, None), |(part, next)| (part, Some(next)));
            let start = paragraph.len();
            paragraph.push_str(&sanitize(part));
            if paragraph.len() > start {
                ranges.push((start..paragraph.len(), value));
            }
            if next.is_some() {
                finish(&mut paragraph, &mut ranges);
            }
            rest = next;
        }
    }
    finish(&mut paragraph, &mut ranges);
    lines
}

/// `paragraph[line]` cut where pieces start and end.
fn cut_pieces<T: Copy>(
    paragraph: &str,
    line: Range<usize>,
    ranges: &[(Range<usize>, T)],
) -> Vec<(String, T)> {
    ranges
        .iter()
        .filter_map(|(range, value)| {
            let (start, end) = (range.start.max(line.start), range.end.min(line.end));
            let text = paragraph.get(start..end).filter(|t| !t.is_empty())?;
            Some((text.to_owned(), *value))
        })
        .collect()
}

/// The byte ranges of `text` (one paragraph, sanitised) that make up its
/// wrapped lines, trailing spaces left out. Always at least one range.
fn wrap_ranges(text: &str, max: usize) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    // The current line: where it starts (once something is on it) and ends.
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut used = 0;
    // Leading spaces are kept on a paragraph's first line (indentation), but a
    // continuation line never starts with the space where the break happened.
    let mut first_line = true;
    let mut push_line = |start: &mut Option<usize>, end: usize, used: &mut usize| {
        let from = start.take().unwrap_or(end);
        let trimmed = text.get(from..end).map_or(0, |line| line.trim_end().len());
        lines.push(from..from + trimmed);
        *used = 0;
    };
    for (at, word) in text.split_word_bound_indices() {
        let w = width(word);
        if word.chars().all(char::is_whitespace) {
            if used + w > max {
                push_line(&mut start, end, &mut used);
                first_line = false;
            } else if used > 0 || first_line {
                start.get_or_insert(at);
                end = at + word.len();
                used += w;
            }
            continue;
        }
        if used + w > max && used > 0 {
            push_line(&mut start, end, &mut used);
            first_line = false;
        }
        if used + w <= max {
            start.get_or_insert(at);
            end = at + word.len();
            used += w;
            continue;
        }
        // A word wider than a whole line: split it between graphemes.
        for (offset, g) in word.grapheme_indices(true) {
            let gw = g.width();
            if used + gw > max && used > 0 {
                push_line(&mut start, end, &mut used);
                first_line = false;
            }
            start.get_or_insert(at + offset);
            end = at + offset + g.len();
            used += gw;
        }
    }
    push_line(&mut start, end, &mut used);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plain text wrapped paragraph by paragraph, without pieces: what
    /// [`wrap_pieces`] must agree with.
    fn wrap(text: &str, max: usize) -> Vec<String> {
        let max = max.max(1);
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let paragraph = sanitize(paragraph);
            for range in wrap_ranges(&paragraph, max) {
                lines.push(paragraph.get(range).unwrap_or_default().to_owned());
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    #[test]
    fn sanitize_neutralises_escape_sequences() {
        assert_eq!(sanitize("plain"), Cow::Borrowed("plain"));
        assert_eq!(sanitize("a\x1b[31mb\tc\u{9b}"), "a\u{FFFD}[31mb c\u{FFFD}");
        assert_eq!(
            sanitize("hi \u{FFFC}"),
            "hi @",
            "a mention without its name"
        );
    }

    #[test]
    fn one_line_joins_lines_before_sanitising() {
        assert_eq!(
            one_line("line one\nline two\r\nthree\x1b"),
            "line one line two three\u{FFFD}"
        );
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
    fn pieces_keep_their_values_across_lines() {
        let line = |pieces: &[(&str, bool)]| -> Vec<(String, bool)> {
            pieces.iter().map(|&(t, m)| (t.to_owned(), m)).collect()
        };
        assert_eq!(
            wrap_pieces(&[("hi ", false), ("@Alice Smith", true), ("!", false)], 9),
            vec![
                line(&[("hi ", false), ("@Alice", true)]),
                line(&[("Smith", true), ("!", false)]),
            ]
        );
        assert_eq!(
            wrap_pieces(&[("one\ntwo ", false), ("@Bo", true)], 20),
            vec![
                line(&[("one", false)]),
                line(&[("two ", false), ("@Bo", true)])
            ]
        );
        assert_eq!(wrap_pieces::<bool>(&[], 5), vec![line(&[])]);
        // Sanitised piece by piece: escapes can't sneak in through a name.
        assert_eq!(
            wrap_pieces(&[("@\x1b[2J", true)], 20),
            vec![line(&[("@\u{FFFD}[2J", true)])]
        );
    }

    #[test]
    fn wrapping_pieces_matches_plain_wrapping() {
        let text =
            "Lorem ipsum 🌊🌊 dolor  sit amet,\n  consectetur adipiscing elit 日本語のテキスト";
        let (a, b) = text.split_at(text.find("dolor").unwrap_or(0));
        for max in 1..30 {
            let marked: Vec<String> = wrap_pieces(&[(a, false), (b, true)], max)
                .into_iter()
                .map(|line| line.into_iter().map(|(t, _)| t).collect())
                .collect();
            assert_eq!(marked, wrap(text, max), "at {max} cells");
        }
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
