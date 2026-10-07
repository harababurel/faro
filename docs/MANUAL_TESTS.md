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
- [ ] **Keys:** `Alt+↑`/`Alt+↓` switch conversations (on macOS this needs Option set to send Meta/Esc+ in the terminal's settings; note which terminals need it). `Alt+S`/`Alt+I` toggle the sidebars. `Esc` clears a notice in the footer.
- [ ] **Quit:** `Ctrl+Q` quits; `Ctrl+C` shows a hint and quits on the second press. In both cases the terminal is fully restored: normal screen, cursor visible, echo on, no mouse escape garbage when moving the mouse afterwards.
- [ ] **Signals:** `kill <pid>` (SIGTERM) and closing the SSH session or tmux pane (SIGHUP) leave no broken terminal and the log ends with "faro exiting".
- [ ] **Resize:** shrinking the window below ~110 columns hides the info sidebar, below ~70 the conversation list; nothing overlaps or panics at very small sizes (e.g. 20×5).
- [ ] **Colours:** truecolour renders the amber theme correctly, with a painted dark background (no terminal-background "holes"). Under tmux, check `set -as terminal-features ",*:RGB"` is needed and works.
- [ ] **Flicker:** no tearing or flicker during the flood (synchronized output, DEC 2026). Note terminals that flicker.
- [ ] **Mouse:** mouse capture is on (clicks do nothing yet, which is expected in M3); `Shift`+drag still selects text natively.
- [ ] **Emoji widths (known issue):** in "Climbing 🧗" and other emoji-heavy chats, note which emoji shift the right border. Expected offenders under tmux: emoji with a variation selector (☺️ ✌️ ❤️), flags (🇵🇹), ZWJ sequences (🧗‍♀️, family emoji), skin tones. Record terminal + tmux version; this feeds the emoji width fallback (DESIGN §7.3, PLAN M8 task 2).
- [ ] **Text safety:** no message can change terminal state (title, colours, cursor); control characters show as `�`.
