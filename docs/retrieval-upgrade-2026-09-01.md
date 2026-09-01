# mcp-sentinel 检索系统改造实录：缺陷、方案与数据流

**日期**: 2026-09-01
**范围**: 检索层（`src/router/`）的全部改动——从 TF-IDF 双路检索升级为"词法 + 语义 + 特征重排"的混合管线
**关联提交**: `37761f4` → `f67340b` → `d65cf0f` → `13bfc33`（GitHub fan-123kf/mcp-sentinel）
**效果**: 17 条核心查询实测 R@1 从 33% → 47%，R@5 从 57% → 82%

---

## 一、起点：改造前检索系统是什么样

网关把 53 个后端工具（github 26 / filesystem 14 / everything 13）收进来后，对 Agent 只暴露 5 个元工具。当 Agent 调 `gateway_search_tools` 时，检索系统负责从 53 个工具里找出最相关的 5 个。改造前的流程是：

```
查询
 ├─ A 路: 原词直接跑 TF-IDF ──┐
 ├─ B 路: 查 11 条中英同义词表，改写后跑 TF-IDF ──┤→ RRF 融合 → 健康重排 → top-5
 └─ (只有这两路)
```

这个系统在 21 条查询的基准测试里暴露了四个结构性缺陷。

## 二、四个实测缺陷（每个都有具体案例）

### 缺陷 1：语义鸿沟——工具明明在库里，检索返回空

**现象**：查询"我想让同事们看到我改的代码"，目标工具 `github::create_pull_request` 就在 53 个工具里，但检索返回 `count: 0`。

**根因**：TF-IDF 是纯词法匹配——查询词和工具的索引文本必须有**字面相同**的词才能得分。"让/同事/看到/改的"这些词，在任何工具的名字和描述里都不存在；那 11 条同义词表里也没有对应条目。查询和目标之间隔着"人类意图词汇 → 工具动作词汇"的翻译鸿沟，词法检索跨不过去。

**后果**：这种情况下没有任何兜底——空结果裸返回给 LLM，只能靠 LLM 自己换词重搜（实测成功率不保证："创建 pull request" 能救回来，"共享我的代码修改" 再试还是空）。

### 缺陷 2：信息浪费——索引只用了工具定义的一小部分

**现象**：盘点 53 个工具的真实定义后发现，索引文本只拼了 `name + description` 两个字段，而 server 实际返回的信息远不止这些：

| 信息 | 数量 | 改造前是否使用 |
|---|---|---|
| description | 53/53 | 用了 |
| title（如 "Read File (Deprecated)"） | 27/53 | 扔掉 |
| 参数描述（如 `head: 返回前 N 行`） | github 79 条 / everything 15 / fs 7 | **全部扔掉** |
| required 参数名 | 44 个工具 | 扔掉 |
| server 名 | — | 扔掉 |

github 的 79 条参数描述是最可惜的——它们承载着描述里没有的行为细节（"返回前 N 行""分页参数"），全部被丢弃。

### 缺陷 3：跨 server 干扰——词频压过归属

**现象**：查询 "list files in a directory"，top-1 是 `github::get_pull_request_files`——因为 "files" 和 "list" 的词频在 github 工具上得分更高，尽管用户意图明显是本地文件系统。

**根因**：server 名不参与索引。检索器不知道"这个工具属于哪个服务"，纯靠词频说话。

### 缺陷 4：同分塌缩——排序退化为随机

**现象**：中文查询命中后，`create_issue`/`update_issue`/`get_issue` 的语义分**完全相同**（例如都是 0.016），第一名的归属由哈希遍历顺序决定——本质是抽签。

**后果**：中文查询的 R@1 = 0%（R@5 有命中但排不进第一）。

## 三、改造方案：三步走，每步对应哪些缺陷

### 第一步：描述增强（修缺陷 2、3）——先榨干免费的信息

把索引文本从两段扩成六段：

```
改造前: "read_file Read the complete contents of a file as text."
改造后: "filesystem Read File (Deprecated) read_file Read the complete contents of a file as text.
        required: path params: tail: If provided, returns only the last N lines of the file; head: ..."
```

- **server 名入索引**（部分修复跨 server 干扰）："github issue" 类查询会因 server 名命中而拉回正确的服务域
- **title 入索引**："Deprecated" 状态、人读名称参与匹配
- **参数描述入索引**：github 工具的索引文本从平均 60 字符扩到 250+ 字符，79 条参数描述首次可被检索

**为什么先做这步**：零新增依赖、零模型成本，半天工作量，先把免费的信息吃干净。而且这步决定了后面 embedding 的输入质量——语义模型看到的文本越丰富，向量越有区分度。

同时修同分塌缩（缺陷 4）：RRF 分数相同时，优先名字里包含查询词的工具——把"抽签"变成"确定性偏好"。

### 第二步：语义检索路（修缺陷 1）——给检索装上"听懂人话"的能力

新增 `EmbeddingIndex`，作为第三路召回接入现有 RRF：

