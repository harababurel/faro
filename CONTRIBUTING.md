# Contributing to faro

Bug reports, ideas and pull requests are welcome. For anything bigger than a small fix, open an issue first so we can agree on the approach.

## Reporting a bug

[Open an issue](https://github.com/harababurel/faro/issues) with:

- `faro --version`
- your OS and terminal, and whether faro runs in tmux, over SSH or through mosh
- what you did, what you expected and what happened
- the relevant lines from `~/.local/share/faro/logs/faro.log` (run with `FARO_LOG=debug` for more). The log never contains message text or phone numbers, but check it before posting.

Rendering bugs are easiest to fix with a screenshot. Most can be reproduced in `faro demo`, which needs no account.

## Building

Install Rust with [rustup](https://rustup.rs) (the toolchain, including the wasm32 target for the web demo, is pinned in `rust-toolchain.toml`) and the build dependencies:

| OS | Packages |
|---|---|
| Arch | `sudo pacman -S protobuf cmake clang pkgconf openssl` |
| Debian/Ubuntu | `sudo apt install protobuf-compiler cmake clang pkg-config libssl-dev` |
| macOS | `brew install protobuf cmake` |

```sh
cargo run -- demo                                       # the demo, from a checkout
FARO_LOG=info,faro=debug cargo run -- demo --seed 7     # log every demo event
cargo run -- demo --activity 5 --no-faults              # flood the UI with messages
```

`faro demo --help` lists the demo's flags. To test with a real account without messaging everyone, use a separate test account and `--allowlist FILE`: faro then talks only to the people and groups in FILE (`faro ids --username NAME` prints the IDs it needs).

## Checks

Every commit must pass these; CI runs them on Linux and macOS, plus the wasm32 build and the website checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --target wasm32-unknown-unknown -p faro-app -p faro-web -- -D warnings
cargo test
```

Clippy runs with pedantic lints and project rules: no panics (`unwrap`, `expect`, indexing), no blocking calls on the UI thread, no `print!`. An exception needs `#[expect(lint, reason = "…")]` with a real reason.

Changes to how faro draws on screen (layout, colours, emoji, images, mouse, keys) can't be fully checked by tests. Say how you checked them, and add a step to [docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md) if it's worth repeating.

## Ground rules

- faro never implements the Signal protocol itself; it uses [presage](https://github.com/whisperfish/presage) and libsignal.
- Nothing slow runs on the UI thread: network and disk work happen elsewhere and report back.
- Never log message bodies or phone numbers, and never commit Signal data, databases, keys or logs.
- Decisions that shape the design are recorded in [docs/DECISIONS.md](docs/DECISIONS.md); read the relevant entries before changing something deliberate.

## Docs

| File | What |
|---|---|
| [docs/DESIGN.md](docs/DESIGN.md) | Architecture, backend, data model, UI, config, themes |
| [docs/PLAN.md](docs/PLAN.md) | Milestones and progress |
| [docs/DECISIONS.md](docs/DECISIONS.md) | What was decided and why |
| [docs/TERMINALS.md](docs/TERMINALS.md) | Terminal-dependent behaviour (images, emoji, colours, bubbles) |
| [docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md) | Checks that need a real terminal |
| [docs/BACKGROUND.md](docs/BACKGROUND.md) | Motivation, research, naming |
| [docs/WEBSITE.md](docs/WEBSITE.md) | The getfaro.dev website |
| [AGENTS.md](AGENTS.md) | Guide for coding agents working on this repo |

## License

faro is [AGPL-3.0](LICENSE). By contributing you agree that your contribution is licensed under the same terms.
