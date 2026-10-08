# faro

A terminal client for [Signal](https://signal.org), built for people who live in tmux.

> **Status: pre-release.** The interface is usable against a built-in **mock backend** (fake contacts and messages). The real Signal backend (via [presage](https://github.com/whisperfish/presage)) is being tested: for now it only runs with an allowlist of who faro may talk to. See the [plan](docs/PLAN.md) for progress.

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
- Two message layouts: **bubbles** (default; rounded in kitty, with a coloured bar elsewhere: `[messages] bubble_style`) and **IRC**-style
- Replies, reactions, copying a message (OSC 52, works over SSH), opening links
- A command palette (`Ctrl+P`): chats, themes, settings, commands and key bindings as categories you open and fuzzy-search; every setting in one list with its current value, on/off ones flipped with `Enter`
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

Building also needs the protobuf compiler and a C toolchain for libsignal and SQLCipher:

| OS | Packages |
|---|---|
| Arch | `sudo pacman -S protobuf cmake clang pkgconf openssl` |
| Debian/Ubuntu | `sudo apt install protobuf-compiler cmake clang pkg-config libssl-dev` |
| macOS | `brew install protobuf cmake` |

Release binaries and packages come later (see [DESIGN.md §13](docs/DESIGN.md)).

## Try it

```sh
cargo run --release -- --mock --data-dir /tmp/faro-try --config /tmp/faro-try/config.toml
```

`--mock` uses fake contacts and scripted activity; no Signal account is involved. The throwaway `--data-dir` and `--config` keep your real ones untouched.

With a real account (while testing):

```sh
faro link                                  # shows a QR code: scan it in Signal › Settings › Linked devices
faro ids --username alice.42               # prints the IDs for an allowlist file
faro --allowlist ~/faro-allowlist.toml     # faro only talks to the people and groups listed
```

| Flag | Environment | What |
|---|---|---|
| `--data-dir DIR` | `FARO_DATA_DIR` | Database, logs and Signal keys. Default `~/.local/share/faro` |
| `--config FILE` | `FARO_CONFIG` | Config file. Default `~/.config/faro/config.toml` |
| `--mock` | | Use the mock backend (without it: the account linked with `faro link`) |
| `--mock-seed N` | `FARO_MOCK_SEED` | Another seed, another scripted conversation |
| `--mock-activity MS` | `FARO_MOCK_ACTIVITY` | Mean time between mock events (default 4000; small values flood the UI) |
| `--mock-no-faults` | `FARO_MOCK_NO_FAULTS` | No scripted outages or send failures |
| `--emoji auto\|full\|compatible` | `FARO_EMOJI` | How to draw emoji (see [mosh](#mosh)); overrides the config |
| `--allowlist FILE` | `FARO_ALLOWLIST` | Talk only to the people and groups in FILE; drop everything else (required with a real account while testing) |
| `faro link [--device-name NAME]` | | Link this computer to your Signal account (QR code) |
| `faro ids [--username NAME]…` | | Print this account's, usernames' and groups' IDs for an allowlist |

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
| `Esc` | Back, dismiss; then close the open chat (nothing is read while no chat is open) |
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

faro guesses how many colours your terminal shows: truecolor when `COLORTERM` says so, inside tmux, or in terminals known to support it (kitty, Ghostty, WezTerm, iTerm2, Alacritty, foot…); otherwise 256 colours, or 16 on the Linux console and similar. Override with `[ui] colors = "truecolor" | "256" | "16"` or in the palette (`Settings › Colours`, which also shows what was detected).

- **Truecolor:** every theme exactly as designed.
- **256 colours:** themes are **approximated**: each colour becomes the nearest of the terminal's 240 fixed colours, so subtle tints can turn grey. Solarized's dark blue background (`#002b36`), for example, becomes the neutral grey `#262626`, much as Solarized's own 256-colour fallback uses `#1c1c1c`: there's no dark blue-teal among those colours. faro keeps the background, panels, selection and bubbles apart so the layout still reads.
- **16 colours:** the **`system` theme** is drawn, whatever theme is set, since faro can't know which 16 colours your terminal uses. You can still pick another theme; it's saved and drawn once faro runs with more colours.

The palette's Themes list says which of these applies.

## Bubbles

Rounded bubbles are drawn with Powerline half circles (U+E0B6, U+E0B4) and Legacy Computing wedges and sextants (U+1FB0E to U+1FB65). kitty draws these itself, whatever your font; other terminals take them from the font, and most fonts without Nerd Font patches lack them (you'd see boxes or question marks). So by default (`bubble_style = "auto"`) faro draws rounded bubbles only when every terminal it's shown in is kitty, inside tmux too (it asks tmux for each attached client's terminal), and otherwise a coloured bar on each bubble's outer side. Through mosh the terminal can't be seen, so it draws bars there; set `[messages] bubble_style = "rounded"` (or `Settings › Bubble style` in the palette) if your terminal or font has the glyphs, or `"bar"` to always have bars. `"mixed"` puts the bar on the outer side and rounds the inner one (it needs the same glyphs as `"rounded"`).

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
