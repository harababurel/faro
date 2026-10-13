//! Filling the `{{name}}` placeholders (WEBSITE.md §6.1).

use std::collections::BTreeMap;
use std::fmt;

/// The values the generator knows, by placeholder name.
pub type Values = BTreeMap<&'static str, String>;

/// Which comment syntax a file uses: HTML and XML only have `<!-- -->`,
/// CSS only has `/* */`. XML also escapes the values it fills: a sitemap
/// is text, a value must not bring markup in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Html,
    Css,
    Xml,
}

/// What's wrong with the placeholders in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A name the generator doesn't know — a template typo that must
    /// not ship unfilled.
    Unknown(String),
    /// A `{{` with no `}}` closer: carries a snippet of the text around it.
    UnclosedPlaceholder(String),
    /// A comment opener with no closer: carries a snippet of the text
    /// after it.
    UnterminatedComment(String),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "unknown or unfilled placeholder {{{{{name}}}}}"),
            Self::UnclosedPlaceholder(around) => {
                write!(f, "unclosed placeholder near \"{around}\"")
            }
            Self::UnterminatedComment(around) => {
                write!(f, "unterminated comment near \"{around}\"")
            }
        }
    }
}

/// Replace every `{{name}}` outside comments with its value. Inside a
/// comment the braces are dropped instead: the template documents its own
/// placeholders in comments, and pasting a whole poster SVG into one
/// would only bloat the page. Comments follow `syntax`: HTML knows only
/// `<!-- -->`, CSS only `/* */`.
///
/// Unknown names, an unclosed `{{`, and an unterminated comment are all
/// problems, so a template typo can't ship.
pub fn fill(text: &str, values: &Values, syntax: Syntax) -> Result<String, Vec<Problem>> {
    let (open, close) = match syntax {
        Syntax::Html | Syntax::Xml => ("<!--", "-->"),
        Syntax::Css => ("/*", "*/"),
    };
    let mut out = String::with_capacity(text.len());
    let mut problems: Vec<Problem> = Vec::new();
    let mut rest = text;
    loop {
        let Some(pos) = rest.find(open) else {
            fill_segment(rest, false, values, syntax, &mut out, &mut problems);
            break;
        };
        let (before, opened) = rest.split_at(pos);
        fill_segment(before, false, values, syntax, &mut out, &mut problems);
        out.push_str(open);
        let (_, inside_or_after) = opened.split_at(open.len());
        if let Some(end) = inside_or_after.find(close) {
            let (inside, closed) = inside_or_after.split_at(end);
            fill_segment(inside, true, values, syntax, &mut out, &mut problems);
            out.push_str(close);
            let (_, after) = closed.split_at(close.len());
            rest = after;
        } else {
            problems.push(Problem::UnterminatedComment(snippet(inside_or_after, 0)));
            break;
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems)
    }
}

/// Replace the placeholders in one stretch of text, comment or not.
fn fill_segment(
    segment: &str,
    in_comment: bool,
    values: &Values,
    syntax: Syntax,
    out: &mut String,
    problems: &mut Vec<Problem>,
) {
    let mut rest = segment;
    let mut seen = 0usize;
    while let Some(pos) = rest.find("{{") {
        let (before, opened) = rest.split_at(pos);
        out.push_str(before);
        seen += pos + 2;
        let (_, after) = opened.split_at(2);
        if let Some(end) = after.find("}}") {
            let (name, closed) = after.split_at(end);
            let (_, after) = closed.split_at(2);
            rest = after;
            seen += end + 2;
            match values.get(name) {
                // In a comment just drop the braces: the comment
                // documents the placeholder, it doesn't want the value.
                Some(_) if in_comment => out.push_str(name),
                // XML is text: a filled value can't bring markup in.
                Some(value) if syntax == Syntax::Xml => out.push_str(&escape(value)),
                Some(value) => out.push_str(value),
                None => {
                    problems.push(Problem::Unknown(name.to_owned()));
                    out.push_str("{{");
                    out.push_str(name);
                    out.push_str("}}");
                }
            }
        } else {
            // `{{` with no `}}`: name the text around it.
            problems.push(Problem::UnclosedPlaceholder(snippet(segment, seen - 2)));
            break;
        }
    }
    out.push_str(rest);
}

