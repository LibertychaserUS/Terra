# Terra PRD（产品需求文档）

> 版本: 0.1.0 — MVP
> 最后更新: 2026-05-20
> 状态: 草案

---

## 1. 产品愿景

### 1.1 Terra 是什么

Terra 是一个 **Agent OS**——运行 AI Agent 的操作系统。

它不是聊天机器人，不是 LLM wrapper，不是又一个 Agent 框架。它是一个完整的运行时环境，让 AI Agent 在经过工程化设计的任务空间中**可靠、可控、可审计**地运行。

### 1.2 Terra 不是什么

- **不是** LangChain/CrewAI 那样的编排框架——那些是应用层，Terra 是操作系统层
- **不是** ChatGPT/Claude 那样的对话产品——Terra 管理 Agent，不替代 Agent
- **不是** 通用 AI 平台——MVP 聚焦 coding/软件工程场景

### 1.3 核心价值主张

> 可靠性不是模型的属性，而是**系统**的属性。

当前 AI Agent 的问题：
| 问题 | 根因 |
|------|------|
| Agent 漫无目的探索 | 没有结构化的任务空间 |
| 上下文污染、重复犯错 | 没有记忆纪律（Memory Cage） |
| 声称完成但未验证 | 没有外部验证机制 |
| 执行危险操作 | 没有权限边界和沙盒 |
| 静默修改自身规则 | 没有 cage integrity |
| 用户从零开始养成 | 没有内建的第一性原理 |
| Token 浪费严重 | 没有结构化通信协议 |

Terra 通过 **Caging Engineering** 的方法论，系统性地解决这些问题。

### 1.4 与 Caging Engineering 的关系

Caging Engineering（笼工程）是 Harness Engineering 的升级版，是**人机协作开发的 workflow 原则**。它同时治理：
- **开发过程**：我们如何构建 Terra
- **产品行为**：Terra 如何治理其内部的 Agent

### 1.5 理论基础

> Keep goals reachable, make unsafe states unreachable, and prune invalid paths after verification.

