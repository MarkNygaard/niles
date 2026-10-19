#!/bin/sh
set -eu
cd "$(dirname "$0")"
cargo +1.99.0 build --locked --release --target wasm32-wasip2
cp "${CARGO_TARGET_DIR:-target}/wasm32-wasip2/release/niles_plugin_notify_06_fixture.wasm" ../notify-0.6.wasm
printf 'wrote %s\n' "$(cd .. && pwd)/notify-0.6.wasm"
