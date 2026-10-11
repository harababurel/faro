//! faro-app also builds for the browser (M13), so it must not depend on the
//! terminal, the file system, processes, signals or SQLite. These checks read
//! the manifest; the wasm32 build in CI catches the rest.

const MANIFEST: &str = include_str!("../Cargo.toml");

/// Crates that only make sense in the terminal binary.
const NATIVE_ONLY: &[&str] = &[
    "clap",
    "crossterm",
    "faro-presage",
    "mio",
    "notify",
    "ratatui-image",
    "rustix",
    "sqlx",
    "tokio",
];

/// `(section, line)` for every dependency line outside `[dev-dependencies]`.
fn dependency_lines() -> Vec<(&'static str, &'static str)> {
    let mut section = "";
    let mut lines = Vec::new();
    for line in MANIFEST.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
            continue;
        }
        let is_dependencies = section.ends_with("dependencies]") && section != "[dev-dependencies]";
        if is_dependencies && !line.is_empty() && !line.starts_with('#') {
            lines.push((section, line));
        }
    }
    lines
}

fn crate_name(line: &str) -> &str {
    line.split(['=', '.', ' ']).next().unwrap_or("")
}

#[test]
fn no_native_only_dependencies() {
    for (section, line) in dependency_lines() {
        assert!(
            !NATIVE_ONLY.contains(&crate_name(line)),
            "{section}: `{line}` is native-only; it belongs in crates/faro"
        );
    }
}

#[test]
fn no_sqlite_and_no_crossterm_backend() {
    for (section, line) in dependency_lines() {
        match crate_name(line) {
            "faro-core" => assert!(
                !line.contains("sqlite"),
                "{section}: faro-core's SQLite engine is native-only (the web uses the memory store)"
            ),
            "ratatui" => assert!(
                !line.contains("crossterm") && line.contains("default-features = false"),
                "{section}: ratatui must not pull in its crossterm backend"
            ),
            _ => {}
        }
    }
}

#[test]
fn the_checks_see_the_dependencies() {
    let names: Vec<&str> = dependency_lines()
        .into_iter()
        .map(|(_, line)| crate_name(line))
        .collect();
    for expected in ["faro-core", "ratatui"] {
        assert!(
            names.contains(&expected),
            "{expected} not found in {names:?}"
        );
    }
}
