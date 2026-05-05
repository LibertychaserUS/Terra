# Terra — 架构方案（Architecture Blueprint）

> 本文档整合 2026-05-20 讨论中产出的所有技术方案和架构设计。
> 基于 Oliver 的设计思路（VISION_OLIVER.md），由 Claude 进行技术细化。

---

## 1. 项目定位

**Terra** 是一个通用 AI Agent 底座平台，受 Caging Engineering 启发。

- **不是** chatbot，不是特定领域工具
- **是** 让 AI Agent 行为可靠、可控、可审计的底座
- **核心洞察**：可靠性不是模型的属性，而是**系统**的属性
- **聚焦**：coding / 软件工程场景（MVP）
- **技术栈**：Rust

### 1.1 与 Caging Engineering 的关系

Terra 受 CE 启发，但不是 CE 的完整参考实现。CE 的 8 cage 概念映射到 Terra 的具体模块中，同时在实践中可能演化出 CE 未定义的部分。

### 1.2 与 OfficeX 的关系

OfficeX（文档操作平台）将在 Terra 建成后重构为其第一个领域 Skill。Terra 通过 MessagePack over stdio 与 Skill 通信，Skill 可以是任何语言。

---

## 2. 核心子系统（6 个）

```
┌─────────────────────────────────────────────────┐
│                    Terra Platform                 │
│                                                   │
│  ┌─────────────────────────────────────────────┐ │
│  │ 1. Intelligence Core (智能核心)               │ │
│  │    Universal Layer + Domain Modules           │ │
│  └─────────────────────────────────────────────┘ │
│                                                   │
│  ┌──────────────┐  ┌──────────────────────────┐ │
│  │ 2. Language   │  │ 3. Multi-Agent Runtime    │ │
│  │ Compilation   │  │    Main + Sub + Cage      │ │
│  │ Layer (JSP)   │  │                           │ │
│  └──────────────┘  └──────────────────────────┘ │
│                                                   │
│  ┌──────────────┐  ┌──────────────────────────┐ │
│  │ 4. Memory     │  │ 5. Caging Layer           │ │
│  │ Database      │  │    8 Cage + Constitution  │ │
│  │ (Rust-safe)   │  │                           │ │
│  └──────────────┘  └──────────────────────────┘ │
│                                                   │
│  ┌─────────────────────────────────────────────┐ │
│  │ 6. Provider Dispatch (LLM 调度)              │ │
│  └─────────────────────────────────────────────┘ │
│                                                   │
│  Skills:                                          │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│  │Superpower│ │ OfficeX  │ │ Future Skills     │ │
│  │(工程规范) │ │(文档操作) │ │                   │ │
│  └──────────┘ └──────────┘ └──────────────────┘ │
└─────────────────────────────────────────────────┘
```

---

## 3. Intelligence Core（智能核心）

Agent 出厂即有的第一性原理。从"一"开始，不从"零"。

```
Intelligence Core
│
├── Universal Layer (通用，L0 只读，编译期内嵌)
│   ├── Reasoning Framework
│   │   - 先理解再行动，先读再写
│   │   - 分解复杂问题，不试图一步解决
│   │   - 寻求证据而非信心（verify, don't believe）
│   │   - 识别自己不知道的东西（不编造）
│   │   - 深度优先：在一个方向推进到底
│   │
│   ├── Behavioral Framework
│   │   - 优雅 > 复杂
│   │   - 最小改动原则
│   │   - 安全默认
│   │   - 测试验证
│   │   - 一致性（遵循项目已有 pattern）
│   │
│   └── Self-Diagnostic Rules
│       - 我是否还在朝目标前进？（防目标偏移）
│       - 我是否在走捷径？（防偷懒）
│       - 我的推理是否够深？（防浅层推理）
│       - 对照 success predicate 检查输出
│       - 不一致时主动上报
│
└── Domain Modules (可插拔，可组合，像 Rust trait)
    ├── coding/      — 编程原则、设计模式、重构规则
    ├── algorithm/   — 复杂度分析、数据结构选型
    └── document/    — 文档操作（OfficeX 能力）

    每个 module 注册自己的：
    - Perspectives（观察视角，用于 Multi-Head Attention）
    - Rules（行为规则，注入 Caging Layer）
    - Verifiers（验证器，注入 Verification Cage）
```

---

## 4. Language Compilation Layer（JSP 式语言编译层）

将用户不规范的自然语言编译为标准化的 Spec IR。

