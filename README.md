<p align="center">
  <pre>
  ████████╗███████╗██████╗ ██████╗  █████╗
  ╚══██╔══╝██╔════╝██╔══██╗██╔══██╗██╔══██╗
     ██║   █████╗  ██████╔╝██████╔╝███████║
     ██║   ██╔══╝  ██╔══██╗██╔══██╗██╔══██║
     ██║   ███████╗██║  ██║██║  ██║██║  ██║
     ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝
  </pre>
</p>

<p align="center">
  <strong>Agent OS for AI — Efficiency is Intelligence.</strong>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> |
  <a href="#features">Features</a> |
  <a href="#architecture">Architecture</a> |
  <a href="#中文文档">中文文档</a>
</p>

<p align="center">
  <img alt="License" src="https://img.shields.io/badge/license-MIT-blue.svg">
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.75%2B-orange.svg">
  <img alt="Tests" src="https://img.shields.io/badge/tests-517%20passed-green.svg">
  <img alt="Crates" src="https://img.shields.io/badge/crates-11-purple.svg">
</p>

---

Terra is a Rust-based AI Agent Operating System inspired by **Caging Engineering**. It provides structured, auditable, and safe agent behavior through behavioral cages, layered memory with pointer indexing, and a pluggable provider architecture.

Unlike chatbot wrappers, Terra treats AI agents as first-class OS processes — with memory ownership, behavioral constraints, and audit trails.

## Features

### Behavioral Caging (9 Cages)

| Cage | Purpose |
|------|---------|
| Specification | Validates task specs before execution |
| Knowledge | Authority ranking for information sources |
| Memory | Ownership, borrowing, and lifetime rules |
| Action | Sandboxed file/shell access with permissions |
| Verification | External verifier layer |
| Convergence | Path pruning + accepted path preservation |
| Parametric Memory | L6 promotion gate (human review required) |
| Output | Structured validation of LLM outputs |
| Meta | Constitution enforcement + Cage Monitor agent |

### 7-Layer Memory Database

```
L0: Authoritative    — Constitution, Intelligence Core (read-only)
L1: Project          — Architecture decisions, project knowledge (append-only)
L2: Task             — Current hypotheses, working state (read-write)
L3: Accepted Paths   — Verified successful routes (with evidence)
L4: Failed Paths     — Compressed failure records (index only)
L5: Dormant Ideas    — Archived, low priority (auto-compacted)
L6: Model Candidates — Needs human review before promotion
```

- **Rust-inspired safety**: ownership per entry, write requires ownership, optimistic concurrency
- **Pluggable backend**: `StorageBackend` trait — ships with SQLite + InMemoryBackend
- **Garbage collection**: ref-count aware, L0/L1 protected from deletion

### DSL Pointer Memory Index

A lightweight pointer table that indexes memory entries with dynamic salience scoring. Pointers work through tool calling (syscalls), not LLM text output.

```
@mem["auth_pattern"]       — exact key lookup
@L1[*]                     — all L1-level pointers
@scope["project"]          — scope filter
@search("query", 5)        — search by annotation, top N
@top(10)                   — top 10 by salience
```

**4-signal salience scoring:**
- Keyword match (0.3) + Semantic similarity (0.4) + Time decay (0.2) + Access frequency (0.1)

### Multi-Agent Runtime

```
Main Agent           — Strongest model, full context, plans and audits
  ├── Worker         — Executes coding/modification tasks
  ├── Explorer       — Search, research, information gathering
  └── Cage Monitor   — Independent compliance checker (lightweight model)
```

Communication via **MessagePack** binary protocol. Each agent has a **Soul** (identity + capabilities + memory view).

### Additional Features

