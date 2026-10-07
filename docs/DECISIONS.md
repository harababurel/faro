# Decision log

Every significant decision, with the reasoning and the alternatives that were rejected. Newest decisions go at the bottom. If a decision is reversed, don't delete it: add a new entry that supersedes it and mark the old one **Superseded by Dxx**.

Status values: **Accepted** (agreed with the project owner), **Judgement** (made by the agent under the owner's "use your best judgement" instruction; fine to revisit with good reason), **Superseded**.

All entries below were made on 2026-10-07 unless stated otherwise.

---

### D1. Build a terminal UI (TUI), not a web app — Accepted

- **Context:** The owner uses Signal over SSH: a personal Mac mini runs a client inside tmux, attached to from other machines.
- **Decision:** faro is a TUI that runs over SSH, inside tmux.
- **Rejected:** web app (doesn't fit an SSH/tmux workflow, and it would expose a Signal session over the web).

### D2. Write our own client rather than keep using gurk — Accepted

- **Context:** gurk works but has: opaque keybindings (F1 help is an unformatted wall of text), a conversation list that fills with every contact ever seen and can't be pruned, input freezing while incoming messages are fetched, and no in-app settings. See [BACKGROUND.md](BACKGROUND.md).
- **Decision:** Build faro, focused on UI/UX. Implementation is done by coding agents, so engineering effort is not the constraint; quality of the result is.
- **Rejected:** contributing to gurk (the problems are in the overall UX design, not single bugs); scli, signal-weechat, Matrix bridge + iamb/gomuks (see BACKGROUND.md).

### D3. Language: Rust — Accepted

- **Decision:** Rust, with ratatui for the TUI.
- **Why:** best TUI ecosystem for the look we want (btop-like visuals); access to presage/libsignal in-process; coding agents are strong in Rust; gurk (Rust + ratatui) is open-source reference code.
- **Rejected:**
  - Go + Bubble Tea: good option, but the owner prefers Rust.
  - Haskell + brick: owner has some Haskell experience and was open to a functional language if it fits. brick is mature and Haskell's concurrency (STM, async) fits well, but: no path to presage/libsignal (forces signal-cli forever), fewer visual widgets than ratatui, and agents are noticeably weaker in Haskell.
  - TypeScript + OpenTUI (what OpenCode uses): younger, less proven.

### D4. Never reimplement the Signal protocol or service layer — Accepted

- **Context:** libsignal (official) covers crypto only. The service layer (linking, websocket, envelopes, groups, attachments, sync) has no public spec; the only reference is the official apps' source, which changes often.
- **Decision:** Use an existing implementation of the service layer.
- **Why:** correctness can't be verified without risking a real account; bugs are silent and costly (lost messages, broken sessions, unlinked device); permanent maintenance burden; no user-visible benefit since all of faro's goals are UI-level.

### D5. Signal backend: presage first — Accepted

- **Context:** Two realistic options: **presage** (Rust library on official libsignal + libsignal-service-rs; used by gurk and Flare) and **signal-cli** (Java; most mature unofficial client; JSON-RPC daemon mode).
- **Decision:** presage, in-process.
- **Why:** faro will be open source and must install easily on many systems. presage gives a single binary with no Java; signal-cli needs a Java runtime newer than some stable distros ship and is packaged inconsistently across distros.
- **Accepted trade-offs:** when Signal changes server behaviour, fixes take longer (presage → faro release → packagers); presage's API changes between versions; presage can't be published on crates.io (git dependencies) so faro can't either; build needs `protoc`.
- **Initially chosen then reversed:** signal-cli daemon was the first recommendation (most reliable), reversed once multi-distro packaging was considered.

### D6. Backend behind a trait; signal-cli backend possible later — Accepted

- **Decision:** All Signal access goes through a `Backend` trait with faro-owned domain types (see DESIGN.md §5). Only the presage implementation is built at first.
- **Why:** keeps UI independent of presage; allows a signal-cli fallback if presage lags behind Signal; enables a mock backend for development and tests.
- **Caveat recorded:** switching backends requires re-linking the device (each backend has its own key store). History survives because faro owns it (D7).
- **Guard against leaky abstraction:** domain types are modelled on Signal concepts, not presage structs.

### D7. faro owns message history in its own SQLite database — Accepted

- **Decision:** faro stores history and UI state (hidden/pinned/muted, drafts, sidebar widths) in its own DB. presage keeps its own store for keys and sessions; faro doesn't touch it.
- **Why:** backend-agnostic; survives backend switches; required for a future signal-cli backend (which only emits live events).

### D8. Licence: AGPL-3.0 — Accepted

- **Why:** presage and libsignal are AGPL-3.0, so faro must be too.

### D9. Config format: TOML, edited with `toml_edit` — Accepted

- **Decision:** single TOML config file; first run writes every option commented out with its default; settings changed in the UI are written back with `toml_edit` so comments and layout are preserved; file is hot-reloaded.
- **Split:** preferences in TOML; per-conversation state in SQLite.

### D10. Themes: OpenCode-compatible JSON format — Accepted

- **Why:** OpenCode's theme library can be reused; the owner uses and likes OpenCode. Plus a `system` theme (terminal palette) and 256-colour fallback.
- **Priority note (owner):** themes are important but not top priority.

### D11. Name: faro — Accepted

- **Meaning:** Italian/Spanish/Portuguese for "lighthouse" (descends from Pharos, the Lighthouse of Alexandria). Pronounced FAH-ro.
- **Availability (checked 2026-10-07):** free on Homebrew (formula + cask), Arch, AUR, Debian, Nixpkgs, Flathub, Docker Hub. Taken on crates.io (a tiny web-search CLI that also installs a `faro` binary), PyPI (small inactive library), npm (tiny 2022 package).
- **Known neighbours:** Grafana Faro (frontend observability, ~1.3M weekly npm downloads; dominates "faro" in developer search results); FARO Technologies (3D measurement; holds the FARO trademark in a different market; acquired by AMETEK in 2025 as far as we know; no registry search was done); Faro, the city in Portugal.
- **Plan:** command is `faro`. If needed when going public, repo/packages can use `faro-chat`. Not a trademark/legal assessment.
- **Rejected names:** see BACKGROUND.md (naming history).

### D12. Logo direction: lighthouse with a rotating beam — Accepted

- Prototype: `python3 tools/logos.py browse lighthouse` (it's the "pharos" lighthouse art; "faro" uses the same icon). Final art to be redone in Rust (splash/about screen), with a `reduce_motion` option.
- The owner also liked the Dune-style signet ring variants; recorded for reference only.

### D13. Default keybindings as proposed in DESIGN.md §7.6 — Accepted

> Amended by D36: `Alt+J/K` switch conversations (arrows kept as a second binding); `Alt+↑` no longer enters message selection.

- Avoid tmux's default prefix `Ctrl+B`. All bindings rebindable via config.

### D14. Release split v0.1 / v0.2 as in DESIGN.md §14 — Accepted

- v0.1 = usable daily driver; drag-resize, right-click menus, settings UI, OSC notifications, attachments, theme picker in v0.2. Inline images in "Later" (must not be forgotten).

### D15. Develop against a mock backend first; real test account only at the presage milestone — Accepted

- **Decision:** Milestones M0–M6 in [PLAN.md](PLAN.md) use only the mock backend. A **separate Signal test account** (second phone number) is needed from **M7 (presage backend)** onward. Never develop against the owner's main account.
- **Action for agents:** tell the owner before starting M7 so they can get the test number ready.

### D16. Storage library: sqlx (SQLite) — Judgement

- **Why:** async fits tokio; presage's SQLite store and gurk both use sqlx, which avoids two SQLite stacks in one binary. Use the runtime query API or offline-checked macros (`.sqlx` directory committed) so builds never need a live `DATABASE_URL`.
- **Check during M2:** align the sqlx version with the one presage's store uses (avoid duplicate versions). If that proves painful, `rusqlite` is the fallback.
- **Rejected:** rusqlite + refinery (sync API; would need `spawn_blocking`), diesel (heavy).

### D17. Paths: XDG-style on both Linux and macOS — Judgement

- Config: `$XDG_CONFIG_HOME/faro/config.toml`, default `~/.config/faro/config.toml` (also on macOS, not `~/Library/Application Support`; terminal users expect `~/.config`).
- Data: `$XDG_DATA_HOME/faro/`, default `~/.local/share/faro/`. Overrides: `--config`/`FARO_CONFIG`, `--data-dir`/`FARO_DATA_DIR`.
- Use the `etcetera` crate (XDG strategy on all Unix).

### D18. Encryption at rest: deferred — Judgement

- v0.1 relies on file permissions (`0600` files, `0700` dirs). Design the store-opening code so a key can be passed later. Revisit after v0.1 (options: presage store passphrase, SQLCipher, OS keychain / Secret Service).

### D19. Message history transfer from phone: assume none for v0.1 — Judgement

- Linked devices start with empty history (as gurk does). Investigate presage support during M7; if supported, add later.

### D20. Cargo workspace with three crates — Judgement

- `faro` (binary + TUI), `faro-core` (domain, Backend trait, mock backend, store, config, themes), `faro-presage` (presage backend; created at M7 so early builds stay fast and don't need protoc).

### D21. Toolchain: stable Rust, edition 2024, pinned via `rust-toolchain.toml` — Judgement

- Track recent stable (presage tends to need a recent compiler). CI runs `fmt`, `clippy -D warnings`, and tests on Linux and macOS.

### D22. Architecture: single process, reducer + effects, tasks over channels — Judgement

- App core is a pure-ish reducer `update(state, event) -> effects`; network/disk work happens in tokio tasks. This is the mechanism behind "never freeze the UI". Details in DESIGN.md §4.

### D23. Defer to the owner on git operations — Judgement (superseded by D33)

- Commit/push when the owner asks. Keep commits small with short imperative messages (existing style: "Add design doc and naming-phase logo prototypes").

### D24. Licence identifier: `AGPL-3.0-only` — Judgement

- Cargo metadata uses SPDX `AGPL-3.0-only`, matching libsignal. Switching to `AGPL-3.0-or-later` is the owner's call and only touches `Cargo.toml` (and the README note) while there are no outside contributors.

### D25. Build settings from M0 — Judgement

- Toolchain pinned to an exact minor (`1.99` in `rust-toolchain.toml`); bump deliberately in its own commit. `rust-version` in `Cargo.toml` matches.
- `unsafe_code = "forbid"` applies to the whole workspace, not only faro-core (nothing needs `unsafe`; a crate can opt out of workspace lints if that changes).
- Clippy `pedantic` is on as warnings (errors in CI), minus `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`.
- Pre-release version string is `0.1.0-dev` until v0.1.0 is tagged.
- faro is Unix-only for now (paths code uses Unix permissions), consistent with Windows being a non-goal.

### D26. Logging: one file in the data dir, size-capped — Judgement

- `<data dir>/logs/faro.log`, `0600`, written through a non-blocking writer so logging never stalls the UI. Filter via `FARO_LOG` (`RUST_LOG` syntax), default `info`.
- At startup, a log over 10 MB is moved to `faro.log.old`. Simpler than date-based rotation; revisit if the daemon mode makes long-running logs common.

### D27. Enforce correctness and efficiency rules mechanically, not by convention — Accepted (2026-10-07)

- **Decision:** the owner likes the workspace-wide `unsafe_code = "forbid"` rule and wants many more constraints like it: rules that tools check, so mistakes fail the build or CI instead of relying on review. Agents should add such constraints as the code grows, each one recorded here or in a follow-up entry.
- **How to add one:** prefer the cheapest mechanism that works: (1) rustc/clippy lints in `[workspace.lints]`; (2) `clippy.toml` `disallowed-methods` / `disallowed-types` / `disallowed-macros`, which can ban specific APIs; (3) CI tools (`cargo-deny`, etc.); (4) architecture tests in `cargo test`; (5) custom lints (`dylint`) only when nothing simpler can express the rule.
- **Candidates** (evaluate when the relevant code exists; tick off in PLAN.md "Mechanical constraints"):
  - No panics in non-test code: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `dbg_macro`; `indexing_slicing` in core logic.
  - Nothing writes to the terminal behind the TUI's back: `print_stdout`, `print_stderr` (with a narrow allowance for pre-TUI CLI output).
  - Never block the UI (D22): `disallowed-methods` for `std::thread::sleep`, `std::fs::*`, blocking `std::net`, `block_on` in the app/reducer crates or modules; `await_holding_lock`; `large_futures`.
  - Privacy: newtypes for message bodies and phone numbers whose `Debug`/`Display` redact, so they can't reach logs by accident.
  - Arithmetic: `arithmetic_side_effects` or `cast_*` lints in the store/timestamp code.
  - Dependencies: `cargo-deny` (licences compatible with AGPL, RustSec advisories, banned/duplicate crates, git sources allowlist for presage), `cargo-machete` for unused deps.
  - Architecture: a test that fails if `faro-core` depends on `ratatui`/`crossterm`, or if the reducer module imports tokio I/O.
  - Exhaustiveness: no `_ =>` wildcard arms on faro's own enums (`wildcard_enum_match_arm`) so new events/actions can't be silently ignored.

### D28. First D27 rules: no panics, no blocking calls, no stray terminal output — Judgement

- **No panics** outside tests: `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `unreachable`, `dbg_macro`, `indexing_slicing`, `exit` (skips destructors such as the log flush guard). `clippy.toml` relaxes these in `#[cfg(test)]` code. `indexing_slicing` applies workspace-wide, not only to core logic: `.get()` is cheap and one rule is simpler than a per-module one.
- **Never block the UI:** `clippy.toml` `disallowed-methods` bans blocking std calls everywhere (`thread::sleep`, `std::fs`, `Path::exists`/`metadata`…, blocking `std::net`, waiting on child processes, `stdin`). Banning them in every crate (rather than only the reducer, which doesn't exist yet) makes every blocking call an explicit exception. Legitimate ones (startup setup, tests, later the store's own thread) carry `#[expect(clippy::disallowed_methods, reason = "...")]`. Add `block_on` entries when tokio/futures are added. `await_holding_lock` (default) and `large_futures` (pedantic) were already on.
- **No stray terminal output:** `print_stdout`, `print_stderr`. Pre-TUI CLI output opts out per function.
- **Exceptions must be justified:** `allow_attributes` + `allow_attributes_without_reason` force `#[expect(lint, reason = "...")]`. `expect` also fails the build when it no longer applies, so exceptions can't go stale.
- Note: `#[expect]` on a macro call such as `println!` is ignored; put it on the enclosing function.

### D29. Domain type choices (M1) — Judgement

- **Privacy by type:** `MessageBody` and `PhoneNumber` have a redacting `Debug` (`MessageBody(<redacted>)`) and no `Display`, so they can't reach logs through `?field`, `{:?}` or derived `Debug` on containing types. Reading the value requires `.expose()`, which is easy to grep for in review. Quote previews and edits use `MessageBody` too. This adopts the D27 privacy candidate.
- **No separate `Recipient` type:** PLAN listed both `Recipient` and `Contact`; they would describe the same thing (a person with a service ID, names and an optional phone). `Contact` covers both; the store table is still `recipients`.
- **Outgoing status lives in `Direction`:** `Direction::Outgoing(DeliveryStatus)`, so an incoming message can't have a delivery status.
- **`DeliveryStatus::update` never regresses:** receipts can arrive late, twice or out of order, so a confirmed status (sent < delivered < read) only moves forward. `Failed` replaces only `Sending`; a retry or a confirmation replaces `Failed`.
- **`ServiceId`** is `Aci(Uuid)` / `Pni(Uuid)` with Signal's string form (bare UUID / `PNI:<uuid>`). Adds the `uuid` crate (presage uses it anyway). **`GroupId`** is the 32-byte v2 ID, shown as hex. **`Timestamp`** is milliseconds since the epoch (`u64`), as Signal uses; no date library until the UI needs formatting.
- **`wildcard_enum_match_arm`** is now on (D27 exhaustiveness candidate). It doesn't see `_` inside tuple patterns, so matches on tuples of faro enums should spell out the variants (see `DeliveryStatus::update`).

### D30. Backend trait shape (M1) — Judgement

Refines the DESIGN.md §5.1 sketch:

- **Native async trait methods, no `Send` bound, no `async-trait` crate.** presage's futures have historically not been `Send` (gurk runs them on a `LocalSet`), so requiring `Send` could make the M7 backend impossible to write. The backend runs in one dedicated task anyway (D22), so nothing else needs `Send`. Consequence: the trait isn't object-safe (no `dyn Backend`); code is generic over `B: Backend`, and choosing mock vs presage at runtime will use an enum or a generic `run::<B>()`.
- **faro picks `sent_at`** (`OutgoingMessage.sent_at`), as every Signal client does, so a sent message's `MessageRef` is known before sending. `send` returns `()`; delivery progress arrives as `BackendEvent::DeliveryUpdate`. This replaces the sketch's `SentMessage` return value.
- **Event channel from the constructor**, not a trait method: one bounded `tokio::sync::mpsc` channel (capacity 1024; backpressure on the backend task, not unbounded memory). Avoids a `fn events()` that could only be called once.
- **Conversation passed to `react`/`edit`/`delete_for_everyone`:** Signal sends these as messages to the conversation (the group or the person), so the backend needs it.
- **Linking left out of the trait** until M7: it happens before a linked backend exists, and presage's flow decides the shape.
- **Capabilities are optional features only** (`Edit`, `DeleteForEveryone`, `Attachments`), a small bitset. Text, replies, receipts, typing and reactions are required. `require()` gives backends a uniform `Unsupported` error.
- **Errors:** `Network`, `Unlinked`, `RateLimited { retry_after }`, `Protocol`, `Storage`, `NotFound`, `Unsupported`, plus `is_transient()`. Detail strings are for logs and must not contain bodies or phone numbers.
- **`block_on` banned** (D27/D28): `tokio` `Runtime::block_on`, `Handle::block_on`, `block_in_place`, and `futures::executor::block_on`, in `clippy.toml` with `allow-invalid` so crates without those dependencies don't warn. `#[tokio::test]` expands to `block_on`, so async tests carry `#[expect(clippy::disallowed_methods, reason = ...)]`.

### D31. Mock backend design (M1) — Judgement

- **Pure core, thin driver.** `mock/sim.rs` is a synchronous, seeded scheduler with no I/O: a priority queue of entries due at "milliseconds since start". `mock.rs` holds it behind a `std::sync::Mutex` (never held across `.await`; poisoning ignored) and runs a driver task that sleeps until the next entry and forwards events. Follow-ups are scheduled from each entry's *due* time, not the wall clock, so for a given seed the sequence doesn't depend on how late the driver wakes. Determinism can be tested without a runtime, by jumping from one due time to the next.
- **RNG: `fastrand` 2** (small, no dependencies, seedable). Its output isn't promised to stay the same across versions, so tests compare two runs with each other instead of checking fixed expected values, and `Cargo.lock` pins the version. Chosen over `rand_chacha`, whose version-stable output isn't needed.
- **Clock: tokio's** (`tokio::time::Instant`, `sleep_until`). Tests use `#[tokio::test(start_paused = true)]` (dev-only `test-util` feature), so time jumps instantly and deterministically. Message timestamps are `epoch + elapsed`; `MockEpoch::Fixed` (`MockConfig::deterministic`) makes them reproducible, `MockEpoch::Now` makes them look normal.
- **Fixed cast, seeded behaviour.** Contacts, groups and IDs come from a constant seed, separate from `MockConfig::seed`, so IDs stay the same across runs and stored history (M2) still matches. Only the activity depends on the seed. The default seed is `0`; a CLI flag can override it later.
- **`MockBackend::new` spawns the driver** with `tokio::spawn` and aborts it on drop, so it must be called inside a Tokio runtime (otherwise tokio panics). This is accepted because it's called once at startup, where a mistake shows up immediately.
- **Behaviour:**
  - On connect, the mock delivers a backlog of messages dated over the past three days (Signal's server queue), then scripted activity: messages with typing indicators, replies (quotes), reactions, edits, deletes, sync messages (`Outgoing`), profile changes, and optionally outages and decryption errors.
  - Events produced during an outage are held back and delivered on reconnect.
  - **Commands fail with `Network` while offline, including before the first `Connected`**, so the app must cope at startup.
  - The app owns the `Sending` state. After `send` returns `Ok`, the mock emits `Sent` → `Delivered` → `Read` (every recipient reads) and replies 60% of the time.
  - Edits and deletes of other people's messages return `NotFound`.
  - Mock messages have no attachments. `Attachments` is off by default, and `fetch_attachment` returns `NotFound` even if it's enabled.
  - `simulate_outage` and `inject_error` trigger outages and errors on demand.

### D32. `faro --mock` before the TUI (M1) — Judgement

> The headless run and its Ctrl+C handling were replaced by the TUI in M3 (D35); the runtime choice still holds.

- **Interim headless run:** until the TUI exists (M3), `faro` starts the mock backend, logs every event's *kind* (debug level; connection changes at info, errors at warn; never bodies, names or phone numbers), and on Ctrl+C prints and logs a one-line summary of event counts. The code is in `crates/faro/src/headless.rs`, to be deleted when the TUI replaces it.
- **Flags:** `--mock` is accepted and does nothing extra for now: the mock is the only backend until M7, which makes presage the default. `--mock-seed <u64>` (env `FARO_MOCK_SEED`, default `0`) picks the scripted activity; the cast never changes (D31).
- **Runtime:** `main` stays synchronous for setup (paths, logging), then builds a **current-thread** tokio runtime and calls `block_on` (with an `#[expect]` on `main`) instead of using `#[tokio::main]`. One thread is enough for a TUI plus I/O tasks and suits presage's `!Send` futures (D30). Revisit if the store or rendering needs more threads.
- **Ctrl+C** uses `tokio::signal::ctrl_c` (tokio `signal` feature). Once the TUI is in raw mode, Ctrl+C arrives as a key event instead, so this only matters before M3.

### D33. Commit and push after every finished task — Accepted (2026-10-07)

- **Decision (owner):** whenever a well-defined chunk of work is finished (a PLAN task or a milestone), agents commit and push it without asking, once the quality gate (fmt, clippy, tests) is green. This supersedes D23.
- Commits stay small with short imperative messages. Pushed history is never rewritten.

### D34. Store design (M2) — Judgement

- **sqlx 0.9, aligned with presage.** presage's SQLite store (`presage-store-sqlite`, main branch) uses `sqlx` 0.9 and `libsqlite3-sys` 0.36 from whisperfish's rusqlite fork through a workspace `[patch.crates-io]`; its default feature is `bundled-sqlcipher`. `sqlx-sqlite` 0.9 accepts `libsqlite3-sys >=0.30.1, <0.38`, so both can share one `libsqlite3-sys` (Cargo allows only one crate with `links = "sqlite3"`). Until M7 faro resolves 0.37 (bundled SQLite). **In M7, copy presage's `[patch.crates-io]` entry** so Cargo picks the fork; faro's database then links SQLCipher too, which behaves like plain SQLite when no key is set. No old version is forced.
- **Runtime queries** (`sqlx::query`/`query_as` with tuple rows), not the compile-time `query!` macros, which need a live `DATABASE_URL` or checked-in offline data. Migrations are embedded with `sqlx::migrate!()` (`crates/faro-core/migrations/`) and run on open.
- **One task owns the only connection (D22).** `Store` is a cloneable handle over a bounded `mpsc` channel (capacity 256, so callers wait under load instead of memory growing); each request carries a `oneshot` reply. A single `SqliteConnection` instead of a pool: one writer, no locking surprises, simple transactions. The task stops on `Store::close` (which closes the connection cleanly, checkpointing the WAL) or when every handle is dropped. Later handles get `StoreError::Closed`.
- **File:** `<data dir>/faro.db` (`Paths::db_file`). Pre-created as `0600` before SQLite opens it; SQLite gives its `-wal`/`-shm` files the same mode. WAL journal, `synchronous = NORMAL`, foreign keys on.
- **Applying events is idempotent:** duplicate messages (same `(sender, sent_at)`) are ignored, receipts go through `DeliveryStatus::update` so they never regress, only newer edits replace the text, and edits to deleted messages are ignored. A delete keeps a placeholder row and drops body, quote preview, reactions and attachments. Events for unknown messages are dropped (debug log). Connection, typing and error events aren't stored.
- **Unread count** goes up only for new incoming, non-deleted messages; `mark_read` resets it. `last_activity` is the newest message's `sent_at`.
- **Every contact gets a direct conversation**, possibly empty (listed after conversations with messages). Hiding empty ones, or showing them only in search, is a UI decision for M3/M4.
- **Schema changes from the DESIGN.md §6 sketch:**
  - STRICT tables and `CHECK` constraints (direction/status agree, quote columns both set or both null, booleans 0/1), so the database rejects bad rows.
  - `conversations.peer` links a direct conversation to its recipient; the title is computed from the contact (`Contact::display_name`) when listing; `title` is used for groups only.
  - `group_members` table (groups have members in the domain model).
  - Quotes stored as `quote_sender` + `quote_sent_at` + `quote_preview`, not a row reference, because the quoted message may not be stored.
  - Left out until something uses them: recipient avatar path and colour seed, `conversation_state.scroll_position`, `attachments.status`. Each is a future migration.
- **Reads validate everything:** a bad stored value becomes `StoreError::Corrupt("<column>")`, never a panic.
- **Testing note:** tests that use the store run on real time. With tokio's paused clock, the runtime looks idle while it waits on SQLite's worker thread, so the clock auto-advances and timers fire early.

### D35. TUI skeleton (M3) — Judgement

- **Crates:** `ratatui` 0.30 (default features off; `crossterm`, `layout-cache`, `underline-color`) and `crossterm` 0.29 with `event-stream`, the version ratatui itself uses, so there is one crossterm in the tree. Both are in `crates/faro` only; a test in `faro-core` (`architecture`) fails if its `Cargo.toml` mentions either (D20). `jiff` gives local time (the system time zone is read once at startup, falling back to UTC). `unicode-segmentation` + `unicode-width` measure graphemes (DESIGN §11).
- **App loop** (`app.rs`): one `tokio::select!` over terminal events, backend events, store results, a redraw timer, the next typing-expiry deadline, and SIGTERM/SIGHUP (which quit cleanly like Ctrl+Q). The reducer (`app/update.rs`) is synchronous: `update(&mut State, AppEvent) -> Vec<Effect>`.
- **Effects worker** (`app/effects.rs`): one task runs effects in order (load conversations, load messages, persist an event, mark read, remember the open conversation) and sends results back as `AppEvent`s over an unbounded channel. Order matters (a persist must land before a later reload). The queue is unbounded so the reducer never waits; the store's bounded channel (D34) is where backpressure happens. On quit, queued writes are finished before the store closes: nothing is lost, and under a 300 msg/s mock flood this took 4 s in a debug build and 0.16 s in release.
- **Loading without gaps:** while an open conversation's history loads, its live events are buffered and replayed on top once the page arrives. The in-memory timeline (`app/timeline.rs`) applies events with the same rules as the store (dedupe, receipts never regress, newer edits win, deletes keep a placeholder), so the screen matches what a restart would show.
- **Rendering:** redraws are coalesced to at most one per 16 ms (about 60 fps) and only for events that change the view (mouse and focus events don't, yet). Each frame is wrapped in synchronized output (DEC mode 2026) to avoid tearing. Message lines are built newest first and only until the pane is full, so long histories cost nothing extra.
- **Terminal safety:** raw mode, alternate screen and mouse capture are entered by `tui::Tui` and restored in its `Drop` and in a panic hook (installed once). All text from the network goes through `ui::text::sanitize`, which replaces control characters with `�` so a message can't inject escape sequences.
- **Theme:** `Theme` (in `crates/faro/src/theme.rs`, because it holds ratatui colours) has the semantic roles from DESIGN §10 plus `unread` and an 8-colour sender palette (`uuid % 8`). One built-in theme, `faro-dark`, with faro's own colours (amber primary), so there is no third-party licence question yet. The theme paints the background instead of relying on the terminal's.
- **Layout:** left sidebar 30 columns (hidden below 70 columns), right info sidebar 28 columns (hidden below 110 columns), the message pane in between with the input as its own panel at the bottom (grows to 6 lines, then scrolls), and a footer with connection state, unread total, and width-aware key hints or the latest notice. Fixed widths until resizing lands (v0.2).
- **IRC layout:** `time  name  text`, names right-aligned in a column that shrinks on narrow panes; a sender's repeated messages within 10 minutes hide the name. Quotes, reactions, `(edited)`, delivery glyphs (◌ ✓ ✓✓, read ✓✓ in the secondary colour, `! not sent`), deleted placeholders, attachment name and size, date rules (Today / Yesterday / weekday and date) and a "new messages" rule above the first unread message as it was when the conversation was opened.
- **Conversation list:** hidden, archived and empty conversations are not shown (DESIGN §7.7 default), except the open one. "Pinned"/"Recent" headers appear only when something is pinned. On startup the last open conversation (stored in `app_state` as `ui.last_open_conversation`) is reopened; on a fresh database the first incoming message opens its conversation, so typing always has a target.
- **Keys in M3** (a subset of D13): `Ctrl+Q` quits; `Ctrl+C` asks for a second press; `Alt+↑/↓` switch conversation (`Alt+J/K` added by D36); `Alt+S`/`Alt+I` toggle sidebars; `Esc` dismisses the notice; printable characters and `Backspace` (by grapheme) edit the input. `Enter` does nothing until sending exists (M4). Mouse events are captured but ignored until M4. Drafts are kept per conversation in memory only, until M4.
- **Startup order:** the backend's account, contacts and groups are loaded (and stored) before the terminal enters raw mode, so the first frame already has names.
- **Mock flags:** `--mock-activity <ms>` (env `FARO_MOCK_ACTIVITY`, default 4000) sets the mean time between scripted events; `--mock-no-faults` (env `FARO_MOCK_NO_FAULTS`) turns off scripted outages and errors. A flood test needs both (`--mock-activity 2 --mock-no-faults`): with faults on, an outage starts within a few hundred events and nothing arrives while offline.
- **Known issue, not fixed in M3:** emoji with variation selectors (☺️ ❤️), flags and ZWJ sequences are drawn narrower by tmux than `unicode-width` reports, which shifts borders on those lines. Tracked by the emoji width fallback (DESIGN §7.3) and PLAN M8 task 2.

### D36. `Alt+J/K` switch conversations — Accepted (2026-10-07)

- **Problem (owner, hands-on):** in the owner's tmux, `Option+↑/↓` (Alt+arrows) already move between panes, so tmux takes them before faro sees them.
- **Decision (owner):** `Alt+K` / `Alt+J` are the default previous / next conversation keys (vim-style; tmux has no default binding for them, and Alt+letter never types text). `Alt+↑/↓` stay as a second binding for setups where tmux doesn't take them. Letter bindings match either case, so Shift or Caps Lock don't break them. Amends D13.
- `Alt+↑` no longer doubles as "enter message selection" (DESIGN §7.4 had both meanings); `↑` on an empty input is enough.
- **macOS note:** Option+letter types a symbol (Option+J is `∆`) unless the terminal sends Option as Alt/Meta: kitty `macos_option_as_alt yes` (or `left`), iTerm2 Left Option → `Esc+`, Terminal.app "Use Option as Meta key". The same applies to `Alt+S`/`Alt+I`. Document in the README (M8); everything becomes rebindable in M5.
- **Owner's first hands-on look at M3** also confirmed the emoji width artifacts (D35); still planned for M8 task 2 unless they get in the way sooner.

### D37. Focus, scrolling and mouse (M4 task 1) — Judgement (message-pane keys amended by D40)

- **Three focusable panes:** conversations, messages, input (default). `Tab` / `Shift+Tab` cycle them, skipping a hidden sidebar; `Esc` returns to the input (and clears the notice); `Enter` in the sidebar or message pane also returns to the input. The focused pane gets the active border colour, and the footer hints follow focus (DESIGN §7.1). Hiding the sidebar while it has focus (or a narrow terminal hiding it) moves focus to the input.
- **Keys per pane:** in the sidebar `↑/↓` or `j/k` switch conversation immediately (like `Alt+J/K`), `g/G` or `Home/End` jump to first/last, `Enter` goes to the input. In the message pane `↑/↓` or `j/k` scroll a row, `g/G` or `Home/End` jump to top/bottom. Unbound letters do nothing outside the input; they don't jump to it, because `j`/`k`/`g` would then behave differently from other letters. `PgUp/PgDn` scroll the messages from any pane (a page is the visible height minus two rows).
- **Mouse:** a left click on a conversation opens it and focuses the input; clicking the message area or input focuses it; the wheel scrolls whatever is under the pointer (3 rows per notch; the sidebar list scrolls independently of the selection). Moves, drags and other buttons don't cost a frame. Right-click menus and drag-resizing stay in v0.2.
- **Scroll position is an anchor, not a row count:** `OpenConversation::scroll` is `None` (follow the newest message) or `Anchor { message, offset }`, the bottom row's distance above that message's last row. New messages below don't move the view; loading older pages above doesn't either. While scrolled up, a badge on the bottom row says "↓ more below" or "↓ N new messages"; reaching the bottom (`End`, `G`, `PgDn`) resumes following and clears the count.
- **Renderer feedback (`Geometry`):** rendering stays pure, but `ui::render` returns what it drew: the sidebar and history rectangles, which conversation is on each sidebar row, the sidebar offset actually used, the normalised scroll anchor and whether the first loaded message is on screen. The app loop feeds it straight back as `AppEvent::Drawn`, which doesn't trigger a redraw. This gives the reducer mouse hit-testing and page sizes without duplicating layout code, and lets the renderer clamp scrolling.
- **Older history loads on demand:** when a scrolled view reaches the top of what's loaded and the last page was full (200), the reducer asks for the previous page (`Effect::LoadOlder`, one at a time). The anchor keeps the view still while the page is prepended.
- **Cost:** following the newest message still lays out only what fits. While scrolled, every loaded message is laid out each frame (simple and exact); fine for thousands of messages, revisit with a layout cache if long histories get slow.

### D38. Input editing and drafts (M4 task 2) — Judgement

- **Editor:** `app/input.rs` holds the text and a cursor (a byte offset on a grapheme boundary), so emoji, flags and accented letters move and delete as one character. Inserted text keeps `\n`, turns tabs into spaces and drops other control characters (pasted escape codes, `\r`).
- **Keys (input focused):** arrows; `Home`/`End` and `Ctrl+A`/`Ctrl+E` for line start/end; `Alt+←/→`, `Ctrl+←/→` and `Alt+B`/`Alt+F` move by word; `Backspace`/`Delete`; `Ctrl+W`, `Alt+Backspace` and `Ctrl+Backspace` delete the previous word, `Alt+D` and `Ctrl+Delete` the next; `Ctrl+U` deletes to line start, `Ctrl+K` to line end (or the line break). These are the readline/Emacs keys that work in shells, so they're familiar over SSH; none clash with tmux's defaults (`Ctrl+B` stays free). A word is a run of letters, digits or `_`.
- **Newlines:** `Alt+Enter` (D13) and `Ctrl+J` work in every terminal, including under tmux; `Shift+Enter` works only where the kitty keyboard protocol reports it. faro doesn't turn that protocol on yet (tmux passes it through only partially, DESIGN §11); revisit in M8. `↑/↓` move between lines of a multi-line message; on the first line `↑` does nothing for now (M4 task 4 uses it to select messages).
- **Paste:** bracketed paste is enabled, so a multi-line paste arrives as one event and goes into the input with its newlines (and focuses the input), instead of arriving as keystrokes where each newline would be `Enter` (= send, from task 3).
- **Drafts are stored**, in `conversation_state.draft` (D34's schema): when switching conversation, on quit (Ctrl+Q, Ctrl+C twice, SIGTERM/SIGHUP), and 1 s after the last edit, so a crash loses at most a second of typing. Only real edits count; moving the cursor doesn't trigger a save. A restored draft opens with the cursor at its end. The in-memory `State::drafts` map is gone; `ConversationSummary::state` is the source of truth.
- **Input box:** wraps at grapheme boundaries, grows to 6 lines, then scrolls to keep the cursor's line visible. The cursor is shown only while the input has focus.

### D39. Sending (M4 task 3) — Judgement

- **The backend moves into the app.** `app::run` takes the backend and hands it to a command task (`app/commands.rs`) that owns it. The reducer asks for backend work with `Effect::Backend(Command)`; the app loop routes those to the command task and everything else to the store worker. Backend futures may be `!Send` (D30), so the task is spawned with `spawn_local` and `main` runs the app inside a `tokio::task::LocalSet` (still the current-thread runtime). `LocalSet::block_on` joins the `block_on` ban in `clippy.toml`.
- **Commands run one at a time, in order**, so messages leave in the order they were written. A slow send delays later commands (typing, receipts from task 6); acceptable for now, revisit if it shows.
- **`Enter` sends** (input focused); blank input does nothing. The text is trimmed at both ends. The message is shown and stored at once as `Sending` (through the same path as any new message, so the sidebar order and `Persist` happen as usual), the input and any stored draft are cleared, and the view jumps to the bottom. faro picks `sent_at` as the wall-clock time, always later than the previous send, so two quick sends can't share an identity.
- **Results:** the command task answers with `AppEvent::SendResult`; `Ok` becomes a `Sent` delivery update and an error becomes `Failed` plus an error notice ("Message not sent: …"). Both go through the normal delivery-update path (stored, never regressing). Receipts then move it to delivered and read.
- **Sends in flight at quit** are abandoned (the command task is aborted). On the next start `Store::fail_unsent` marks any message still `Sending` as `Failed`, because its outcome is unknown; better a visible "not sent" the user can retry than a spinner forever. Retrying a failed message comes with message selection (task 4).
- **Glyphs:** `◌` sending, `✓` sent, `✓✓` delivered, `✓✓` (secondary colour) read, `! not sent` (error colour) failed. The input's footer hints start with `enter send`.

### D40. Message selection (M4 task 4) — Judgement

- **Selection is the message pane's mode.** Focusing the message pane (`↑` on an empty input, `Tab`, a click) selects the message on the bottom row (the newest, or where the view is scrolled). `↑/↓`, `j/k` move to the older/newer message and `g/G`, `Home/End` to the oldest/newest loaded; `↓` past the newest returns to the input, mirroring `↑`. **Amends D37:** these keys no longer scroll a row at a time; `PgUp/PgDn` and the wheel still scroll freely. One concept ("the pane has a selected message") beats a separate selection mode on top of a scrolling pane.
- **Revealing:** moving the selection asks the renderer to bring the message on screen (`Selection::reveal`, like the sidebar's): if it's above the view, its top (with its date rule) goes to the top row; if below, its last line to the bottom row. The renderer reports the resulting anchor; `Drawn` clears the flag. Selecting the oldest loaded message brings the top of the history on screen, which loads the previous page as before.
- **Mouse:** the renderer also reports which message is on each history row (`Geometry::history_rows`), so a left click selects that message and a right-click opens its menu (D37 had deferred right-click menus; with row mapping they cost a few lines). Any click or wheel while a popup is open just closes it.
- **Actions:** `r` reply: the quote (target + preview) waits in `OpenConversation::reply_to`, shown as a bar in the input box, sent with the next message, dropped on `Esc` or switching conversation. `+` react: a picker with Signal's six default reactions (`1`–`6`, arrows, `Enter`); choosing your current reaction removes it. The reaction is shown and stored at once and undone (with an error notice) if the backend fails (`Command::React` carries the previous emoji). `y` copy: the message text via OSC 52 (crossterm's `osc52` feature); under tmux this needs `set -g set-clipboard on`. `o` open link: http(s) links found in the text (surrounding brackets and punctuation trimmed); one opens directly, several open a menu. Links open with `open`/`xdg-open` (spawned, not awaited; failures show a notice), **except over SSH** (`SSH_CONNECTION`/`SSH_CLIENT`/`SSH_TTY` set), where a browser would open on the far machine, so the link is copied instead. Detection misses a tmux server started locally and attached over SSH later; the menu's link items and config (M5) can refine this.
- **Action menu** (`Enter` or right-click): Retry sending (failed own messages; resends with the same `sent_at`, back through `Sending`), Reply, React…, Copy text, and one "Open …" per link (up to 5), each with its key. Deleted messages offer nothing, so no menu opens. Edit and delete (D13 keys `e`/`d`) aren't in M4's scope and come later.
- **Popups** (`State::popup`) are modal: they take every key except `Ctrl+Q`/`Ctrl+C` (`keys::popup_action`), sit just above the message they act on (else below, else at the bottom), and get their own footer hints.
- **Notices** gain an `Info` level that clears itself after 3 s ("Copied to the clipboard"); warnings and errors stay until `Esc` or the next action, as before.

### D41. Conversation list curation (M4 task 5) — Judgement

- **Keys act on the sidebar's highlight**, which is the open conversation, and only while it's listed in the current view. With the sidebar focused: `p` pin/unpin, `K`/`J` move a pin up/down, `m` mute (picker: 1 hour, 8 hours, 1 day, 1 week, until unmuted; on a muted conversation `m` unmutes at once), `h` hide/unhide, `a` archive/unarchive, `l` add to/remove from the allowlist, `L` allowlist mode on/off, `A` switch between the chats and the archive, `Space` (or right-click on a conversation) opens a conversation menu with the same actions and their keys. `Enter` keeps going to the input (D37). `Alt+1`…`Alt+9` open the Nth pinned conversation from anywhere (DESIGN §7.6), switching back to the chats if needed. The capitals are distinct keys because `Shift` arrives already applied to the character.
- **Hide vs archive.** Hidden conversations come back when any new message arrives (incoming or sent, including from the phone), as DESIGN §7.7 says; this is what Signal calls "archive". Archived conversations stay archived whatever arrives. Making the hide behaviour configurable waits for M5.
- **Two lists, not one long one.** The sidebar shows either the **chats** (Pinned, Recent) or the **archive** (Archived, "Hidden until a message", and in allowlist mode "Not on the allowlist"), each under a header. The chats end with a "▸ Archive (N)" row and the archive starts with "◂ Chats"; both are clickable. This keeps hidden things findable (and undoable) before the palette exists, without cluttering the main list. Startup opens the list the remembered conversation is in.
- **One place decides where a conversation goes:** `State::section` (archived → hidden → pinned → not allowed → recent), and `State::listed_in(view)` lists a view's sections in display order, so the sidebar, `Alt+J/K`, `j/k` and the reducer always agree. Empty conversations are listed only when open, pinned or allowed (in allowlist mode).
- **Invariants:** pinning un-hides and un-archives; hiding or archiving unpins; archiving clears `hidden`. New pins go to the end; moving a pin renumbers the pins 0, 1, 2… and stores those that changed.
- **Moving on.** When the open conversation leaves the current view (hidden, archived, unarchived from the archive, taken off the allowlist, allowlist mode turned on), the conversation that took its place opens, else the one before it. With nothing left: from the archive, back to the chats (where it now is); from the chats, the conversation closes (draft saved).
- **Allowlist mode:** a per-conversation `allowed` flag (migration `0002_allowlist.sql`) and a global on/off switch. With it on, the chats list only allowed and pinned conversations; the rest go to the archive's "Not on the allowlist" section, where `l` brings them back. The switch is a preference, which belongs in the config file (D17), but that arrives in M5; until then it's stored in `app_state` (`ui.allowlist_mode`) and M5 moves it.
- **Mute** stores `muted_until`; "until unmuted" is `ConversationState::MUTED_FOREVER` (the largest timestamp SQLite's INTEGER holds). `ConversationState::muted_at(now)` is the one check. Muted conversations are drawn dim (name, unread dot and count) and don't count towards the footer's unread total; nor do archived or hidden ones. The info panel shows "Muted until Thu 19:00" (or "Muted"), and where the conversation is listed unless it's simply recent.
- **Menus vs the reaction picker:** in menus `h` and `l` are shortcuts (Hide, Allow), not moves; only the reaction picker, which is horizontal, keeps `h`/`l` for moving (`keys::popup_action(key, horizontal)`).
- **Notices** confirm each change ("Pinned (Alt+2 opens it)", "Hid Alice until a new message arrives", "Archived Bob (A shows the archive)", "Muted for 8 hours"). There's no undo beyond doing the opposite action. Bulk "hide all older than N days" (DESIGN §7.7) belongs to the palette (task 8).

### D42. Typing indicators, read receipts and unread counts (M4 task 6) — Judgement

- **Read means seen.** A message counts as read only when it's on screen: the terminal has focus, the open conversation's history has loaded, and the view is at the bottom. Until then it counts as unread, even in the open conversation (the sidebar count goes up and the "new messages" rule goes above it if there isn't one yet). When it becomes seen (focus comes back, a key is pressed, the view scrolls back down, the history loads), faro sends read receipts for the waiting incoming messages in one `Command::MarkRead` and clears the stored unread count. Opening a conversation still clears its count at once, as before (it's an explicit action).
- **Focus** comes from terminal focus reporting (crossterm `EnableFocusChange`). Any key or mouse event also counts as focus, so terminals without focus reporting behave as "always focused". Under tmux, focus reporting needs `set -g focus-events on`.
- **Typing:** "started" on the first edit and again every 10 s while edits go on (Signal clients time the indicator out after 15 s); "stopped" after 3 s without an edit, when the input becomes empty, and when switching, closing or quitting. Nothing after sending: the message itself ends the indicator for recipients. Cursor moves aren't edits. The "stopped" sent on quit may be lost because the command task is dropped on exit; recipients time out anyway.
- **Failures** of receipts or typing are only logged: nobody needs an error notice for them.
- **Not yet:** read syncs from our other devices (a message read on the phone should clear the unread count here) need the conversation for each read message, which presage gives only by lookup; this goes to M7 task 3. Settings to turn off sending receipts or typing indicators belong in the config file (M5).
