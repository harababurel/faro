//! Paths typed into the palette or pasted into the input (D77). Pure: the
//! file system is read by the terminal binary (`system::list_files`,
//! `system::check_paste`).

use std::path::PathBuf;

/// Split a palette file query into `(dir_prefix_with_trailing_slash, file_prefix)`.
/// For example, `"~/Downloads/rep"` splits into `("~/Downloads/", "rep")`, and
/// `"~/"` splits into `("~/", "")`.
pub fn split_file_query(query: &str) -> (&str, &str) {
    match query.rfind('/') {
        Some(idx) => (&query[..=idx], &query[idx + 1..]),
        None => ("", query),
    }
}

/// Expand a leading `~` or `~/` using `HOME`.
pub fn expand_tilde(raw: &str) -> PathBuf {
    if raw == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    if let Some(rest) = raw.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(raw)
}

/// If `raw` is a single-line path-like paste (after stripping surrounding quotes,
/// `file://`, and backslash escapes), return the expanded candidate `PathBuf` (D77).
pub fn candidate_paste_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
        return None;
    }
    let unquoted = trimmed
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
        .unwrap_or(trimmed);
    let stripped = if let Some(rest) = unquoted.strip_prefix("file://localhost/") {
        format!("/{rest}")
    } else if let Some(rest) = unquoted.strip_prefix("file:///") {
        format!("/{rest}")
    } else if unquoted == "~"
        || unquoted.starts_with('/')
        || unquoted.starts_with("~/")
        || unquoted.starts_with("./")
        || unquoted.starts_with("../")
    {
        unquoted.to_owned()
    } else {
        return None;
    };
    let mut unescaped = String::with_capacity(stripped.len());
    let mut chars = stripped.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && let Some(next) = chars.next()
        {
            unescaped.push(next);
        } else {
            unescaped.push(ch);
        }
    }
    Some(expand_tilde(&unescaped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_file_query_into_directory_and_prefix() {
        assert_eq!(split_file_query("~/"), ("~/", ""));
        assert_eq!(split_file_query("~/Downloads/rep"), ("~/Downloads/", "rep"));
        assert_eq!(split_file_query("/tmp/.secret"), ("/tmp/", ".secret"));
        assert_eq!(split_file_query("report.pdf"), ("", "report.pdf"));
    }

    #[test]
    fn parses_candidate_paste_paths() {
        assert_eq!(
            candidate_paste_path("  '/tmp/my\\ file.png' \n"),
            Some(PathBuf::from("/tmp/my file.png"))
        );
        assert_eq!(
            candidate_paste_path("file:///tmp/photo.jpg"),
            Some(PathBuf::from("/tmp/photo.jpg"))
        );
        assert_eq!(
            candidate_paste_path("./notes.txt"),
            Some(PathBuf::from("./notes.txt"))
        );
        assert_eq!(candidate_paste_path("hello world"), None);
        assert_eq!(candidate_paste_path("/tmp/a\n/tmp/b"), None);
    }
}
