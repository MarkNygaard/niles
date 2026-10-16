# Rebuild the test plugin and copy it where the tests read it.
# Needs: rustup target add wasm32-wasip2
$ErrorActionPreference = "Stop"
Push-Location $PSScriptRoot
try {
    cargo build --release --target wasm32-wasip2
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Copy-Item target/wasm32-wasip2/release/niles_plugin_sleepmat.wasm ../sleepmat.wasm
    Write-Output "wrote $(Resolve-Path ../sleepmat.wasm)"
} finally {
    Pop-Location
}
