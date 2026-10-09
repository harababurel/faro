# Pasting images from the clipboard over SSH (research, 2026-10-09)

Status: **proposed, not scheduled** (D89). **Phase 0 done** (D90, see "Phase 0 results" below). This note keeps the investigation, the owner's hands-on experiment and an implementation plan, so the feature can be picked up later without redoing the work.

## Summary

- **Question (owner):** can faro send an image that is on the clipboard of the machine in front of the user, when faro runs elsewhere (SSH, tmux)?
- **Today:** no. faro only *writes* text to the clipboard (OSC 52, `app/system.rs`). Images can be sent with `Send a file…` or by pasting a file path (D77).
- **Answer:** yes over SSH, with and without tmux, when the terminal is **kitty**. kitty's clipboard protocol (**OSC 5522**) reads any MIME type, including `image/png`, in-band over the terminal connection. The owner's setup was tested by hand (below): a 285 KB PNG came back intact through kitty → SSH → tmux 3.8.
- **Not over mosh.** mosh syncs screen state instead of relaying bytes, so the request and the reply are dropped. Out of scope (owner, 2026-10-09).
- **Main implementation risk:** faro's input. crossterm would turn the reply into hundreds of thousands of fake key presses, so faro must read the reply itself (Phase 0 below).

## Why each hop matters

