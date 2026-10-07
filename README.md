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

Not much to build yet; see the [plan](docs/PLAN.md). You'll need Rust (stable, via rustup). Later milestones (the Signal backend) also need `protoc`; see [AGENTS.md](AGENTS.md#development-environment) for per-OS packages.

## License

[AGPL-3.0](LICENSE)
