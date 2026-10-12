# Architecture and decisions

MCP Sentinel is a single-instance Rust gateway. Axum serves HTTP JSON-RPC; Tokio manages backend I/O; SQLite persists tool registration and invocation history. The default retrieval path is model-free.

## Discovery

Backend tools receive stable `server::tool` identifiers. Metadata is scanned before indexing. The client sees six meta-tools and searches for concrete schemas on demand. The router maintains TF-IDF as a baseline and fallback; optional BGE-M3 dense/sparse rankings are fused by RRF and may be reranked by a Cross-Encoder.

Server filtering precedes truncation and reranking. Model inference runs in `spawn_blocking`. Search returns its actual pipeline with the results so traces and evaluations can distinguish model retrieval from fallback.

## Invocation

The gateway checks metadata and derives a policy from trusted backend annotations or conservative name heuristics. Unknown names require confirmation. Only reads may retry transient failures. A write cannot become retry-safe merely because a client asks for fallback.

Backend responses are scanned in full, including error content and structured output, before delivery. `isError` records a failed call. Scanning cannot roll back server-side effects; annotations and client confirmation are not authentication.

## State and lifetime

Health state is updated from invocation outcomes and persisted as call records. Registration and history restoration keep new tools discoverable and recover consecutive failures across restarts. Background tasks aggregate daily statistics and clean old records.

The trace store is bounded to 200 entries. It stores decision metadata without invocation arguments or output bodies; search queries may still contain sensitive information. Traces do not survive restart.

Stdio responses are matched to request IDs through oneshot channels. Failed writes and timeouts remove pending entries; stdout EOF closes all remaining waiters. Child processes terminate on backend drop. Stderr is inherited to avoid filling an unread pipe. Ctrl+C triggers graceful HTTP shutdown.

## Deliberate limits

- Single-instance SQLite, no distributed coordination.
- HTTP JSON responses; complete Streamable HTTP sessions and SSE remain future work.
- Loopback binding without inbound authentication; use trusted local clients and servers.
- Rule-based injection detection with false positives and false negatives.
- Model code exists, but full inference and retrieval quality require a provisioned environment.
- `web_ui` is reserved and does not enable a UI.

The next useful improvements are protocol conformance tests, inbound authentication, a larger labeled corpus, and reproducible model latency/quality measurements.
