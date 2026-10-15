# Rebuild the test plugin and copy it where the tests read it.
# Needs: rustup target add wasm32-wasip2
$ErrorActionPreference = "Stop"
Push-Location $PSScriptRoot
try {
    cargo build --release --target wasm32-wasip2
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Copy-Item target/wasm32-wasip2/release/niles_plugin_printer.wasm ../printer.wasm
    Write-Output "wrote $(Resolve-Path ../printer.wasm)"
} finally {
    Pop-Location
}