**为什么选混合而不是替换**：实测确认了词法检索的不可替代场景——用户直接说工具名（"create_issue"）时，TF-IDF 精确命中，而 embedding 会把词投进语义空间产生模糊。保留词法路保住精确匹配的下限，embedding 路负责词法跨不过去的语义鸿沟，RRF 融合两边名次。

**模型选型**：bge-small-zh-v1.5（512 维，中英双语，ONNX CPU）。选它的理由：中文查询对英文工具描述是这个系统的真实场景（实测中文查询占比高），纯英文模型（默认的 bge-small-en）不行；本地推理（不调远程 API）是因为检索在调用热路径上，每次任务都要走，加 100-300ms 网络延迟和费用不可接受。

**工程上绕过的一个坑**：fastembed 默认从 HuggingFace 下载模型，但这台网络环境下 huggingface.co 直连失败，镜像站对大文件的重定向又会丢失 hf-hub 库必需的 Content-Range 响应头。解法是绕过 hf-hub 下载器：用 urllib 从镜像手动下载 6 个模型文件（95MB ONNX + tokenizer），以 `UserDefinedEmbeddingModel`（纯内存字节）方式加载。模型文件带外预置，不进 git。

**为什么做成 feature flag**（`SENTINEL_EMBEDDING=1` 开启，默认关）：语义路是新增的复杂依赖（模型文件、ONNX Runtime、2s 编码延迟），出问题时用一行环境变量回退到纯词法，不需要回滚代码。构建失败时也做了粘滞降级——模型缺失只报一次错，之后自动 TF-IDF-only。

### 第三步：特征重排（修 embedding 引入的新问题）

语义路上线后解决了"检索不到"，但暴露了一个**新失败模式**：embedding 会"自信地搜错"——返回一个语义沾边但实际不对的工具，且分数看起来很可信（0.7+）。词法检索失败时症状明显（返回空，LLM 一眼看出要换词）；embedding 失败时症状隐蔽（看起来像是找到了），更容易骗过 LLM。

解法是新增 `rerank.rs`，在 RRF 融合之后、最终排序之前，用四个**可解释特征**重新打分：

| 特征 | 权重 | 作用 |
|---|---|---|
| RRF 名次（归一化） | 0.45 | 保留三路融合的共识信号 |
| **name_overlap** | 0.30 | 查询词在工具**名字**里的覆盖率（含紧凑匹配："pull request" 命中 `create_pull_request`） |
| desc_overlap | 0.10 | 查询词在描述+参数文本里的覆盖率 |
| **param_match** | 0.10 | 只看参数层的命中（79 条参数描述参与排序） |
| same_server | 0.05 | 会话连贯性：上次 top-5 里出现过的 server 微幅提权 |

**name_overlap 是关键**：它是"自信地搜错"的解毒剂。一个语义沾边但名字与查询毫不相关的候选，name_overlap=0，总分被压下去；名字精确匹配的工具即使 RRF 名次稍差也能翻盘。典型实证：查询 "list files in a directory"，`filesystem::list_directory` 的 name_overlap（list/files/directory 三词全中）碾压词法分接近的 `github::get_pull_request_files`——上轮的跨 server 干扰案例就此修复。

**为什么用特征加权重排而不是 cross-encoder**：cross-encoder（精排模型）延迟 +100ms 且 Rust 生态不成熟；特征重排全部来自已有数据（名字、schema、会话记录），零新增依赖、亚毫秒级、每个分数都能解释"为什么排这"。

## 四、改造后的完整检索流程

Agent 调用 `gateway_search_tools(query)` 后，一次检索经过五个阶段：

```
阶段 0: 离线准备（启动时一次）
  网关拉起 3 个后端 → initialize 握手 → tools/list 收 53 个工具定义
  → Tool 结构解析 name/description/title/annotations/inputSchema
  → 存两份索引:
     · TF-IDF 索引（六段增强文本，词频哈希表，纯内存，构建 <1s）
     · Embedding 索引（同样六段文本 → bge-small-zh 编码成 53 个 512 维向量，构建 ~3s）
  → tool_id → input_schema 映射表（供重排用）

阶段 1: 三路召回（每次查询）
  A 路: 原词跑 TF-IDF → top-20（精确词面匹配）
  B 路: 11 条同义词表改写后再跑 TF-IDF → top-20（中文意图兜底）
  C 路: 查询编码成 512 维向量 → 与 53 个工具向量算余弦 → top-20（语义匹配，~2s）
  三路看到的是同一份增强文本，比较的是同一个语料库

阶段 2: RRF 融合
  三路各自是排名列表，按公式合成: 每个工具得分 = Σ 1/(60+名次)
  只看名次不看原始分（三路分数量纲不可比，RRF 天然规避）
  效果: 多路共同命中的工具得分最高（共识加权）

阶段 3: 健康重排（沿用原有逻辑）
  final = 融合分 × (1-0.4 + 0.4×健康分)
  僵尸工具（7 天未调用）→ 分数归零，从候选中剔除
  降级工具（连续失败 ≥5 次）→ 健康惩罚 0.1，重度打压

阶段 4: 特征重排（新增）
  查询分词一次，对每个融合候选:
    name_overlap（名字覆盖率，0.30）+ desc_overlap（0.10）
    + param_match（参数层命中，0.10）+ same_server（会话连贯，0.05）
    + RRF 名次归一分（0.45）
  → 新的 final_score

阶段 5: 截断与记录
  按 final_score 排序；同分时名字含查询词者优先（tiebreak）
  取 top-5；每个候选返回 tool_id / 一行描述 / 三组分数 / 健康提示
  （不返回完整 schema——这是省 token 的设计代价，参数由 LLM 按描述猜）
  记录搜索 trace（查询/候选数/选中工具/策略）
  本次 top-5 的 server 名记入 last_servers，供下次的会话连贯特征用
```

