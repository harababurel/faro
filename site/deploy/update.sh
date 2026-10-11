#!/usr/bin/env bash
# Deploy getfaro.dev on the server (WEBSITE.md §6.8): pull, build into
# releases/<commit>/, switch `current` in one atomic step, keep the last five.
# nginx serves $root/current, so visitors never see a half-built site, rollback is
# one symlink change, and nginx needs no reload.
#
#   /srv/getfaro.dev/repo/site/deploy/update.sh
#   FARO_SITE_ROOT=/elsewhere site/deploy/update.sh     # another layout
#
# Rollback: ln -sfn releases/<older commit> /srv/getfaro.dev/.current-new &&
#           mv -T /srv/getfaro.dev/.current-new /srv/getfaro.dev/current
set -euo pipefail

root=${FARO_SITE_ROOT:-/srv/getfaro.dev}
keep=5

cd "$root/repo"
git pull --ff-only
commit=$(git rev-parse --short=12 HEAD)
release=$root/releases/$commit
mkdir -p "$root/releases"

if [[ -d $release ]]; then
    echo "Release $commit already built."
else
    tmp=$root/releases/.building-$commit-$$
    trap 'rm -rf -- "$tmp"' EXIT
    mkdir -p "$tmp"
    site/build.sh "$tmp"
    printf '%s\n' "$commit" > "$tmp/commit.txt"
    mv -T "$tmp" "$release"
    trap - EXIT
fi

# Atomic switch: a new symlink renamed over the old one.
ln -sfn "releases/$commit" "$root/.current-new"
mv -T "$root/.current-new" "$root/current"

# Keep the newest $keep releases (never the one being served).
cd "$root/releases"
ls -1dt -- */ 2>/dev/null | sed 's#/$##' | tail -n +$((keep + 1)) | while read -r old; do
    [[ $old == "$commit" ]] || rm -rf -- "$old"
done

echo "getfaro.dev now serves $commit."
