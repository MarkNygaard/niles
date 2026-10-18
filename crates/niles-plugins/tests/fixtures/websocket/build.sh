#!/bin/sh
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/niles_plugin_websocket_fixture.wasm ../websocket.wasm
