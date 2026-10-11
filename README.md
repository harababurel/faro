# faro

A terminal client for [Signal](https://signal.org), built for people who live in tmux.

> **Status: pre-release.** The interface can be tried with **`faro demo`** (fake contacts and messages, no account needed). The real Signal backend (via [presage](https://github.com/whisperfish/presage)) works and is in daily use; for testing it can be limited to an allowlist of who faro may talk to. See the [plan](docs/PLAN.md) for progress.

faro is an unofficial, independent project. It is not affiliated with, endorsed by, or supported by Signal Messenger LLC or the Signal Technology Foundation.

<!-- TODO(owner): replace the text capture below with real screenshots (kitty + tmux, faro-dark and faro-light). -->

```text
╭ Conversations ─────────────╮╭ Scranton Branch 📄 ────────────────────────────────────────────────╮
│ ● Kelly's mom            6 ││  Stanley Hudson                                                    │
│   Scranton Branch 📄       ││ ┃Is it mandatory?  02:33                                           │
│ ● Dwight K. Schrute III… 4 ││                                                                    │
│ ● Ryan Howard            4 ││  Michael Scott                                                     │
│ ● Kelly Kapoor 💁🏽‍♀️        5 ││ ┃▎ Stanley Hudson: Is it mandatory?                                │
│ ● Mariam · Cairo office… 7 ││ ┃Did I stutter?               02:34                                │
│ ● Jim Halpert            4 ││                                                                    │
│ ● Finer Things Club 🍷   5 ││  Meredith Palmer                                                   │
│                            ││ ┃Is there cake?  02:38                                             │
│                            ││                                                                    │
│                            ││╭──────────────────────────────────────────────────────────────────╮│
│                            │││ Message Scranton Branch 📄…                                      ││
│                            ││╰──────────────────────────────────────────────────────────────────╯│
╰────────────────────────────╯╰────────────────────────────────────────────────────────────────────╯
 ● connected · demo · 35 unread          enter send · alt+j/k conversations · esc close · tab focus
```

<sub>A text capture of `faro demo` in tmux (cut down). Emoji may look misaligned in your browser; in a terminal they line up.</sub>

## What it does (today, in the demo)

- Conversation list with pinning, muting (with durations), hiding, archiving and an allowlist mode
- Two message layouts: **bubbles** (default; rounded in kitty, with a coloured bar elsewhere: `[messages] bubble_style`) and **IRC**-style, with configurable density (`comfortable` / `compact`) and timestamps (`absolute` / `relative` / `off`)
- Replies, reactions (with `:shortcode:` search over every standard emoji and its official keywords: `:cool` finds 😎), editing (`e`) and deleting (`D`) your own messages, forwarding (`f`), copying a message (OSC 52, works over SSH), opening links, per-recipient delivery/read receipts (`i`), and sending `@mentions` with `@` autocomplete and `:shortcode:` emoji completion
- Full-text message and voice-note transcript search (`Ctrl+F` or `/`), in the open chat or across all conversations, plus jump to unread (`u`) and a per-chat links & files browser (`Alt+I` info panel and palette)
- Attachments and inline images: images and stickers render in the message pane (kitty graphics, sixel, iTerm2 or half-block fallback) and in a full-size viewer (`v`); receive, download (`d`), save to `Downloads` (`s`) and send files from the palette or by pasting a path
- Voice notes and opt-in Whisper transcription: voice notes show their duration and a 16-bar waveform (`▶ 0:14 ▁▃▆█▅▂…`), play locally (`x`) with live progress (`■ 0:05 / 0:14`), and can be transcribed (`t` or automatically) with a local Whisper CLI or an OpenAI-compatible remote endpoint (with explicit consent and per-conversation permission)
- A command palette (`Ctrl+P`): chats, themes, settings, commands, key bindings and search as categories you open and fuzzy-search; every setting in one list with its current value (`*` marks the ones you changed), cycled in place with `←`/`→`, flipped with `Enter`, or put back to its default with `Ctrl+R`
- Read receipts only for what you've actually seen (terminal focus + scrolled to the bottom), typing indicators
- Terminal bell (with a do-not-disturb key, `Alt+N`), unread count in the window/tmux title, and OSC 99 / 777 desktop notifications (with tmux passthrough and configurable privacy)
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
cargo run --release -- demo
```

`faro demo` runs faro against a fake account: the new temp at a paper company's Scranton branch, with scripted coworkers, groups, photos and voice notes. No Signal account is involved. Its data lives in its own directory under the system temp dir (`$TMPDIR` or `/tmp`, as `faro-demo-<uid>`), kept until the system clears it; `--fresh` starts over. The demo never reads or writes your real data directory (it ignores `--data-dir` and `FARO_DATA_DIR`), but it does use your real config file, so a theme you pick in the demo stays picked.

With a real account:

```sh
faro link                                  # shows a QR code: scan it in Signal › Settings › Linked devices
faro                                       # talks to everyone, like any Signal client
faro ids --username alice.42               # for testing: prints the IDs for an allowlist file
faro --allowlist ~/faro-allowlist.toml     # for testing: faro only talks to the people and groups listed
```

| Flag | Environment | What |
|---|---|---|
| `--data-dir DIR` | `FARO_DATA_DIR` | Database, logs and Signal keys. Default `~/.local/share/faro`. Never used by `faro demo` |
| `--config FILE` | `FARO_CONFIG` | Config file. Default `~/.config/faro/config.toml` |
| `--emoji auto\|full\|compatible` | `FARO_EMOJI` | How to draw emoji (see [mosh](#mosh)); overrides the config |
| `--allowlist FILE` | `FARO_ALLOWLIST` | For testing: talk only to the people and groups in FILE; drop everything else |
| `--no-allowlist` | | Does nothing now (talking to everyone is the default); still accepted |
| `faro demo` | | Run with a fake account and scripted activity, in its own data directory (see [Try it](#try-it)) |
| `faro demo --seed N` | | Another seed, another order of scripted scenes (default 0) |
| `faro demo --activity MS` | | Mean time between demo events (default 4000; small values flood the UI) |
| `faro demo --no-faults` | | No scripted outages or send failures |
| `faro demo --fresh` | | Wipe the demo's data and start over |
| `faro demo --dir DIR` | | Keep the demo's data in DIR (must be empty, new, or an earlier demo directory) |
| `faro link [--device-name NAME]` | | Link this computer to your Signal account (QR code) |
| `faro ids [--username NAME]…` | | Print this account's, usernames' and groups' IDs for an allowlist |

The demo's flags have no environment variables on purpose: a leftover variable can't change what a demo run touches. `--emoji` and `--allowlist` also work after `demo`.

## Keys

Press `F1` (or `?` outside the input box) for the full list, or search it in the palette's `Keys`. Every key can be rebound in the config file.

| Key | What |
|---|---|
| `Ctrl+P` | Command palette: `Enter`/`→` opens a category, `←`/`Backspace` goes back (`←`/`→` cycles settings, `Ctrl+R` puts one back to its default); `@` chats, `/` search, `:` settings, `>` commands, `?` keys jump straight in |
| `Ctrl+F` (or `/` in messages) | Search messages and voice-note transcripts (in the open chat or across all chats) |
| `Alt+J` / `Alt+K` (or `Alt+↓` / `Alt+↑`) | Next / previous conversation |
| `Alt+1`…`Alt+9` | Pinned conversation 1–9 |
| `Tab` / `Shift+Tab` | Move between the conversation list, the messages and the input |
| `Enter` | Send (`Alt+Enter`, `Shift+Enter` or `Ctrl+J`: new line; `@name` and `:shortcode:` complete inline) |
| `↑` on an empty input | Select a message: `r` reply, `f` forward, `+` react (`/` searches all emoji), `y` copy, `i` info/receipts, `e` edit, `D` delete, `u` jump to unread, `o` open link, `d` download, `v` view/open, `s` save, `x` play/stop voice note, `t` transcribe, `Enter` menu |
| `PgUp` / `PgDn` | Scroll the messages |
| `Alt+S` / `Alt+I` | Show or hide the conversation list / conversation info (with shared links & media) |
| `Alt+N` | Do not disturb: turn the bell off (the footer says `bell off`) or back on |
| `Ctrl+V` | Paste an image from the clipboard (kitty; see [Images](#images)); it waits above the input until you press `Enter`, `Ctrl+O` shows it |
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
set -g allow-passthrough on            # for inline images (kitty, sixel, iTerm2)
set -as terminal-features ",*:RGB"     # truecolor themes (otherwise tmux rounds colours to 256)
set -g extended-keys on                # more distinguishable key combinations
set -g focus-events on                 # read receipts and the bell know whether you're looking
```

## mosh

mosh measures emoji differently from kitty and tmux, which misaligns borders. faro detects when it (or any client of its tmux session) runs under mosh and then draws emoji in a simpler form that every terminal measures alike (❤️ becomes ❤, a family becomes its people), switching back when mosh detaches. Force either way with `--emoji full|compatible` or `[ui] emoji` in the config. Other programs in the same tmux can still look misaligned through mosh.

## Images

`Ctrl+P › Settings › Inline images` (or `[media] inline_images` in the config) offers:

- **`auto`** (default): detect Kitty graphics, Sixel or iTerm2 at startup; otherwise use coloured Unicode half-blocks. Mixed terminal types attached to tmux, detached tmux, mosh and 256-colour mode use half-blocks; 16-colour mode disables images. The settings page explains the current choice.
- **`kitty`, `sixel`, `iterm2`**: force that graphics protocol, overriding automatic fallbacks, including colour depth. Use these when you know your viewing terminal supports it. They require tmux passthrough inside tmux, cannot work through mosh, and may display no image in incompatible terminals or other attached clients.
- **`halfblocks`**: always use coloured text cells (disabled at 16 colours).
- **`off`**: show attachment details without inline pictures.

Changes apply immediately and are saved. Forced modes work without a capability query, using estimated cell sizing if no query ran at startup. Returning to Auto reuses startup detection; if startup skipped the query, restart to detect graphical support. JPEG, PNG, GIF and WebP render as static images, including stickers; `v` opens a larger viewer.

**Pasting an image** from your clipboard: press `Ctrl+V` in a chat. This works in kitty, directly or over SSH, and inside tmux (with `set -g allow-passthrough on`), but not through mosh. kitty asks for permission; choose "allow all" to stop it asking until faro restarts. The image is staged above the input with its size; `Ctrl+O` shows it, `Enter` sends it (with whatever you typed as a caption), `Esc` removes it. Nothing is sent before you press `Enter`. `Cmd+V` stays kitty's text paste.

## Colours

faro guesses how many colours your terminal shows: truecolor when `COLORTERM` says so, inside tmux, or in terminals known to support it (kitty, Ghostty, WezTerm, iTerm2, Alacritty, foot…); otherwise 256 colours, or 16 on the Linux console and similar. Override with `[ui] colors = "truecolor" | "256" | "16"` or in the palette (`Settings › Colours`, which also shows what was detected).

- **Truecolor:** every theme exactly as designed.
- **256 colours:** themes are **approximated**: each colour becomes the nearest of the terminal's 240 fixed colours, so subtle tints can turn grey. Solarized's dark blue background (`#002b36`), for example, becomes the neutral grey `#262626`, much as Solarized's own 256-colour fallback uses `#1c1c1c`: there's no dark blue-teal among those colours. faro keeps the background, panels, selection and bubbles apart so the layout still reads.
- **16 colours:** the **`system` theme** is drawn, whatever theme is set, since faro can't know which 16 colours your terminal uses. You can still pick another theme; it's saved and drawn once faro runs with more colours.

The palette's Themes list says which of these applies.

## Bubbles

Rounded bubbles are drawn with Powerline half circles (U+E0B6, U+E0B4) and Unicode 16 block glyphs: quarter circles (U+1FBEC to U+1FBEF), octants (U+1CD00 to U+1CDE5) and quarter blocks. A recent kitty draws these itself, whatever your font (an older one shows boxes at the corners of taller bubbles: update kitty, or use `"bar"`); other terminals take them from the font, and most fonts without Nerd Font patches lack them (you'd see boxes or question marks). So by default (`bubble_style = "auto"`) faro draws rounded bubbles only when every terminal it's shown in is kitty, inside tmux too (it asks tmux for each attached client's terminal), and otherwise a coloured bar on each bubble's outer side. Through mosh the terminal can't be seen, so it draws bars there; set `[messages] bubble_style = "rounded"` (or `Settings › Bubble style` in the palette) if your terminal or font has the glyphs, or `"bar"` to always have bars. `"mixed"` puts the bar on the outer side and rounds the inner one (it needs the same glyphs as `"rounded"`).

`bubble_padding` (on by default) keeps a blank cell between the text and each rounded side. With `density = "compact"`, rounded bubbles from one sender join up: they touch and share one edge on their aligned side, rounded only at the top and bottom of the run. `match_widths` (on by default) gives neighbours in such a run that are within 3 cells of each other the same width, so they line up.

## Development

```sh
cargo test                                   # tests
cargo fmt --check                            # formatting
cargo clippy --all-targets -- -D warnings    # lints (pedantic; no panics, no blocking calls)
FARO_LOG=info,faro=debug cargo run -- demo --seed 7   # log every demo event
```

The last three must pass before every commit; CI runs them on Linux and macOS.

- [Design](docs/DESIGN.md): architecture, UI, configuration, themes
- [Implementation plan](docs/PLAN.md): milestones and progress
- [Decision log](docs/DECISIONS.md): what was decided and why
- [Manual tests](docs/MANUAL_TESTS.md): checks that need a real terminal
- [Background](docs/BACKGROUND.md): motivation, research, naming
- [Website](docs/WEBSITE.md): design and plan for getfaro.dev
- [AGENTS.md](AGENTS.md): guide for coding agents working on this repo

## License

[AGPL-3.0](LICENSE). The bundled OpenCode themes are MIT-licensed; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

`faro demo`'s cast is an affectionate parody of characters from NBC's *The Office*. faro is not affiliated with NBCUniversal or *The Office*.
