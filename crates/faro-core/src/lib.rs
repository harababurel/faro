//! Core of faro: domain types, the `Backend` trait, the mock backend, the store,
//! config and themes. Nothing in here touches the terminal.

pub mod audio;
pub mod backend;
pub mod clipboard;
pub mod config;
pub mod domain;
pub mod guard;
pub mod mock;
#[cfg(not(target_arch = "wasm32"))]
pub mod paths;
pub mod rt;
pub mod store;
pub mod theme_file;

#[cfg(all(target_arch = "wasm32", feature = "sqlite"))]
compile_error!("the SQLite store doesn't build for wasm32: use `--no-default-features`");

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
