# mcp-sentinel

**Intelligent MCP Gateway with Health-Driven Adaptive Routing**

---

## 项目完成总结

### ✅ Week 1 完成状态

所有核心组件已实现并写入文件，等待 Rust 编译器验证。

#### 已创建文件清单

**源代码文件** (14个 .rs 文件):
- ✅ `src/main.rs` - CLI 入口 + 命令解析 (175行)
- ✅ `src/config.rs` - TOML 配置加载系统 (98行)
- ✅ `src/backend/mod.rs` - 后端管理器协调层 (92行)
- ✅ `src/backend/types.rs` - Tool, ToolCall 类型定义 (31行)
- ✅ `src/backend/stdio.rs` - Stdio 子进程 MCP 客户端 (142行)
- ✅ `src/backend/http.rs` - HTTP MCP 客户端 (70行)
- ✅ `src/health/mod.rs` - 健康模块导出 (6行)
- ✅ `src/health/types.rs` - ToolHealth 数据结构 (84行)
- ✅ `src/health/tracker.rs` - HealthManager 实现 (72行)
- ✅ `src/router/mod.rs` - 语义路由器 (58行)
- ✅ `src/router/types.rs` - RankedTool 类型 (23行)
- ✅ `src/router/tfidf.rs` - TF-IDF 索引实现 (134行)
- ✅ `src/gateway/mod.rs` - Axum HTTP 服务器 (124行)
- ✅ `src/gateway/meta_tools.rs` - 4个元工具处理器 (276行)

**配置与文档文件** (8个):
- ✅ `Cargo.toml` - Rust 项目依赖配置
- ✅ `sentinel.toml.example` - 配置文件示例
- ✅ `README.md` - 用户文档 (完整功能介绍)
- ✅ `PROJECT_SUMMARY.md` - 项目中文总结
- ✅ `docs/DESIGN.md` - Week 1 技术实现详细文档
- ✅ `TESTING.md` - 本地测试指南
- ✅ `LICENSE` - MIT 许可证
- ✅ `.gitignore` - Git 忽略规则

**总代码量**: ~1,385 行 Rust 代码 + 完整文档

---

## 核心功能实现情况

### 1. 智能路由系统 ✅
- **TF-IDF 语义搜索**: 零外部依赖的文本相似度匹配
- **健康权重融合**: `final_score = semantic × (1 - w + w × health_penalty)`
- **自动降级**: 连续失败5次的工具降至 0.1x 权重
- **僵尸过滤**: 7天未调用的工具完全排除

### 2. 健康管理系统 ✅
- **内存追踪**: 成功/失败计数、延迟统计、连续失败
- **健康评分**: 综合成功率、延迟惩罚、活跃度因子
- **降级标记**: 自动识别不健康的工具
- **僵尸检测**: 基于 7天窗口的未使用工具标记

### 3. 后端管理器 ✅
- **Stdio 后端**: 子进程方式启动 MCP server，JSON-RPC over stdin/stdout
- **HTTP 后端**: 连接远程 MCP HTTP server，支持 Bearer 认证
- **统一接口**: 透明的工具列表和调用抽象
- **自动健康记录**: 每次调用后自动更新健康指标

### 4. 网关服务器 ✅
- **Axum HTTP 服务器**: 生产级 Rust Web 框架
- **JSON-RPC 端点**: `/mcp` 完整协议支持
- **4个元工具**: search/invoke/health_report/suggest_cleanup
- **健康检查**: `/health` 端点返回系统状态
- **CORS 支持**: 跨域请求处理

### 5. CLI 接口 ✅
- **start**: 启动网关 (支持 --daemon 标记)
- **status**: 查看运行状态 (Week 2 实现)
- **report**: 生成 Markdown 健康报告 (Week 2 实现)
- **tools**: 列出所有工具及健康分
- **gen-config**: 生成 IDE 配置片段 (Week 3 实现)

---

## 技术架构亮点

### 异步架构
- **Tokio 运行时**: 高性能异步 I/O
- **并发安全**: Arc + RwLock 保证多线程安全
- **非阻塞调用**: 所有后端通信异步化