```
User Input (自然语言，术语不规范)
    │
    ▼
┌─────────────────────────────────────┐
│ 1. Lexical Pass                      │
│    提取关键实体和意图                  │
├─────────────────────────────────────┤
│ 2. Semantic Resolution Pass          │
│    映射到项目上下文中的具体对象        │
├─────────────────────────────────────┤
│ 3. Terminology Normalization         │
│    统一命名规范                       │
├─────────────────────────────────────┤
│ 4. Completeness Check                │
│    检测缺失信息                       │
├─────────────────────────────────────┤
│ 5. Ambiguity Detection               │
│    识别歧义，生成澄清问题             │
└──────────────┬──────────────────────┘
               │
               ▼
          Spec IR (标准化中间表示)
          所有下游操作的统一输入
          Plan / PRD / Code / Test 共享同一术语体系
```

Spec IR 类似编译器的 LLVM IR——它是所有下游操作的统一输入。编译一次的成本（几百 token）远小于走错路再回头的成本（几千甚至几万 token）。

---

## 5. Multi-Agent Runtime

### 5.1 角色架构

```
Main Agent (最强推理模型)
├── 接收用户需求 + 全部对话上下文
├── 查询 Intelligence Core 制定 Plan
├── 生成 Sub-agent 任务指令 (MessagePack)
├── 审计 Sub-agent 返回结果
│
├── Sub-agent: Worker (执行具体编码/修改)
├── Sub-agent: Explorer (搜索/研究/信息收集)
│
└── Cage Sub-agent (独立监控者)
    ├── 不介入业务，只做规范检查
    ├── 监控：目标偏移、推理深度、偷懒、规则违反
    └── 可用轻量模型或规则引擎实现（token-efficient）
```

### 5.2 Provider 适配策略（Model Dispatch Policy）

```
多模型 Provider (如 OpenAI, DashScope):
  Main Agent  → 最强推理模型 (opus/o3/qwen-max)
  Worker      → 平衡模型 (sonnet/4o/qwen-plus)
  Explorer    → 快速模型 (haiku/4o-mini/qwen-turbo)
  Cage Agent  → 轻量模型或规则引擎

单模型 Provider (如本地 Llama):
  Main Agent  → 唯一模型, max reasoning depth
  Worker      → 同模型, 限制 max_tokens + temperature
  Explorer    → 同模型, 限制 max_tokens
  Cage Agent  → 同模型, 极简 system prompt + 规则检查
```

Main Agent 在初始化时检测 Provider 的 capability matrix，自动制定 dispatch 策略。

### 5.3 Agent 间通信

```
人 ↔ Agent:    自然语言（用户友好）
Agent ↔ Agent: MessagePack 二进制（高信息密度、无歧义、token-efficient）
Agent ↔ Tool:  MessagePack / JSON Schema 验证的结构化调用

存储/传输层和表示层分离：
  数据层:   MessagePack binary (source of truth)
  Agent I/O: 反序列化后的结构化数据
  展示层:   Markdown (人类审阅时通过确定性程序渲染)
```

MVP 阶段消息结构用 JSON Schema 定义：

```json
{
  "$schema": "terra-tip/1.0",
  "from": "main",
  "to": "worker-1",
  "op": "fix_bug",
  "target": {"file": "src/auth.py", "line": 42},
  "constraints": ["pass_tests", "minimal_change"],
  "expect": ["diff", "test_result"],
  "budget": {"max_tokens": 2000}
}
```

---

## 6. Memory Database

**不是 MD 档案库，是完整的数据库系统。**

### 6.1 作用域维度（Scope）

```
S0: Global Scope (全局)
    Intelligence Core, Constitution, 用户偏好
    存储: Rust 内嵌 + terra-global.db
    生命周期: 永久

S1: Project Scope (项目级)
    项目架构决策、约定、已验证的模式
    存储: project/.terra/project.db (每项目独立 SQLite)
    生命周期: 项目存续期间

S2: Session Scope (会话级)
    当前工作会话的目标、上下文、进行中的决策
    存储: moka cache + session log file
    生命周期: 会话结束时，关键决策晋升到 S1

S3: Task Scope (任务级)
    具体任务的执行状态、Sub-agent 调度结果
    存储: moka cache only
    生命周期: 任务结束 → accepted path 晋升 S1，其余丢弃
```

### 6.2 语义维度（Level，源自 Caging Engineering Memory Middleware）

