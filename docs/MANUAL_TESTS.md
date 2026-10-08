# Manual test checklist

Checks that agents can't do: they need a real terminal, real eyes and real hands (AGENTS.md rule 10). Agents add an item here whenever a change affects rendering, input, mouse, colours or emoji. The owner ticks items off and notes the terminal, OS and whether tmux/SSH were involved.

Run against a throwaway data dir so the real one stays clean:

```sh
cargo run --release -- --mock --data-dir /tmp/faro-manual
```

Terminal matrix (DESIGN §15): kitty, WezTerm, Ghostty, iTerm2, Alacritty, foot; each with and without tmux; locally and over SSH; macOS and Linux. Not every item needs every combination: **kitty + tmux over SSH** and **macOS Terminal/iTerm2** are the priorities.

## M3: TUI skeleton

- [ ] **Startup:** the conversation list and the last open conversation (or, on a fresh data dir, the first conversation that gets a message) appear within a second; names, not phone numbers or IDs.
- [ ] **Live updates:** new mock messages appear in the open conversation and bump unread counts in the sidebar and footer; "… is typing" shows and clears.
- [ ] **Flood:** `cargo run --release -- --data-dir /tmp/faro-flood --mock-activity 2 --mock-no-faults`. Typing in the input box stays instant (no visible lag or dropped keys) while hundreds of messages a second arrive. `Ctrl+Q` exits within a second.
- [ ] **Keys:** `Alt+J`/`Alt+K` switch conversations; `Alt+↓`/`Alt+↑` do too unless tmux takes them (it does in the owner's setup, D36). On macOS this needs Option sent as Alt/Meta (kitty `macos_option_as_alt yes`, iTerm2 Left Option → `Esc+`, Terminal.app "Use Option as Meta key"); otherwise Option+J types `∆`. `Alt+S`/`Alt+I` toggle the sidebars. `Esc` clears a notice in the footer.
- [ ] **Quit:** `Ctrl+Q` quits; `Ctrl+C` shows a hint and quits on the second press. In both cases the terminal is fully restored: normal screen, cursor visible, echo on, no mouse escape garbage when moving the mouse afterwards.
- [ ] **Signals:** `kill <pid>` (SIGTERM) and closing the SSH session or tmux pane (SIGHUP) leave no broken terminal and the log ends with "faro exiting".
- [ ] **Resize:** shrinking the window below ~110 columns hides the info sidebar, below ~70 the conversation list; nothing overlaps or panics at very small sizes (e.g. 20×5).
- [ ] **Colours:** truecolour renders the amber theme correctly, with a painted dark background (no terminal-background "holes"). Under tmux, check `set -as terminal-features ",*:RGB"` is needed and works.
- [ ] **Flicker:** no tearing or flicker during the flood (synchronized output, DEC 2026). Note terminals that flicker.
- [ ] **Mouse:** mouse capture is on (clicks do nothing yet, which is expected in M3); `Shift`+drag still selects text natively.
- [x] **Emoji widths (known issue, seen by the owner on first try; measured, fixed in M4.5, D47):** in "Climbing 🧗" and other emoji-heavy chats, note which emoji shift the right border. Expected offenders under tmux: emoji with a variation selector (☺️ ✌️ ❤️), flags (🇵🇹), ZWJ sequences (🧗‍♀️, family emoji), skin tones. Record terminal + tmux version; this feeds the emoji width fallback (DESIGN §7.3, PLAN M4.5). Run `python3 tools/emoji_widths.py` inside the tmux where faro runs and directly in the terminal (outside tmux), and keep the output.
- [ ] **Text safety:** no message can change terminal state (title, colours, cursor); control characters show as `�`.

## M4: interactions

- [ ] **Focus:** `Tab`/`Shift+Tab` move the highlighted border between conversations, messages and input; the footer hints change with it; the cursor shows only when the input has focus. `Esc` and `Enter` return to the input.
- [ ] **Sidebar keys:** with the sidebar focused, `j`/`k` and `↑`/`↓` switch conversations, `g`/`G` jump to first/last.
- [ ] **Scrolling:** `PgUp`/`PgDn` from the input; `j`/`k`, `g`/`G` with the messages focused. While scrolled up, new mock messages don't move the view and the "↓ N new messages" badge counts them; `G`/`End`/`PgDn` back to the bottom clears it. Scrolling to the top of a long conversation keeps loading older messages without jumping.
- [ ] **Mouse:** clicking a conversation opens it (and you can type straight away); the wheel scrolls messages over the message area and the list over the sidebar; under tmux this needs `set -g mouse on`. Check that `Shift`+drag (or your terminal's bypass modifier) still selects text.
- [ ] **Input editing:** `Alt+Enter` and `Ctrl+J` add a line; `←/→`, `↑/↓` between lines, `Home/End`, `Ctrl+A/E`; word moves with `Alt+←/→` (Option+arrows on macOS), `Ctrl+←/→`, `Alt+B/F`; `Ctrl+W` / `Alt+Backspace` delete a word; `Ctrl+U`/`Ctrl+K`. The cursor lands in the right cell after emoji and CJK text. In kitty without tmux, check whether `Shift+Enter` adds a line (it won't yet: needs the kitty keyboard protocol).
- [ ] **Paste:** pasting several lines puts them all in the input box, line breaks kept, nothing sent. Works under tmux too.
- [ ] **Drafts:** type something, switch conversation and back: still there. Type, quit, restart: still there. Type, `kill -9` faro after a couple of seconds, restart: still there.
- [ ] **Sending:** type and press `Enter`: the message appears at once with `◌`, then `✓`, `✓✓` and a coloured `✓✓` as the mock "delivers" and "reads" it; sometimes someone replies. The input and the conversation's draft are empty afterwards (restart to check). Pressing `Enter` on a blank or spaces-only input does nothing. Sending while scrolled up jumps to the bottom.
- [ ] **Selection:** `↑` on an empty input highlights the newest message (tinted row, `▌` bar); `k`/`j` move through messages and the view follows, including up into older history; `↓` past the newest puts you back in the input. `Tab` into the messages and clicking a message select too. Check the tint is visible but subtle in your terminal's colours.
- [ ] **Reply:** `r` on a message shows "↳ Replying to …" above the input; send: the new message shows the quote. `Esc` before sending cancels the reply.
- [ ] **React:** `+` opens the picker just above the message; `1`–`6` or arrows + `Enter` add a reaction (shown under the message); picking the same one again removes it. Check the emoji line up in kitty and under tmux.
- [ ] **Copy:** `y` copies the message text: paste it in another app on your *local* machine, both directly and over SSH. Under tmux this needs `set -g set-clipboard on`.
- [ ] **Links:** send yourself a message with a link (e.g. `see https://signal.org.`), select it, press `o`: locally the browser opens on the link without the full stop; over SSH it's copied instead (footer says so). A message with two links shows a menu.
- [ ] **Menu:** `Enter` on a message (or right-click) shows the action menu above it; `↑↓` + `Enter` or the letter shortcuts work; `Esc` or a click closes it. On a failed message ("! not sent"), the menu offers "Retry sending" first.
- [ ] **Pin:** `Tab` to the sidebar, `p` pins the highlighted conversation under "▾ Pinned" (footer: "Pinned (Alt+N opens it)"); pin a second one, then `K`/`J` reorder them. `Alt+1`/`Alt+2` open them from the input (on macOS this needs Option as Alt). Under tmux, check `Alt+digit` isn't taken by your tmux config (`bind -n M-1` etc.).
- [ ] **Hide:** `h` hides the open conversation and the next one opens; it shows up again in the list as soon as a mock message arrives in it.
- [ ] **Archive:** `a` archives; the list ends with "▸ Archive (N)". `A` (or clicking that row) shows the archive with "Archived" and "Hidden until a message" sections and a "◂ Chats" row at the top; `a`/`h` there bring a conversation back. New messages don't unarchive.
- [ ] **Mute:** `m` opens the mute picker next to the conversation; `1`–`5` or arrows + `Enter`. The conversation turns dim (name, dot and count) and drops out of the footer's unread total; `Alt+I` shows "Muted until …". `m` again unmutes. Check the dim colour still reads well.
- [ ] **Allowlist:** `l` on two conversations, then `L`: the title says "Conversations · allowlist" and only those (plus pinned ones) remain; the rest are in the archive under "Not on the allowlist". Quit and restart: still in allowlist mode. `L` again turns it off.
- [ ] **Conversation menu:** `Space` in the sidebar, or right-clicking a conversation (without opening it), shows its menu beside the list; the letter shortcuts work (including `h`, which in menus is Hide, not a move).
- [ ] **Persistence:** pins, mutes, hidden, archived and allowlist all survive a restart.
- [ ] **Keys after the key map rewrite (D43):** everything above still works the same, and the footer hints look as before in each pane (input, sidebar, archive, messages).
- [ ] **Palette:** `Ctrl+P` opens a box near the top over the conversation; type `@ali`: matched letters are highlighted, `Enter` opens Alice. Try `>mute 8` (mutes the open conversation for 8 hours), `@` (conversations, hidden/archived ones marked), `?` (keybinding list), `:` (settings). Since D52 the palette is a tree; see "Palette tree" below. Run a command, reopen: it's on top. Arrows, `Tab`, `Ctrl+N/P`, wheel and clicking a result work; clicking outside or `Esc` closes. The cursor sits in the query box. Check under tmux that `Ctrl+P` reaches faro (not bound in your tmux config). Known: where the box overlaps emoji in the sidebar, its left edge can look broken under tmux (the emoji width issue, M8 task 2).
- [ ] **Help:** `F1` anywhere, or `?` in the sidebar or message pane, opens the keybinding list; `?` in the input types a `?`.
- [ ] **Bulk hide:** palette → "Hide conversations inactive for 1 week": old mock conversations disappear into the archive's "Hidden until a message"; one comes back when a message arrives.
- [ ] **Read only when seen:** with `set -g focus-events on` in tmux, switch to another tmux window (or another app) while a mock message arrives in the open conversation: the sidebar count goes up and a "new messages" rule appears. Come back: the count clears. Same when scrolled up with `PgUp`: the count clears once you scroll back to the bottom. Without `focus-events`, the count clears on your next key press instead.
- [ ] **Typing indicators** can't be seen with the mock (it ignores them); check them in M7 on the test account's phone: "typing…" appears while you type, disappears ~3 s after you stop, and when you switch conversation.
- [ ] **Bubbles (D45):** the default layout. Others' messages sit on the left in a cool-coloured box, yours on the right in a warm one; times and ticks at the bottom right inside each box; reactions just under it; in groups the name is above the first bubble of a run. Check the two colours are distinct from each other, from the background and from the selection tint, in kitty and under tmux. Emoji in bubbles may push the right edge out by a cell under tmux (the emoji width issue, M8 task 2).
- [ ] **Selection over bubbles:** `↑` on an empty input: the selected bubble's rows get the tint and `▌` bar across the pane; check it's obvious which message is selected, including your own (right-hand) ones.
- [ ] **Layout switch:** palette → `:` Message layout › IRC: the classic `time  name  text` view; "bubbles" switches back. Quit and restart: the last choice is kept (it's in the config file now).
- [ ] **Failed sends:** run without `--mock-no-faults` and send while the footer says offline: the message shows `! not sent` in red with an error notice. Quit right after sending and restart: a message that hadn't been confirmed shows `! not sent`, not `◌`.


## Palette tree (D52)

- [ ] **Root and levels:** `Ctrl+P` shows five categories (Chats, Themes, Settings, Commands, Keys) with `›` after each and the title "Command palette". `Enter` on Settings: the title becomes "Settings"; `colo` + `→` on Colours: "Settings › Colours", its values with `●` on the current one. `←` goes back with Colours still selected; `Backspace` on the empty query goes back again. `Esc` closes from any level.
- [ ] **Typing filters the level only:** at the root, `ali` shows "No matches (@ chats, …)"; `@ali` jumps into Chats and finds Alice. `>mute 8` + `Enter` mutes the open conversation for 8 hours. `?` lists the keys; `F1` and `?` outside the input open Keys directly, and `Backspace` from there shows the root.
- [ ] **Cursor keys in a query:** type `abc` in a level: `←` moves the cursor (doesn't go up) until it's at the start; `→` moves it back, and at the end opens the selected category.
- [ ] **Mouse:** the wheel moves the selection; clicking a category opens it, clicking a command runs it; clicking outside closes.
- [ ] **Footer:** at the root it lists `@ chats · : settings · > commands · ? keys`; below the root `← back`; at Settings `enter flip/open`.

## One settings list and colour-depth notes (D57)

- [ ] **The list:** `Ctrl+P`, `:`: every setting at once under bold, dim section headers (Appearance, Notifications, Conversations, Input), each with its current value on the right; the multi-value settings end in `›`. Say whether it reads well and whether the headers stand out enough from the settings.
- [ ] **Headers are skipped:** `↑`/`↓` never stop on a header (and wrap from Colours to Mouse and back); `PgUp`/`PgDn` and the wheel don't either; clicking a header does nothing.
- [ ] **Filtering:** `:bell` shows only "Notifications" and Bell; `:notif` shows both notification settings; `:zzz` says "No matches".
- [ ] **Flip in place:** `:bell` + `Enter`: the row turns `off`, the footer says `bell off`, the palette stays open with `bell` still typed; `Enter` again turns it back on. Same with a click on Mouse (then the mouse is off, so turn it back with the keyboard). The config file changes each time.
- [ ] **Values:** `Enter` on Colours, Emoji, Message layout or Bubble style opens its values (`●` on the current one); choosing one applies it and closes.
- [ ] **Themes once:** themes are only at the root (`Ctrl+P`, Themes); the settings list has no Theme row (D60).
- [ ] **256-colour note:** `env -u TMUX COLORTERM= TERM=xterm-256color faro --mock …` (outside tmux, or forced with `Settings › Colours › 256`): Themes shows a dim line "256 colours: themes are approximated (truecolor: Settings › Colours)" above the list. Preview solarized: its blue background turns grey, as the README says. At truecolor there's no note.
- [ ] **16-colour note:** `Settings › Colours › 16`: Themes says "16 colours: the system theme is drawn; others need 256 colours or more". Choose `nord`: the notice says "Theme: nord (drawn as system until 256 colours or more)", faro stays in `system`, and `theme = "nord"` is in the file. `Settings › Colours › auto` (in kitty): nord appears.

## M4.5: emoji widths (D47)

- [ ] **Plain ssh, tmux:** run `faro --mock` in the usual tmux with some traffic (default activity). Borders, unread counts and bubbles stay straight in emoji-heavy chats (Family, Bogdan, Mum ❤️, Climbing). This includes the ratatui fix, which affected every terminal.
- [ ] **Through mosh:** attach the same tmux session through mosh. Within a couple of seconds of focusing the faro pane, a notice says mosh was detected, emoji switch to the compatible form (❤️ → ❤, families split into people) and borders are straight in the faro pane. Other panes (e.g. the coding harness) can still look wrong through mosh; faro can't fix those.
- [ ] **Detach mosh, keep ssh:** after the mosh client detaches, focusing faro switches back to full emoji with a notice.
- [ ] **Override:** `faro --mock --emoji compatible` and `--emoji full` keep that mode whatever is attached (no notices).
- [ ] **Ctrl+L** repaints the whole screen without quitting or freezing.
- [ ] **Palette over emoji:** open `Ctrl+P` and resize the pane so the palette's left border lands on sidebar emoji. The border stays unbroken (the emoji under it is blanked).
- [ ] **Copy keeps the original:** in compatible mode, select a message with ❤️ or a family emoji, `y`, paste elsewhere. The original emoji are pasted.
- [ ] **Half flags:** a lone regional indicator shows as a letter (needs a real message containing one; the mock has none, so this waits for M7).


## Emoji glitches under tmux (D55)

- [ ] **Flags over ssh:** `faro --mock --emoji full` in your usual tmux, attached over plain ssh. Find the `🇷🇴 🇩🇪 🇯🇵 🏳️‍🌈 🏴‍☠️` message and scroll it up and down past the top and bottom (`PgUp`/`PgDn`, wheel) for a while, also with the sidebar's `Lukas 🇩🇪` visible: no tofu, stray characters or shifted border pipes. Detach and re-attach tmux, switch windows away and back: still clean.
- [ ] **Flags through mosh:** the same through mosh (compatible mode, auto-detected): flags show as letters (`RO DE JP`), the rainbow and pirate flags as their parts; no trail while scrolling.
- [ ] **Skin tones:** reactions and messages with 🫶🏽 (and 👍🏽) stay aligned. 👍🏽 keeps its tone; 🫶🏽 is drawn without one (tmux can't join that tone; expected).
- [ ] **✌ and friends:** the mock's `❤️ ☺️ ✌️ (variation selectors)` message keeps the bubble's right edge and the border straight in tmux over ssh; through mosh it reads `❤ ☺ ☮` and is straight too.
- [ ] **ZWJ sequences without VS16:** type `🧗‍♀` or `❤‍🔥` without the trailing VS16 (e.g. paste them from a terminal `printf '\U1F9D7\u200d\u2640'`) into the input: the cursor and the input border stay where they should; faro shows them fully qualified (🧗‍♀️, ❤️‍🔥).
- [ ] **Stale mosh clients:** `tmux list-clients -t faro`. If an old mosh client is listed while you're on ssh, faro stays in compatible mode. Detach it (`tmux detach-client -t <tty>`) and focus faro: it switches back to full emoji with a notice.
- [ ] **Directly in kitty (no tmux):** `faro --mock` in a kitty window: flags and emoji look right and the borders are straight.

## Bubble styles (D58)

Run `faro --mock --config /tmp/faro-test.toml` (bubbles is the default layout). `Settings › Bubble style` in the palette switches between auto, rounded, bar and mixed (D66).

- [ ] **Rounded in kitty, directly:** with `auto`, faro in a plain kitty window draws rounded bubbles. One-line bubbles are pills: half circles at both ends that meet the fill with no seam. Taller bubbles have edges above and below, two thirds of a row high, with diagonal corners that read as rounded and meet the fill without a gap or lighter line. Check at two or three font sizes and with a couple of fonts (kitty draws these glyphs itself, so they should look the same in every font).
- [ ] **Rounded in kitty, under tmux over ssh:** with only kitty clients attached to the session, `auto` is rounded too. Attach a second client from another terminal (or through mosh): on the next focus change, faro switches to bars; detach it and refocus: rounded again.
- [ ] **Through mosh:** `auto` draws bars (mosh hides the terminal). With `bubble_style = "rounded"` the rounded shapes show and stay aligned while scrolling.
- [ ] **Bars:** `bubble_style = "bar"`: others' bubbles have a heavy bar on their left edge in the sender's colour (the same as their name in groups; one colour per person in direct chats), yours a bar on their right edge in the theme's primary colour. The box keeps its fill; text starts just after the bar. Say if the text feels too close to the bar.
- [ ] **Light and dark:** `faro-dark`, `faro-light` and one OpenCode theme (e.g. `nord`), in both styles: caps, edges and bars are visible and look intentional on the pane background.
- [ ] **`system` theme:** Theme › system: bars whatever the style, with a blank line between bubbles.
- [ ] **Blank lines:** consecutive bubbles are separated by a blank line in both styles; pills don't touch.
- [ ] **Selection:** select a message (`Tab` to the messages, `k`) in both styles: the selected bubble is one clean tinted block (no caps or edge glyphs left half-coloured); in bar style the bar keeps its colour.
- [ ] **Reactions:** reactions sit under the bubble (under the bottom edge for taller rounded ones).
- [ ] **Palette and file:** choosing a style writes `bubble_style = "…"` to the config; editing the file applies it live ("Settings reloaded").
- [ ] **Old setting:** a config with `bubble_corners = "square"` (from D56) draws bars and the footer says to write `bubble_style = "bar"`.
- [ ] **Narrow pane:** shrink the pane to about 40 columns: bubbles stay intact in both styles, edges as wide as the bubble.

## No chat open (D60)

- [ ] **Startup:** faro starts with no conversation open: the message pane shows the logo (D61), the list has the focus and highlights the chat you had open last (a plain highlight, the name not in the accent colour). Its unread count is unchanged.
- [ ] **Browsing doesn't read:** `j`/`k` (and `Alt+J/K` from anywhere) move the highlight; nothing opens, unread counts stay. `m`, `p`, `h`, `Space` act on the highlighted chat. `Enter` or a click opens it; only then does its unread count clear.
- [ ] **Esc steps back:** in a chat, write a draft, press `r` on a message to reply, then `Esc` repeatedly: the reply bar goes, then (from the list or messages) focus returns to the input, then the chat closes and the logo comes back with the list focused on that chat. Reopen it: the draft is there. The footer shows `esc close` while writing.
- [ ] **Incoming while closed:** with nothing open, new messages (the mock sends them) raise unread counts and ring the bell (if on); nothing opens by itself, and the other phone (M7) shows them unread.
- [ ] **Hidden list:** hide the list (`Alt+S`), then `Esc` out of the chat: the list comes back, focused.

## The logo screen (D61)

- [ ] **Sizes:** with no chat open, resize the terminal (inside tmux) through roughly 200×50, 130×24 (lighthouse and wordmark side by side), 100×32 (stacked, big wordmark), 100×26 (stacked, small wordmark or the small lighthouse), 80×20 (small lighthouse with "faro" and the tagline beside it) and 60×12 (just "faro"). Each is centred, nothing is cut or wraps, the hints under it are readable and their keys line up.
- [ ] **It looks like the prototype:** compare with `python3 tools/logos.py anim faro` in the same terminal: red and white stripes, the lamp glowing as the beam sweeps, waves rolling, stars twinkling.
- [ ] **Smooth, and only when needed:** the beam turns smoothly (about 10 frames a second) directly in kitty, in tmux, over SSH and over mosh (mosh may drop frames; it shouldn't smear or leave stray dots). Open a chat: the animation stops (`top` shows faro idle). Switch to another tmux window or app so the terminal loses focus: it stands still and resumes when you come back.
- [ ] **CPU:** in a release build with the mock (`--mock-activity 0`), `top` shows faro's CPU with the logo moving (a few percent at most) and close to zero with it still (`reduce_motion`, unfocused, or a chat open).
- [ ] **Reduce motion:** `Ctrl+P`, `:`, `Reduce motion` → `Enter`: the logo stops at once and `reduce_motion = true` is written to the config; flip it back and it moves again. Editing the file by hand does the same live.
- [ ] **Light theme:** with `faro-light` (and an OpenCode light theme such as `github-light`) the lighthouse has grey and red stripes, an amber lamp and beam, no stars, and the wordmark's left edge is clearly visible.
- [ ] **256 and 16 colours:** `Settings › Colours` → 256: the logo keeps its look in approximated colours. → 16: only the small lighthouse with "faro" is drawn, in your terminal's own colours (red, yellow, default text), and nothing animates. Also try the `system` theme at truecolor: the same small lighthouse.
- [ ] **No glitches beside another pane:** split the tmux window with another pane next to faro and leave the logo animating for a minute, also while typing in the other pane: no stray characters, nothing shifts in either pane, the border stays straight.
- [ ] **Empty and loading:** with a fresh `--data-dir` the logo shows "Loading…" briefly; with no conversations at all it says "No conversations yet" and hints only the palette and keybindings.

## Setting previews (D59)

- [ ] **Bubble style:** `Ctrl+P`, `:bubble`, `Enter`: the list opens on the style in use and nothing changes. `↓`/`↑` redraw the bubbles behind the palette in each style straight away; `Esc` puts the old style back and the config file is unchanged; `Enter` keeps the selected one, closes the palette and writes `bubble_style`.
- [ ] **Layout and colours:** the same with `:layout` (bubbles ↔ IRC behind the palette) and `:colours` (256 and 16 visibly change the colours; 16 draws `system`). `←` back to the settings list also puts the value in use back.
- [ ] **Footer:** in those lists (and in Themes) the footer says `↑↓ preview · enter use it · ← back · esc cancel`.
- [ ] **No flicker or leftovers:** move quickly through the values a few times: no stray characters or misaligned borders after each redraw (if any, say which setting).

## M5: configuration (D48)

Use a scratch config to keep your real one clean: `faro --mock --config /tmp/faro-test.toml` (or try the real `~/.config/faro/config.toml`, which faro creates on first run).

- [ ] **First run:** the file is created with every setting commented out, each with a one-line explanation, then every default key binding per context. It reads well in your editor; nothing in it looks wrong or misleading.
- [ ] **Live edit:** with faro running, uncomment `layout = "bubbles"` and change it to `"irc"`, save: within about half a second the layout changes and the footer says "Settings reloaded". Works with your editor's normal save (vim/neovim write via a temporary file and rename; check that's picked up).
- [ ] **Bad value:** set `layout = "bubble"` and save: the footer shows a red "Config: messages.layout: expected … keeping …" and the layout doesn't change. Fix it: "Settings reloaded".
- [ ] **Broken file:** delete a closing quote and save: a red "Config: line N: …" and nothing changes. Fix it again.
- [ ] **Mouse off:** palette (`Ctrl+P`) → `:` Mouse (`Enter` flips it off), or `[mouse] enabled = false`: clicks and wheel no longer reach faro, and you can select text with the mouse in kitty and in tmux (tmux's own mouse mode decides there). `Enter` on Mouse again brings it back. Check the terminal is left in a sane state after quitting either way.
- [ ] **Palette toggles keep your comments:** add a comment of your own to the file, then palette → `:` Message layout › IRC and `:` Allowlist mode (`Enter` flips it on): only those lines change in the file (open it), your comment is still there, and no "Settings reloaded" appears for faro's own write.
- [ ] **Key binding:** add `"f2" = "palette.open"` under `[keybindings.global]`, save: `F2` opens the palette (check tmux passes F2 through). A typo in an action name shows a red notice and the rest still works.
- [ ] **Emoji setting:** under mosh, `:` Emoji › full / compatible / auto switch immediately (auto re-detects mosh). With `--emoji full` on the command line, the palette marks emoji as decided by `--emoji` and choosing one only saves it.
- [ ] **Migration:** with a data dir from before M5 where you had switched to IRC or turned on allowlist mode, the first M5 start writes `layout = "irc"` / `allowlist = true` into the config file (uncommenting the template line), and the mode is as you left it.
- [ ] **Symlinked config:** if your config is a symlink into a dotfiles repo, a palette toggle edits the target and the link stays a link.

## M6: notifications (D49)

Run `faro --mock --mock-no-faults --config /tmp/faro-test.toml` in a tmux window (a mock message every few seconds), then switch to another window.

- [ ] **Bell under tmux:** a message in a background conversation marks faro's window with tmux's bell flag in the status line (`monitor-bell` is on by default; `bell-action` decides whether tmux passes it on). In kitty without tmux, the bell is whatever kitty does (visual or audible, `enable_audio_bell`).
- [ ] **No bell for what you're reading:** with faro focused, at the bottom of a conversation, messages arriving there don't ring; scroll up (or switch to another tmux window, with `focus-events on`) and they do.
- [ ] **Not too often:** with `--mock-activity 50` (a flood), the bell rings at most every 2 seconds, not on every message.
- [ ] **Muted / archived:** mute a busy conversation (`m`) or archive it (`a`): its messages no longer ring. `[notifications] bell = false` silences everything.
- [ ] **Window title:** the tmux pane title (`tmux display -p '#{pane_title}'`, or `set -g set-titles on` to see it in kitty's title bar) reads `faro (N)` with the footer's unread count, and `faro` when everything is read. Directly in kitty, the window/tab title does the same. After quitting (and after `:` Notifications › Unread count in the window title › off, or `title = false`), your previous title is back.
- [ ] **Mentions only:** `@` on a conversation in the sidebar (or its menu, or the palette) shows "Notifying for mentions only", dims it in the list like a muted one, and the info panel (`Alt+I`) says "Mentions only". Its messages no longer ring or count in the title, except that a message mentioning you still rings (D72; with `--mock`, some group messages start with `@You`). `@` again goes back to every message.

- [ ] **Do not disturb (D53):** `Alt+N` (check it reaches faro under tmux and, on macOS, with Option as Alt): a notice says the bell is off, the footer shows `bell off` in orange after "connected", background messages no longer ring, and the config file now has `bell = false` under `[notifications]`. `Alt+N` again: the marker goes and the bell rings again. Same from the palette (`>disturb`). Restart with the bell off: the marker is back.

## M8: themes and colours (D51)

Switch with the palette (`Ctrl+P` › Themes) or `theme = "…"` in the config; colours with `:` Colours or `[ui] colors = "…"`.

- [ ] **Each theme in kitty, directly and under tmux:** `faro-dark`, `faro-dusk`, `faro-light`. Everything is readable: message text, times and ticks (muted), the selected sidebar row, the palette's selected row, menus, the "↓ N new messages" badge, both bubble colours (distinct from each other and from the background), sender names in a group. Nothing looks out of place in the light theme (e.g. a dark block left over from the dark theme).
- [ ] **system theme:** Themes › system in a dark and in a light kitty colour scheme: faro uses the terminal's background and text colours; the selected sidebar/palette/menu row and the selected message are in reverse video; muted text is dim; bubbles have no fill (own messages on the right, others on the left). Readable in both schemes.
- [ ] **256-colour fallback, detected:** outside tmux, `TERM=xterm-256color COLORTERM= faro --mock …`: `:` Colours says "auto (256 colours detected)"; the theme looks close to the truecolor one (greys may be a step off), and the bubbles, selection and input box are still distinct from the background.
- [ ] **Plain tmux:** in `tmux -f /dev/null` (no `terminal-features RGB`) faro sends truecolor and tmux rounds it to 256 colours itself: it should look like the previous check, not garbled.
- [ ] **macOS Terminal.app** (no truecolor before macOS 26): detected as 256 colours (`TERM_PROGRAM=Apple_Terminal`), and looks right.
- [ ] **Forced depths:** Colours › truecolor in a 256-colour terminal shows what happens when the guess is overridden wrongly (garbled or approximated colours, terminal-dependent); Colours › 16 switches to the system theme whatever theme is set; Colours › auto goes back.
- [ ] **Live:** editing `theme` in the config file while faro runs switches the theme within about half a second; a misspelt name shows a warning pointing to `Ctrl+P › Themes` and keeps `faro-dark`.

## Themes: OpenCode's, your own, live preview (D54)

- [ ] **Preview:** `Ctrl+P` › Themes starts on the theme in use (marked `●`). `↓`/`↑` and the wheel redraw the whole UI (sidebar, messages, bubbles, footer, the palette itself) in the selected theme at once; typing `nord` previews the best match. `Esc` puts the configured theme back; so do `←` / `Backspace` (up to the root) and a click outside. `Enter` keeps it: a notice says `Theme: …`, the config file gets `theme = "…"`, and reopening Themes starts on it. Footer while there: `↑↓ preview · enter use it · ← back · esc cancel`.
- [ ] **OpenCode themes in kitty, directly and under tmux:** spot-check `tokyonight`, `catppuccin`, `gruvbox`, `nord`, `dracula`, `solarized`, `github-light`, `catppuccin-light`, `lucent-orng` (transparent upstream: drawn on black here) and `vesper-light`. Each should look like itself (compare with OpenCode if it's at hand) and stay usable: text, times (muted), the selected row, the palette, unread badges, both bubbles distinct from each other and from the background, sender names in a group chat distinguishable. Note any theme that looks wrong.
- [ ] **256 colours:** a few of those with `[ui] colors = "256"`: close to truecolor; bubbles, selection and the input box still distinct.
- [ ] **Your own theme:** copy `crates/faro/themes/opencode/nord.json` to `~/.config/faro/themes/mynord.json` while faro runs: a notice says "Themes reloaded" and `mynord` (and `mynord-light`) appear under Themes, labelled "your file". Choose it, then change its `background` in the file and save: the background changes within about half a second, without touching the palette.
- [ ] **A broken theme file:** save `~/.config/faro/themes/broken.json` with `{`: a warning names the file and says what's wrong; the other themes still work. Restart faro: the same warning shows at startup. Remove the file.
- [ ] **Replacing a built-in theme:** a file named `nord.json` in the themes directory replaces OpenCode's `nord` (one `nord` listed, labelled "your file"); one named `system.json` is refused with a warning.

## Mixed bubbles (D66)

- [ ] `Ctrl+P` › Settings › Bubble style › mixed (previews while moving): your bubbles have the bar on the right and round on the left; others' the bar on the left and round on the right. Taller bubbles have square corners on the bar's side, and the bar looks continuous down the text rows. Check in kitty direct and through tmux, a dark and a light theme, and with a message selected (a plain tinted block).

## Allowlist guard (D62)

- [ ] `faro --mock --allowlist <file>` with a missing file, or with `people = ["someone"]`: faro exits at once with an error naming the file and the problem.
- [ ] With an empty list (`people = []` and `groups = []`): no conversations show up while the mock is active. The log says `allowlist guard on … people=0 groups=0` and counts dropped events, never names or text.
- [ ] With a real account (M7): a message from someone off the list never appears. Sending to an allowed chat works.

## M7: the Signal backend (D63)

- [ ] Contacts (D68), personal account: a minute after starting, `Ctrl+P` › Chats lists your Signal contacts (no restart needed; an iPhone answers only once Signal is opened on it); the log says "asked the phone for its contacts" and "contacts synced from the phone".
- [ ] `--no-allowlist` (D67), on your own account: faro starts, the footer says `no allowlist`, and every chat shows up as messages arrive. Without either flag faro refuses and names both.

Use a separate data dir for the test account, e.g. `--data-dir ~/.local/share/faro-test`.

- [ ] `faro link --data-dir …` shows a QR code that the phone scans at once (Signal › Settings › Linked devices › Link new device). The phone lists "faro (<host>)". faro prints the account's ID.
- [ ] Running `faro link` again on the same data dir refuses and explains how to start over.
- [ ] `faro ids --data-dir … --username <yours>` prints your ID and, after a message in the test group, the group's title and ID, plus an allowlist to copy.
- [ ] `faro --data-dir …` without `--allowlist` refuses to start.
- [ ] With the allowlist: the group and your direct chat appear, with names. A message from the phone shows up as outgoing. Messages, replies, reactions, edits and deletions work both ways. Typing indicators and receipts (✓✓, read) show.
- [ ] A message from anyone else never appears.
- [ ] Caught up (D64): quit faro, send a message to the group from the phone, run `faro ids` (it reads the queue), then start faro: the message is there, unread. Restarting again doesn't duplicate it or count it twice.
- [ ] Names (D64): group members show their Signal profile names (in the sidebar, the palette's `Chats ›` and message senders), not the start of their ID.
- [ ] The palette's `Chats ›` lists the allowed chats even before they have messages; choosing one opens it.
- [ ] Same names (D65): two people with the same profile name show as "Name · xxxx" and "Name · yyyy" (the first 4 hex digits of their IDs) in the palette's `Chats ›`, the sidebar and as message senders.
- [ ] Nicknames (D65): `Space` on a direct chat › "Set a nickname…", type a name, `Enter`: the name changes everywhere, the other person's tag goes away, and it survives a restart. Opening it again starts from the nickname; clearing the text (or "Remove the nickname") brings the profile name back. `Ctrl+P` › Commands › "Set a nickname…" lists everyone, group members too.
- [ ] Read on the phone (D70): with faro running and a chat showing unread messages, open that chat on the phone: within a few seconds faro's count for it clears (sidebar, footer total, window title). With the chat open in faro but the terminal unfocused, the "new messages" rule above those messages goes too. Reading only part of a busy group on the phone leaves the newer messages unread.
- [ ] Read in faro (D71): send a message to the test account from another phone or person, leave it unread on the test phone, then open the chat in faro (terminal focused, at the bottom): within a few seconds the test phone's unread badge for that chat clears. The sender still sees it as read (✓✓ filled).
- [ ] Mentions (D72): in the test group, have someone mention you and someone else from their phone (type `@` and pick). In faro both show as `@Name` in bold accent, yours (`@You`) reversed, in IRC and in bubbles (`:` › Layout), never as a blank box; wrapped mentions keep their colour on the next line. `y` on the message copies the names as text. Set the group to "mentions only" (`@`), unfocus the terminal: a message mentioning you rings the bell, one mentioning only someone else doesn't. Mute the group: a mention of you no longer rings. Edit a message on the phone to add or remove a mention: faro follows.
- [ ] Reaction removal (D73): react to a message in faro (`+`), check the phone shows it, then pick the same emoji again in faro: the reaction disappears on the phone (Android and iOS), not only in faro. Switching to another emoji replaces it on the phone.
- [ ] Wi-Fi off: the footer shows the connection dropping and coming back. Messages sent meanwhile arrive after reconnecting.
- [ ] Restarting keeps the history.

## M9: attachments and inline images (D75–D79)

- [ ] **Mock inline images:** run `faro --mock --data-dir /tmp/faro-m9` and open "Climbing 🧗" (or "Family"): the photo attachment downloads in the background (`· downloading…` briefly) and renders inline above its `📎 crag-photo.png · …` line, both in bubbles and in IRC layout (`Settings › Message layout`). Stickers (`[sticker]`) render as compact inline WebP images (at most 8 rows tall) without a `📎` line when inline images are on.
- [ ] **Protocols (kitty, tmux, mosh, settings):**
  - Directly in kitty and inside tmux (with `allow-passthrough` enabled automatically for the pane): `Settings › Inline images › auto` renders crisp kitty graphics (unicode placeholders) that scroll cleanly at the top and bottom of the message pane.
  - Through mosh, or with `Settings › Inline images › halfblocks`: images render as half-block cells (`▀`/`▄`) in truecolor (or nearest xterm-256 colours when `Settings › Colours › 256`).
  - With `Settings › Inline images › off` or `Settings › Colours › 16`: only the `📎 name · size` line is shown, no image box.
- [ ] **Overlays over images:** in kitty (direct or tmux), open `Ctrl+P` or a message action menu (`Enter`) so it overlaps an inline image: while the overlay is open, the image box cleanly drops to its `[image …]` placeholder with no bleed-through or garbled cells, and redraws the image as soon as the overlay closes.
- [ ] **Image viewer popup:** select a message with a downloaded image (`↑` or click) and press `v` (or `Enter` › *View*): a full-window popup displays the image scaled to fit, with the file name and dimensions in the title and `s save · y copy path · esc close` in the footer (`o open` too when running locally). `Esc` closes it.
- [ ] **Non-auto downloads and retry:** in the mock chat, find a PDF attachment (`route-topos.pdf · … · not downloaded (d)`), select the message and press `d` (or `Enter` › *Download*): it downloads and the status suffix disappears. With `Settings › Auto-download › off`, incoming images wait for `d` too; with `all`, PDFs download automatically as well.
- [ ] **Open, Save to Downloads, Copy path (SSH-aware):**
  - Select a downloaded attachment and press `s` (or `Enter` › *Save to Downloads*): the footer confirms `Saved to …/Downloads/…`; pressing `s` a second time saves `name (1).ext` without overwriting.
  - In the message menu (`Enter`), *Copy path* copies the local path via OSC 52.
  - When running locally, *Open* (`o` in the viewer or from the menu) opens the file in the system viewer (`open` / `xdg-open`). Over SSH, *Open* is omitted from the menu so it never opens on the remote machine.
- [ ] **Sending a file from the palette:** in an open chat, open `Ctrl+P` › *Send a file…*: browse folders with `↑`/`↓` and `Enter`, use `Backspace` on an empty filter to go up a folder, or type `~/` to jump to your home directory. Press `Enter` on a file: the palette closes and a `📎 filename · size  esc remove` bar appears above the input box. Press `Esc` once to remove the staged file (leaving any reply bar intact), or stage it again and press `Enter` (with or without a text caption) to send. The sent message echoes immediately (and renders inline if it's an image).
- [ ] **Sending a file by pasting a path:** drag a file into the terminal (or paste `/path/to/file.png` into the input box): instead of inserting the path text, faro stages the file in the attachment bar above the input. Pasting ordinary text or a non-existent path inserts the text as usual.
- [ ] **Real Signal account (with `--allowlist` on the test account, or `--no-allowlist` on personal):**
  - Send a photo, a sticker, and a document from a phone to faro: the photo and sticker download into `<data dir>/attachments/` (`0600`) and render inline; the document downloads on `d` and saves with `s`.
  - Send an image and a document from faro (with and without a caption): both arrive on the phone with their filename, content type, dimensions, and caption intact.

## M10: voice notes and transcription (D80–D82)

- [ ] **Mock voice notes & waveform:** run `faro --mock --data-dir /tmp/faro-m10` and open "Climbing 🧗" (or "Family"): voice note messages auto-download (`· downloading…` briefly) and render as `▶ 0:04 ▁▃▆█▅▂▃▁… · 64 KB` in both bubbles and IRC layouts (`Settings › Message layout`).
- [ ] **Local playback and live progress (`x` / `v`):**
  - When running locally (not over SSH or mosh), select a downloaded voice note (`↑` or click) and press `x` (or `v`, or `Enter` › *Play*): audio plays through the system player (`afplay` on macOS, `mpv`/`ffplay`/`pw-play`/`paplay`/`aplay` on Linux, or `[voice] player` in `config.toml`).
  - While playing, the row shows `■ 0:01 / 0:04 … · playing`, advancing every second and colouring the played portion of the 16-bar waveform in the theme's accent colour.
  - Pressing `x` (or `Enter` › *Stop*) stops playback immediately; starting another voice note stops the first.
- [ ] **SSH and mosh playback guard:** over SSH (`SSH_CONNECTION` / `SSH_TTY`) or mosh, pressing `x` on a voice note does not play audio on the remote host; instead it shows a warning notice pointing to `s` (*Save to Downloads*) and *Copy path* (OSC 52), and the message menu omits *Play*.
- [ ] **Opt-in local Whisper transcription (`[voice] transcribe = "local"`):**
  - Set `transcribe = "local"` and `transcribe_model = "/path/to/ggml-base.bin"` under `[voice]` in `config.toml` (or via `Ctrl+P` › `:` › *Voice note transcription* › `local`, or with a custom wrapper script in `transcribe_command`).
  - Select a voice note and press `t` (or `Enter` › *Transcribe*): the row shows `· transcribing…` (and `❝ transcribing…` beneath it) without blocking the UI, then renders `❝ <text> [local]` underneath. Restart faro: the transcript is persisted in SQLite and still shown.
  - Press `y` (*Copy*) or `r` (*Reply*) on a transcribed voice note: the transcript is included in the copied text and reply quote.
- [ ] **Opt-in remote Whisper transcription, consent popup & per-conversation permission (`[voice] transcribe = "remote"`):**
  - Put `transcribe_api_key = "sk-..."` under `[voice]` in `config.toml`: faro rejects it with a footer warning that API keys must come from `$FARO_TRANSCRIBE_API_KEY`, `$OPENAI_API_KEY`, or the OS keychain (`security` on macOS, `secret-tool` on Linux).
  - Choose `Ctrl+P` › `:` › *Voice note transcription* › `remote`: a modal consent popup explains that remote transcription sends people's voice recordings to `<transcribe_url>/audio/transcriptions` and requires enabling per conversation. `Esc` cancels and leaves transcription as it was; `Enter` on *I understand — enable remote transcription* records consent and sets `transcribe = "remote"`.
  - In a conversation where remote transcription has not yet been allowed, press `t` on a voice note: faro asks for confirmation (or toggle *Allow remote voice transcription* from the conversation menu `Space` or `Ctrl+P` › `>`). Once allowed for that chat, `t` (and incoming voice notes ≤ `transcribe_max_secs`) transcribe in the background and show `❝ <text> [remote]`. Disallowing it in the conversation menu stops sending that chat's voice notes.
- [ ] **Real Signal account:** send a voice note from a phone to faro: it arrives with `voice_note = true`, auto-downloads into `<data dir>/attachments/` (`0600`), extracts its duration and 16-bar waveform (via `ffmpeg` for AAC/M4A or Opus), plays locally with `x`, and transcribes when `t` is pressed.

## M8: terminal matrix (D50; run after M7 against the real backend, and once now with the mock)

Columns: **direct** = faro straight in kitty; **tmux** = kitty → tmux (with the README's tmux settings); **SSH** = kitty → ssh → tmux on the other machine; **mosh** = kitty → mosh → tmux. Mark each cell ✓, ✗ (add a note below) or – (not applicable). Also try one other terminal per OS (WezTerm/Ghostty/Alacritty/foot on Linux, iTerm2 and Terminal.app on macOS) for the rows marked †.

| Check (what to look for) | Linux direct | Linux tmux | Linux SSH | Linux mosh | macOS direct | macOS tmux | macOS SSH | macOS mosh |
|---|---|---|---|---|---|---|---|---|
| **Rendering †:** borders straight, no stray characters after scrolling, resizing and flooding (`--mock-activity 50 --mock-no-faults`) | | | | | | | | |
| **Emoji:** emoji-heavy chats aligned; under mosh the "mosh detected" notice and compatible emoji, elsewhere full emoji | | | | | | | | |
| **Colours †:** `:` Colours shows the expected detection (truecolor in kitty and tmux; 256 in Terminal.app / plain SSH without tmux); each theme readable; `system` follows the terminal scheme | | | | | | | | |
| **Mouse:** click a conversation, a message, the input; wheel scrolls messages and the list; right-click menu; `:` Mouse › off lets the terminal select text | | | | | | | | |
| **Clipboard (OSC 52):** select a message, `y`, paste in another app on the machine you sit at | | | | | | | | |
| **Bell:** a message in a background conversation rings (kitty bell / tmux bell flag in the status line) | | | | | | | | |
| **Title:** window or pane title shows `faro (N)`; the old title comes back on quit | | | | | | | | |
| **Focus events:** switch to another window/pane and back: messages arriving meanwhile stay unread and ring; on return they're read | | | | | | | | |
| **Bracketed paste:** paste a multi-line text into the input: it arrives as one message draft with its newlines, nothing is sent early | | | | | | | | |
| **Keys:** `Alt+J/K`, `Alt+1..9`, `Shift+Tab`, `Alt+Enter`/`Shift+Enter` new line, `F1`, `Ctrl+P` all reach faro | | | | | | | | |
| **Ctrl+L:** repaints the whole screen; nothing freezes | | | | | | | | |
| **Quit and restore:** `Ctrl+Q` leaves the shell as it was (cursor visible, no mouse escape garbage when clicking afterwards, title restored) | | | | | | | | |

Notes (terminal, versions, what went wrong):

- 