**数据流转一览**（一条查询的完整生命周期）：

```
Agent 发出: {"query": "帮我登记一个线上故障"}
    ↓
gateway_search_tools handler (gateway/meta_tools.rs)
    ↓
SemanticRouter::search (router/mod.rs)
    ├→ TfIdfIndex::search("帮我登记一个线上故障") → [] (中文词无字面命中)
    ├→ expand_query → "帮我登记一个线上故障 incident outage issue"
    │   └→ TfIdfIndex::search(改写) → [update_issue, create_issue, ...]
    └→ EmbeddingIndex::search_ranked(原词) → [create_issue(0.71), update_issue(0.69), ...]
    ↓
RRF: create_issue 同时被 B、C 两路命中 → 共识分最高
    ↓
健康重排: 全部健康，分数不变
    ↓
特征重排: "登记/线上/故障"分词后，create_issue 的 name_overlap 与 desc_overlap 与 update_issue 接近，
         RRF 名次差异主导 → 保持 create_issue 领先
    ↓
返回 top-5 + trace_id → LLM 挑选 → gateway_invoke → 治理检查(annotations优先) → 转发后端
```

## 五、实测效果与遗留问题

### 效果（17 条核心查询，真网关实测）

| 指标 | 改造前 | 改造后 |
|---|---|---|
| R@1 | 33% | **47%** |
| R@5 | 57% | **82%** |
| "list files in a directory"（跨 server 干扰） | MISS | **R@1** |
| "看看提交历史"（中文） | EMPTY | **R@5** |
| 单元测试 | 23 | **29**（新增 6 个） |

### 诚实声明的遗留问题

1. **语义鸿沟类 R@1 仍不满**：3 条里 2 条未进第一（"who owns this email address" 仍 MISS）。这类查询的意图词（email→user）连 bge-small-zh 的语义空间都映射不准，需要更强的模型或 cross-encoder 精排——已列为后续项
2. **每次查询 +2s 的编码延迟**：bge-small-zh 在 CPU 上单查询编码 2 秒。对 LLM 秒级轮次尚可接受，但可通过 ONNX 线程配置优化到亚百毫秒——未做，列入待办
3. **重排权重是默认值**：0.45/0.30/0.10/0.10/0.05 是工程判断的初始值，没有跑过网格搜索标定。`RerankWeights` 已支持配置覆盖，等真实查询日志积累后应该重标
4. **同义词表仍是 11 条硬编码**：语义路已经分担了它大部分职责，但纯中文短词（"读取"→read）在 embedding 编码 2s 的延迟下，词法改写仍是更快的路径

## 六、设计决策速查（为什么这么做）

| 决策 | 理由 |
|---|---|
| 混合三路而非 embedding 替换 | 实测词法在精确关键词场景最强（用户直接说工具名），替换会伤最强项；混合保下限 |
| 本地 ONNX 而非远程 API | 检索在热路径上，网络延迟/费用/故障点都不可接受 |
| 特征重排而非 cross-encoder | 零依赖、亚毫秒、可解释；cross-encoder 留作规模上去后的选项 |
| 语义路默认关（flag 控制） | 新增复杂依赖要有回退开关；构建失败自动降级不阻塞启动 |
| RRF 而非加权分数融合 | 三路分数量纲不可比（词法 0-1、余弦 -1~1），RRF 只看名次天然规避 |
| 增强文本先行 | 免费信息先榨干；且 embedding 的输入质量直接受文本丰富度影响 |
| 模型文件带外预置不进 git | 95MB 二进制不适合版本库；网络环境无法在线下载，预置是唯一稳定方式 |

## 七、改动文件清单

```
src/router/mod.rs        # 三路召回调度 + 特征重排接入 + schema/last_servers 状态
src/router/embedding.rs  # [新增] EmbeddingIndex（bge-small-zh ONNX，UserDefined 加载）
src/router/rerank.rs     # [新增] QueryFeatures + RerankWeights + rerank_score
src/router/tfidf.rs      # 索引文本增强（六段拼接）
src/backend/types.rs     # Tool 结构加 title 字段
Cargo.toml               # fastembed 5 (ort-download-binaries)
```