```
L0: Authoritative Sources ('static)
    Constitution, Intelligence Core, 锁定规则
    权限: 只读    生命周期: 永久

L1: Project Memory ('project)
    项目级知识: 架构决策、已知约束、历史决定
    权限: append-only / proposal-based    生命周期: 项目存续

L2: Task Memory ('task)
    当前任务状态: 假设、尝试、观察、错误、临时计划
    权限: 可写、可重置    生命周期: 任务结束后可回收

L3: Accepted Paths ('archive)
    验证通过的成功路径
    权限: append with verification evidence    生命周期: 归档

L4: Failed Path Index ('task)
    压缩的失败记录
    权限: append compressed    生命周期: 任务后压缩

L5: Dormant Ideas ('weak)
    未采纳但可能有用的想法
    权限: archive    生命周期: 随时可 GC

L6: Model Memory Candidates (review)
    反复验证的模式，候选晋升
    权限: 需人工审核    生命周期: 审核后决定
```

### 6.3 Rust 式记忆安全规则

```
Ownership:   每条记忆有且仅有一个 owner
Borrowing:   多 agent 可读，修改需独占锁
Lifetime:    低层级引用不能指向高层级生命周期（防悬垂指针）
No null:     不允许空引用，必须 Some(content) 或显式 None
No dangling: 解引用前检查 alive 状态和 version
GC:          引用计数(实时) + 完整性扫描(定期)
Versioning:  修改创建新版本，旧版本保留到显式 prune
```

### 6.4 三层存储架构

```
┌──────────────────────────────────────┐
│ S0: Intelligence Core                 │
│     Rust 内嵌 + terra-global.db      │
│     编译期确定，不走运行时数据库       │
└──────────────────┬───────────────────┘
                   │
┌──────────────────┼───────────────────┐
│ 缓冲层: moka Cache                   │
│   当前活跃项目的 Working Set          │
│   Attention score 驱动的 TTI          │
│   写入先落 cache，异步 flush          │
│                                       │
│   项目切换: flush dirty → 清空 →     │
│             预加载新项目 high-priority │
└──────────────────┬───────────────────┘
                   │ flush / load
┌──────────────────┼───────────────────┐
│ 持久层: SQLite                        │
│                                       │
│ terra-global.db (跨项目共享知识)       │
│                                       │
│ project-a/.terra/project.db (L1-L6)  │
│ project-b/.terra/project.db (L1-L6)  │
│ project-c/.terra/project.db (L1-L6)  │
│                                       │
│ 每个项目完全隔离                      │
└──────────────────────────────────────┘
```

### 6.5 查询路径

```
Agent 需要读记忆
    │
    ▼
Bloom Filter (纳秒) — 快速排除不存在的 key
    │
    ▼
moka Cache (纳秒) — 热数据命中
    │ miss
    ▼
Project SQLite (毫秒) — 项目级持久存储
    │ miss
    ▼
Global SQLite (毫秒) — 跨项目共享知识
    │ miss
    ▼
MemoryNotFound
```

### 6.6 Attention-based 检索（Multi-Head）

指针权重是**动态计算的**（不是静态标注），本质上就是 RAG 的语义检索：

```
当前上下文 → Embedding → Query (Q)
所有记忆描述 → Embedding → Keys (K)（预计算，存在向量索引中）
Score = softmax(Q · Kᵀ / √d)
Top-K → 加载到上下文

Multi-Head: Intelligence Core 的每个 Domain Module 贡献一个观察视角
  Head 1 (coding/):   代码质量视角
  Head 2 (algorithm/): 算法性能视角
  Head 3 (Universal):  历史经验视角
  Head 4 (Universal):  约束和规范视角
→ 多视角检索，合并去重 → 全面的上下文
```

### 6.7 Artifact 版本化存储

Plan/Spec/Test 不是记忆，是产物（Artifact），记忆系统存引用：

```
project/.terra/
├── project.db                  # S1 Project Memory
├── artifacts/
│   ├── plans/plan-001.msgpack  # Plan 版本 1
│   ├── specs/spec-ir-001.msgpack
│   ├── tests/test-run-001.msgpack
│   └── reviews/review-001.msgpack
├── sessions/
│   └── session-*.log
└── vectors/
    └── faiss.index
```

---

## 7. Command Protocol（预建命令程序）

Agent 不写数据库代码，只发出固定命令：

```
记忆:
  mem_write     {key, content, scope, level}
  mem_read      {key}
  mem_update    {key, content}
  mem_promote   {key, target_scope}
  mem_archive   {key}
  mem_search    {query, top_k, heads}

Artifact:
  artifact_save  {type, content}
  artifact_load  {type, version?}
  artifact_list  {type}

追踪:
  trace_event      {event_type, data}
  trace_checkpoint  {title, summary}

会话:
  session_start   {project, goal}
  session_switch  {project}
  session_end     {}
```

