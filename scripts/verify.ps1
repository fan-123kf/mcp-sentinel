# PowerShell verification script for Windows users

Write-Host "🔍 Step 1: Checking Rust installation..." -ForegroundColor Cyan
try {
    $rustVersion = cargo --version
    Write-Host "✅ $rustVersion" -ForegroundColor Green
} catch {
    Write-Host "❌ Rust not found. Please install from https://rustup.rs/" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "🔍 Step 2: Checking Node.js installation..." -ForegroundColor Cyan
try {
    $nodeVersion = node --version
    Write-Host "✅ Node.js $nodeVersion" -ForegroundColor Green
} catch {
    Write-Host "❌ Node.js not found. Please install from https://nodejs.org/" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "🔨 Step 3: Running cargo check..." -ForegroundColor Cyan
cargo check
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Code compiles successfully" -ForegroundColor Green
} else {
    Write-Host "❌ Compilation failed" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "🔨 Step 4: Running cargo build..." -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Release build successful" -ForegroundColor Green
} else {
    Write-Host "❌ Build failed" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "🧪 Step 5: Running tests..." -ForegroundColor Cyan
cargo test
if ($LASTEXITCODE -eq 0) {
    Write-Host "✅ Tests passed" -ForegroundColor Green
} else {
    Write-Host "❌ Tests failed" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "✅ All verification steps passed!" -ForegroundColor Green
Write-Host ""
Write-Host "To start the gateway:" -ForegroundColor Yellow
Write-Host "  1. Copy sentinel.toml.example to sentinel.toml"
Write-Host "  2. Edit sentinel.toml with your backend configurations"
Write-Host "  3. Run: cargo run --release -- start"
