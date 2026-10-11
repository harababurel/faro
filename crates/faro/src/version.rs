//! The version line (D97): `0.1.0-dev (8fe5755 2026-10-11)`, made by
//! `build.rs`; just the version where the build had no git.

/// The version, then the commit and its date when known. `faro --version`
/// prints it after the name.
pub const LINE: &str = env!("FARO_VERSION");

/// What the UI draws: [`LINE`], fixed in tests so snapshots don't change
/// with every commit.
pub const fn shown() -> &'static str {
    if cfg!(test) {
        "1.2.3 (abc1234 2026-01-01)"
    } else {
        LINE
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_line_starts_with_the_crate_version() {
        assert!(super::LINE.starts_with(env!("CARGO_PKG_VERSION")));
    }
}
