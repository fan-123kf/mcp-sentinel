# mcp-sentinel 全面评测与修复总结报告

**评测周期**: 2026-09-01
**评测者**: Hermes Agent (glm-5.3-flash) ｜ **对象**: mcp-sentinel v0.1.0（Rust MCP 网关）
**方法**: 四轮递进式实验（编译验证 → 功能基准 → 纠错回路 → 端到端对照）+ 修复回归
**产物目录**: `docs/eval\`

---

## 第一部分：修复记录（本轮完成）

评测共发现 **3 类功能性缺陷 + 1 类卫生问题**，全部修复并回归验证：

### Fix-1 治理漏洞：`move_file` 漏拦 🔴→✅

**发现**（第二轮治理矩阵）：`filesystem::move_file` 在无 `confirmed` 时直接执行成功。根因：`governance.rs` 的 Write 关键词表缺 `move`——同类的 `rename/replace/sync/import/restore/copy/upload` 也全部漏网。

**修复**：
- destructive 表：+`unlink, truncate, wipe, erase`
- writes 表：+`move, rename, replace, sync, import, restore, apply, edit, append, assign, copy, upload, commit, push, close, reopen`

**回归**：
- 新增 3 个单元测试（mutators 全拦截 / destructive 扩展 / reads 不误伤），`cargo test` **19+3 全过**
- 实机复测治理矩阵 **6/6 符合预期**：move_file 🛑拦截、edit_file 🛑拦截、merge 拦截、read/echo 正常放行

### Fix-2 日报聚合崩溃：`daily_stats` 从未工作 🔴→✅

**发现**（第二轮持久化取证）：启动即报 `Failed to aggregate daily stats`，`daily_stats` 表恒为空。根因：SQL 在 `LIMIT/OFFSET` 内嵌相关子查询，SQLite 报 `no such column: tool_calls.tool_id`——聚合语句从未成功执行过。

**修复**：用窗口函数重写——`ROW_NUMBER() OVER (PARTITION BY tool_id, success ORDER BY latency_ms)` 直接定位 p95 行，消除 OFFSET 相关子查询。

**回归**：重启后启动日志**不再报错**；对 59 条调用记录实测聚合，**12 个工具的日统计正确落库**（含 p95 延迟，如 `filesystem::search_files p95=464ms`）。

### Fix-3 Windows spawn（历史修复，此轮验证保持）

`npx` → `npx.cmd`（Windows 批处理）、`async_process` 同步流误配 tokio 异步 IO 重写为 tokio 原生 `Command`。本轮重启三次均正常拉起全部后端。

### Fix-4 卫生清理

无用 import（`ChildStdout`）、无用变量 warning 已消减（12→11，其余为 dead-field 类低危项，保持不动以免碰架构）。

### 修复后基线

- 编译：0 error，warning 11（全部 dead-code 类）
- 单元 + 集成测试：**22/22 通过**（含 4 个新回归测试）
- 网关启动：52 工具索引，无 WARN/ERROR
- 治理矩阵：6/6 拦截/放行正确

---

## 第二部分：四轮评测结论汇总

### 轮次结构

| 轮 | 问题 | 手段 |
|---|---|---|
| 1 | 能跑吗 | 修复 19 处编译错误，网关活体运行（52 工具） |
| 2 | 声称属实吗 | tiktoken 计数 + 21 查询检索基准 + 健康/治理/持久化取证 |
| 3 | 短板致命吗 | 3 个隔离子代理 6 任务纠错回路实验 |
| 4 | 划算吗 | 网关 vs 直连端到端对照（同任务同模型同后端） |

### 2.1 项目声称 vs 实测（最终裁定表）

| 项目声称 | 裁定 | 关键证据 |
|---------|------|---------|
| 5 元工具代理全部后端 | ✅ | tools/list=5，52 工具注册 |
| Token 节省 ~90% | ✅ **92.4%（13.2x）** | tiktoken cl100k_base 实测：7,614→576 |
| 健康驱动路由 | ✅ **最扎实功能** | 6 连败 → 4/4 查询中不可见 → 报告自动诊断 auth 过期 |
| 治理三级管控 | ⚠️→✅（修复后） | move_file 漏拦已修，6/6 复测通过 |
| 决策追踪 | ✅ | trace 全链路、不含参数（隐私设计属实） |
| 检索 5-15ms | ✅ p50=2ms / p99=4.3ms | N=30 实测 |
| 服务 300 工具场景 | ❌ 存疑 | 52 工具时 R@1=33%，中文 R@1=0% |
| 完整编译通过 | ❌→✅ | 19 处修复 + 本轮 2 处功能修复 |

### 2.2 三大功能的量化画像

**Token 账（实测）**

```
静态全量注入 53 工具: 7,614 tok/轮
网关 5 元工具:        576 tok/轮   → 省 92.4%
50 轮对话累计差:      ~35 万 tok
```

**检索质量（21 查询校准基准）**

| 类别 | R@1 | R@5 |
|------|-----|-----|
| 精确关键词 | 62% | 75% |
| 中文同义词 | 0% | 67% |
| 自然语言 | 50% | 50% |
| 语义鸿沟 | **0%** | **0%** |
| **总 MRR=0.426** | 33% | 57% |

结构性缺陷实证：同分塌缩（中文查询命中后语义分完全相同，排序退化）、跨 server 干扰（"list files" 被 github 工具淹没）、语义鸿沟硬失败（零词面重叠→召回为 0）。

**LLM 纠错对冲（隔离 agent 实验）**

两轮共 12 个 agent-任务，**12/12 达成，0 次错误工具调用**。三种自救模式：改写查询（T4 语义鸿沟 6 步迭代成功）、批量假设检验、系统性排查后正确放弃（零幻觉）。代价：简单任务 15s vs 多轮试探 200s——**检索弱不导致失败，导致延迟**。

### 2.3 端到端对照（第四轮核心数字）

| 指标 | 网关组 | 直连组（53 工具全量） |
|------|--------|---------------------|
| 任务成功率 | **6/6** | **6/6** |
| 静态 token/轮 | **576** | 7,614 |
| 墙钟耗时 | 153.9s | 100.5s（快 53%） |
| **LLM API 轮次** | **7** | 8 |
| 错误工具调用 | 0 | 0 |

**洞察**：网关"慢 53%"是墙钟时间，**API 轮次反而少 1**（搜索合并进单轮推理）——按轮计费场景下发现成本比直觉便宜。直连组的全量 schema 有隐性红利（`list_directory_with_sizes` 顺带解决 T6、T4 零调用），这是网关一行式 description 给不了的。

### 2.4 最终定性

> **mcp-sentinel 是一次"用单轮变深换常驻变薄"的架构交易**：正确性无损（12/12 任务、0 错误调用），常驻 context 降 92.4%，代价是发现环节的延迟。交易在"工具多、对话长、模型强"时是赚的。
>
> 它真正的不可替代性不是省 token——**是直连模式永远给不了的健康治理与审计**（坏工具自动出局、写操作强制确认、全程可追踪）。
>
> 修复后的工程状态：22/22 测试通过、无启动报错、日报功能首次真正工作。剩余短板：检索器（TF-IDF + 11 条同义词表）——上 embedding（建议 BM25 + embedding + RRF 混合，本地小模型）是数据支撑的下一步。

---

## 第三部分：遗留事项与建议

| 事项 | 优先级 | 说明 |
|------|--------|------|
| 上混合检索（BM25+embedding+RRF） | 高 | R@1=33% 是最硬的证据；RRF 骨架已在，只需换一路 |
| 治理标注流程化 | 高（企业） | 启发式兜底可以，但生产需 server 提供方显式标注副作用 |
| 按用户过滤检索 | 高（企业） | 多租户权限是 embedding 之外更关键的缺口 |
| 300+ 工具规模复测 | 中 | 验证发现成本曲线是否随规模反转 |
| stdio 后端 initialize 握手 | 中 | 当前跳过握手直接 tools/list，严格 server 会翻车 |
| dead-code warnings | 低 | 11 个 warning 均为未读字段类，不影响功能 |

## 附录：产物清单

```
docs/eval\
├── TEST_REPORT.md              # 第二轮：基准报告
├── RECOVERY_REPORT.md          # 第三轮：纠错回路（v2 含白名单更正）
├── E2E_COMPARISON_REPORT.md    # 第四轮：端到端对照
├── EVAL_SUMMARY.md             # 本报告
├── bench.json / queries.json   # 检索基准原始数据
├── token_account.json          # tiktoken 计数
├── backend_tools.json / meta_tools.json  # 真实工具定义快照
└── e2e_comparison.json / e2e_stats.json  # 对照实验数据

(local eval runner, not committed)\
├── sentinel_cmd.py             # 网关组 CLI（5 元工具壳）
├── direct_cmd.py               # 直连组 CLI（53 工具全量）
└── sandbox\note.txt            # 任务沙箱
```
