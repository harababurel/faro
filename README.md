# faro

A terminal client for [Signal](https://signal.org), built for people who live in tmux.

> **Status: pre-release.** The interface is usable against a built-in **mock backend** (fake contacts and messages). The real Signal backend (via [presage](https://github.com/whisperfish/presage)) isn't written yet, so faro can't send or receive real Signal messages today. See the [plan](docs/PLAN.md) for progress.

faro is an unofficial, independent project. It is not affiliated with, endorsed by, or supported by Signal Messenger LLC or the Signal Technology Foundation.

<!-- TODO(owner): replace the text capture below with real screenshots (kitty + tmux, faro-dark and faro-light). -->

```text
╭ Conversations ─────────────╮╭ Climbing crew 🧗 ──────────────────────────────────────────────────╮
│ ● Émile & Zoë Ångström   5 ││                                                                    │
│   Climbing crew 🧗         ││ ─────────────────────────── Yesterday ──────────────────────────── │
│ ● Alice Moreau           4 ││                                                  brb ☕  03:08 ✓✓  │
│ ● Lukas 🇩🇪               4 ││ ────────────────────────── new messages ────────────────────────── │
│ ● Bogdan 🚴‍♂️              5 ││  Lukas 🇩🇪                                                          │
│ ● 陈伟 Chen Wei          6 ││  hey!  22:36                                                       │
│ ● Mum ❤️                 2 ││ ───────────────────────────── Today ────────────────────────────── │
│                            ││  +15555550108                                                      │
│                            ││  line one                                                          │
│                            ││  line two  22:33                                                   │
│                            ││╭──────────────────────────────────────────────────────────────────╮│
│                            │││ Message Climbing crew 🧗…                                        ││
│                            ││╰──────────────────────────────────────────────────────────────────╯│
╰────────────────────────────╯╰────────────────────────────────────────────────────────────────────╯
 ● connected · 39 unread               enter send · alt+j/k conversations · tab focus · pgup scroll
```

<sub>A text capture of `faro --mock` in tmux (cut down). Emoji may look misaligned in your browser; in a terminal they line up.</sub>

## What it does (today, with the mock)

- Conversation list with pinning, muting (with durations), hiding, archiving and an allowlist mode
- Two message layouts: **bubbles** (default, with rounded or square corners) and **IRC**-style
- Replies, reactions, copying a message (OSC 52, works over SSH), opening links
- A command palette (`Ctrl+P`): chats, themes, settings, commands and key bindings as categories you open and fuzzy-search
- Read receipts only for what you've actually seen (terminal focus + scrolled to the bottom), typing indicators
- Terminal bell (with a do-not-disturb key, `Alt+N`) and an unread count in the window/tmux title
- A commented config file that faro reloads while running and writes back to without losing your comments
- Themes: `faro-dark`, `faro-dusk`, `faro-light`, `system` (your terminal's own 16 colours), OpenCode's 33 themes (dark and light), and your own theme files; previewed live as you move through them; automatic 256-colour fallback
- Never blocks: the UI keeps up while messages flood in

## Install from source

You need Rust via [rustup](https://rustup.rs). The exact toolchain is pinned in `rust-toolchain.toml`; rustup installs it on the first build.

```sh
git clone https://github.com/harababurel/faro
cd faro
cargo build --release          # binary: target/release/faro
```

Once the Signal backend lands, building will also need the protobuf compiler and a C toolchain for libsignal:

| OS | Packages |
|---|---|
| Arch | `sudo pacman -S protobuf cmake clang pkgconf` |
| Debian/Ubuntu | `sudo apt install protobuf-compiler cmake clang pkg-config` |
| macOS | `brew install protobuf cmake` |

Release binaries and packages come later (see [DESIGN.md §13](docs/DESIGN.md)).

## Try it

```sh
cargo run --release -- --mock --data-dir /tmp/faro-try --config /tmp/faro-try/config.toml
```

`--mock` uses fake contacts and scripted activity; no Signal account is involved. (It's currently on whether you pass it or not.) The throwaway `--data-dir` and `--config` keep your real ones untouched.

| Flag | Environment | What |
|---|---|---|
| `--data-dir DIR` | `FARO_DATA_DIR` | Database, logs and (later) Signal keys. Default `~/.local/share/faro` |
| `--config FILE` | `FARO_CONFIG` | Config file. Default `~/.config/faro/config.toml` |
| `--mock` | | Use the mock backend |
| `--mock-seed N` | `FARO_MOCK_SEED` | Another seed, another scripted conversation |
| `--mock-activity MS` | `FARO_MOCK_ACTIVITY` | Mean time between mock events (default 4000; small values flood the UI) |
| `--mock-no-faults` | `FARO_MOCK_NO_FAULTS` | No scripted outages or send failures |
| `--emoji auto\|full\|compatible` | `FARO_EMOJI` | How to draw emoji (see [mosh](#mosh)); overrides the config |

## Keys

Press `F1` (or `?` outside the input box) for the full list, or search it in the palette's `Keys`. Every key can be rebound in the config file.

| Key | What |
|---|---|
| `Ctrl+P` | Command palette: `Enter`/`→` opens a category, `←`/`Backspace` goes back; `@` chats, `:` settings, `>` commands, `?` keys jump straight in |
| `Alt+J` / `Alt+K` (or `Alt+↓` / `Alt+↑`) | Next / previous conversation |
| `Alt+1`…`Alt+9` | Pinned conversation 1–9 |
| `Tab` / `Shift+Tab` | Move between the conversation list, the messages and the input |
| `Enter` | Send (`Alt+Enter`, `Shift+Enter` or `Ctrl+J`: new line) |
| `↑` on an empty input | Select a message: `r` reply, `+` react, `y` copy, `o` open link, `Enter` menu |
| `PgUp` / `PgDn` | Scroll the messages |
| `Alt+S` / `Alt+I` | Show or hide the conversation list / conversation info |
| `Alt+N` | Do not disturb: turn the bell off (the footer says `bell off`) or back on |
| In the list | `j`/`k` move, `Space` menu, `p` pin, `m` mute, `@` mentions only, `h` hide, `a` archive, `l` allow, `L` allowlist mode, `A` archive view |
| `Ctrl+L` | Redraw the screen |
| `Esc` | Back, dismiss |
| `Ctrl+Q` (or `Ctrl+C` twice) | Quit |

The input box has readline-style editing (`Ctrl+A`/`E`/`U`/`K`/`W`, `Alt+B`/`F`/`D`).

## Configuration

On first run faro writes `~/.config/faro/config.toml` (or `$XDG_CONFIG_HOME/faro/config.toml`) with **every setting commented out**, showing its default and what it does, followed by every default key binding. Uncomment and edit; faro applies changes while it runs and reports mistakes in the footer (keeping the previous value). Settings changed from the palette (`:`) are written back, keeping your comments.

| What | Where |
|---|---|
| Config | `~/.config/faro/config.toml` (`--config`) |
| Your themes | `themes/*.json` beside the config file |
| Data (database, later Signal keys) | `~/.local/share/faro/` (`--data-dir`) |
| Log | `<data dir>/logs/faro.log`; level via `FARO_LOG`, e.g. `FARO_LOG=debug` |

The same paths are used on macOS. Files are created readable only by you. faro never logs message bodies or phone numbers.

## Themes

`Ctrl+P` › Themes lists every theme; moving through the list previews each one, `Enter` keeps it (and writes `theme = "…"` to the config), `Esc` puts the old one back. faro ships its own themes and [OpenCode](https://github.com/anomalyco/opencode)'s (`tokyonight`, `catppuccin`, `gruvbox`, `nord`, `dracula`…, each with a `-light` variant where it has one).

Your own themes go in `~/.config/faro/themes/` as JSON files in [OpenCode's theme format](https://opencode.ai/docs/themes/); the file name is the theme's name. faro needs `background`, `text` and `primary`; it also uses `backgroundPanel`, `backgroundElement`, `textMuted`, `secondary`, `accent`, `border`, `borderActive`, `success`, `warning`, `error` and `info`, and works out the rest (message bubbles, sender colours). Save a file and faro picks it up at once; a broken one is named in the footer. A file named after a built-in theme replaces it, so copying [`nord.json`](crates/faro/themes/opencode/nord.json) is a quick way to tweak it.

## tmux

faro works in tmux as it is, but these settings make it better (in `~/.tmux.conf`):

```tmux
set -g mouse on                        # clicks and the wheel reach faro
set -g set-clipboard on                # copying a message (OSC 52) reaches your clipboard, even over SSH
set -g allow-passthrough on            # for future inline images
set -as terminal-features ",*:RGB"     # truecolor themes (otherwise tmux rounds colours to 256)
set -g extended-keys on                # more distinguishable key combinations
set -g focus-events on                 # read receipts and the bell know whether you're looking
```

## mosh

mosh measures emoji differently from kitty and tmux, which misaligns borders. faro detects when it (or any client of its tmux session) runs under mosh and then draws emoji in a simpler form that every terminal measures alike (❤️ becomes ❤, a family becomes its people), switching back when mosh detaches. Force either way with `--emoji full|compatible` or `[ui] emoji` in the config. Other programs in the same tmux can still look misaligned through mosh.

## Colours

faro guesses how many colours your terminal shows: truecolor when `COLORTERM` says so, inside tmux, or in terminals known to support it (kitty, Ghostty, WezTerm, iTerm2, Alacritty, foot…); otherwise 256 colours (themes are rounded to the nearest), or the `system` theme on 16-colour terminals. Override with `[ui] colors = "truecolor" | "256" | "16"` or in the palette (`:` Appearance › Colours).

## Development

```sh
cargo test                                   # tests
cargo fmt --check                            # formatting
cargo clippy --all-targets -- -D warnings    # lints (pedantic; no panics, no blocking calls)
FARO_LOG=info,faro=debug cargo run -- --mock-seed 7   # log every mock event
```

The last three must pass before every commit; CI runs them on Linux and macOS.

- [Design](docs/DESIGN.md): architecture, UI, configuration, themes
- [Implementation plan](docs/PLAN.md): milestones and progress
- [Decision log](docs/DECISIONS.md): what was decided and why
- [Manual tests](docs/MANUAL_TESTS.md): checks that need a real terminal
- [Background](docs/BACKGROUND.md): motivation, research, naming
- [AGENTS.md](AGENTS.md): guide for coding agents working on this repo

## License

[AGPL-3.0](LICENSE). The bundled OpenCode themes are MIT-licensed; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
