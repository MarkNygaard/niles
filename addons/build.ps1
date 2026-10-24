# Build the add-ons named, or all of them, and put each component beside
# its manifest as integration.wasm: where `niles plugin check
# addons/<name>` looks for it, and the name a release gives it.
# Needs: rustup target add wasm32-wasip2
$ErrorActionPreference = "Stop"
Push-Location $PSScriptRoot
try {
    $names = $args
    if ($names.Count -eq 0) {
        $names = Get-ChildItem -Directory | Where-Object { Test-Path (Join-Path $_ "niles-integration.toml") } | ForEach-Object { $_.Name }
    }
    foreach ($name in $names) {
        cargo build --release --target wasm32-wasip2 -p $name
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        Copy-Item "target/wasm32-wasip2/release/$($name -replace '-', '_').wasm" "$name/integration.wasm"
        Write-Output "wrote addons/$name/integration.wasm"
    }
} finally {
    Pop-Location
}
