//! Core of faro: domain types, the `Backend` trait, the mock backend, the store,
//! config and themes. Nothing in here touches the terminal.

pub mod audio;
pub mod backend;
pub mod clipboard;
pub mod config;
pub mod domain;
pub mod guard;
pub mod mock;
#[cfg(feature = "native")]
pub mod paths;
#[cfg(feature = "native")]
pub mod store;
pub mod theme_file;

// M13 Phase 0 spike: the browser build needs the spike's timers and clock.
#[cfg(all(target_arch = "wasm32", not(feature = "wasm-spike")))]
compile_error!("faro-core on wasm32 needs `--no-default-features --features wasm-spike`");

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
