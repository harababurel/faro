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
- [x] No `block_on` / `block_in_place` (tokio, futures) via `disallowed-methods` (M1, D30; `LocalSet::block_on` added in M4, D39)
- [x] The database rejects bad rows: STRICT tables, `CHECK` constraints, foreign keys on; reads validate values into `StoreError::Corrupt` instead of panicking (M2, D34)
- [x] `faro-core` never depends on terminal crates: the `architecture` test fails if its `Cargo.toml` mentions `ratatui` or `crossterm` (M3, D20, D35)
- [x] Network text can't inject escape sequences: everything rendered goes through `ui::text::sanitize`, with a test (M3, D35)
- [x] Rendering never panics on tiny terminals: `ui::tests::tiny_terminals_never_panic` draws every size up to 40×12 (M3)
- [x] Key bindings stay consistent: every default key parses, no key is bound twice in a context, every bindable action has a unique config name and a default key (`app::keymap::tests`, M4, D43)
- [x] Compatible emoji mode draws only graphemes every terminal measures alike, and all text goes through `sanitize` first: `ui::tests::compatible_mode_draws_only_what_every_terminal_measures_alike` renders every screen and checks every cell (M4.5, D47)
- [x] ratatui never sends a wide character's hidden cell (`emoji::tests::hidden_cells_are_never_sent`, M4.5, D47)
- [x] The config template, parser and write-back can't drift: every setting is one `SETTINGS` entry; tests check the template parses to the defaults with no problems, every setting appears in it with its default, uncommenting every setting and key binding changes nothing, and the palette's setting paths exist (`faro_core::config::tests`, `config::tests`, M5, D48)
- [x] Themes stay readable and complete: every built-in theme loads under its name and is listed in the config help; WCAG contrast ratios are checked for text (≥ 7), muted text (≥ 3.5) and coloured text (≥ 3) on every background they're drawn on (incl. both bubbles and the selection), also after the 256-colour mapping (within 10%); the 256/16-colour fallbacks draw no RGB (resp. no indexed) colour on any screen; surfaces stay distinct in 256 colours (`theme::tests`, `ui::tests::the_colour_fallbacks_draw_only_colours_the_terminal_has`, M8, D51)

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

## M4.5: Emoji widths (moved forward from M8 task 2, D46)

The owner's main complaint after trying the prototype: misaligned borders and stray characters around emoji. Fixed before M5 because it affects every screen and needs no config, notifications or real backend.

**Tasks**
1. Measure: `tools/emoji_widths.py` compares the widths faro assumes with what a terminal draws (cursor position reports). The owner runs it inside the tmux where faro runs **and** directly in the terminal they look at (kitty), on each machine they use; agents run it in their own tmux.
2. Make what faro draws agree with the terminal: for each kind of grapheme the terminal measures differently (variation selectors, skin tones, flags, ZWJ sequences, keycaps), draw a fallback with an unambiguous width instead (e.g. drop the selector or skin tone, split a ZWJ sequence into its parts, flags as letters). Detect what's possible at startup; a manual override covers what can't be detected (e.g. the outer terminal behind tmux). The override is a CLI flag/env var until M5 makes it a config setting.
3. Fix what still overflows: sidebar names, bubbles, popups and the palette over emoji; copy (`y`) keeps the original text.
4. Enforce it (D27): a test that no rendered cell holds a grapheme the current mode considers unsafe.

**Acceptance**
- The emoji-heavy mock conversations render with straight borders and no stray characters in the owner's tmux + kitty, locally and over SSH.

## M5: Configuration

**Tasks**
1. Config schema (serde) with defaults; generate commented default file on first run (DESIGN.md §9).
2. Load with error reporting (toast/footer message); keep last good config on error.
3. Hot reload with `notify` (debounced) for config and theme files. (Theme files: M8, with the themes themselves.)
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
3. Map presage contents → `BackendEvent` (data messages, sync messages for own sent messages, receipts, typing, reactions, edits, deletes, group updates, contact sync, read syncs from our other devices clearing unread counts (D42)).
4. Implement `Backend` commands: send, react, mark_read, send_typing, (edit/delete if straightforward).
5. Reconnect with backoff; surface `ConnectionState`; typed errors.
6. Investigate history transfer from phone (D19) and note findings in DECISIONS.md.
7. Select backend at runtime: `--mock` vs default presage.