~15 条命令覆盖全部交互。每条命令经过：
1. 审计记录
2. Cage 权限检查
3. 执行
4. 结果记录

可审计 + 可回放。用户和 Agent 用同一套命令协议。

---

## 8. Sandbox（文件系统沙盒）

Agent 的文件操作受 SandboxGuard 约束：

```
项目目录内 (sandbox root):
  src/, tests/, Cargo.toml 等  → 可读写删除新增
  .terra/project.db            → 只能通过 Command Protocol 操作
  .terra/constitution.toml     → 只读（Meta-Cage 保护）

项目目录外:
  任何路径                      → 可读、可执行，不可写/删除/新增
```

所有文件 I/O 经过 SandboxGuard 检查。违规操作被拦截并记录到审计日志。

---

## 9. Caging Layer

CE 的 8 cage 映射到 Terra 的具体实现：

```
Specification Cage   → Language Compilation Layer + TaskSpec
Knowledge Cage       → Intelligence Core + authority ranking
Memory Cage          → Memory Database 的 Rust 式安全规则
Action Cage          → Skill Registry + Tool Protocol + Permissions + **SandboxGuard**
Verification Cage    → External Verifier Layer + Generate-Verify-Repair loop
Convergence Cage     → Path pruning + accepted path preservation
Parametric Mem Cage  → L6 promotion gate (需人工审核)
Meta-Cage            → Constitution + Cage Sub-agent 主动监控
```

---

## 9. Sleep Consolidation（睡眠沉淀机制）

会话结束 / 空闲时触发：
1. **清理**：去掉 S3 中无用的临时数据
2. **抽象**：具体操作 → 通用原则（"auth.py:42 加了 null check" → "认证模块入口需显式输入检查"）
3. **提名晋升**：S3→S1（值得保留的模式），S1→S0（反复验证的原则）
4. **用户确认**：候选清单交给用户选择保留/丢弃/稍后

---

## 10. 压缩保护机制

上下文使用率达 70% 时触发**主动外化**（不是被动压缩）：
1. 将 accepted paths 写入 Memory Database
2. 将当前 plan 状态写入 Artifact
3. 将关键决策和推理链写入 Memory Database
4. 更新指针表
5. 然后才允许压缩

---

## 11. 技术栈

### 核心

```
语言:         Rust (2021 edition)
异步运行时:    tokio
HTTP:         reqwest
LLM 调度:     async-openai (OpenAI-compatible)
序列化:       serde + rmp-serde (MessagePack) + serde_json + serde_yaml
数据库:       rusqlite (SQLite, WAL 模式)
缓存:         moka (高性能并发缓存, TTL/TTI, Attention score 驱动)
向量检索:      usearch 或 faiss-rs (FAISS 绑定)
Bloom Filter: bloom crate (快速存在性检查)
CLI:          clap
终端渲染:      ratatui 或 colored
日志/追踪:     tracing (结构化日志，对应 Terra Trace 层)
错误处理:      anyhow + thiserror
Embedding:    ort (ONNX Runtime, 本地运行 sentence-transformers)
测试:         内建 #[test] + 集成测试
```

### Skill 接口

```
Terra Core (Rust) ←→ MessagePack over stdio ←→ Skill (任何语言)
```

Python Skill (如 OfficeX) 通过 stdio 通信。可选 PyO3 绑定提供 `import terra`。

### 复用的外部成果

```
RAG 架构参考:    LlamaIndex (检索模式), Letta/MemGPT (记忆分层)
Embedding 模型:  all-MiniLM-L6-v2 (本地默认, 384维)
                 OpenAI embeddings (可选增强)
```

---

## 12. Rust 项目结构（Cargo Workspace）

