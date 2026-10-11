# faro: design

Status: agreed design, written before any code (2026-10-07). Companion docs: [DECISIONS.md](DECISIONS.md) (why), [PLAN.md](PLAN.md) (implementation order and progress), [BACKGROUND.md](BACKGROUND.md) (context and research). Keep this file current when the design changes; items marked **Open** are not decided yet.

## 1. Why faro exists

The owner uses Signal by SSH-ing into a personal Mac mini and running a terminal Signal client inside tmux. The existing client (gurk) works, but has problems that faro is meant to fix:

- Keybindings are hard to discover; the help screen is an unformatted wall of text.
- The conversation list fills up with every contact ever seen, with no way to hide or prune them.
- Typing freezes while incoming messages are being fetched.
- No configuration or settings UI inside the app.

faro is a terminal (TUI) Signal client focused on a polished, discoverable, never-blocking interface, inspired by **OpenCode** (command palette, panels, input box, themes) and **btop** (meaningful colour, glyph-based graphics).

## 2. Goals and non-goals

### Goals

1. **Never block the UI.** Network, decryption and storage run off the UI thread; typing is always responsive.
2. **Discoverable.** Every action is reachable from a fuzzy command palette that also shows its keybinding.
3. **Curated conversation list.** Hide, archive, pin, mute; optionally show only an allowlist.
4. **Works great over SSH + tmux**, and in modern terminals (kitty, WezTerm, Ghostty, iTerm2, foot, Alacritty).
5. **Configurable from the UI and from a file**, with hot reload; the file has commented defaults.
6. **Themeable**, with a large set of built-in themes.
7. **Easy to install** on macOS and common Linux distros as a single binary.

### Non-goals (for now)

- Being a primary Signal device. faro is always a *linked device* (like Signal Desktop).
- Voice/video calls.
- Stories.
- Windows support (not ruled out later; not designed against).
- Reimplementing any part of the Signal protocol or service layer (D4).

## 3. Decisions

All decisions, their reasoning and rejected alternatives are in [DECISIONS.md](DECISIONS.md). The most important ones, in short:

- TUI over SSH/tmux, not web (D1). Rust + ratatui (D3).
- Never reimplement the Signal protocol or service layer (D4). presage as the first backend (D5), behind a `Backend` trait (D6).
- faro owns history and UI state in its own SQLite DB via sqlx (D7, D16).
- AGPL-3.0 (D8). TOML config with `toml_edit` + hot reload (D9). OpenCode-compatible themes (D10).
- Name **faro**, lighthouse logo (D11, D12).
- Develop against a mock backend; a separate test Signal account is needed from milestone M7 (D15).

## 4. Architecture

### 4.1 Overview

```
┌──────────────────────────────── faro process ────────────────────────────────┐
│                                                                              │
│   terminal input ──AppEvent──▶ ┌────────────────┐ ──Command──────▶ Backend    │
│   (crossterm)                  │    App core    │                  (presage)  │
│                                │ state, reducer │ ◀─BackendEvent──     │      │
│   ratatui view ◀───render───── └────────────────┘                     ▼      │
│                                   ▲         │ ▲               presage store  │
│   timers, config/theme ───────────┘         ▼ │              (keys, sessions)│
│   file watcher                          faro store                           │
│                                         (SQLite: history, UI state)          │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Single process, async (tokio).** Each component is a task communicating over channels.
- **App core** owns all UI state and is updated by a reducer: `fn update(state, event) -> Vec<Effect>`. It never awaits network or disk directly; effects are dispatched to tasks. This is what guarantees no freezing (Goal 1).
- **Render loop** draws on state change, capped to a frame rate (e.g. 60 fps for animations, idle otherwise), using synchronized output (DEC mode 2026) where supported to avoid flicker.
- **Backend task** wraps presage; translates between presage types and faro domain types. The app drives it through a **command task** (`app/commands.rs`) that owns the backend and runs the reducer's `Effect::Backend(Command)`s one at a time, reporting results as `AppEvent`s. Backend futures may be `!Send` (D30), so that task is spawned with `spawn_local` and the whole app runs inside a `LocalSet` on the current-thread runtime (D39).
- **Store task** owns the SQLite connection; all writes go through it.
- **Watcher task** watches config and theme files for hot reload.

### 4.2 Later: daemon mode

`faro daemon` runs the backend + store headless (e.g. under launchd/systemd) so messages arrive while no UI is open; `faro` (the TUI) then attaches over a local Unix socket. The channel boundaries in 4.1 are designed so this becomes a transport change, not a redesign. Not in the first release.

### 4.3 Workspace layout (D20)

```
crates/
  faro/            # binary: CLI args, TUI (ratatui views), app core
  faro-core/       # domain types, Backend trait, events, store (SQLite), config, themes
  faro-presage/    # Backend impl on presage (heavy deps isolated here; added in M7)
.github/workflows/ci.yml   # fmt, clippy, test on Linux + macOS
Cargo.toml         # workspace metadata and shared lints (D25, D27, D28)
clippy.toml        # banned blocking calls, test relaxations (D28)
rust-toolchain.toml        # pinned Rust version
tools/
  logos.py         # naming-phase logo prototypes
docs/
  DESIGN.md        # what to build
  DECISIONS.md     # why (decision log)
  PLAN.md          # milestones and progress
  BACKGROUND.md    # context and research
  MANUAL_TESTS.md  # checks only a human at a real terminal can do
AGENTS.md          # onboarding for coding agents
```

A **mock backend** lives in `faro-core` (`src/mock.rs`, `src/mock/`) and is always compiled in; users meet it as **`faro demo`** (D87), and tests use it directly. The demo runs in its own data directory (`<temp>/faro-demo-<uid>`, `--dir`, `--fresh`), never the real one: layered guards (a `.faro-demo` marker, ownership and symlink checks, no overlap with the real data dir or `FARO_DATA_DIR`, and a `profile` marker in each database checked by both demo and real runs) keep demo and real data apart. It shares only the config file. The cast is an affectionate parody of *The Office* (D88): you are the unnamed new temp at the Scranton branch, and coworkers, a Japanese supplier, a client in Cairo and Kelly's mom talk in written scenes (`mock/cast.rs`, `mock/scenes.rs`, attachments in `mock/assets.rs`), chosen so every UI stress case (very long names, a nameless contact, ZWJ emoji and skin tones, flags, right-to-left text, CJK, Devanagari, combining marks, multi-line and long messages, links, photos, stickers, voice notes, edits, deletes, replies, mentions, many quiet conversations) still appears; tests enforce that coverage. A backlog of whole scenes arrives on connect, then seeded live activity continues scenes and adds replies, reactions, edits, deletes, sync messages, outages and errors. With a fixed seed and tokio's paused clock its event sequence is deterministic (D31). `faro demo --seed` picks the scene order, `--activity <ms>` its pace, and `--no-faults` turns off scripted outages and errors (for flood tests, D35).

Inside `crates/faro`: `app.rs` (event loop), `app/` (state, reducer, effects worker, backend command task, in-memory timeline), `tui.rs` (terminal setup and restore), `theme.rs`, and `ui.rs` + `ui/` (pure rendering from `State`) (D35).

## 5. Backend interface

### 5.1 Trait

Implemented in `crates/faro-core/src/backend.rs` (D30). Shown here with the `impl Future` returns written as `async fn`:

```rust
pub trait Backend {                       // no Send bound: futures may be !Send (D30)
    fn capabilities(&self) -> Capabilities;

    async fn whoami(&self) -> Result<Account>;
    async fn contacts(&self) -> Result<Vec<Contact>>;
    async fn groups(&self) -> Result<Vec<Group>>;

