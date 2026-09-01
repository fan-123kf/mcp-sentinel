# mcp-sentinel 深度实测报告（第二轮：全量化、可复现）

**日期**: 2026-09-01
**测试者**: Hermes Agent（glm-5.3-flash）| **被测对象**: mcp-sentinel v0.1.0（修复 19 处编译错误后可运行）
**链路**: Hermes 桌面端 → mcp__sentinel__* 5 元工具 → HTTP localhost:3000/mcp → 网关 → 3 个 stdio 后端（53 工具）
**原始数据**: `docs/eval\`（backend_tools.json / bench.json / token_account.json / queries.json）

---

## 1. 测试方法（可复现声明）

- **检索质量**: 21 条人工标定查询（期望工具全部对照真实 tools/list 校准，非猜测），按 4 类分层：精确关键词 8 / 中文同义词 6 / 自然语言 4 / 语义鸿沟 3。指标：Recall@1、Recall@5、MRR
- **Token 账**: 用 **tiktoken cl100k_base**（GPT-4 同款分词器）对真实 schema 计数，非字符估算
- **延迟**: N=30 检索采样 p50/p90/p99 + 单次调用开销分解
- **健康生命周期**: 6 次连续失败（>failure_limit=5）触发降级 → 观察 4 组不同查询下的可见性
- **治理**: 8 个代表性工具 × confirmed 两态矩阵
- **持久化**: 直接查 SQLite 取证

---

## 2. Token 经济账（tiktoken 实测）

| 组成 | 工具数 | 真实 token 数 |
|------|--------|--------------|
| github server | 26 | 4,424 |
| filesystem server | 14 | 1,901 |
| everything server | 13 | 1,289 |
| **静态全量合计** | **53** | **7,614 / 每轮对话** |
| **5 个元工具** | **5** | **576 / 每轮对话** |

**压缩率 13.2 倍，节省 92.4%**（README 声称 90%，实测属实且略超）。

多轮放大账（网关方案另有搜索+结果 overhead，此处只计常驻项）：
- 10 轮对话：静态 76,140 vs 元工具 5,760
- 50 轮对话：静态 380,700 vs 元工具 28,800

**裁定：✅ 项目最强声明成立，且有分词器级证据。**

---

## 3. 检索质量基准（21 查询，校准期望）

| 类别 | n | Recall@1 | Recall@5 | 解读 |
|------|---|----------|----------|------|
| exact_keyword | 8 | **62%** (5/8) | 75% | 词面命中即可得分，仍有 2 个失败 |
| chinese_synonym | 6 | **0%** (0/6) | 67% | 能进 top-5 但**从未排第一**——排序无区分度 |
| natural_language | 4 | 50% (2/4) | 50% | "simulate a slow operation" 命中，"compress a file" 指向 edit_file（错） |
| semantic_gap | 3 | **0%** (0/3) | **0%** | 词面零重叠全军覆没，top-1 全是错误工具 |
| **OVERALL** | 21 | **33%** | **57%** | **MRR = 0.426** |

**三类结构性失败（有实证）**：

1. **同分塌缩**：中文查询命中后 `create_issue`/`update_issue`/`get_issue` 语义分**完全相同**（0.016）——RRF 融合后排序在语义上无区分度，第一名实际由 TF-IDF 的哈希遍历顺序决定。中文 R@1 = 0% 的直接原因。
2. **跨 server 干扰**：`list files in a directory` 的 top-1 是 `github::get_pull_request_files`——"files"+"list" 的词频压过了 server 归属。**网关没有 server 域先验**。
3. **语义鸿沟硬失败**：`how do I let teammates see my code changes`（期望 create_pull_request）——top-1 `search_code`，期望工具**不在 top-5**。这不是排序问题，是召回为 0。TF-IDF 的理论天花板被实测踩到。

**反直觉发现——延迟换正确性的账**：p50=2ms 的检索有 67% 概率给出错误第一候选；此时 LLM 需要"读到错误候选→重新搜索/直接错调"，每轮纠错成本远超检索本身。**检索省的 token 有一部分被错误候选的 LLM 纠错轮次吃回去了**——具体比例取决于 LLM 的纠错能力，本测试未量化（见 §8 局限）。

---

## 4. 健康生命周期实测（全场最有价值的验证）

**时间线**（github::create_issue，假 GITHUB_TOKEN）：

| 阶段 | 操作 | 观察 |
|------|------|------|
| t0 | 连续 6 次失败（permission 类） | health 1.000 → **0.000**，consecutive_failures=6 > limit(5) |
| t1 | 4 组不同查询重搜（"create a GitHub issue"/"create issue"/"open a new ticket"/"file an issue for a bug"） | **4/4 全部不可见**——降级工具被从 Agent 视野彻底剔除 |
| t2 | `gateway_invoke` 直接打降级工具 | **网关仍放行**（只是透传后端报错）——惩罚只在搜索层 |
| t3 | 健康报告 | 自动诊断："0% health, likely auth token expired"；suggest_cleanup 给出修复建议 |

**同时确认的边界**：
- 降级是**内存态**（重启网关即清零），持久层只存调用明细——"黑名单"不跨会话
- 剔除是硬截断（final_score=0），没有"降级但仍以低优先级可见"的灰度
- **invoke 层不做健康拦截**：搜索层把坏工具藏起来了，但一旦 LLM 从别处（记忆/猜测）拿到 tool_id，仍可直达——防线只有一层

**裁定：✅ "健康分驱动路由"是本项目最扎实、经得起实测的功能。**

---

## 5. 治理矩阵实测（8 工具 × 2 态）

| 工具 | 推断等级 | 无 confirmed | confirmed=true |
|------|---------|--------------|----------------|
| filesystem::read_file | Read | ✅ 执行 | 执行 |
| filesystem::list_directory | Read | ✅ 执行 | 执行 |
| everything::echo | Read | ✅ 执行 | 执行 |
| github::update_issue | Write | 🛑 拦截 | 到达后端(auth fail) |
| github::create_issue | Write | 🛑 拦截 | 到达后端(auth fail) |
| github::merge_pull_request | Write | 🛑 拦截 | （未执行，留证） |
| **filesystem::move_file** | **Write** | ⚠️ **放行了！** | — |
| everything::get-sum | Read | ✅ 执行 | — |

**🔴 发现一个治理漏洞**：`move_file`（改数据，属写操作）在无 confirmed 时**直接执行成功**。对照源码 `governance.rs` 的启发式规则——关键词表是 `create/update/write/send/post/put/set/merge`，**"move" 不在表里**，被默认当成 Read。假设一个 agent 把用户文件 `move` 到垃圾桶路径，治理层不会拦。

**这实证了上一轮的判断**：按工具名猜等级的启发式有系统性漏网，`move/rename/replace/sync/import` 这类词都不在关键词表。企业落地前必须改为显式标注。

---

## 6. 延迟与开销

| 指标 | 数值 | 解读 |
|------|------|------|
| 检索 p50 / p90 / p99 (N=30) | 2.0 / 3.0 / 4.3 ms | 含 HTTP 一跳，可忽略 |
| 单次 invoke 端到端 | 17.7ms | 其中后端执行 1ms，**网关附加 ≈ 17ms** |
| 网关附加开销分解 | HTTP 往返 + JSON-RPC 解析 + 治理检查 + 健康写库 + trace | 对 LLM 秒级循环完全无感 |

**裁定：✅ 性能声明（检索 5-15ms）属实甚至更好；invoke 一跳 17ms 是无感成本。**

---

## 7. 持久化取证（SQLite 直接查验）

- `tool_calls`：15 行，记录每次调用（tool_id/success/latency_ms/error_type/时间戳）✅
- `tool_registry`：52 行——**启动时 53 个工具全量注册**（含 schema 全文）✅
- `daily_stats`：0 行——聚合任务报错（启动日志即有 `Failed to aggregate daily stats`），**日报功能实际是坏的** 🔴

---

## 8. 综合裁定

| 项目声明 | 实测结果 | 证据 |
|---------|---------|------|
| 5 元工具代理全部后端 | ✅ | tools/list=5，52 工具注册 |
| token 节省 ~90% | ✅ **92.4%（13.2x）** | tiktoken 实测 |
| 健康驱动路由 | ✅ **最扎实功能** | 6 次失败→4/4 查询不可见 |
| 治理三级管控 | ⚠️ 有漏洞 | move_file 漏拦 |
| 决策追踪 | ✅ | trace 全链路、不含参数 |
| 检索 5-15ms | ✅ p50=2ms | N=30 实测 |
| 服务 300 工具场景 | ❌ **52 工具时已塌缩** | R@1=33%，中文 0% |
| 完整编译通过 | ❌ | 19 处修复 |

**最终定性**：一个**账算得很准（token 92.4%）、健康闭环真实有效、但检索质量撑不起野心**的概念验证。33% 的 Recall@1 意味着 LLM 三分之二的搜索第一眼看到的是错误工具——在"搜索→选择→调用"的链路里，这把精排压力全部转嫁给了 LLM 的纠错轮次。**它的健康层值得借鉴，它的检索层是上 embedding 的最硬证据，它的治理层在写关键词补全前不可上生产。**

## 9. 测试局限（诚实声明）

- 查询集为人工构造（21 条），非真实用户日志，MRR 有分布偏差
- 期望工具以"测试者判断"标定，个别（如 compress→gzip）存在标注争议
- 未测 LLM 纠错回路（错误候选→LLM 是否能自救）——这决定 33% R@1 的真实体感
- 未做与"全量注入"的端到端对照（真正的 53 工具直连 agent 跑同一任务集）
- daily_stats 聚合损坏，p95 数据不可用，健康报告的延迟列缺准