```
terra/
├── Cargo.toml                      # workspace root
├── crates/
│   ├── terra-core/                 # Intelligence Core
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── reasoning.rs
│   │       ├── behavioral.rs
│   │       ├── diagnostic.rs
│   │       └── modules/            # Domain modules
│   │
│   ├── terra-memory/               # Memory Database
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── store.rs            # SQLite 存储
│   │       ├── index.rs            # 向量索引 (Attention/RAG)
│   │       ├── safety.rs           # Ownership, Borrowing, Lifetime
│   │       ├── pointer.rs          # 智能指针
│   │       ├── gc.rs               # 垃圾回收
│   │       ├── promotion.rs        # 层级晋升链
│   │       └── cache.rs            # moka 缓冲层
│   │
│   ├── terra-protocol/             # TIP 通信协议
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── message.rs          # 消息定义
│   │       ├── codec.rs            # MessagePack 编解码
│   │       ├── schema.rs           # 消息 schema 验证
│   │       ├── render.rs           # → Markdown 渲染
│   │       └── executor.rs         # Command Protocol 执行器
│   │
│   ├── terra-agents/               # Multi-Agent Runtime
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── orchestrator.rs     # Main Agent 调度
│   │       ├── worker.rs
│   │       ├── explorer.rs
│   │       └── cage.rs             # Cage Sub-agent
│   │
│   ├── terra-caging/               # Caging Engine
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── specification.rs
│   │       ├── knowledge.rs
│   │       ├── memory_cage.rs
│   │       ├── action.rs
│   │       ├── verification.rs
│   │       ├── convergence.rs
│   │       └── meta.rs
│   │
│   ├── terra-providers/            # LLM Provider Dispatch
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── adapter.rs          # Provider trait
│   │       ├── openai.rs           # OpenAI-compatible
│   │       ├── dispatch.rs         # 调度 + retry
│   │       └── capability.rs       # Capability matrix
│   │
│   ├── terra-compile/              # Language Compilation Layer
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── lexical.rs
│   │       ├── semantic.rs
│   │       ├── normalize.rs
│   │       └── spec_ir.rs
│   │
│   ├── terra-trace/                # Audit Trail
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── checkpoint.rs
│   │       └── indexer.rs
│   │
│   └── terra-cli/                  # CLI 入口
│       └── src/
│           └── main.rs
│
├── skills/
│   └── superpower/                 # 内建工程规范 Skill
│
├── tests/                          # 集成测试
├── docs/
│   ├── PRD.md
│   ├── TDD.md
│   └── BDD.md
├── CONSTITUTION.md
└── README.md
```

---

## 13. 数据流总览

```
User (自然语言)
    │
    ▼
Language Compilation Layer (JSP 式)
    │ Spec IR
    ▼
Main Agent (最强推理模型)
    │ 查询 Intelligence Core
    │ 制定 Plan
    │
    ├──→ Command Executor ──→ Memory Database
    │    (固定命令, ~15 tokens/cmd)  (Bloom → moka → SQLite → FAISS)
    │
    ├──→ Sub-agent Worker (MessagePack) ←──┐
    │    执行具体任务                        │ Cage Sub-agent
    │    MessagePack 返回结果                │ 持续监控
    │                                        │ 规范性 + 深度
    ├──→ Sub-agent Explorer (MessagePack) ←─┘
    │    搜索/研究
    │
    │    Main Agent 审计结果
    │    Verification Cage 外部验证
    ▼
Verified Output → render_to_markdown → User (自然语言)

空闲时:
Sleep Consolidation → 清理 + 抽象 + 提名晋升 → 用户确认 → 有序成长
```

---

## 14. 开发计划

### Phase 0: 项目骨架
- 创建 Cargo workspace
- CONSTITUTION.md
- CI (GitHub Actions)

### Phase 1: PRD / TDD / BDD
- PRD (中文)
- TDD (英文)
- BDD (英文)
- 从三者推导 Goal

### Phase 2: 核心模块（按依赖顺序）
1. `terra-protocol` — MessagePack + Command Protocol + render
2. `terra-memory` — Memory Database (Rust safety + SQLite + moka + FAISS)
3. `terra-core` — Intelligence Core
4. `terra-compile` — Language Compilation Layer
5. `terra-providers` — Provider Dispatch
6. `terra-caging` — 8 Cage
7. `terra-agents` — Multi-Agent Runtime
8. `terra-trace` — Audit Trail

### Phase 3: CLI + 集成测试 + Superpower Skill

### Phase 4: OfficeX 重构为 Terra Skill

---

## 15. 优化细节

| 优化项 | 技术 | 效果 |
|--------|------|------|
| SQLite WAL | `PRAGMA journal_mode=WAL` | 读写并发 |
| Lazy Loading | 指针只存 metadata，需要时才加载 content | 内存节约 |
| Bloom Filter | 查询前快速排除不存在的 key | 减少无效 I/O |
| Embedding 增量更新 | FAISS IndexIDMap 按 ID 增删 | 避免全量重建 |
| Copy-on-Write | 晋升时延迟复制，修改时才创建副本 | 减少复制开销 |
| Attention-driven TTI | moka cache 的 TTI 由 attention score 驱动 | 热数据更持久 |
| Multi-Head Retrieval | 多 Domain Module 视角并行检索 | 全面的上下文 |
