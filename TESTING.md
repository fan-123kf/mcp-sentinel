# 验证与测试

默认构建不需要模型或 Node.js。

```bash
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D clippy::all
cargo test --locked
cargo run --locked -- eval --output retrieval-eval.json
```

测试覆盖治理、三层规则扫描、HTTP 模拟调用链、12 后端聚合、有限 trace、TF-IDF、配置与 SQLite 历史恢复。模型运行测试默认跳过，需要完整模型环境。

评测口径见 docs/EVALUATION.md。CI 使用 Linux / Windows 双平台；本地验证脚本位于 scripts/verify.sh 和 scripts/verify.ps1。
