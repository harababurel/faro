#!/usr/bin/env bash
# Build getfaro.dev into <out dir> (called by site/deploy/update.sh).
#
# M13 Phase 1: `faro-site` fills site/template from faro's own code —
# version, date, key table, theme chips, the poster (rendered by the
# generator itself, §6.4) and the OG image from site/generated/ when it
# exists — and content-hashes the asset names (WEBSITE.md §6.1). Text files
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

(cd "$here/.." && cargo run --release --locked -p faro-site -- "$here/template" "$out")

find "$out" -type f \( -name '*.html' -o -name '*.css' -o -name '*.js' -o -name '*.svg' -o -name '*.json' -o -name '*.txt' \) \
    -exec gzip -9 -k -n -f {} +
if command -v brotli >/dev/null 2>&1; then
    find "$out" -type f \( -name '*.html' -o -name '*.css' -o -name '*.js' -o -name '*.svg' -o -name '*.json' -o -name '*.txt' \) \
        -exec brotli -q 11 -k -f {} +
fi
find "$out" -type d -exec chmod 755 {} +
find "$out" -type f -exec chmod 644 {} +