    // faro picks msg.sent_at, so the MessageRef is known before sending;
    // delivery progress arrives as BackendEvent::DeliveryUpdate.
    async fn send(&self, to: ConversationId, msg: OutgoingMessage) -> Result<()>;
    async fn react(&self, conversation: ConversationId, target: MessageRef, change: ReactionChange) -> Result<()>; // Set(emoji) | Remove(emoji), D73
    async fn edit(&self, conversation: ConversationId, target: MessageRef, edit: OutgoingEdit) -> Result<()>;
    async fn delete_for_everyone(&self, conversation: ConversationId, target: MessageRef) -> Result<()>;
    async fn mark_read(&self, messages: &[MessageRef]) -> Result<()>;
    async fn send_typing(&self, to: ConversationId, state: TypingState) -> Result<()>;
    async fn fetch_attachment(&self, attachment: &AttachmentRef) -> Result<Vec<u8>>; // bytes; faro writes the file (D75)
}
```

- **Events** (`BackendEvent`: connection state, messages incl. sync, delivery updates, reactions, edits, deletes, typing, contact/group updates, unsolicited errors) come over a bounded `tokio::sync::mpsc` channel. Each backend's constructor returns `(backend, BackendEvents)`; the trait has no `events()` method.
- **As built (D62): the allowlist guard.** `faro-core::guard::Guarded<B>` wraps any backend when `--allowlist FILE` is given. Sends, reactions, edits, deletions and typing to conversations off the list fail with `BackendError::NotAllowed`. Receipts go only to allowed senders. Incoming events, contacts and groups off the list are dropped before the app sees them. A missing or bad file stops faro. The presage backend requires it while testing with a real account.
- **As built (D63): the presage backend.** `crates/faro-presage`: `convert.rs` maps presage's contents to `BackendEvent`s and builds outgoing messages; `backend.rs` implements `Backend` and the reconnecting receive loop (local tasks); `link.rs` links a device. Linking is the `faro link` subcommand (QR code on stdout); `faro ids` prints the IDs an allowlist needs. Outside `faro demo`, faro runs the linked account, behind `--allowlist` while testing or with `--no-allowlist` (D67).
- **As built (D70): reads on other devices.** A read sync from the phone becomes `BackendEvent::ReadElsewhere`; the store lowers each conversation's unread count to the incoming messages after the newest one read and reports by how much, which the reducer subtracts. The other way round (D71), `mark_read` sends a read sync to our own devices with the receipts, so the phone clears what was read in faro.
- **As built (D64): catching up.** `Backend::stored_events` returns what the backend kept (presage saves every message); `app::load_directory` applies it to the store before the first draw, through the guard. Group member lists supply profile keys, so profile names show without a contact sync.
- **Linking** is not in the trait: it happens before a backend exists and is backend-specific. Designed in M7.
- **Capabilities** are optional features only (`Edit`, `DeleteForEveryone`, `Attachments`). Text, replies, receipts, typing and reactions are required of every backend.

### 5.2 Rules

- **Domain types are faro's own**, modelled on Signal concepts, not on presage or signal-cli structs.
- **Identifiers are Signal's stable IDs:** service IDs (ACI UUIDs) for people, group IDs (derived from the group master key) for groups. Phone numbers are display data, not keys.
- **Sync messages** (messages you sent from your phone) are normalised into the same `BackendEvent::Message` with `direction = Outgoing`.
- **Capabilities** gate UI actions (e.g. hide "edit" if a backend can't edit).
- **Errors** are typed (`Network`, `Unlinked`, `RateLimited`, `Protocol`, `Storage`, `NotFound`, `Unsupported`) so the UI can show useful states instead of crashing. `is_transient()` says whether a retry may help. Error details never contain message bodies or phone numbers.

## 6. Data model (faro's SQLite)

presage keeps its own store for keys, sessions and protocol state; faro never touches it directly. faro's DB (`<data dir>/faro.db`, `0600`) holds history and UI state. Schema: `crates/faro-core/migrations/` (STRICT tables with `CHECK` constraints; D34).

| Table | Key fields |
|---|---|
| `recipients` | service_id (PK), contact name, profile name, phone (nullable) |
| `conversations` | id (PK; `direct:<service id>` or `group:<hex>`), kind (direct/group), peer (direct: recipient), title (groups), last_activity, unread_count |
| `group_members` | group_id, service_id |
| `conversation_state` | conversation_id, **hidden**, **archived**, **pinned** (order), **muted_until**, draft text, **allowed** (allowlist, D41) |
| `messages` | id, conversation_id, sender, sent_at (sender's timestamp), received_at, body, quote_sender, quote_sent_at, quote_preview, edited_at, deleted, direction, status (sending/sent/delivered/read/failed; outgoing only) |
| `reactions` | message_id, sender, emoji (one per sender) |
| `attachments` | id, message_id, backend_id, content_type, file_name, size, local_path (nullable), width, height, status (remote/downloaded/failed; D75) |
| `app_state` | key/value: sidebar widths, last open conversation, etc. |

- Messages are identified Signal-style by `(sender, sent_at)` (unique); faro adds its own row ID. History is paged backwards with `(sent_at, sender)` as the cursor.
- Planned columns not yet added (each a future migration when first used): recipient avatar path and colour seed, conversation scroll position. (Attachment download status came with M9, migration `0006`, D75.)
- SQLite via `sqlx` 0.9 with embedded migrations (D16), version-compatible with presage's store (D34).
- One store task owns the only connection; the app uses a cloneable `Store` handle over a channel (D22, D34). Applying a backend event is idempotent: duplicates are ignored, receipts never regress, only newer edits win.
- faro keeps its own history even though presage's store can also hold messages (D7).

## 7. UI

### 7.1 Layout

```
┌ Conversations ──────┐┌ Alice ─────────────────────────────────────┐┌ Info ─────────┐
│ ▸ Pinned            ││                                            ││ Alice         │
│   ● Alice        2  ││   Hey, are we still on for tonight?  19:02 ││ +1 555 …      │
│     Book club       ││                                            ││ Safety number │
│ ▸ Recent            ││ 19:03  Yes! 7pm at the usual place 👍      ││ Muted: no     │
│     Bob             ││                                            ││ Shared media  │
│     Mum             ││                                            ││               │
│                     ││ ╭────────────────────────────────────────╮ ││               │
│                     ││ │ Type a message…                        │ ││               │
│                     ││ ╰────────────────────────────────────────╯ ││               │
└─────────────────────┘└────────────────────────────────────────────┘└───────────────┘
 ● connected · 3 unread · NORMAL       ctrl+p commands · ↑ select message · alt+enter newline
