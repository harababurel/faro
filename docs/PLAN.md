# Implementation plan

How to get from an empty repo to v0.1 and beyond. Read [DESIGN.md](DESIGN.md) for *what* to build and [DECISIONS.md](DECISIONS.md) for *why*.

## How to use this file

- Work milestone by milestone, in order. Within a milestone, tasks can be reordered.
- A milestone is done when all its **acceptance criteria** pass.
- Keep the **Progress** section at the bottom up to date (tick tasks, note deviations). If a design decision changes, record it in DECISIONS.md and update DESIGN.md.
- Every milestone ends with: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all green.

## Key constraint: no real Signal account until M7

M0–M6 are built entirely against a **mock backend**. A separate **test Signal account** (second phone number; never the owner's main account) is needed from **M7** onward. **Tell the owner before starting M7** so they can prepare it. The first usable prototype (M0–M4) needs no account at all.

---

## Ongoing: mechanical constraints (D27)

The owner wants correctness and efficiency rules enforced by tools wherever possible (like `unsafe_code = "forbid"`). In every milestone, look for rules the new code makes worth enforcing, add them, and record them in D27 (or a new entry). Adopted so far:

- [x] `unsafe_code = "forbid"`, workspace-wide (M0)
- [x] clippy `pedantic` as errors in CI (M0)
- [x] No panics outside tests: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `unreachable`, `dbg_macro`, `indexing_slicing`, `exit` (M1, D28)
- [x] Never block the UI: `clippy.toml` `disallowed-methods` for blocking std fs/net/process/sleep/stdin; per-function `#[expect]` for legitimate cases (M1, D28)
- [x] No stray terminal output: `print_stdout`, `print_stderr` (M1, D28)
- [x] Lint exceptions need `#[expect(..., reason = "...")]`: `allow_attributes`, `allow_attributes_without_reason` (M1, D28)
- [x] Message bodies and phone numbers can't be logged by accident: `MessageBody`/`PhoneNumber` redact in `Debug`, have no `Display` (M1, D29)
- [x] No `_ =>` on enums: `wildcard_enum_match_arm` (M1, D29)
- [x] No `block_on` / `block_in_place` (tokio, futures) via `disallowed-methods` (M1, D30)

## M0: Scaffolding

**Tasks**
1. Cargo workspace: `crates/faro` (bin), `crates/faro-core` (lib). (`faro-presage` comes in M7.)
2. `rust-toolchain.toml` (stable, with `rustfmt`, `clippy`); edition 2024; workspace-level lints (`unsafe_code = "forbid"` in faro-core, clippy pedantic selectively).
3. GitHub Actions CI: fmt, clippy `-D warnings`, test, on `ubuntu-latest` and `macos-latest`. Cache with `Swatinem/rust-cache`.
4. `faro --version` and `faro --help` via `clap`.
5. Logging with `tracing` to a file in the data dir (never stdout while the TUI is running).
6. README "Development" section: prerequisites and commands.

**Acceptance**
- `cargo run -- --version` prints the version; CI is green on both OSes.

## M1: Core domain and mock backend

**Tasks**
1. Domain types in `faro-core`: `ServiceId`, `GroupId`, `ConversationId`, `Recipient`, `Contact`, `Group`, `Message`, `MessageRef` (sender + sent_at), `Direction`, `DeliveryStatus`, `Reaction`, `Quote`, `AttachmentRef`, `TypingState`, `ConnectionState`.
2. `Backend` trait, `BackendEvent`, `Capabilities`, typed `BackendError` (DESIGN.md §5).
3. `MockBackend`:
   - seeded fake contacts and groups (include emoji-heavy names, long names, many old empty conversations to exercise curation),
   - sends scripted incoming messages, typing indicators, receipts and reactions on timers,
   - echoes sent messages with status progression (sending → sent → delivered → read),
   - can simulate disconnects and errors,
   - deterministic mode (fixed seed, manual clock) for tests.
4. `faro --mock` flag (also the default until M7).

**Acceptance**
- Unit tests show the mock emits a deterministic event sequence for a given seed.

## M2: Store

**Tasks**
1. SQLite via `sqlx` (D16): schema from DESIGN.md §6, migrations embedded.
2. Store task owning the connection; API via channel: upsert recipients/conversations, insert/update messages, reactions, receipts, conversation_state, app_state; queries for conversation list and paginated message history.
3. Paths (D17) with `etcetera`; create dirs `0700`, files `0600`.
4. Persist all `BackendEvent`s; load state at startup.

**Acceptance**
- Migration test on a temp DB; round-trip tests; restart keeps history (mock).

## M3: TUI skeleton

**Tasks**
1. App loop: tokio + `crossterm::event::EventStream`; terminal setup/teardown with panic hook restoring the terminal; alternate screen; mouse capture on by default.
2. Reducer/effects (D22): `update(&mut State, AppEvent) -> Vec<Effect>`; effects dispatched to backend/store tasks; render only when state changes (plus animation ticks when needed).
3. Layout (DESIGN.md §7.1): left sidebar, message pane, **input box as a distinct panel**, optional right sidebar, footer with context-sensitive key hints.
4. Theme struct with semantic roles (DESIGN.md §10) and one built-in theme; all widgets read colours from the theme.
5. Message rendering: IRC layout first; sender colours; date separators; unread marker.
6. Snapshot tests with `TestBackend` + `insta` (normal and narrow widths).

**Acceptance**
- `faro --mock` shows conversations and messages, updates live while you type, never stalls input (manually verified with the mock flooding messages). Snapshots committed.

## M4: Core interactions and the command palette (first prototype)

**Tasks**
1. Conversation switching (keys + mouse click), scrolling (keys + wheel), focus cycling.
2. Input: multi-line (`Alt+Enter`), word movement/deletion, drafts per conversation.
3. Send via backend; show delivery status glyphs.
4. Message selection (`↑` on empty input): reply (quote), react, copy (OSC 52), open link.
5. Conversation curation: hide, archive, pin (with `Alt+1..9`), mute, allowlist mode; hide empty conversations by default.
6. Typing indicators, read receipts (send on view), unread counts.
7. Keybinding system: action enum, default map (DESIGN.md §7.6), rebindable from config (config loading comes in M5; hardcode defaults here behind the same API).
8. Command palette (`Ctrl+P`) with `nucleo`: commands + conversations + (later) settings; keybinding hints; `>`/`@`/`:` prefixes; recency ranking. `F1`/`?` opens it filtered to keybindings.
9. Bubbles layout as an alternative to IRC layout.

**Acceptance**
- A full chat flow against the mock is pleasant to use entirely from the keyboard, and basic mouse use works. **This is the first prototype to show the owner.**

## M5: Configuration

**Tasks**
1. Config schema (serde) with defaults; generate commented default file on first run (DESIGN.md §9).
2. Load with error reporting (toast/footer message); keep last good config on error.
3. Hot reload with `notify` (debounced) for config and theme files.
4. Write-back API using `toml_edit` that changes one key and preserves comments (used by the settings UI in v0.2, and by palette toggles now, e.g. layout switch).
5. Apply: keybindings, messages layout/density/timestamps, mouse on/off, notifications, theme, reduce_motion.

**Acceptance**
- Editing the file while faro runs changes behaviour live; toggling layout from the palette updates the file without losing comments (test).

## M6: Notifications (v0.1 scope)

**Tasks**
1. Terminal bell on new messages (respect mute and focus).
2. Window/tmux title with unread count (OSC 0/2), restored on exit.
3. Per-conversation overrides (mute, mentions only).

**Acceptance**
- Under tmux, an incoming mock message in a background conversation triggers the bell and updates the title.

## M7: presage backend ⚠️ needs the test Signal account

**Tasks**
1. New crate `faro-presage`; add presage + its SQLite store (align sqlx versions, D16). Document build prerequisites (`protoc` etc.) in README.
2. Device linking: render the provisioning URL as a QR code in the terminal (`qrcode` crate, half-block rendering); device name `faro (<hostname>)`.
3. Map presage contents → `BackendEvent` (data messages, sync messages for own sent messages, receipts, typing, reactions, edits, deletes, group updates, contact sync).
4. Implement `Backend` commands: send, react, mark_read, send_typing, (edit/delete if straightforward).
5. Reconnect with backoff; surface `ConnectionState`; typed errors.
6. Investigate history transfer from phone (D19) and note findings in DECISIONS.md.
7. Select backend at runtime: `--mock` vs default presage.

**Acceptance**
- With the test account: link, receive and send in a direct chat and a group, see receipts/typing/reactions; messages sent from the phone appear as outgoing; restart keeps history; input never freezes during heavy incoming traffic.

## M8: v0.1 hardening and release

**Tasks**
1. Terminal matrix checklist (DESIGN.md §15) on Linux and macOS, with and without tmux, over SSH. The owner does the hands-on part; agents prepare the checklist and fix issues.
2. Emoji-heavy conversations under tmux; fix width issues found.
3. Two or three built-in themes + `system` theme; 256-colour fallback.
4. README: install-from-source instructions, tmux recommended settings, screenshots.
5. Tag `v0.1.0`; build release binaries (manual or cargo-dist).

**Acceptance**
- The owner uses faro as their daily Signal client instead of gurk.

---

## v0.2 (after v0.1; details to be planned then)

Drag-resizable sidebars with hover highlight; right-click context menus; settings UI in the palette writing back to TOML; OpenCode theme compatibility + theme picker with live preview; OSC 9/99/777 desktop notifications with privacy levels; edit/delete for everyone; attachments (send/receive/open); emoji picker with `:shortcode:` search and width fallback; packaging (cargo-dist binaries, Homebrew tap, AUR `faro-bin`/`faro-git`, `.deb`, Nix flake).

## Later (backlog: do not lose)

- **Inline images** (kitty graphics / sixel / half-block fallback via `ratatui-image`, tmux passthrough)
- `faro daemon` + attachable TUI
- signal-cli JSON-RPC backend
- Encryption at rest
- Animated lighthouse splash/about screen (port from `tools/logos.py`)
- Message search
- Voice note playback via external player
- Optional sync of hidden/allowlist state via config

---

## Progress

| Milestone | Status | Notes |
|---|---|---|
| Docs, naming, repo setup | ✅ Done (2026-10-07) | README, LICENSE, DESIGN, DECISIONS, BACKGROUND, PLAN, AGENTS, logo prototypes |
| M0 Scaffolding | ✅ Done (2026-10-07); CI green on ubuntu + macos | Commits `8c3fd5b`, `f71ddfb`. `--data-dir`/`FARO_DATA_DIR` added now (needed for the log path); `--config` deferred to M5. `faro` prints a placeholder until M3. 8 tests (paths, CLI, log rotation). Decisions D24–D27. |
| M1 Core + mock backend | ✅ Done (2026-10-07) | D27 lints adopted first (D28). Task 1: domain types in `faro-core/src/domain/` (D29; `Recipient` folded into `Contact`). Task 2: `Backend` trait, events, capabilities, errors in `faro-core/src/backend/` (D30; linking deferred to M7, `send` returns `()` since faro picks `sent_at`). Task 3: `MockBackend` in `faro-core/src/mock.rs` + `mock/` (D31; `fastrand`, tokio paused clock); acceptance test `mock::tests::same_seed_same_event_sequence` passes. Task 4: `faro [--mock] [--mock-seed N]` runs the mock headless until Ctrl+C and logs event kinds plus a summary (D32; `crates/faro/src/headless.rs`, removed in M3). |
| M2 Store | ⏳ Not started | |
| M3 TUI skeleton | ⏳ Not started | |
| M4 Interactions + palette (first prototype) | ⏳ Not started | |
| M5 Configuration | ⏳ Not started | |
| M6 Notifications | ⏳ Not started | |
| M7 presage backend (test account needed) | ⏳ Not started | |
| M8 v0.1 hardening + release | ⏳ Not started | |
