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
- **Plan:** command is `faro`. If needed when going public, repo/packages can use `faro-chat`. Not a trademark/legal assessment. *(Package names settled by D85.)*
- **Rejected names:** see BACKGROUND.md (naming history).

### D12. Logo direction: lighthouse with a rotating beam — Accepted

- Prototype: `python3 tools/logos.py browse lighthouse` (it's the "pharos" lighthouse art; "faro" uses the same icon). Final art to be redone in Rust (splash/about screen), with a `reduce_motion` option. **Done** for the empty message pane (D61, `ui/logo.rs`); an about screen is still later.
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
>
> The `--mock` / `--mock-seed` flags were replaced by the `faro demo` subcommand (D87).

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
- **Known issue, not fixed in M3:** emoji with variation selectors (☺️ ❤️), flags and ZWJ sequences are drawn narrower by tmux than `unicode-width` reports, which shifts borders on those lines. Tracked by the emoji width fallback (DESIGN §7.3) and PLAN M8 task 2. *(Amended by D46: not reproduced with tmux 3.6b; moved to M4.5.)*

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

### D41. Conversation list curation (M4 task 5) — Judgement (allowlist mode storage superseded by D48)

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

### D43. Key bindings as data (M4 task 7) — Judgement

- **One table, one lookup.** `app/keymap.rs` replaces the per-pane `match`es: `defaults.rs` lists `(context, key, action)` bindings (DESIGN §7.6) and `State::keymap` (`Keymap`) answers `lookup(key, focus)`. Contexts are `global`, `input`, `conversations`, `messages`, the config file's `[keybindings.<context>]` tables (DESIGN §9). Lookup order: the focused pane's bindings, then global ones, then (in the input only) an unmodified character types itself. Pane bindings winning over global ones is new (before, global keys were checked first); with the defaults nothing changes, and it lets a user give a key a pane-specific meaning.
- **Keys are written as in the config file:** `ctrl+p`, `alt+j`, `shift+enter`, `shift+tab`, `space`, `pgup`, `f1`; a single character is case-sensitive (`G`, `+`, `?`) and `shift+g` means `G`. Shift is dropped from characters and `BackTab` (it's already in the key) and kept elsewhere, so `shift+enter` is a binding of its own. `Ctrl`/`Alt` with a capital also matches the lowercase binding, since terminals differ on whether Shift reaches us there. Consequence: `Shift+arrow` and `Shift+Enter` outside the input no longer act like the plain key.
- **Actions have stable dotted names** (`app.quit`, `conversation.next`, `conversation.pinned.3`, `input.word_left`, `message.react`, `list.switch_view`…; full list in `keymap/defaults.rs`). Typing a character and popup keys have no name: they can't be bound.
- **Rebinding API for M5:** `Keymap::apply(context, key, action)` takes one config entry; the action `"none"` unbinds. A rebound key replaces the old binding in that context and becomes the key shown in hints. Mistakes come back as `KeymapError` (unknown section, key or action) and change nothing, so M5 can list them in an error notice and keep the rest.
- **Hints follow the keymap.** The footer's hints name actions, and their keys come from `Keymap::hint` (the first key bound to each action; `↑↓` for bare arrows, `alt+j/k` for a shared modifier). An unbound action loses its hint. The palette (task 8) will use the same names and hints.
- **Popups keep fixed keys** (arrows, `j`/`k`, `Enter`, `Esc`, shortcut letters; `keymap::popup_action`): they're generic list navigation, and their shortcut letters are shown in the menu itself.
- **Enforced by tests:** every default key parses, no key is bound twice in a context, every bindable action has a unique name and at least one default key.

### D44. The command palette (M4 task 8) — Judgement

- **`nucleo-matcher`, not `nucleo`.** The palette searches a few hundred short strings (commands, conversations, keybindings), which the low-level matcher (same project as `nucleo`) does synchronously in well under a millisecond. `nucleo` proper runs its own thread pool for huge lists; we don't need one, and matching in the reducer keeps the palette simple and deterministic in tests. If a list ever gets big (message search), that's the time for `nucleo` or a worker.
- **One list, built when it opens.** `app/palette.rs` builds the items from the state (`palette::items`), matches the query against item titles, and ranks by score plus a small bonus for recently used commands (3 per place in a 10-long list, so a clearly better match still wins). Nothing typed: the 3 most recent commands, then every conversation in list order, then the other commands. Prefixes: `>` commands, `@` conversations, `:` settings (empty until M5, says so), `?` the keybinding list (our addition; `F1` and `?` outside the input open it, as DESIGN §7.6 says).
- **Commands** are the global actions (next/previous conversation, show the archive/chats, allowlist mode, show/hide the sidebars, keybindings, quit), the open conversation's curation (pin/unpin, move pin, mute **for each duration directly**, unmute, hide, archive, allowlist; shown as `Climbing › Pin`), the selected message's actions (reply, react, copy, open link) when one is selected, and "Hide conversations inactive for 1 week / 1 month / 3 months / 1 year". Each shows its key from the keymap (D43). Labels follow the current state ("Pin" or "Unpin"); recency is tracked per command, not per label.
- **Bulk hide** (DESIGN §7.7): hides every conversation with no message in that time, except pinned, archived and already hidden ones; like `h`, each comes back with a new message. Fixed choices instead of typing a number keep it one step.
- **Conversations include hidden, archived, empty and not-allowed ones**, with notes (`1 unread · hidden`); choosing one switches the sidebar to the list it's in and opens it. This is the "find hidden conversations by name" D41 left for the palette. Starting a conversation with a contact who has none yet is not in M4.
- **Keys while open:** typed text edits the query with the input's editing keys (as bound; no new lines), `↑↓`/`Tab`/`Ctrl+P/N` move (wrapping), `PgUp/PgDn` page, `Enter` runs (with nothing matching, stays open), `Esc` closes, `Ctrl+Q`/`Ctrl+C` still quit. Paste goes into the query on one line. Mouse: wheel moves, click on a result runs it, click outside closes. Keybinding lines are a reference: choosing one closes the palette.
- **Recent commands are kept in memory only** (`State::recent_commands`); persisting them can come with the config/state work in M5.
- **Look:** a box near the top, centred, at most 76 columns; query row with a `›` prompt and the prefix hint while empty; results with a type glyph (`@` person, `#` group, `›` command), dim path, matched characters in the primary colour, key or notes on the right; the footer shows the palette's keys. The renderer reports the box, rows and offset (`Geometry::palette`) for mouse hits.

### D45. The bubbles layout (M4 task 9) — Judgement (storage and the palette command superseded by D48)

- **Bubbles are the default**, as DESIGN §9's sample config says (`layout = "bubbles"`); IRC stays one palette command away ("Use the IRC layout" / "Use the bubbles layout", `PaletteCommand::SwitchLayout`). No key binding: it's a preference, not something switched mid-conversation, and a key would need a free chord in every context.
- **Stored in `app_state` (`ui.message_layout` = `bubbles`/`irc`)** until the config file exists (M5), like allowlist mode (D41); `AppEvent::Loaded` carries it (`None`: nothing stored, keep the default), `Effect::SetMessageLayout` writes it. M5 moves it to `[ui] layout` in TOML.
- **Shape:** others' messages on the left, own on the right; a bubble is as wide as its text, at most 75% of the pane (never under 20 columns unless the pane is), with one cell of padding inside a background colour (`bubble_own` warm, `bubble_other` cool; new theme roles). The time, `edited` and the delivery glyph sit at the bottom right of the bubble, on the last line if they fit with a 2-cell gap, else on a line of their own. Quotes, attachments and the deleted placeholder are the same content as in IRC (shared `messages::content`). Reactions go just under the bubble, on its side.
- **Names only in groups, only for others, only at the start of a run** (same 10-minute rule as IRC). One blank line between bubbles, even from the same sender: same-coloured boxes stacked with no gap read as one message. Date and unread rules replace the blank line. The blank line belongs to the next message's leading rows (not selectable, like rules).
- **Selection** keeps the existing tint and `▌` bar over the whole row (the bubble's colour is replaced on the selected message's rows). Simple and clearly visible; a bubble-shaped highlight can come with M8 polish if it looks wrong in practice.
- `compact` density (DESIGN §7.3) is not in M4.

### D46. Emoji widths before M5 — Accepted (owner, 2026-10-07)

- **Decision (owner):** after trying the M4 prototype, the owner's main complaint is alignment and stray characters around emoji. PLAN M8 task 2 moves forward as **M4.5**, before M5.
- **First measurement (agent, Debian, tmux 3.6b):** `tools/emoji_widths.py` found **no** disagreement between tmux and faro's widths (`unicode-width` 0.2.2 per grapheme) for variation selectors, skin tones, flags, ZWJ sequences or keycaps. D35's "tmux draws them narrower" was an unmeasured guess; it depends on versions. Suspects for the owner's artifacts: an older tmux on the machine where faro runs, or the outer terminal (kitty) disagreeing with tmux, which faro can't detect from inside tmux.
- **Approach (Judgement, to confirm against the owner's numbers):** draw graphemes whose width is in doubt as fallbacks with an unambiguous width, per category, chosen by startup detection where possible and a manual override otherwise (config setting from M5). The probe's numbers decide which categories need it by default.

### D47. Emoji: compatible mode under mosh, and two ratatui pitfalls (M4.5) — Accepted (owner, 2026-10-07; details Judgement)

- **Cause 1, mosh (confirmed with the owner's measurements):** the owner reaches faro through kitty → **mosh** → tmux. tmux 3.6b/3.7c and kitty agree with faro's widths (`unicode-width` 0.2.2 per grapheme); mosh adds up code-point widths and disagrees on 10 of 24 probe samples: VS16 emoji (❤️ = 1), skin tones (4), keycaps (1), ZWJ sequences (family = 8). Attached over plain ssh, the same tmux session looked right. faro can't change mosh, so it avoids what mosh measures differently.
- **Compatible emoji mode** (`ui/emoji.rs`): graphemes whose grapheme width differs from the sum of their code points' widths are drawn without their decorations (VS16, ZWJ, keycap, skin tones, tags): ❤️ → ❤, 👍🏽 → 👍, 👨‍👩‍👧‍👦 → 👨👩👧👦, 1️⃣ → 1. Applied in `text::sanitize`, so every measurement after sanitising matches what's drawn. Copy (`y`) keeps the original text. A **lone regional indicator** (half a flag) becomes its letter in every mode: kitty draws it 2 cells wide, tmux 1.
- **Safety net:** `emoji::finish` runs over every drawn frame and replaces anything the mode doesn't allow with a same-width stand-in (the message input isn't sanitised: its cursor maths uses the raw text). A UI test renders the fixtures (both layouts, menus, picker, palette) in compatible mode and asserts no stand-ins were needed and no cell is unsafe (D27).
- **Detection:** `--emoji auto|full|compatible` (env `FARO_EMOJI`, default `auto`; a config setting in M5). Auto checks at startup and again on terminal focus or resize (at most every 2 s): outside tmux, whether faro descends from a `mosh-server` process; inside tmux, whether **any** client attached to faro's session does (`tmux list-clients` + `ps`), since one mosh client is enough to break that client's view. A switch shows a notice and repaints. Any failure counts as "no mosh".
- **Cause 2, ratatui (found while flood-testing in plain tmux):** ratatui-core 0.1.2's diff re-sends the hidden second cell of a wide grapheme containing VS16, and the crossterm backend writes it without moving the cursor. The terminal has already advanced two cells, so the rest of the run lands one cell to the right: shifted borders and counts, rows spilling into the next. This affected every terminal, not only mosh. `emoji::finish` now marks every wide cell `CellDiffOption::ForcedWidth`, which makes the diff skip hidden cells (regression test `hidden_cells_are_never_sent`). A frame-by-frame replay of a recorded flood went from a broken grid at frame 77 to none in 459 frames.
- **Cut wide characters:** ratatui never draws a hidden cell, so a popup border over the right half of an emoji would go missing. `finish` blanks a wide character when one of its hidden cells holds a visible symbol (every overlay is bordered; recoloured blank cells are fine).
- **`Ctrl+L` redraws** the whole screen (`app.redraw`). Not via `Terminal::clear`, which asks the terminal for the cursor position and blocks until the reply, which the event reader swallows: faro hung for 2 s and quit. `Tui::clear` sends the clear sequence and forgets the last frame instead.
- **Follow-up (D55):** the glitches left after this were tmux itself joining code points onto the wrong cell (flags, VS16 emoji, some skin tones), in plain ssh too; fixed in faro's backend and `emoji.rs`. Compatible mode also draws whole flags as letters now.
- **Rejected:** a width table per terminal (can't know the terminal behind tmux), replacing emoji with text shortcodes (ugly; compatible mode keeps most emoji), turning compatible mode on always (loses skin tones and families where they work).

### D48. The config file (M5) — Judgement

- **One table of settings.** `faro-core/src/config.rs` lists every setting once (`SETTINGS`: path, one-line help, getter, setter). The first-run template, the parser, write-back and the palette's values all come from it, so they can't drift apart; tests check that the template parses to the defaults, that uncommenting every line changes nothing, and that every setting is in the template. Pure and terminal-free (faro-core); file I/O, the watcher and applying settings live in `crates/faro` (`config.rs`, `app/update/settings.rs`).
- **Hand-written parsing over `serde` derive.** The file is read with `toml_edit` (needed anyway for write-back) and each setting is read on its own, so a bad value loses only that setting: it keeps its **previous** value (at startup, the default) and the footer names the key, what was expected and what's kept. Invalid TOML keeps the whole last good config ("line N: …"). Unknown keys are warnings; `messages.density` and `messages.timestamps` (in DESIGN's sketch, not built) say "isn't supported yet". A setting removed from the file goes back to its default. With `serde` derive, one typo would have thrown away the whole file. No `toml`/`serde` dependency.
- **Settings in M5:** `theme` (built-ins only; only `faro-dark` exists, unknown names warn and fall back), `reduce_motion` (parsed; nothing animates; since D61 it stops the logo's animation), `[ui] emoji` (auto/full/compatible; `--emoji`/`FARO_EMOJI` win for the run, D47), `[conversations] allowlist` (D41), `[messages] layout` (bubbles/irc, D45), `[notifications] bell`/`desktop`/`privacy` (parsed and kept in `State::config` for M6), `[mouse] enabled` (mouse capture switched live), and `[keybindings.<context>]` (`Keymap::apply`, D43: the key map is rebuilt from the defaults plus the file on every change; bad entries are skipped and reported). **Not yet:** density and timestamps (the IRC layout's time column and bubbles' time placement would need reworking; not cheap), theme files, and settings for read receipts and typing indicators (D42).
- **Template:** every setting commented out with its default and a one-line explanation under uncommented table headers, then every default key binding per context, commented out, with its title (`# "ctrl+p" = "palette.open"   # Command palette`). Written `0600` (directory `0700`) only when the file doesn't exist; a failure to write it is a warning, not fatal. `--config FILE` / `FARO_CONFIG` override the path.
- **Write-back** (`config::set`): if the key is set, only its value changes (its comment and spacing stay); if it's commented out in its table (`# layout = "bubbles"`), that line becomes `layout = "irc"`; otherwise a missing table is appended at the end, a top-level key goes before the first table, and anything else falls back to `toml_edit`'s insert. Every text-level edit is re-parsed and checked before it's used; a file with a syntax error isn't touched (the change applies for the session and an error notice says it wasn't saved). The file is rewritten **in place**, not via a temporary file and a rename, so a symlink into a dotfiles repository survives. A single writer task does the writes in order, and the app waits for it on quit.
- **Hot reload:** `notify` watches the file's **directory** (editors that save by renaming are seen), ignores `Access` events (otherwise reading the file would trigger another read), waits for 300 ms of quiet, reads the file with `tokio::fs` and sends the text to the reducer as `AppEvent::ConfigRead`; parsing a few kilobytes in the reducer is CPU-only and well under a millisecond. A deleted file keeps the current settings. The reducer applies only what changed: reading back a file faro just wrote (or any reload with identical values) does nothing, with no notice. The same problems aren't reported twice; a reload that changes something says "Settings reloaded" (or shows the remaining problems), and fixing the file says so too.
- **Live values vs the file.** `State::config` is the file as last read, plus changes made in the app. Values that the app also changes keep their own fields (`State::layout`, `list.allowlist`, `env.emoji`) and are kept in step; the mouse and the theme are read straight from `State::config` by the app loop.
- **Palette `:` settings.** One item per value (`Message layout: IRC`, `Allowlist mode: on`, `Emoji: compatible`, `Mouse: off`), the current one marked `current`; choosing one applies it and saves it. They also show up in normal searches. The old "Use the IRC/bubbles layout" command is gone (the settings replace it). `L` (allowlist mode) saves to the file as well.
- **Moving preferences out of SQLite (D41, D45):** at startup, a stored `ui.allowlist_mode` / `ui.message_layout` that differs from the default is written into the config file, unless the file already sets that key (the file wins), and is then deleted from `app_state` (`Store::remove_app_state`). If the file can't be written, the value is used for that run and the move is retried next start. `app_state` is no longer read for them.

### D49. Notifications in v0.1: bell, window title, per-conversation level (M6) — Judgement

- **Which conversations notify** (`State::notifies`): those in the main list (pinned or recent; not archived, not hidden, not left out by allowlist mode) and not muted. Archived and not-allowed conversations are "out of the way" by the user's choice, so they neither ring nor count, matching the footer's unread total, which uses the same rule. A hidden conversation comes back to the list when a message arrives (DESIGN §7.7), so it does ring.
- **Terminal bell** (`update/notify.rs`, `Effect::Bell`, written by the app loop as a plain BEL): rings for a new incoming, non-deleted message that becomes unread, i.e. not one already in front of the user (open conversation, terminal focused, view at the bottom, D42); the open conversation does ring while the terminal is unfocused or scrolled up. Messages from our own other devices never ring. Nothing rings before the conversation list has loaded (mutes and archives aren't known yet). **Rate limit:** at most one bell per 2 s (`BELL_INTERVAL`); messages inside the window are dropped, not queued, so a flood rings once and then at most every 2 s while it lasts. `[notifications] bell = false` turns it off (live).
- **Window title** (`State::window_title`, `Tui::set_title`): `faro` or `faro (N)`, where N is the footer's unread total (conversations that notify, above). No conversation names: titles show up in tmux status lines, window switchers and screen shares. Set with OSC 0 (tmux makes it the pane title, `#{pane_title}`), checked once per drawn frame and written only when it changes. Before the first change the terminal's title is pushed on its title stack (XTWINOPS `CSI 22;0t`); on exit (also from the panic hook) or when turned off, faro writes an empty title and then pops (`CSI 23;0t`): terminals with a stack (xterm, kitty, tmux) get their old title back, the rest are left blank rather than with a stale count. New setting `[notifications] title = true` (live) for people whose tmux/shell manages titles. Bell and title are also `:` palette settings.
- **Per-conversation level** (`ConversationState::notify`, `NotifyLevel::{All, Mentions}`, migration `0003_notify.sql`): "every message" (default) or "mentions only", next to the existing mute. There is no separate "off" level: muting "until unmuted" already is that, with durations, and a third state would mean two ways to silence a conversation that disagree in the UI. A mute wins over the level. **Mentions aren't modelled** in the domain yet (presage gives them as body ranges; M7 maps them), so "mentions only" notifies for nothing until then, which is documented in the code and here; when M7 adds mentions, `State::notifies` gets the message and lets mentions through for this level (and probably for muted groups, as Signal does by default; to decide then). Switched with `@` in the sidebar (`conversation.mentions`), the conversation menu and the palette; the sidebar draws mentions-only conversations quietly like muted ones, the info panel says "Mentions only", and their unread messages don't count in the title or footer total.
- **Not in v0.1:** desktop notifications (OSC 9/99/777, native) are v0.2; `desktop` and `privacy` stay parsed and their help says "not used yet".

### D50. M8's agent-doable tasks before M7 — Judgement

- M7 (presage backend) waits for the owner's test Signal account (rule 4). Meanwhile, M8 tasks that don't depend on it are done first: **task 1** (prepare the terminal-matrix checklist), **task 3** (themes, `system` theme, 256-colour fallback) and **task 4** (README). **Task 5** (tag `v0.1.0`, release binaries) and the owner's hands-on run of the terminal matrix stay **after M7**: v0.1 isn't a release without the real backend, and the matrix has to be re-run against it anyway. The README says the Signal backend is pending until then.

### D51. Built-in themes, the system theme and colour depth (M8 task 3) — Judgement

- **Themes:** `faro-dark` (default, unchanged), `faro-dusk` (cooler dark: teal and lamp yellow on blue slate), `faro-light` (ink and deep amber on warm paper) and `system`. All colours are faro's own: bundling a well-known palette (Catppuccin, Gruvbox, Solarized…) would mean checking and carrying each licence, and those can come as theme files in v0.2 instead.
- **`system`** uses only `Color::Reset` and the 16 ANSI colours, so it follows the user's terminal scheme (light or dark). Faro can't know those colours, so nothing relies on two of them contrasting: selections and badges use **reverse video**, muted text is **dim** (SGR 2) rather than "bright black" (which equals the background in some schemes, e.g. Solarized dark), and **bubbles have no fill** (alignment still separates own and others' messages). Implemented as a `terminal_palette` flag on `Theme`, read by `Theme::selected`, `Theme::muted` and `Theme::inverse`; renderers use those instead of building selection/muted styles from roles.
- **Detecting colour depth** (`ColorDepth::detect`, once at startup): `COLORTERM=truecolor|24bit` → truecolor. **Inside tmux → truecolor** even without `COLORTERM` (which tmux servers often lack, e.g. started over SSH): tmux itself rounds RGB to the nearest 256 (or 16) colours for each client whose terminal lacks RGB, so sending RGB is always safe there and best where it's supported. Otherwise known truecolor terminals by `TERM` (`xterm-kitty`, `xterm-ghostty`, `alacritty`, `foot`, `wezterm`, `contour`, `rio`, `*-direct`: `TERM` survives SSH where `COLORTERM` doesn't) or `TERM_PROGRAM` (iTerm2, WezTerm, Ghostty, VS Code, Hyper); the Linux console, `vt*`, `ansi`, `dumb`, `*-16color`/`*-8color` → 16; everything else → 256 (Apple Terminal, mosh, `xterm-256color` over SSH). No terminal queries (DECRQSS/XTGETTCAP): replies would race the event reader (AGENTS gotcha).
- **Override:** `[ui] colors = "auto" | "truecolor" | "256" | "16"`, live, also in the `:` palette ("Colours: auto (256 colours detected)").
- **256 colours:** each RGB colour becomes the nearest of the 240 fixed xterm colours (16–255; 0–15 are left out because the user's palette decides them) by the "redmean" weighted distance, **once when the theme is loaded** (`theme::Themes` caches the theme per name and depth; nothing is mapped per frame). Dark themes' near-black greys round together, so afterwards the background, panel, selection and both bubbles are **kept apart**: a surface that rounded to the same colour as one it sits next to takes the next nearest instead.
- **16 colours:** the `system` theme, whatever theme is set (mapping RGB onto 16 colours whose real values are unknown would only guess).
- **Mechanical checks** (D27; PLAN list): contrast ratios per theme (text ≥ 7, muted ≥ 3.5, coloured text/active border/badge ≥ 3, border visible ≥ 1.4) on every background each role is drawn on, again after the 256 mapping with 10% slack; surfaces distinct in 256 colours; every fallback screen free of RGB (and, at 16, of indexed) colours; every theme name in the config help.
- **Deferred:** theme files (OpenCode JSON) and the live-preview picker stay v0.2 (theme files need a schema, a loader, error reporting and a watcher on `themes/`; not trivial). `NO_COLOR` isn't honoured yet (it would mean a monochrome theme; small, but not asked for).
- **Follow-up (2026-10-08):** theme files and the live-preview picker were pulled forward at the owner's request: D54. Bundling well-known palettes stopped being a licence question once OpenCode's MIT-licensed theme files were bundled with their notice.
- **Follow-up (2026-10-08):** the Themes list now says what the depth does (a note at 256 and 16 colours), and a theme chosen at 16 colours is saved and drawn once the depth allows: D57.

### D52. The palette is a tree — Accepted (owner, 2026-10-08; details Judgement)

- **Decision (owner):** the flat palette (D44: commands, conversations, settings and keys in one fuzzy list) mixed too much; typing "theme" listed every value of every theme setting. `Ctrl+P` now opens a **root level of categories** (`Chats`, `Themes`, `Settings`, `Commands`, `Keys`), typing filters **only the level shown**, `Enter`/`→` opens a category, `←` or `Backspace` on an empty query goes up, `Esc` closes, and the palette's title is the breadcrumb. Prefixes `@` `:` `>` `?` jump from the root into chats, settings, commands and keys. Supersedes D44's "one list" and "prefixes filter the list" bullets, and D48's "settings also show up in normal searches".
- **Judgement, keys:** `→` opens categories only when the cursor is at the end of the query (otherwise it moves the cursor); it never runs a leaf, so moving around with arrows can't change a setting by accident. `←` goes up when the cursor is at the start of the query (always, when it's empty), so it still moves the cursor inside a query. Going up selects the item that led down and clears the query. At the root, `←`/`Backspace` do nothing (only `Esc` closes).
- **Judgement, prefixes:** only at the root and only as the first character; below the root they're ordinary characters (so `?` can be searched for in `Keys`). What follows the prefix becomes the category's query (`@bo` → `Chats`, query `bo`), so `:` + `appear` + `Enter` still reaches a setting group in a few keys. **`Themes` gets no prefix:** no remaining character suggests themes, and `th` + `Enter` at the root is as quick; the prefix hint stays short.
- **Judgement, settings tree:** `Settings ›` lists groups (`Appearance`: a `Theme ›` shortcut into the themes level, colours, emoji, message layout; `Notifications`: bell, window title; `Conversations`: allowlist mode) and `Mouse` on its own. Each setting is a level of its values drawn as radio items (`●` current, `○`), and the item opening it shows the current value on the right (`Bell ›   on`). Choosing a value applies it, saves it (unchanged, `settings::choose`) and closes. Groups with one setting (`Conversations`) are kept for the settings to come (hide behaviour, receipts).
- **Follow-up (2026-10-08):** the settings groups were flattened into one list under section headers, and on/off settings flip in place: D57 (supersedes the "settings tree" bullet below, except that multi-value settings still open their values).
- **Judgement, the rest:** `Commands` holds everything that was a command (curating the open conversation, mute durations, bulk hide, the selected message's actions, sidebars, keybindings, quit), the 3 most recent first; recency still only boosts commands. "Show keybindings" opens `Keys` under the root. Root items show their prefix on the right (`Themes` shows the theme in use). Nothing matching at the root says which prefixes exist. Footer: the root lists the prefixes; deeper levels show `← back`. Mouse unchanged: wheel moves, a click opens a category or runs a leaf, a click outside closes.

### D53. Do not disturb — Accepted (owner, 2026-10-08; details Judgement)

- **Decision (owner):** a global do-not-disturb toggle (a key plus a palette command) that turns the bell off and on, with a clear marker while it's off.
- **Judgement: it is the `[notifications] bell` setting**, written back to the config (`settings::toggle_bell` → `SettingChoice::Bell`), not a separate runtime flag. It persists across restarts, live-reloads, and stays one concept: "bell off" in the file, in `Settings › Notifications › Bell` and from the key are the same state, so they can't disagree. A runtime-only flag was rejected: two switches for one bell, and the marker would have to explain which one is off.
- **Key `Alt+N`** (`notifications.toggle_bell`, global): free in every context (the input uses `Alt+B/F/D` and `Alt+arrows`/`Alt+Enter/Backspace`; the sidebar and message pane use bare letters). Palette: `Commands › Turn do not disturb on (bell off)` / `… off (bell on)`, with the key shown.
- **Marker:** the footer's left side reads `● connected · bell off · 3 unread`, `bell off` in the warning colour. Plain text, not 🔕 (an emoji whose width differs between terminals and under mosh, D47) nor a rare symbol like `⦸` (missing from many fonts). The window title is unchanged (it's often shown elsewhere, D49).

### D54. OpenCode's themes, user theme files and a live preview — Accepted (owner, 2026-10-08; details Judgement)

- **Decision (owner):** bundle OpenCode's themes, load the user's own theme files (OpenCode's JSON format, D10) with hot reload, and preview themes live while moving through the picker.
- **Judgement, licence:** OpenCode is MIT ("Copyright (c) 2025 opencode"); its 33 theme files are copied unchanged into `crates/faro/themes/opencode/` with its `LICENSE`, and the notice is in `THIRD_PARTY_NOTICES.md` (MIT asks for the notice in copies; AGPL-3.0 can include MIT code). Compiled in with `include_str!` (about 260 KB of JSON), parsed on first use, not at startup unless the config names one.
- **Judgement, where code lives:** parsing (`faro-core::theme_file`: JSON → RGBA per role and variant, no terminal types, D20) is separate from mapping (`crates/faro/src/theme/import.rs`: RGBA → a `Theme` of ratatui colours). Resolution follows OpenCode's `resolveTheme`: a name is looked up in `defs`, then among the roles; a role may name a def of its own name (carbonfox does); cycles and unknown names are errors naming the role.
- **Judgement, variants:** each file yields its dark variant under the file's name and its light variant as `<name>-light` when the colours faro uses differ (aura and ayu have one). OpenCode picks the variant from the terminal's background; faro can't ask the terminal without racing the event reader (D51), so both are listed. Order in `Themes`: faro's own, the user's, OpenCode's dark, OpenCode's light; OpenCode's and the user's are labelled on the right.
- **Judgement, mapping:** roles with the same meaning map by name, `info` becomes `unread`. Only `background`, `text` and `primary` are required; the rest fall back to related roles (DESIGN §10). Opacity is composited over the background; a transparent background (lucent-orng) is drawn over black or white, since faro can't show the terminal's background behind a coloured UI. Surfaces equal to the one beneath (panel = background, selection = panel) are nudged towards the text so selections stay visible.
- **Judgement, readability:** imported themes keep their own colours and are **not** held to faro's contrast rules (D51): solarized's text is under 5:1 by design, and changing it would make it a different theme. Two exceptions: coloured text and muted text under **3:1** on the background are moved towards the text colour just enough (vesper-light's primary was 1.5:1, monokai-light's 1.6:1), and colours faro *derives* are made readable: others' bubbles sit between panel and selection, own bubbles are others' tinted with primary as far as text keeps 90% of its contrast, unread is info made ≥ 3:1 on background and selection, and the 8 sender colours come from the status, accent, syntax, markdown and diff colours (greys and near-duplicates skipped, each ≥ 3:1 on the background and others' bubbles, repeated when a theme has fewer: vesper has three). Tested per bundled theme.
- **Judgement, user files:** `themes/*.json` beside the config file (`~/.config/faro/themes/`; with `--config`, beside that file), created empty (`0700`) at startup so it's there to fill. Read synchronously at startup (before the config is applied, so `theme = "<mine>"` resolves), then re-read entirely whenever a `.json` there changes (a second `notify` watcher, same 300 ms debounce, `AppEvent::ThemesRead`; the reducer rebuilds them). A broken file is skipped and named in a warning (`Themes: broken.json: not valid JSON…`). A user theme replaces a built-in one of the same name (so tweaking `nord` means copying `nord.json`), except `system`, the 16-colour fallback, which is refused with a warning; an explicit `x-light.json` beats the light variant of `x.json`. The theme cache is keyed by name, colour depth and a generation counter of the user themes, so editing the theme in use redraws at once.
- **Judgement, unknown names:** the warning no longer lists every theme (there are 68 built in); it points to `Ctrl+P › Themes`.
- **Judgement, preview:** moving through the Themes level (keys, wheel, typing a filter) previews the selected theme: `Palette::preview`, drawn via `State::theme_name()`, never written to the config. `Enter` applies and saves (`settings::choose_theme`); `Esc`, `←`/`Backspace` up a level or a click outside drop the preview, so the configured theme returns without a "restore" step. Opening the level starts on the theme in use, so opening it changes nothing. Footer: `↑↓ preview · enter use it · ← back · esc cancel`.
- **Deferred:** a `mention` role (mentions arrive with M7); exporting faro's built-in themes as files; `NO_COLOR`.

### D55. The remaining emoji glitches were tmux joining what faro sends; a frame log to find them — Judgement

- **Symptom (owner):** stray characters and border pipes shifted by a cell, over both ssh and mosh; the mock's `🇷🇴 🇩🇪 🇯🇵 🏳️‍🌈 🏴‍☠️` message left a trail of tofu while scrolling.
- **Method:** a hidden frame log (`FARO_DEBUG_FRAMES=<file>`, `crates/faro/src/frames.rs`) writes every frame faro meant to draw (the same cell grid as the snapshots). `tools/frames/record.py` runs a scripted mock session in a private tmux, recording faro's raw output with `pipe-pane` next to the frame log; `tools/frames/replay.py` feeds the output back one frame (up to each `\x1b[?2026l`) at a time into a pane of the same size and compares `capture-pane` with the intended frame after each one: in that tmux, and with `--nested` also in an outer tmux attached to it (what a tmux client is sent and redraws, i.e. the owner's view). Before the fixes: a 2900-frame session diverged in 1176 frames (first at frame 84), a 2663-frame flood in 893 (first at 98), and nested, the outer view diverged in 341 of the first 1500 frames. tmux 3.6b's source (`screen-write.c`, `utf8.c`) explained each case.
- **Cause 1, tmux widening a cell.** tmux receives a flag or a VS16 emoji (❤️) one code point at a time: it stores the first, narrow code point, then widens the cell when the rest arrives (`screen_write_combine`, `variation-selector-always-wide`). Widening marks the next cell as padding **without clearing a wide character that started there**, whose own padding is left orphaned one cell further. The next write there erases the emoji (frame 84: `Lukas 🇩🇪` drawn over an old family emoji became `Lukas    `), and redraws from tmux's grid (attach, window switch, the outer client) show the damage. **Fix:** `tui/backend.rs` wraps crossterm's backend and writes a blank (same style) into the second cell before any such "widened" grapheme (several code points, a narrow first one, wide in total). Every terminal draws the same picture; tmux has nothing left to get wrong.
- **Cause 2, tmux joins onto the cell before the cursor, even after a cursor move.** It compares only the first code point of that cell: a regional indicator after a cell starting with one (even a complete flag) joins it, so two flags side by side become one cell (tmux width 2, faro 4: probe sample "two flags in a row"); anything after a cell ending with a ZWJ joins it. **Fix (every mode):** `emoji::display` puts a space between graphemes that start with a regional indicator, `emoji::finish` turns a flag that still follows one in the frame into its letters (🇩🇪 → DE), and graphemes that start with a joiner or modifier or end with a ZWJ lose those (`loose_decoration`).
- **Cause 3, skin tones on bases tmux doesn't join.** tmux 3.6 joins a skin tone only onto a fixed list of bases (`utf8_should_combine`); after any other (🫶🏽 🏃🏽 🤞🏽 ☝🏽) it's a separate 2-cell character, so tmux draws 4 cells where faro (and kitty) draw 2 (base-flood frame 98, a 🫶🏽 reaction). **Judgement:** faro drops skin tones after bases outside tmux's list, in every mode (`TMUX_SKIN_TONE_BASES`, copied from tmux 3.6). Outside tmux those emoji lose their tone too; the alternative (a per-terminal guess) can't know what's behind tmux, and a wrong guess breaks the row.
- **Cause 4, emoji tmux always draws two cells wide.** tmux 3.6 ships a width table (`utf8_default_width_cache`, the default of `codepoint-widths`) that makes ☝ ⛹ ✌ ✍ 🏋 🏌 🕴 🕵 🖐 two cells wide even without VS16; faro, kitty and mosh draw them one cell wide on their own. Found in a compatible-mode recording (frame 518: `❤ ☺ ✌` pushed the rest of the row one cell right), because compatible mode had stripped the VS16 from ✌️. **Fix:** in full mode these always get VS16 (two cells for faro, kitty and tmux alike; ☝🏽 → ☝️ when tmux can't join the tone). **Judgement, compatible mode:** mosh draws them one cell wide whatever follows, so no form of them is safe through mosh into tmux; they're drawn as a near equivalent instead: ☝ → 👆, ⛹ → 🏀, ✌ → ☮, ✍ → 📝, 🏋 → 💪, 🏌 → ⛳, 🕴 → 🕺, 🕵 → 🔍, 🖐 → ✋ (`TMUX_WIDE`). Copy still gives the original.
- **Cause 5, ZWJ sequences without VS16.** tmux sizes a joined cell by its first code point (two cells once VS16 joins it), while faro (unicode-width) adds up the parts of a sequence that isn't fully qualified: 🧗‍♀ is 3 cells for faro and 2 for tmux, ❤‍🔥 and 🏳‍🌈 3 and 1, 👁‍🗨 2 and 1; fully qualified (🧗‍♀️, ❤️‍🔥, 🏳️‍🌈, 👁️‍🗨️) both say 2 (probed with `#{cursor_x}`). Found by a later recording (frame 1149 of 2699: the input showed `🧗‍♀` typed but its VS16 not yet). **Fix:** full mode adds VS16 after every one-cell code point in a ZWJ sequence (`wants_vs16`); compatible mode splits every ZWJ sequence into its parts, as it already did for the fully qualified ones.
- **mosh and flags. Judgement:** mosh stores a flag's two halves in separate cells and can redraw one without the other (tofu); compatible mode now draws flags as their letters (🇷🇴 → RO), like the half flags of D47.
- **Not the cause:** the input editor (its cursor maths uses ratatui's per-grapheme widths; `finish` stand-ins keep widths), ratatui (never splits graphemes; its diff already resends the trailing cell when a wide character becomes narrow). tmux 3.6b ignores the application's synchronized-output mode and forwards cells as they come, so half-drawn frames are possible but are corrected by the end of the frame.
- **Result:** with all the fixes, 20,122 recorded frames (six sessions: mixed use, a flood at 15 ms per event, typing, conversation switching, and in compatible mode mixed use and typing in a 97-column pane next to another pane) replay with **0 divergent frames**, both directly and nested (inner and outer). The typing sessions include bare ✌, ☝🏽 and 🫶🏽 in the (unsanitised) input. Cause 5 came later; with its fix, two more sessions (2669 frames of mixed use, 4966 of typing, both with rounded bubbles) replay with 0 divergent frames too. New `tools/emoji_widths.py` samples: two flags in a row, the England tag flag, 🫶🏽 🏃🏽 🤞🏽 ☝🏽 🧑🏽‍💻.
- **Mosh detection (D47) checked against the owner's session:** the tmux session `faro` had three clients, two through `mosh-server` (one probably stale) and one over ssh, and 13 `mosh-server` processes were lingering. Detection is right per D47 (one mosh client is enough to break that client's view), so faro stays in compatible mode even while the owner looks through ssh until the stale mosh clients go away (`tmux detach-client`, or kill old `mosh-server`s).

### D56. Rounded bubble corners — Accepted (owner, 2026-10-08; details Judgement)

- **Superseded by D58** (same day): the half-block edges looked like a rectangle with smaller rectangles cut out of its corners; `bubble_corners` became `bubble_style`.

- **Decision (owner):** bubbles get rounded corners: a top edge row `▗▄▄…▄▖` and a bottom edge row `▝▀▀…▀▘` in the bubble's colour on the pane's background, as a setting `[messages] bubble_corners = "rounded" | "square"`, rounded by default.
- **Judgement, no blank line between rounded bubbles:** the bottom edge colours the upper half of its row and the next top edge the lower half of its row, so two bubbles in a row are already a full row apart (half + half), the same gap as the blank line square bubbles keep. Keeping the blank as well would leave two rows between bubbles and waste height. Square bubbles (and the `system` theme) keep the blank line. Rounded bubbles still take one more row per message than square ones (two edges instead of one blank); that's the price of the look.
- **Judgement, layout:** the edges span the bubble's padding cells (`▗`/`▝` above and below the left padding, `▖`/`▘` above and below the right). The sender's name stays above the top edge, reactions below the bottom edge. Edge rows belong to the message (after the rules in `Block`), so a click on an edge selects it and windowing/scroll anchoring count them like any other row.
- **Judgement, selection:** the selection tint replaces the bubble's fill, so the edges would float as stripes in the tinted rows; the tint blanks edge glyphs on the pane's background (`conversation::blank_bubble_edges`), leaving a clean tinted block as with square bubbles.
- **Judgement, `system` theme:** it draws bubbles without a fill (D51), so there's nothing to round: always square, whatever the setting.
- **Widths:** `▗▄▖▝▀▘` are single code points, one cell in faro, tmux and mosh (East Asian Ambiguous like the box-drawing borders faro already draws, so a terminal set to draw ambiguous characters wide breaks both alike); nothing for compatible mode to change. Replays of rounded-bubble sessions show no divergent frames once D55's cause 5 was fixed (it was found by the first of them, in the input): 2221 frames in compatible mode, then 2669 of mixed use and 4966 of typing in full mode, directly and nested.
- **Palette:** `Settings › Appearance › Bubble corners` (rounded/square), next to the message layout; choosing saves it (D48), and editing the file applies it live.

### D57. One settings list, and what the colour depth does to themes — Accepted (owner, 2026-10-08; details Judgement)

- **Decision (owner):** `Settings ›` shows **every setting in one list** under non-selectable **section headers**, each row with its current value. Typing filters across all settings and keeps a header only for sections with matches. `Enter` on an on/off setting **flips it in place** (the palette stays open, the row updates); on a multi-value setting it opens its values (radio items, current marked). `Theme ›` keeps the live-preview picker. Supersedes D52's settings tree (groups as levels) and its "choosing a value closes" for on/off settings.
- **Decision (owner):** the Themes level explains the colour depth: at 256 colours a muted note that themes are approximated (and where to switch to truecolor), at 16 that the `system` theme is drawn and other themes need 256 colours or more. README and DESIGN explain 16 → `system` and 256 → approximation, with solarized as the example.
- **Judgement, sections:** `Appearance` (Theme ›, Colours, Emoji, Message layout, Bubble corners, renamed Bubble style by D58), `Notifications` (Bell, Unread count in the window title), `Conversations` (Allowlist mode), `Input` (Mouse; D52 left it on its own, and a header with nothing but a lone setting under no section read oddly). The order of sections and settings is `SettingsGroup::ALL` / `SettingsGroup::settings`.
- **Judgement, filtering:** a list with headers keeps **item order** instead of ranking by score (`palette::grouped`); the best-scoring match is selected, so `Enter` still takes the best match. A header's title counts as a match for every setting under it (`notif` lists both notification settings, `appear` the whole section), so the old `:appear` + `Enter` habit still narrows to that section. Levels without headers rank as before (`palette::ranked`).
- **Judgement, headers in navigation:** `ItemKind::Header` items are never selected: single steps (`↑`/`↓`, the wheel) jump over them and wrap; a page that lands on one takes the nearest setting onwards, else backwards; a click on one does nothing.
- **Judgement, switches:** Bell, the window-title count, Allowlist mode and Mouse are `ItemKind::Switch` (`Setting::is_switch`). `Enter` (or a click) applies the other value through the same `settings::choose` (so the notice, the config write and live effects are unchanged), then rebuilds the level with the query, the scroll position and the selection kept. `→` doesn't flip (it only opens levels, D52). Switches show `on`/`off` without `›`. The footer at Settings reads `enter flip/open`.
- **Judgement, the note** is a field of the palette (`Palette::note`, built by `palette::note(state, level)` on every level change), drawn as a muted line between the rule and the list, not an item: it can't be selected or filtered away, and "No matches" still shows under it. Truecolor gets no note. The depth is the effective one (`State::colors()`: the `[ui] colors` override, else what was detected).
- **Judgement, choosing a theme at 16 colours** keeps D51's behaviour: the name is saved to `theme` and `Theme::for_depth(Ansi16)` draws `system` until the depth allows more (a better terminal, or `Settings › Colours`); the notice says so ("Theme: nord (drawn as system until 256 colours or more)"). Refusing the choice was rejected: the config is shared between terminals (the same file over SSH from kitty and from a 16-colour console), and the choice is right for the others. The `Colours` row shows the detection for `auto` ("auto (256 colours detected)"), as since D51.
- **Example checked (test `theme::tests::solarized_turns_grey_in_256_colours`):** at 256 colours solarized's background `#002b36` becomes xterm 235 (`#262626`): there's no dark blue-teal among the 240 fixed colours, so the nearest is a neutral grey. Solarized's own 256-colour fallback uses 234 (`#1c1c1c`), one step darker, for the same reason.

### D58. Bubble styles: rounded where the terminal draws the glyphs, bars elsewhere — Accepted (owner, 2026-10-08; details Judgement)

- **Decision (owner):** D56's half-block edges looked like "a rectangle with smaller rectangles cut out of the corners". The owner tried candidate glyphs in kitty and chose:
  - **rounded, one-line bubbles:** Powerline half-circle caps U+E0B6 (left) and U+E0B4 (right) in place of the padding cells, in the bubble's colour on the pane's background, the row between them filled;
  - **rounded, taller bubbles:** edge rows two thirds of a row high: top `🭊` U+1FB4A + `🬹` BLOCK SEXTANT-3456 (U+1FB39) × n + `🬿` U+1FB3F, bottom `🭥` U+1FB65 + `🬎` BLOCK SEXTANT-1234 (U+1FB0E) × n + `🭚` U+1FB5A, in the bubble's colour on the pane's background;
  - **bar** (replaces square), after OpenCode's message boxes: the filled box with a heavy `┃` (U+2503) on the bubble's **outer** side (left for others, right for own) in place of that side's padding cell; others' bars in the sender's name colour (in a direct chat that's the person's), own bars in the theme's primary; no edges;
  - the `system` theme (no fill) always draws bars;
  - `[messages] bubble_style = "auto" | "rounded" | "bar"`, default `auto`: rounded only where the terminal is known to draw these glyphs itself, else bars.
- **Widths, probed before building:** in tmux 3.6b (a private server, and an outer tmux attached to it) each of U+E0B4, U+E0B6, U+1FB4A, U+1FB3F, U+1FB65, U+1FB5A, U+1FB39, U+1FB0E, U+2503, U+2579 moves the cursor one cell (`#{cursor_x}`), directly and nested, and the outer pane holds the same bytes; unicode-width 0.2 and glibc 2.43's `wcwidth` (what mosh uses) say 1 for each. So nothing for the emoji code (D47, D55) or compatible mode to change.
- **`auto`, who draws the glyphs:** kitty draws box drawing and block elements (U+2500–259F), Powerline (U+E0B0–E0BF) and Legacy Computing (U+1FB00–1FBAE) itself, pixel-exact, whatever the font (`font_for_cell` in kitty's `fonts.c` returns its `BOX_FONT` for those ranges). Elsewhere they come from the font: Powerline glyphs only from a Nerd/Powerline font, Legacy Computing only from fonts with Unicode 13 symbols, so tofu is likely. `system::inspect_terminal` (D47's check, extended): outside tmux, kitty if `TERM` contains `kitty` (forwarded over SSH), `TERM_PROGRAM=kitty` or `KITTY_WINDOW_ID` is set; inside tmux, `tmux list-clients -F "#{client_pid} #{client_termname}"` for faro's session, kitty only if **every** client's terminal is (one other client would see tofu). The answer is `Environment::glyphs` (`BlockGlyphs::Native`/`Font`), found at startup (when emoji or bubbles are `auto`) and again on focus and resize (D47's throttled check, now also run when only the bubble style is `auto`), and choosing `auto` checks at once.
- **Judgement, mosh:** under mosh the tmux client's `TERM` is what `mosh-server` sets (`xterm-256color`), not kitty, so `auto` draws bars there even when kitty is at the end. That's safe rather than right; `bubble_style = "rounded"` is the way to have them through mosh (mosh measures them one cell like everyone else).
- **Judgement, the old setting:** `bubble_corners` was public for a few hours. It's still read when `bubble_style` isn't set ("rounded" → rounded, "square" → bar) with a footer warning naming the new key and value; if both are set, `bubble_style` wins and the old key is reported as ignored. The file isn't rewritten: faro edits the config only for a setting the user changes, and the warning says exactly what to write. Generic (`config::RENAMED`), so later renames reuse it.
- **Judgement, blank lines:** a blank line separates bubbles again in every style. One-line rounded bubbles have no edges, so two in a row would touch; the two-thirds edges of taller ones leave a third of a row each, which made gaps uneven without the blank line. Cost: a taller rounded bubble takes two rows more than its text; one-line bubbles (most messages) cost no more than bars.
- **Judgement, the bar's cell:** the bar is drawn on the bubble's fill (not the pane's background), so the box stays a full rectangle with the bar as its edge and the text starts in the next cell. OpenCode's prompt also ends its bar with `╹` over a `▀` half row; faro's bubbles don't (they'd take one more row each), so ╹ is probed but unused.
- **Kept:** edge rows belong to their message (clicks, selection, windowing and scroll anchoring); the selection tint blanks caps and edges on the pane's background (`conversation::blank_bubble_edges`, now with the new glyphs), so a selected bubble is a clean tinted block in both styles, the bar keeping its colour on the tint; the sender's name above, reactions below (under the bottom edge).
- **Palette:** `Settings › Bubble style` (auto/rounded/bar) in the Appearance section; choosing saves it (D48), editing the file applies it live.
- **Replays:** `tools/frames` (D55), 9,208 frames, 0 divergent, direct and nested: a mixed session in rounded (seed 41, 2,781 frames), a mixed session in bar (seed 43, 2,609), and a typing session in rounded with compatible emoji (seed 45, 3,818). The recordings were checked to contain caps and sextants (rounded) or bars (bar). Every glyph is one cell in tmux 3.6b (direct and nested), unicode-width 0.2 and glibc `wcwidth`.

### D59. Visual settings preview like themes — Accepted (owner, 2026-10-08; details Judgement)

- **Decision:** the owner asked for a live preview of the bubble style, or else `Space` to apply a value without closing the palette and `Enter` to apply and close, whichever is better. Live preview, for every setting whose values only change how things are drawn: **Colours, Message layout, Bubble style**. Moving through a setting's values draws the selected one at once, `Enter` applies, saves and closes, `Esc` (or going back up) puts the value in use back, unsaved. It's the theme picker's behaviour (D54), so one rule covers every visual choice.
- **Judgement, why not `Space`:** a second "apply but stay" key would be the only one of its kind in the palette, `Space` is also a character the query needs (`Space` in the filter), and it would save the file on each try. The preview shows the same thing without writing anything, and it's what the theme list already taught.
- **How:** `Palette::preview` is a `Preview` (`Theme(name)` or `Setting(SettingChoice)`), set by `Palette::sync_preview` at the Themes level and at `Setting` levels where `Setting::previews()`. The renderer never reads the config for these directly: `State::theme_name()`, `State::colors()`, `State::message_layout()` and `State::bubble_style()` return the previewed value while the palette is open, else the one in use. Values levels open on the value in use (the item marked current), so opening one changes nothing.
- **Judgement, not previewed:** Emoji (switching modes repaints the whole screen, and `auto` waits for a look at the terminal) and the on/off settings (they flip in place in the settings list, D57; mouse, bell and title aren't drawn in the message view anyway).
- **Known limit:** previewing bubble style `auto` uses the last look at the terminal (`Environment::glyphs`). If neither the emoji mode nor the bubble style was `auto` at startup, that look hasn't happened, so the preview shows bars; choosing `auto` checks the terminal and the next frame is right.

### D60. Themes listed once; no chat open by default — Accepted (owner, 2026-10-08; details Judgement)

- **Themes once (owner):** the palette listed themes twice: the root's `Themes ›` and a `Theme ›` row in `Settings › Appearance`. Only the root category stays: 68 themes are the palette's biggest list to browse, and going straight to Themes from `Ctrl+P` was the owner's idea of the tree (D52). `:theme` finds nothing in the settings now; the root's Themes row shows the theme in use.
- **No chat open (owner):** faro starts with no conversation open, and the message pane shows the logo (D61). Nothing opens by itself, also not when the first message arrives on a fresh start (the old "open where things happen" rule is gone), so nothing is read until the user opens it, like closing a chat in other apps.
- **Esc, one step at a time (owner's choice):** cancel the reply being written; else clear the notice and go back to the input (from the list or the messages); else close the open conversation. Closing saves the draft and stops typing (`curation::close`, also used when the last conversation leaves the list), shows the list if it was hidden and gives it the focus. With nothing open, Esc only clears a notice.
- **Judgement, the highlight is a cursor while nothing is open:** before, the sidebar's highlight *was* the open conversation and moving it opened the next one. Opening reads, so moving through the list with nothing open now moves only a cursor (`ConversationList::cursor`; `State::highlighted()` = the open conversation, else the cursor if it's listed, else the first listed). `j`/`k`, `g`/`G` and `Alt+J/K` move it (focusing the list); `Enter` (`Action::FocusInput`, footer "open") or a click opens it; the curation keys and the conversation menu act on it. It's drawn with the selection background but without the open conversation's bold primary name. With a conversation open, moving still switches conversations, as before (D36, D37).
- **Judgement, startup:** the cursor starts on the conversation open when faro last quit (`ui.last_open_conversation`, still written on every open), in the list it's in, with the list focused; without one, on the first listed.
- **Kept:** read receipts and unread counts already depended on a conversation being open and seen (D42), so with nothing open every incoming message stays unread, rings the bell if its conversation notifies (D49) and counts in the title.

### D61. The logo screen while no chat is open — Accepted (owner, 2026-10-08; details Judgement)

- **Decision (owner):** with no conversation open (D60) the message pane shows the large animated lighthouse from the naming phase (D12, `tools/logos.py`, the "faro" entry), ported to Rust; when the pane is too small for it, a small lighthouse drawn with block characters in the theme's colours with the word faro beside or under it, plus a few muted hints. Opening a chat replaces it.
- **How:** `ui/logo/art.rs` ports the prototype's `Scene` (half-block pixels, two per cell, over braille dots, 2×4 per cell), the 40×13 lighthouse (red and white tower, lamp glowing with the beam, a fan of braille beam lines whose length follows the turn, waves, 14 twinkling stars), the 5×7 wordmark font (F A R O) with its gradient, and the tagline. `ui/logo.rs` picks the form and draws it centred in the pane on the pane's background (a cell with one half-pixel is `▀`/`▄` over the background; two make `█` or `▀` with fg on top, bg below). Stars come from a fixed SplitMix64 seed instead of Python's `random` (positions differ, the look doesn't).
- **Judgement, forms by pane size** (inside the border; the notes below need a blank line plus one row each):
  1. **Full:** lighthouse, wordmark at scale 2 and tagline side by side, 90 × 13.
  2. **Stacked:** lighthouse above the wordmark at scale 2 (46 wide, 23 tall with the tagline) or scale 1 (40 wide, 20 tall; 18 without the tagline, which is left out where it doesn't fit its 45 columns).
  3. **Small:** a 9 × 8 lighthouse in the theme's colours (red stripes and roof = `error`, white stripes = `text`, lamp = `warning`, gallery and rocks = `border`), the word in bold `primary` beside it at mid-height with the tagline under it if it fits (else the word under the lighthouse). Every cell is one colour over the background, so the terminal's default colours work too.
  4. **Word:** just "faro".
  The largest form that fits with its notes wins; a short pane drops the last hint or two before giving up a form (down to the status and two hints); the word alone drops notes until it fits.
- **Judgement, notes under every form:** a status line when there's one ("Loading…", the old empty-pane text, or "No conversations yet"), then hints from the key map (rebinding shows), keys right-aligned in a column: `alt+j/k` choose a chat, `enter` open it (", or click one" with the mouse on; only while the list is shown), `ctrl+p` command palette, the help key (`?` with the list focused, `f1` otherwise) keybindings. The chat hints are left out while loading or with no conversations.
- **Judgement, animation:** the beam turns, the lamp glows, waves roll and stars twinkle as in the prototype, at 10 fps (`logo::FRAME`), only while the logo is on screen, the terminal has focus and `reduce_motion` is off (`State::logo_moves`), and only for the two big forms (`Geometry::logo_animated`, reported by the renderer). The app loop schedules the next frame then and only then (`app::logo_frame`), so a still logo costs no wakeups. Time comes from the app loop (`View::logo_time`, counted from when it started moving, starting at the still frame `t = 0.35`), so the renderer stays a pure function and tests are deterministic. Otherwise the still frame is drawn.
- **Judgement, colours:** truecolor draws the logo's own colours (it's the brand); at 256 colours each is mapped to the nearest xterm colour with the themes' mapping (`theme::nearest_256`); at 16 colours and with the `system` theme (the terminal's own palette, whose values faro can't see) only the small form or the word are drawn, in theme colours. Glows (lamp, beam, stars) fade towards the pane's background rather than black, which on dark themes is what the prototype did. **On light backgrounds** (luminance over 0.4) the stars are skipped (pale dots vanish on a light sky), the white stripes become a mid grey (172, 176, 188), the lamp and beam amber (236, 160, 40) and the wordmark's pale yellow start a darker amber; the red, the waves, the gallery and the rocks stay. The tagline uses the theme's muted text, like the hints.
- **`reduce_motion`:** the config key (parsed since M5) now stops the animation; it's an on/off switch in `Settings › Appearance` ("Reduce motion"), saved like every palette setting and live on reload (the app loop reads it from `State::config`).
- **Rendering safety:** only `▀ ▄ █`, braille (U+2801–28FF) and plain text are drawn: one cell in unicode-width and so in tmux and mosh (`logo::tests::only_single_cell_symbols`). Block elements are East Asian *ambiguous* width, as everywhere else faro uses them; terminals set to draw ambiguous characters wide are out of scope.
- **Measured:** release build, mock with no activity (`--mock-activity 3600000`; `0` is a flood), 160×40 tmux pane: animating 1.6 % of a core and about 19 wakeups/s; still (`reduce_motion`) and with a chat open 0 % and 0 wakeups/s. Replays (`tools/frames`, `logo` scenario: idle spells, resizes through the forms, opening and closing a chat, the palette over it): 923 frames at 110×32 (seed 61) and 449 beside a 40-column pane (130 wide, seed 63, `--steady`), 0 divergent, direct and nested. With resizes, split recordings replay with 0 divergent direct, but nested replays drift after a pane resize because the outer client's size wins (a replay-tool limit, hence `--steady`).
- **Not now:** an about screen with the logo (PLAN "Later"); a splash at startup (the empty pane is the splash now).

### D62. An allowlist guard for testing with a real account — Accepted (owner, 2026-10-08; details Judgement)

- **Why:** M7 is tested on a real Signal account. Other people may message it. faro, and whoever develops it, must never send anything to anyone else and never see or store anyone else's messages.
- **What:** `faro-core::guard`. `Guarded<B>` wraps any `Backend`, and a forward task filters its events. `--allowlist FILE` (`FARO_ALLOWLIST`) turns it on. The file is TOML with `people` (ACI UUIDs) and `groups` (64-hex group IDs). Parsing is strict: unknown keys, non-lists and bad IDs stop faro, and so does a missing file. It never falls back to "no guard".
  - **Out:** send, react, edit, delete and typing to a conversation that isn't allowed fail with `BackendError::NotAllowed` (not transient, so not retried). Read receipts go only to allowed people and to known members of allowed groups.
  - **In:** messages, edits, deletions, reactions and typing about other conversations are dropped before the app sees them. So are contacts other than allowed people and members of allowed groups, and groups not on the list. Delivery updates, connection and error events pass (they carry no one else's content). Only counts are logged.
- **Presage backend (M7):** while testing, it refuses to start without `--allowlist`. This is lifted (or becomes an explicit opt-out) for the v0.1 release.
- **Known gap to close in M7:** the guard works at the `Backend` trait. presage and libsignal-service may send protocol-level messages on their own, below the trait: delivery receipts on receive, retry receipts, profile keys, sync messages to our own devices. Check each one and suppress or filter it inside `faro-presage` where possible. Record here what can't be suppressed.
- **presage's own store (owner, 2026-10-08):** presage saves every incoming message, and fetches and saves every sender's profile, in its own database, whoever the sender is. It sends nothing to anyone but our own devices on its own: no delivery or retry receipts; libsignal-service's `ServiceCipher` has no way to send. The owner accepted presage keeping what it receives, since nobody uses this account. No filtering store wrapper; the guard's job is that nothing is ever **sent** off the list, and that faro never shows or stores anyone else.
- **Own read syncs (D71, 2026-10-08):** when faro sends read receipts it also sends a read sync to this account's own ACI, so only our other devices get it; it lists only messages the guard let through.
- **Process (owner's agreement):** separate data dir for the test account; the owner scans the QR code; Signal's Contacts permission is denied on the phone; the device is unlinked after testing.

### D63. The presage backend: linking, IDs, mapping — Judgement (2026-10-08)

- **Crate:** `faro-presage`. It depends on presage and presage-store-sqlite at a pinned git revision. The workspace patches `libsqlite3-sys` to whisperfish's SQLCipher build, as presage does; faro's own sqlx database links the same library. `.cargo/config.toml` sets `git-fetch-with-cli` so the user's git URL rewrites apply. The build needs `protoc` (CI installs it).
- **Store:** presage's database is `<data dir>/presage.db`, created `0600` (it holds the account's keys), with no passphrase yet (D18). New identity keys are trusted (`OnNewIdentity::Trust`), as Signal's apps do.
- **Linking is a subcommand, not a TUI screen:** `faro link [--device-name]` prints a half-block QR code (RGB black and white where the terminal shows truecolor, since themes can redefine every palette entry; basic ANSI black and white elsewhere; quiet zone) on stdout and waits. The default device name is `faro (<short hostname>)`. If the data dir is already linked, it refuses and says how to start over. Simpler than a TUI screen, and it works the same over SSH.
- **`faro ids [--username …]`** prints this account's ACI, each username's ACI (a server lookup) and the groups presage knows (title and ID, never members or messages), followed by a ready-to-edit allowlist. presage only learns groups from group messages, so `ids` first catches up with queued messages (up to 30 s). faro doesn't keep those messages.
- **Backend selection:** `--mock` gives the mock; otherwise the linked account, which refuses to start without `--allowlist` (D62) and says to run `faro link` if nothing is linked.
- **Group IDs:** faro's `GroupId` is the public group identifier derived from the master key (zkgroup), never the key itself. The backend keeps the master keys (and revisions) it has seen, to send.
- **Mapping (`convert.rs`, pure, tested):** data messages (text, quotes, attachments as references, stickers as "[sticker] 🙂") → `Message`. Reactions, deletions and edits map to their events, including those sent from the phone (sync "sent" transcripts become outgoing messages). Delivery and read receipts → `DeliveryUpdate`, typing → `Typing`. Profile-key, timer and group-change messages show nothing. Message authors given as 16-byte UUIDs are understood.
- **Events the app needs that presage doesn't push:** a group is announced (`GroupUpdated`) the first time it's seen and when its revision grows. A sender is announced (`ContactUpdated`) the first time this run, and again 5 s later, once presage has fetched their profile. Missing profiles are fetched in the background, so startup never waits for the network.
- **Reconnects:** the receive loop reports `Connecting` / `Connected` / `Disconnected` and retries with backoff from 1 s to 60 s. An authorisation failure means the device was unlinked: it reports `Unlinked` and stops.
- **Capabilities:** edit and delete-for-everyone; attachments aren't downloaded yet.
- **Not yet:** read syncs from the phone clearing unread counts (D42; needs a new event), delivery receipts from faro (presage sends none; the guard would have to cover them), syncing faro's own read receipts to the phone, removing a reaction with its emoji (the trait doesn't pass it; Signal clients may want it), attachments, history transfer (D19).

### D64. Catching up from presage's store, and profile keys from groups — Judgement (2026-10-08)

- **Why:** on the test account the sidebar stayed empty and contacts showed as the start of their ID. (1) presage saves every message it receives or sends, but faro only stored what arrived while the TUI ran; `faro ids` reads the queue and drops what it reads (D63), so the owner's first group message was in presage's database and not in faro's. (2) presage only learns profile keys from the phone's contact sync (empty: Contacts permission denied) and, in a background task, from incoming messages (cut short when `faro ids` exits), so no profile names could be fetched.
- **Catching up:** a new `Backend::stored_events(conversations)` (default: none; the mock keeps nothing) returns what the backend kept about those conversations, oldest first. `app::load_directory` asks for every known group and contact before the first draw and applies the events to the store, which ignores what it already has (messages by sender and time, statuses never move backwards, edits only forward). The reducer never sees them, so nothing rings or counts twice.
  - The guard asks the inner backend only about allowed conversations and filters the result like live events (`guard::tests::stored_events_are_asked_and_filtered_like_live_ones`).
  - presage keeps what this device sent as plain contents from us to `metadata.destination`: `convert::stored` maps those as outgoing; receipts and typing we sent are skipped; only events the store keeps are returned.
  - **Limits:** every start re-reads every allowed thread (fine for a test account; a watermark can come later). A deletion that arrived while faro wasn't running isn't seen: presage replaces the target with an empty message and doesn't keep the deletion.
- **Profile keys:** a group's member list carries every member's profile key. `PresageBackend::save_profile_key` stores them in presage (when missing or changed) whenever groups are read or a group changes; the existing background refresh then fetches the profiles, and `ContactUpdated` brings the names. Usernames can't be looked up from an ID, and phone numbers are hidden unless shared, so profile names are what faro shows.
- **Already there:** the palette's `Chats ›` lists every known conversation, empty ones included, and opens them; empty chats stay out of the sidebar (DESIGN §7.7) until they have a message or are open.

### D65. Telling same-name people apart, and nicknames — Accepted (owner, 2026-10-08)

- **Why:** on the test account two people with the same profile name looked identical everywhere (the palette's `Chats ›`, the sidebar, message senders). Usernames can't be looked up from an ID and phone numbers are usually hidden (D64), so faro has nothing else to show by default. The owner chose both of the options below.
- **Short tag on a clash:** when two or more people faro knows (contacts and people with a direct chat, not this account) have the same name, compared case-insensitively, each gets ` · ` and the first 4 hex digits of their Signal ID after it ("Alex · 3f9a"). Nobody else gets a tag. `State::recount_names` finds the clashes again whenever contacts, conversations or nicknames change; `State::person_name` adds the tag, and `conversation_title` uses `person_name` for direct chats so every place agrees.
- **Nicknames:** a name the user gives someone, kept only in faro (`recipients.nickname`, migration `0004_nicknames.sql`), never sent or synced. It replaces their profile or address-book name everywhere and takes part in clash detection (giving one of two people with the same name a nickname ends the clash). Ways in: the conversation menu (`Space` / right-click on a direct chat) › "Set a nickname…", the open chat's entry in `Commands ›`, and `Commands ›` › "Set a nickname…" (lists everyone faro knows). At the `Nickname` level the palette's query is the name, so typing doesn't filter. `Enter` saves it (trimmed); an empty name, or "Remove the nickname", takes it away. The query starts from the current nickname.
- **Judgement:** 4 hex digits (one in 65,536 chance that two clashing people share them; enough for a handful of people); the tag only on a clash, so names stay clean otherwise.

### D66. A mixed bubble style — Accepted (owner, 2026-10-08)

- `[messages] bubble_style = "mixed"` (and `Settings › Bubble style › mixed`): the bar style's `┃` on the bubble's outer side (own: right, theme primary; others: left, sender's colour), and the rounded style on the inner side (a Powerline half circle on one-line bubbles; on taller ones the edge rows' diagonal corner). On the bar's side the edge rows end square (their fill glyph), and the bar runs down the text rows only.
- Like `rounded`, it's an explicit choice: drawn whatever the terminal (no kitty check; `auto` still picks only rounded or bar). The `system` theme still always draws bars. Selection blanks the edges as in the rounded style.

### D67. `--no-allowlist` for the owner's own account — Accepted (owner, 2026-10-08)

- **Why:** the owner wants to use faro day to day on their personal account (on the Mac mini), where the D62 safeguards aren't wanted.
- `faro --no-allowlist` runs the Signal backend without the guard: faro talks to everyone, like any Signal client. It must be typed every time: no environment variable or config key, and it can't be combined with `--allowlist`. Without either flag the Signal backend still refuses to start, and the error names both.
- faro logs a warning at startup, and the footer shows a muted `no allowlist` after the connection status.
- The test account keeps running with `--allowlist` as before (D62); its data dir stays separate.

### D68. Ask the phone for contacts; list new people and groups at once — Accepted (owner, 2026-10-08)

- **Why:** on the owner's personal account, freshly linked, faro started with `contacts=0 groups=0` and an empty Chats list. presage only requests a contact sync when asked, and faro never asked. Contacts or groups that arrived while faro ran were stored but only listed after a restart.
- **Contact sync:** the presage backend asks the phone for its contacts once per run, after the first catch-up with queued messages (`PresageBackend::request_contacts`, in a background task). Signal's desktop apps do the same after linking. The request is a sync message to our own devices only (D62). The phone's answer arrives as `Received::Contacts`, and each contact is announced as `ContactUpdated`. Behind the guard, contacts off the list are dropped as before.
- **Listed at once:** `ContactUpdated` and `GroupUpdated` for a conversation the app doesn't have add it to `State::conversations` (as the store already does), and the list is sorted again. Empty conversations still stay out of the sidebar (DESIGN §7.7); the palette's `Chats ›` lists them.
- **Seen on the owner's iPhone:** the phone answers only once Signal is opened on it (iOS doesn't run it in the background); the contacts then arrive within a second and are listed at once.
- **Still missing:** the full group list. presage learns a group only from a message in it; Signal keeps the list in its storage service, which presage doesn't read.

### D69. Media gets milestones in v0.2: M9 attachments and images, M10 voice notes and transcription — Accepted (owner, 2026-10-08)

- **Why:** the owner wants inline images and voice notes scheduled, not left in the "Later" backlog.
- **M9** bundles attachments (receive, open/save, send) with inline images (kitty graphics, sixel, half-block fallback; tmux passthrough; half-blocks through mosh). **M10** adds voice note playback through an external player, plus **opt-in transcription** (the owner's idea): a local Whisper model or a remote OpenAI-compatible endpoint, off by default, with explicit consent for the remote kind since it sends other people's voice to a third party.
- Both come after v0.1 (M8) and before the rest of v0.2. Their designs are done when each starts; PLAN.md has the outlines. Opening and playing over SSH/mosh happen on the remote machine, so both milestones offer save/copy (and, for images, inline display) there.
- Still in "Later": recording and sending voice notes. Calls stay out of scope.

### D70. Reads on the phone clear unread counts here — Judgement (2026-10-08)

- **Why:** D42 and D63 left it for M7: a chat read on the phone stayed unread in faro.
- **Event:** `BackendEvent::ReadElsewhere(Vec<MessageRef>)`, from the `read` entries of a `SyncMessage` (sender ACI, string or 16 bytes, and the message's timestamp; entries missing either are skipped). No conversation: presage only gives one by lookup, and faro's store can find it from the message.
- **Store:** `Store::read_elsewhere` (also run when the event is applied while catching up, D64): for each conversation, the newest message read (by `(sent_at, sender)`, the store's order) means everything up to it was read, as in Signal, so the unread count drops to at most the number of incoming, non-deleted messages after it. Counts never go up; messages faro doesn't have are ignored. It returns how much each count dropped.
- **Reducer:** the event becomes `Effect::ReadElsewhere`; the store's answer, `AppEvent::UnreadCleared`, lowers each summary's count by **how much** it dropped, not to a value, so a message that arrives meanwhile still counts and a chat opened meanwhile stays at zero. In the open conversation, messages up to the newest one read leave `unreceipted` (the phone already sent their receipts), and the "new messages" rule goes if it was above them: they aren't new to the user any more. The title and footer follow the counts.
- **Guard:** read syncs carry no content and only point at messages; the store ignores messages it doesn't have and never stored anyone else's, so they can't reveal or change anything about others. Still, `Shared::admit` keeps only pointers to messages from listed people and members of allowed groups (dropping the event if none are left), so other people's IDs never reach the app. `stored_events` filters the same way.
- **Mock:** an occasional "read on the phone" of a conversation's newest incoming message (`Activity::ReadElsewhere`).
- **Limits:** a read sync that arrives before the message it points at (possible after a long time offline) is ignored; the message then counts as unread until read here. `viewed` syncs (view-once media, voice notes) aren't mapped. This closes D42's and D63's "read syncs" item.

### D71. faro's reads are synced to the phone — Judgement (2026-10-08)

- **Why:** D63's "not yet": a chat read in faro stayed unread on the phone.
- **What:** `PresageBackend::mark_read` sends, after the read receipts, one `SyncMessage` whose `read` entries name each message read (sender ACI as a string, which every Signal client parses, and the message's timestamp), to this account's own ACI, as presage's `request_contacts` does; libsignal-service delivers it to our other devices only. Padded like presage's own sync messages (`SyncMessage::with_padding`, hence the `rand` 0.9 dependency, the version libsignal-service uses). Built by `convert::read_sync` (tested); our own messages and PNI senders are left out.
- **Errors:** each receipt and the sync are tried even if an earlier one failed; the first error is returned (and only logged, D42).
- **Guard (D62):** it only goes to our own devices, and lists only messages the guard passed: `Guarded::mark_read` drops receipts for people off the list before the inner backend sees them, so their messages are never in the sync either.
- No echo: Signal doesn't deliver a device's sync messages back to it, and the phone doesn't answer them. This closes D63's "syncing faro's own read receipts to the phone".

### D72. Mentions: shown as names, ring "Mentions only" chats; a mute still wins — Judgement (2026-10-08)

- **Why:** D49 and D63 left mentions for M7: Signal sends a mention as U+FFFC in the body plus a body range naming the person, so faro showed a blank box and "Mentions only" never rang.
- **Model:** `Message::mentions: Vec<Mention { who, start, len }>`, byte offsets into the body; `BackendEvent::Edited` carries the edit's mentions, which replace the old ones. `domain::mention_parts(body, mentions)` cuts a body into text and mentions, dropping ranges that are out of bounds, overlap an earlier one or fall inside a character, so a bad client can't break drawing.
- **Mapping:** `convert::mentions` reads `DataMessage::body_ranges` (ACI as a string or 16 bytes) and turns Signal's UTF-16 offsets into byte offsets (`byte_offset`; offsets inside a surrogate pair or past the end drop the mention). Style ranges (bold, spoilers…) and mentions inside quotes are left for later.
- **Store:** migration `0005_mentions.sql` (`mentions(message_id, start, length, service_id)`, cascading). Written with the message, replaced by an applied edit (older edits change nothing, mentions included), dropped on delete, loaded with each page.
- **Drawing (both layouts, via `messages::content`):** each mention is `@` + `State::person_name` (so nicknames and D65's tags apply; this account is "@You"), bold in the accent colour; mentions of this account are also reversed so they stand out. `text::wrap_pieces` wraps styled pieces like plain text (one wrapping routine, `wrap_ranges`). A placeholder left without its mention (quotes, unknown ranges) is drawn as `@` by `text::sanitize`, so U+FFFC never reaches the screen. Accent on both bubble colours is now a contrast rule (≥ 3); every bundled theme passes.
- **Copy and reply:** `State::readable_body` writes mentions as `@Name`; `y` copies that, and a reply's quote preview uses it too, since faro doesn't send mentions yet and a bare U+FFFC would reach the other side as a blank.
- **Notifications:** `State::rings_for(summary, message, now)`: "All" rings for every message as before; "Mentions only" rings when the message mentions this account. **A mute silences mentions too** (D49's open question): a mute is an explicit "quiet", and Signal's per-chat "mentions" override is a separate setting faro doesn't have. "Mentions only" chats still don't count in the window title or the footer total (`State::notifies` unchanged): the unread count is a number, so faro can't tell which unread messages mention us. An unread-mention badge would need per-message read state; later.
- **Mock:** about 15 % of group messages start with a mention, of this account half the time, so "Mentions only" can be tried with `--mock`.
- **Later:** sending mentions (completion in the input, body ranges out), mentions in quotes, text styles.

### D73. Taking a reaction back names the emoji — Judgement (2026-10-08)

- **Why:** faro sent a removal as `remove: true` with no emoji. Signal's Android app checks the emoji before looking at `remove` and drops a reaction message without a valid one ("Reaction text is not a valid emoji"), and the other apps always send it, so a reaction taken back in faro stayed on other people's phones.
- **What:** `Backend::react` takes a `ReactionChange`: `Set(emoji)` or `Remove(emoji)`, the emoji being taken back (Signal's apps send the same). `convert::reaction` maps it to `emoji` plus `remove`; the mock and the guard pass it through (the guard still checks the conversation first, D62). `Command::React` carries the change; the reducer's toggle builds `Remove(previous)` when the chosen emoji is already ours, `Set` otherwise, and the local echo and the undo on failure work as before (`ReactionChange::emoji` is what's left).
- Incoming removals were already understood with or without an emoji (a removal drops the sender's reaction whatever it names).

### D74. History transfer on link: possible, planned as M11 (v0.2) — Judgement (2026-10-08; D19 revisited, M7 task 6)

- **How Signal does it:** the new device advertises `capabilities=backup5` in the link URL (Signal Desktop's `Provisioner.preload.ts`). If the user picks "Transfer message history", the phone uploads an encrypted archive to the transit CDN and puts its key in `ProvisionMessage.ephemeralBackupKey` (field 14). The new device long-polls `GET /v1/devices/transfer_archive` for `{cdn, key}` (or `RELINK_REQUESTED` / `CONTINUE_WITHOUT_UPLOAD`), downloads, checks the HMAC, decrypts (AES-256-CBC; keys from libsignal's `MessageBackupKey`), and reads gzip'd, varint-delimited `backup.proto` frames (libsignal `message-backup`, `Purpose::DeviceTransfer`).
- **Our stack:**
  - libsignal-service-rs (our rev) keeps the key in `NewDeviceRegistration.ephemeral_backup_key`, but advertises no capabilities and has no `transfer_archive` call.
  - presage (pinned rev) drops the key (`manager/linking.rs`); presage PR #430 (an open draft) would expose it, and issue #396 asks for the feature.
  - Nothing in whisperfish parses the archive.
  - mautrix-signal (Go, AGPL) does the whole thing (`pkg/signalmeow/backup.go`), which shows the phone serves unofficial clients that advertise the capability.
- **What faro would need:**
  - add `backup5` to the QR URL, but only once everything below works, or the phone uploads for nothing;
  - the key, from PR #430 or a pinned fork;
  - the long-poll and the CDN download;
  - parsing with `libsignal-message-backup` (AGPL-3.0, git-only, the same libsignal tag libsignal-service-rs pins) or a lenient prost decode;
  - mapping text, quotes, reactions, contacts and groups into faro's store as stored events, **through the allowlist guard** (the archive holds every conversation);
  - progress and a skip option in `faro link`.
- **Limits:** it only happens at link time, so existing installs re-link. Media backfill is separate. The format moves with Signal's releases, and libsignal's strict validator may reject archives from newer phones.
- **Other routes:** legacy Android `.backup` files aren't worth it. Newer on-device backups reportedly use the same format, keyed by the Account Entropy Pool presage already stores, so a later "import a backup" could reuse the parser.
- **Decision:** not in v0.1; proposed as **M11** in v0.2, after M9 and M10. The owner can reorder it.

### D75. Attachments: faro writes the files, images download by themselves up to 25 MB — Judgement (2026-10-08, M9)

- **Backend boundary:** `fetch_attachment` returns the bytes, not a path (D30 amended). Backends stay free of file layout and permissions; faro writes `<data dir>/attachments/<sent_at>-<sender8>-<n>-<safe name>` (`0600`, directory `0700`) through a temporary file and a rename.
- **Ids:** presage needs the whole `AttachmentPointer` (key and digest included) to download. The presage backend stores it in the opaque `AttachmentId` as `p1:<hex protobuf>`. This puts attachment keys in `faro.db`, which is `0600` like `presage.db`, which already holds them. Rows from before M9 (CDN id only) heal when presage's copy of the message is replayed at startup (D64): a message that's already stored takes its attachments' ids from the replay, by position.
- **Status:** `remote` / `downloaded` (with `local_path`) / `failed` in the store (migration `0006`, plus `width`/`height`). *Downloading* is only in memory, so a crash can't leave anything stuck.
- **Policy (owner may change):** `[media] auto_download = "images"` (default) downloads images up to 25 MB once they're loaded in the open conversation. That is what inline images need, and what Signal's own apps do by default. Everything else waits for `d` / the menu / the palette. `"all"` takes every kind up to the limit; `"off"` nothing. The 25 MB limit is a constant for now; a setting can follow if the owner wants one. Failures aren't retried automatically.
- **Concurrency:** downloads leave the sequential command queue (a 50 MB video mustn't hold up a text message) and run as local tasks, at most 3 at once.
- **No percentages:** presage reads the whole download before returning, and the same for uploads. A real progress bar would mean reimplementing `get_attachment` on libsignal-service's stream. Deferred.

### D76. Open locally, otherwise view, save or copy the path — Judgement (2026-10-08, M9)

- `open`/`xdg-open` only when faro isn't over SSH (as for links, D40). Over SSH the viewer would open on the wrong machine. *View* (inline, in a popup), *Save to Downloads* and *Copy path* (OSC 52) work everywhere.
- **Save:** to `$XDG_DOWNLOAD_DIR` if set, else `~/Downloads` (created if missing), never overwriting (`name (1).ext`). A folder chooser was considered; the palette path picker (D77) could offer one later, but a fixed, predictable place is quicker. The destination is in the notice.
- **Keys:** `d` download, `v` view/open, `s` save, in the message pane. They were free, and the palette and the menu list the same.

### D77. Sending a file: a palette path picker or a pasted path, one file per message — Judgement (2026-10-08, M9)

- **Picking:** a palette level whose query is a path, with the folder listed off the UI thread as you type, because the reducer can't touch the disk. An `/attach` input command was rejected: faro's input has no commands, and the palette is already where actions live.
- **Pasting:** a bracketed paste of exactly one existing regular file attaches it. That is what dragging a file into kitty, iTerm2, GNOME Terminal and others produces. The check is asynchronous; until it answers, the paste is held (milliseconds).
- **One file, caption = input text.** Several attachments per message can come later; the domain type is already a list.
- **Copied in first:** the command task copies the file into `attachments/` before uploading. History, the local echo and *Retry* then use faro's copy even if the original moves. The cost is disk space, as in Signal Desktop.
- **Limits and safety:** 100 MB, Signal's limit. The guard checks the conversation before the backend uploads anything (D62). The mock "uploads" with a delay and can fail like any send.

### D78. Inline images with ratatui-image's sliced protocol; graphics only where every hop supports it — Judgement (2026-10-08, M9; automatic selection retained, explicit overrides added by D84)

- **Crate:** `ratatui-image` 11.1 (MIT; ratatui 0.30), `default-features = false` to avoid linking libchafa. `image` 0.25 decodes JPEG/PNG/GIF/WebP. HEIC isn't decoded (phones send JPEG to Signal). 12.0 is still a release candidate.
- **Sliced, not stateful:** `SlicedProtocol` draws images cut at the pane's edges (kitty placeholders skip rows, sixel drops bands, iTerm2 per-row slices, half-blocks per row), which a scrolling chat needs. `StatefulImage` resizes while rendering, i.e. on the UI thread.
- **Detection:**
  - The query has to run before crossterm's `EventStream` owns stdin. So it runs once, at startup, on a blocking thread.
  - faro then keeps the result only if nothing in between breaks it. mosh passes no graphics. With several tmux clients, every client gets the passthrough, so they must all be on the same kind of terminal (`#{client_termname}`, D58's check).
  - 16 colours: off. 256: half-blocks mapped like themes (D51).
  - Settings: `[media] inline_images = "auto" | "halfblocks" | "off"`, default `auto`.
- **Side effect:** inside tmux, `ratatui-image` runs `tmux set -p allow-passthrough on` for faro's pane, even for half-blocks. It is harmless, scoped to the pane, and the recommended setting anyway (§11). It is documented rather than patched.
- **Layout:** images are blank rows in the line-based layouts. Their size is a pure function of the attachment's dimensions, the cell's pixel size and the pane width (40 × 14 cells at most; stickers 8 rows). So windowing and anchors don't change, and a frame never waits for a decode. Images whose size isn't known yet (no dimensions in the pointer, not downloaded) show only their line until the download reads the header.
- **Cache:** decoded and encoded images live in the app loop, not in `State` (they're rendering resources, not facts). They are keyed by file and box, capped at 32, emptied on protocol change and `Ctrl+L`.
- **Overlays:** kitty, sixel and iTerm2 images give way to placeholders while a popup or the palette is open. Without that, the terminal-side image shows through or leaves holes, and kitty's upload (carried once, in the first cell) could be overwritten in the frame that carries it. Half-blocks are ordinary cells and stay.
- **Frames (D55):** `emoji::finish` skips image cells. Half-block images go through the `tools/frames` replay like any text. The graphics protocols can't be replayed in tmux, so the owner checks them by eye (MANUAL_TESTS M9).

### D79. Stickers are drawn as images — Judgement (2026-10-08, M9)

- The sticker's `AttachmentPointer` (WebP) becomes an ordinary attachment without a file name, so it downloads, caches and draws like a photo. It is at most 8 rows tall. Animated stickers show their first frame.
- The body stays `[sticker] 😀`, so copying, reply quotes, notifications and terminals without images still say what it was.

### D80. Voice notes: identification, pure-Rust WAV analysis (`ffmpeg` fallback), auto-download, and waveform row — Judgement (2026-10-08, M10)

- **Identification:** Signal marks voice notes with `AttachmentPointerFlags::VoiceMessage` (`flags & 1 != 0`), mapped by `faro-presage::convert::attachment` onto `AttachmentRef::voice_note: bool`. When the flag is absent (for example in replayed or local files), an `audio/*` attachment with no file name or a `voice-*` / `signal-voice*` file name is also treated as a voice note (`AttachmentRef::is_voice_note()`).
- **Auto-download:** With `[media] auto_download` set to `"images"` (default) or `"all"`, voice notes up to 25 MB (`AUTO_DOWNLOAD_LIMIT`) download automatically in the open conversation alongside images (`AttachmentRef::auto_downloads`). Voice notes are small (typically tens of KB) and need their local file before playback, waveform analysis or transcription can run.
- **Duration and waveform (`faro-core::audio` + `ffmpeg` fallback):**
  - A pure-Rust RIFF/WAVE reader (`faro-core::audio:: inspect_wav`) parses 8/16/24/32-bit PCM and 32-bit float WAV files directly with no extra crates, returning `duration_secs: Option<u32>` and a 16-bar RMS waveform (`Vec<u8>`, values `0..=7`).
  - For compressed audio from phones (AAC/M4A, Ogg Opus), the download worker falls back to `ffmpeg -v error -i <path> -ac 1 -ar 8000 -f s16le -` (capped at 10 minutes of samples) to compute the exact duration and 16 RMS bars in one pass; if `ffmpeg` is absent, it tries `ffprobe` for the duration and derives a deterministic 16-bar envelope from the attachment bytes so the row stays informative everywhere.
  - Stored in SQLite (migration `0007_voice_notes.sql`: `voice_note`, `duration_secs`, `waveform`, `transcript`, `transcript_source` on `attachments`; `remote_transcription` on `conversation_state`).
- **Row layout (`ui/messages.rs`):**
  - Voice notes render as their own row in both `Bubbles` and `Irc` layouts instead of `📎 name · size`: `▶ 0:14 ▂▄▆█▅▃▂▆█▄▃▁▃▅▄▂ · 42 KB` when idle, or `■ 0:05 / 0:14 ▂▄▆█▅▃▂▆█▄▃▁▃▅▄▂ · playing` while playing (played bars in `accent`, unplayed bars muted). Every glyph (`▶`, `■`, `▁▂▃▄▅▆▇█`, `·`) is 1 cell wide across kitty, tmux and mosh (`EmojiMode::Compatible`).

### D81. Playback through an external player, SSH/mosh guard, and progress ticks — Judgement (2026-10-08, M10)

- **External player (`[voice] player = "auto"`):** faro spawns an OS player off the UI thread (`tokio::process::Command` with `kill_on_drop(true)`) rather than linking an audio output library. `"auto"` tries `afplay` on macOS and `mpv --no-video --really-quiet`, `ffplay -nodisp -autoexit -loglevel quiet`, `pw-play`, `paplay`, `aplay -q` on Linux/Unix; a custom command string in `[voice] player` is split on whitespace with the file path passed as the final argument.
- **Single active player:** `app.rs` owns the child process (`PlayingProcess`). Pressing `x` (or `v` on a voice note) starts playback or stops it if that note is already playing; starting another note, closing the conversation or quitting kills the previous player. A 1 s timer (`AppEvent::PlaybackTick`) advances the elapsed counter and waveform highlight until the child exits (`AppEvent::PlaybackFinished`).
- **SSH and mosh guard:** when `State::env.over_ssh` or `State::env.mosh` is true, `Play` refuses to run a player on the remote machine and puts up a notice pointing to `s` (*Save to Downloads*) and *Copy path* (OSC 52). The message menu omits *Play* over SSH/mosh for the same reason *Open* is omitted for files (D76).

### D82. Opt-in Whisper transcription: local CLI or remote OpenAI-compatible API, explicit consent, and per-conversation permission — Judgement (2026-10-08, M10)

- **Opt-in (`[voice] transcribe = "off" | "local" | "remote"`, default `"off"`):**
  - Transcription is off by default. Users can trigger it manually on a selected voice note (`t` in the message pane, *Transcribe* in the message menu or palette) or automatically for downloaded voice notes in the open conversation up to `[voice] transcribe_max_secs = 300` (5 minutes; manual `t` ignores the duration cap).
  - Jobs run in the background (`Command::Transcribe` → `AppEvent::Transcribed`), separate from attachment downloads, while the note shows `· transcribing…`.
- **Local backend (`transcribe = "local"`):**
  - Runs `[voice] transcribe_command` (default `"whisper-cli"`, also trying `whisper-cpp` and `whisper`) with `[voice] transcribe_model` (path to a GGML `.bin` model, `~` expanded) and optional `[voice] transcribe_language` (`"auto"` or an ISO-639-1 code). If the command string contains `{file}` or `transcribe_model` is empty, faro passes the audio path directly to the command so custom wrapper scripts work without flags; otherwise it converts non-WAV audio to a `0600` 16 kHz mono WAV temp file via `ffmpeg` if needed and invokes `<cmd> -m <model> [-l <lang>] -nt -np -f <path>`.
- **Remote backend (`transcribe = "remote"`):**
  - Posts `multipart/form-data` (`file`, `model`, optional `language`, `response_format=json`) to `<transcribe_url>/audio/transcriptions` (default `https://api.openai.com/v1`, model `whisper-1`). Runs `curl` with `--config -` over stdin so the `Authorization: Bearer <key>` header never appears in process arguments (`ps` / `/proc`).
  - **API key:** never read from `config.toml`. Resolved from `$FARO_TRANSCRIBE_API_KEY`, else `$OPENAI_API_KEY`, else the OS keychain (`security find-generic-password -s faro-transcribe -w` on macOS, `secret-tool lookup service faro-transcribe` on Linux). Setting `transcribe_api_key` or `api_key` in `config.toml` produces a config warning explaining where to put the key.
- **Remote consent and per-conversation permission:**
  - **Explicit consent (`Popup::ConfirmRemoteTranscription`):** turning on `transcribe = "remote"` or running a remote transcription before consenting opens a confirmation popup naming `transcribe_url` and explaining that voice audio from allowed chats is sent to that endpoint. Confirming persists `voice.remote_consent = "true"` in `app_state`.
  - **Per-conversation flag (`ConversationState::remote_transcription`):** remote transcription is disallowed by default for every conversation (`RemoteTranscription::Disallowed`). It is toggled per conversation from the conversation menu, the palette, or by confirming manual `t` on a voice note in that chat (which offers to allow remote transcription for the conversation). Local transcription never leaves the machine and needs neither remote consent nor the per-conversation flag.
  - **Display and storage:** transcripts are saved in `attachments.transcript` + `attachments.transcript_source` (`local` / `remote`) so each note is transcribed once. They render directly below the voice note row as `❝ <text> [local]` or `❝ <text> [remote]`, and `State::readable_body` includes the transcript so `y` (*Copy*) and `r` (*Reply*) work on transcribed voice notes.

### D83. M12 (v0.2 UX, messaging, search and polish) and 3-wave parallel subagent plan — Accepted (owner, 2026-10-08)

- **Scope:** the owner requested a palette navigation improvement (`←`/`→` cycling setting values in place, `Enter` opening the setting sub-menu, and staying inside `Ctrl+P` when changing settings/themes so multiple options can be changed in one session) plus all 10 recommended v0.2 daily-driver features:
  1. **Palette navigation & multi-edit (`Settings ›` & `Themes ›`):** `←`/`→` on any setting row in `Settings ›` cycles through its choices in place (applying and saving immediately with live preview behind the palette); `Enter` on a multi-choice setting opens its vertical sub-menu (`Level::Setting(s)`); pressing `Enter` on a value inside `Level::Setting(s)` applies it and steps back up to `Settings ›` instead of closing `Ctrl+P`; pressing `Enter` in `Themes ›` applies the theme and moves the current marker while keeping `Themes ›` open.
  2. **Edit & "Delete for Everyone" on own messages (`e` / `D`):** `e` loads the selected outgoing message into the input box (`✎ Editing message…  esc cancel`) and sends a Signal `EditMessage` (`Backend::edit`); `D` opens a confirmation popup (`Popup::ConfirmDelete`) and sends a Signal `Delete` (`Backend::delete`).
  3. **Sending `@mentions` with inline `@` autocomplete:** typing `@` in the input box opens an inline member autocomplete popup filtering conversation participants, inserts `@Name`, and populates `OutgoingMessage::mentions` (mapped to UTF-16 `BodyRange` with `MentionAci` in `faro-presage`).
  4. **Full-text message & transcript search (`/` or `Ctrl+F`):** searches `messages.body` and `attachments.transcript` in SQLite (`Store::search_messages`), scoped to the open conversation (`/` in `messages` or `Ctrl+F`) or across all conversations from `Ctrl+P`, and jumps to the target message in context (`Store::messages_around`).
  5. **Delivery receipts (`✓✓`) & per-recipient "Message Info" popup (`i`):** maps `ReceiptMessage::Type::Delivery` in `faro-presage`, stores per-recipient delivery/read timestamps in SQLite (migration `0008_receipts.sql`), distinguishes dim `✓✓` (delivered) from accent `✓✓` (read), and shows per-recipient timestamps in `Popup::MessageInfo` (`i` in `messages`).
  6. **OSC 99 (kitty) / OSC 9 / OSC 777 desktop notifications over SSH & tmux:** wires `[notifications] desktop = "auto" | "osc" | "native" | "off"` and `privacy = "none" | "sender" | "preview"`, emitting terminal OSC sequences wrapped in tmux DCS passthrough when inside tmux and respecting Do Not Disturb (`Alt+N`) and per-conversation mute/mention rules.
  7. **Emoji `:shortcode:` autocomplete & full reaction search:** `:shortcode` completion popup in the input box and `/` filter search over all tmux-safe emoji inside the `+` reaction picker.
  8. **Jump to unread marker (`u`) & sticky unread position:** `u` in `messages` jumps straight to `OpenConversation::unread_marker` (`── unread ──`) so long unread backlogs can be read top-down.
  9. **Wire up `[messages] density` and `timestamps` settings:** activates `density = "comfortable" | "compact"` (removing blank separator lines between same-sender bubbles in `compact`) and `timestamps = "relative" | "absolute" | "off"`, exposed in `Settings ›` with live preview and `←`/`→` cycling.
  10. **Shared links & media in the right info panel (`Alt+I`) & palette:** lists recent URLs and downloaded files/media of the open conversation in `ui/info.rs` and under a palette sub-menu (`Level::ChatLinks`).
  11. **Forward message (`f` in message selection):** `f` on a selected message opens a conversation picker (`Level::ForwardTo`) and sends the message's text/attachment to the chosen allowed conversation.
- **Parallel vs. sequential execution (3 waves):** to let multiple subagents work concurrently in isolated working copies (`Workspace="share"`) without stepping on each other's files or migrations, M12 is structured into **3 sequential waves of parallel tracks** in `docs/PLAN.md` (Wave 1: Tracks `1A`, `1B`, `1C`; Wave 2: Tracks `2A`, `2B`, `2C`; Wave 3: Tracks `3A`, `3B`). Only Track `1B` adds a SQLite migration (`0008_receipts.sql`).

### D84. Explicit inline-image protocols and an explanation of Auto — Accepted (owner, 2026-10-09)

- **Why:** the owner's kitty renders crisp images directly, but mixed terminal types attached to tmux trigger conservative half-block fallback. Users should be able to understand and override that choice, like emoji and bubble style.
- **Setting:** extend the existing `[media] inline_images` / `Ctrl+P › Settings › Inline images` to `"auto" | "kitty" | "sixel" | "iterm2" | "halfblocks" | "off"`. Auto remains the default with D78's detection and fallback rules. Values cycle and save through the existing settings machinery.
- **Overrides:** explicit graphics modes select that protocol immediately, without a capability query or confirmation dialog. They override mixed/detached tmux, mosh and colour-depth fallback decisions. **Judgement:** colour depth limits text-cell colours, not graphics payloads, so forced graphics remain available at 256/16 colours. These choices cannot make an incompatible terminal or mosh carry graphics; the values page says that compatibility and tmux passthrough are required.
- **Explanation:** the values page shows Auto's selected mode and reason (colour depth, mosh, mixed/detached tmux clients, startup detection, or restart needed to query). The app records only the query result in State; encoded images remain in the app-loop cache. The renderer refreshes the note while the page stays open.
- **Lifecycle:** no new stdin queries after EventStream starts (**amended by D90**: allowed only by pausing crossterm as D90 describes). Explicit modes work live even if startup skipped detection; without a query they use the existing picker's estimated cell pixel size. Returning to Auto uses the saved startup result, or half-blocks until restart if there was no query. Protocol changes invalidate cached/in-flight encodes; overlay placeholders and frame safeguards are retained.

### D85. Package names: `faro` everywhere, `faro-chat` on crates.io — Accepted (owner, 2026-10-09)

- **Why:** settles the open question "repo/package name `faro` vs `faro-chat`" (D11) before going public.
- **Availability (rechecked 2026-10-09):** `faro` is free on Homebrew (formula + cask), Arch, AUR and every repo Repology tracks (Debian, Fedora, Nixpkgs, Alpine, MacPorts…). On crates.io it belongs to `eddraz/faro`, a small web-search CLI (0.3.0, active, also installs a `faro` binary). `faro-chat` and `farochat` are free everywhere checked.
- **Decision:** the project, repo, command and every package are `faro` (Homebrew tap, AUR `faro-bin`/`faro-git`, `.deb`, Nix flake). Only the crates.io package is `faro-chat`, still installing a `faro` binary (`[[bin]] name = "faro"`), as `fd` does with `fd-find`. The README lists the install command per package manager.
- **Consequences:** distros that already have (or later get) another `faro` may rename ours (as Debian ships `fd` as `fd-find`); that's their call. Claim the AUR names with the first release. crates.io publishing still waits for presage's git-only dependencies (§13 in DESIGN.md); until then `faro-chat` is only reserved.
- **Website (owner's preference):** `getfaro.dev` as the main domain, optionally `farochat.dev` redirecting to it. `faro.chat` is taken.
- **Rejected:** `faro-chat` everywhere (consistent but longer to type, and the name people see would differ from the command); bare `faro` on crates.io (taken).

### D86. Names reserved and AUR packaging — Accepted (owner, 2026-10-09)

- **Reserved (owner):** `getfaro.dev` (Namecheap; no email service, SPF `v=spf1 -all` and DMARC `p=reject` so nobody can send mail as the domain; `www`/`@` still point at the parking page until the website exists); `faro-chat` 0.0.0 placeholder on crates.io (owner `harababurel`, library only, no binary, so `cargo install faro-chat` can't install a broken `faro`); the Homebrew tap `harababurel/homebrew-tap` (empty `Formula/`). `farochat.dev` not bought.
- **AUR packages:** `faro-git` (latest commit, built from source), and with the first release `faro` (tagged release from source) and `faro-bin` (the release's pre-built binary, no Rust needed). All three `provides`/`conflicts` `faro`.
- **`faro-git` PKGBUILD** (`packaging/aur/faro-git/`, Judgement): Arch's Rust guidelines (`cargo fetch --locked`, then `--frozen` builds and `cargo test` in `check()`), `RUSTUP_TOOLCHAIN=stable`, `--remap-path-prefix` so the build directory isn't embedded in the binary, `!debug` (the release profile strips, so a `-debug` package would be empty), `!lto` (the profile sets its own). Runtime depends: `glibc`, `libgcc` and `openssl` (presage's SQLCipher store links `libcrypto` on Linux; on macOS it uses Apple's crypto). Installs `/usr/bin/faro`, the README and `THIRD_PARTY_NOTICES.md` (bundled MIT themes). Built, installed and checked with `namcap` on the owner's Arch machine (only the usual `ld-linux` false alarm left).
- **Not uploaded yet:** the source is cloned over HTTPS, so it needs the GitHub repo to be public, which waits for the owner. The owner's AUR account and SSH key (`~/.ssh/aur` on the Arch machine) are set up and tested. Testing while the repo is private: switch the source to `git+ssh://` locally and never commit that.
- **Official Arch repos:** not something to apply for; a packager adopts popular AUR packages. Needs nothing beyond tagged releases and an offline-buildable source package.

### D87. `faro demo` replaces `--mock`: demo data never meets real data — Accepted (owner, 2026-10-09; details Judgement)

- **Problem:** `faro --mock` only swapped the backend. Forgetting `--data-dir` wrote fake contacts, messages and attachments into the real `faro.db`, showed real chats beside fake ones, and "sent" messages typed into real chats to the mock, where they were never delivered. Silent and irreversible. Supersedes the CLI parts of D32 and D35's `--mock-*` flags.
- **Shape:** `faro demo [--seed N] [--activity MS] [--no-faults] [--fresh] [--dir DIR]`. `--mock`, `--mock-*` and `FARO_MOCK_*` are removed with no environment replacement: demo settings are typed every time, so a leftover variable can't change what a demo run touches. `--emoji`, `--allowlist` and `--no-allowlist` are global; `faro demo --no-allowlist` is an error. The real run's missing-allowlist error points at `faro demo`. User-facing wording says "demo"; the code keeps `MockBackend`/`MockConfig`.
- **Where (owner's choice):** `<temp>/faro-demo-<uid>/` (`std::env::temp_dir()`, i.e. `$TMPDIR` or `/tmp`), reused until the system clears it; `--fresh` resets it; `--dir` puts it elsewhere. The demo never uses `--data-dir` (typed: an error naming `--dir`) or `FARO_DATA_DIR` (environment: ignored and logged), and honours only a typed `--allowlist` (`FARO_ALLOWLIST` is ignored and logged); `clap::ArgMatches::value_source` tells typed flags from environment values. The config (`--config`/`FARO_CONFIG`/default) is the real one, read and written: changing the theme in the demo is a deliberate choice (owner).
- **Guards (each independent, D27):**
  1. The demo dir is created `0700` with a `.faro-demo` marker. An existing one must be a real directory (not a symlink), owned by us, without `presage.db`, and either marked and `0700` or empty (then tightened and marked); nothing at its top level may be a symlink, and `faro.db` must be a regular file. `--fresh` wipes only a marked dir that passed every check and holds no `presage.db` or foreign `faro.db` anywhere inside (`paths::prepare_demo_dir`).
  2. Each database carries `app_state` `profile` = `demo` | `signal`, read through a read-only connection before the file is created, chmodded, switched to WAL or migrated (`Store::open` takes the expected profile): real runs refuse `demo`; an unmarked existing database is real history and becomes `signal`; the demo refuses anything not `demo` unless the database is empty.
  3. The demo dir may not be, contain, or sit inside the real data dir or `FARO_DATA_DIR`'s dir, compared after resolving symlinks.
  4. Real runs, `faro link` and `faro ids` refuse a data dir that is, or sits inside, a directory carrying `.faro-demo`.
  All checks run before logging starts (the demo's logs live in its own dir).
- **UI:** `State::profile`: a `demo` marker in the footer, `faro demo` / `faro demo (N)` as the window title, and "demo · fake contacts, nothing is sent" on the logo screen.
- **Unchanged:** *Save to Downloads* and *Send a file…* still use real files (explicit actions); transcription follows the config.
- **Judgement:** the uid comes from `rustix::process::getuid` (rustix was already in the dependency tree; `libc` would need `unsafe`, which the workspace forbids).
- **Rejected:** keeping `--mock` with a warning (the failure stays one forgotten flag away); a persistent demo dir under `~/.cache` (owner preferred the temp dir); `FARO_DEMO_*` environment variables.

### D88. The demo cast is an Office parody played as written scenes — Accepted (owner, 2026-10-09; details Judgement)

- **Problem:** the mock's stress-test people and one shared list of random texts made every conversation meaningless (anyone said anything anywhere).
- **Decision (owner):** an affectionate parody of NBC's *The Office* (US). This account is **the new temp** at the Scranton branch: never named (not the Linux username either); characters call it "the temp" and mentions show as `@You`. Precedents for using the show's characters in open source: Munder Difflin (a multi-agent harness with the cast as agents) and the `schrute` R package on CRAN (the show's transcripts). The cast is demo data only, not the project's name or brand, so it can be swapped without touching anything else.
- **Guardrails:** original lines only (short catchphrases allowed); no "Dunder Mifflin" trademark anywhere (tested); no images of the actors or from the show (the bundled images are generated objects and scenes); no jokes about ethnicity, nationality, religion, gender, sexuality, body or age, and no Benihana waitresses (their joke is a racial stereotype; owner). A disclaimer is in the README and `THIRD_PARTY_NOTICES.md`.
- **Language coverage (owner):** Tanaka-san, a Japanese paper-mill rep (double-width characters); Mariam, a paper client in Cairo (right-to-left text, Arabic-Indic digits, Latin inside Arabic); Kelly's mom (Hindi, Devanagari); German and Spanish asides from Dwight and Oscar. Their scenes are written respectfully, in correct language, with an English line in each Japanese and Arabic scene.
- **As built:** the cast is data: `mock/cast.rs` (`Who`, `PEOPLE`, quiet contacts and clients, `GROUPS`, `Chat`) and `mock/scenes.rs` (30 `Scene`s of `Line`s with replies, mentions, reactions, edits and deletes, written with const builders; `REPLIES` pools for answering your messages; `REACTS` favourite reactions). Attachments are in `mock/assets.rs`: generated images under `crates/faro-core/assets/demo/` (`include_bytes!`), `.txt` files written in character, synthesized voice notes. `sim.rs` plays scenes with the existing engine: the backlog is filled with whole scenes; each message activity advances one of at most 2 running scenes (never two in one chat) or starts the next in a seeded order, reshuffled when all have played. Generic activities are voiced by the cast and stay out of chats where a scene is running. Typing time is capped by the activity rate so flood mode keeps up. Kelly has only a profile name, so her ZWJ name and profile changes show. `WORLD_SEED` was bumped so old mock databases' history is orphaned.
- **Tests:** `world_stresses_the_ui` and `scenes_stress_the_ui` enforce every stress case (long name, ZWJ name, nameless contact, 30+ quiet conversations, a 40+ member group, a long group title, RTL, CJK, Devanagari, combining marks, flags, skin tones, variation selectors, multi-line, links, wrapping, edits, deletes, replies, mentions of you, photos, a sticker, files, voice notes); `scenes_are_consistent` checks speakers, reply targets, assets and the trademark.
- **Rejected:** renaming the old cast (still nonsense); a lighthouse-keepers cast (kept as the alternative if the parody ever has to go); runtime-parsed script files (constants are type-checked and can't fail at runtime); using the Linux username for the account.

### D89. Pasting clipboard images over SSH: kitty's OSC 5522, tmux passthrough, no mosh — Proposed (owner's request, 2026-10-09)

- **Why:** the owner wants to paste an image from the local clipboard into faro running on another machine. Today faro only writes text to the clipboard (OSC 52); images go through `Send a file…` or a pasted path (D77).
- **Findings (owner's hands-on test, kitty → SSH → Mac mini → tmux 3.8, `allow-passthrough on`):** kitty's clipboard protocol (OSC 5522) returns images in-band. A 514 KB PNG came back over plain SSH, and a 285 KB PNG came back intact through tmux once the request was wrapped in tmux passthrough. `kitten clipboard` hangs inside tmux because it doesn't wrap its request. Listing types needs no prompt; reading data prompts in kitty. Full log, commands and protocol notes: [research/clipboard-image-paste.md](research/clipboard-image-paste.md).
- **Scope:** kitty only, over SSH with or without tmux. **mosh is out of scope** (owner): it relays screen state, not bytes, so neither request nor reply crosses.
- **Main constraint:** crossterm would turn the reply into one key event per byte (`Alt+]`, then hundreds of thousands of `Char`s), and D84 forbids stdin queries after `EventStream` starts. The plan starts with a spike (pause crossterm and read the tty directly, or faro's own input reader) whose outcome amends D84.
- **Plan:** Phase 0 spike, Phase 1 `faro-core::clipboard` parser, Phase 2 `Alt+V` / palette *Paste image* staging the image like a pasted file, optional Phase 3 kitty paste events (Cmd+V, single-use password) with its tmux risks. Details and open questions in the research note; scheduled in PLAN.md "Later".

### D90. Reading terminal replies after startup: pause crossterm, read the tty directly (amends D84) — Accepted (owner, 2026-10-09)

- **Why:** D89 needs kitty's OSC 5522 reply while the TUI runs; D84 forbade stdin queries once `EventStream` starts. Phase 0 spike: `tools/clipboard-spike` on branch `spike/clipboard-osc5522` (fake-kitty harness `fake_kitty.py`), then the owner's run in kitty → SSH → Mac mini → tmux.
- **Decision:** to read a reply, faro:
  1. drops the `EventStream` (crossterm's global reader outlives it; its thread exits when woken);
  2. calls `crossterm::event::poll(20 ms)` once as a barrier, so the old thread has let go (keys read here stay queued for the next stream);
  3. writes the request, then on a blocking thread waits on fd 0 with mio (epoll/kqueue; `poll(2)` doesn't work on macOS ttys) and reads exactly `FIONREAD` bytes, so it never blocks. It never sets `O_NONBLOCK`: the tty is shared with stdout;
  4. stops at the end of the reply, an error status, or a timeout, then creates a new `EventStream`.
- **Late-reply guard:** a reply that arrives after faro stopped waiting reaches crossterm as keys (`Alt+]`, then one `Char` per byte). The app watches for `Alt+]` followed by `5522;`, then drops the stream, discards crossterm's queued events and drains the tty the same way until `DONE`.
- **Inside the reply:** packets never contain `ESC`, so any `ESC [ … final` inside one (tmux put kitty's focus-in report in the middle of the image on the owner's machine) is cut out before decoding. Bytes outside packets during a read are dropped. Focus reports from either place are turned back into focus events (D42).
- **Results:** fake kitty, 300 KB PNG: identical every time, directly and through tmux 3.6b, with any write sizes; no stray keys; typing works right after. Owner, kitty → SSH → tmux on macOS: a 285,083-byte PNG intact in 1.6 s, `EPERM` when denied, no prompt for the type list.
- **Rejected:**
  - Rebuilding the reply from crossterm's key events. crossterm 0.29 wedges on bursts: it returns after each 1 KB read, its fd is blocking and mio is edge-triggered, so once the kernel tty buffer fills (and throttles), no new edge comes and input stops for good, later keys included. It worked only with an artificially slow writer.
  - faro's own input reader, and a helper in a tmux popup: not needed.
- **Several tmux clients:** every attached kitty answers, and the first answer wins. In the owner's first run a second kitty with text on its clipboard answered first; the guard drained the real reply. Phase 2 has to handle this (see the D89 open questions).

### D91. Pasting clipboard images: `Ctrl+V`, staged never sent, every kitty asked — Accepted (owner, 2026-10-09; details Judgement)

- **Why:** D89 Phase 2, on top of D90 (how the reply is read) and `faro-core::clipboard` (Phase 1).
- **Owner's choices:**
  - **`Ctrl+V`** (`attachment.paste_image`, global; also *Paste an image from the clipboard* in the open chat's palette commands). `Alt+V` belongs to the owner's Superwhisper. kitty keeps `Cmd+V` and `Ctrl+Shift+V` to itself, so faro never sees them.
  - **25 MB** limit (the same as automatic image downloads).
  - **Several kitty clients attached to faro's tmux session: ask them all.** The first type list with an image wins; after a list without one, others get 500 ms. The other screen's kitty may show a prompt nobody answers; anything it sends later carries an old request id and is drained by the guard (D90).
  - **kitty remembers the permission:** a random password per run (`pw`, name `faro`). After "allow all", no more prompts until faro restarts. Older kittys ignore it.
  - **Never sent automatically:** the image is staged like a pasted file (`📎 clipboard-YYYYMMDD-HHMMSS.png · 600×313 · 278 KB  ctrl+o view ·  esc remove`); only `Enter` sends it. Remembering the permission skips kitty's prompt, never this step.
  - **`Ctrl+O`** (`attachment.view_staged`) shows the staged image in the full-size viewer (`Popup::StagedImage`); `Esc` closes it and the image stays staged.
  - **2 minutes** to answer kitty's prompt, fixed (**Judgement:** a setting only if someone asks).
- **How (Judgement):**
  - `Effect::ReadClipboardImage { conversation }` → `Router` → a channel to the app loop, which owns the input. `clipboard::Paste::begin` drops the `EventStream` and starts `clipboard::start`.
  - The task checks for kitty (`Terminal::kitty`: directly, or any tmux client in kitty and not under mosh) and gives `NotKitty`/`Mosh` otherwise. Then it runs the barrier, writes the list request from the loop's thread (wrapped for tmux), reads it on a blocking thread, writes the data request (best of PNG, JPEG, GIF, WebP, with id and password), and reads the data.
  - The image is saved as `<attachments>/clipboard-<local time>.<ext>` (`0600`, atomic, `-2`… on clashes) with its dimensions. Then `AppEvent::ClipboardImage` (after any focus reports) makes the loop create a new `EventStream`, and the reducer stages the image or shows a notice.
  - Timeouts: 5 s for the type list, 2 min for the prompt, 5 s between packets.
  - The blocking reader has a mio `Waker` so quitting never waits on it.
  - **Late-reply guard** (`clipboard::LateGuard`): armed after the first paste. It holds back `Alt+]` or `Esc`, then `]`, `5`, `5`, `2`, `2`, `;` for at most 50 ms. A full match drains the tty (`AppEvent::ClipboardDrained`); anything else gives the keys back in order. Before the first paste nothing is ever held.
- **Known limits:**
  - While faro waits for the prompt, keys typed into faro are dropped: kitty's prompt has the focus anyway.
  - The clipboard file stays in `attachments/` after sending (sending copies it under the message's name). Cleanup can come later. (**Done in D93.**)
- **Rejected:**
  - Asking only when a single kitty client is attached (the owner's main session has two).
  - Making the timeout a setting.
  - Paste events (`Cmd+V`): Phase 3, later.

### D92. Backend events that arrive before the conversation list are applied on top of it — Accepted (Judgement, 2026-10-09)

- **Found:** while testing D91 end to end (`faro demo` behind a terminal that answers the startup image query slowly), the conversation list stayed empty except for chats created by later messages.
- **Cause:** a startup race. `Effect::LoadConversations` is the first job of the sequential store worker, so the list it reads predates any event persisted afterwards. Backend events that reached the reducer before `AppEvent::Loaded` were applied to `State::conversations`, then `on_loaded` replaced the whole list with the older snapshot. It shows whenever startup is slowed: the 500 ms image query timing out, a slow disk, or a burst of messages while presage catches up. The demo's backlog made it obvious.
- **Decision:** while `State::loading`, the reducer holds `AppEvent::Backend` events in `State::early_events`. `on_loaded` installs the store's list, then applies them in order (still `loading`, so the backlog doesn't ring the bell, D49) and returns their effects (`Persist`…).
- **Rejected:** merging the snapshot with the list built from events (the store has state the events lack, like pins, mute and drafts); loading the list before starting the backend (the backend and the store start independently).
- **Test:** `update::tests::messages_arriving_before_the_list_are_kept`.

### D93. Pasted clipboard images are deleted once copied, and swept at startup — Accepted (Judgement, 2026-10-09)

- **Why:** D91 left `attachments/clipboard-*.ext` behind after sending, because sending copies the file under the message's name, and the local echo and *Retry* use the copy.
- **Decision:**
  - `commands::copy_outgoing_attachments` deletes the source after copying it when `clipboard::is_pasted` is true: a file directly in `attachments/` whose name starts with `clipboard-`, a name only faro gives.
  - At startup nothing is staged, so `clipboard::sweep` (a blocking task started by `app::run`) deletes every such file left over, e.g. one removed with `Esc` or still staged at quit.
  - Received attachments never start with `clipboard-` (they start with the send time), and files in subfolders are never touched.
- **Tests:** `clipboard::tests::pasted_images_are_recognised_and_swept`; `tools/clipboard/e2e.py` checks that after sending, the pasted file is gone and exactly one copy remains.

### D94. An About popup with the small lighthouse — Accepted (Judgement, 2026-10-09; easy to change)

- **Why:** the backlog's "About screen with the logo". It needed no input from the owner to do it simply and reversibly.
- **Decision:**
  - *About faro* in the palette's commands opens `Popup::About`, centred on the screen. It shows the small theme-coloured lighthouse (`ui::logo::draw_small_lighthouse`; left out when the screen is too narrow) beside:
    - the version, and the logo screen's tagline;
    - getfaro.dev and the repository;
    - the licence and "not affiliated with Signal";
    - credits (presage and libsignal, OpenCode's themes);
    - the data folder.
  - `Esc`, `Enter` or `q` closes it.
  - Version, repository and licence come from Cargo (`CARGO_PKG_*`), so they can't go stale.
- **Not done:** no key of its own (the palette is enough), and no animation (the big logo screen has that).
- **Test:** the `ui::tests::about_popup` snapshot (version masked).
- **Amended (2026-10-10, owner's requests):** the links are shown in full (`https://getfaro.dev`, `https://github.com/harababurel/faro`) so terminals make them clickable, and the config file faro loaded (`State::config_path`, the default or `--config`) is shown above the data folder, the two labels padded so the paths line up.

### D95. The allowlist guard becomes optional for v0.1 — Accepted (owner, 2026-10-11; done the same day, M8 task 5)

- **Why:** the guard (D62) is a safeguard for testing on a real account. It does nothing for real users, and making them type `--no-allowlist` every time (D67) was only a stop-gap. D62 already said the requirement would be lifted for the release.
- **Decision (to do before tagging v0.1, M8 task 5):**
  - the Signal backend starts normally with neither flag: no guard, like any Signal client;
  - `--allowlist FILE` (and `FARO_ALLOWLIST`) stays, as an optional switch for development and testing; everything in D62 still holds when it's given;
  - `--no-allowlist` is still accepted but does nothing (help text says so), so existing commands keep working;
  - the footer's muted `no allowlist` marker goes (unguarded becomes the normal case); a guarded run could show `allowlist` instead.
- **Not affected:** the in-app allowlist mode (D41, `conversations.allowlist`), which only decides which chats the main list shows.
- **Test account:** keeps running with `--allowlist` as before; its data dir stays separate.
- **As built:** `main.rs` no longer refuses to start; `serve(…, guarded)` and `Directory::guarded` replace `unguarded`; `state::Guard` defaults to `Off` and is `On` behind `--allowlist` (the demo with `--allowlist` too), where the footer shows a muted `allowlist` (`ui/footer.rs::GUARDED`) instead of the old `no allowlist`. `--no-allowlist` stays in `--help` as "does nothing now", still conflicts with `--allowlist`, and is no longer refused by `faro demo`. README updated. Tests: `ui::tests::running_behind_an_allowlist_shows_in_the_footer`, `cli::tests::no_allowlist_is_still_accepted_but_excludes_an_allowlist`.

### D96. Sidebar markers for groups, muted and mentions-only chats — Accepted (owner, 2026-10-11; done the same day, M8 task 6)

- **Why:** in the sidebar a muted chat is only dimmed, the same as a "mentions only" one, and the open highlight hides even that; groups and direct chats look the same. The owner noticed neither while testing M7. The info panel (`Alt+I`) says it, but it's easy to miss.
- **Decision:**
  - **Kind:** `#` before a group's name, nothing before a direct chat, in the left column beside the unread dot (` ● # Climbing`). Every row gets the same two cells, so names stay aligned. The palette's `Chats ›` does the same: direct chats lose their `@ `.
  - **Notifications:** on the right edge, with room kept so it's never cut off: `⊘` muted, `@` mentions only, nothing when notifications are on. Drawn in the theme's muted colour, and shown even when the chat is open. Dimming stays as it is.
  - Single characters, not emoji (🔕): one cell everywhere, themed, no tmux/mosh width risk (D55), fine in the `system` theme and in terminals without emoji fonts.
- **When (Judgement, agent's recommendation; easy to move):** before the v0.1 release: it's small, settled, and visible in daily use and screenshots.
- **As built (Judgement):** the marker comes *after* the unread count (` ● # Climbing   3 @ `), not before it, so markers stay in one column whatever the counts; a row with neither ends at the border as before. `ui/sidebar.rs`: `kind_glyph`, `notify_glyph` (muted wins); the `#` is in the muted colour too. Palette `Chats ›` and *Forward to*: direct chats get a blank glyph. Test: `ui::tests::sidebar_markers_for_groups_muted_and_mentions_only`; sidebar snapshots updated.

### D97. The version line: version, commit and commit date — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** the logo screen didn't say which faro was running; with frequent pulls and rebuilds on two machines, the version alone (`0.1.0-dev`) doesn't tell builds apart.
- **What:** `0.1.0-dev · 8fe5755 · 2026-10-11`: the crate version, the short commit hash (`+` when tracked files had uncommitted changes; untracked files don't count) and the **commit's** date, not the build date, so rebuilding the same commit gives the same line. Without git (a release tarball, crates.io, or the crate unpacked inside some other repository: `git ls-files` must know `Cargo.toml`) it's just the version. Packagers can set `FARO_BUILD_COMMIT` / `FARO_BUILD_DATE`.
- **Where:** a muted `v…` line under the tagline on the logo screen (full, stacked and small-beside forms; dropped when it doesn't fit; the word-only form has none), `faro <line>` in About, and `faro --version`.
- **How:** `crates/faro/build.rs` runs git at build time and sets `FARO_VERSION`; it reruns when `HEAD`, the index, `crates/` or `Cargo.lock` change, so the `+` follows edits. `version::shown()` returns a fixed line under `cfg(test)` so snapshots don't change with every commit.
- **Amended (owner, 2026-10-11):** the line reads `0.1.0-dev (8fe5755 2026-10-11)`, hash and date grouped in parentheses like `rustc --version`, so the date reads as part of the build's label rather than today's date. Same change: the logo screen's tagline (D61) is now `unofficial Signal client for the terminal` (one phrase, "unofficial" first, 41 columns instead of 45).

### D98. Rounder bubbles, and compact runs that join — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** taller rounded bubbles (D58) have diagonal corners beside fully round one-line capsules, which looks inconsistent. In compact density, bubbles from one sender sit on top of each other but don't connect.
- **Chosen with a preview** (`tools/bubble_shapes.py`, run by the owner in kitty inside tmux): of A (D58's wedges), B (quarter circles), B+ (B with padding) and C (two diagonals per corner on full-row edges), the owner preferred B+, and B-compact / B+-compact for runs. A and C are dropped: C's full-row edges add too much height.
- **Decision (owner):**
  - **Taller rounded bubbles:** Unicode 16's justified quarter circles (U+1FBEC–1FBEF, radius half a cell wide) at the corners, quarter-row top and bottom edges (`▂` U+2582, `🮂` U+1FB82) and half-cell sides (`▐` `▌`). One-line bubbles keep their Powerline capsule ends. `mixed` gets the same corners on its rounded side.
  - **`[messages] bubble_padding`** (on by default): a padding cell inside each rounded side (B+); off is B.
  - **Compact runs join** (compact density with `rounded`/`mixed`; no separate switch): consecutive bubbles from one sender touch, share one edge on their aligned side, rounded only at the run's top and bottom. On the other side a wider bubble's edge and corner are drawn in its neighbour's row, past the neighbour's end, so outward corners stay round; cells shared by two bubbles use Unicode 16 octants (U+1CD00–1CDE5) or older block characters. Inside a run one-line bubbles are rounded rectangles; on their own they're still capsules.
  - **`[messages] match_widths`** (on by default): in a joined run, a bubble within 3 cells of its neighbour's width is widened to match; a chain of such neighbours takes the widest width.
  - **What ends a run:** another sender, a new day, the unread rule, a pause longer than the name-grouping window, or a reaction row under the previous bubble.
  - **Compact keeps a blank line between runs** (before: none at all, so two people's runs in a group would merge into one shape).
  - **Older kitty** without these glyphs shows boxes; documented (README, setting help), not detected. `auto` is unchanged (D58: rounded only when every client is kitty).
- **Widths:** every new glyph is one cell in tmux 3.6b, glibc 2.43 `wcwidth` and unicode-width 0.2.2 (Unicode 17); kitty draws U+1FBCE–1FBEF, U+1CD00–1CDE5 and U+1CE51–1CEAF itself. macOS's own tables (mosh to a Mac) are unchecked; `auto` never picks rounded under mosh.
- **As built (Judgement):** `ui/bubbles/grid.rs` composes rectangles of octants (2 × 4 per cell) and maps each cell's pattern to its glyph (`grid::octant`: older block characters first, then U+1CD00 onwards in ascending pattern order, computed rather than tabled); a rounded corner alone in its cell becomes a quarter circle. `bubbles::Outline` (a bubble's cells and rounded sides) and `bubbles::Run` (`first`, `above`, `below`, `inner`) carry the neighbours; `messages::run` builds them (`joins`, `matched_width`, natural widths measured once per window in `Context::widths`). One-line bubbles on their own and bars take the old path (`Bubble::flat`). The padding cells count against the 75 % width cap, so capsules may be up to two cells narrower than before. `is_edge` replaces `EDGE_GLYPHS` for the selection's blanking. Tests: `grid::tests`, `taller_rounded_bubbles_have_quarter_circle_corners_and_thin_edges`, `compact_rounded_runs_join_on_their_aligned_side`, `bubble_padding_adds_a_cell_inside_rounded_sides`, `match_widths_lines_up_close_bubbles_in_a_run`, snapshot `bubbles_rounded_compact`. Replay (`tools/frames/record.py mixed --bubbles rounded --density compact --seed 51`, new `--density` flag): 3,068 frames, 0 divergent, directly and nested; the recording holds quarter circles, quarter blocks, half-cell sides and octants at run steps.

### D99. Defaults, and the bubble settings in a section of their own — Accepted (owner, 2026-10-11; details Judgement)

- **Defaults (owner):** `[messages] density = "compact"` (was comfortable) and `timestamps = "relative"` (was absolute). Already as wanted: `ui.emoji = "auto"`, `layout = "bubbles"`, `bubble_padding` and `match_widths` on, `reduce_motion` off.
- **`bubble_style` stays `auto`** (owner, on the agent's recommendation): a `rounded` default would show boxes to everyone outside a recent kitty. The owner sets `bubble_style = "rounded"` in their own config (that also gives rounded bubbles through mosh, where `auto` draws bars).
- **Palette sections (owner, on the agent's recommendation):** Appearance is split into `Appearance` (Colours, Emoji, Reduce motion), `Messages` (Message layout, Timestamps: both layouts) and `Bubbles` (Bubble style, Bubble padding, Match bubble widths, Message density: bubbles layout only). With the IRC layout the Bubbles rows stay visible, dimmed, with "bubbles layout only" as the value, and can't be selected (skipped by arrows, pages and clicks). Rejected: hiding them (people wouldn't learn they exist, and searching "padding" in IRC would say "No matches"). The config keys stay under `[messages]`; their help text says "Bubbles layout only".
- **As built (Judgement):** `Item::disabled` (drawn muted, no `›`, never selected); `Setting::bubbles_only`; `Palette::selected_item` returns `None` for a row that can't be selected, so nothing (Enter, `←`/`→`, flips) acts on headers or disabled rows; `grouped` treats only headers as headers and picks the best match among selectable rows. The fixtures pin comfortable density and absolute times so older snapshots stay as they were. Tests: `bubble_settings_are_dimmed_and_skipped_in_the_irc_layout`, updated settings-list and navigation tests, snapshot `palette_settings_level`.

### D100. Put one setting back to its default — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** after trying values there was no way back to a default short of editing the file. A reset-all was considered and not done (uncommon in terminal apps, and it would wipe key bindings and deliberate choices); one setting at a time is the useful part.
- **Decision (owner):** `Ctrl+R` on a row of `Settings ›` puts that setting back to its default, applied at once. The file gets the default **written out** (`timestamps = "relative"`), not the line commented out, so it says what's in use; the cost, accepted: that setting no longer follows a later change of faro's default. Settings that differ from their default get a `*` before their value. Footer hint `ctrl+r default`; notice "Timestamps: back to its default (relative)", or "… is already at its default".
- **As built (Judgement):** `PaletteKey::Reset` (`Ctrl+R`, palette only, not rebindable like the other palette keys); `settings::reset` sets the default through `Config::set` and goes through the same `apply` as a reload (so live effects match editing the file), then `settings::save` writes the key; `update/palette.rs::reset` refreshes the row in place. `palette::Setting::key` maps settings to config keys; `Setting::changed` compares `Config::value` with the default's; `Item::changed` draws the `*` in the theme's primary colour. Disabled rows (D99) can't be selected, so `Ctrl+R` can't reach them. Test: `ctrl_r_puts_a_setting_back_to_its_default_and_writes_it`.

### D101. Matched widths fill the dents of a run — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** in a joined compact run (D98), a short bubble between two wider ones (widths 32, 19, 41) left a notch on the ragged side, so the run's outline wasn't convex.
- **Decision (owner):** under `match_widths`, first fill dents: each bubble is widened to the narrower of the widest bubble above it and the widest below it in the run, so the ragged edge only steps outward down to the widest bubble and inward after it (32, 19, 41 → 32, 32, 41; 30, 10, 12, 35 → 30, 30, 30, 35; a bubble sticking out, 20, 40, 20, stays). Then D98's 3-cell matching runs on the result. Any run length. Not a true convex hull: steps can't be cut diagonally on a grid; "no dents" is the intent.
- **As built (Judgement):** `messages::line_up` (pure: prefix and suffix maxima, then chains within `MATCH_CELLS`), applied by `matched_width` to the run around a message, at most `MATCH_REACH` (64) messages either way, once per window; every member's width is kept in `Context::matched` so a bubble and its neighbours always agree. Runs longer than that may keep a dent where two such windows meet. Tests: `line_up_fills_dents_then_matches_close_widths`, `match_widths_fills_the_dent_of_a_short_bubble_between_wider_ones`.

### D102. Links stay on one line when they can — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** links in messages were split over lines, so the terminal's link detection opened only part of them. Two causes: the wrapper broke inside URLs at `/`, `.` and `:` (Unicode word boundaries) whenever the line already held something, and a bubble's 75 % width cap split links longer than that.
- **Decision (owner):**
  - **Wrapping:** a whitespace-separated chunk holding an http(s) link is one unbreakable word: it moves to a line of its own instead of splitting, in both layouts. Only a link wider than a whole line is still split (between graphemes, as before). Other text breaks as before.
  - **Bubbles:** a message whose link (with the punctuation stuck to it) is wider than the 75 % cap may grow up to the whole pane, just enough for the link; other bubbles are unchanged.
  - **Later (backlog):** terminal hyperlinks (OSC 8), so a link wider than the pane still opens whole. Until then the full link is in the message menu (`Enter` › open/copy link) and in `Ctrl+P` › *Links & files in this chat*.
- **As built (Judgement):** `text::words` (whitespace runs; a non-space run with a link from `selection::find_links` is kept whole, other runs split at word boundaries) feeds `wrap_ranges`; `bubbles::Frame::max_inner_for(message, pane)` used by both `measure` and `render`, so compact-run neighbours agree. Tests: `text::tests::links_move_to_a_line_of_their_own_instead_of_splitting`, `ui::tests::a_link_longer_than_a_bubble_widens_it_instead_of_being_split`.

### D103. Every standard emoji, with Unicode's official keywords — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** the `:` search (D83) knew about 100 hand-picked emoji, each by one or two names; `:cool` didn't find 😎, whose official CLDR keywords include "cool" (phone keyboards such as SwiftKey search those keywords).
- **Decision (owner, on the agent's recommendations):**
  - **Data:** Unicode's `emoji-test.txt` (Emoji 18.0) and CLDR 48.2.3's English annotations, pinned, turned into a compiled-in table by `tools/emoji/gen_table.py` (committed output `crates/faro/src/ui/shortcodes/table.rs`, ~140 KB). Unicode License v3, credited in `THIRD_PARTY_NOTICES.md`.
  - **One entry per emoji, no skin-tone variants** (1,906 instead of ~3,800); a skin-tone preference may come later.
  - **Up to Emoji 16.0:** probed with cursor reports in tmux 3.6b, every emoji up to 17.0 is two cells wide and Emoji 18.0's nine are not (and have no CLDR keywords yet); 17.0's eight are left out too, as newer than glibc 2.43's tables (mosh) and many emoji fonts. Every included emoji passes faro's own checks (unchanged by `sanitize` in full mode, two cells, drawable).
  - **The old names stay** as aliases (`:thumbsup:`, `:+1:`, `:tada:`…), ranked first.
- **As built (Judgement):** each emoji's code is its CLDR name in lowercase ASCII words joined by `_` (`smiling_face_with_sunglasses`, `flag_cook_islands`, `keycap_hash`), unique; results are labelled `:code:` (or the alias matched), and `:code:` typed in full is replaced by its emoji (`shortcodes::exact`). Ranking: exact alias, exact code, alias prefix (`heart` → `heart_eyes`), exact keyword (`cool` → 😎, 🆒), code prefix, prefix of a keyword or name word (`coo` → cook, cookie, Cook Islands), code substring; ties by the everyday list, then Unicode's order. An empty query still lists the everyday emoji. In compatible mode (mosh), joined emoji may come apart into their parts as before (D47, D55). Tests: `official_keywords_find_emoji`, `aliases_name_emoji_in_the_table_and_codes_are_unique`, `typing_a_whole_code_gives_its_emoji`, and the validity test now over the whole table.

### D104. Reaction picker: recent reactions, a search hint, a longer list; reactions in Message info — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** the picker's quick row always showed Signal's six default reactions, and nothing on screen said that typing searches every emoji (D83, D103), so the owner took the six as a limit. Signal itself takes any single emoji as a reaction. The search showed at most 8 matches, with no way to see more. Message info (`i`) didn't say who reacted with what.
- **Decision (owner):** show a hint, raise the match limit, and fill the quick row with recent reactions rather than the defaults. Add reactions to Message info.
- **As built (Judgement):**
  - **Quick row:** `State::quick_reactions()` gives the reactions last sent from here, newest first, then `REACTIONS` (Signal's defaults) until there are six. Emoji are compared without U+FE0F (`state::same_emoji`), so `❤` and `❤️` count as one. Most recent first, not fixed slots: `Enter` on the first slot repeats the last reaction, and the digits shift as you react. With no history the row is exactly the old one.
  - **What counts:** only reactions sent from faro (`ReactionChange::Set`), from the quick row or from search. Taking one back isn't a use. Reactions synced from other devices are left out, so replays at startup (D64) can't reorder the row.
  - **Kept in `app_state`** under `ui.recent_reactions` (space-separated): read with the conversation list at startup (`AppEvent::Loaded::recent_reactions`), written by `Effect::SaveRecentReactions` after each reaction, both on the store worker. Stored entries faro doesn't know (`shortcodes::is_known`, or a default) are dropped.
  - **Hint:** a muted `/ search` at the end of the quick row, and `/ search` in the footer. Letters start a search too, but `h`/`l`/`q` move or close in the quick row, so the hint names `/`.
  - **Search:** up to `REACT_SEARCH_LIMIT` = 50 matches, `REACT_SEARCH_ROWS` = 10 shown at once (fewer in a short pane). The window follows the selection, and an `n/N` counter on the prompt line shows the position when not everything fits.
  - **Picking your own:** the picker opens on your current reaction and underlines it, comparing without U+FE0F. Choosing it again removes it under the name it was sent with (D73).
  - **Message info:** one `Reaction:` row per reaction (`🔥 Alice`, `👍 You`), the same emoji together in the order each first appeared, yours first within its emoji. Reactions have no timestamps in the store, so none are shown.
  - Tests: `quick_reactions_prefer_recent_ones`, `reactions_toggle_and_are_undone_on_failure` (updated), `reaction_search_scrolls_past_the_visible_rows`, the Message info test, and the `reaction_picker` snapshot.

### D105. getfaro.dev: a static landing page with the real faro demo running in the browser — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** faro needs a website, and the best way to show a terminal program to someone who hasn't installed it is to let them use it. `faro demo` (D87, D88) already has everything a visitor needs: fake contacts, scripted scenes, no account, nothing sent.
- **Decision (owner):** a single static page at getfaro.dev, built around a terminal panel that runs `faro demo` compiled to WebAssembly, entirely in the browser. The full design and plan are in [WEBSITE.md](WEBSITE.md); its §8 lists the individual decisions:
  - **The demo:** preloads and starts by itself on capable desktops, unfocused until clicked. Phones and slow connections get a poster rendered from the real app, and so does anyone with JavaScript off. It uses a fixed seed, no outages, and starts fresh on every visit.
  - **Hosting:** a self-managed server behind nginx, building on the server, with an atomic symlink switch.
  - **Analytics:** self-hosted Plausible Community Edition, served from the site's own origin. No cookies, no third-party requests. The site has its own privacy policy and terms.
  - **Code:** one repo and workspace. `crates/faro` is to be split into `faro-app` (shared state, reducer, UI) and `faro` (the native binary), plus a new `faro-web`.
- **Judgement (to settle in the plan's Phase 0):** the renderer. A throwaway prototype of each, a custom Canvas 2D renderer (which draws block, octant and Powerline glyphs itself) and Ratzilla's WebGL2 backend, judged on fidelity first; keep one. Also the webfont.
- **Rejected:**
  - **xterm.js:** its own width tables would bring back the emoji misalignment fixed in D47/D55.
  - **WASI in a browser terminal:** crossterm has no WASI support, and it would need SharedArrayBuffer headers.
  - **A server running `faro demo` for each visitor:** hosting cost and a sandboxing burden, since the demo still touches real files (D87).
  - **Third-party hosting and a hosted analytics service.**

### D106. The web demo's renderer: a custom Canvas 2D renderer, not Ratzilla — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** D105 left the renderer to a Phase 0 bake-off (WEBSITE.md §6.3, D-d): a custom Canvas 2D renderer against Ratzilla's WebGL2 backend, judged on fidelity first, then speed and size.
- **How it was tested (Judgement):** both drew the same real faro frames, exported by an ignored test (`crates/faro/src/ui/web_spike.rs`). The frames come from the demo's cast and the deterministic mock (seed 42, no outages), run through the real store, reducer and `ui::render`: rounded bubbles, flags and ZWJ emoji, seven themes, the palette, the logo and 60 flood frames. Both used the same webfont and size. The prototypes are under `spikes/renderers/` (throwaway): `canvas/canvas.js` and `ratzilla/`.
- **Results:**
  - **Fidelity:** Canvas 2D matches the frame cell for cell. Ratzilla can only draw glyphs its font atlas has, so every octant edge, Unicode 16 quarter circle and Powerline cap showed as a missing-glyph box. Half blocks left thin lines between rows (the logo), and italic slanted block glyphs. Its built-in static atlas drew wrong emoji. The owner confirmed by eye: Ratzilla "almost passable", Canvas 2D "looks perfect".
  - **Speed** (owner's machine, 60 flood frames, full redraws): Canvas 2D avg 1.64 ms, max 5.0 ms; Ratzilla avg 0.58 ms, max 7.4 ms. Both are within the 4 ms budget on average.
  - **Size:** Ratzilla, beamterm and serde_json alone are 1.1 MB gzipped; the Canvas renderer is a few KB.
- **Decision (owner):** keep the custom Canvas 2D renderer and drop Ratzilla. Block elements, octants, sextants, quarter circles, Powerline caps, box drawing and Braille are drawn as shapes; text comes from the webfont (D107), and emoji from the system's colour emoji font, centred in the cells faro gave them.
- **Found while building it (rules for the real renderer):**
  - **Wide glyphs:** ratatui leaves the cell behind a wide glyph with no background. Terminals paint the wide glyph's background across every cell it covers, so the renderer must too. Without that, every flag had a dark box beside it.
  - **Braille:** the logo's beam and stars are Braille characters made at runtime (`0x2800 + dots`), so they are drawn as dots, not taken from a font.
- **Rejected:** Ratzilla/beamterm, for the fidelity and size above. Patching beamterm to draw block glyphs itself would mean maintaining a fork.

### D107. The site's webfont: JetBrains Mono, subset — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** WEBSITE.md §5.2 asked for one self-hosted monospace webfont of 60 KB or less, to be picked in Phase 0.
- **What the font must cover (Judgement):** found by `spikes/fonts/charset.py` from faro's UI and demo sources and the exported frames. The renderer draws blocks, octants, box drawing, Powerline and Braille (D106); emoji and other scripts (CJK, Arabic, Devanagari) go to system fonts at faro's width. That leaves ASCII, Latin-1, Latin Extended-A and about 30 symbols: `– — ‘ ’ “ ” • … · ‹ ›`, `← ↑ → ↓ ↳`, `≤ ≥ ⊘`, `■ ▶ ▸ ▾ ◂ ○ ◌ ●`, `✓ ✗ ✎ ❝`, `�`.
- **Compared** (`spikes/fonts/coverage.py`; the same frame drawn by the Canvas renderer at 13 and 14 px with each subset):
  - **JetBrains Mono:** missing `↳ ✎ ❝`; 20.7 KB regular + bold, plus 11.4 KB for the italic.
  - **Cascadia Mono:** missing 5 of the symbols; 24.6 KB.
  - **Iosevka:** missing `✎ ❝`; 24.6 KB, but 0.5 em wide, so it needs about 16 px to fill the same width, and it crowds emoji.
  - **IBM Plex Mono:** missing 13; **Fira Code:** missing 10.
- **Decision (owner):** JetBrains Mono (OFL-1.1), self-hosted as three subset woff2 files (Regular, Bold, Italic), about 32 KB in total. Faro draws quotes in italic, and a real italic looks better than the browser's fake slant. The three missing symbols `↳ ✎ ❝` are drawn as shapes by the renderer, so they look the same on every OS. Credited in `THIRD_PARTY_NOTICES.md` and the site footer when the site is built.
- **Rejected:** Iosevka (too narrow at a readable size), Cascadia Mono (a close second on looks; more missing symbols), IBM Plex Mono and Fira Code (too many missing symbols).

### D108. The mock draws the same on every target; a throwaway wasm build of it — Accepted (owner, 2026-10-11; details Judgement; the throwaway build superseded by D110)

- **Why:** M13 Phase 0 builds `MockBackend` and `Sim` for wasm32 (WEBSITE.md §7). The same seed told a different story there: fastrand's `choice` and `shuffle` pick indices with `usize`, which takes 32 random bits on wasm32 and 64 on 64-bit targets. The poster is rendered natively and must match the live demo's first frame (§6.4).
- **Decision (owner):** `mock/sim.rs` uses its own `pick` and `shuffle`, which draw `u64` indices. On 64-bit targets they make exactly the draws fastrand makes, so native event sequences, tests and snapshots are unchanged. The first 300 events of seed 42 are byte-identical on wasm32 and natively (`spikes/mock-wasm`).
- **As built (Judgement), throwaway until M13 Phase 2:**
  - `faro-core` has a default `native` feature (sqlx, etcetera, rustix; the `store` and `paths` modules) and a `wasm-spike` feature (gloo-timers, wasm-bindgen-futures, web-time). `cargo build -p faro-core --target wasm32-unknown-unknown --no-default-features --features wasm-spike` builds the core; wasm32 without `wasm-spike` stops with a `compile_error!`.
  - `mock/rt.rs` gives the mock its clock, sleep and spawn: tokio natively (tests still pause it), browser timers on wasm32.
  - Phase 2 replaces these with the planned seams (§6.5).
- **Superseded by D110** (the features and `mock/rt.rs`); the `pick`/`shuffle` decision stands.

### D109. Server files for getfaro.dev in `site/deploy/`; two changes to the runbook draft — Accepted (owner, 2026-10-11; details Judgement)

- **Why:** M13 Phase 0 prepares the server from WEBSITE.md §6.10. The owner runs the server steps; the Plausible dashboard's hostname is `plausible.harababurel.com` (owner).
- **As built (Judgement):**
  - **Files and runbook:** `site/deploy/README.md` (the runbook, step by step); `site/deploy/nginx/{getfaro.dev.conf, plausible.harababurel.com.conf, snippets/getfaro-headers.conf}`; `logrotate/getfaro`; `plausible/{compose.override.yml, write-env.sh}`.
  - **Deploying:** `update.sh` pulls, builds into `releases/<commit>/` through `site/build.sh`, switches `current` atomically and keeps 5 releases. Until Phase 1, `site/build.sh` copies `site/placeholder/` and gzips it.
  - **Checked locally** with Debian's nginx 1.30.4 and a fake Plausible (`spikes/deploy-test/check.sh`, throwaway): config syntax, gzip_static, security headers, the www redirect, the proxied script and event endpoint (Host and `X-Forwarded-For`), the dashboard websocket upgrade, release pruning, and `.env` written `0600` and never overwritten. TLS, Docker and DNS weren't tested here.
- **Changed from the §6.10 draft:**
  - **Tracker snippet:** Plausible CE 3.1+ gives each site its own script (`/js/pa-….js`) and an *inline* `plausible.init()`, which `script-src 'self'` blocks. nginx proxies `/js/s.js` to the per-site script (`PLAUSIBLE_SCRIPT` in the template, filled in by the owner), and the init lives in a same-origin file, `/js/init.js`, loaded first. The CSP stays as designed.
  - **Log folder:** getfaro.dev's logs go in `/var/log/nginx/getfaro/`, not `/var/log/nginx/getfaro.dev.*.log`. Debian's `/etc/logrotate.d/nginx` already matches `/var/log/nginx/*.log` (14 days), and logrotate rejects two rules for one file. Retention stays 30 days, as the privacy policy says.
  - **Smaller changes:** the dashboard's websocket `Connection` header comes from a `map` with a name of its own (`$plausible_connection_upgrade`), so it can't clash with another site's map; `DISABLE_REGISTRATION=invite_only` until the owner's account exists, then `true`.

### D110. faro-core on the web: a runtime shim and an in-memory store engine — Judgement (M13 Phase 2, 2026-10-11)

- **Why:** the web demo runs the mock and the store in the browser (WEBSITE.md §6.5), where there's no tokio runtime, no file system and no SQLite. D108's throwaway features were only enough for the mock.
- **Decision:**
  - **`faro_core::rt`** (was `mock/rt.rs`): `Instant`, `sleep`, `sleep_until`, `spawn` (an abortable `Task`) and `since_unix_epoch`. Natively tokio's (tests keep the paused clock; `rt::Instant` *is* `tokio::time::Instant`); on wasm32 browser timers, `web-time` and `wasm_bindgen_futures::spawn_local`. The store, the mock and the guard use it.
  - **One feature, `sqlite`** (on by default), replaces `native` and `wasm-spike`. wasm-only and native-only dependencies are chosen by target, not by feature. `paths` is native-only by target. faro-core builds for wasm32 with `--no-default-features` (CI); with `sqlite` on wasm32 it stops with a `compile_error!`. The workspace dependency has default features off; the binary asks for `sqlite`.
  - **A second store engine, not a second store:** `Store` serves the same `Request`s from an `Engine`, `Sqlite(Db)` or `Memory(Mem)`, so the reducer and the effects worker can't tell them apart. `Store::memory()` opens the memory one. `store/mem.rs` mirrors `db.rs` query by query: senders sort by their string form (SQL compares the TEXT column), search lowercases ASCII only (SQLite's `lower()`), a replaced reaction keeps its place (an upsert keeps its row id), and the checks run in SQL's order so a failed write changes nothing.
  - **Every engine-agnostic store test runs on both** (`engines!` generates `name::sqlite` and `name::memory`); the four that use real files stay SQLite-only. The memory engine passed all 21 on its first run.
  - Pure attachment-name helpers (`attachment_file_name`, `sanitize_file_name`, `extension_for`, `content_type_for`) moved from `paths` into an always-built `file_names` module, which `paths` re-exports.
- **Rejected:** a separate "web store" type (two APIs to keep in step); keeping D108's features (they fenced whole modules by feature where the line is really the target).

### D111. The shared app in its own crate, `faro-app`, with seams instead of `cfg` — Judgement (M13 Phase 2, 2026-10-11; the split itself is WEBSITE.md D-e, option A)

- **Why:** the web demo runs faro's real reducer and renderer (D105). Fencing native code with `#[cfg]` inside `crates/faro` would spread through many files and only the wasm CI build would catch a missing fence (§6.5, option B).
- **Decision:**
  - **`crates/faro-app`** (a library): `app::{event, files, image_cache, input, keymap, palette, state, term_event, timeline, update}`, `theme` (with the bundled `themes/`), `ui`, `version` (with `build.rs`), and `fixtures` (test-only, and for the binary's tests behind a `test-support` feature). Moved with `git mv`. **`crates/faro`** keeps the loop and the outside world and re-exports faro-app's modules under its own `app::`, `ui` and `theme`, so its code didn't change.
  - **No target `cfg` in faro-app.** What it needs from the platform comes in through types it owns:
    - **Terminal input:** `app::term_event` mirrors the part of crossterm's event types faro uses (same names and shape, so call sites only changed their imports). The binary converts at the boundary (`tui/input.rs`). Keys faro has no name for become `KeyCode::Other` rather than being dropped, so they still count as input (focus, catching up on receipts), and modifier bits are copied as they are.
    - **Time:** `faro_core::rt::Instant`; `jiff` with its `js` feature on wasm32.
    - **Images:** the `ImageCache` trait (`active_protocol`, `font_size`, `get` → a `DrawImage`) with faro's own `ImageProtocol`, `CellSize` and `ImageKey`, and the pure sizing and protocol choice. The binary implements it with ratatui-image; `NoImages` is for renderers without images. Finished decodes go straight from the decode tasks into the cache on its own channel (`Images::next_decoded`), not through the reducer: `AppEvent::ImageDecoded` carried a ratatui-image type and the reducer ignored it.
    - **Plain data out of native modules:** `Command` and `ClipboardError` into `app::event`, `Terminal` into `app::state`, the file-query and pasted-path helpers into `app::files`, `grid` into `ui::text`.
  - **Kept honest by tools:** `faro-app/tests/architecture.rs` fails if its manifest gains crossterm, ratatui-image, tokio, notify, mio, rustix, sqlx, clap or faro-presage, SQLite in faro-core, ratatui's crossterm backend, or loses ratatui's `std` feature (without it, and without crossterm, ratatui builds `no_std` and its layout cache can't link; workspace builds hid that because the binary's crossterm turns `std` on). CI runs clippy on faro-app for wasm32.
  - **Snapshots moved with their tests:** insta names files after the crate, so `faro__ui__tests__*` became `faro_app__ui__tests__*`; only the `source:` header changed. The ui tests use `fixtures::Undecoded`, which reports what the real cache does before decoding; the one test that decodes a real image moved to the binary (`app/images.rs`).
  - `app::run` grew past clippy's 100 lines with the decode branch; it has an `#[expect(clippy::too_many_lines, reason)]` rather than a split loop.
- **Rejected:** re-exporting crossterm's types by target (faro-app would then depend on crossterm natively, and the web build would need a `cfg` switch); a `Box<dyn Any>` payload to keep decodes in `AppEvent`.

### D112. `faro-web` starts as a stub that draws a real frame — Judgement (M13 Phase 2, 2026-10-11)

- **Why:** Phase 2 is done when `cargo build -p faro-web --target wasm32-unknown-unknown` passes in CI (§7); the shell is Phase 3.
- **Decision:** `faro_web::first_frame(width, height)` builds a frame the way the demo will: `MockBackend` (seed 42, no outages or errors, no live activity), its backlog applied to `Store::memory()`, `State` loaded through the real reducer, the newest chat opened with Enter, rendered by `ui::render` with `NoImages` into a ratatui `TestBackend`. It waits for the backlog with `rt::sleep` polls (the mock sends it in one go after `Connected`), so it runs on either target. A native test checks the frame (the chat list, an open chat with its messages, connected). CI builds and lints it for wasm32.
- **For Phase 3:** it's a plain library for now, under the workspace lints. The wasm-bindgen module (a `cdylib`), its own lint table (§6.5: wasm-bindgen generates `unsafe` code), which chat opens first, the browser time zone and the poster match (§6.4) all come with the shell. *(Phase 3, D114: the `cdylib` and the time zone are done; no lint table of its own was needed; nothing opens first, as in the terminal (D60); the poster match waits for the poster.)*

### D113. Store effects and messaging commands run through shared code — Judgement (M13 Phase 3, 2026-10-11)

- **Why:** the web demo needs the terminal app's store worker and backend command task. Two copies would drift, as the two store engines would without their shared tests (D110).
- **Decision:**
  - **`faro_app::app::storage::execute(&Store, Effect) -> Option<AppEvent>`** does every store effect: loads, search, receipts, writes, and the `app_state` keys `ui.last_open_conversation` and `ui.recent_reactions`. A failure is logged and returned as `AppEvent::StoreFailed`; a non-store effect is dropped with an error in the log. Moved unchanged from `crates/faro/src/app/effects.rs`.
  - **`faro_app::app::commands::execute(&B: Backend, Command) -> Option<AppEvent>`** does send, react, edit, delete for everyone, read receipts and typing. `Fetch` and `Transcribe` return `None`: each platform does those itself (files and processes natively, memory in the browser, D117). The binary copies outgoing attachments into `attachments/` first, then calls it.
  - **`update::affects_view`** (whether an event costs a frame) moved out of the binary's `app.rs`, so both loops redraw on the same events.
  - **Not shared:** the worker loops themselves (about ten lines each: receive, execute, send the result). The binary runs them on tokio, the web demo with `rt::spawn`, and faro-app stays free of tokio (D111's architecture test).

### D114. The web shell: a session, one event loop, listeners that only post — Judgement (M13 Phase 3, 2026-10-11)

- **Why:** WEBSITE.md §6.2: the browser's version of `app::run`, drawing on `requestAnimationFrame` and only when something changed.
- **Decision:**
  - **`session.rs`, the parts without the page:** `Session::start` wires the mock as visitors get it (`visitor_mock()`: seed 42, default activity, no outages or errors, D-b), `Store::memory()`, `State` with `Profile::Demo`, the browser's environment (native block glyphs, half-block images, truecolour, `EmojiMode::Exact` (D118)), the store worker and command task (D113), and the demo config (D116). Then it asks for the chat list. Nothing opens by itself: the visitor starts on the logo screen, as in the terminal (D60). Nothing here touches the DOM, so the whole pipeline is tested natively: open a chat and send, copy and the files notice, voice notes.
  - **`live.rs`, the loop:** one `select!` over page input, backend events, worker results, the next animation frame (requested only when dirty), the reducer's `next_deadline()` (`Tick`) and the logo timer. The timer runs only while the last frame drew the logo moving, the same rule as `app::logo_frame` (D61). `ui::render` draws into `CanvasBackend`, which remembers the rows that changed, and the painter repaints only those (D106). The parent's size comes from a `ResizeObserver`: the grid is refitted, the font measured again if the device pixel ratio changed, and ratatui's last frame forgotten. faro's window title becomes `document.title` (the page's own when faro has none). The last 256 frame times are kept (`Timings`, D119).
  - **`dom.rs`, the listeners:** they read the event, send `live::Input` down a channel, and never borrow the state. A `focus()` call made from Rust makes the browser dispatch events at once, which would find the state borrowed. The one exception is the keydown chord check (D116), which uses `try_borrow` and gives faro the key if the state is busy. **Every listener is non-passive:** a passive listener's `preventDefault` is ignored, so Tab would move the page's focus and the wheel would scroll the page. The wheel is faro's only while the canvas has the focus. Paste is heard on the document and taken when the canvas is the active element. The context menu is suppressed, and the canvas gets `tabindex="0"`.
  - **`input.rs`, pure DOM-to-faro mapping (tested natively):** `key` names, `Shift+Tab` as `BackTab`, F1–F24. Alt chords, and Ctrl chords whose character isn't ASCII, are read from `code` as on a US layout, because macOS turns Option+letter into another character (`Option+J` is `∆`). Mouse buttons, drags and the cell under a point. Wheel deltas become scroll steps (50 CSS pixels per step; line and page modes scaled).
  - **`web.rs`, what the page calls:** `start(canvas, font, size, columns, rows)`, where `0` means fill the parent, returns a `Demo`; `free()` stops it. Measuring and test hooks: `draw`, `frame`, `repaint`, `inspect`, `timings`, `text`. The canvas fires `faro-bell` and `faro-exit`.
  - **wasm32-only:** `dom`, `live`, `media`, `paint` and `web`. They need the page, and their tasks aren't `Send`, which `rt::spawn` requires natively.
  - **Lints:** faro-web stays under the workspace lints. wasm-bindgen's generated glue doesn't trip `forbid(unsafe_code)`, so the separate lint table planned in WEBSITE.md §6.5 and D112 wasn't needed.
  - **Time zone:** the visitor's UTC offset when the demo starts (`getTimezoneOffset`), fixed for the visit. jiff's time zone database would add hundreds of kilobytes to a demo with made-up dates.
  - **No logging in the browser yet:** faro-web installs no `tracing` subscriber, so `tracing` calls go nowhere. §6.2 planned the console in debug builds.

### D115. Effects in the browser go to the page, or become a notice — Judgement (M13 Phase 3, 2026-10-11)

- **Why:** WEBSITE.md §4.8: what a browser can't do must say so plainly and never leave the UI waiting.
- **Decision:** `router.rs` (`Router::dispatch`):
  - Store effects go to the store worker and `Effect::Backend` to the command task.
  - `CheckTerminal` is answered at once with `BROWSER`: no mosh, block glyphs drawn natively, half-block images, not kitty.
  - `SaveSetting` and `DesktopNotify` are dropped: settings last for the visit (D-c).
  - `Copy` uses `navigator.clipboard.writeText`; if the browser refuses, a notice says so. Copying a demo attachment's path gets the notice instead, because the path means nothing outside the demo.
  - `OpenUrl` opens only `http(s)` links, in a new tab with `noopener,noreferrer`; anything else is `OpenFailed`.
  - `Bell` fires `faro-bell` on the canvas, and the page decides how it looks (the test page flashes the panel's border).
  - `CheckPaste` keeps a pasted path as text.
  - `PlayAudio` and `StopAudio` go to Web Audio (D117).
  - `OpenFile`, `SaveToDownloads`, `ListFiles`, `StageFile` and `ReadClipboardImage` answer `AppEvent::Unavailable` with "Not in the web demo: try it in your terminal".
- **`AppEvent::Unavailable(String)`** (new in faro-app): a warning notice that also closes the file picker (`Level::AttachFile`) and clears a pending clipboard read (`State::pasting`), so nothing waits for an answer that won't come. The terminal app never sends it.
- **A `Page` trait** (`copy`, `open`, `bell`, `play`, `stop`) stands between the router and the DOM (`dom::CanvasPage`), so the router is tested natively with a fake page.

### D116. The demo's settings and keys, and quitting it — Judgement (M13 Phase 3, 2026-10-11)

- **Why:** WEBSITE.md §4.3: the browser keeps some chords, and faro must not bind or advertise them.
- **Decision:**
  - **`demo.toml`**, compiled into the module (`include_str!`) and read by faro's own config parser (a test checks it has no problems):
    - `[voice] transcribe = "local"`, which here means the canned transcripts (D117);
    - `ctrl+w` unbound in the input, since it closes the tab (Alt+Backspace and Ctrl+Backspace still delete a word);
    - `ctrl+v` unbound globally: the browser's paste event brings the text, and clipboard images are a terminal feature (D91).

    Bindings that go to `none` also drop out of the hints.
  - **Settings last for the visit (D-c):** `SaveSetting` is dropped (D115), and nothing goes to `localStorage`.
  - **Who gets a key:**
    - The browser always keeps some chords (`input::route`): Cmd/Win shortcuts, F5/F11/F12, Shift+Insert, Ctrl+V, Ctrl+Shift+I/J/C/R, the zoom keys, and Ctrl+W/T/N/Tab.
    - Keys without Ctrl or Alt are faro's while the canvas has the focus.
    - Any other Ctrl or Alt chord is faro's only if faro does something with it right now: the palette, a popup, or the focused pane's bindings (`keys::takes`). Otherwise reload, history and the like keep working.
  - **Quitting (`Ctrl+Q`):** the loop draws the last frame, stops, and fires `faro-exit`. The page frees the demo and offers to start a fresh one (§4.5). The test page is `crates/faro-web/www/index.html`.

### D117. Attachments, images, voice notes and transcripts in memory — Judgement (M13 Phase 3, 2026-10-11)

- **Why:** the demo's cast sends photos, stickers and voice notes, and the browser has no file system, processes or speech recognition (WEBSITE.md §4.8).
- **Decision:**
  - **`Blobs`:** a downloaded attachment's bytes stay in memory under a made-up path (`/faro-demo/<sender>-<sent at>-<index>`), given to the app as `AttachmentStatus::Downloaded(path)`, so the shared code treats it like a file. The command task does `Fetch` itself: the mock's bytes, and a voice note's duration and waveform from `audio::inspect_wav`.
  - **Images:** half-blocks, with the browser doing the decoding and scaling (`createImageBitmap` at high resize quality, drawn to a scratch canvas, read back with `getImageData`). That keeps image codecs out of the wasm module; §6.2 planned the `image` crate. `images::Images` implements `ImageCache` (32 images kept, oldest dropped first; 256-colour mapping as natively). Decodes come back to the loop as `live::Input::Decoded`.
  - **Voice notes:** Web Audio (`media::Audio`), one at a time. A generation counter makes sure a note still decoding never starts after a stop or a newer play. The `AudioContext` is resumed on play, for browsers that suspend it until the visitor interacts.
  - **Transcripts:** `TranscriptSource::Demo` (drawn `[demo]`), with a canned text for each mock voice note (Michael's and Andy's). There's no speech recognition and nothing is sent anywhere. It's in faro-core's domain and the store codec, but SQLite's `CHECK` constraint (`0007_voice_notes.sql`) doesn't allow `demo`. Only the memory store sees it, since the terminal app never produces it.

### D118. `EmojiMode::Exact` for renderers that draw each cell — Judgement (M13 Phase 3, 2026-10-11)

- **Why:** full emoji mode works around tmux (D55): it splits flags that touch, drops skin tones on bases tmux can't join, and keeps joining cells apart. The canvas draws each grapheme in exactly the cells faro gave it, so those workarounds would only damage the text.
- **Decision:**
  - **The mode:** `EmojiMode::Exact`, chosen when `EmojiSetting::exact` is set (the renderer draws cells itself). `EmojiSetting::full()` returns `Exact` there, so `auto` and `[ui] emoji = "full"` both arrive at it. `compatible` stays compatible.
  - **What Exact keeps:** VS16 added where it's missing (a browser draws ✌ as a wide colour emoji whatever its width says), half flags as letters, loose joiners trimmed, the cut-wide repair and the pinned wide cells.
  - **What Exact drops:** the tmux joining rules (`tmux_joins`, `separate_joining_cells`) and the skin-tone rewrite.
  - **Tests:** `exact_mode_keeps_what_tmux_would_break`. The terminal app sets `exact: false`, so its behaviour and snapshots are unchanged.
- **Differs from §6.2's "only the cut-wide repair":** the presentation and half-flag fixes are about the text itself, not the terminal, so they stay.

### D119. The web demo's size and speed, and how they're checked — Judgement (M13 Phase 3, 2026-10-11; size re-measured 2026-10-12)

- **Size**, measured on a release build of `9c7db9d` without `wasm-opt`:
  - `faro_web_bg.wasm` is 2,869,008 bytes: **1,018,339 gzipped** (`-9`) and about 746 KB with brotli `-q 11`. The budget is 1.5 MB compressed (§6.6), so it passes.
  - In Phase 3 testing, `wasm-opt -Oz` made the gzipped file slightly *larger*. `build.sh` still runs it when it's installed; CI doesn't install it, so the budget is checked without it.
- **CI:** the wasm32 job builds the module with `build.sh`, using wasm-bindgen-cli at the `Cargo.lock` version (`taiki-e/install-action`), and fails above 1,572,864 bytes gzipped. It hasn't been seen green yet: GitHub refused to start the jobs over billing.
- **Frame time** (budget: under 4 ms, §6.6), 1280×760 at device pixel ratio 2, on the logo screen:
  - **Debian, headless Chrome, software rendering:** live frames p50 2.7 ms, p95 8.3 ms; a whole frame from scratch 4.5–7.9 ms.
  - **Apple M1, headless Chrome** (`shot.mjs …?bench`, 2026-10-12): a whole frame from scratch averaged 2.14 ms (p95 2.3, max 2.7); repainting every row averaged 1.4 ms (p95 1.7).
  - **Result:** within the budget with a GPU. Under software rendering it holds at p50 but not at p95.
  - **While idle,** nothing is drawn: frames are requested only when dirty, and the logo timer runs only while the logo moves.
- **Deferred:**
  - **Batching text runs in the painter:** it draws one `fillText` per cell today. Batching would let JetBrains Mono's ligatures join characters across cells.
  - **Measuring live frames on a mid-range laptop** in each browser: that's part of the owner's checks (MANUAL_TESTS "Web demo").
- **How to measure:**
  - Build: `crates/faro-web/build.sh`, then `python3 crates/faro-web/tools/bundle.py` writes `www/dist/demo.html`.
  - Headless: `CHROME=<browser> node crates/faro-web/tools/shot.mjs '<url>?bench' out.png` prints `window.bench()` (`repaint` and `frame`).
  - Live frames: `demo.timings()` gives the last 256 frames' p50, p95 and max, split into render and paint.
