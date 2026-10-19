#!/bin/sh
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-wasip2
cp "${CARGO_TARGET_DIR:-target}/wasm32-wasip2/release/niles_plugin_speech_stream_fixture.wasm" ../speech-stream.wasm
echo "wrote $(cd .. && pwd)/speech-stream.wasm"