$ErrorActionPreference = "Stop"
cargo build --release --target wasm32-wasip2
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Copy-Item "$env:CARGO_TARGET_DIR/wasm32-wasip2/release/niles_plugin_speech_stream_fixture.wasm" ../speech-stream.wasm
Write-Output "wrote $(Resolve-Path ../speech-stream.wasm)"