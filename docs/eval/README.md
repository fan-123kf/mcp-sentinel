# 评测报告索引 (Evaluation Reports)

本目录包含对 mcp-sentinel 的四轮实测报告与原始数据。所有结论均来自与真实 LLM agent (glm-5.3-flash) 的端到端实验，方法与原始数据一并列出以便复现。

## 阅读顺序

| 文件 | 内容 | 一句话结论 |
|------|------|-----------|
| [EVAL_SUMMARY.md](EVAL_SUMMARY.md) | **总报告**（先读这个）：修复记录 + 四轮结论汇总 + 遗留事项 | 修复后 22/22 测试通过；token 省 92.4%，健康闭环有效，检索是下一步短板 |
| [TEST_REPORT.md](TEST_REPORT.md) | 第二轮：检索基准 + token 计数 + 健康生命周期 + 治理矩阵 | R@1=33%（语义鸿沟 0%）；发现 move_file 治理漏洞 |
| [RECOVERY_REPORT.md](RECOVERY_REPORT.md) | 第三轮：LLM 纠错回路（隔离 agent 6 任务） | 弱检索可被 LLM 架构性对冲：12/12 任务、0 错误调用 |
| [E2E_COMPARISON_REPORT.md](E2E_COMPARISON_REPORT.md) | 第四轮：网关 vs 直连端到端对照 | 正确性持平（6/6 vs 6/6）；53% 时间换 92.4% token |

## 方法摘要

- **Token 计数**: tiktoken cl100k_base 对真实 tools/list schema 计数（非字符估算）
- **检索基准**: 21 条人工标定查询，期望工具对照真实 tools/list 校准，4 类分层（精确关键词/中文同义词/自然语言/语义鸿沟），指标 Recall@1/@5/MRR
- **纠错回路**: 3 个全新上下文的隔离子代理，只给 5 元工具 CLI 壳，零答案泄露
- **端到端对照**: 同模型同任务同执行后端，唯一变量为工具暴露方式（5 元工具 vs 53 工具全量）
- **治理**: 8 工具 × confirmed 两态矩阵；健康: 连续失败降级 → 4 组查询可见性验证

## 修复闭环

评测发现的问题已全部修复并回归（详见 EVAL_SUMMARY.md 第一部分）：

1. 编译（19 处，代码在修复前无法在任何现代 Rust 上编译）
2. 治理漏洞（`move_file` 无确认即执行 → 关键词表补 16 个变更动词 + 3 个回归测试）
3. daily_stats 聚合崩溃（LIMIT/OFFSET 内相关子查询被 SQLite 拒绝 → 窗口函数重写）
4. Windows spawn（npx.cmd + tokio 原生 stdio 后端重写）

最终基线: `cargo test` 22/22 通过，治理矩阵 6/6，日报功能首次真正落库。

## 数据

`data/` 目录包含各轮原始数据（bench.json 检索基准、token_account.json 分词器计数、e2e_comparison.json 对照指标、backend_tools.json 真实工具 schema 快照等）。子代理执行日志与本地 SQLite 含机器路径，未纳入版本库。
