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
- 2026-10-07: M0 done (commits `8c3fd5b`, `f71ddfb`): workspace (`crates/faro`, `crates/faro-core`), CLI, XDG paths with `0700`/`0600`, file logging, CI workflow. Pushed; CI green on ubuntu + macos.
- 2026-10-07: owner asked for correctness/efficiency rules to be enforced by tools wherever possible (D27, rule 9 below). Candidate rules are listed in D27; adopted ones are ticked in PLAN.md "Ongoing: mechanical constraints".
- 2026-10-07: M1 started. D27 rules adopted (D28): no panics, no blocking std calls, no `print!`, and lint exceptions only via `#[expect(lint, reason = "...")]`.
- 2026-10-07: M1 task 1 done: domain types (D29), including redacting `MessageBody`/`PhoneNumber` and the `wildcard_enum_match_arm` lint.
- 2026-10-07: M1 task 2 done: `Backend` trait, `BackendEvent`, `Capabilities`, `BackendError` (D30), and the `block_on` ban.
- 2026-10-07: development moved from the owner's Mac mini to a Debian Linux machine as well; the toolchain, build and SSH push access are verified there (`protoc` installed, so M7 prerequisites are met).
- 2026-10-07: M1 task 3 done: `MockBackend` (D31). M1 acceptance test passes (`mock::tests::same_seed_same_event_sequence`).
- 2026-10-07: M1 task 4 done, **M1 complete**: `faro [--mock] [--mock-seed N]` runs the mock headless on a current-thread tokio runtime until Ctrl+C, logging event kinds and a summary (D32).
- **Next: M2 (store)**; see docs/PLAN.md M2 and DESIGN.md §6. Notes:
  - D16 says `sqlx` (SQLite) aligned with presage's store. Two crates linking different `libsqlite3-sys` versions fail to build (`links = "sqlite3"`), so check which `sqlx` version presage's SQLite store uses before choosing; ask the owner if it forces an old version.
  - The store runs in its own task and is the only owner of the connection; the app talks to it over a channel (D22). Blocking calls are banned (D28); `sqlx` is async, so that should be fine.
  - Paths (`Paths`, `0700`/`0600` helpers) already exist in `faro-core/src/paths.rs`; the DB lives in the data dir.
  - Acceptance includes "restart keeps history (mock)": the mock's cast IDs are stable across runs (D31), but each run's backlog is new messages with new timestamps, so history grows on each restart; that's expected.
  - Once the store exists, `headless.rs` could persist events too, so a restart can be checked from the CLI before the TUI exists.

### Gotchas found so far

- `#[expect(...)]` on a macro call (`println!`) is ignored; put it on the enclosing `fn`.
- `#[tokio::test]` expands to `block_on`, which is banned: add `#[expect(clippy::disallowed_methods, reason = "#[tokio::test] expands to block_on; tests may block")]` (on the test fn, or once on the whole `mod tests`).
- Trait impls with `async fn` bodies that never `.await` trip `clippy::unused_async_trait_impl`. In real impls, write those methods as `fn ... -> impl Future<Output = ...>` returning `std::future::ready(...)` (see `mock.rs`).
- `wildcard_enum_match_arm` doesn't catch `_` inside tuple patterns; spell out variants by hand. In tests, prefer `let ... else` / `if let` over a `_ =>` arm.
- Blocking std calls (fs, `Path::exists`, sleep…) are banned everywhere; startup code and tests opt out per function/module with `#[expect(clippy::disallowed_methods, reason = "...")]`.
- `fastrand::Rng::choice` needs an `ExactSizeIterator`: collect a filtered iterator into a `Vec` first.
- Pedantic `too_many_lines` caps functions at 100 lines; split big `match`es into one method per arm.
- Commit and push automatically after each finished task or milestone (D33, superseding D23); don't wait to be asked.

### Code map

| Path | What |
|---|---|
| `Cargo.toml` | Workspace: shared package metadata, `[workspace.lints]` (unsafe forbidden, clippy pedantic, D28 no-panic/no-print rules), release profile |
| `rust-toolchain.toml` | Pinned Rust version (D25) |
| `clippy.toml` | Test relaxations for panic lints; `disallowed-methods` list of blocking calls incl. `block_on` (D28, D30) |
| `crates/faro/src/main.rs` | Entry point: parse CLI, resolve paths, start logging (synchronous), then a current-thread tokio runtime running the mock headless until Ctrl+C (D32) |
| `crates/faro/src/cli.rs` | `clap` CLI definition (`--data-dir` / `FARO_DATA_DIR`, `--mock`, `--mock-seed` / `FARO_MOCK_SEED`) |
| `crates/faro/src/headless.rs` | Interim stand-in for the TUI: drives a `Backend`, logs event kinds (never content), returns a `Summary`. Delete in M3 |
| `crates/faro/src/logging.rs` | `tracing` to `<data dir>/logs/faro.log`, non-blocking, `FARO_LOG` filter, rotation over 10 MB (D26) |
| `crates/faro-core/src/backend.rs`, `backend/` | `Backend` trait, `Account`, `OutgoingMessage`/`OutgoingEdit`, `Capabilities`; `event.rs` (`BackendEvent`), `error.rs` (`BackendError`) (D30) |
| `crates/faro-core/src/domain/` | Domain types (D29): `ids.rs` (ServiceId, GroupId, ConversationId, Timestamp, MessageRef), `sensitive.rs` (redacting MessageBody, PhoneNumber), `people.rs`, `message.rs`, `presence.rs` |
| `crates/faro-core/src/mock.rs`, `mock/` | `MockBackend` + `MockConfig` (D31): `mock.rs` (Backend impl, driver task, paused-clock tests), `sim.rs` (seeded synchronous scheduler: backlog, scripted activity, receipts, outages), `world.rs` (fixed cast, canned texts) |
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
8. **Git:** after finishing each well-defined chunk of work (a PLAN task or milestone), with the quality gate green, **commit and push without asking** (D33). Small commits, short imperative messages (e.g. "Add mock backend event generator"). Never rewrite pushed history.
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