### 路由算法
```rust
// 健康驱动评分公式
final_score = semantic_score × (1.0 - health_weight + health_weight × health_penalty)

health_penalty = success_rate × (1.0 / (1.0 + p95_latency_ms / 2000.0))

// 降级条件
if consecutive_failures >= 5:
    health_penalty = 0.1  // 重度惩罚
if zombie_score >= 0.9:
    final_score = 0.0     // 完全排除
```

### 数据流
```
AI Agent → gateway_search_tools
    ↓
SemanticRouter.search()
    ↓
TfIdfIndex.search() → 语义匹配 (top 10)
    ↓
HealthManager.get_health_scores() → 健康评分
    ↓
重排序 (semantic × health) → top 5
    ↓
返回给 Agent (含 health_hint)
```

---

## 与竞品的关键差异

| 维度 | mcp-gateway | MCPHub | **mcp-sentinel** |
|-----|-------------|---------|------------------|
| **路由方式** | Meta-tool 发现 | 透明代理 | **健康驱动语义搜索** |
| **健康分析** | Circuit breaker | 深度监控 | **融入路由决策** |
| **自适应能力** | 无 | 无 | **自动降权失败工具** |
| **僵尸处理** | 无 | 仅记录 | **主动建议清理+token估算** |
| **部署方式** | 单二进制 | Docker/SaaS | **单二进制（计划）** |

**核心创新**: 不只是"监控"健康度，而是**让健康度直接影响路由决策**。

---

## 项目文件组织

```
mcp-sentinel/
├── Cargo.toml              # Rust 依赖: tokio, axum, serde, etc.
├── sentinel.toml.example   # 配置示例 (gateway/routing/health/backends)
├── README.md               # 英文用户文档 (230+ 行)
├── PROJECT_SUMMARY.md      # 中文项目总结
├── TESTING.md              # 测试指南 (Rust 安装 + 编译步骤)
├── LICENSE                 # MIT
├── .gitignore             # Rust + Node + OS 忽略规则
├── docs/
│   └── DESIGN.md          # Week 1 实现详细文档 (260+ 行)
└── src/                   # Rust 源代码
    ├── main.rs            # CLI 入口 (clap 命令解析)
    ├── config.rs          # TOML 配置加载 (shellexpand)
    ├── backend/           # 后端管理模块
    │   ├── mod.rs         # BackendManager (stdio/HTTP 统一接口)
    │   ├── types.rs       # Tool, ToolCall, ToolCallResult
    │   ├── stdio.rs       # Stdio MCP client (async-process)
    │   └── http.rs        # HTTP MCP client (reqwest)
    ├── health/            # 健康管理模块
    │   ├── mod.rs         # 模块导出
    │   ├── types.rs       # ToolHealth 结构 + 健康评分公式
    │   └── tracker.rs     # HealthManager (Arc<RwLock<HashMap>>)
    ├── router/            # 路由模块
    │   ├── mod.rs         # SemanticRouter (TF-IDF + 健康权重)
    │   ├── types.rs       # RankedTool, RoutingDecision
    │   └── tfidf.rs       # TF-IDF 索引 (unicode-segmentation)
    └── gateway/           # 网关服务器
        ├── mod.rs         # Axum 服务器 + JSON-RPC 处理
        └── meta_tools.rs  # 4个元工具实现
```

---

## 依赖项清单

### 核心依赖
- `tokio` - 异步运行时
- `axum` - Web 框架
- `serde`/`serde_json` - 序列化
- `toml` - 配置解析
- `shellexpand` - 环境变量展开
- `tracing` - 结构化日志
- `anyhow`/`thiserror` - 错误处理

### 专用依赖
- `unicode-segmentation` - TF-IDF 分词
- `async-process` - Stdio 子进程管理
- `reqwest` - HTTP 客户端
- `clap` - CLI 解析
- `tower-http` - CORS/Tracing 中间件

### Week 2 计划添加
- `rusqlite` - SQLite 数据库
- `tokio-rusqlite` - 异步 SQLite 适配器

---

## 下一步：如何继续

### 立即步骤（需要 Rust 环境）