- **Intelligence Core** — Pluggable domain modules (Coding, Algorithm) with ranked rules
- **5-Pass Compilation** — Lexical + Terminology + Semantic + Completeness + Ambiguity detection
- **Output Contract** — JSON Schema validation of LLM structured outputs
- **Provider Dispatch** — OpenAI-compatible adapter with retry, backoff, capability detection
- **Event Bus** — NullBus / CollectorBus / ChannelBus for operation visibility
- **Session Persistence** — Auto-save, resume, multi-session history
- **Context Compaction** — 70% threshold auto-trigger, externalize to L5 memory
- **Smart Greeter** — Context-aware startup (git status, session history, suggestions)
- **Conversation + Goal Mode** — Goal adds drift detection and spec gating over conversation base

## Quick Start

### Prerequisites

- **Rust** 1.75+ ([rustup.rs](https://rustup.rs))
- **macOS** or **Linux** (Windows: untested, may work with WSL)
- **API Key** for any OpenAI-compatible LLM provider

SQLite is bundled — no external database required.

### Install

```bash
git clone https://github.com/LibertychaserUS/Terra.git
cd Terra
cargo build --release

# Binary at target/release/terra (~12MB)
# Copy to PATH:
cp target/release/terra /usr/local/bin/
```

Or:

```bash
cargo install --path crates/terra-cli
```

### Configure

```bash
# Any OpenAI-compatible provider:
export TERRA_PROVIDER_API_KEY="your-key"
export TERRA_PROVIDER_BASE_URL="https://api.openai.com/v1"   # optional
export TERRA_PROVIDER_MODEL="gpt-4"                           # optional

# DashScope (default):
export DASHSCOPE_API_KEY="your-key"
```

### Run

```bash
terra init     # Initialize project (.terra/ directory)
terra chat     # Interactive dialogue
terra          # Same as terra chat
terra doctor   # Check environment
terra run --task "Explain this project's architecture"
```

## Commands

| Command | Alias | Description |
|---------|-------|-------------|
| `/goal "desc"` | `/g` | Enter Goal Mode (adds drift detection) |
| `/done` | `/d` | Exit Goal Mode, trigger sleep consolidation |
| `/status` | `/s` | Show mode, drift score, token usage |
| `/plan` | `/p` | Show/propose execution plan |
| `/commit` | `/c` | Stage + commit with LLM-generated message |
| `/undo` | | Revert all uncommitted changes |
| `/remember "x"` | | Save to L1 project memory |
| `/sessions` | | List past sessions |
| `/compact` | | Compact context (free memory) |
| `/sleep` | | Manual sleep consolidation |
| `/clear` | | Clear screen and context |
| `/help` | `/h` | Show command list |
| `/quit` | `/q` | Exit Terra |

## Architecture

```
┌─────────────────────────────────────────────────────┐
│  terra-cli        CLI + TUI + Renderer              │  L5
├─────────────────────────────────────────────────────┤
│  terra-agents     REPL + Orchestrator + SubAgents   │  L4
├─────────────────────────────────────────────────────┤
│  terra-executor   Command routing + Caging pipeline │  L3
│  terra-caging     9 Cages + Sandbox + Constitution  │
│  terra-compile    5-pass compilation + ExecutionPlan │
├─────────────────────────────────────────────────────┤
│  terra-protocol   MessagePack codec                 │  L2
│  terra-memory     7-layer DB + Pointer Index        │
├─────────────────────────────────────────────────────┤
│  terra-core       Intelligence Core                 │  L1
│  terra-trace      Audit trail                       │
│  terra-providers  LLM adapters                      │
├─────────────────────────────────────────────────────┤
│  terra-types      Shared types (zero deps)          │  L0
└─────────────────────────────────────────────────────┘
```

Strict layer hierarchy: lower layers never import higher layers.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `TERRA_PROVIDER_API_KEY` | — | LLM provider API key |
| `DASHSCOPE_API_KEY` | — | Fallback (DashScope) |
| `TERRA_PROVIDER_BASE_URL` | DashScope endpoint | API endpoint |
| `TERRA_PROVIDER_MODEL` | `qwen-plus` | Model name |
| `RUST_LOG` | `terra=warn` (chat) | Log level override |

## Testing

```bash
cargo test --workspace    # 517 tests, all passing
```

## System Requirements

| Requirement | Details |
|-------------|---------|
| OS | macOS (ARM64/x86_64), Linux (x86_64) |
| Rust | 1.75+ (2024 edition) |
| Disk | ~200MB (build), ~12MB (binary) |
| Memory | ~50MB runtime |
| Network | Required for LLM API calls only |
| SQLite | Bundled (no install needed) |

## License

MIT

---

# 中文文档

## 简介

Terra 是一个基于 Rust 的 **AI Agent 操作系统**，灵感来自 **Caging Engineering（笼式工程）**。它通过 9 个行为笼、7 层记忆数据库和 DSL 指针索引，为 AI Agent 提供结构化、可审计、安全的行为框架。

Terra 不是 chatbot wrapper。它把 AI Agent 当作操作系统中的一等进程——拥有记忆所有权、行为约束和审计追踪。

## 核心特性

- **9 个行为笼 (Caging)** — 规范笼、知识笼、记忆笼、行动笼、验证笼、收敛笼、参数记忆笼、输出笼、元笼。程序化约束 Agent 行为
- **7 层记忆数据库** — L0（权威层，只读）到 L6（候选层，需人工审核）。Rust 式所有权 + 借用 + 生命周期
- **DSL 指针记忆索引** — 轻量级指针表，动态 salience 评分（关键词 + 语义 + 时间衰减 + 频率）。通过 tool calling 访问，不污染 LLM 输出
- **可插拔存储后端** — `StorageBackend` trait 抽象持久层。默认 SQLite，可替换为 PostgreSQL/RocksDB
- **多 Agent 运行时** — Main Agent + Worker + Explorer + Cage Monitor，MessagePack 通信
- **5-pass 编译** — 词法 + 术语 + 语义 + 完备性 + 歧义检测
- **输出契约 (Output Contract)** — JSON Schema 验证 LLM 结构化输出
- **Provider 适配** — OpenAI 兼容接口 + 重试 + 退避 + 能力检测
- **会话持久化** — 自动保存、恢复、多会话历史
- **上下文压缩** — 70% 阈值自动触发，关键内容外化到 L5 记忆
- **对话 + 目标双模式** — 对话是基础状态，目标模式叠加漂移检测和规范门控

## 快速开始

```bash
# 安装
git clone https://github.com/LibertychaserUS/Terra.git
cd Terra && cargo build --release
cp target/release/terra /usr/local/bin/

# 配置 API Key（任何 OpenAI 兼容端点）
export DASHSCOPE_API_KEY="your-key"

# 运行
terra init     # 初始化项目
terra chat     # 交互对话
terra doctor   # 环境检查
```

## 系统要求

- macOS (ARM64/x86_64) 或 Linux (x86_64)
- Rust 1.75+
- 12MB 磁盘（二进制）
- SQLite 已内置，无需额外安装

## 记忆系统

```
L0: 权威层     — 宪法、Intelligence Core（只读）
L1: 项目层     — 架构决策、项目知识（只追加）
L2: 任务层     — 当前假设、工作状态（读写）
L3: 已验证路径 — 验证通过的成功路径（带证据）
L4: 失败索引   — 压缩的失败记录（索引）
L5: 休眠想法   — 归档，低优先级
L6: 候选层     — 需人工审核才能晋升
```

**DSL 指针索引**：`@mem["key"]`、`@L1[*]`、`@search("query", top_k=5)`、`@top(10)`

## 命令列表

| 命令 | 简写 | 说明 |
|------|------|------|
| `/goal "描述"` | `/g` | 进入目标模式 |
| `/done` | `/d` | 退出目标模式 |
| `/status` | `/s` | 查看状态 |
| `/plan` | `/p` | 查看/提议执行计划 |
| `/commit` | `/c` | Git 提交（LLM 生成消息） |
| `/remember "x"` | | 记住到 L1 |
| `/compact` | | 压缩上下文 |
| `/help` | `/h` | 帮助 |
| `/quit` | `/q` | 退出 |
