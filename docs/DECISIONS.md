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

- **Interim headless run:** until the TUI exists (M3), `faro` starts the mock backend, logs every event's *kind* (debug level; connection changes at info, errors at warn; never bodies, names or phone numbers), and on Ctrl+C prints and logs a one-line summary of event counts. The code is in `crates/faro/src/headless.rs`, to be deleted when the TUI replaces it.
- **Flags:** `--mock` is accepted and does nothing extra for now: the mock is the only backend until M7, which makes presage the default. `--mock-seed <u64>` (env `FARO_MOCK_SEED`, default `0`) picks the scripted activity; the cast never changes (D31).
- **Runtime:** `main` stays synchronous for setup (paths, logging), then builds a **current-thread** tokio runtime and calls `block_on` (with an `#[expect]` on `main`) instead of using `#[tokio::main]`. One thread is enough for a TUI plus I/O tasks and suits presage's `!Send` futures (D30). Revisit if the store or rendering needs more threads.
- **Ctrl+C** uses `tokio::signal::ctrl_c` (tokio `signal` feature). Once the TUI is in raw mode, Ctrl+C arrives as a key event instead, so this only matters before M3.

### D33. Commit and push after every finished task — Accepted (2026-10-07)

- **Decision (owner):** whenever a well-defined chunk of work is finished (a PLAN task or a milestone), agents commit and push it without asking, once the quality gate (fmt, clippy, tests) is green. This supersedes D23.
- Commits stay small with short imperative messages. Pushed history is never rewritten.
