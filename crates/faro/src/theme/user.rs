//! The user's own theme files (D54): `themes/*.json` beside the config file,
//! in OpenCode's format, read at startup and again whenever one changes.

use super::{Source, Theme, import};

/// A theme file as read from disk: its file name and its text, or why it
/// couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeText {
    pub file: String,
    pub text: Result<String, String>,
}

/// The user's themes, and a count of how often they were read (the app
/// loop's cache loads the theme again when it changes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserThemes {
    pub themes: Vec<Theme>,
    pub generation: u64,
}

impl UserThemes {
    /// Replace the themes with those in `files`; returns what's wrong with
    /// the files that couldn't be used (those are skipped).
    pub fn load(&mut self, files: Vec<ThemeText>) -> Vec<String> {
        let (themes, problems) = build(files);
        self.themes = themes;
        self.generation += 1;
        problems
    }
}

/// The themes in `files`, sorted by name. A file's name (without `.json`)
/// names its dark variant; the light one is `<name>-light` unless a file of
/// that name exists. `system` can't be replaced: it's the fallback for
/// terminals with 16 colours.
fn build(mut files: Vec<ThemeText>) -> (Vec<Theme>, Vec<String>) {
    files.sort_by(|a, b| a.file.cmp(&b.file));
    let stems: Vec<&str> = files.iter().filter_map(|f| stem(&f.file)).collect();
    let mut themes: Vec<Theme> = Vec::new();
    let mut problems = Vec::new();
    for file in &files {
        let Some(name) = stem(&file.file) else {
            continue;
        };
        if name == "system" {
            problems.push(format!(
                "{}: the system theme can't be replaced; rename the file",
                file.file
            ));
            continue;
        }
        let parsed = file.text.clone().and_then(|text| {
            faro_core::theme_file::parse(&text)
                .and_then(|theme| import::variants(name, &theme, Source::User))
                .map_err(|e| e.to_string())
        });
        match parsed {
            Ok(variants) => themes.extend(
                variants
                    .into_iter()
                    .filter(|t| t.name == name || !stems.contains(&t.name.as_str())),
            ),
            Err(problem) => problems.push(format!("{}: {problem}", file.file)),
        }
    }
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    (themes, problems)
}

fn stem(file: &str) -> Option<&str> {
    file.strip_suffix(".json").filter(|s| !s.is_empty())
}