**Acceptance**
- With the test account: link, receive and send in a direct chat and a group, see receipts/typing/reactions; messages sent from the phone appear as outgoing; restart keeps history; input never freezes during heavy incoming traffic.

## M8: v0.1 hardening and release

**Tasks**
1. Terminal matrix checklist (DESIGN.md §15) on Linux and macOS, with and without tmux, over SSH. The owner does the hands-on part; agents prepare the checklist and fix issues.
2. ~~Emoji-heavy conversations under tmux; fix width issues found.~~ Moved to M4.5 (D46). Here: re-check emoji in the terminal matrix (task 1).
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
| M2 Store | ✅ Done (2026-10-07) | D34. `faro-core/src/store.rs` + `store/` (`db.rs` SQL, `codec.rs` conversions, `tests.rs`); schema in `faro-core/migrations/0001_initial.sql`. sqlx 0.9 is compatible with presage's store (copy its `[patch.crates-io]` in M7). Store task owns the one connection; `Store` handle over a channel. `<data dir>/faro.db`, `0600`. Headless run persists contacts, groups and all events and prints history stats. Acceptance test `store::tests::restart_keeps_mock_history` passes; manual double run kept history. |
| M3 TUI skeleton | ✅ Done (2026-10-07); manual checks pending | D35. ratatui 0.30 + crossterm 0.29 in `crates/faro` only. `app.rs` select loop, `app/` (state, reducer, effects worker, timeline), `tui.rs` (terminal restore incl. panic hook), `theme.rs` (`faro-dark`), `ui/` (sidebar, IRC messages, input panel, info, footer). `headless.rs` removed. 4 insta snapshots (120, 100, 60 columns + loading) and a tiny-size no-panic test. Smoke-tested in tmux by the agent: live updates, switching, typing, quit. Flood (`--mock-activity 2 --mock-no-faults`, ~300 msg/s): input still responsive, memory flat, release build 55% CPU. Hands-on items in [MANUAL_TESTS.md](MANUAL_TESTS.md); known emoji width issue under tmux (M8 task 2). |
| M4 Interactions + palette (first prototype) | ✅ Done (owner's hands-on checks pending) | Keys amended first: `Alt+J/K` switch conversations, arrows kept (D36). Task 1 done: focus cycling, sidebar and message-pane keys, anchored scrolling with older pages loaded on demand, mouse click and wheel, renderer `Geometry` feedback (D37). Task 2 done: input editor with cursor, word/line editing, multi-line, bracketed paste, drafts stored on switch/quit/1 s after editing (D38). Task 3 done: `Enter` sends through a backend command task (`LocalSet`), local `Sending` echo, `Sent`/`Failed` from the result, unsent messages marked failed on startup (D39). Task 4 done: message selection in the message pane, reply bar, reaction picker, OSC 52 copy, open/copy links, action menu with retry, right-click menu, self-clearing info notices (D40). Task 5 done: pin (with order and `Alt+1..9`), mute with durations, hide (back on a new message), archive, allowlist flag (migration 0002) and mode; chats/archive views in the sidebar, conversation menu and mute picker, moving on when the open conversation leaves the list (D41). Task 6 done: read receipts sent only when messages are seen (terminal focus + view at the bottom), unread counts follow the same rule, typing indicators started/refreshed/stopped (D42). Task 7 done: key bindings as a per-context table with stable action names, context-first lookup, `Keymap::apply` for the M5 config, footer hints read from the key map (D43). Task 8 done: `Ctrl+P` palette with `nucleo-matcher` over commands, conversations (incl. hidden/archived) and keybindings (`?`, `F1`); `>`/`@`/`:`/`?` prefixes, recency, keys shown, bulk hide by inactivity, mouse (D44). Task 9 done: bubbles layout (default) beside IRC, switched from the palette and remembered in `app_state`, new `bubble_own`/`bubble_other` theme roles (D45). |
| M4.5 Emoji widths (from M8 task 2) | ✅ Done (2026-10-07; owner's hands-on checks pending) | D46, D47. Task 1: `tools/emoji_widths.py`; the owner's numbers showed **mosh** (between kitty and tmux) is what disagrees; tmux and kitty agree with faro. Task 2: compatible emoji mode (`ui/emoji.rs`), auto-detected from mosh clients of faro's tmux session or faro's own ancestry, re-checked on focus/resize, `--emoji auto\|full\|compatible` override; half flags drawn as letters everywhere. Task 3: palette/picker measure sanitised text; wide characters cut by an overlay border are blanked; `Ctrl+L` repaints. Found while flood-testing: ratatui re-sent the hidden cell after VS16 emoji, shifting the rest of the row in **every** terminal; fixed with `ForcedWidth` (replay: 0 broken frames in 459, was broken at frame 77). Task 4: whole-screen test in compatible mode + diff regression test. |
| M5 Configuration | ✅ Done (2026-10-07; owner's hands-on checks pending) | D48. Task 1: settings table in `faro-core/src/config.rs` with defaults and a commented first-run template (every setting plus every default key binding), `--config`/`FARO_CONFIG`. Task 2: per-setting parsing: bad values keep the previous value and name the key, invalid TOML keeps the last good config, unknown keys warn; problems shown in the footer once. Task 3: `notify` on the config directory, 300 ms debounce, `AppEvent::ConfigRead`, only changes applied (theme files wait for M8). Task 4: `config::set` with `toml_edit` (keeps comments, uncomments the template line), in-place writes from one writer task. Task 5: key bindings, layout, allowlist mode, emoji (`--emoji` wins), mouse capture live, theme name checked, notifications and `reduce_motion` parsed for later; density/timestamps not yet. Preferences moved out of `app_state` once at startup. Task 6: palette `:` settings (layout, allowlist, emoji, mouse) write back. Acceptance test `config::tests::toggling_the_layout_from_the_palette_keeps_the_file_s_comments`; live editing smoke-tested in tmux. |
| M6 Notifications | ✅ Done (2026-10-07; owner's hands-on checks pending) | D49. Task 1: terminal bell (`Effect::Bell`, `update/notify.rs`) for new unseen incoming messages in conversations that notify (main list, not muted, every-message level), at most once per 2 s, `[notifications] bell`. Task 2: window/tmux title `faro (N)` via OSC 0, written only on change, terminal title pushed/popped (blanked where there's no title stack), `[notifications] title`; bell and title are `:` palette settings. Task 3: per-conversation "mentions only" level (migration 0003, `@` in the sidebar, menu, palette; silent until M7 maps mentions), next to mute. Acceptance smoke-tested in a private tmux server: background mock messages set `#{window_bell_flag}` and fire `alert-bell` (≥ 2 s apart), `#{pane_title}` reads `faro (49)` and returns to the old title on quit and when turned off live. Desktop notifications stay v0.2. |
| M7 presage backend (test account needed) | ⏳ Not started | |
| M8 v0.1 hardening + release | 🚧 In progress (tasks 1, 3, 4 pulled ahead of M7, D50) | Task 3 done: themes `faro-dusk`, `faro-light`, `system` (terminal's 16 colours, reverse-video selection, dim muted text) beside `faro-dark`; colour depth detected (`COLORTERM`, tmux, known terminals) with `[ui] colors` override; 256-colour fallback mapped once at load, surfaces kept apart; 16 colours → `system`; `:` palette theme/colour settings with write-back; contrast tests (D51). Task 4 done: README rewritten (pre-release status, install from source incl. the future `protoc` packages, trying the mock, flags, keys, config, recommended tmux settings, mosh, colours) with a text capture of the UI; real screenshots left to the owner (TODO comment in the README). |