/// A short snippet of `text` around byte offset `pos`, on char boundaries.
fn snippet(text: &str, pos: usize) -> String {
    let mut start = pos.saturating_sub(24);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let (_, rest) = text.split_at(start);
    let mut end = 48.min(rest.len());
    while !rest.is_char_boundary(end) {
        end += 1;
    }
    let (near, _) = rest.split_at(end);
    let lead = if start > 0 { "…" } else { "" };
    let trail = if start + end < text.len() { "…" } else { "" };
    format!("{lead}{}{trail}", near.trim())
}

/// HTML-escape text for the generated markup.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Values {
        [
            ("name", "faro".to_owned()),
            ("big", "<b>big</b>".to_owned()),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn fills_known_names() {
        let filled = fill("v{{name}}!", &values(), Syntax::Html).unwrap();
        assert_eq!(filled, "vfaro!");
    }

    #[test]
    fn unknown_names_are_an_error() {
        let problems = fill("{{name}} {{bogus}}", &values(), Syntax::Html).err();
        assert_eq!(problems, Some(vec![Problem::Unknown("bogus".to_owned())]));
    }

    #[test]
    fn an_unclosed_placeholder_names_the_text_around_it() {
        let problems = fill("a {{name here", &values(), Syntax::Html).err();
        let Some(Problem::UnclosedPlaceholder(around)) =
            problems.and_then(|p| p.into_iter().next())
        else {
            panic!("expected an unclosed placeholder");
        };
        assert!(around.contains("{{name here"));
    }

    #[test]
    fn comments_keep_the_name_not_the_value() {
        let filled = fill(
            "<!-- {{name}} docs --><p>{{name}}</p>",
            &values(),
            Syntax::Html,
        )
        .unwrap();
        assert_eq!(filled, "<!-- name docs --><p>faro</p>");
    }

    #[test]
    fn css_comments_count_in_css() {
        let filled = fill(
            "/* {{name}} is inline */ .a { color: red; }",
            &values(),
            Syntax::Css,
        )
        .unwrap();
        assert_eq!(filled, "/* name is inline */ .a { color: red; }");
    }

    #[test]
    fn html_comments_are_plain_text_in_css() {
        let filled = fill("<!-- {{name}} -->", &values(), Syntax::Css).unwrap();
        assert_eq!(filled, "<!-- faro -->");
    }

    #[test]
    fn css_comments_are_plain_text_in_html() {
        let filled = fill("/* {{name}} */", &values(), Syntax::Html).unwrap();
        assert_eq!(filled, "/* faro */");
    }

    #[test]
    fn an_unclosed_comment_is_an_error() {
        let problems = fill("<!-- {{name}}", &values(), Syntax::Html).err();
        let Some(Problem::UnterminatedComment(around)) =
            problems.and_then(|p| p.into_iter().next())
        else {
            panic!("expected an unterminated comment");
        };
        assert!(around.contains("{{name}}"));
    }

    #[test]
    fn values_are_inserted_as_they_come() {
        let filled = fill("{{big}}", &values(), Syntax::Html).unwrap();
        assert_eq!(filled, "<b>big</b>");
    }

    #[test]
    fn xml_comments_work_like_html_ones() {
        let filled = fill(
            "<!-- {{name}} docs --><url>{{name}}</url>",
            &values(),
            Syntax::Xml,
        )
        .unwrap();
        assert_eq!(filled, "<!-- name docs --><url>faro</url>");
    }

    #[test]
    fn xml_values_are_escaped() {
        let filled = fill("<loc>{{big}}</loc>", &values(), Syntax::Xml).unwrap();
        assert_eq!(filled, "<loc>&lt;b&gt;big&lt;/b&gt;</loc>");
    }

    #[test]
    fn escapes_text() {
        assert_eq!(
            escape("<a href=\"x\">&'"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }
}
