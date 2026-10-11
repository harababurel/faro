#!/usr/bin/env bash
# Build faro's web demo into crates/faro-web/www/pkg/ (not committed):
#   crates/faro-web/build.sh            release (what the site serves)
#   PROFILE=dev crates/faro-web/build.sh
# Needs the wasm32 target (rust-toolchain.toml) and wasm-bindgen-cli at the
# same version as the wasm-bindgen crate in Cargo.lock. wasm-opt (binaryen) is
# used when it's installed.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
profile="${PROFILE:-release}"
dir="$profile"
[[ "$profile" == dev ]] && dir=debug

want="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$root/Cargo.lock")"
have="$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }')"
if [[ "$have" != "$want" ]]; then
    echo "wasm-bindgen-cli $want is needed (found: ${have:-none}):" >&2
    echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
    exit 1
fi

cargo build --manifest-path "$root/Cargo.toml" -p faro-web \
    --target wasm32-unknown-unknown --profile "$profile" --locked
wasm-bindgen --target web --no-typescript --out-dir "$here/www/pkg" \
    "$root/target/wasm32-unknown-unknown/$dir/faro_web.wasm"
if command -v wasm-opt >/dev/null; then
    wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
        -o "$here/www/pkg/faro_web_bg.wasm" "$here/www/pkg/faro_web_bg.wasm"
fi
wasm="$here/www/pkg/faro_web_bg.wasm"
echo "built $wasm: $(wc -c < "$wasm") bytes, $(gzip -9 < "$wasm" | wc -c) gzipped"