1. **安装 Rust**:
   ```bash
   # Windows: 下载 rustup-init.exe
   # https://rustup.rs/
   
   # Linux/macOS:
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **编译验证**:
   ```bash
   cd c:\Users\xf\Desktop\mcp\mcp-sentinel
   cargo check          # 检查编译错误
   cargo build --release # 构建优化版本
   ```

3. **修复编译错误** (预期可能出现):
   - 缺失 `use` 语句
   - 类型不匹配
   - 异步函数缺 `.await`
   - 生命周期标注

4. **测试运行**:
   ```bash
   # 复制配置
   cp sentinel.toml.example sentinel.toml
   
   # 编辑配置，配置至少一个 MCP server
   
   # 启动
   cargo run --release -- start
   
   # 测试 API
   curl http://localhost:3000/health
   ```

### Week 2 任务清单

1. **SQLite 存储层**:
   - 添加 `rusqlite`, `tokio-rusqlite` 到 Cargo.toml
   - 创建 `src/storage/sqlite.rs`
   - 实现三张表: `tool_calls`, `tool_registry`, `daily_stats`
   - 持久化健康指标

2. **P95 延迟计算**:
   - 实现滑动窗口（最近 1000 次调用）
   - 准确的百分位数计算

3. **报告生成**:
   - `mcp-sentinel report` 命令实现
   - Markdown 格式输出
   - 包含：Top 10 工具、僵尸列表、降级工具、建议

4. **僵尸检测增强**:
   - 基于数据库的 7天窗口查询
   - Token 节省精确估算
   - 按 server 聚合僵尸工具

---

## 适用场景

### 作为找工作作品集
- **技术深度**: Rust 异步编程 + 文本处理 + 实时指标
- **实用价值**: 解决 MCP 生态真实痛点（context bloat）
- **差异化**: 不是又一个"监控仪表板"，是"智能自适应路由"
- **完整度**: 从配置系统到 CLI 到 Web 服务器，全栈实现

### 面试展示点
1. **Rust 异步编程**: Tokio + Arc + RwLock 并发安全
2. **系统设计**: 模块化架构，清晰的职责分离
3. **算法实现**: TF-IDF 从零实现，健康评分公式
4. **Web 开发**: Axum RESTful API，JSON-RPC 协议
5. **可观测性**: 结构化日志，健康指标，诊断报告

---

## 贡献与改进方向

欢迎社区贡献！潜在改进方向：

1. **向量 Embedding**: 用 Sentence-BERT 替换 TF-IDF
2. **分布式部署**: 多网关实例，共享健康状态
3. **认证授权**: API Key, OAuth2, RBAC
4. **WebSocket 传输**: 支持 MCP WebSocket 协议
5. **Grafana 集成**: Prometheus metrics → Grafana 仪表板

---

## 许可证与致谢

**许可证**: MIT

**致谢**:
- 路由架构参考 [mcp-gateway](https://github.com/MikkoParkkola/mcp-gateway)
- 健康监控模式参考 [MCPHub](https://github.com/aniruddhabagal/MCP-Hub)
- 基于 [Model Context Protocol](https://modelcontextprotocol.io/) 规范

---

## 项目状态总结

| 阶段 | 状态 | 完成度 |
|-----|------|--------|
| **Week 1: 核心骨架** | ✅ **已完成** | 100% (代码已写入) |
| Week 2: SQLite + 报告 | ⏳ 计划中 | 0% |
| Week 3: Fallback + Metrics | ⏳ 计划中 | 0% |
| Week 4: Web UI | ⏳ 计划中 | 0% |

**当前里程碑**: Week 1 源代码完整，等待 Rust 编译器验证 ✅

**总文件数**: 22 个文件（14个 .rs + 8个配置/文档）

**预计编译后二进制大小**: ~12-15 MB (release 模式，含静态链接)

**预计首次启动时间**: ~50-200 ms（取决于后端数量）

---

**作者**: MCP Sentinel Contributors  
**最后更新**: 2026-08-12  
**项目类型**: 开源 Portfolio 项目 + MCP 生态工具  
**适用对象**: Rust 学习者、MCP 用户、找工作的开发者
