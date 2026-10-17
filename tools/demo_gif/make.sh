#!/usr/bin/env bash
# Make the README demo: build the web demo, record the storyboard on a virtual
# clock, and encode it.
#
#   CHROME=<chrome-headless-shell> tools/demo_gif/make.sh [--no-build] [--install] [record.mjs flags]
#
#   --no-build   reuse crates/faro-web/www/dist/demo.html as it is
#   --install    copy the GIF to docs/images/faro-demo.gif (the README) and the
#                MP4 to site/template/media/faro-tour.mp4 (the site's tour, D143)
#
# Needs: the web demo's build tools (crates/faro-web/build.sh), Node 22+,
# ffmpeg, and chrome-headless-shell (`npx playwright install chromium-headless-shell`
# puts one in ~/.cache/ms-playwright/): full Chrome has no
# HeadlessExperimental.beginFrame. Output in target/demo-gif/:
#   faro-demo.gif   1×, for the README
#   faro-demo.mp4   2×, H.264 (the site's tour, social posts)
#   faro-demo.webm  2×, VP9
#   frames/         the recorded frames and stills
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
build=1
install=0
pass=()
for arg in "$@"; do
    case "$arg" in
        --no-build) build=0 ;;
        --install) install=1 ;;
        *) pass+=("$arg") ;;
    esac
done
out=target/demo-gif
# GitHub shows README images at most ~1000 px wide and stops animating very
# large GIFs; keep well under its 10 MB limit.
max_gif_bytes=$((6 * 1024 * 1024))

if [[ -z "${CHROME:-}" ]]; then
    CHROME="$(ls ~/.cache/ms-playwright/chromium_headless_shell-*/*/chrome-headless-shell 2>/dev/null | tail -1 || true)"
    [[ -n "$CHROME" ]] || { echo "set CHROME to a chrome-headless-shell binary" >&2; exit 1; }
    export CHROME
fi

if [[ $build == 1 ]]; then
    crates/faro-web/build.sh
    python3 crates/faro-web/tools/bundle.py
fi

node tools/demo_gif/record.mjs --out "$out/frames" "${pass[@]}"

concat="$out/frames/frames.ffconcat"
ff=(ffmpeg -loglevel error -y -f concat -safe 0 -i "$concat")

# GIF: half size (the frames are 2×), one palette for the whole clip built
# from what changes, no dithering (flat terminal colours), and only the
# changed rectangle stored per frame.
"${ff[@]}" -vf "fps=12,scale=iw/2:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff:max_colors=256[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
    -loop 0 "$out/faro-demo.gif"
# Video: 2×, constant frame rate, playable everywhere (yuv420p).
"${ff[@]}" -vf "fps=30" -c:v libx264 -preset slow -tune animation -crf 18 \
    -pix_fmt yuv420p -movflags +faststart "$out/faro-demo.mp4"
"${ff[@]}" -vf "fps=30" -c:v libvpx-vp9 -crf 30 -b:v 0 -pix_fmt yuv420p \
    -row-mt 1 "$out/faro-demo.webm"

size() { stat -c %s "$1" 2>/dev/null || stat -f %z "$1"; }
for f in "$out"/faro-demo.{gif,mp4,webm}; do
    printf '%-28s %6s KB\n' "$f" $(( $(size "$f") / 1024 ))
done
if (( $(size "$out/faro-demo.gif") > max_gif_bytes )); then
    echo "the GIF is over $((max_gif_bytes / 1024 / 1024)) MB: shorten the storyboard or lower --fps" >&2
    exit 1
fi
if [[ $install == 1 ]]; then
    cp "$out/faro-demo.gif" docs/images/faro-demo.gif
    cp "$out/faro-demo.mp4" site/template/media/faro-tour.mp4
    echo "installed docs/images/faro-demo.gif and site/template/media/faro-tour.mp4"
fi
