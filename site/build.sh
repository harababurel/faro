#!/usr/bin/env bash
# Build getfaro.dev into <out dir> (called by site/deploy/update.sh).
#
# M13 Phase 0: the placeholder page, copied as it is. Phase 1 replaces the body of
# this script with the site generator (WEBSITE.md §6.1); update.sh stays the same.
# Text files get precompressed .gz twins for nginx's gzip_static.
set -euo pipefail

out=${1:?usage: site/build.sh <out dir>}
here=$(cd "$(dirname "$0")" && pwd)

cp -R "$here/placeholder/." "$out/"
find "$out" -type f \( -name '*.html' -o -name '*.css' -o -name '*.js' -o -name '*.svg' -o -name '*.json' -o -name '*.txt' \) \
    -exec gzip -9 -k -n -f {} +
find "$out" -type d -exec chmod 755 {} +
find "$out" -type f -exec chmod 644 {} +
