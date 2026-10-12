> 历史对齐记录（本轮整理前）。当前特性、指标和复现命令以 README.md 与 EVALUATION.md 为准。

# 当前功能、描述对齐与执行计划

2026-10-12：核对发现仅有五个元工具、缺少三层安全扫描、模型默认未启用，提供的检索性能数字没有实测支持。

## 执行计划

1. 增加 gateway_security_scan；注册/索引时隔离可疑 description/schema，调用返回前扫描完整 output。
2. 追踪授权拒绝和调用错误，保持 VecDeque 200 条缓冲，trace ID 加入单调序号。
3. 配置 hybrid，修复本地模型路径、查询重复编码和 TF-IDF 文档频率，支持无模型构建。
4. 增加 eval CLI、真实 tokenizer 统计；严格模型模式禁止降级冒充模型评测。
5. 增加容量、HTTP 调用链安全、structuredContent 扫描、SQLite 恢复测试，同步 README。

上述代码工作已实现；完整模型运行指标尚待验证。

## 六个元工具

| 工具 | 用途 |
|---|---|
| gateway_search_tools | 自然语言检索，返回候选工具和 inputSchema，支持服务过滤 |
| gateway_invoke | 按 server::tool 调用；写入/删除须 confirmed；只读 transient 错误有限重试 |
| gateway_health_report | Markdown 健康报告 |
| gateway_get_trace | 查询搜索和调用 trace |
| gateway_suggest_cleanup | 基于使用情况提供清理建议 |
| gateway_security_scan | 对 description/schema/output 执行规则扫描 |

客户端初始 tools/list 只接收六个元工具，具体工具通过搜索按需发现。

## 检索实现

启用 hybrid 并配置模型时：BGE-M3 稀疏与稠密双路召回 → RRF 融合 → Cross-Encoder（bge-reranker-v2-m3）精排 → 健康惩罚。两路复用查询编码，服务过滤在截断和重排前执行。默认 tfidf 不加载 BGE-M3；模型缺失时显式降级，trace 和评测记录实际路径。

配置 routing.strategy = "hybrid" 或 SENTINEL_EMBEDDING=1 启用双路召回。FASTEMBED_MODEL_DIR 指定本地目录，包含 onnx/model.onnx（及外部数据文件）和 tokenizer/config 文件，重排器放在其 reranker 子目录。SENTINEL_DOWNLOAD_MODELS=1 允许通过 Hugging Face 下载模型。需支持 ONNX Runtime 的构建环境，依赖锁定 fastembed 5.17.4，本地加载不改变进程工作目录。

## 安全与治理

递归扫描 description/title/annotations、schema 字符串和键以及完整调用结果，包括 structuredContent 和 isError 内容。规则识别指令覆盖、角色伪造、秘密外传、绕过确认，处理中英文、大小写和部分零宽字符。命中后隔离元数据或阻断输出，诊断不回显攻击正文。

这是规则防护，存在漏报和误报，不能保证识别所有注入，不能撤销已发生的后端副作用。后端应受信任；annotations 不是认证凭据，confirmed 是显式调用标志，不是独立的人类批准证明。网关默认绑定 127.0.0.1，尚不提供入口认证。

副作用按 Read/Write/Destructive 分类，明确只读工具才允许自动重试，未知工具按写操作保守处理。isError 进入失败统计。最近 200 条搜索/调用元数据由 VecDeque 保存，拒绝和输出拦截也返回 trace_id。trace 不存调用参数和结果正文，但搜索 query 可能含敏感信息；重启后内存 trace 不保留。

## 对齐状态和实测口径

| 描述 | 当前状态 |
|---|---|
| 6 个元工具 | 已实现且测试验证 |
| 10+ server / 50+ 工具 | 12 模拟 HTTP 后端 / 60 安全工具验证；固定语料 53 工具来自 3 个 server |
| 按需发现与调用 | 已实现，搜索包含参数 schema |
| 工具列表减少 90%+ | cl100k_base 实测 92.47% |
| 8K → 700 | 实测 6,950 → 523，仅初始工具定义 |
| 稀疏+稠密+RRF+Cross-Encoder | 代码实现、类型检查通过；完整运行待验证 |
| R@5 92.7%、MRR 0.891、Top-1 91% | 尚不符合“实测”措辞，保留为目标 |
| 三级治理、确认、错误分类、重试 | 已实现并测试 |
| 三层规则安全扫描 | 已接入调用链，有规则覆盖边界 |
| VecDeque 决策追踪 | 已实现，拒绝/输出拦截也追踪；最近 200 条 |

工具语料 docs/eval/data/backend_tools.json 包含 GitHub 26、Filesystem 14、Everything 13，共 53 个工具。eval/retrieval-19.jsonl 固定取已有 queries.json 顺序前 19 条查询，保留既有标签，没有按结果调换答案；这是 19 条查询，不是 19 类。

TF-IDF 降级基线：Recall@5 63.16%，MRR@20 0.491，Top-1 42.11%。19 条单答案查询的 Top-1 以 1/19 为步长，91% 不是该分母下可直接得到的准确率。不要混淆 Hit@5 和多答案 Recall@5。

cl100k_base 精确 token 统计为 6,950 → 523，减少 92.47%；仅统计紧凑 JSON 工具列表，不含搜索返回 schema、调用内容、消息封装和整段对话。8K→700 只能视为近似量级。容量测试使用模拟后端，不是 12 个真实产品的兼容性验证。

本机缺少模型文件；GNU 无对应 ONNX 预编译库，MSVC 缺 link.exe。模型特性的 cargo check 在 DOCS_RS=1 跳过原生库链接的条件下通过，仅证明 Rust 类型检查，不代表模型可运行。

## 运行和复现

```powershell
Copy-Item examples/minimal.toml sentinel.toml
cargo test --no-default-features
cargo run --no-default-features -- start
# TF-IDF 基线
cargo run --no-default-features -- eval --output retrieval-eval.json
# 完整 ONNX 环境和模型就绪后；缺模型或降级会失败
cargo run -- eval --require-models --output retrieval-model.json
# 需 tiktoken
python scripts/token_account.py retrieval-eval.tools.json retrieval-eval.meta.json --output token-account.json
```

评测导出逐查询排名。Recall@5 按全部相关工具计算；MRR@20 取前 20 的首个相关位置；Top-1 为首位命中率。无相关答案的查询不混入这些分母。严格模型模式校验实际执行双路融合与 Cross-Encoder，不把降级结果标为模型指标。

网关提供 POST /mcp 和 GET /health；HTTP 后端当前仅支持 JSON 响应，完整 Streamable HTTP 会话/SSE 兼容性仍需补齐。

## 可使用的描述

面向 Agent 的 MCP 智能网关：通过按需发现与调用将多后端工具收敛为六个元工具；实现 Read/Write/Destructive 分级、显式确认、错误分类与安全重试，接入 description/schema/output 三层规则扫描，并使用 VecDeque 保存最近检索和调用元数据。以 53 个工具的固定语料统计，初始工具定义从 6,950 降至 523 tokens（cl100k_base），减少 92.47%。实现 BGE-M3 稀疏/稠密召回、RRF 融合与 Cross-Encoder 重排，模型指标待完整运行环境复测。
