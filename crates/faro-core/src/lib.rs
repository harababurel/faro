//! Core of faro: domain types, the `Backend` trait, the mock backend, the store,
//! config and themes. Nothing in here touches the terminal.

pub mod backend;
pub mod config;
pub mod domain;
pub mod mock;
pub mod paths;
pub mod store;

#[cfg(test)]
mod architecture {
    /// D20: the core never touches the terminal, so it can back other front ends
    /// (and a future daemon). Checked on the manifest, at compile time.
    #[test]
    fn core_does_not_depend_on_terminal_crates() {
        let manifest = include_str!("../Cargo.toml");
        for banned in ["ratatui", "crossterm"] {
            assert!(!manifest.contains(banned), "faro-core depends on {banned}");
        }
    }
}
