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
| [docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md) | Hands-on checks only a human at a real terminal can do; add to it per rule 10. |

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
- 2026-10-07: **M2 complete** (store, D34): `faro-core::store` (SQLite via sqlx 0.9, one task owns the connection, `Store` handle over a channel), schema in `crates/faro-core/migrations/`. The headless run persists everything to `<data dir>/faro.db`; restart keeps history (tested and checked by hand).
- 2026-10-07: **M3 complete** (TUI skeleton, D35): ratatui + crossterm in `crates/faro`; select loop, synchronous reducer, sequential effects worker, IRC message view, sidebars, input panel, footer, `faro-dark` theme, insta snapshots. `headless.rs` is gone. Flood-tested in tmux (`--mock-activity 2 --mock-no-faults`). Hands-on checks for the owner are in `docs/MANUAL_TESTS.md`.
- 2026-10-07: keys amended after the owner's first hands-on look: `Alt+J/K` switch conversations, arrows kept as a second binding (D36). Emoji width artifacts confirmed by the owner; still planned for M8.
- 2026-10-07: M4 task 1 done (D37): focus (`Tab`), per-pane keys (`app/update/keys.rs`), anchored scrolling + `LoadOlder`, mouse click/wheel (`app/update/navigation.rs`), `ui::render` returns `Geometry`, fed back as `AppEvent::Drawn`.
- **Next: M4 task 2 (multi-line input, word movement/deletion, persisted drafts)**, then tasks 3–9; see docs/PLAN.md M4 and DESIGN.md §7.2–7.7. Notes:
  - **Sending needs the backend inside the app.** Today `main` passes only the event receiver to `app::run` and keeps the `MockBackend` alive outside it. Backend futures may be `!Send` (presage, D30), so run commands on the current-thread runtime (e.g. a command task that owns the backend, fed by a channel from the effects side), never in the reducer. The app owns the `Sending` state (D31): insert the outgoing message locally, then apply `Sent`/`Failed` from the result.
  - Keys live in `app/update/keys.rs` (`key_action(key, focus)`, tuple matches per pane); M4 task 7 makes `Action` rebindable. Shifted characters arrive with `SHIFT` stripped before matching. Input editing is still append-only (`Insert`/`DeleteBackward` in `navigation.rs`); task 2 needs a cursor in `OpenConversation`.
  - Drafts are in-memory (`State::drafts`); the store already has `ConversationState::draft` and `set_conversation_state` for persisting them. Hide/archive/pin/mute map to the same struct.
  - The unread marker is fixed when a conversation opens (`OpenConversation::unread_marker`); read receipts to the backend (`mark_read`) are M4 task 6.
  - Add every new key, mouse or rendering behaviour to `docs/MANUAL_TESTS.md` (rule 10).

### Gotchas found so far

- `#[expect(...)]` on a macro call (`println!`) is ignored; put it on the enclosing `fn`.
- `#[tokio::test]` expands to `block_on`, which is banned: add `#[expect(clippy::disallowed_methods, reason = "#[tokio::test] expands to block_on; tests may block")]` (on the test fn, or once on the whole `mod tests`).
- Trait impls with `async fn` bodies that never `.await` trip `clippy::unused_async_trait_impl`. In real impls, write those methods as `fn ... -> impl Future<Output = ...>` returning `std::future::ready(...)` (see `mock.rs`).
- `wildcard_enum_match_arm` doesn't catch `_` inside tuple patterns; spell out variants by hand. In tests, prefer `let ... else` / `if let` over a `_ =>` arm.
- Blocking std calls (fs, `Path::exists`, sleep…) are banned everywhere; startup code and tests opt out per function/module with `#[expect(clippy::disallowed_methods, reason = "...")]`.
- `fastrand::Rng::choice` needs an `ExactSizeIterator`: collect a filtered iterator into a `Vec` first.
- Pedantic `too_many_lines` caps functions at 100 lines; split big `match`es into one method per arm.
- Commit and push automatically after each finished task or milestone (D33, superseding D23); don't wait to be asked.
- Store tests use real time, not `start_paused`: SQLite works on its own thread, so a paused clock auto-advances while waiting and timers fire early (D34).
- Pedantic clippy wants `as_chunks::<N>()` over `chunks_exact(N)`, `type` aliases at module level (`items_after_statements`), and `assert_eq!(v, vec![])` over `assert!(v.is_empty())`.
- Store SQL uses runtime `sqlx::query`/`query_as` with tuple rows (up to 16 columns), not `query!` (needs a DB at compile time).
- **Snapshots:** after a UI change run `INSTA_UPDATE=always cargo test -p faro`, then read `git diff crates/faro/src/ui/snapshots/` before committing. Snapshot files store cells, so wide or variation-selector emoji make rows look misaligned in the `.snap` file even when the layout is right.
- **Smoke-testing the TUI from an agent:** use a private tmux server and `respawn-pane` (chaining the command into `new-session` fails silently): `tmux -L faro-smoke -f /dev/null new-session -d -x 110 -y 30 -s s; tmux -L faro-smoke set -g remain-on-exit on; tmux -L faro-smoke respawn-pane -k -t s "./target/debug/faro --data-dir <scratch dir>"`. Drive it with `send-keys` (`M-Down`, `-l 'text'`, `C-q`), read it with `capture-pane -p`, then `kill-server`. Always use a scratch `--data-dir`.
- **Flood tests** need `--mock-no-faults` as well as a small `--mock-activity`, or the mock spends most of its time in scripted outages. Judge CPU on a release build; debug is several times slower and takes seconds to flush queued writes on quit.
- Key handling: match on `(code, modifiers)` tuples (no wildcard-lint trouble). Event handlers take `&Event` (`needless_pass_by_value`). `single_match_else` wants `if let ... else`.
- **Run the quality gate with `set -o pipefail`** if you pipe its output (`cargo clippy ... | tail`): otherwise the pipe reports `tail`'s exit code and a failing gate looks green (this once let a lint failure get pushed).

