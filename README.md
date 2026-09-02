# mcp-sentinel

**Intelligent MCP Gateway with Health-Driven Adaptive Routing, Governance & Decision Tracing**

[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![MCP](https://img.shields.io/badge/MCP-2024--11--05-blue.svg)](https://modelcontextprotocol.io/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## 项目一句话定位

mcp-sentinel 是一个面向 **AI Agent / LLM 客户端** 的 **Model Context Protocol (MCP) 智能网关**。

当 Agent 同时接入 GitHub、Linear、Slack、Filesystem、Database 等 10+ 个 MCP server（合计 50+ 工具）时，会出现三个真实痛点：

1. **Context 浪费**：每个工具定义占用大量 token，未使用的工具白白占用 prompt
2. **选择质量下降**：Agent 要从 50 个工具中挑对的那一个，缺乏"哪个能用、哪个坏了"的信号
3. **失控的副作用**：写/删除类工具与只读工具混在一起，难以做风险分级和审计

mcp-sentinel 把 **5 个元工具** 暴露给 Agent，把 **后端 50+ 工具** 收进来自己管，配套 **健康驱动路由 + 工具治理（governance）+ 决策追踪（decision trace）**，让 Agent 只看到"该看到的工具"、只做"被允许的操作"、每一步都可追溯。

---

## 核心创新点（区别于同类 MCP 网关）

| 维度 | 传统 MCP 客户端 | mcp-gateway / MCPHub | **mcp-sentinel** |
|------|---------------|---------------------|-------------------|
| 工具列表 | 静态全量塞给 LLM | 元工具聚合 | ✅ **元工具 + 健康感知路由** |
| 健康监控 | 无 | Circuit breaker | ✅ **健康分直接驱动排序** |
| 自动降级 | 无 | 监控 + 告警 | ✅ **僵尸工具自动剔除，连续失败自动降权** |
| 副作用治理 | 无 | 无 | ✅ **Read/Write/Destructive 三级策略 + 显式确认** |
| 自动重试 | 无 | 无 | ✅ **按错误类别识别 transient 错误并退避重试** |
| 决策可观测 | 无 | 日志 | ✅ **决策追踪（trace_id）+ 隐私保护元数据** |
| 持久化 | 无 | PostgreSQL | ✅ **SQLite 零配置 + 自动聚合 + 自动清理** |
| 报告生成 | 无 | Web UI | ✅ **CLI 一键 Markdown 健康报告 + 剪枝后的新 toml** |
| 错误分类 | 无 | 无 | ✅ **7 类错误自动分类（Validation/Auth/Permission/Rate/Timeout/Unavail/Execution）** |

**一句话差异化**：把"健康度"从监控指标升级为**路由决策的一等公民**，并补齐 LLM Agent 真正需要的 **governance + observability**。

---

## 核心能力详解

### 1) 元工具抽象（Token 优化）

Agent 看到的不是 50 个具体工具，而是 **5 个稳定的元工具**：

- `gateway_search_tools` — 自然语言检索，按健康分排序返回 top-K
- `gateway_invoke` — 统一调用入口，含治理（governance）授权检查与重试
- `gateway_health_report` — 一键生成 Markdown 健康报告
- `gateway_get_trace` — 通过 `trace_id` 拉取决策追踪
- `gateway_suggest_cleanup` — 推荐要剪掉的僵尸 server / 工具，并预估可节省的 token

实际效果：每次对话从 ~50 个工具定义（约 8K tokens）缩减到 5 个元工具（约 500 tokens），**节省约 90%+ 的工具列表开销**。

### 2) 健康驱动混合路由（Health-Driven Hybrid Routing）

`SemanticRouter` 在每次检索时：

1. **BGE-M3 稀疏检索**（ Learned Lexical）：
   - BGE-M3 一次前向同时输出 token indices + weights 的稀疏向量
   - 替换：TF-IDF + 同义词扩展（自动覆盖中英跨语言意图）
2. **BGE-M3 稠密检索**（Semantic）：
   - 1024 维 L2 归一化向量，余弦相似度
   - 一次前向同时完成，无需独立 embedding 模型
3. **Reciprocal Rank Fusion (RRF, k=60)**：融合两路排名（无需可比分数尺度）
4. **Cross-Encoder 重排**（可选）：
   - bge-reranker-v2-m3 对 top-20 候选精排
   - 端到端训练模型，显著优于特征规则
5. **健康感知重排**：

```text
final_score = semantic_score × (1 - w + w × health_penalty)

其中：
  semantic_score  = RRF 融合后的相关性分
  health_penalty  = success_rate × (1 / (1 + p95_latency_ms / 2000)) × staleness
  w               = health_weight（默认 0.4）

 僵尸工具（7 天无调用） → final_score = 0（直接排除）
 降级工具（连续失败 ≥ 5）→ health_penalty = 0.1（重度惩罚）
```

> 实测召回质量用 `eval/tool-retrieval.jsonl` 评测（Recall@K、MRR、延迟），以本地跑出来的数字为准。

### 3) 健康追踪 + 持久化（SQLite 三表 + 后台任务）

每次调用后，`HealthManager` 同时更新内存状态与 SQLite：

- **内存**：实时维护 `ToolHealth { success_count, failure_count, latency_p95, consecutive_failures, last_call, call_count_7d, zombie_score, health_score }`
- **持久化**：写入 `tool_calls` 表，后台任务每小时聚合一次 `daily_stats`，按 `retention_days` 自动清理

由此可以精确计算 **p95 延迟、7 日调用频次、连续失败次数**，给路由层提供真实数据。

**三张表**：

- `tool_calls` — 每次调用明细（可按时间窗口聚合）
- `tool_registry` — 工具元数据（tool_id、server_name、schema_json）
- `daily_stats` — 每日聚合（call_count / success_count / avg_latency / p95_latency）

### 4) 治理（Governance）：副作用分级 + 自动重试

`governance.rs` 用工具名启发式推断策略（不依赖外部标注）：

| 工具名包含 | side_effect | confirmation_required | retry_safe | max_attempts |
|-----------|-------------|------------------------|------------|--------------|
| delete / remove / destroy / drop / purge | **Destructive** | ✅ | ❌ | 1 |
| create / update / write / send / post / put / set / merge | **Write** | ✅ | ❌ | 1 |
| 其他（默认） | **Read** | ❌ | ✅ | 2 |

调用时：

- 写/删类工具必须显式传 `confirmed: true` 才放行
- 只读工具遇到 **transient 错误**（RateLimited / Timeout / Unavailable）会自动退避重试（指数回退 100ms × attempts）
- 每次调用的 side-effect 等级、是否确认、尝试次数、结果、错误类别、延迟都会写入 **DecisionTrace**

### 5) 错误分类（7 类）

`classify_error` 对错误文本做关键词匹配，自动归类：

- `Validation`（invalid / validation / schema）
- `Authentication`（unauthorized / authentication / token）
- `Permission`（forbidden / permission）
- `RateLimited`（429 / rate limit）
- `Timeout`（timeout / timed out）
- `Unavailable`（unavailable / connection / 503）
- `Execution`（兜底）

> 这是后续做自动 fallback、告警分流的依据。

### 6) 决策追踪（Decision Trace）

`DecisionTraceStore` 用环形缓冲（200 条）记录最近决策，包含：

- `SearchTrace { query, candidate_count, selected_tools, strategy }`
- `InvocationTrace { tool_id, side_effect, confirmation_required, confirmed, attempts, outcome, error_category, latency_ms }`

**隐私保护**：trace 故意**不**记录 `arguments` 与结果内容，只记录元数据，方便事后审计但不泄漏用户数据。`gateway_get_trace(trace_id)` 即可拉取。

### 7) 诊断 + 一键剪枝（CLI）

两条命令专为"清理门户"设计：

- `mcp-sentinel report` — 输出 Markdown 报告：Summary、Top 10 Most-Used、Zombie 工具、Degraded 工具、可执行 Recommendations
- `mcp-sentinel gen-config --aggressive` — 基于使用数据自动生成一份 **剪枝后的 sentinel.toml**，把僵尸 server 整段移除，并估算可节省的 token

---

## 技术栈与代码规模

| 项目 | 详情 |
|------|------|
| 语言 | Rust（edition 2021，最低 1.75） |
| 异步运行时 | `tokio 1.40` (full) |
| Web 框架 | `axum 0.7` + `tower-http` (CORS / Trace) |
| 序列化 | `serde` + `serde_json` + `toml` |
| 配置展开 | `shellexpand`（支持 `${ENV}` 与 `~`） |
| 数据库 | `rusqlite 0.31` (bundled) + `tokio-rusqlite 0.5` |
| 进程管理 | `async-process 2.3`（stdio MCP 子进程） |
| HTTP 客户端 | `reqwest 0.12`（HTTP MCP 后端） |
| CLI | `clap 4.5` (derive) |
| 分词 | `unicode-segmentation` |
| 代码量 | **~2,900 行 Rust**（src + tests，含注释） |

---

## 快速开始

### 前置

- Rust 1.75+（[rustup.rs](https://rustup.rs/)）
- Node.js 18+（用 `npx` 跑 stdio MCP server，例如 filesystem、github）

### 30 秒跑起来

```bash
git clone https://github.com/yourusername/mcp-sentinel.git
cd mcp-sentinel

# 最小配置（不需要任何 API key）
cp examples/minimal.toml sentinel.toml
cargo run --release -- start
```

启动后监听 `http://localhost:3000/mcp`，并立即生效。

### 在 Claude Desktop / Cursor 里连接

```json
{
  "mcpServers": {
    "sentinel": {
      "url": "http://localhost:3000/mcp"
    }
  }
}
```

之后 Agent 只会看到 5 个元工具，后端的 GitHub / Linear / Filesystem … 对它透明。

### 完整 `sentinel.toml` 示例

```toml
[gateway]
port = 3000
web_ui = true
log_level = "info"

[routing]
strategy = "tfidf"
top_k = 5
health_weight = 0.4

[health]
zombie_threshold_days = 7
consecutive_failure_limit = 5
degraded_score_penalty = 0.1

[storage]
db_path = "~/.config/mcp-sentinel/sentinel.db"
retention_days = 30

[backends.github]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env = { GITHUB_TOKEN = "${GITHUB_TOKEN}" }

[backends.filesystem]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[backends.linear]
transport = "http"
url = "http://localhost:4000/mcp"
auth = { type = "bearer", token = "${LINEAR_TOKEN}" }
```

---

## 五个元工具

### 1. `gateway_search_tools` — 自然语言检索

输入：
```json
{ "query": "create a GitHub issue", "top_k": 5, "server_filter": "github" }
```

返回（示例）：
```json
{
  "query": "create a GitHub issue",
  "results": [
    {
      "tool_id": "github::create_issue",
      "name": "create_issue",
      "server": "github",
      "description": "Create a new issue in a repository",
      "scores": { "semantic": "0.892", "health": "0.956", "final": "0.847" },
      "health_hint": "healthy",
      "degraded": false
    }
  ],
  "count": 1,
  "trace_id": "trc-20260829174512345678"
}
```

### 2. `gateway_invoke` — 调用 + 治理 + 重试

输入：
```json
{
  "tool_id": "github::create_issue",
  "arguments": { "repo": "owner/repo", "title": "Bug", "body": "..." },
  "confirmed": true,
  "allow_fallback": true
}
```

返回（成功）：
```json
{ "status": "success", "result": { ... }, "latency_ms": 312, "attempts": 1, "trace_id": "..." }
```

返回（失败）：
```json
{
  "status": "error",
  "error": "MCP error: ...",
  "error_category": "RateLimited",
  "latency_ms": 1204,
  "attempts": 2,
  "trace_id": "...",
  "fallback_allowed": false
}
```

> 写/删类工具若 `confirmed=false` 会直接被 `policy.authorize()` 拒绝。

### 3. `gateway_health_report` — Markdown 健康报告

输入：`{ "scope": "all", "time_window_days": 7 }`，输出整段 Markdown，含 Summary / Top 10 Most-Used / Zombie / Degraded / Recommendations。

### 4. `gateway_get_trace` — 决策追踪

输入：`{ "trace_id": "trc-..." }`，返回该次搜索或调用的元数据（**不含参数与结果**，避免泄漏）。

### 5. `gateway_suggest_cleanup` — 清理建议

输入：`{ "aggressive": false }`（true 时把 7 天缩短为 3 天），返回 JSON：

```json
{
  "zombie_tools": [{ "tool_id": "...", "reason": "No calls in 7+ days", "estimated_token_waste": 150 }],
  "zombie_servers": [{ "server": "obsidian", "zombie_count": 23, "reason": "23 tools unused for 7+ days, wasting ~3450 tokens/turn" }],
  "degraded_tools": [{ "tool_id": "linear::update_issue", "health_score": "0.123", "recommendation": "Check server connectivity and auth tokens" }],
  "estimated_token_savings": 3450,
  "recommendations": ["Remove 23 zombie tools to reduce context bloat", "Investigate 1 degraded tools for connectivity issues"]
}
```

---

## CLI 命令一览

| 命令 | 用途 |
|------|------|
| `mcp-sentinel start` | 启动网关（Axum HTTP server，`/mcp` + `/health` + `/`） |
| `mcp-sentinel status` | 打印配置 / 数据库 / 工具健康摘要 |
| `mcp-sentinel report [--output FILE] [--days N]` | 输出 Markdown 健康报告（默认 7 天窗口） |
| `mcp-sentinel tools list [--sort-by health_score\|tool_id]` | 列出所有工具及健康分 |
| `mcp-sentinel gen-config [--aggressive] [--output FILE]` | 生成剪枝后的 `sentinel.toml`，自动移除僵尸 server |

后台任务（启动后常驻）：
- 每小时聚合前一天的 `daily_stats`
- 每天清理超过 `retention_days` 的 `tool_calls`

---

## 架构与数据流

```text
AI Agent
  │  POST /mcp  (JSON-RPC)
  ▼
Axum Router ── CorsLayer + TraceLayer
  │
  ▼
meta_tools dispatcher ──► 5 个 handler
  │
  ├─ gateway_search_tools ─► SemanticRouter (TF-IDF + 同义词扩展 + RRF)
  │                          └─ HealthManager ─► final_score 重排
  │                          └─ DecisionTraceStore.record_search (返回 trace_id)
  │
  ├─ gateway_invoke ─► ToolPolicy.infer (side-effect 分级)
  │                  └─ policy.authorize (写/删需 confirmed)
  │                  └─ BackendManager.invoke_tool
  │                       ├─ Stdio: async_process 子进程 + JSON-RPC over stdin/stdout
  │                       └─ Http : reqwest + JSON-RPC over HTTP
  │                  └─ classify_error + transient 重试（指数回退）
  │                  └─ DecisionTraceStore.record_invocation
  │
  ├─ gateway_health_report ─► generate_health_report (Markdown)
  ├─ gateway_get_trace      ─► DecisionTraceStore.get
  └─ gateway_suggest_cleanup─► generate_cleanup_suggestions (JSON)

HealthManager.record_*  ─► 内存 HashMap<ToolHealth>  +  SQLite tool_calls
                          └─ 每小时聚合 daily_stats
                          └─ 每天清理 retention_days 之前的记录
```

---

## 项目结构

```
mcp-sentinel/
├── src/
│   ├── main.rs              # clap CLI: start / status / report / tools list / gen-config
│   ├── config.rs            # TOML 配置加载 + env 展开 + 单元测试
│   ├── decision_trace.rs    # 环形缓冲 (200) 的 DecisionTraceStore + trace_id
│   ├── governance.rs        # ToolPolicy (Read/Write/Destructive) + 错误分类 (7 类)
│   │
│   ├── backend/             # 后端适配层：stdio 子进程 + HTTP 客户端
│   │   ├── mod.rs           # BackendManager：聚合所有后端，统一调用入口
│   │   ├── stdio.rs         # JSON-RPC over stdin/stdout，oneshot 通道按 id 路由响应
│   │   ├── http.rs          # JSON-RPC over HTTP（支持 bearer auth）
│   │   └── types.rs         # Tool / ToolCall / ToolCallResult
│   │
│   ├── health/              # 健康追踪 + 诊断 + 报告生成
│   │   ├── mod.rs
│   │   ├── tracker.rs       # HealthManager（内存 + storage 双写）
│   │   ├── diagnostics.rs   # Markdown 报告 + cleanup 建议
│   │   └── types.rs         # ToolHealth / HealthScore（含完整单元测试）
│   │
│   ├── router/              # 混合路由：词法召回 + 同义词扩展 + RRF 融合 + 健康重排
│   │   ├── mod.rs           # SemanticRouter
│   │   ├── hybrid.rs        # expand_query（中英同义词表）+ reciprocal_rank_fusion
│   │   ├── tfidf.rs         # TfIdfIndex + cosine_similarity（含完整单元测试）
│   │   └── types.rs         # RankedTool
│   │
│   ├── storage/             # 持久化层（SQLite + tokio-rusqlite）
│   │   ├── mod.rs           # ToolCallRecord / ToolRegistry / DailyStat
│   │   └── sqlite.rs        # StorageManager：建表 / 写入 / p95 / 聚合 / 清理
│   │
│   └── gateway/             # HTTP server + JSON-RPC dispatcher
│       ├── mod.rs           # start_gateway + Axum routes (/, /health, /mcp)
│       └── meta_tools.rs    # 5 个元工具的实现 + SearchTrace/InvocationTrace 写入
│
├── eval/
│   └── tool-retrieval.jsonl # 检索评测集（标注意图 → 期望工具）
├── tests/
│   └── integration_test.rs  # 配置加载 + 临时目录集成测试
├── examples/
│   ├── minimal.toml         # 最小配置（仅 filesystem，无需 API key）
│   └── README.md            # 配置示例说明
├── scripts/
│   ├── verify.sh            # Linux/macOS 验证脚本
│   └── verify.ps1           # Windows 验证脚本
├── docs/
│   ├── DESIGN.md            # 技术设计文档
│   ├── QUICK_START.md       # 快速开始指南
│   └── TROUBLESHOOTING.md   # 故障排查文档
│
├── Cargo.toml               # Rust 依赖
├── sentinel.toml.example    # 完整配置示例
└── README.md                # 本文件
```

---

## 配置项速查

| 配置 | 默认 | 说明 |
|------|------|------|
| `gateway.port` | `3000` | HTTP 监听端口 |
| `gateway.web_ui` | `true` | 是否启用 web UI 占位 |
| `gateway.log_level` | `"info"` | 日志级别 |
| `routing.strategy` | `"tfidf"` | 路由策略（`tfidf` / 未来 `semantic`） |
| `routing.top_k` | `5` | 检索返回数量 |
| `routing.health_weight` | `0.4` | 健康分在最终排名中的权重（0-1） |
| `health.zombie_threshold_days` | `7` | 多少天无调用标记为僵尸 |
| `health.consecutive_failure_limit` | `5` | 连续失败多少次触发降权 |
| `health.degraded_score_penalty` | `0.1` | 降级工具的硬惩罚值 |
| `storage.db_path` | `~/.config/mcp-sentinel/sentinel.db` | SQLite 路径 |
| `storage.retention_days` | `30` | 调用记录保留天数 |

---

## 性能与资源

- 启动时间：~50–200ms（取决于后端数量）
- 检索延迟：~5–15ms（TF-IDF + 健康查询，无网络 I/O）
- 内存占用：~10–20MB 基线 + 每工具 ~1KB
- Token 节省：~85–95%（相比静态工具列表，5 个元工具替代 50+ 工具）
- 数据库大小：~1–5MB / 30 天 / 1000 调用/天

---

## 评测与质量保证

```bash
cargo test               # 单元 + 集成测试
cargo clippy             # 静态检查
cargo fmt --check        # 格式检查
```

**测试覆盖**：

- 单元测试：TF-IDF 检索、RRF 融合、同义词扩展、Config 解析与默认值、ToolHealth 计算、僵尸/降级判定、ToolPolicy 推断与授权、错误分类
- 集成测试：示例配置可加载、临时目录可写

**评测脚本**：见 `eval/tool-retrieval.jsonl`（标注意图 → 期望工具），用于线下回归 RRF + 健康加权的检索质量。

---

## 路线图

- ✅ Week 1：核心 TF-IDF 路由 + 元工具抽象
- ✅ Week 2：SQLite 持久化 + p95 + Markdown 报告 + gen-config
- ✅ Week 3：Governance（副作用分级 + 自动重试 + 错误分类）+ Decision Trace + 同义词扩展 + RRF
- ⏳ Week 4：向量 Embedding 替代 TF-IDF、Prometheus metrics、React Web UI、SSE 日志

---

## 适用场景

- **LLM Agent 作品集 / 技术深度展示**（本项目 README 设计目标之一）
- MCP 生态生产力工具：给你的 Claude / Cursor / Cline 接 10+ server 的同时，把健康和治理都管起来
- 企业内部 MCP 代理：在不修改客户端的前提下，统一管理 server、token、token 节省与审计

---

## 许可证

MIT — 详见 [LICENSE](LICENSE)

## 致谢

- 路由架构参考 [mcp-gateway](https://github.com/MikkoParkkola/mcp-gateway)
- 健康监控模式参考 [MCPHub](https://github.com/aniruddhabagal/MCP-Hub)
- 协议基础：Anthropic [Model Context Protocol](https://modelcontextprotocol.io/)

---

**项目类型**：开源 Portfolio 项目 · MCP 生态基础设施  
**最后更新**：2026-08-29