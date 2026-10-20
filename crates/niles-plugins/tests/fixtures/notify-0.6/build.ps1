$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
cargo +1.99.0 build --locked --release --target wasm32-wasip2
Copy-Item "$env:CARGO_TARGET_DIR/wasm32-wasip2/release/niles_plugin_notify_06_fixture.wasm" ../notify-0.6.wasm
Write-Output "wrote $(Resolve-Path ../notify-0.6.wasm)"
