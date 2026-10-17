# The README demo

`docs/images/faro-demo.gif` (the README's animation) and
`site/template/media/faro-tour.mp4` (the site's "Watch a 30-second tour", D143)
are recorded from the real
web demo (`crates/faro-web`), driven by a storyboard, on a virtual clock. The
same build and storyboard give the same frames on any machine, at any speed, so
the GIF can be regenerated whenever the UI, the demo's scenes or the storyboard
change.

```sh
CHROME=<chrome-headless-shell> tools/demo_gif/make.sh            # build, record, encode
CHROME=<chrome-headless-shell> tools/demo_gif/make.sh --install  # …and copy the GIF and the MP4 into place
```

Output is in `target/demo-gif/`: `faro-demo.gif` (1×, for the README),
`faro-demo.mp4` (2×, the site's tour and posts), `faro-demo.webm` (2×) and
`frames/` (the unique frames, `frames.ffconcat` with how long each stays on
screen, and any stills).

**Needs:** the web demo's build tools (see `crates/faro-web/build.sh`), Node 22+,
ffmpeg with libx264 and libvpx, and **chrome-headless-shell**
(`npx playwright install chromium-headless-shell` puts one in
`~/.cache/ms-playwright/`, which `make.sh` finds by itself). Full Chrome won't
do: it has no `HeadlessExperimental.beginFrame`.

## How it works

- `record.mjs` opens `crates/faro-web/www/dist/demo.html?record` in
  chrome-headless-shell (`--deterministic-mode`). `?record` starts the demo with
  `start_recording`: the visitor's mock, except everyone answers every message
  within seconds (`MockConfig::eager_answers`, D142). The website never uses it.
- Chrome runs on **virtual time**, starting at 2026-01-01 09:41 UTC in the UTC
  time zone. The recorder moves the clock on by exactly one frame (1/fps), asks
  Chrome to draw with `HeadlessExperimental.beginFrame`, and keeps the picture.
  The mock's timers, `Date.now`, `performance.now` and `requestAnimationFrame` all
  follow that clock, so the seeded live activity happens at the same moments on
  every run.
- Identical consecutive frames are merged (`frames.ffconcat` holds durations).
  `make.sh` encodes: a GIF at half size with one palette for the whole clip and
  no dithering, plus H.264 and VP9 at full size.

## The storyboard

`storyboard.jsonc` is a JSON list of steps with `//` line comments. Each step
can have any of these keys, applied in this order:

| Key | What it does |
|---|---|
| `"record": true` | start keeping frames (anything before it is setup and isn't in the clip); `false` stops |
| `"key": "j", "mods": ["alt"]` | press a key (`Enter`, `Escape`, `ArrowUp`, `F1`… or one character), with `alt`/`ctrl`/`shift`/`meta` |
| `"type": "text", "cps": 12` | type text, one key per character, at `cps` characters a second |
| `"until": "text"` | keep going until `text` is on screen (`"gone": true`: until it isn't); fails after `within` ms (default 15000) |
| `"expect": "text"` | fail unless `text` is on screen now |
| `"wait": 500` | let time run, in ms. Defaults to 300 after a `key` or `until`, 0 otherwise |
| `"still": "name"` | save the current frame as `frames/still-name.png` |
| `"dump": "label"` | print the screen as text |
| `"note": "…"` | a label for error messages and `--verbose` |

Screen checks read `demo.text()`, the screen as text. **Every beat should
check something** (`until`/`expect`): when a change to the scenes means a beat
no longer happens, the recording fails with the screen printed, instead of
quietly producing a different GIF.

## Working on it

- `node tools/demo_gif/record.mjs --dry --verbose` plays the storyboard without
  taking pictures (a few seconds) and prints when each step ends, in virtual
  time and in clip time.
- `--dpr 1` makes small frames for a quick look; add `still`s to see a moment.
- Other flags: `--cols`/`--rows` (default 120×34), `--font` (CSS px, default
  14), `--dpr` (default 2), `--fps` (default 12), `--story`, `--out`, `--html`.
- `make.sh` fails if the GIF is over 6 MB (GitHub stops animating very large
  GIFs).
