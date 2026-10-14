# faro in your terminal

faro runs in any modern terminal, directly, in tmux, over SSH and through mosh. This page covers what depends on the terminal: tmux settings, inline images, mosh and emoji, colours, and bubble styles. Every setting here is also in the palette (`Ctrl+P` › Settings) and in the config file.

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

faro asks tmux which terminals are attached to its session, so it can choose image, emoji and bubble settings that work in all of them.

## Images

`Settings › Inline images` (or `[media] inline_images` in the config) offers:

- **`auto`** (default): pictures with the kitty graphics protocol, sixel or iTerm2 when the terminal supports one, otherwise coloured half-blocks. The setting's page and its `auto` row say what faro chose and why, e.g. `auto (kitty)` or `auto (halfblocks: a mosh client is attached)`.
- **`kitty`, `sixel`, `iterm2`**: force that protocol, overriding every fallback. Use one when you know your terminal supports it; it needs `allow-passthrough` inside tmux, can't work through mosh, and other attached terminals that don't support it show nothing.
- **`halfblocks`**: always coloured text cells.
- **`off`**: attachment details only, no pictures.

JPEG, PNG, GIF and WebP (including stickers) are drawn as still images; `v` opens a larger viewer. Changes apply at once and are saved.

**Why `auto` may draw half-blocks.** Pictures need every terminal that shows faro to support the same protocol, so `auto` falls back to half-blocks under mosh, when tmux has a client of a different kind attached (often a forgotten session from another machine), with no client attached, at 256 colours, and (no images at all) at 16 colours. The Inline images page names the client in the way and the command to drop it (`tmux detach-client -t /dev/ttys022`). Once it's gone faro asks the terminal again and switches to pictures; no restart needed.

**Reattaching from another screen.** When the set of attached tmux clients changes, faro asks the terminal again and redraws every picture for its cell size, so pictures fit their box on a different screen or scale too.

**Pasting an image:** press `Ctrl+V` in a chat. This works in kitty, directly or over SSH, and inside tmux (with `allow-passthrough on`), but not through mosh. kitty asks for permission; choose "allow all" to stop it asking until faro restarts. The image waits above the input with its size: `Ctrl+O` shows it, `Enter` sends it (with what you typed as a caption), `Esc` removes it. `Cmd+V` stays kitty's text paste.

## mosh and emoji

mosh measures some emoji differently from kitty and tmux, which misaligns borders. faro detects when it, or any client of its tmux session, runs under mosh and then draws those emoji in a simpler form every terminal measures alike (❤️ becomes ❤, a family becomes its people), switching back when mosh detaches. Force either way with `--emoji full|compatible` or `[ui] emoji` in the config. Other programs in the same tmux can still look misaligned through mosh.

## Colours

faro guesses how many colours your terminal shows: truecolor when `COLORTERM` says so, inside tmux, or in terminals known to support it (kitty, Ghostty, WezTerm, iTerm2, Alacritty, foot…); otherwise 256 colours, or 16 on the Linux console and similar. Override it with `[ui] colors = "truecolor" | "256" | "16"` or `Settings › Colours`, which also shows what was detected.

- **Truecolor:** every theme exactly as designed.
- **256 colours:** themes are approximated with the nearest of the terminal's 240 fixed colours, so subtle tints can turn grey (Solarized's dark blue background becomes a neutral grey). faro keeps the background, panels, selection and bubbles apart so the layout still reads.
- **16 colours:** the `system` theme (your terminal's own 16 colours) is drawn whatever theme is set. Your choice is kept and drawn once faro runs with more colours.

The palette's Themes list says which of these applies.

## Themes

`Ctrl+P` › Themes lists every theme; moving through the list previews each one, `Enter` keeps it (and writes `theme = "…"` to the config), `Esc` puts the old one back. faro ships its own themes (`faro-dark`, `faro-dusk`, `faro-light`, `system`) and [OpenCode](https://github.com/anomalyco/opencode)'s (`tokyonight`, `catppuccin`, `gruvbox`, `nord`, `dracula`…, with a `-light` variant where there is one).

Your own themes go in `~/.config/faro/themes/` as JSON files in [OpenCode's theme format](https://opencode.ai/docs/themes/); the file name is the theme's name. faro needs `background`, `text` and `primary`; it also uses `backgroundPanel`, `backgroundElement`, `textMuted`, `secondary`, `accent`, `border`, `borderActive`, `success`, `warning`, `error` and `info`, and works out the rest (message bubbles, sender colours). Save a file and faro picks it up at once; a broken one is named in the footer. A file named after a built-in theme replaces it, so copying [`nord.json`](../crates/faro-app/themes/opencode/nord.json) is a quick way to tweak it.

## Bubbles

`[messages] bubble_style` (or `Settings › Bubble style`):

- **`auto`** (default): rounded bubbles where every terminal showing faro is kitty (inside tmux too), otherwise `bar`. Through mosh faro can't see the terminal, so it draws bars.
- **`rounded`**: rounded ends. These need Powerline half circles (U+E0B6, U+E0B4) and Unicode 16 block glyphs (quarter circles U+1FBEC–U+1FBEF, octants U+1CD00–U+1CDE5). A recent kitty draws them itself; other terminals take them from the font, and most fonts without Nerd Font patches lack them (you'd see boxes or question marks).
- **`bar`**: a coloured bar on each bubble's outer side; works everywhere.
- **`mixed`**: the bar outside, rounded inside (needs the same glyphs as `rounded`).

`bubble_padding` (on by default) keeps a blank cell between the text and each rounded side. With `density = "compact"`, rounded bubbles from one sender join up, rounded only at the top and bottom of the run; `match_widths` (on by default) gives neighbours in a run that are within 3 cells of each other the same width.

`[messages] layout = "irc"` switches to one line per message instead of bubbles; `timestamps` can be `absolute`, `relative` or `off`.
