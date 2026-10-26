$ErrorActionPreference = "Stop"
cargo build --release --target wasm32-wasip2
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Copy-Item "$env:CARGO_TARGET_DIR/wasm32-wasip2/release/niles_plugin_speech_stream_06_fixture.wasm" ../speech-stream-0.6.wasm
Write-Output "wrote $(Resolve-Path ../speech-stream-0.6.wasm)"