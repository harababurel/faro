# Background and research

Context gathered before implementation started (2026-10-07). Useful for understanding *why* faro looks the way it does. Decisions themselves live in [DECISIONS.md](DECISIONS.md).

## 1. The owner's setup and use case

- Uses **Signal** every day.
- Runs a terminal Signal client inside a long-lived **tmux** session on a personal **Mac mini**, and attaches to it over **SSH** from other machines.
- Has multiple other machines (Arch Linux, Debian, others) and may run faro on them too. faro is expected to become an **open-source** project used by others.
- Terminal: modern emulators such as **kitty**; runs almost everything inside **tmux**.
- UI taste: likes **btop** (glyph-based graphs, purposeful colour, nothing is plain white-on-black) and **OpenCode** (command palette, input box as a distinct panel, sidebars, many themes). OpenCode is their favourite coding harness.

## 2. Problems with gurk (the current client)

gurk ([boxdot/gurk-rs](https://github.com/boxdot/gurk-rs)) is a Rust + ratatui client built on presage. As of 2026-10:

| Problem | Detail |
|---|---|
| Opaque keybindings | Bindings are in the README; `F1` opens a help panel but it's an unformatted, non-interactive wall of text. |
| Channel picker adds nothing | `Ctrl+P` opens a popup that just duplicates the sidebar (arrow keys already navigate it). |
| Cluttered conversation list | Auto-populated with every contact ever chatted with, including old/empty ones; no way to hide or remove them. `Alt+M` mutes but the entry stays. |
| Input freezes | Typing freezes while an incoming message from another contact is being fetched. Likely gurk's event loop, not presage. |
| No settings UI | Config file exists (`gurk.toml`, keybindings remappable) but nothing is configurable in-app; defaults aren't great. |

Good things to keep in mind from gurk: configurable keybindings, `file://` upload syntax, clipboard copy, mute.

## 3. Alternatives that were considered

| Option | Verdict |
|---|---|
| **scli** (Python, urwid, on signal-cli) | Lightweight, worth knowing about; doesn't address the UX goals. |
| **signal-cli + weechat** (signal-weechat plugin) | Fine for IRC-style users; not the experience wanted. |
| **mautrix-signal bridge + Matrix TUI** (iamb, gomuks) | Most setup (Matrix homeserver + bridge on the Mac mini), but well-maintained clients. Rejected as too heavy and indirect. |
| **siggo** (Go) | Long inactive. |
| Flare, Whisperfish | GUI (GTK / Sailfish), not terminal. |
| Phone only | Possible fallback; not what the owner wants. |
| Web app | Not usable over SSH/tmux; would expose a Signal session over the web. |

## 4. Signal client architecture primer

| Layer | What it does | Who provides it |
|---|---|---|
| Crypto / protocol | Double Ratchet, PQXDH, sealed sender, group credentials | **libsignal** (official, Rust, AGPL) |
| Service layer | Linking, server websocket, envelopes, contacts/groups sync, attachments | Official apps each have their own; unofficial: `libsignal-service-java` (signal-cli), `libsignal-service-rs` (presage) |
| Client | Storage, account state, UI | signal-cli, presage + gurk/Flare, Whisperfish, faro |

- **signal-cli:** Java; most mature unofficial client; `daemon` mode keeps the server connection open and exposes JSON-RPC over a socket (or `jsonrpc` over stdio). Doesn't keep queryable history. Needs a recent Java; packaging varies by distro; GPLv3.
- **presage:** Rust library; single binary; includes a store (keys, sessions, messages); not on crates.io; builds need `protoc`; API changes between versions; smaller maintainer team; AGPL.
- Both are always **linked (secondary) devices**.

## 5. Terminal-environment findings

- **Mouse capture** disables native text selection → provide built-in copy (OSC 52, works over SSH/tmux), `Shift`+drag bypass, and a toggle.
- **tmux** forwards right-click to the app when the app has mouse mode on (`set -g mouse on`).
- **Emoji widths** (ZWJ sequences, skin tones, flags, VS16) disagree between terminals, tmux and Rust width tables → misaligned borders. Mitigation: grapheme-aware width + optional fallback rendering.
- **kitty keyboard protocol** is only partially supported by tmux → defaults must not depend on it (e.g. `Alt+Enter` for newline, not only `Shift+Enter`).
- **kitty graphics protocol** (inline images) is awkward under tmux → deferred.
- **Desktop notifications over SSH** are possible via OSC 9 / OSC 99 (kitty) / OSC 777; under tmux they need `allow-passthrough on`. The terminal bell always works.
- **Truecolor** under tmux needs the `RGB` terminal feature; fall back to 256 colours.

## 6. Naming history

Criteria that emerged: easy to pronounce, not pretentious, not an existing well-known product, no "Signal" (trademark), free on major package managers, and a name that supports a good ASCII/glyph logo (all candidates were prototyped in `tools/logos.py`).

| Name | Outcome |
|---|---|
| **faro** | **Chosen.** Lighthouse logo was the owner's favourite. |
| pharos | Liked (favourite logo) but sounded a bit pretentious; faro is the same idea, plainer. |
| keeper, lightkeep | Lighthouse alternatives; keeper collides with Keeper Security. |
| aldis | Too close to ALDI (and invites "aldis nuts" jokes). |
| blip | Owner not a fan. |
| tern | Homebrew name taken. |
| helio | A bit pretentious/generic. |
| lumen | Simplistic logo; AUR package exists; Laravel Lumen. |
| morse | Logo liked, name generic and taken on Homebrew/Debian. Lamp alternatives: qsl, didah, dit, blinker (Debian taken). |
| cairn | Dull grey; pronunciation unclear. |
| glint | Logo looked like the Gemini logo. |
| wisp | Felt like a video game. |
| sotto, tightbeam | No strong feelings either way. |
| signet | Name liked a lot; several logo variants built (wax seal, ring stamping, envelope, Dune ring, sandworm, black wax, spinning ring, line art). Dune variants use Dune IP (fan nod only). Not chosen. |
| starling | Sounds like Starlink; also a UK bank. Murmuration logo liked. Flock alternatives: skein, murmur, roost. |
| hush | Exists as a product; owner disliked the sound. |
| coo | Whimsical, but reads as "COO". |
| tincan, porch, nook, cove | Late batch; not chosen. |
