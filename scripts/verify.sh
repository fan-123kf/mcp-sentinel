#!/bin/bash
# Verification script to ensure mcp-sentinel can build and run

set -e

echo "🔍 Step 1: Checking Rust installation..."
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust not found. Please install from https://rustup.rs/"
    exit 1
fi
echo "✅ Rust $(rustc --version)"

echo ""
echo "🔍 Step 2: Checking Node.js installation..."
if ! command -v node &> /dev/null; then
    echo "❌ Node.js not found. Please install from https://nodejs.org/"
    exit 1
fi
echo "✅ Node.js $(node --version)"

echo ""
echo "🔨 Step 3: Running cargo check..."
cargo check
echo "✅ Code compiles successfully"

echo ""
echo "🔨 Step 4: Running cargo build..."
cargo build --release
echo "✅ Release build successful"

echo ""
echo "🧪 Step 5: Running tests..."
cargo test
echo "✅ Tests passed"

echo ""
echo "✅ All verification steps passed!"
echo ""
echo "To start the gateway:"
echo "  1. Copy sentinel.toml.example to sentinel.toml"
echo "  2. Edit sentinel.toml with your backend configurations"
echo "  3. Run: cargo run --release -- start"