### Code map

| Path | What |
|---|---|
| `Cargo.toml` | Workspace: shared package metadata, `[workspace.lints]` (unsafe forbidden, clippy pedantic, D28 no-panic/no-print rules), release profile |
| `rust-toolchain.toml` | Pinned Rust version (D25) |
| `clippy.toml` | Test relaxations for panic lints; `disallowed-methods` list of blocking calls incl. `block_on` (D28, D30) |
| `crates/faro/src/main.rs` | Entry point: parse CLI, resolve paths, start logging, read the time zone (synchronous), then a current-thread tokio runtime: open the store, start the mock, load the directory, run the TUI, close the store |
| `crates/faro/src/cli.rs` | `clap` CLI definition (`--data-dir` / `FARO_DATA_DIR`, `--mock`, `--mock-seed`, `--mock-activity`, `--mock-no-faults`) |
| `crates/faro/src/app.rs` | `Directory`/`load_directory` (account, contacts, groups) and `run`: the `tokio::select!` loop over terminal, backend, store results, redraw timer, typing deadline and signals (D35) |
| `crates/faro/src/app/` | `event.rs` (`AppEvent`, `Effect`), `state.rs` (`State`, `OpenConversation`, panels, notices, title/name helpers), `update.rs` + `update/` (the reducer; `keys.rs` key map, `navigation.rs` focus/scroll/mouse, `tests.rs`), `timeline.rs` (in-memory event application matching the store), `effects.rs` (sequential effects worker) |
| `crates/faro/src/tui.rs` | `Tui`: raw mode, alternate screen, mouse capture, panic hook, synchronized-output draws; restores the terminal on drop |
| `crates/faro/src/theme.rs` | `Theme` with semantic roles and sender palette; built-in `faro-dark` |
| `crates/faro/src/ui.rs`, `ui/` | Pure rendering from `State`: layout in `ui.rs`; `sidebar.rs`, `conversation.rs` (pane, typing line, input panel), `messages.rs` (IRC layout), `info.rs`, `footer.rs`, `text.rs` (sanitize, width, wrap), `tests.rs` + `snapshots/` |
| `crates/faro/src/fixtures.rs` | Test-only cast, history and loaded state for reducer and snapshot tests |
| `crates/faro/src/logging.rs` | `tracing` to `<data dir>/logs/faro.log`, non-blocking, `FARO_LOG` filter, rotation over 10 MB (D26) |
| `crates/faro-core/src/backend.rs`, `backend/` | `Backend` trait, `Account`, `OutgoingMessage`/`OutgoingEdit`, `Capabilities`; `event.rs` (`BackendEvent`), `error.rs` (`BackendError`) (D30) |
| `crates/faro-core/src/domain/` | Domain types (D29): `ids.rs` (ServiceId, GroupId, ConversationId, Timestamp, MessageRef), `sensitive.rs` (redacting MessageBody, PhoneNumber), `people.rs`, `message.rs`, `presence.rs` |
| `crates/faro-core/src/mock.rs`, `mock/` | `MockBackend` + `MockConfig` (D31): `mock.rs` (Backend impl, driver task, paused-clock tests), `sim.rs` (seeded synchronous scheduler: backlog, scripted activity, receipts, outages), `world.rs` (fixed cast, canned texts) |
| `crates/faro-core/src/paths.rs` | XDG paths (D17) incl. `db_file()`, `0700` dirs / `0600` files helpers, with tests |
| `crates/faro-core/src/store.rs`, `store/` | `Store` handle, request channel and serve task, `StoreError`, `ConversationSummary`/`ConversationState` (D34); `db.rs` (all SQL), `codec.rs` (domain ↔ column conversions), `tests.rs` (incl. restart acceptance test) |
| `crates/faro-core/migrations/` | SQLite schema, embedded with `sqlx::migrate!()` and run on open |
| `docs/MANUAL_TESTS.md` | Hands-on checklist for things agents can't verify (rule 10) |
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
10. **Terminal behaviour can't be fully verified by agents.** When a change affects rendering, input, mouse, colours or emoji, say so and add an item to [docs/MANUAL_TESTS.md](docs/MANUAL_TESTS.md) instead of claiming it works.

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
