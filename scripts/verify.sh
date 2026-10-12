#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D clippy::all
cargo test --locked
cargo run --locked -- eval --output retrieval-eval.json
