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
- [ ] **Emoji widths (known issue, seen by the owner on first try):** in "Climbing 🧗" and other emoji-heavy chats, note which emoji shift the right border. Expected offenders under tmux: emoji with a variation selector (☺️ ✌️ ❤️), flags (🇵🇹), ZWJ sequences (🧗‍♀️, family emoji), skin tones. Record terminal + tmux version; this feeds the emoji width fallback (DESIGN §7.3, PLAN M8 task 2).
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
- [ ] **Read only when seen:** with `set -g focus-events on` in tmux, switch to another tmux window (or another app) while a mock message arrives in the open conversation: the sidebar count goes up and a "new messages" rule appears. Come back: the count clears. Same when scrolled up with `PgUp`: the count clears once you scroll back to the bottom. Without `focus-events`, the count clears on your next key press instead.
- [ ] **Typing indicators** can't be seen with the mock (it ignores them); check them in M7 on the test account's phone: "typing…" appears while you type, disappears ~3 s after you stop, and when you switch conversation.
- [ ] **Failed sends:** run without `--mock-no-faults` and send while the footer says offline: the message shows `! not sent` in red with an error notice. Quit right after sending and restart: a message that hadn't been confirmed shows `! not sent`, not `◌`.

