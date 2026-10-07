# faro

A terminal client for [Signal](https://signal.org), built for people who live in tmux.

> **Status:** early work in progress. Nothing to install yet.

faro is an unofficial, independent project. It is not affiliated with, endorsed by, or supported by Signal Messenger LLC or the Signal Technology Foundation.

## Goals

- A polished TUI: command palette (`Ctrl+P`) with fuzzy search, sidebars, mouse support, themes
- Never blocks: incoming messages and network activity don't freeze typing
- Works well over SSH and inside tmux, in modern terminals like kitty
- Easy to configure, from the UI or from a commented config file with hot reload
- Single binary, packaged for macOS and Linux

## Documentation

- [Design](docs/DESIGN.md): architecture, UI, configuration, themes
- [Implementation plan](docs/PLAN.md): milestones and progress
- [Decision log](docs/DECISIONS.md): what was decided and why
- [Background](docs/BACKGROUND.md): motivation, research, naming
- [AGENTS.md](AGENTS.md): guide for coding agents working on this repo

## Development

Early days: `faro` builds and runs, but has no UI yet. Today it runs a mock Signal backend (fake contacts and messages, no account needed) until you press Ctrl+C, logs what happens, and prints a summary. See the [plan](docs/PLAN.md) for progress.

**Prerequisites**

- Rust via [rustup](https://rustup.rs). The exact version is pinned in `rust-toolchain.toml`; rustup installs it automatically on first build.
- Later milestones (the Signal backend) also need `protoc`; see [AGENTS.md](AGENTS.md#development-environment) for per-OS packages.

**Commands**

```sh
cargo run -- --help            # run faro
cargo run -- --data-dir /tmp/faro-dev   # use a throwaway data dir
FARO_LOG=info,faro=debug cargo run -- --mock-seed 7   # log every mock event; another seed, another conversation
cargo test                     # tests
cargo fmt --check              # formatting
cargo clippy --all-targets -- -D warnings   # lints
```

The last three must pass before every commit; CI runs them on Linux and macOS.

**Files**

| What | Where | Override |
|---|---|---|
| Config | `~/.config/faro/config.toml` (or `$XDG_CONFIG_HOME/faro/`) | coming in M5 |
| Data (DB, Signal keys) | `~/.local/share/faro/` (or `$XDG_DATA_HOME/faro/`) | `--data-dir`, `FARO_DATA_DIR` |
| Log | `<data dir>/logs/faro.log` | level via `FARO_LOG`, e.g. `FARO_LOG=debug` |

The same XDG paths are used on macOS. faro never logs message bodies or phone numbers.

## License

[AGPL-3.0](LICENSE)
