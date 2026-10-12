//! Content-hashed asset names and reference rewriting (WEBSITE.md §6.1).

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

/// How many hex digits of the sha256 go into an asset's new name; nginx
/// caches anything with at least 8 (`site/deploy/nginx/getfaro.dev.conf`).
const HASH_CHARS: usize = 10;

/// One hashed asset as it can appear in a file: `(old reference, new reference)`.
pub type Refs = Vec<(String, String)>;

/// The first [`HASH_CHARS`] hex digits of the content's sha256.
pub fn hash_prefix(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(HASH_CHARS);
    for byte in digest.iter().take(HASH_CHARS / 2) {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => char::from(b'0' + nibble),
        _ => char::from(b'a' + nibble - 10),
    }
}

/// `fonts/x.woff2` → `fonts/x.<hash>.woff2`.
pub fn hashed_name(rel: &str, prefix: &str) -> String {
    let dot = rel.rfind('.').filter(|dot| {
        rel.rfind('/')
            .is_none_or(|slash| *dot > slash && *dot + 1 < rel.len())
    });
    match dot {
        Some(dot) => {
            let (stem, ext) = rel.split_at(dot);
            format!("{stem}.{prefix}{ext}")
        }
        None => format!("{rel}.{prefix}"),
    }
}

/// How a file in directory `dir` (site-relative, `""` for the root) can
/// refer to each hashed asset: relative to `dir`, `./`-relative to `dir`,
/// and from the site root. Longest first, so a longer spelling is
/// rewritten before the shorter one inside it.
pub fn refs_for(dir: &str, hashed: &BTreeMap<String, String>) -> Refs {
    let up = if dir.is_empty() {
        String::new()
    } else {
        "../".repeat(dir.split('/').count())
    };
    let mut refs = Refs::new();
    for (old, new) in hashed {
        refs.push((format!("{up}{old}"), format!("{up}{new}")));
        refs.push((format!("./{up}{old}"), format!("./{up}{new}")));
        refs.push((format!("/{old}"), format!("/{new}")));
    }
    refs.sort_by(|a, b| b.0.cmp(&a.0));
    refs
}

/// Rewrite every reference in `text` that names a hashed asset.
pub fn rewrite(text: &str, refs: &Refs) -> String {
    let mut out = text.to_owned();
    for (old, new) in refs {
        out = replace_whole(&out, old, new);
    }
    out
}

/// Replace `old` in `text`, but only where it stands alone: neither
/// neighbour may be a path character, so `style.css` doesn't match inside
/// `notstyle.css` or `other/style.css`.
fn replace_whole(text: &str, old: &str, new: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(old) {
        let (before, matched) = rest.split_at(pos);
        let (matched, after) = matched.split_at(old.len());
        out.push_str(before);
        let before_ok = before.chars().next_back().is_none_or(|c| !is_path_char(c));
        let after_ok = after.chars().next().is_none_or(|c| !is_path_char(c));
        if before_ok && after_ok {
            out.push_str(new);
        } else {
            out.push_str(matched);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_short_and_deterministic() {
        assert_eq!(hash_prefix(b"abc"), "ba7816bf8f");
        assert_eq!(hash_prefix(b"abc"), hash_prefix(b"abc"));
        assert_ne!(hash_prefix(b"abc"), hash_prefix(b"abd"));
    }

    #[test]
    fn the_hash_goes_before_the_extension() {
        assert_eq!(hashed_name("fonts/x.woff2", "ab12"), "fonts/x.ab12.woff2");
        assert_eq!(hashed_name("og.png", "ab12"), "og.ab12.png");
        assert_eq!(hashed_name("no-ext", "ab12"), "no-ext.ab12");
        // A dot in a directory name isn't an extension.
        assert_eq!(hashed_name("v1.2/x", "ab12"), "v1.2/x.ab12");
    }

    #[test]
    fn rewrites_references_in_html_and_css() {
        let hashed = BTreeMap::from([
            ("style.css".to_owned(), "style.ab12.css".to_owned()),
            ("fonts/x.woff2".to_owned(), "fonts/x.cd34.woff2".to_owned()),
        ]);
        let refs = refs_for("", &hashed);
        let html = "<link href=\"style.css\"><script src=\"/js/init.js\">";
        assert_eq!(
            rewrite(html, &refs),
            "<link href=\"style.ab12.css\"><script src=\"/js/init.js\">"
        );
        let css = "src: url(\"fonts/x.woff2\") format(\"woff2\")";
        assert_eq!(
            rewrite(css, &refs),
            "src: url(\"fonts/x.cd34.woff2\") format(\"woff2\")"
        );
    }

    #[test]
    fn absolute_references_get_rewritten_too() {
        let hashed = BTreeMap::from([("style.css".to_owned(), "style.ab12.css".to_owned())]);
        let refs = refs_for("", &hashed);
        assert_eq!(
            rewrite("<link href=\"/style.css\">", &refs),
            "<link href=\"/style.ab12.css\">"
        );
    }

    #[test]
    fn only_whole_paths_match() {
        let hashed = BTreeMap::from([("style.css".to_owned(), "style.ab12.css".to_owned())]);
        let refs = refs_for("", &hashed);
        assert_eq!(
            rewrite("notstyle.css and other/style.css", &refs),
            "notstyle.css and other/style.css"
        );
        assert_eq!(rewrite("xstyle.css", &refs), "xstyle.css");
    }

    #[test]
    fn dot_slash_references_get_rewritten() {
        let hashed = BTreeMap::from([("style.css".to_owned(), "style.ab12.css".to_owned())]);
        let refs = refs_for("", &hashed);
        assert_eq!(
            rewrite("<link href=\"./style.css\">", &refs),
            "<link href=\"./style.ab12.css\">"
        );
    }

    #[test]
    fn query_and_fragment_suffixes_survive() {
        let hashed = BTreeMap::from([("style.css".to_owned(), "style.ab12.css".to_owned())]);
        let refs = refs_for("", &hashed);
        assert_eq!(
            rewrite(
                "<link href=\"style.css?v=1\"><a href=\"style.css#a\">",
                &refs
            ),
            "<link href=\"style.ab12.css?v=1\"><a href=\"style.ab12.css#a\">"
        );
    }

    #[test]
    fn a_nested_file_refers_up_first() {
        let hashed = BTreeMap::from([("style.css".to_owned(), "style.ab12.css".to_owned())]);
        let refs = refs_for("deep/deeper", &hashed);
        assert!(refs.contains(&(
            "../../style.css".to_owned(),
            "../../style.ab12.css".to_owned()
        )));
    }
}
