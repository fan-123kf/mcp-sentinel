# Verify the default, model-free build. Node.js is only needed for backend demos.
$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo fmt -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed' }
    cargo clippy --locked --all-targets -- -D clippy::all
    if ($LASTEXITCODE -ne 0) { throw 'Clippy check failed' }
    cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
    cargo run --locked -- eval --output retrieval-eval.json
    if ($LASTEXITCODE -ne 0) { throw 'Evaluation failed' }
    Write-Host 'All default-build checks passed.'
} finally {
    Pop-Location
}
