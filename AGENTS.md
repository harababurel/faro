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

- 2026-10-07: planning finished; no Rust code yet. **Next: M0 (scaffolding) in docs/PLAN.md.**

## Rules

1. **Follow PLAN.md in order** and update its Progress table when a milestone or notable task completes.
2. **Record decisions.** Any new or changed decision goes into DECISIONS.md (append; mark superseded entries) and DESIGN.md is updated to match. Mark agent-made calls as **Judgement**.
3. **Never block the UI thread.** No network or disk I/O in the reducer or render path (DECISIONS D22).
4. **No real Signal account before M7.** Use the mock backend. Before starting M7, ask the owner to prepare a **separate test account**. Never use the owner's main account for development.
5. **Never reimplement Signal protocol/service-layer code** (D4). Domain types are faro's own, not presage's (D6).
6. **Privacy:** never log message bodies or phone numbers by default; never commit Signal data, databases, keys or logs (see `.gitignore`).
7. **Quality gate before every commit:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
8. **Git:** commit and push only when the owner asks. Small commits, short imperative messages (e.g. "Add mock backend event generator").
9. **Terminal behaviour can't be fully verified by agents.** When a change affects rendering, input, mouse, colours or emoji, say so and add an item to the manual test checklist instead of claiming it works.

## Working with the owner

- Wants **plain, concise explanations** and clear recommendations with a default ("I'd do X; say if you prefer Y").
- Wants to be **asked before big or irreversible decisions**; for smaller technical choices, use best judgement and record it.
- Values UI/UX polish highly; inspirations are **OpenCode** and **btop**.
- Runs faro inside **tmux**, often over **SSH**, in modern terminals like **kitty**; uses macOS (Mac mini host) and Linux (Arch, Debian).

## Development environment

- Rust stable via rustup (version pinned in `rust-toolchain.toml` once M0 lands).
- Until M7 nothing beyond Rust is required.
- From M7 (presage): `protoc` (protobuf compiler) is required. libsignal's dependencies may also need `cmake`, `clang` and `pkg-config`; confirm when adding presage and document exact packages in the README.
  - Arch: `sudo pacman -S protobuf cmake clang pkgconf`
  - Debian/Ubuntu: `sudo apt install protobuf-compiler cmake clang pkg-config`
  - macOS: `brew install protobuf cmake`
- Logo prototypes (Python 3, no deps): `python3 tools/logos.py browse lighthouse`.
