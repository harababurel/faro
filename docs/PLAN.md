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
- [x] tmux can't mis-join emoji: the backend blanks the cell after every grapheme tmux widens (`tui::backend::tests`), flags never touch (`emoji::tests::flags_drawn_side_by_side_become_letters`), loose joiners and skin tones tmux can't join are dropped (D55)
- [x] The config template, parser and write-back can't drift: every setting is one `SETTINGS` entry; tests check the template parses to the defaults with no problems, every setting appears in it with its default, uncommenting every setting and key binding changes nothing, and the palette's setting paths exist (`faro_core::config::tests`, `config::tests`, M5, D48)
- [x] Themes stay readable and complete: every built-in theme loads under its name and is listed in the config help; WCAG contrast ratios are checked for text (≥ 7), muted text (≥ 3.5) and coloured text (≥ 3) on every background they're drawn on (incl. both bubbles and the selection), also after the 256-colour mapping (within 10%); the 256/16-colour fallbacks draw no RGB (resp. no indexed) colour on any screen; surfaces stay distinct in 256 colours (`theme::tests`, `ui::tests::the_colour_fallbacks_draw_only_colours_the_terminal_has`, M8, D51)
- [x] Imported themes stay usable: all 33 bundled OpenCode themes import; derived colours (unread, sender names, own bubbles) meet their contrast rules, coloured and muted text meets the 3:1 floor, sender colours are distinct, surfaces stay apart in 256 colours and every theme's fallbacks draw only colours the terminal has; user theme files are named, layered and reported (`theme::tests::files`, D54)
- [x] Inline images never corrupt terminal frames or leak colours: `emoji::finish` skips graphics-protocol cells (`emoji::tests::graphics_protocol_cells_are_left_alone`), half-blocks map to xterm-256 at 256 colours (`ui::tests::inline_images_show_a_placeholder_then_draw_halfblocks_and_map_to_256_colours`), graphics protocols fall back to placeholders while overlays are open, and `tools/frames` `images` scenario replays with 0 divergent frames (M9, D78)
- [x] Voice transcription keys never live in config or process arguments, and remote transcription requires both global consent and per-conversation permission (`faro_core::config::tests::voice_settings_parse_and_reject_api_key_in_config`, `app::update::tests::voice_note_remote_consent_flow`, `curl --config -` over stdin, M10, D82)
- [x] Terminal replies can't crash faro or exhaust memory: the OSC 5522 reader never panics, never stores more than its limit and never buffers more than one packet, whatever the input (`faro_core::clipboard::tests::survives_corrupted_and_random_input`, `an_endless_packet_is_dropped_not_buffered`; D89 Phase 1)
- [x] A clipboard reply never reaches the reducer as keys, and pasted images are private: `app::clipboard::tests::a_late_reply_never_reaches_the_reducer` feeds a recorded reply through the late-reply guard (both `ESC` forms); `saves_the_image_private_with_its_size` checks `0600` (D90, D91)
- [x] Host focus release happens only at the end of Escape's dismissal chain (`app::update::tests::focus`, web session test); resizing an unfocused app cannot send read receipts (`presence::a_resize_does_not_reclaim_focus_or_send_receipts`). Demo startup opens its chat without pretending the visitor typed (`faro_web::demo` / `session` tests; D125).
- [x] Web panel actions use shared settings/palette reducers and preserve keyboard focus/drafts (native `faro_web::session` tests); font refit rejects non-finite values and sizes outside 13–32 px (`metrics::valid_font_size` tests; D126). Browser input/layout checks run in the CI site job (D130); real-browser/device checks remain in MANUAL_TESTS.
- [x] Phone stills are deterministic, scheme/current-marker consistent and self-contained for bundled font glyphs; unsupported grapheme runs retain their cell positions (`faro-site::scene::phone`/`phone_font` tests, D128–D129). Generated phone assets must be hashed, six in total, and within existing first-load/per-frame limits (`site/check.py`). Responsive loader regressions and real-browser tests are required in the CI site job (D130).
- [x] The browser harness fails on zero assertions, missing prerequisites, console/network errors, failed assertions and timeouts; CI requires Chrome and Node 22, never silently skips no-JS coverage, and retains bounded failure artifacts (D130; harness guard/unit checks).

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
3. Hot reload with `notify` (debounced) for config and theme files. (Theme files came later, with D54.)
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
1. ✅ Terminal matrix checklist (DESIGN.md §15) on Linux and macOS, with and without tmux, over SSH. The owner does the hands-on part; agents prepare the checklist and fix issues. (Owner's sign-off 2026-10-13 from weeks of daily use; see MANUAL_TESTS "M8: terminal matrix" for what wasn't covered.)
2. ~~Emoji-heavy conversations under tmux; fix width issues found.~~ Moved to M4.5 (D46). Here: re-check emoji in the terminal matrix (task 1).
3. Two or three built-in themes + `system` theme; 256-colour fallback.
4. README: install-from-source instructions, tmux recommended settings, screenshots.
5. ✅ Make the allowlist guard optional (D95, done 2026-10-11): the Signal backend starts with no flag, `--allowlist` stays for testing, `--no-allowlist` becomes a no-op.
6. ✅ Sidebar markers (D96, done 2026-10-11): `#` before group names (also in the palette, where direct chats lose their `@`), and on the right edge before the unread count `⊘` for muted and `@` for mentions only, in the muted colour.
7. ✅ Bubble shapes (D98, done 2026-10-11; owner's hands-on checks pending): quarter-circle corners on taller rounded bubbles, `[messages] bubble_padding`, compact runs that join on their aligned side, `[messages] match_widths`, a blank line between runs in compact.
8. ✅ Tag `v0.1.0`; build release binaries (manual or cargo-dist). Make the repo public first; then upload `faro-git` to the AUR, add `faro` and `faro-bin` PKGBUILDs and the Homebrew formula in `harababurel/tap` (D86). (Done 2026-10-13: GitHub release with 4 archives and 2 debs, `brew install harababurel/tap/faro`, AUR `faro`/`faro-bin`/`faro-git`; the owner installed it with Homebrew on the Mac mini and `faro-bin` on the Arch server (all three AUR packages installed at least once while publishing); the `.deb` was checked on a Debian server on 2026-10-17 (v0.1.1); build-from-source not yet tried.)
   Packaging plan (D133): `dist` builds macOS arm64/x86_64 and Linux x86_64/aarch64 archives with checksums and pushes the Homebrew formula; `cargo-deb` adds `faro_amd64.deb`/`faro_arm64.deb` (stable names for `releases/latest/download/`); `faro-bin` uses the Linux archive. No `curl | sh` installer. Nix later. Then the clean-machine check: one install per method (brew on macOS, `faro-bin` on Arch, the `.deb` on Debian, from source).

**Acceptance**
- The owner uses faro as their daily Signal client instead of gurk.

---

## v0.2 (after v0.1; details to be planned then)

Media has its own milestones (M9, M10, D69); the rest of v0.2 is planned when v0.1 ships:

Drag-resizable sidebars with hover highlight; right-click context menus; settings UI in the palette writing back to TOML; ~~OpenCode theme compatibility + theme picker with live preview~~ (done early, D54); OSC 9/99/777 desktop notifications with privacy levels; edit/delete for everyone; emoji picker with `:shortcode:` search and width fallback; packaging (cargo-dist binaries, Homebrew tap, AUR `faro-bin`/`faro-git`, `.deb`, Nix flake).

## M9: attachments and inline images (v0.2, D69)

Designed in DESIGN.md §7.9 (D75–D79). Defaults: images download by themselves up to 25 MB; inline images `auto`; one file per sent message.

**Tasks**
1. Receive: download attachments through presage into `<data dir>/attachments/` (`0600`), with a download status (the planned `attachments.status` migration), a size limit above which faro asks first, and progress shown on the message. Never on the UI thread.
2. Open and save: the system viewer (`open` / `xdg-open`) when faro runs on the machine in front of the user. Over SSH or mosh that would open on the remote machine, so offer the alternatives there: show it inline (task 4), save it to a chosen folder, copy its path.
3. Send: attach a file from the input (a path, with completion; a pasted path, which is what dragging a file into most terminals gives), a caption, and the upload progress. Sending stays behind the guard (D62).
4. Inline images: thumbnails in the message pane, a full-size view in a popup. `ratatui-image` with the kitty graphics protocol, sixel or iTerm2 where the terminal has one, and half-block cells everywhere else. Through tmux, passthrough (`allow-passthrough on`, already in the recommended settings); through mosh, half-blocks only. Detection like D47/D58 (terminal and every tmux client), and a setting such as `[media] inline_images = "auto" | "halfblocks" | "off"`. Stickers drawn as images too.
5. Rendering safety: images must not break the D55 frame checks (redraw on scroll and resize, clear on palette/popup overlap); add the cases to `tools/frames`.

**Acceptance**
- A photo sent from the phone shows inline in kitty directly and through tmux, as half-blocks through mosh, and opens or saves; a file sent from faro arrives on the phone.

## M10: voice notes and transcription (v0.2, D69)

Designed in DESIGN.md §7.10 (D80–D82). Defaults: voice notes auto-download up to 25 MB; external player `auto`; transcription `off` until enabled (`local` or `remote`, with explicit consent and per-conversation permission for `remote`).

**Tasks**
1. Voice notes as their own row: a play glyph, the duration and, where cheap, a waveform drawn from the audio.
2. Playback through an external player, configurable (e.g. `afplay` on macOS, `mpv --no-video` / `pw-play` on Linux), with play/stop keys and progress. Over SSH or mosh the sound would play on the remote machine: say so, and offer save/copy instead (no audio forwarding planned).
3. **Transcription, opt-in (owner's idea, 2026-10-08):** off by default. When the user sets it up, incoming voice notes (or ones chosen by hand) are transcribed in the background and the text shown under the note and kept in faro's database. Two kinds of backend, chosen in the config:
   - **local**: a Whisper model on the user's machine (e.g. whisper.cpp through `whisper-rs`, or an external command such as `whisper-cli`), with the model's path or a one-off download;
   - **remote**: an OpenAI-compatible `/audio/transcriptions` endpoint (OpenAI, Groq, a self-hosted server), with the key read from the environment or the OS keychain, never from the config file.

   The remote kind sends other people's voice to a third party: faro asks for explicit consent when it's turned on, says so where the transcript is shown, and allows it per conversation. Language detection, a length limit and the cost are open questions for the design.
4. Not planned yet: recording and sending voice notes from faro.

**Acceptance**
- A voice note from the phone plays locally and, with transcription set up, shows its text a few seconds later without freezing anything.

Pulled forward on 2026-10-08 at the owner's request: the palette as a tree (D52), do not disturb (D53), and OpenCode's themes bundled, user theme files with hot reload and the live-preview theme picker (D54).

## M11: history transfer on link (v0.2, proposed, D74)

Outline only. Bring the phone's message history into faro when linking, as Signal Desktop does.

**Tasks**
1. Get `ephemeralBackupKey` from presage (upstream PR #430, or a pinned fork until it lands).
2. Long-poll `GET /v1/devices/transfer_archive`, download the archive from the CDN, verify and decrypt it.
3. Parse it with `libsignal-message-backup` (same libsignal tag as libsignal-service-rs) and map text, quotes, reactions, contacts and groups into faro's store as stored events, through the allowlist guard.
4. `faro link`: advertise `capabilities=backup5` (only now that 1–3 work), show progress, let the user skip, handle `RELINK_REQUESTED` and `CONTINUE_WITHOUT_UPLOAD`.
5. Out of scope: media backfill. Existing installs must re-link to use it.

**Acceptance**
- Linking a fresh data dir with "Transfer message history" chosen on the phone shows the recent history of the allowed chats in faro.

## M12: UX, messaging, search and polish (v0.2, D83)

All 11 features approved by the owner on 2026-10-08. Organized into **3 sequential waves of parallel tracks** so an orchestrating agent can spawn 2–3 subagents concurrently per wave using isolated worktrees (`Workspace="share"`) with minimal file overlap.

### Parallel Subagent Protocol (for all M12 waves)
- **Isolated worktrees:** Spawn each track in a wave with `Workspace="share"` (or a separate clone/worktree) on its own local branch (`git fetch origin && git switch -c m12-track-1a origin/main`).
- **Quality gate & rebase before push:** Each worker runs `set -o pipefail; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --workspace --exclude faro && cargo test -p faro -- --test-threads=1`, then `git fetch origin && git rebase origin/main`, re-runs the gate if the rebase brought changes (regenerating insta snapshots with `INSTA_UPDATE=always cargo test -p faro -- --test-threads=1` if needed), and pushes with `git push origin HEAD:main`.
- **Sequential wave boundary:** Wait for all tracks in **Wave N** to push to `origin/main` before spawning **Wave N+1**.
- **Non-workspace `view_file` ban:** Subagents must **never** call `view_file` on paths outside the repo workspace (such as `~/.cargo/registry/...`); use `run_command` if inspecting external crate sources.

---

### Wave 1 — Protocol Actions, Store Plumbing & Palette/Notification Core (3 Parallel Tracks)

Run **Tracks 1A, 1B, and 1C in parallel**:

#### Track 1A: Outgoing Edit, Delete for Everyone & Sending `@mentions`
- **Primary files:** `crates/faro-core/src/backend.rs`, `crates/faro-core/src/guard.rs`, `crates/faro-core/src/mock.rs`, `crates/faro-presage/src/{backend.rs,convert.rs}`, `crates/faro/src/app/{commands.rs,update/sending.rs,update/selection.rs}`, `crates/faro/src/ui/input.rs`.
- **Tasks:**
  1. **Edit & Delete for Everyone (`e` / `D` in `messages`):**
     - Add `Backend::edit(&mut self, conversation: ConversationId, target_sent_at: Timestamp, new_body: MessageBody, mentions: Vec<Mention>)` and `Backend::delete(&mut self, conversation: ConversationId, target_sent_at: Timestamp)` (guarded by `faro-core::guard`, simulated in `MockBackend`, mapped to Signal `EditMessage` and `DataMessage::Delete` in `faro-presage`).
     - Pressing `e` (`Action::EditMessage`, `message.edit`) on an outgoing message loads its text into the input editor with an `✎ Editing message · esc cancel` bar (`OpenConversation::editing: Option<Timestamp>`); `Enter` dispatches `Command::Edit`, updates the timeline and store (`BackendEvent::Edited`), and clears the bar.
     - Pressing `D` (`Action::DeleteMessage`, `message.delete`) on an outgoing message opens `Popup::ConfirmDelete(Timestamp)`; confirming dispatches `Command::Delete` and marks the message deleted in the timeline and store (`BackendEvent::Deleted`).
  2. **Sending `@mentions` with inline `@` autocomplete:**
     - Add `mentions: Vec<Mention>` to `OutgoingMessage` and encode UTF-16 `BodyRange` (`AssociatedValue::MentionAci`) in `faro-presage::convert`.
     - In `Input` focus, typing `@` followed by characters opens an inline completion popup above the input box listing participants of the open conversation matching the prefix; `Tab`/`Enter` inserts `@Name` and tracks the mention so sending builds accurate byte-offset `Mention` structs.

#### Track 1B: Delivery Receipts, "Message Info" Popup & Search/Jump Store Queries
- **Primary files:** `crates/faro-core/migrations/0008_receipts.sql` (**only Track 1B adds a migration in M12**), `crates/faro-core/src/store/` (`db.rs`, `db/messages.rs`, `tests.rs`), `crates/faro-presage/src/convert.rs`, `crates/faro/src/ui/{messages.rs,popup.rs}`.
- **Tasks:**
  1. **Delivery receipts & per-recipient tracking (`0008_receipts.sql`):**
     - Map `ReceiptMessage::Type::Delivery` in `faro-presage::convert::receipt` to `DeliveryStatus::Delivered` (currently ignored).
     - Add migration `0008_receipts.sql` creating `message_receipts (conversation_id, sender, sent_at, recipient, status, updated_at)` to record per-recipient delivery and read timestamps in groups and direct chats.
     - Render `DeliveryStatus::Delivered` as dim `✓✓` and `DeliveryStatus::Read` as `accent`-coloured `✓✓` in `ui/messages.rs`.
     - Add `i` (`Action::MessageInfo`, `message.info`) and menu item *Message info* opening `Popup::MessageInfo`, displaying sender, sent/edited timestamp, attachment details, and per-recipient delivered/read timestamps.
  2. **Store queries for search & context jump (used by Track 2A in Wave 2):**
     - Add `Store::search_messages(conversation: Option<ConversationId>, query: &str, limit: u32) -> Vec<SearchHit>` searching `messages.body` and `attachments.transcript` (case-insensitive substring/token match, newest first).
     - Add `Store::messages_around(conversation: ConversationId, target: Timestamp, radius: u32) -> Vec<StoredMessage>` so the UI can jump to any historical message in context.

#### Track 1C: `Ctrl+P` Palette Navigation & Multi-Edit + OSC Desktop Notifications
- **Primary files:** `crates/faro/src/app/{palette.rs,update/palette.rs,update/settings.rs,update/notify.rs}`, `crates/faro/src/ui/palette.rs`, `crates/faro/src/tui.rs`.
- **Tasks:**
  1. **Palette `←`/`→` value cycling & staying open on setting/theme changes (owner's request):**
     - In `Level::Settings`, pressing `←` or `→` (`PaletteKey::Edit(Edit::Left | Edit::Right)`) on any setting row (both multi-choice `ItemKind::Level(Level::Setting(s))` and boolean `ItemKind::Switch(s)`) cycles backward/forward through that setting's choices in place, immediately applying the new value (`settings::choose`) and writing back to `config.toml` while keeping `Settings ›` open and showing the live UI change behind the palette. (If the user has typed a non-empty filter query in `Settings ›`, `←` at cursor 0 still steps up only when not on a cycleable setting, or `Backspace` on empty steps up; document the exact rule and show a hint `←/→ cycle · enter list` in `Settings ›`.)
     - Only `Enter` (not `→`) opens the vertical sub-menu `Level::Setting(s)`.
     - Inside `Level::Setting(s)`, pressing `Enter` on an option applies and saves it, then calls `up(state)` to return to `Level::Settings` with that setting row still selected (instead of setting `state.palette = None`).
     - Inside `Level::Themes`, pressing `Enter` applies and saves the selected theme and updates the `current` marker on the list while keeping `Level::Themes` open (`Esc` or `←` exits).
  2. **OSC 99 (kitty) / OSC 9 / OSC 777 desktop notifications over SSH & tmux:**
     - Wire `[notifications] desktop = "auto" | "osc" | "native" | "off"` and `privacy = "none" | "sender" | "preview"` in `faro-core::config` (promote from `PLANNED` to `SETTINGS`) and add `Setting::Desktop` and `Setting::Privacy` under `SettingsGroup::Notifications`.
     - Add `Effect::DesktopNotify { title: String, body: String }`, emitted by `update/notify.rs` alongside `Effect::Bell` when a message rings (`State::rings_for`), formatted according to `privacy` (`none` → `"New message"`, `sender` → sender/chat title, `preview` → sender + `readable_body`).
     - In `tui.rs`, emit OSC 99 (`\x1b]99;i=faro:d=0;...\x1b\\`) when kitty is detected (`BlockGlyphs::Native`), OSC 9 / OSC 777 elsewhere, automatically wrapped in tmux DCS passthrough (`\x1bPtmux;\x1b...\x1b\\`) when `$TMUX` is set; for `"native"` when not `over_ssh`, fall back to `osascript` (macOS) or `notify-send` (Linux) off the UI thread.

---

### Wave 2 — Search UI, Emoji Autocomplete & Message View Polish (3 Parallel Tracks)

Start after Wave 1 is merged to `origin/main`. Run **Tracks 2A, 2B, and 2C in parallel**:

#### Track 2A: Full-Text Message & Transcript Search UI + Jump to Unread (`u`)
- **Primary files:** `crates/faro/src/app/{palette.rs,update/palette.rs,update/navigation.rs,state.rs}`, `crates/faro/src/ui/messages.rs`.
- **Tasks:**
  1. **In-chat & global message search (`/` or `Ctrl+F`):**
     - Bind `/` in `messages` and `Ctrl+F` globally (`Action::SearchMessages`, `message.search`) to open `Level::Search(Option<ConversationId>)` in the palette (scoped to the open conversation when one is open, with a toggle/prefix to search all conversations, or listed at `Level::Root` as `Search ›` with `/` prefix).
     - As the user types (debounced or on each edit via `Effect::SearchMessages`), query `Store::search_messages` (from Track 1B) and display matching messages/transcripts with sender, date, and snippet.
     - Pressing `Enter` on a search hit opens the conversation (if needed), loads `Store::messages_around` centered on the hit (`Effect::LoadAround`), selects that message in the message pane, and highlights the matched query term.
  2. **Jump to unread marker (`u`):**
     - Bind `u` in `messages` (`Action::JumpUnread`, `message.jump_unread`) and add *Jump to unread* to the palette commands: scrolls to and selects the first unread message at `OpenConversation::unread_marker` (or loads around it if scrolled/paged away).

#### Track 2B: Wire Up `[messages] density` and `timestamps` Settings
- **Primary files:** `crates/faro-core/src/config.rs`, `crates/faro/src/app/{palette.rs,update/settings.rs,state.rs}`, `crates/faro/src/ui/{messages.rs,bubbles.rs}`.
- **Tasks:**
  1. Promote `[messages] density = "comfortable" | "compact"` and `[messages] timestamps = "relative" | "absolute" | "off"` from `config::PLANNED` into active `Config` fields and `SETTINGS`.
  2. Add `Setting::Density` and `Setting::Timestamps` under `SettingsGroup::Appearance` with live preview (`Setting::previews()`) and `←`/`→` cycling (from Track 1C).
  3. In `ui/messages.rs` and `ui/bubbles.rs`:
     - `compact` omits the blank separator line between consecutive messages from the same sender within 5 minutes and tightens one-line bubble chrome where appropriate.
     - `timestamps = "relative"` formats timestamps relative to `View::now_timestamp()` (`just now`, `5m`, `2h`, `yesterday`), `"absolute"` keeps `HH:MM`, and `"off"` hides timestamps on unselected messages (still showing on the selected message).

#### Track 2C: Emoji `:shortcode:` Autocomplete & Full Reaction Search
- **Primary files:** `crates/faro/src/ui/{shortcodes.rs,popup.rs}`, `crates/faro/src/app/{input.rs,update/selection.rs,state.rs}`.
- **Tasks:**
  1. Add `crates/faro/src/ui/shortcodes.rs` with a curated, zero-dependency static table of common emoji and `:shortcode:` aliases, pre-validated against `ui/emoji.rs` (`D55` tmux width rules).
  2. **Input `:shortcode:` completion:** typing `:word` (at least 2 letters after `:`) in the input box shows a compact completion popup above the input; `Tab`/`Enter` replaces `:word` with the chosen emoji.
  3. **Searchable reaction picker:** in `Popup::ReactionPicker` (`+` on a message), pressing `/` or typing letters transitions the picker into search mode over `shortcodes.rs` so the user can react with any emoji beyond the 6 defaults.

---

### Wave 3 — Forwarding & Info Panel Links/Media (2 Parallel Tracks)

Start after Wave 2 is merged to `origin/main`. Run **Tracks 3A and 3B in parallel**:

#### Track 3A: Forward Message (`f` in Message Selection)
- **Primary files:** `crates/faro/src/app/{update/selection.rs,update/palette.rs,palette.rs,keymap/defaults.rs}`.
- **Tasks:**
  1. Bind `f` in `messages` (`Action::ForwardMessage`, `message.forward`) and add *Forward…* to the message action menu and palette.
  2. Open `Level::ForwardTo(MessageRef)` in the palette listing allowed conversations (`Chats ›` order).
  3. Choosing a target conversation switches to it and sends a copy of the source message's readable body and any downloaded local attachments via `sending::send` (respecting the allowlist guard, D62).

#### Track 3B: Shared Links & Media in the Right Info Panel (`Alt+I`) & Palette
- **Primary files:** `crates/faro/src/ui/info.rs`, `crates/faro/src/app/{palette.rs,update/palette.rs}`.
- **Tasks:**
  1. Extend `crates/faro/src/ui/info.rs` (the right-hand info sidebar toggled by `Alt+I`) to display two new sections below participants/details for the open conversation: **Recent files & media** (downloaded images, voice notes, documents) and **Recent links** (extracted URLs from loaded messages).
  2. Add *Links & files in this chat…* (`Level::ChatLinks`) to the palette's open-chat commands so every shared URL and downloaded attachment in the conversation can be fuzzy-filtered and opened/copied/viewed with one keystroke.

**Acceptance (M12)**
- All 11 features pass unit, reducer, and snapshot tests, `tools/frames` replay stays at 0 divergent frames, and `docs/MANUAL_TESTS.md` includes an `## M12` hands-on section.

## M13: website, getfaro.dev (D105)

Design, copy, server runbook and full task lists: [WEBSITE.md](WEBSITE.md) (§7 is the plan, §8 the decisions). In short:

1. **Phase 0: spikes and server setup.**
   - Build the mock backend for `wasm32`.
   - Prototype both renderers (custom Canvas 2D and Ratzilla WebGL2) and keep one.
   - Choose the webfont.
   - Set up DNS, nginx and TLS, and self-hosted Plausible, then deploy a placeholder page.
2. **Phase 1: static landing page.**
   - The page, copy, privacy and terms pages, and analytics.
   - The site generator, plus a poster, phone frames and OG image rendered from the real app.
   - A CI site job, and deploying to the server. Shippable on its own.
3. **Phase 2: faro builds for the web.**
   - Target-neutral seams (`term_event`, `clock`, runtime shim).
   - The `faro-app` / `faro` / `faro-web` split with an architecture test.
   - `store/mem.rs` with the store tests run on both backends.
   - A CI wasm build.
4. **Phase 3: the web shell and renderer.**
   - The event loop, renderer and web router (clipboard, links, Web Audio).
   - Web keybindings, the web emoji mode and half-block images.
5. **Phase 4: the demo on the page.**
   - Preload and autostart, with the focus model (Esc or a click leaves the panel).
   - Theme chips, restart, fullscreen.
   - Demo analytics events, the phone fallback, headless browser tests, MANUAL_TESTS entries.
6. **Phase 5: launch polish.**
   - Check every install command and statement on the page.
   - 404, sitemap and security headers.
   - The AGPL source link for the deployed build.
   - Update the README.

**Acceptance (M13)**
- getfaro.dev loads in ≤ 200 KB, reads without JavaScript and works on phones.
- On a laptop, the real demo runs in the panel without a click, and can be used and left with the keyboard in current Chrome, Firefox and Safari.
- Native faro behaves exactly as before.

## Later (backlog: do not lose)

- Investigate search-jump effect routing: `on_messages_around_loaded` discards `open_conversation`'s effects when switching chats, including `StopAudio` and draft persistence. Pre-existing (confirmed in HEAD during D125 review); reproduce with a playing voice note and a cross-chat search result before fixing it.
- `faro daemon` + attachable TUI
- signal-cli JSON-RPC backend
- Encryption at rest
- ~~About screen with the logo~~: done as a popup from the palette (D94); the animated lighthouse is the empty message pane (D61)
- Drag-resizable sidebars with hover highlight
- Optional sync of hidden/allowlist state via config
- Recording and sending voice notes
- Terminal hyperlinks (OSC 8) for links in messages, so a link split over lines still opens whole (kitty; tmux 3.4+ with the `hyperlinks` terminal feature). ratatui has no support, so it goes through `tui/backend.rs`; needs replays and hands-on checks (D102).
- Packaging (cargo-dist binaries, Homebrew tap, AUR `faro-bin`/`faro-git`, `.deb`, Nix flake)
- **Guided tour inside the live demo** (idea, 2026-10-17; owner: document for later). The README demo's storyboard (`tools/demo_gif/storyboard.jsonc`, D142) drives the real wasm app on getfaro.dev while the visitor is idle, so they see the interactions without exploring, with crisp text, no video bytes, and always in step with the UI. It hands over on the visitor's first click or key and doesn't come back. Open questions: it touches the focus and takeover rules (D125: the demo starts without the keys, and synthetic keys must not claim focus, mark chats read or send receipts), so a tour would need its own input path into the reducer rather than DOM key events; whether it runs on its own or behind a "▶ Take the tour" button; reduced motion; whether it uses eager answers (D142) when the visitor's own sends don't; analytics. Until then the site links the recorded tour video (D143).
- Paste images from the local clipboard over SSH (kitty OSC 5522, tmux passthrough, not mosh; D89). Tested by hand on the owner's setup; phased plan in [research/clipboard-image-paste.md](research/clipboard-image-paste.md). Phase 0 (input-handling spike) done: pause crossterm and read the tty directly (D90). Phase 1 done: `faro-core::clipboard` (requests, `Reader`). Phase 2 done: `Ctrl+V` stages the image, `Ctrl+O` shows it (D91; owner's hands-on checks in MANUAL_TESTS "Clipboard images"). Phase 3 (kitty paste events, plain `Cmd+V`) later, starting with a hand test.

---

## Progress

- 2026-10-19: **Issue forms** (D146): bug report and feature request forms, contact links for security reports and Signal support.
- 2026-10-19: **v0.1.2 released** (tag at `485373b`): transcription settings (D145). GitHub release with 4 archives and 2 debs, Homebrew tap updated; AUR PKGBUILDs bumped in the repo for the owner to push. Release notes added by the owner (the agent's token can't edit releases).
- 2026-10-19: **Transcription settings** (D145): a *Voice notes* section in the palette with mode-dependent rows (model, command, endpoint, remote model, language: presets plus *Custom…*), local model `small`, separate `transcribe_remote_model`, a background check for the local command with install hints, `auto_transcribe` (off by default) with the length limit as its sub-option, timeouts that grow with the note, and transcripts drawn as one paragraph `❝text❞ [local]`.
- 2026-10-19: **Security policy** (D144): `SECURITY.md` with GitHub private vulnerability reporting (the owner enables the repository setting), linked from the README and CONTRIBUTING. Also: the config watcher test no longer depends on runner speed (`ea01113`).
- 2026-10-17: **Tour video on getfaro.dev** (D143): *▶ Watch a 30-second tour* under the panel and the phone gallery opens `media/faro-tour.<hash>.mp4` (the recorder's MP4, copied by `make.sh --install`) in a `<dialog>`; nothing loads before the click (`site/check.py` `check_tour`). nginx caches hashed `.mp4` (deploy the updated `getfaro.dev.conf`). The README GIF links to getfaro.dev. The guided tour inside the live demo is documented in the backlog.
- 2026-10-17: **README demo GIF** (D142): `tools/demo_gif/` records the web demo from a storyboard on CDP virtual time (`make.sh`: build, record, encode GIF/MP4/WebM); `docs/images/faro-demo.gif` replaces the poster PNG in the README. `MockConfig::eager_answers` (wasm `start_recording`, test page `?record`) makes the "Jim answers" beat certain; visitors are unchanged.
- 2026-10-13: **The demo starts mostly read** (D134): only Scranton Branch (3) and Jim (2) start with unread messages; the rest of the backlog arrives read on the phone, so live messages are what's new. Website: the footer and legal pages share the text column, the notes under the demo are trimmed to one line and follow the demo's width.
- 2026-10-13: **v0.1.0 released** (M8 task 8, D133): repo public, tag at `eff73ec` (first run failed before publishing and was re-tagged), GitHub release with 4 archives and 2 debs, Homebrew formula in `harababurel/tap`, AUR `faro`/`faro-bin`/`faro-git` live. Terminal matrix signed off by the owner (task 1). Legal pages and FAQ filled in. CI green on GitHub, including the site job's browser suite. The owner installed it with Homebrew (Mac mini) and `faro-bin` from the AUR (Arch server), and checked `y` copy over mosh. Owner's: `.deb` and build-from-source installs, screenshots, deployment.
- 2026-10-14: **v0.1.1 released**: inline images (D136 forced protocols measure the cell size; D137 the client in the way is named, and faro asks the terminal again when graphics become possible or the tmux clients change). GitHub release with 4 archives and 2 debs, Homebrew tap updated; AUR PKGBUILDs bumped in the repo for the owner to push.
- 2026-10-13: **M13 4.6 complete, Phase 4 code complete** (D130): real-wasm CDP browser suite wired into the site job with required Chrome/Node 22, loader/unit checks and failure artifacts. Local runs pass 101 assertions/22 scenarios (~42 s), 54 loader checks, five unit tests and 700 native tests/full quality gate; site/no-JS budgets pass. Negative prerequisite/timeout runs fail with reports and cleanup. Actual GitHub execution remains unobserved, not claimed green. Manual/device checks stay deferred. Next: Phase 5 launch polish; no deployment/publication by agents.
- 2026-10-13: **M13 Phase 5 mechanical polish** (D131): 404 page + nginx `error_page`, `robots.txt`, `sitemap.xml`, canonical/twitter tags, footer link to the build's source commit, README points at getfaro.dev. Full gate, site checks and the browser suite (101/22) pass locally. Owner's: clean-machine install check, screenshots, legal placeholders, FAQ approval, deployment.
- 2026-10-13: **M8 release pipeline prepared** (D133): vendored OpenSSL for Linux binaries, dist (4 targets, Homebrew formula), `.deb`s via a custom publish job, `faro`/`faro-bin` PKGBUILDs. Verified locally (x86_64 archive, debs installed in debian:12/ubuntu:22.04 containers, faro-bin makepkg); the workflow itself is unproven until a real tag runs.
- 2026-10-13: **M13 4.5 complete** (4.5b, D129): hashed, self-contained phone SVGs supply a no-JS swipe/keyboard gallery with readable-width floor and labels below the frames. Coarse/narrow devices initially fetch no demo; resize frees/restarts sessions with cached wasm, retained choices and stale-start fences. 700 native tests/full quality gate, 54 Node checks and Chromium mobile/desktop/resizing checks pass; full site/no-JS checks pass (40,692/40,960 B page, 161,570/204,800 B conservative first load). Review findings fixed, including complex graphemes, scheme CSS and retry/timeout recovery. Real-phone emoji and narrow fullscreen checks remain deferred; headless lacks emoji fonts and fullscreen permission. Next: 4.6 CI browser tests. No deployment by agents.
- 2026-10-13: **M13 4.5a complete** (D128): faro-site emits six deterministic 44×24 SVGs (conversation, root palette and Themes, dark/light) from the real app state/reducer/exporter. Theme preview/current markers are scheme-correct; generation restores caller theme/palette. 687 native tests and full quality gate pass; local raster composition previews checked; full site/no-JS checks pass, unchanged 40,448/40,960 B page budget. Assets remain unreferenced; **4.5 is not complete**. Next: 4.5b responsive gallery/mobile poster, asset hashing and font integration. Owner defers deployment.
- 2026-10-13: **M13 4.4 complete** (D127): page-only Plausible demo start/focus/accepted-send/committed-theme events, explicit opt-out and nonfatal tracker failures, detail-less `faro-sent`. 684 native tests and native/wasm lints pass; 29 Node regression checks and real-wasm Chromium checks verify counts, preview/scheme/restoration silence, manual starts, restart deduplication and no typed content in payloads. Site/no-JS checks pass (40,448/40,960 B page, 1,021,363 B gzipped wasm). Dashboard/real-browser owner checks remain deferred. Next: 4.5 phones; no deployment by agents.
- 2026-10-13: owner deployed `13c3c7d` (4.3b) and authorised Phase 4.4 analytics with local Qwen implementation/review. Manual checks remain deferred; deployment remains the owner's.
- 2026-10-13: **M13 4.3b complete** (D126): seven theme chips/All palette, motion preference/override, same-seed restart retaining choices, fullscreen/optional non-Escape Keyboard Lock and font refit. Native keyboard cleanup and failed-initialisation retry corrected after browser checks. 683 tests, wasm/native lints, unchanged native snapshots, integrated Chromium dpr1/2 checks, audio cleanup and no-refetch checks pass. Final site: 40,399/40,960 B HTML/CSS/JS, no-JS passed. Manual checks remain deferred; deployment is the owner's. Next: 4.4 analytics.
- 2026-10-13: **M13 4.3b plumbing checkpoint** (D126): reducer-backed panel APIs, settings events, explicit idle-loop wake, validated font refit and audio cleanup; seven generated theme chips and control markup/styles. Native/wasm gates and unchanged native snapshots checked. Page JS wiring/fullscreen and integrated browser verification remain in progress; manual checks are deferred. This checkpoint intentionally keeps the previous loader, so new controls remain hidden until the wiring lands.
- 2026-10-13: owner deployed `778dcaf` to production and reports it seems fine. Website manual checks remain unrun and are explicitly deferred; this is not acceptance of those checks. M13 4.3b (panel controls) authorised to proceed autonomously with local Qwen implementation/review; deployment remains the owner's.
- 2026-10-13: **M13 Phase 4.3a done** (D125): focus hints and amber panel border, click-to-enter, outside-click and end-of-Escape-chain exit, window/visibility focus suspension, and unfocused input gating. Startup opens its chat without synthetic keypresses or clearing unread messages; a resize cannot reclaim focus. Quality gate green (670 tests, one ignored spike), wasm lints/build, site/no-JS checks and headless Chromium smoke checks; native snapshots unchanged. Owner checks: MANUAL_TESTS "Website: focus and Escape". Next: 4.3b panel controls; deployment remains the owner's manual step.
- 2026-10-12: **M13 Phase 3 code done** (D113–D119; commits `e2fcd43`…`6b2ce31`). The owner's browser checks are pending (MANUAL_TESTS "Web demo").
  - **What runs:** `crates/faro-web` runs the full demo in a `<canvas>`: the mock, the in-memory store, the shared reducer and renderer, the Canvas 2D painter, DOM input, a web router (clipboard, links, bell, Web Audio, notices for files), an embedded demo config, canned transcripts, half-block images, exact emoji, and quit-to-restart.
  - **Shared with the terminal app:** store effects and messaging commands now run through faro-app (D113).
  - **Size:** the wasm is 1.02 MB gzipped, under the 1.5 MB budget; CI checks it.
  - **Frame time:** 2.1 ms for a full frame on an Apple M1. Under software rendering, p95 reaches 8.3 ms (D119).
  - **Next:** the owner's checks close Phase 3. Phase 1 can start now (it doesn't depend on Phase 3); Phase 4 needs both.
- 2026-10-11: **M13 Phase 2 done** (D110–D112), ahead of Phase 1 (it needs neither Phase 1 nor the server). faro-core has the `rt` shim and a `sqlite` feature, with an in-memory store engine; every engine-agnostic store test passes on SQLite and in memory. The shared app is `crates/faro-app` (state, reducer, key map, palette, themes, rendering), with faro's own terminal event types, an `ImageCache` trait and an architecture test; `crates/faro` is the terminal binary on top of it. `crates/faro-web` is a stub that draws a real frame from the mock. CI builds and lints faro-core, faro-app and faro-web for wasm32. Native faro is unchanged: every snapshot passes with its old contents (renamed `faro_app__…`). Next: Phase 1 (waits on the owner's operator details for the legal pages and the open FAQ answers) or Phase 3.
- 2026-10-11: M13 Phase 0, agent's part done: `MockBackend` and `Sim` build and run on wasm32 behind a throwaway `wasm-spike` feature, with the same events as natively (D108). Canvas 2D renderer kept over Ratzilla (D106), JetBrains Mono chosen (D107). Server files and runbook in `site/deploy/` (D109).
- 2026-10-11: M13 Phase 0 server setup done by the owner, following `site/deploy/README.md`: DNS, Plausible CE (2FA on, registration closed, goals added), nginx and TLS for both hosts, log rotation, first deploy of the placeholder. End-to-end check passed (redirects, security headers, `/js/s.js`, `/api/event`, visits with the right countries in the dashboard). Phase 0 is complete; Phase 1 (static page) is next.
- 2026-10-11: website planned (M13, D105): design, copy, privacy/terms drafts, server runbook and phased plan in [WEBSITE.md](WEBSITE.md). Nothing built yet.
- 2026-10-11: **M7 accepted.** The owner ran the M7 checks on the test account (smithers): guard, linking, `faro ids`, messages/replies/reactions/edits/deletions both ways, typing, receipts, read syncs both ways, mentions, names, nicknames, catch-up and history all pass; the personal account on the Mac mini (`--no-allowlist`, contacts) has been in daily use without problems. Deferred: Wi-Fi off (on the Mac mini). Remaining clipboard checks pass too (late answer, too large, over mosh); two kitty windows deferred. Next: M8 (D96 sidebar markers, D95 optional guard, terminal matrix, release).
- 2026-10-10: owner checked clipboard paste by hand (9 of 13 items in MANUAL_TESTS "Clipboard images" pass; too large, two kitty windows, late answer and over mosh left) and About (D94, now with full links and the config file). Terminal.app → SSH showed emoji misalignment, noted for the M8 matrix.
- 2026-10-09: clipboard image paste done (D89–D91, D93; owner's hands-on checks in MANUAL_TESTS "Clipboard images"). Phase 0 spike (branch `spike/clipboard-osc5522`) settled how to read terminal replies (D90). `faro-core::clipboard` is the protocol. `Ctrl+V` stages the image (never sends it) and `Ctrl+O` shows it, asking every kitty attached to tmux. Pasted files are cleaned up. Also: a startup race that could wipe early messages from the list (D92), and an About popup (D94). End-to-end check: `tools/clipboard/e2e.py`.
- 2026-10-09: D84 image settings complete: explicit Kitty/Sixel/iTerm2 protocols, Auto result/fallback notes in the palette, live config updates and protocol-cache invalidation tests. Quality gate green (fmt, clippy, 460 tests); snapshot reviewed. Hands-on checks: MANUAL_TESTS "Explicit protocols (D84)".
- 2026-10-09: demo refinements done (owner's hands-on checks pending, see MANUAL_TESTS "Demo"): `faro demo` replaces `--mock` and can't reach real data (D87); the demo cast is an Office parody played as written scenes (D88).

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
| M7 presage backend (test account needed) | ✅ Done (2026-10-11; owner's hands-on checks pass, Wi-Fi-off check deferred) | Safeguards first: allowlist guard (D62). `faro-presage` (D63): `faro link` (terminal QR), `faro ids` (IDs for the allowlist), the backend behind a mandatory `--allowlist`, mapping of messages, receipts, typing, reactions, edits, deletions and sync transcripts (tested), reconnect with backoff. Next: link the test account, then the acceptance checks with the owner. Read syncs from the phone clear unread counts (D70); reads in faro are synced to the phone (D71). Mentions are drawn as `@Name` and ring "mentions only" chats (D72). Taking a reaction back names the emoji, so phones remove it (D73). Attachments and stickers are in M9 (D75–D79). Not yet: delivery receipts (moved to M12 Track 1B), history transfer (M11). |
| M8 v0.1 hardening + release | ✅ v0.1.0 released (2026-10-13; Homebrew, AUR and `.deb` installs checked by the owner, from-source pending) | Released 2026-10-13 (D133): GitHub release (4 archives, 2 debs), Homebrew tap, AUR `faro`/`faro-bin`/`faro-git`; terminal matrix signed off by the owner the same day. Task 3 done: themes `faro-dusk`, `faro-light`, `system` (terminal's 16 colours, reverse-video selection, dim muted text) beside `faro-dark`; colour depth detected (`COLORTERM`, tmux, known terminals) with `[ui] colors` override; 256-colour fallback mapped once at load, surfaces kept apart; 16 colours → `system`; `:` palette theme/colour settings with write-back; contrast tests (D51). Task 4 done: README rewritten (pre-release status, install from source incl. the future `protoc` packages, trying the mock, flags, keys, config, recommended tmux settings, mosh, colours) with a text capture of the UI; real screenshots left to the owner (TODO comment in the README). Task 1 prepared: [MANUAL_TESTS.md](MANUAL_TESTS.md) "M8: terminal matrix" (Linux/macOS × direct, tmux, SSH, mosh × rendering, emoji, colours, mouse, OSC 52, bell, title, focus, paste, keys, Ctrl+L, restore); the owner runs it after M7. Matrix and release both done 2026-10-13 (see above). Owner-reported glitches (2026-10-08, D55): tmux joined code points onto the wrong cell (flags, VS16 emoji, skin tones on bases it doesn't know); fixed in `tui/backend.rs` and `ui/emoji.rs`, found with the frame log (`FARO_DEBUG_FRAMES`) and `tools/frames/`; 20,122 replayed frames, 0 divergent, direct and nested (was 1176 of 2900). Bubble styles (owner's choice, D58, superseding D56's half-block corners): rounded (Powerline caps on one-line bubbles, two-thirds Legacy Computing edges on taller ones) or a bar on the outer side in the sender's colour; `[messages] bubble_style = "auto"` draws rounded only where every terminal is kitty (it draws those glyphs itself), detected with the D47 terminal check; the `system` theme always draws bars. Logo screen (owner's choice, D61): the animated lighthouse from `tools/logos.py` ported to `ui/logo.rs` fills the message pane while no chat is open, in four sizes, with key hints; 10 fps only while visible, focused and `reduce_motion` off (now a palette switch). |
| M9 Attachments + inline images (v0.2) | ✅ Done (2026-10-08; owner's hands-on checks pending) | D75–D79. Task 1: `AttachmentStatus` (`Remote`/`Downloaded`/`Failed`), `width`/`height` (migration `0006_attachments.sql`), `Backend::fetch_attachment` returning bytes, `p1:` hex-encoded `AttachmentPointer` in `AttachmentId` (refreshed on replay), atomic `0600` writes into `<data dir>/attachments/`, background downloads (`Command::Fetch` → `AppEvent::Fetched`, max 3 concurrent, auto-download images ≤ 25 MB in the open conversation, `[media] auto_download`), mock PNG/PDF/sticker attachments. Task 2: `d` download, `v` view/open, `s` save to Downloads (`name (N).ext` deduplication), copy path (OSC 52), per-attachment action menu items, `Open` omitted over SSH (`State::env.over_ssh`). Task 3: `Send a file…` in the palette (`Level::AttachFile`, non-blocking directory listings, `~` expansion) and pasted file paths (`Effect::CheckPaste`), staged attachment bar above the input (`Esc` removes it first), caption-optional sending, outgoing files copied into `attachments/` (`0600`) before `Backend::send` (`upload_attachment` in presage, behind the allowlist guard). Task 4: `ratatui-image` 11.1 (`SlicedProtocol`) + `image` 0.25, startup protocol query (`Picker::from_query_stdio`), kitty/sixel/iTerm2 with tmux `allow-passthrough` or half-block fallback (mosh, mixed tmux clients, 256 colours mapped to xterm palette, none at 16 colours), `[media] inline_images`, `app/images.rs` LRU cache (32 entries) with off-thread decoding, `Popup::ImageViewer`, stickers (`[sticker]` + `image/webp`). Task 5: graphics protocols suppressed while overlays are open, `emoji::finish` skips `cell.skip` cells, `images` scenario in `tools/frames/record.py` (343 frames, 0 divergent direct and nested). |
| M10 Voice notes + transcription (v0.2) | ✅ Done (2026-10-08; owner's hands-on checks pending) | D80–D82. Task 1: `AttachmentRef::{voice_note, duration_secs, waveform, transcript, transcript_source}` + `RemoteTranscription` (migration `0007_voice_notes.sql`), presage `VoiceMessage` flag mapping (`flags & 1 != 0`), `faro-core::audio` pure-Rust RIFF/WAVE PCM/float duration & 16-bar RMS waveform (`▁▂▃▄▅▆▇█`) with `ffmpeg`/`ffprobe` fallback, mock synthesized 8 kHz WAV voice notes, `▶ duration waveform · size` row in both layouts. Task 2: external audio player (`app/player.rs`, `[voice] player`, `x`/`v` play/stop, `■ elapsed / total` progress with `accent` played waveform bars, 1 s `PlaybackTick`, SSH/mosh guard). Task 3: opt-in Whisper transcription (`[voice] transcribe = "off"\|"local"\|"remote"`, `t` in messages, menu/palette *Transcribe*, auto-transcribe up to `transcribe_max_secs`), local CLI (`whisper-cli`/`whisper-cpp`/`whisper` or `{file}` wrapper) and remote OpenAI-compatible `/audio/transcriptions` via `curl --config -` (API key from env or OS keychain, rejected from `config.toml`), `Popup::ConfirmRemoteTranscription` (`voice.remote_consent` in `app_state`) and per-conversation `RemoteTranscription::Allowed`, persisted `❝ <text> [local\|remote]` under the voice note and in `readable_body`. |
| M11 History transfer on link (v0.2) | 📋 Proposed (D74) | Outline in this file; needs presage to expose the link-time key (PR #430). |
| M12 UX, messaging, search & polish (v0.2) | ✅ Done (2026-10-09; owner's hands-on checks pending) | D83. Executed in 3 sequential waves of parallel subagent tracks: **Wave 1** (`1A`: edit own messages `e`, delete for everyone `D` with `Popup::ConfirmDelete`, send `@mentions` with inline `@` autocomplete and `body_ranges`; `1B`: per-recipient delivery/read receipts `0008_receipts.sql`, `DeliveryStatus::Read` `✓✓` in `accent`, `Popup::MessageInfo` `i`, `Store::search_messages` & `messages_around`; `1C`: `Ctrl+P` `←`/`→` in-place setting value cycling & staying inside `Ctrl+P` on setting/theme selection + OSC 99/777 desktop notifications with privacy levels and tmux passthrough), **Wave 2** (`2A`: full-text message & transcript search `/` & `Ctrl+F` with context loading + `u` jump to unread marker; `2B`: `[messages] density` & `timestamps` settings with live preview; `2C`: `:shortcode:` emoji autocomplete + searchable reaction picker `ui/shortcodes.rs`), **Wave 3** (`3A`: forward message `f` via `Level::ForwardTo`; `3B`: shared links & downloaded media in `Alt+I` info panel + `Level::ChatLinks`). |
| M13 Website: getfaro.dev | 🚧 Phases 0–4 code complete (D105–D130); Phase 5 mechanical polish done (D131: 404, robots, sitemap, canonical, source-commit link, README link). Browser suite green in CI. Owner deployed `13c3c7d`; later deployment/manual checks deferred. Legal pages and FAQ filled in. Left for the owner: the build-from-source install check, screenshots, deploying the commits after `13c3c7d`. | Static page plus `faro demo` in WebAssembly, nginx and self-hosted Plausible. [WEBSITE.md](WEBSITE.md). |
