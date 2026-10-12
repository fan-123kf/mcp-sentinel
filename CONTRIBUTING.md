# Contributing

Use stable Rust and a native linker. The default build is model-free; Node.js is needed only for the filesystem backend demo.

Before submitting a change:

```bash
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D clippy::all
cargo test --locked
cargo run --locked -- eval --output retrieval-eval.json
```

For retrieval changes, compare per-query rankings on the fixed dataset and document regressions. Do not change labels to improve reported scores. Model results must use `--features models --require-models`; fallback results are a separate baseline.

For invocation changes, verify confirmation, retry boundaries, `isError`, security-blocked output, and trace creation. Include a regression test for observable behavior.

Keep tokens, local configuration, models, databases, and caches out of commits. CI checks default builds on Linux and Windows; it does not certify ONNX inference or every MCP server.
