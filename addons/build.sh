#!/bin/sh
# Build the add-ons named, or all of them, and put each component beside
# its manifest as integration.wasm: where `niles plugin check
# addons/<name>` looks for it, and the name a release gives it.
# Needs: rustup target add wasm32-wasip2
set -eu
cd "$(dirname "$0")"
if [ "$#" -eq 0 ]; then
    set -- $(for manifest in */niles-integration.toml; do dirname "$manifest"; done)
fi
for name in "$@"; do
    cargo build --release --target wasm32-wasip2 -p "$name"
    cp "target/wasm32-wasip2/release/$(echo "$name" | tr - _).wasm" "$name/integration.wasm"
    echo "wrote addons/$name/integration.wasm"
done