```

- **Left sidebar:** conversations (sections: pinned, recent; archived behind a toggle). Unread counts, mute markers, typing indicator.
- **Main pane:** message list + **input box** rendered as a distinct panel with its own background colour (OpenCode style).
- **Read receipts and typing (D42):** a message is read only once it's actually seen (terminal focused, view at the bottom); only then are receipts sent and the unread count cleared. Reading a chat on the phone clears its count here too (D70). Typing indicators: "started" on the first edit and every 10 s, "stopped" after 3 s idle, on an empty input, or on leaving the conversation.
- **Right sidebar (optional):** conversation info, members, safety number, shared links/files.
- **Footer:** connection status, unread total, mode, and **context-sensitive key hints** (change with focus).
- Sidebars are **collapsible**, **resizable** (mouse drag or keys), and **auto-hide on narrow terminals**. Widths persist in `app_state`.
- **As built in M3 (D35):** fixed widths (left 30, right 28), auto-hidden below 70 and 110 columns; `Alt+S`/`Alt+I` toggle them. Pinned/Recent headers appear only when something is pinned. Resizing comes in v0.2.

### 7.2 Command palette (`Ctrl+P`)

- Fuzzy search (`nucleo`) over **commands, conversations and settings** in one list.
- Every entry shows its **keybinding** (if any) right-aligned, and its **path** for nested items (`Settings › Messages › Alignment`).
- Prefixes: `>` commands only, `@` conversations only, `:` settings only.
- Recently used commands rank higher.
- Sub-menus exist for browsing, but **typing never requires opening a sub-menu first**.
- **As built in M4 task 8 (D44):** `nucleo-matcher` over commands (global, the open conversation's curation with each mute duration as its own command, the selected message's actions, bulk hide), every conversation (hidden, archived and empty ones too, with notes) and, with `?`, the keybinding list. Nothing typed: recent commands, then conversations, then commands. `F1`, or `?` outside the input, opens the keybinding list. No settings until M5.
- **As built since D52 (owner's choice, 2026-10-08): a tree, not one flat list.** `Ctrl+P` opens the root: `Chats ›`, `Themes ›`, `Settings ›`, `Commands ›`, `Keys ›`. Typing filters **only the level shown** (same ranking; recent commands first in `Commands`). `Enter` (or `→` at the end of the query) opens a category; `Enter` runs a leaf and closes; `←` at the start of the query, or `Backspace` on an empty one, goes up a level (the item that led down is selected); `Esc` closes. The title is the breadcrumb (`Settings › Notifications › Bell`). At the root, `@` `:` `>` `?` jump into chats, settings, commands and keys (what follows becomes that level's query); `Themes` has no prefix. `F1`/`?` open `Keys` directly. Settings: `Appearance` (Theme ›, Colours, Emoji, Message layout), `Notifications` (Bell, Unread count in the window title), `Conversations` (Allowlist mode), and `Mouse`; each setting is a level listing its values as radio items (`●` current, `○` others), its current value shown beside it one level up. The "typing never requires opening a sub-menu" bullet above is superseded.
- **Since D57 (owner's choice, 2026-10-08): one settings list.** `Settings ›` shows every setting at once, each with its current value on the right, under **section headers** (`Appearance`: Theme ›, Colours, Emoji, Message layout, Bubble corners; `Notifications`: Bell, Unread count in the window title; `Conversations`: Allowlist mode; `Input`: Mouse). Headers can't be selected: `↑`/`↓`, pages, the wheel and clicks skip them. Typing filters across all settings in list order and keeps a header only above a match; a header's title matches its whole section (`notif` lists both notification settings). `Enter` on an on/off setting flips it in place (applied, saved, the palette stays open with the query kept); on any other setting it opens its values as before. `Theme ›` still opens the live-preview picker. The breadcrumb is at most two deep (`Settings › Colours`).
- **Sections since D99:** `Appearance` (Colours, Emoji, Reduce motion), `Messages` (Message layout, Timestamps), `Bubbles` (Bubble style, Bubble padding, Match bubble widths, Message density), then Notifications, Conversations, Media, Input. While the IRC layout is in use the `Bubbles` rows stay listed but dimmed, with "bubbles layout only" as their value; arrows, pages and clicks skip them and nothing acts on them (`Item::disabled`, `Setting::bubbles_only`).
- **Reset (D100):** `Ctrl+R` on a settings row puts that setting back to its default (applied, and written to the file with the value spelled out); settings that differ from their default show `*` before their value.
- **Since D57: a note at Themes** about the colour depth, a muted line between the rule and the list: at 256 colours "themes are approximated (truecolor: Settings › Colours)", at 16 "the system theme is drawn; others need 256 colours or more"; nothing at truecolor.
- **Planned for M12 (D83, owner's choice):** in `Settings ›`, `←` and `→` cycle through the highlighted setting's values in place (applying and saving immediately with live preview), while `Enter` opens the vertical sub-menu (`Level::Setting(s)`). Choosing a setting value inside `Level::Setting(s)` steps back up to `Settings ›` (instead of closing `Ctrl+P`), and choosing a theme in `Themes ›` keeps `Themes ›` open so multiple settings/themes can be changed in one palette session.

### 7.3 Messages

- **Layout option:** `bubbles` (own messages right, others left) or `irc` (all left, `time  name  text`), plus a `compact` density toggle.
- Sender names coloured deterministically per person (hash → theme palette), like Signal/IRC.
- Replies render a dimmed quoted preview; reactions as a compact row (`👍 2  ❤️ 1`); edited messages marked `(edited)`; deleted shown as a dim placeholder.
- Delivery status glyphs for own messages: `◌` sending, `✓` sent, `✓✓` delivered, `✓✓` in the secondary colour for read, `! not sent` in the error colour for failed (D39).
- Unread marker line; date separators.
- Links are detected and openable; attachments show name/size and open in the system viewer.
- **As built in M4 task 9 (D45):** bubbles are the default (palette: "Use the IRC layout" / "Use the bubbles layout", remembered in `app_state` until M5). Bubbles size to their text, at most 75% of the pane, with 1 cell of padding on `bubbleOwn`/`bubbleOther`; others left, own right; time, `edited` and delivery glyph at the bottom right inside the bubble; reactions under it; names above others' first bubble of a run in groups only; one blank line between bubbles. No `compact` toggle yet.
- **As built (D58, superseding D56):** `[messages] bubble_style = "auto" | "rounded" | "bar"`. **Rounded:** one-line bubbles get Powerline half-circle caps (U+E0B6/U+E0B4) in place of their padding cells; taller ones get two-thirds edge rows with diagonal corners above and below (`🭊🬹🬹🬿` … `🭥🬎🬎🭚`, Legacy Computing), caps and edges in the bubble's colour on the pane's background. **Bar:** a heavy `┃` on the bubble's outer side in place of that padding cell (others: the sender's colour; own: the theme's primary), like OpenCode's message boxes. **Auto** (default) is rounded only where every terminal between faro and the user is kitty (which draws those glyphs itself; elsewhere fonts often lack them): `TERM`/`TERM_PROGRAM`/`KITTY_WINDOW_ID` outside tmux, every attached client's `#{client_termname}` inside it, rechecked with D47's mosh check. **Mixed (D66):** the bar on the outer side, rounded on the inner one (square edge corners on the bar's side). The `system` theme always draws bars. A blank line separates bubbles in every style; a selected bubble is a plain tinted block (caps and edges blanked, the bar kept).
- **As built (D98, superseding D58's taller-bubble edges):** taller rounded bubbles are rounded rectangles drawn on a grid of octants (`ui/bubbles/grid.rs`, 2 × 4 per cell): half-cell sides (`▐` `▌`), quarter-row edges (`▂` above, `🮂` below) and Unicode 16 quarter circles (U+1FBEC–1FBEF) at the corners; `mixed` the same on its rounded side. One-line bubbles keep their capsule caps. `[messages] bubble_padding` (on) puts a blank cell inside each rounded side. **Compact runs:** with `density = "compact"` and a rounded style, consecutive bubbles from one sender (same day, within the name-grouping window, no reaction row or unread rule between) join: they touch, share one edge on their aligned side, rounded only at the run's top and bottom, and a wider bubble's edge and corner are drawn in its neighbour's row past the neighbour's end (cells shared by two bubbles use octants U+1CD00–1CDE5). One-line bubbles in a run are rectangles too. `[messages] match_widths` (on) first fills dents (each bubble at least the narrower of the widest above and the widest below it in the run, D101), then widens bubbles within 3 cells of a joined neighbour to the widest of that chain. Compact keeps a blank line between runs (other senders, or after a reaction row).
- **Emoji:** rendered as-is. Widths are computed with grapheme clustering; a setting replaces problematic sequences (ZWJ families, flags, skin tones) with a fallback where terminals disagree. Emoji picker with `:shortcode:` fuzzy search.
- **As built in M4.5 (D47):** `--emoji auto|full|compatible` (config setting in M5). Compatible mode drops VS16, ZWJ, keycaps, skin tones and tags from graphemes that terminals adding up code points (mosh) measure differently (❤️ → ❤, 👨‍👩‍👧‍👦 → 👨👩👧👦); auto picks it when a mosh client is attached. A lone regional indicator is drawn as its letter in every mode. Copy keeps the original text. `Ctrl+L` repaints the screen.
- **As built after D55:** in every mode, two flags side by side get a space between them (tmux would join them into one cell), graphemes starting with a joiner or modifier or ending with a ZWJ lose those, and skin tones on bases tmux 3.6 doesn't join (🫶🏽, 🏃🏽) are dropped; emoji tmux always draws two cells wide (☝ ✌ ✍ 🖐…) always get VS16. Compatible mode also draws flags as their letters (🇷🇴 → RO), and those emoji as a near equivalent (✌ → ☮, ☝ → 👆).

### 7.4 Message selection and actions

- `↑` on an empty input enters **message selection**.
- Actions on the selected message: reply, react, copy, edit (own), delete, open link/attachment, info.
- The same actions appear in a **context menu**, opened by right-click or `Enter` on a selected message. Every mouse action has a keyboard equivalent.

**As built in M4 task 4 (D40), M9 (D76) and M10 (D81–D82):** selection lives in the message pane. Entering it (`↑` on an empty input, `Tab`, or a click) selects the message on the bottom row; the selected message has a tinted background and a `▌` bar in the margin, and the view scrolls to keep it visible. `r` reply (a "↳ Replying to …" bar above the input; `Esc` cancels), `+` react (picker with your recent reactions topped up with Signal's defaults, D104, and `/` to search every emoji; picking your current one removes it), `y` copy (OSC 52), `o` open link (copied instead over SSH; several links → a menu), `d` download attachment, `v` view image / open attachment (or play/stop a voice note), `s` save attachment to Downloads (§7.9), `x` play/stop voice note, `t` transcribe voice note (§7.10). `Enter` or right-click opens the action menu, which also lists each link, attachment/voice-note actions, and "Retry sending" for failed messages. Popups sit just above (or below) the message they act on. Edit (`e`), delete for everyone (`D`), message info (`i`), forward (`f`), search (`/`), and jump to unread (`u`) are planned in M12 (D83).

### 7.5 Mouse

Click to focus panes and open conversations; scroll anywhere; drag sidebar borders (border highlights on hover); right-click for context menus; click toggles on panel headers. Mouse capture can be turned off (setting + palette command) so native terminal text selection works; `Shift`+drag bypasses capture in most terminals. Copy uses **OSC 52**, which works over SSH and tmux.

**As built in M4 task 1 (D37):** click a conversation to open it, click the message area or input to focus it, wheel scrolls whatever is under the pointer. While scrolled up, a "↓ N new messages" badge shows on the bottom row.

### 7.6 Default keybindings (confirmed, D13)

Chosen to avoid tmux's default prefix (`Ctrl+B`) and common terminal shortcuts. All rebindable. On macOS, `Alt` bindings need the terminal's "Option as Alt/Meta" setting (kitty `macos_option_as_alt`, iTerm2 "Esc+", Terminal.app "Use Option as Meta key"); otherwise Option+letter types a symbol (D36).

| Key | Action |
|---|---|
| `Ctrl+P` | Command palette |
| `Ctrl+Q` | Quit (`Ctrl+C` asks for confirmation) |
| `Alt+K` / `Alt+J` | Previous / next conversation (also `Alt+↑` / `Alt+↓`, D36) |
| `Alt+1`…`Alt+9` | Jump to pinned conversation N (D41) |
| `Tab` / `Shift+Tab` | Cycle focus between panes (conversations, messages, input; D37) |
| `Esc` | Close popup / remove staged file / cancel the reply / clear; back to the input; then close the open conversation (D37, D60, D77) |
| `↑` (empty input) | Select the newest message (focuses the message pane, D40) |
| `r` `y` `+` `o` `d` `v` `s` | (In selection) reply, copy, react, open link, download / view / save attachment (D40, D76) |
| `Enter` | Send; in selection: open the message's action menu (D40) |
| `Alt+Enter` | Newline (also `Ctrl+J`; `Shift+Enter` when the kitty keyboard protocol is available) (D38) |
| `Ctrl+A/E`, `Alt+B/F`, `Ctrl+W`, `Alt+D`, `Ctrl+U/K` | Input editing: line start/end, word left/right, delete word back/forward, delete to line start/end; arrows, `Home/End` and `Alt`/`Ctrl`+arrows too (D38) |
| `Alt+S` / `Alt+I` | Toggle left / right sidebar |
| `Alt+N` | Do not disturb: the bell off / on (D53) |
| `Ctrl+V` / `Ctrl+O` | Paste an image from the clipboard (kitty) / show the staged image before sending it (D91) |
| `F1` or `?` (in non-input focus) | Help: opens the palette listing all keybindings |
| `PgUp` / `PgDn` | Scroll messages by a page, from any pane (D37) |
| `↑↓` / `j k`, `g G` / `Home End` | In the sidebar: switch conversation, first/last (D37). In the message pane: select an older/newer message, oldest/newest loaded; `↓` past the newest goes back to the input (D40) |
| In a popup | `↑↓`/`←→`/`j k` move (`h l` too in the reaction picker), `Enter` chooses, `Esc` closes; menus take their keys as shortcuts, the pickers digits (D40, D41) |
| `p` `K`/`J` `m` `h` `a` `l` `L` `A` `Space` | In the sidebar: pin, move pin up/down, mute, hide, archive, allowlist add/remove, allowlist mode, chats ⇄ archive, conversation menu (D41) |

**As built in M4 task 7 (D43):** the bindings above are a table (`app/keymap/defaults.rs`) per context (`global`, `input`, `conversations`, `messages`); the focused pane's bindings win over global ones. Every bindable action has a stable name (`conversation.next`, `message.react`, …) used by the config file (§9), and footer hints read their keys from the key map. Popup keys are fixed.

### 7.7 Conversation list curation

Hide (disappears until a new message arrives, configurable), archive, pin, mute (with durations), and an optional **allowlist mode** that only shows chosen conversations. Default: conversations with no messages are not shown. Bulk "hide all older than N days" from the palette.

**As built in M4 task 5 (D41):** the sidebar has two lists: the **chats** (Pinned, Recent; ending in a "▸ Archive (N)" row) and the **archive** (Archived, "Hidden until a message", "Not on the allowlist"; starting with "◂ Chats"). `A` or a click on those rows switches. Sidebar keys act on the highlighted conversation: `p` pin (`K`/`J` reorder pins), `m` mute (1 hour … until unmuted), `h` hide, `a` archive, `l` allowlist, `L` allowlist mode, `Space` or right-click for a menu. Hidden conversations return on any new message; archived ones don't. With allowlist mode on, the chats show only allowed and pinned conversations. When the open conversation leaves the list, the next one opens. Muted conversations are dim and left out of the unread total. Allowlist mode is stored in `app_state` until the config file exists (M5). Bulk hide is in the palette ("Hide conversations inactive for 1 month", D44). Not built yet: configurable hide behaviour (M5).

**Names (D65):** a person is shown by their nickname (set in faro, kept only in faro), else their address-book or profile name, else their phone number, else the start of their ID. When two people faro knows have the same name, both get the first 4 hex digits of their ID after it ("Alex · 3f9a"). Nicknames are set from the conversation menu, the open chat's commands, or `Commands ›` › "Set a nickname…".

### 7.8 Graphics and motion

Colour carries meaning (btop style): status, unread, mentions, own vs others, errors. Glyph graphics where they add information (e.g. activity sparkline in the info panel, connection quality). The empty message pane (no chat open) shows the animated lighthouse logo (D61); an about screen may reuse it later. A `reduce_motion` setting disables animations.

### 7.9 Attachments and images (M9, D75–D79)

**Receiving (D75).**
- A backend only hands over bytes: `Backend::fetch_attachment(&AttachmentRef) -> Result<Vec<u8>>` (was a path). faro writes the file itself, so where files live and their permissions are decided in one place.
- Files go to `<data dir>/attachments/` (`0700`), one file per attachment (`0600`), named `<sent_at>-<sender's first 8 hex>-<n>-<file name>`, with the name reduced to safe characters (or `attachment.<ext>` from the content type). Each file is written to a temporary name and renamed, so a half-written file is never seen.
- `AttachmentId` is opaque to faro. The presage backend puts the whole `AttachmentPointer` in it (`p1:` + hex of the protobuf: CDN location, key, digest, size). presage's `get_attachment` needs all of that, and the store keeps it like any other id. Rows stored before M9 hold only the CDN id. They are refreshed when the backend replays the message (D64): a replayed message updates its attachments' ids by position.
- `AttachmentRef` gains `width`/`height` (from the pointer, or read from the file's header after a download) and `status`: `Remote`, `Downloaded(path)` or `Failed`. Migration `0006` adds `width`, `height` and `status` (`local_path` already exists). "Downloading" is in memory only (`State::downloads`), so nothing is ever left stuck in it.
- **When:** images download by themselves once they're in the open conversation, i.e. loaded, arrived, or reached by paging back. The limit is 25 MB each (`AUTO_DOWNLOAD_LIMIT`). Anything else (bigger images, videos, files, or everything with `[media] auto_download = "off"`) waits for the user: `d`, the message menu, or the palette. `"all"` downloads every kind up to the limit. Failures are never retried by themselves; the message shows `download failed` and `d` tries again.
- Downloads run on the command task's side (`Command::Fetch`), several at a time (at most 3), never in the reducer or on the UI thread. A finished download is `AppEvent::Fetched`; the reducer updates the timeline and the store (`Effect::SaveAttachment`).
- **Progress:** presage hands over the whole file at the end, so faro shows *downloading…* rather than a percentage. The same goes for uploads (the message shows `◌` until sent).

**On the message (both layouts, D78).** Each attachment adds one line, `📎 name · size`, plus its state when it isn't simply downloaded: `· downloading…`, `· not downloaded (d)`, `· download failed (d)`. An image that's downloaded, or about to be, also gets an image box of its own size above that line (below). With inline images off, or for other files, there is only the line.

**Open, save, copy path, view (D76).** The message menu lists, per attachment:
- *Download* while it isn't downloaded.
- Once it is: *Open* (`open`/`xdg-open`, local sessions only), *Save to Downloads* (`$XDG_DOWNLOAD_DIR` or `~/Downloads`, created if missing; `name (1).ext` if taken), *Copy path* (OSC 52, so it works over SSH) and, for images, *View* (a popup as big as the screen allows).
- Over SSH (`State::env.over_ssh`) *Open* is left out, because it would open on the remote machine. *View*, *Save* and *Copy path* remain.

Keys in the message pane: `d` download, `v` view (for an image, else open locally, else save), `s` save. Each acts on the selected message's first attachment; with several, a menu asks which. The palette's open-chat commands offer the same for the selected message.

**Sending (D77).**
- *Send a file…* (palette, open-chat commands) opens a path picker in the palette. The query is a path (`~` expands); the list shows that folder's entries, read off the UI thread (`Effect::ListFiles` → `AppEvent::FilesListed`), folders first; dotfiles only when the query's last part starts with `.`. `Enter` on a folder goes into it; `Enter` on a file attaches it.
- A bracketed paste that is exactly one path (quotes, `file://` and backslash escapes removed) is checked off the UI thread (`Effect::CheckPaste`). If it names a regular file, the file is attached; otherwise the text is pasted as usual. This is what dragging a file into most terminals does. Over SSH it only works for files on the machine faro runs on.
- **One file per message** for now. An attached file shows as a bar above the input (`📎 name · size  esc remove`), beside the reply bar. The input text becomes the caption, and `Enter` sends even with no text. Files over 100 MB (Signal's limit) are refused with a notice.
- On send, the command task first copies the file into `attachments/` (the local echo points at the copy, so history and *Retry* don't depend on the original), then calls `Backend::send` with `OutgoingMessage::attachments`. The presage backend uploads (`upload_attachment`) and sends the pointer. The guard checks the conversation before anything is uploaded (D62).

**Pasting images from the clipboard (D89–D91).**
- `Ctrl+V` (or *Paste an image from the clipboard* in the open chat's palette commands) asks kitty for the clipboard over OSC 5522. This works directly in kitty and over SSH, with or without tmux (the request is wrapped in passthrough), but not over mosh. Other terminals get "Pasting images needs kitty".
- faro lists the clipboard's types first (no prompt), then asks for the best image type (PNG, JPEG, GIF, WebP) with a per-run password, so kitty can offer "allow all" until faro restarts. Every kitty attached to faro's tmux session answers; the first list with an image wins.
- **Reading the reply (D90):** the app loop drops crossterm's `EventStream` and reads the tty itself with mio + `FIONREAD`, then makes a new stream. While faro waits for kitty's prompt (2 min), it keeps drawing and receiving, but keys typed into it are dropped. Focus reports found in the reply become focus events again.
- A reply that comes after faro stopped waiting reaches crossterm as keys (`Alt+]`, `5522;`…). The late-reply guard (armed after the first paste) holds such keys back for up to 50 ms and drains the rest of the reply from the tty.
- The image is saved as `attachments/clipboard-<local time>.<ext>` (`0600`) and **staged, never sent**: the bar shows `📎 name · W×H · size  ctrl+o view ·  esc remove`, the name shortened first. `Ctrl+O` opens it in the full-size viewer; `Enter` sends it like any attached file, with the input as its caption. Limit: 25 MB. The pasted file is deleted once sending has copied it, and leftovers (removed with `Esc`, or staged at quit) are swept at the next start (D93).

**Inline images (D78, D84).**
- `ratatui-image` 11 (MIT), default features off (no libchafa), with `image` decoding JPEG, PNG, GIF (first frame) and WebP. It is used through `sliced::SlicedProtocol`, which can draw an image cut off at the top or bottom of the pane, as scrolling needs.
- **Protocol:**
  - `[media] inline_images = "auto"` (default) asks the terminal once at startup, after entering the alternate screen and before input reading starts (`Picker::from_query_stdio`, on a blocking thread). The answer is kitty graphics (unicode placeholders, which survive tmux with passthrough), sixel, iTerm2, or half-blocks.
  - Through mosh, or when tmux has several clients on different terminals (D58's check, extended), it falls back to half-blocks.
   - Auto disables images at 16 colours; at 256 colours it uses half-blocks mapped to the nearest xterm colours. Explicit half-blocks are also disabled at 16 colours.
   - `"kitty"`, `"sixel"` and `"iterm2"` force that graphics protocol, overriding automatic colour-depth and terminal-hop fallbacks. They require a compatible terminal and tmux passthrough; they cannot work through mosh. No confirmation dialog or extra stdin query. Without startup detection, image sizing uses the picker's estimated cell pixel size.
  - `"halfblocks"` never asks; `"off"` shows only the attachment line.
   - Switching from `off` to `auto` while running gives half-blocks until the next start, because faro can't query once input reading has begun.
   - `Ctrl+P › Settings › Inline images` lists all six modes. Its note explains Auto's current result/fallback reason, or the requirements of a forced mode, and updates while open. Settings apply live and save through the normal config writer; returning to Auto reuses any startup detection result.
  - Inside tmux, `ratatui-image` turns `allow-passthrough` on for faro's pane (`tmux set -p`), which kitty, sixel and iTerm2 need there.
- **Size:** decided at layout time from `width`/`height` and the cell's pixel size (from the query; half-blocks assume 1:2). The image keeps its shape inside at most 40 columns (less in narrow panes) and 14 rows. Both layouts lay out the box as blank rows, so windowing, scroll anchoring and selection see ordinary rows. The renderer then draws each visible image over its rows (`Window::images`: where, how big, which file).
- **Decoding** happens off the UI thread: the frame reports the images it lacked (`Geometry::images_wanted`); the app loop decodes and encodes them with `spawn_blocking` into `app/images.rs::Images`, a cache keyed by file and box size, which drops the least recently drawn beyond 32. Until an image is ready, its box is drawn as a placeholder (panel colour, *loading image…*). The cache is emptied when the protocol changes and on `Ctrl+L`, so kitty images are sent again after a clear or when another terminal attaches.
- **Overlays and frame safety:** while a popup or the palette is open, kitty, sixel and iTerm2 images are drawn as their placeholders and half-blocks keep drawing. This has two reasons. The terminal-side image would otherwise show through or leave holes. And kitty's one-time image upload rides on the image's first cell, which an overlay drawn in the same frame would erase. The image viewer is itself a popup and draws its image last. The "more below" badge clips images above it. `emoji::finish` skips image cells. `tools/frames` gets an `images` scenario (half-blocks) for the D55 replay.
- **Stickers (D79)** are images too: the sticker's pointer becomes an attachment (`image/webp`, no file name), drawn at most 8 rows tall. The body stays `[sticker] 😀` for copying, replies and terminals without images.

### 7.10 Voice notes and transcription (M10, D80–D82)

**Identification, download and audio metadata (D80).**
- Signal sets `AttachmentPointerFlags::VoiceMessage` (`flags & 1 != 0`) on voice notes, mapped by `faro-presage::convert::attachment` onto `AttachmentRef::voice_note: bool`. When the flag is absent, an `audio/*` attachment with no file name or a `voice-*` / `signal-voice*` file name is also treated as a voice note (`AttachmentRef::is_voice_note()`).
- With `[media] auto_download` set to `"images"` (default) or `"all"`, voice notes up to 25 MB download automatically in the open conversation alongside images.
- When a voice note is downloaded, faro computes `duration_secs: Option<u32>` and a 16-bar RMS waveform (`Vec<u8>`, values `0..=7`) off the UI thread before emitting `AppEvent::Fetched`:
  - `faro-core::audio::inspect_wav` reads RIFF/WAVE PCM/float files directly in pure Rust.
  - For compressed formats (AAC/M4A, Ogg Opus), the worker invokes `ffmpeg -v error -i <path> -ac 1 -ar 8000 -f s16le -` (capped at 10 minutes) or falls back to `ffprobe` + a deterministic 16-bar envelope.
- Migration `0007_voice_notes.sql` adds `voice_note`, `duration_secs`, `waveform`, `transcript` and `transcript_source` to `attachments`, and `remote_transcription` to `conversation_state`.

**Row and playback (D80, D81).**
- In both `Bubbles` and `Irc` layouts, a voice note renders as its own row:
  - Idle: `▶ 0:14 ▂▄▆█▅▃▂▆█▄▃▁▃▅▄▂ · 42 KB` (plus `· downloading…`, `· not downloaded (d)`, `· transcribing…`, etc. when applicable).
  - Playing: `■ 0:05 / 0:14 ▂▄▆█▅▃▂▆█▄▃▁▃▅▄▂ · playing`, with played waveform bars in `accent` and remaining bars muted.
- Pressing `x` (or `v` when the attachment is a voice note) plays or stops the selected voice note through an external player (`[voice] player = "auto"`: `afplay` on macOS; `mpv --no-video --really-quiet`, `ffplay -nodisp -autoexit -loglevel quiet`, `pw-play`, `paplay`, `aplay -q` on Linux/Unix, or a custom command). At most one player child runs at a time; a 1 s timer (`AppEvent::PlaybackTick`) advances progress until the child exits (`AppEvent::PlaybackFinished`).
- **Over SSH or mosh (`State::env.over_ssh || State::env.mosh`):** sound would play on the remote host, so `Play` refuses with a notice pointing to `s` (*Save to Downloads*) and *Copy path* (OSC 52), and the message menu omits *Play*.

**Opt-in Whisper transcription (D82).**
- Controlled by `[voice] transcribe = "off" | "local" | "remote"` (default `"off"`). Users can transcribe a selected voice note manually (`t` in the message pane, *Transcribe* in the message menu or palette) or automatically for downloaded voice notes in the open conversation up to `[voice] transcribe_max_secs = 300` (5 minutes).
- **Local (`"local"`):** runs `[voice] transcribe_command` (default `"whisper-cli"`, also trying `whisper-cpp` and `whisper`) with `[voice] transcribe_model` (GGML `.bin` path) and optional `[voice] transcribe_language` (`"auto"` or ISO-639-1 code). Custom commands without a model path or containing `{file}` receive the audio path directly.
- **Remote (`"remote"`):** posts `multipart/form-data` to `<transcribe_url>/audio/transcriptions` (default `https://api.openai.com/v1`, model `[voice] transcribe_model = "whisper-1"`) via `curl --config -` so the bearer key is passed over stdin, never on the command line. The API key is read from `$FARO_TRANSCRIBE_API_KEY`, `$OPENAI_API_KEY`, or the OS keychain (`security` on macOS, `secret-tool` on Linux), and rejected with a warning if placed in `config.toml`.
- **Consent and per-conversation permission:**
  - Turning on `transcribe = "remote"` or running a remote transcription before consenting opens `Popup::ConfirmRemoteTranscription` naming `transcribe_url`; accepting records `voice.remote_consent = "true"` in `app_state`.
  - Remote transcription is also opt-in per conversation (`ConversationState::remote_transcription`, default `Disallowed`, toggled in the conversation menu, palette, or when confirming `t` on a voice note). Local transcription runs locally and needs neither remote consent nor the per-conversation flag.
- **Display:** transcripts are stored in `attachments.transcript` + `attachments.transcript_source` (`local` / `remote`), rendered below the voice note row as `❝ <text> [local]` or `❝ <text> [remote]`, and included in `State::readable_body` for *Copy* (`y`) and *Reply* (`r`).

## 8. Notifications

| Method | Works over SSH | Notes |
|---|---|---|
| Terminal bell (BEL) | Yes | tmux `monitor-bell` / visual bell |
| OSC 9 / OSC 777 / OSC 99 (kitty) desktop notifications | Yes | Under tmux requires `allow-passthrough on`; faro wraps sequences for tmux automatically |
| Window / tmux title with unread count | Yes | OSC 0/2 |
| Native OS notification | Local only | When faro runs on the same machine as the display |

Per-conversation overrides (mute, mentions only). **Privacy levels:** show nothing / sender only / sender + preview (default: sender only).

- **As built in M6 (D49):** bell and title only; desktop notifications are v0.2. A conversation *notifies* if it's in the main list (not archived, hidden or left out by allowlist mode), not muted, and set to every message (`State::notifies`). The bell (`[notifications] bell`) rings for a new incoming message in such a conversation unless it's already in front of the user (D42), at most once every 2 s. The window title (`[notifications] title`, OSC 0, pushed/popped on the terminal's title stack) is `faro (N)` with the footer's unread total over conversations that notify. Per conversation: mute (with durations) and "mentions only" (`@`).
- **Mentions (D72, M7):** a new message that mentions this account rings a "mentions only" conversation (`State::rings_for`); a mute silences mentions too. "Mentions only" conversations don't count in the title or footer total. Mentions are drawn as `@Name` in bold accent (reversed when it's us), in both layouts.
- **Do not disturb (D53, owner's choice):** `Alt+N` (`notifications.toggle_bell`) or the palette's "Turn do not disturb on/off" flips `[notifications] bell` and writes it back, so it persists and live-reloads like any setting. While the bell is off the footer says `bell off` in the warning colour after the connection status. The title count is unaffected.

## 9. Configuration

- **File:** `$XDG_CONFIG_HOME/faro/config.toml`, default `~/.config/faro/config.toml` on both Linux and macOS. `--config` / `FARO_CONFIG` override.
- **Data:** `$XDG_DATA_HOME/faro/` (default `~/.local/share/faro/`): faro DB, presage store, attachments, logs. `--data-dir` / `FARO_DATA_DIR` override. Files created `0600`, dirs `0700`.
- **First run** writes a config file with every option present, **commented out, showing its default**, with a one-line explanation.
- **UI edits** go through `toml_edit`: only the changed key is written; comments and layout are preserved.
- **Hot reload:** config and theme files are watched; changes apply live. Invalid config shows an error toast and keeps the last good config.
- **What lives where:** preferences (theme, layout, keybindings, notifications) in TOML; per-conversation state (hidden, pinned, muted, drafts) in SQLite. Syncing the hidden/allowlist set via config is a backlog item.

Sketch:

```toml
# theme = "opencode"
# reduce_motion = false

[messages]
# layout = "bubbles"        # "bubbles" | "irc"
# bubble_style = "auto"     # "auto" (rounded in kitty, bars elsewhere) | "rounded" | "bar" | "mixed"
# bubble_padding = true     # a blank cell inside each rounded side
# match_widths = true       # compact runs: line up bubbles within 3 cells of each other
# density = "compact"       # "comfortable" | "compact"
# timestamps = "relative"   # "relative" | "absolute" | "off"

[notifications]
# bell = true
# desktop = "auto"          # "auto" | "osc" | "native" | "off"
# privacy = "sender"        # "none" | "sender" | "preview"

[media]
# inline_images = "auto"   # "auto" | "kitty" | "sixel" | "iterm2" | "halfblocks" | "off" (D84)
# auto_download = "images" # "images" | "all" | "off" (D75)

[voice]
# player = "auto"                       # "auto" | command (D81)
# transcribe = "off"                    # "off" | "local" | "remote" (D82)
# transcribe_command = "whisper-cli"    # local Whisper binary or wrapper script
# transcribe_model = ""                 # local GGML .bin path, or remote model ("whisper-1")
# transcribe_url = "https://api.openai.com/v1"
# transcribe_language = "auto"          # "auto" or ISO-639-1 ("en", "ro", …)
# transcribe_max_secs = 300             # max duration auto-transcribed (seconds)

[mouse]
# enabled = true

[keybindings.global]          # also: input, conversations, messages (D43)
# "ctrl+p" = "palette.open"
# "alt+j" = "none"            # unbind
```

- **As built in M5 (D48):** settings are one table in `faro-core/src/config.rs` (template, parsing, write-back); I/O in `crates/faro/src/config.rs`; applied by `app/update/settings.rs`. Built: `theme` (built-ins only), `reduce_motion` (parsed), `[ui] emoji` (`--emoji` wins), `[conversations] allowlist`, `[messages] layout`, `[notifications] bell`/`desktop`/`privacy` (parsed, for M6), `[mouse] enabled` (live), `[keybindings.<context>]`. Not yet: `density`, `timestamps` (warn "isn't supported yet"). Theme files came with D54. M8 (D51) added `[ui] colors` (`auto`/`truecolor`/`256`/`16`) and three more built-in themes, both in the `:` palette. Each setting is read on its own: a bad value keeps its previous value and the footer names the key; invalid TOML keeps the last good config; unknown keys warn. Reloads (`notify` on the directory, 300 ms debounce) apply only what changed, so faro reading back its own writes is a no-op. Palette `:` lists each setting's values and writes the choice back. Allowlist mode and the layout moved here from SQLite `app_state` (migrated once at startup; the file wins).

## 10. Themes

- Theme files in **OpenCode's JSON format** (named `defs` + semantic roles), loaded from built-ins and `~/.config/faro/themes/` (as built: D54, below).
- Semantic roles used by faro include: `background`, `backgroundPanel` (input box, popups), `backgroundElement` (selection), `text`, `textMuted`, `primary`, `secondary`, `accent`, `border`, `borderActive`, `success`, `warning`, `error`, plus faro-specific roles (e.g. `bubbleOwn`, `bubbleOther`, `unread`, `mention`) with sensible fallbacks to standard roles. Built so far (M4): `bubbleOwn`, `bubbleOther`, `unread`, and the sender palette.
- A **`system`** theme uses the terminal's own 16-colour palette.
- Automatic **fallback to 256 colours** when truecolor isn't available.
- **What each depth draws (D51, D57):** at **truecolor** every theme exactly; at **256 colours** an **approximation**, each colour rounded to the nearest of the 240 fixed xterm colours, so themes built on subtle tints lose them: solarized's dark blue background `#002b36` has no near match and becomes the neutral grey 235 (`#262626`), a step from Solarized's own 256-colour fallback (234, `#1c1c1c`); at **16 colours** the **`system` theme**, whatever theme is set (faro can't see the 16 colours the terminal uses, so mapping onto them would only guess). Choosing a theme at 16 colours still saves it; it's drawn once the depth allows (a better terminal, or `Settings › Colours`). The Themes list says which case applies (§7.2).
- **Live preview** while moving through the theme picker; theme files hot-reload.
- **Licences:** OpenCode's themes are MIT (checked, D54); its licence ships beside them and in `THIRD_PARTY_NOTICES.md`.
- **As built in M3 (D35):** `Theme` lives in `crates/faro/src/theme.rs` (it holds ratatui colours, which `faro-core` can't depend on; parsing theme files can still live in `faro-core` later). One built-in theme, `faro-dark`, with faro's own colours; roles are those above plus `unread` and an 8-colour sender palette.
- **As built in M8 (D51):** four built-in themes with faro's own colours (nothing bundled from elsewhere, so no licence question): `faro-dark` (default), `faro-dusk` (cooler dark), `faro-light`, and `system`. `system` uses only `Color::Reset` and the 16 ANSI colours and sets `Theme::terminal_palette`: selections and badges are drawn in reverse video (`Theme::selected`, `Theme::inverse`), muted text is dim (`Theme::muted`), and bubbles have no fill. Renderers ask for `theme.selected()` / `theme.muted()` rather than building those styles from roles.
- **Colour depth (D51):** `ColorDepth::detect` at startup: `COLORTERM=truecolor|24bit`, or inside tmux (tmux rounds RGB to 256/16 colours per client when the outer terminal lacks RGB), or a known truecolor `TERM`/`TERM_PROGRAM` (kitty, Ghostty, Alacritty, foot, WezTerm, iTerm2, `*-direct`…) → truecolor; the Linux console, `vt*`, `*-16color` → 16; anything else → 256. `[ui] colors = "auto"|"truecolor"|"256"|"16"` overrides it (also in the `:` palette, live). At 256 colours every RGB colour is mapped once at theme load to the nearest fixed xterm colour (16–255, "redmean" distance), and surfaces that differ in the theme (background, panel, selection, both bubbles) are kept different; at 16 colours the `system` theme is used, whatever theme is set. The app loop keeps the loaded theme in `theme::Themes` and rebuilds it only when the name or depth changes.
- **As built (D54): theme files.** `faro-core::theme_file` parses OpenCode's JSON to RGBA per role and variant (hex `#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`, `defs` and role references, `transparent`/`none`, ANSI numbers, `{ "dark", "light" }`; resolution order as OpenCode's `resolveTheme`). `crates/faro/src/theme/import.rs` maps roles onto faro's (`background`, `backgroundPanel`, `backgroundElement`, `text`, `textMuted`, `primary`, `secondary`, `accent`, `border`, `borderActive`, `success`, `warning`, `error`; `info` becomes `unread`). Only `background`, `text` and `primary` are required; the rest fall back (panel → background, element → panel, muted → text blended into the background, secondary → primary, accent → secondary, info → secondary, border → muted, borderActive/success/warning/error → primary). Translucent colours are drawn over the background, a transparent background over black (dark) or white (light). Surfaces equal to the one under them are nudged towards the text so panels and selections stay visible. Derived: `bubbleOther` (between panel and element), `bubbleOwn` (others' bubble tinted with primary while text stays about as readable), `unread` (info, readable on background and selection), 8 sender colours (from the status, accent, syntax, markdown and diff colours: no greys, no near-duplicates, each ≥ 3:1 on the background and others' bubbles). Coloured and muted text that's under 3:1 on the background is moved towards the text just enough; everything else keeps the file's colour, so imported themes are not held to faro's own contrast rules (D51).
- **Bundled:** OpenCode's 33 themes (`crates/faro/themes/opencode/`, compiled in with `include_str!`, parsed on first use). Each file gives `name` (its dark variant) and `name-light` when the light variant differs. The Themes level lists faro's, then the user's, then OpenCode's dark and light ones, labelled on the right.
- **User themes:** `themes/*.json` beside the config file (`~/.config/faro/themes/`, created `0700` at startup), same format, named after the file. Read at startup and again (all of them) whenever one changes (`notify` on the directory, 300 ms debounce, `AppEvent::ThemesRead`). A broken file is skipped and named in a warning; a user theme replaces a built-in one of the same name, except `system`; an explicit `name-light.json` beats the light variant generated from `name.json`. Editing the theme in use redraws it at once (`Themes` caches by name, depth and the user themes' generation).
- **Live preview:** moving through `Ctrl+P › Themes` draws the selected theme (`Palette::preview`, `State::theme_name()`); `Enter` keeps and saves it, `Esc`, `←` or anything that leaves the level puts the configured one back unsaved. Opening the level starts on the theme in use. At 256 or 16 colours a note above the list says themes are approximated or that `system` is drawn (`palette::note`, D57); at 16 the preview, like the theme, is `system`.
- **As built (D60):** themes are listed once, as the root category `Themes ›`; the settings list has no Theme row.
- **As built (D60): no chat open.** faro starts with nothing open (the message pane shows the logo, D61) and nothing opens by itself. `Esc` steps back (reply, notice/focus) and then closes the open conversation, keeping its draft and focusing the list. With nothing open the list's highlight is a cursor: `j`/`k`/`Alt+J/K` move it without reading anything, `Enter` or a click opens, curation keys act on it.
- **As built (D61): the logo screen.** With nothing open the message pane shows the lighthouse logo (`ui/logo.rs`, art in `ui/logo/art.rs`), centred, in the largest form that fits: lighthouse + wordmark + tagline side by side (90 × 13), stacked (wordmark scale 2 or 1), a small theme-coloured lighthouse with the word, or the word alone; under it a status line ("Loading…", "No conversations yet") and key hints from the key map. The beam turns at 10 fps while the logo is visible, the terminal is focused and `reduce_motion` is off; otherwise it stands still and nothing is scheduled. Truecolor draws the logo's own colours, 256 colours their nearest xterm colours, 16 colours and the `system` theme only the small form in theme colours; light backgrounds get no stars and darker whites.
- **As built (D59): settings preview too.** The values of `Settings › Colours`, `Message layout` and `Bubble style` preview the same way: moving through them draws the selected value (`Palette::preview` is a theme or a `SettingChoice`; the renderer reads `State::colors()`, `message_layout()`, `bubble_style()`), `Enter` keeps and saves it, `Esc` or `←` puts the one in use back. A values level opens on the value in use. Emoji and the on/off settings don't preview.
- Mentions (D72) use `accent`, which must be readable on both bubble colours too (contrast rule ≥ 3); no separate `mention` role.
- **Not yet:** a separate `mention` role. `NO_COLOR` isn't honoured yet. Transparent backgrounds can't show the terminal's own background (they're drawn over black/white).

## 11. Terminal compatibility

- **Input:** crossterm. Use the **kitty keyboard protocol** when available (more distinguishable keys), but **all defaults must work without it** (tmux only partially supports it).
- **tmux** recommended settings (documented, detected where possible): `set -g mouse on`, `set -g set-clipboard on`, `set -g allow-passthrough on`, `set -as terminal-features ",*:RGB"`, `set -g extended-keys on`, `set -g focus-events on` (read receipts wait for focus, D42).
- **Width:** grapheme-aware width calculation (`unicode-segmentation` + `unicode-width`); test emoji-heavy content specifically under tmux.
- **mosh** measures emoji per code point, unlike kitty and tmux ≥ 3.5: faro switches to compatible emoji when any client of its tmux session (or faro itself) runs under `mosh-server` (D47). Other panes in the same tmux (e.g. a shell) still look wrong through mosh; that's outside faro's control.
- **Frame output:** every wide cell is sent with `ForcedWidth`, so ratatui never writes a wide character's hidden cell (D47).
- **tmux builds emoji one code point at a time** and, when it widens a cell for a flag or VS16 emoji, leaves whatever wide character started in the next cell half-cleared. faro's backend (`tui/backend.rs`) writes a blank into that next cell first (D55). `FARO_DEBUG_FRAMES=<file>` logs every intended frame, for comparing with what a terminal shows (`tools/frames/`).
- **Colours (D51):** truecolor is assumed inside tmux (tmux rounds RGB itself for clients without it) and in known truecolor terminals even when `COLORTERM` was lost over SSH; elsewhere 256 colours, mapped once at theme load; 16-colour terminals get the `system` theme. `[ui] colors` overrides the guess. Details in §10.
- **Images (M9, D78):** kitty graphics (unicode placeholders), sixel or iTerm2 where the startup query finds them, half-blocks elsewhere; through tmux by passthrough (faro turns `allow-passthrough` on for its pane), half-blocks through mosh or when tmux clients differ; none at 16 colours. Details in §7.9.

## 12. Security and privacy

- faro stores Signal keys (via presage) and message history on disk. Files are user-only (`0600`).
- Encryption at rest is deferred until after v0.1 (D18); store-opening code should accept a key later. Options: presage store passphrase, SQLCipher, OS keychain / Secret Service.
- Logs (`tracing`, to `<data dir>/logs/faro.log`, filter via `FARO_LOG`; D26) **never include message bodies or phone numbers** unless an explicit debug flag is set. Enforced by type: `MessageBody` and `PhoneNumber` redact in `Debug` and have no `Display`; the value is read with `.expose()` (D29).
- Notification privacy defaults to sender only.
- Device name shown in the phone's linked-devices list defaults to `faro (<hostname>)`.

## 13. Packaging and distribution

- **Not on crates.io yet** (presage depends on Signal's git-only crates). `cargo install --git` works; build needs `protoc`. When it can be published, the crate is `faro-chat` (installing the `faro` binary), since `faro` is taken there (D85).
- **Release binaries** via cargo-dist / GitHub Actions: `x86_64` and `aarch64` for Linux (glibc; musl if feasible) and macOS.
- **Homebrew:** own tap first (`harababurel/tap`, repo `harababurel/homebrew-tap`, created and empty, D86), homebrew-core later.
- **Arch (D86):** AUR `faro-git` (PKGBUILD in `packaging/aur/faro-git/`, ready, uploaded once the repo is public), then `faro` (release from source) and `faro-bin` (pre-built) with the first release; all `provides`/`conflicts` `faro`. Runtime depends: `glibc`, `libgcc`, `openssl` (SQLCipher). Official repos only if a packager adopts it.
- **Debian/Ubuntu:** `.deb` via `cargo-deb` attached to releases.
- **Nix:** flake in the repo.
- **Names (D85):** `faro` everywhere (repo, command, Homebrew, AUR, `.deb`, Nix); `faro-chat` only on crates.io. Website: `getfaro.dev`, designed in [WEBSITE.md](WEBSITE.md) (D105).

## 14. Release plan (confirmed, D14)

Task-level breakdown and progress tracking: [PLAN.md](PLAN.md).

### v0.1: usable daily driver (MVP)

- Link as secondary device (QR in terminal); receive and send text in direct and group chats
- Never-blocking architecture (§4.1); faro's own history DB
- Conversation list with hide / pin / mute; allowlist mode
- Command palette with keybinding hints; context-sensitive footer
- Input box panel; multi-line input
- Reply, react, read receipts, typing indicators
- Bubbles and IRC layouts
- Config file with commented defaults + hot reload
- Bell + title notifications
- 2–3 built-in themes + `system` theme

### v0.2: polish and reach

- Full mouse support: drag-resize sidebars, right-click context menus
- OpenCode theme compatibility + theme picker with live preview (pulled forward: done, D54)
- **Settings UI in the palette writing back to TOML (M5/M12, done, D48/D83):** including in-place `←`/`→` value cycling and staying inside `Ctrl+P` when picking a setting value or theme
- **OSC desktop notifications with privacy levels (M12, done, D83):** OSC 99 (`kitty`/`foot`/`Ghostty`/`WezTerm`) and OSC 777 (`rxvt`/fallback) with tmux DCS passthrough (`\ePtmux;...\e\\`) and `off`/`generic`/`sender`/`preview` privacy levels
- **Edit / delete for everyone / forward / send `@mentions` (M12, done, D83):** `e` edit, `D` delete with confirmation, `f` forward to another chat, inline `@` mention autocomplete
- **Delivery receipts & message info (M12, done, D83):** per-recipient `Delivered`/`Read` tracking (`0008_receipts.sql`), `✓✓` in `accent` when read, `i` message info popup (who reacted with what since D104)
- **Full-text search & jump to unread (M12, done, D83):** `/` in chat, `Ctrl+F` across all chats (matching message text and voice-note transcripts), `u` jump to unread marker
- **Density, timestamps & shared links/media (M12, done, D83):** `[messages] density` (`comfortable`/`compact`) and `timestamps` (`absolute`/`relative`/`off`) with live preview; recent links & downloaded files in the `Alt+I` info panel and palette (`Links & files in this chat…`)
- **Attachments and inline images (M9, done, D75–D79):** receive, open or save (SSH-aware), send; thumbnails in the message pane via kitty graphics / sixel / iTerm2 / half-block fallback (`ratatui-image`), with tmux passthrough
- **Voice notes (M10, done, D80–D82):** playback through an external player; opt-in transcription of incoming voice notes with a local Whisper model or a remote OpenAI-compatible endpoint (consent required for remote)
- **Emoji picker & shortcodes (M12, done, D83):** inline `:shortcode:` autocomplete in input and searchable reaction picker (`r`); emoji width fallback (M4.5, done, D47/D55). **Since D103:** every emoji up to Emoji 16.0 (1,906, no skin-tone variants) with CLDR's English keywords, generated into `ui/shortcodes/table.rs` by `tools/emoji/gen_table.py`; each emoji's code is its CLDR name in `_`-joined ASCII; the everyday aliases (`:thumbsup:`, `:+1:`) stay and rank first. **Since D104:** the reaction picker's quick row shows your recent reactions first (kept in `app_state` `ui.recent_reactions`), hints `/ search`, and its search lists up to 50 matches, 10 at a time, scrolling with a counter.
- Release binaries, Homebrew tap, AUR, `.deb`, Nix flake

### Later (backlog, do not forget)

- `faro daemon` + attachable TUI (§4.2)
- signal-cli JSON-RPC backend (D6)
- Encryption at rest (§12)
- Logo in an about screen (the animated lighthouse already fills the empty message pane, D61)
- Recording and sending voice notes

## 15. Testing strategy

- **Core logic:** unit tests against the mock backend (reducer: event in → state + effects out).
- **UI:** snapshot tests with ratatui's `TestBackend` + `insta`, including narrow widths and emoji-heavy content.
- **Store:** migration tests on a temp DB.
- **Backend:** thin integration tests behind a feature flag, run manually against a **test Signal account** (never the author's main account during development).
- **Terminal matrix (manual checklist per release):** kitty, WezTerm, Ghostty, iTerm2, Alacritty, foot; each with and without tmux; over SSH; macOS and Linux. Agents cannot verify this; it is the main human testing task. The checklist lives in [MANUAL_TESTS.md](MANUAL_TESTS.md); agents add an item whenever a change affects rendering, input, mouse, colours or emoji.

## 16. Candidate crates

| Purpose | Crate |
|---|---|
| TUI | `ratatui`, `crossterm` |
| Inline images | `ratatui-image`, `image` |
| Async | `tokio` |
| Signal | `presage`, presage SQLite store |
| Storage | `sqlx` (SQLite) or `rusqlite` |
| Fuzzy matching | `nucleo` |
| Config | `serde`, `toml_edit` |
| File watching | `notify` |
| Text width | `unicode-width`, `unicode-segmentation` |
| QR in terminal | `qrcode` |
| Logging | `tracing`, `tracing-appender` |
| Paths | `etcetera` or `directories` |
| Native notifications | `notify-rust` |
| Clipboard (local) | `arboard`, plus OSC 52 |
| Snapshot tests | `insta` |

## 17. Open questions

Resolved on 2026-10-07 (see DECISIONS.md): storage library (D16), own history DB (D7), encryption at rest (D18, deferred), history transfer (D19, assume none), workspace layout (D20).

Still open:

1. Does presage support importing message history from the phone when linking? Investigate in M7.
2. Final logo artwork and colours.
3. ~~Licences of bundled themes~~: OpenCode's themes are MIT; notice shipped (D54).
4. ~~Repo/package name `faro` vs `faro-chat`~~: `faro` everywhere, `faro-chat` on crates.io (D85).
5. Optional sync of hidden/allowlist state via the config file (backlog).