| Hop | Can an app read a clipboard image through it? |
|---|---|
| Terminal | Only kitty's OSC 5522 carries images. OSC 52 is text only, and many terminals refuse OSC 52 reads. |
| SSH | Yes. Escape sequences and replies are just bytes on the connection. |
| tmux | Yes, **if** the request is wrapped in tmux passthrough (`ESC P tmux; … ESC \`, with `allow-passthrough on`). tmux 3.8 delivered kitty's reply back to the requesting pane. A bare (unwrapped) OSC 5522 request is not passed on. |
| mosh | No. mosh runs its own terminal emulator on the server and only syncs the screen; unknown sequences and terminal replies never cross. Same reason inline images fall back to half-blocks under mosh (D78). |

## The experiment (owner, 2026-10-09)

**Setup:** kitty on the owner's local machine → plain `ssh` (checked: not mosh) → Mac mini (`flanders`, fish shell) → tmux 3.8 with `allow-passthrough on`. An image was on the local clipboard.

| # | Where | Command | Result |
|---|---|---|---|
| 1 | inside tmux | `kitten clipboard -g -m image/png out.png` | **Hung forever.** Ctrl+C did nothing (kitten puts the tty in raw mode while waiting). Needed `pkill -f 'kitten clipboard'` from another pane. |
| 2 | over SSH, **outside** tmux (`echo $TMUX` empty) | `perl -e 'alarm 5; exec @ARGV' kitten clipboard -g -m image/png ~/out.png; echo "exit: $status"` | `exit: 0`; kitty showed its "a program wants to read the clipboard" prompt; `~/out.png` = `PNG image data, 1376 x 768`, 514 KB. **Plain SSH works.** |
| 3 | inside tmux | same as 2 | **Hung**; the `perl` alarm did not stop it (kitten seems to handle that signal while waiting); **no kitty prompt**, so the request never reached kitty. Stopped with `pkill`. |
| 4 | inside tmux | hand-wrapped *list types* request, see below | Reply: `^[]5522;type=read:status=OK^[\` `^[]5522;type=read:status=DATA:mime=Lg==;aW1hZ2UvcG5nIHRleHQvaHRtbAo=^[\` `^[]5522;type=read:status=DONE^[\` (payload = `image/png text/html`). **No prompt.** Round trip through tmux works when wrapped. |
| 5 | inside tmux | hand-wrapped `image/png` request, see below | kitty prompted, owner allowed; `status=OK bytes=285083`; `PNG image data, 600 x 313`. **A real image comes back intact through tmux** (about 70 chunks). |

Conclusion: `kitten clipboard` does not wrap its request for tmux; wrapping it by hand (as faro would) makes it work.

### Commands to re-run the test (fish-compatible)

Run inside a tmux pane with an image on the local clipboard. `printf` uses octal (`\033` = ESC, `\134` = `\`) so fish quoting can't mangle backslashes; perl owns the timer, so the command always ends.

List the MIME types on the clipboard (no permission prompt):

```fish
stty raw -echo; printf '\033Ptmux;\033\033]5522;type=read;Lg==\033\033\134\033\134'; perl -e 'alarm 3; $|=1; while (sysread(STDIN, $c, 1)) { print($c eq "\e" ? "^[" : $c) }'; stty sane; echo
```

Read the PNG into `~/out.png` (kitty asks for permission; 15 s to allow):

```fish
rm -f ~/out.png; stty raw -echo; printf '\033Ptmux;\033\033]5522;type=read;aW1hZ2UvcG5n\033\033\134\033\134'; perl -MMIME::Base64 -e 'alarm 15; $buf=""; while (sysread(STDIN, $c, 65536)) { $buf .= $c; last if $buf =~ /status=(DONE|E[A-Z]+)/ } $d=""; while ($buf =~ /status=DATA:mime=[^;]*;([^\e]*)\e\x5c/g) { $d .= decode_base64($1) } open(my $f, ">", "$ENV{HOME}/out.png") or die; binmode $f; print $f $d; close $f; ($s) = $buf =~ /status=(OK|E[A-Z]+)/; print STDERR "status=$s bytes=" . length($d) . "\r\n"'; stty sane; file ~/out.png; ls -lh ~/out.png
```

Outside tmux, drop the wrapper: `printf '\033]5522;type=read;aW1hZ2UvcG5n\033\134'`. Delete `~/out.png` afterwards (it is a copy of the clipboard). If anything hangs, `pkill -9 -f <name>` from another pane, then `stty sane` in the stuck one.

## OSC 5522 in short

Source: kitty's [clipboard protocol](https://sw.kovidgoyal.net/kitty/clipboard/) and the [paste events extension](https://rockorager.dev/misc/bracketed-paste-mime/).

- **Request:** `ESC ] 5522 ; type=read ; <base64 of space-separated MIME types> ESC \`. Payload `.` (`Lg==`) lists the available types.
- **Reply:** `type=read:status=OK`, then `type=read:status=DATA:mime=<b64 mime>;<b64 chunk>` packets (≤ 4096 bytes per chunk before base64, each chunk padded, types sent one after another), then `type=read:status=DONE`.
- **Errors** (instead of `OK`): `ENOSYS` (location not available), `EPERM` (user or system denied), `EBUSY` (e.g. several multiplexer clients at once).
- **Rejected or unsupported requests get no reply at all**, so a timeout is mandatory.
- **Listing types needs no permission** (spec; matches test 4). Reading data prompts (test 5).
- **Fewer prompts (kitty ≥ 0.42.2):** send `pw=<b64 password>:name=<b64 name>` with the request. kitty can offer to allow all future requests with that password on that tty. Use a random password per faro run; `name` must be set or the password is ignored.
- **Paste events (kitty ≥ 0.44.1, private mode `?5522`):** with `CSI ? 5522 h` set, the terminal answers the user's own paste key with an unsolicited type list plus a single-use `pw`; the app then requests the type it wants without a prompt. This **replaces bracketed paste (2004) for every paste**, text included.

## What faro's code means for this

- **crossterm must never see the reply.** In crossterm 0.29 `event/sys/unix/parse.rs::parse_event`, `ESC` followed by anything other than `[` or `O` becomes an Alt+key. A 5522 reply would arrive as `Alt+]` followed by one `Char` key event per byte: about 380,000 key presses for a 285 KB image, typed into the input.
- **D84 says "no new stdin queries after EventStream starts."** This feature needs a deliberate, documented exception (Phase 0).
- **Already in place:**
  - `tui::wrap_tmux_passthrough` (D83 Track 1C) wraps a sequence for tmux.
  - `system::inspect_terminal` → `Terminal { mosh, glyphs, graphics }` reads every tmux client's `client_termname` (kitty detection) and mosh.
  - `OpenConversation::staged_attachment` + the `📎` bar, attachments copied to `<data dir>/attachments/` (`0600`) before `Backend::send`, behind the allowlist guard (D77, D62).

## Proposed design

**When it's offered:** kitty detected (`Terminal::glyphs == BlockGlyphs::Native`, i.e. every tmux client is kitty) and no mosh. Elsewhere the action shows "Pasting images needs kitty without mosh" instead of doing nothing.

**Flow (Phase 1–2):**
1. The user presses a key (proposed `Alt+V`, `input.paste_image`) or picks *Paste image from clipboard* in the palette, with a conversation open.
2. faro asks for the type list (`.`): no prompt. If there's no `image/*` type: notice "No image on the clipboard".
3. faro asks for the best image type (`image/png`, then `image/jpeg`, `image/gif`, `image/webp`) with `pw`/`name=faro` (random password per run). Footer: "Waiting for kitty… (allow the clipboard prompt)".
4. Chunks are decoded as they arrive, with a size cap (proposed 25 MB decoded, like auto-download) and timeouts (proposed 60 s for the first packet while the user answers the prompt, 5 s between packets).
5. The image is written atomically as `<data dir>/attachments/clipboard-<YYYYMMDD-HHMMSS>.png` (`0600`) and staged like a pasted path: `📎 clipboard-….png · 285 KB  esc remove`.
6. Notices for `EPERM` ("Clipboard access denied in kitty"), `EBUSY`, timeout ("No answer from the terminal"), too large.

All wrapping uses `wrap_tmux_passthrough` when `$TMUX` is set.

## Implementation plan

Phases are sequential. Each ends with the quality gate (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`).

### Phase 0: spike, who reads the reply (decides the approach)

Try, in order, on a scratch branch:

1. **Pause crossterm:** drop the `EventStream`, make sure crossterm's reader is no longer polling stdin, read `/dev/tty` directly (`spawn_blocking` + `poll` with timeouts) until `DONE`/error/timeout, then create a new `EventStream`. Check what crossterm does with bytes it already buffered and with keys typed during the read (kitty's prompt takes focus, so few are expected; decide whether to drop or replay them).
2. If 1 is racy: **faro's own input reader**, a thread that owns the tty, splits out OSC 5522 replies and hands every other byte to a key parser. Bigger change: crossterm's parser is private, so it means another parser crate or our own.
3. Fallback idea for tmux only: run the request from a short-lived helper in `tmux display-popup` (its own pty), which writes the file and exits. Unverified: passthrough and reply routing for popups.

**Acceptance:** directly in kitty and inside tmux, a hand-triggered read returns the image bytes; no key events appear in the input during or after; typing works right after; repeated reads work. Record the outcome as a decision (amending D84's lifecycle rule).

### Phase 0 results (2026-10-09, D90)

- **Option 1 (pause crossterm) works.** Spike: `tools/clipboard-spike` on branch `spike/clipboard-osc5522`, a standalone crate. `fake_kitty.py` runs it in a pty, directly or behind a private tmux, and answers like kitty. The exact mechanism is in D90.
- **Fake kitty (Linux, tmux 3.6b):** a 300 KB PNG was identical on every read, directly and through tmux, with writes of 1–50, 1–3,000 and 1–65,536 bytes. No stray keys; typing works right after.
- **Owner (kitty → SSH → Mac mini → tmux, macOS):** a 285,083-byte PNG intact in 1.6 s; `EPERM` when denied; no prompt for the type list.
- **Found along the way:**
  - **Rebuilding the reply from key events is out:** crossterm wedges on bursts and input stops for good (D90).
  - **A late reply** (after faro stopped waiting) types garbage and then wedges input. It needs the guard in D90.
  - **tmux put kitty's focus-in report inside the image data.** The parser cuts `ESC [ … final` out of packets. Focus reports from inside or outside packets become focus events again.
  - **Several tmux clients:** every attached kitty answers and the first wins. In the owner's first run another kitty, with text on its clipboard, answered first.
- **Re-run:** `cd tools/clipboard-spike && cargo build && python3 fake_kitty.py target/debug/clipboard-spike <png> <scratch dir> [--tmux] [--focus-inside]`. In a real terminal: `./target/debug/clipboard-spike --log /tmp/spike.log --out /tmp`, then `1` (list types) and `2` (read the image). Never press `4` with an image: approach B wedges input.

### Phase 1: protocol module (pure, testable)

`faro-core::clipboard` (no terminal crates, per the architecture test):
- request builders (list, read with MIME list, `pw`/`name`), incremental reply parser (packets split across reads, `OK`/`DATA`/`DONE`/errors, base64 validation per chunk, size cap), MIME choice.
- Unit tests, including the exact bytes captured in test 4 and a synthetic multi-chunk PNG; malformed and truncated input never panics (D28).

### Phase 2: app integration

- `Action::PasteImage` (`Alt+V`, `input.paste_image`) in the keymap defaults, footer hint, palette command; README key table.
- Availability from `Terminal` (kitty, no mosh); explanatory notice otherwise.
- `Effect::ReadClipboardImage` → Phase 0 mechanism → `AppEvent::ClipboardImage(Result<StagedAttachment, ClipboardError>)`; reducer stages it or shows the notice. No I/O in the reducer (D22).
- File written through the existing `0600` atomic writer into `attachments/`.
- Snapshots for the waiting footer and the staged bar; reducer tests for each outcome.
- `docs/MANUAL_TESTS.md` section (rule 10): kitty direct, kitty → SSH, kitty → SSH → tmux; deny the prompt; no image on the clipboard; huge image; non-kitty terminal; mosh shows the notice.
- DESIGN.md (attachments section, clipboard row in the crates table) updated to match.

### Phase 3 (optional): paste with the normal paste key

Enable paste events (`CSI ? 5522 h`, wrapped for tmux) so Cmd+V with an image stages it with no prompt (single-use `pw`). Needs its own hand test first. Risks:
- The mode belongs to the kitty window, not the tmux pane. Other programs in other panes would get OSC 5522 instead of bracketed paste. faro would have to turn it off on focus loss (`focus-events`, D42), on exit and in the panic hook.
- Text pastes also arrive through 5522, so faro must request `text/plain` itself; bracketed paste (2004) is not used while the mode is on.
- Needs kitty ≥ 0.44.1 on the client; the mode can be queried with `CSI ? 5522 $ p`.

### Mechanical constraints to add (rule 9, D27)

- A test feeding a recorded 5522 reply through the input path asserts it produces **no** key events.
- Parser fuzz-style tests: random splits and corruption never panic and never exceed the size cap.
- Test that clipboard images are written `0600`.

## Open questions (for the owner)

- Key binding: `Alt+V`, or another?
- Size cap: 25 MB decoded, or smaller?
- Several tmux clients attached (each kitty may prompt or answer): offer the feature only with one client, or accept the first complete reply?
- Pursue Phase 3 at all, given the risk to other tmux panes?
- kitty version on the local machine (decides whether `pw`/`name` and paste events are available). Not recorded during the test.

## Out of scope

- mosh (impossible in-band; a side-channel upload helper was discussed and set aside).
- Terminals without OSC 5522 (the plan targets kitty; other terminals are unverified).
- Copying images *to* the clipboard.