来源: [Caging Engineering v0.1.0](https://github.com/LibertychaserUS/caging-engineering)

---

## 2. 用户角色

### 2.1 主要用户：软件工程师

- 使用 Terra 运行 AI Agent 来辅助 coding
- 通过 CLI 与 Terra 交互
- 期望 Agent 行为可预测、可审计
- 不愿意从零养成 Agent，期望开箱即有基础能力

### 2.2 次要用户：Skill 开发者

- 在 Terra 上构建领域 Skill（如 OfficeX 文档操作）
- 通过 MANIFEST.yml 注册 Skill 能力
- 通过 MessagePack over stdio 与 Terra 通信

### 2.3 未来用户：Agent 运营者

- 配置和运行 Agent 完成更广泛的任务
- 管理 Provider、Skill、记忆等资源
- （MVP 不覆盖）

---

## 3. 核心功能

### 3.1 Intelligence Core（智能核心）

Agent 出厂即有的第一性原理——从"一"开始，不从"零"。

**Universal Layer（通用层，只读）：**
- Reasoning Framework：如何推理（深度优先、证据驱动、分解复杂问题）
- Behavioral Framework：如何行动（优雅代码、最小改动、安全默认）
- Self-Diagnostic Rules：如何自检（防目标偏移、防偷懒、防浅层推理）

**Domain Modules（领域模块，可插拔）：**
- `coding/`：编程原则、设计模式、重构规则
- `algorithm/`：复杂度分析、数据结构选型
- 未来可扩展：`document/`、`security/` 等

每个 Domain Module 注册自己的 Perspectives（观察视角）、Rules（行为规则）、Verifiers（验证器）。

### 3.2 Language Compilation Layer（语言编译层）

**问题**：用户自然语言术语不规范，导致 Agent 理解偏差。

**方案**：JSP 式编译——用户输入 → 术语规范化 → Spec IR（标准化中间表示）

编译流程：
1. Lexical Pass：提取关键实体和意图
2. Semantic Resolution：映射到项目上下文
3. Terminology Normalization：统一命名
4. Completeness Check：检测缺失信息
5. Ambiguity Detection：识别歧义

输出的 Spec IR 是所有下游操作的统一输入，确保 Plan / PRD / Code / Test 术语一致。

### 3.3 Multi-Agent Runtime（多智能体运行时）

**Main Agent（主 Agent）：**
- 使用当前可用的最强推理模型
- 接收用户需求 + 全部对话上下文
- 制定计划、分发任务、审计结果

**Sub-Agents（子 Agent）：**
- Worker：执行具体编码/修改
- Explorer：搜索/研究/信息收集
- 接收 Main Agent 的结构化指令（MessagePack），执行后返回结果

**Cage Sub-agent（笼子 Agent）：**
- 独立的监控者，不介入业务
- 监测 Main Agent 的规范性：目标偏移、推理深度、偷懒、规则违反
- 可用轻量模型或规则引擎实现

**Provider 适配：**
- 多模型 Provider → Main 用最强，Worker 用平衡，Explorer 用快速
- 单模型 Provider → 同模型不同参数（max_tokens, temperature）

### 3.4 Memory Database（记忆数据库）

不是 MD 档案库，是完整的**数据库系统**。

**作用域维度（Scope）：**
- S0 Global：Intelligence Core + Constitution + 用户偏好
- S1 Project：项目级知识（每项目独立 SQLite）
- S2 Session：当前会话上下文
- S3 Task：具体任务执行状态

**语义维度（Level）：**
- L0 Authoritative → L1 Project → L2 Task → L3 Accepted Paths → L4 Failed Index → L5 Dormant → L6 Model Candidates

**安全规则（Rust 式）：**
- Ownership：每条记忆有且仅有一个 owner
- Borrowing：多读单写
- Lifetime：低层级不引用高层级
- No null / No dangling：编译期保证

**三层存储：**
- L0 Rust 内嵌 + terra-global.db
- moka Cache（缓冲层，Attention score 驱动 TTI）
- Per-Project SQLite（持久层）

**检索机制：** Multi-Head Attention（每个 Domain Module 贡献一个观察视角）

### 3.5 Command Protocol（命令协议）

Agent 不写数据库代码。~15 条固定命令覆盖所有交互：
- mem_write / mem_read / mem_search / mem_promote / mem_archive
- artifact_save / artifact_load / artifact_list
- trace_event / trace_checkpoint
- session_start / session_switch / session_end

每条命令经过：审计记录 → Cage 权限检查 → 执行 → 结果记录。
可审计 + 可回放。用户和 Agent 用同一套协议。

### 3.6 Caging Layer（笼层）

CE 的 8 cage 实现：
- Specification Cage → Language Compilation + TaskSpec
- Knowledge Cage → Intelligence Core + authority ranking
- Memory Cage → Memory Database 安全规则
- Action Cage → Skill Registry + Tool Protocol + SandboxGuard
- Verification Cage → 外部验证 + Generate-Verify-Repair loop
- Convergence Cage → Path pruning + accepted path preservation
- Parametric Memory Cage → L6 promotion gate
- Meta-Cage → Constitution + Cage Sub-agent

### 3.7 Sandbox（沙盒）

- 项目目录内：可读写删除新增
- 项目目录外：只读 + 可执行
- .terra/ 内的 constitution 等：只读（Meta-Cage 保护）
- 所有文件 I/O 经过 SandboxGuard 检查

### 3.8 Sleep Consolidation（睡眠沉淀）

会话结束/空闲时：
1. 清理无用临时数据
2. 具体操作抽象为通用原则
3. 提名晋升（S3→S1, S1→S0）
4. 用户确认

### 3.9 通信协议

- 人 ↔ Agent：自然语言
- Agent ↔ Agent：MessagePack 二进制
- Agent ↔ Tool：MessagePack / JSON Schema
- 人类审阅：程序渲染为 Markdown
- Token efficiency 是一等公民

### 3.10 压缩保护

上下文使用率达 70% 时触发主动外化：
- Accepted paths / plan / 关键决策写入 Memory Database
- 更新指针表
- 然后才允许压缩

---

## 4. 非功能需求

| 需求 | 标准 |
|------|------|
| CLI-first | 所有命令支持 `--as-json` 机器可读输出 |
| Token efficiency | Agent 间通信用 MessagePack，不用自然语言 |
| 无外部服务硬依赖 | 可纯本地运行（embedding 用本地模型） |
| Provider 无关 | 任何 OpenAI 兼容端点 |
| 可审计 | 所有操作有审计日志，可回放 |
| 安全 | Sandbox 隔离，API key 仅从环境变量读取 |
| 可扩展 | 通过 Skill 扩展领域能力 |
| 优雅代码 | 代码质量是一等公民，不为完成而堆砌 |

---

## 5. MVP 范围

### 5.1 MVP1（已完成）：端到端路径

✅ User Task → Spec IR → Main Agent → Provider → Worker → Trace
✅ CLI: `terra run --task "..." [--as-json]`
✅ DryRun Provider
✅ 16 tests 通过

### 5.2 MVP2（当前目标）：核心能力

- [ ] 真实 Provider 接入（OpenAI-compatible）
- [ ] Memory Database 基础版（SQLite + moka + 基本 read/write/search）
- [ ] Intelligence Core 基础规则
- [ ] Sandbox 文件系统保护
- [ ] Config 系统（用户设置 + 环境识别）
- [ ] Superpower Skill 基础版

### 5.3 未来版本

- Language Compilation Layer 完整版（需 LLM 做语义解析）
- Cage Sub-agent 主动监控
- Sleep Consolidation
- Multi-Head Attention 检索
- Streaming 支持
- OfficeX 重构为 Terra Skill

---

## 6. 与竞品的差异

| 维度 | LangChain/CrewAI | Hermes/OpenClaw | Terra |
|------|-------------------|-----------------|-------|
| 定位 | Agent 框架 | 通用 Agent | Agent OS |
| 起点 | 从零搭建 | 从零养成 | 从"一"开始（Intelligence Core） |
| 记忆 | 外挂 vector store | 基本记忆 | Rust 式安全的 Memory Database |
| 验证 | 无 / 自我评判 | 基本 | 外部验证 + Cage Sub-agent |
| 治理 | 无 | 无 | Constitution + 8 Cage |
| Token | 不关注 | 不关注 | MessagePack + Command Protocol |
| 成长 | 不成长 | 无序成长 | 有序成长（Sleep + 用户确认） |
