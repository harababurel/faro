#!/usr/bin/env bash
# Build getfaro.dev into <out dir> (called by site/deploy/update.sh).
#
# M13 Phase 1: `faro-site` fills site/template from faro's own code —
# version, date, key table, theme chips, the poster and the OG image
# (both rendered by the generator itself, §6.4) — and content-hashes the
# asset names (WEBSITE.md §6.1). Phase 4 adds the web demo: the wasm module
# is built first and `faro-site --demo` copies it into <out>/demo/ with
# hashed names plus the demo/manifest.json the page reads (§6.8). Text files
# get precompressed .gz twins for nginx's gzip_static, and .br twins when
# brotli is installed (nginx serves those only once it has the brotli
# module; see site/deploy/README.md).
set -euo pipefail

out=${1:?usage: site/build.sh <out dir>}
here=$(cd "$(dirname "$0")" && pwd)

# update.sh always passes a fresh dir; a manual run into an old one would
# leave stale hashed files behind.
if [[ -d $out && -n $(ls -A "$out") ]]; then
    echo "site/build.sh: output dir '$out' is not empty: use an empty one (stale hashed files would linger)" >&2
    exit 1
fi

# The web demo's wasm module (needs the wasm32 target and wasm-bindgen-cli:
# site/deploy/README.md step 5). Skip with SITE_NO_DEMO=1 to build only the
# page while working on it; the page doesn't load the demo yet, so nothing
# else changes.
pkg="$here/../crates/faro-web/www/pkg"
demo=()
if [[ ${SITE_NO_DEMO:-} != 1 ]]; then
    "$here/../crates/faro-web/build.sh"
    demo=(--demo "$pkg")
fi

(cd "$here/.." && cargo run --release --locked -p faro-site -- "$here/template" "$out" "${demo[@]+"${demo[@]}"}")

# Text files get a precompressed twin, except the demo's manifests: nginx's
# gzip_static would keep serving a stale .gz after someone edits one on disk
# (the emergency switch, §6.8).
precompress() {
    find "$out" -type f ! -name 'manifest*.json' \
        \( -name '*.html' -o -name '*.css' -o -name '*.js' -o -name '*.svg' -o -name '*.json' -o -name '*.txt' -o -name '*.wasm' \) \
        -exec "$@" {} +
}
precompress gzip -9 -k -n -f
if command -v brotli >/dev/null 2>&1; then
    precompress brotli -q 11 -k -f
fi
find "$out" -type d -exec chmod 755 {} +
find "$out" -type f -exec chmod 644 {} +
