# Reproducible evaluation

Run from the repository root:

```bash
cargo run --locked -- eval --output retrieval-eval.json
python -m pip install -r scripts/requirements-eval.txt
python scripts/token_account.py retrieval-eval.tools.json retrieval-eval.meta.json --output token-account.json
```

The corpus is `docs/eval/data/backend_tools.json`: GitHub 26, Filesystem 14, Everything 13. The default query file is `eval/retrieval-19.jsonl`, selected in existing query order without changing labels to fit results. The CLI validates that every labeled tool exists.

Recall@5 averages the fraction of relevant tools returned in the first five results. MRR@20 averages the reciprocal rank of the first relevant result within twenty. Top-1 measures first-result hits. Queries without relevant labels are excluded from these denominators. The current 19-query set has one labeled answer per query, so Hit@5 equals Recall@5 for this dataset only.

The model-free baseline is Recall@5 0.6316, MRR@20 0.491, Top-1 0.4211. The report includes every returned ranking and its actual pipeline. This small corpus is a regression fixture, not a general benchmark.

Token accounting uses compact JSON tool definitions and an explicit tokenizer (`cl100k_base`). The script records tokenizer version and input hashes. The current definitions measure 6,950 backend tokens and 521 meta-tool tokens, a 92.50% reduction. This excludes search responses, calls, message wrappers and full conversations. It is not a claim about total conversation cost.

For model measurements:

```bash
cargo run --locked --features models -- eval --require-models --output retrieval-model.json
```

Provision BGE-M3, the reranker and a compatible ONNX environment first. Strict mode rejects missing models and fallback during evaluation. Type checking with native linking disabled is not evidence of inference working. No full model accuracy or latency is currently claimed.

The 12-backend/60-tool test uses local mock HTTP servers. It demonstrates aggregation, schema discovery, invocation and tracing rather than compatibility with twelve external products. Earlier experiment notes remain under `docs/eval/`; current reproducible results take precedence over historical reports.
