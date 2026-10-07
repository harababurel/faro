# AGENTS.md

Onboarding for coding agents working on faro. Read this first, then the docs it points to.

## What faro is

A terminal (TUI) client for Signal, written in Rust, built for people who run everything over SSH inside tmux. Focus: a polished, discoverable UI that never freezes (OpenCode-style command palette and panels, btop-style colour and glyph graphics). It uses **presage** for the Signal protocol and never reimplements it. Licence: AGPL-3.0. Unofficial; not affiliated with Signal.

## Read before working

| File | Read it for |
|---|---|
| [docs/PLAN.md](docs/PLAN.md) | **Where we are and what to do next.** Milestones, acceptance criteria, progress table. |
| [docs/DESIGN.md](docs/DESIGN.md) | What to build: architecture, backend trait, data model, UI, config, themes, notifications. |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Why things are the way they are, and what was rejected. Don't relitigate accepted decisions without a new reason. |
| [docs/BACKGROUND.md](docs/BACKGROUND.md) | The owner's setup, problems with gurk, research notes, naming history. |

## Current state (update this section as work progresses)

- 2026-10-07: planning finished.
- 2026-10-07: M0 done locally (commits `8c3fd5b`, `f71ddfb`): workspace (`crates/faro`, `crates/faro-core`), CLI, XDG paths with `0700`/`0600`, file logging, CI workflow. Pushed; CI green on ubuntu + macos.
- 2026-10-07: owner asked for correctness/efficiency rules to be enforced by tools wherever possible (D27, rule 9 below). Candidate rules are listed in D27; adopted ones are ticked in PLAN.md "Ongoing: mechanical constraints".
- 2026-10-07: M1 started. D27 rules adopted (D28): no panics, no blocking std calls, no `print!`, and lint exceptions only via `#[expect(lint, reason = "...")]`.
- 2026-10-07: M1 task 1 done: domain types (D29), including redacting `MessageBody`/`PhoneNumber` and the `wildcard_enum_match_arm` lint.
- **Next: M1 task 2 (`Backend` trait, `BackendEvent`, `Capabilities`, `BackendError`)** in docs/PLAN.md.

### Code map

| Path | What |
|---|---|
| `Cargo.toml` | Workspace: shared package metadata, `[workspace.lints]` (unsafe forbidden, clippy pedantic, D28 no-panic/no-print rules), release profile |
| `rust-toolchain.toml` | Pinned Rust version (D25) |
| `clippy.toml` | Test relaxations for panic lints; `disallowed-methods` list of blocking calls (D28) |
| `crates/faro/src/main.rs` | Entry point: parse CLI, resolve paths, start logging. Placeholder output until the TUI (M3) |
| `crates/faro/src/cli.rs` | `clap` CLI definition (`--data-dir` / `FARO_DATA_DIR`) |
| `crates/faro/src/logging.rs` | `tracing` to `<data dir>/logs/faro.log`, non-blocking, `FARO_LOG` filter, rotation over 10 MB (D26) |
| `crates/faro-core/src/domain/` | Domain types (D29): `ids.rs` (ServiceId, GroupId, ConversationId, Timestamp, MessageRef), `sensitive.rs` (redacting MessageBody, PhoneNumber), `people.rs`, `message.rs`, `presence.rs` |
| `crates/faro-core/src/paths.rs` | XDG paths (D17), `0700` dirs / `0600` files helpers, with tests |
| `.github/workflows/ci.yml` | fmt, clippy `-D warnings`, test on ubuntu + macos (`--locked`) |

## Rules

1. **Follow PLAN.md in order** and update its Progress table when a milestone or notable task completes.
2. **Record decisions.** Any new or changed decision goes into DECISIONS.md (append; mark superseded entries) and DESIGN.md is updated to match. Mark agent-made calls as **Judgement**.
3. **Never block the UI thread.** No network or disk I/O in the reducer or render path (DECISIONS D22).
4. **No real Signal account before M7.** Use the mock backend. Before starting M7, ask the owner to prepare a **separate test account**. Never use the owner's main account for development.
5. **Never reimplement Signal protocol/service-layer code** (D4). Domain types are faro's own, not presage's (D6).
6. **Privacy:** never log message bodies or phone numbers by default; never commit Signal data, databases, keys or logs (see `.gitignore`).
7. **Quality gate before every commit:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
8. **Git:** commit and push only when the owner asks. Small commits, short imperative messages (e.g. "Add mock backend event generator").
9. **Prefer rules the tools enforce over conventions** (D27). When new code introduces a rule worth keeping (no panics, no blocking calls in the UI, no secrets in logs…), enforce it with lints, `clippy.toml` bans, CI checks or tests, and list it in PLAN.md "Ongoing: mechanical constraints".
10. **Terminal behaviour can't be fully verified by agents.** When a change affects rendering, input, mouse, colours or emoji, say so and add an item to the manual test checklist instead of claiming it works.

## Working with the owner

- Wants **plain, concise explanations** and clear recommendations with a default ("I'd do X; say if you prefer Y").
- Wants to be **asked before big or irreversible decisions**; for smaller technical choices, use best judgement and record it.
- Values UI/UX polish highly; inspirations are **OpenCode** and **btop**.
- Runs faro inside **tmux**, often over **SSH**, in modern terminals like **kitty**; uses macOS (Mac mini host) and Linux (Arch, Debian).

## Development environment

- Rust stable via rustup (version pinned in `rust-toolchain.toml`; rustup installs it automatically).
- Until M7 nothing beyond Rust is required.
- From M7 (presage): `protoc` (protobuf compiler) is required. libsignal's dependencies may also need `cmake`, `clang` and `pkg-config`; confirm when adding presage and document exact packages in the README.
  - Arch: `sudo pacman -S protobuf cmake clang pkgconf`
  - Debian/Ubuntu: `sudo apt install protobuf-compiler cmake clang pkg-config`
  - macOS: `brew install protobuf cmake`
- Logo prototypes (Python 3, no deps): `python3 tools/logos.py browse lighthouse`.
