#!/bin/sh
# Rebuild the test plugin and copy it where the tests read it.
# Needs: rustup target add wasm32-wasip2
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/niles_plugin_hello.wasm ../hello.wasm
echo "wrote $(cd .. && pwd)/hello.wasm"
