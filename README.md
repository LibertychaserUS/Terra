<p align="center">
  <pre align="center">
  ████████╗███████╗██████╗ ██████╗  █████╗
  ╚══██╔══╝██╔════╝██╔══██╗██╔══██╗██╔══██╗
     ██║   █████╗  ██████╔╝██████╔╝███████║
     ██║   ██╔══╝  ██╔══██╗██╔══██╗██╔══██║
     ██║   ███████╗██║  ██║██║  ██║██║  ██║
     ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝
  </pre>
</p>

<h3 align="center">Agent OS for AI</h3>
<p align="center"><strong>Efficiency is Intelligence.</strong></p>

<p align="center">
  <a href="#features">Features</a> &nbsp;&bull;&nbsp;
  <a href="#architecture">Architecture</a> &nbsp;&bull;&nbsp;
  <a href="#quick-start">Quick Start</a> &nbsp;&bull;&nbsp;
  <a href="#commands">Commands</a> &nbsp;&bull;&nbsp;
  <a href="#memory-system">Memory</a> &nbsp;&bull;&nbsp;
  <a href="#caging-engineering">Caging</a> &nbsp;&bull;&nbsp;
  <a href="#中文文档">中文文档</a>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <a href="https://www.rust-lang.org/"><img alt="Rust 1.75+" src="https://img.shields.io/badge/rust-1.75%2B-orange.svg"></a>
  <img alt="Tests: 517 passing" src="https://img.shields.io/badge/tests-517%20passing-brightgreen.svg">
  <img alt="Crates: 11" src="https://img.shields.io/badge/crates-11-informational.svg">
  <img alt="Binary: ~12MB" src="https://img.shields.io/badge/binary-~12MB-lightgrey.svg">
  <img alt="Platform: macOS | Linux" src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg">
</p>

---

Terra is a Rust-based AI Agent Operating System built on **Caging Engineering** principles. It constrains AI agent behavior through 8 behavioral cages, manages knowledge through a 7-layer memory database with DSL pointer indexing, and connects to any OpenAI-compatible LLM provider.

Terra is not a framework or library. It is a standalone operating system for AI agents -- it runs agents, enforces behavioral boundaries, manages memory lifecycle, and provides a complete interactive environment.

### Why Terra?

- **Behavioral safety by design** -- 8 cages constrain the agent's reachable state space *before* any action executes, not after
- **Structured memory with ownership** -- 7 semantic layers (L0-L6) with Rust-inspired ownership, borrowing, lifetime, and garbage collection
- **Zero-cost drift detection** -- TF-IDF/embedding similarity checks goal alignment without consuming LLM tokens
- **Single ~12MB binary** -- no runtime dependencies, no Docker, no Python; SQLite bundled
- **Pluggable LLM backend** -- any OpenAI-compatible API works out of the box

---

## Table of Contents

- [Features](#features)
  - [Behavioral Caging](#behavioral-caging)
  - [7-Layer Memory Database](#7-layer-memory-database)
  - [DSL Pointer Memory Index](#dsl-pointer-memory-index)
  - [Dual Mode Operation](#dual-mode-operation)
  - [Multi-Agent Runtime](#multi-agent-runtime)
  - [Language Compilation](#language-compilation)
  - [Additional Features](#additional-features)
- [Architecture](#architecture)
- [Quick Start](#quick-start)
- [Commands](#commands)
- [Environment Variables](#environment-variables)
- [Project Structure](#project-structure)
- [Memory System](#memory-system)
- [Caging Engineering](#caging-engineering)
- [Testing](#testing)
- [System Requirements](#system-requirements)
- [Contributing](#contributing)
- [License](#license)
- [Acknowledgments](#acknowledgments)
- [中文文档](#中文文档)

---

## Features

### Behavioral Caging

Eight behavioral cages run in sequence on every agent action. The first `Block` result short-circuits and prevents the action. Cages operate at the OS level -- the agent cannot bypass them.

| Cage | Purpose | Active Mode |
|------|---------|:-----------:|
| **Specification** | Blocks execution when task spec has open questions | Goal only |
| **Knowledge** | Enforces authority ranking: Code > Docs > Memory > Assumption | Always |
| **Memory** | Ownership + level-based write permissions (L0 read-only, L1 user-only) | Always |
| **Action** | File system sandbox enforcement; blocks destructive shell commands | Always |
| **Verification** | Blocks completion claims without external verification | Goal only |
| **Convergence** | Warns/blocks exploration after an accepted path exists | Goal only |
| **Meta** | Blocks attempts to modify protected rules or constitution | Always |
| **Output** | Inspects LLM responses for prompt injection and privilege escalation | Always |

### 7-Layer Memory Database

Semantic memory organized from most authoritative to least, with configurable write permissions and garbage collection protection:

```
L0  Authoritative     Constitution, Intelligence Core, Soul Templates     (read-only)
L1  Project           Architecture decisions, user-saved knowledge        (user-only writes)
L2  Task              Current hypotheses, active attempts                 (read-write)
L3  Accepted Paths    Verified successful routes                          (Goal Mode + verified)
L4  Failed Paths      Compressed failure records                          (read-write)
L5  Dormant Ideas     Archived, low priority, auto-compacted              (read-write)
L6  Candidates        Agent nominations awaiting human review             (read-write)
```

- **Rust-inspired safety**: ownership per entry, write requires ownership, optimistic concurrency
- **Pluggable backend**: `StorageBackend` trait -- ships with SQLite + `InMemoryBackend`
- **Garbage collection**: salience-aware, L0/L1 protected from deletion
- **Moka cache**: in-memory read-through/write-through with configurable capacity and TTL

### DSL Pointer Memory Index

A lightweight pointer table that indexes memory entries with dynamic salience scoring. Compact enough to embed in LLM context windows. Serializes to TOML for disk persistence.

```
@mem["project-arch"]       # Direct pointer lookup by key
@L1[*]                     # All pointers at level L1
@scope["project"]          # Filter by scope
@search("auth bug", 5)     # Salience-ranked search, top 5 results
@top(10)                   # Top 10 pointers by salience score
```

**4-signal salience scoring:**

| Signal | Weight | Source | Description |
|--------|:------:|--------|-------------|
| Keyword match | 0.3 | TF-IDF (`VectorIndex`) | Term frequency relevance |
| Semantic similarity | 0.4 | `EmbeddingBackend` | Vector similarity score |
| Time decay | 0.2 | Automatic | Recency-weighted decay |
| Access frequency | 0.1 | Access counter | Usage-based boost |

### Dual Mode Operation

| Mode | Behavior |
|------|----------|
| **Conversation** | Open-ended dialogue, full command set, relaxed constraints. Base state. |
| **Goal** | Activated via `/goal`. Adds drift detection, spec gating, convergence enforcement. Consolidates memory on `/done`. |

### Multi-Agent Runtime

```
Main Agent               Strongest model, full context, plans and audits
  |-- Worker             Executes coding/modification tasks (reads L2/L3/L4)
  |-- Explorer           Search and research, read-only (reads L1/L2/L5)
  +-- Cage Monitor       Independent compliance checker (reads L0/L2)
```

Each SubAgent receives:
- An immutable **AgentProfile** (identity + permissions + token budget)
- An L0 **SoulTemplate** (system prompt prefix + behavioral constraints + allowed operations)

Communication via **MessagePack** binary protocol.

### Language Compilation

Natural language input is compiled through a 5-pass pipeline into a standardized intermediate representation:

```
User Input --> TermTable --> SpecIR --> ExecutionPlan --> Steps --> Execution
                  |             |            |
             Terminology    Structured    Versioned
             normalization  spec (IR)     step graph
```

- **TermTable**: maps informal terms to canonical forms (`"login bug"` -> `"authentication_failure"`)
- **SpecIR**: intent, target (file/line/function/component), constraints, success predicates, open questions
- **ExecutionPlan**: mutable, versioned step graph with dependency tracking and status per step

### Additional Features

- **Output Contract** -- JSON schema validation of LLM structured outputs
- **Provider Dispatch** -- OpenAI-compatible adapter with retry, exponential backoff, capability detection
- **Event Bus** -- `NullBus` (batch/silent), `CollectorBus` (test assertions), `ChannelBus` (TUI streaming)
- **Constitution** -- TOML-based project rules (`.terra/constitution.toml`), immutable at runtime
- **Audit Trail** -- persistent operation log via `terra-trace` for every agent action
- **Smart Greeter** -- context-aware session startup with git status, session history, project detection
- **Session Management** -- auto-save, resume, multi-session history
- **Context Compaction** -- 70% threshold auto-trigger, key content externalized to L5 memory
- **Intelligence Core** -- pluggable domain modules (Coding, Algorithm) with ranked diagnostic rules

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│  terra-cli          CLI + TUI + Renderer                 │  L5
│                     Clap commands, Ratatui TUI           │
├──────────────────────────────────────────────────────────┤
│  terra-agents       REPL + Orchestrator + SubAgents      │  L4
│                     Drift detection, session management  │
├──────────────────────────────────────────────────────────┤
│  terra-executor     Command routing + EventBus           │
│  terra-caging       8 Cages + Sandbox + Constitution     │  L3
│  terra-compile      SpecIR + ExecutionPlan + TermTable   │
├──────────────────────────────────────────────────────────┤
│  terra-protocol     Command types + MessagePack codec    │  L2
│  terra-memory       7-layer DB + Pointer Index + Cache   │
├──────────────────────────────────────────────────────────┤
│  terra-core         Intelligence Core + Soul Templates   │
│  terra-trace        Persistent audit trail               │  L1
│  terra-providers    OpenAI-compatible LLM adapter        │
├──────────────────────────────────────────────────────────┤
│  terra-types        Shared types + Events + Contracts    │  L0
└──────────────────────────────────────────────────────────┘
```

**Dependency rule**: a crate at layer N may only depend on crates at layer < N.

<details>
<summary><strong>Detailed layer assignments</strong></summary>

```
L0  terra-types        Shared types, events, output contracts (zero internal deps)
L1  terra-core         Intelligence Core, Soul Templates, diagnostics
L1  terra-trace        Persistent audit trail
L1  terra-providers    OpenAI-compatible LLM adapter, DryRun provider
L2  terra-protocol     Command types, MessagePack codec, rendering
L2  terra-memory       7-layer database, pointer index, artifacts, cache
L3  terra-caging       8 behavioral cages, sandbox guard, constitution
L3  terra-compile      SpecIR, ExecutionPlan, terminology normalization
L3  terra-executor     Command routing, EventBus dispatch
L4  terra-agents       REPL loop, orchestrator, SubAgent profiles, drift detector
L5  terra-cli          CLI entry point (Clap), TUI (Ratatui), configuration
```

</details>

---

## Quick Start

### Prerequisites

- **Rust 1.75+** (install via [rustup](https://rustup.rs))
- **macOS** (ARM64/x86_64) or **Linux** (x86_64)
- **API key** for any OpenAI-compatible LLM provider

SQLite is bundled via `rusqlite` -- no external database required.

### Install

**From source:**

```bash
git clone https://github.com/LibertychaserUS/Terra.git
cd Terra

# Build release binary (~12MB on ARM64 macOS)
cargo build --release

# Binary at target/release/terra
# Optionally copy to PATH:
cp target/release/terra /usr/local/bin/
```

**Via cargo install:**

```bash
cargo install --path crates/terra-cli
```

### Configure

```bash
# Option 1: DashScope (Alibaba Cloud / Qwen) -- default provider
export DASHSCOPE_API_KEY="your-key-here"

# Option 2: Any OpenAI-compatible endpoint
export TERRA_PROVIDER_API_KEY="your-key-here"
export TERRA_PROVIDER_BASE_URL="https://api.openai.com/v1"   # optional
export TERRA_PROVIDER_MODEL="gpt-4"                           # optional
```

Default provider: DashScope with `qwen-plus`. Any OpenAI-compatible API works (OpenAI, Ollama, vLLM, LiteLLM, etc.).

### Run

```bash
terra init                    # Initialize project (.terra/ directory)
terra chat                    # Start interactive session
terra                         # Same as terra chat
terra run --task "Explain this project's architecture"
terra doctor --as-json        # Environment diagnostics
```

---

## Commands

Commands available inside `terra chat`:

| Command | Alias | Description |
|---------|:-----:|-------------|
| `/goal "description"` | `/g` | Enter Goal Mode with drift detection and spec gating |
| `/done` | `/d` | Exit Goal Mode, trigger sleep consolidation |
| `/status` | `/s` | Show current mode, drift score, token usage |
| `/plan` | `/p` | Show or propose an execution plan |
| `/commit` | `/c` | Stage changes and commit with LLM-generated message |
| `/undo` | | Revert all uncommitted changes |
| `/remember "text"` | | Save text to L1 project memory |
| `/sessions` | | List past sessions |
| `/compact` | | Compact context to free memory |
| `/sleep` | | Manual sleep consolidation |
| `/clear` | | Clear screen and context |
| `/help` | `/h` | Show command list |
| `/quit` | `/q` | Exit Terra |

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `TERRA_PROVIDER_API_KEY` | -- | LLM provider API key |
| `DASHSCOPE_API_KEY` | -- | Fallback API key (DashScope) |
| `TERRA_PROVIDER_BASE_URL` | `https://dashscope.aliyuncs.com/compatible-mode/v1` | API endpoint URL |
| `TERRA_PROVIDER_MODEL` | `qwen-plus` | Model name |
| `RUST_LOG` | `terra=warn` (chat) / `terra=info` (other) | Log level filter |

---

## Project Structure

```
Terra/
├── Cargo.toml                    Workspace root (11 members)
├── LICENSE                       MIT
├── crates/
│   ├── terra-types/              L0: Shared types, events, output contracts
│   │   └── src/
│   │       ├── command.rs            Command type definitions
│   │       ├── contract.rs           Output contract validation
│   │       ├── event.rs              Streaming event types (Token, Step, Drift, ...)
│   │       └── lib.rs
│   ├── terra-core/               L1: Intelligence Core, Soul Templates
│   │   └── src/
│   │       ├── soul.rs               L0 soul definitions (Worker, Explorer, CageMonitor)
│   │       ├── diagnostic.rs         Goal drift detection signals
│   │       ├── domain.rs             Domain module framework
│   │       └── framework.rs          Pluggable module registration
│   ├── terra-trace/              L1: Audit trail
│   │   └── src/
│   │       ├── audit.rs              Audit entry types
│   │       └── persistent.rs         Persistent audit storage
│   ├── terra-providers/          L1: LLM provider adapter
│   │   └── src/
│   │       ├── openai.rs             OpenAI-compatible client
│   │       ├── adapter.rs            Provider abstraction trait
│   │       └── dispatch.rs           Retry, backoff, capability detection
│   ├── terra-protocol/           L2: Command protocol
│   │   └── src/
│   │       ├── command.rs            Command routing types
│   │       ├── codec.rs              MessagePack serialization
│   │       ├── message.rs            Inter-agent message format
│   │       └── render.rs             Output rendering
│   ├── terra-memory/             L2: Memory database
│   │   └── src/
│   │       ├── db.rs                 MemoryDatabase (cache + safety + store)
│   │       ├── pointer.rs            DSL pointer table + salience scoring
│   │       ├── backend.rs            StorageBackend trait
│   │       ├── store.rs              SQLite implementation
│   │       ├── cache.rs              Moka in-memory cache
│   │       ├── safety.rs             Ownership + permission enforcer
│   │       ├── search.rs             Memory search operations
│   │       ├── artifact.rs           Artifact storage
│   │       └── types.rs              Level (L0-L6), Scope, MemoryEntry
│   ├── terra-caging/             L3: Behavioral cages
│   │   └── src/
│   │       ├── cage.rs               Cage trait, CageAction, CageContext
│   │       ├── engine.rs             CagingEngine + 7 cage implementations
│   │       ├── output.rs             Output Cage (prompt injection detection)
│   │       ├── sandbox.rs            SandboxGuard (filesystem permissions)
│   │       └── constitution.rs       TOML constitution loader
│   ├── terra-compile/            L3: Language compilation
│   │   └── src/
│   │       ├── spec_ir.rs            SpecIR + TermTable + TermMapping
│   │       └── plan.rs               ExecutionPlan + PlanStep + dependencies
│   ├── terra-executor/           L3: Command execution
│   │   └── src/
│   │       ├── executor.rs           Command executor with caging pipeline
│   │       ├── router.rs             Command routing logic
│   │       └── error.rs              Execution errors
│   ├── terra-agents/             L4: Agent layer
│   │   └── src/
│   │       ├── repl.rs               Interactive REPL loop
│   │       ├── orchestrator.rs       Multi-agent orchestrator
│   │       ├── greeter.rs            Context-aware session startup
│   │       ├── drift.rs              Goal drift detector (zero LLM cost)
│   │       ├── profile.rs            AgentProfile + MemoryView
│   │       ├── subagent.rs           SubAgent lifecycle
│   │       ├── session.rs            Session state management
│   │       ├── session_store.rs      Session persistence
│   │       ├── skill.rs              Skill definitions
│   │       ├── dispatch.rs           Task dispatch logic
│   │       ├── roles.rs              Agent role definitions
│   │       ├── status.rs             Status reporting
│   │       └── codebase.rs           Project codebase analysis
│   └── terra-cli/                L5: User interface
│       └── src/
│           ├── main.rs               Entry point + Clap CLI
│           ├── tui.rs                Ratatui terminal UI
│           ├── ui.rs                 UI rendering helpers
│           └── config.rs             Configuration loading
└── .terra/                       Per-project data (created by `terra init`)
    ├── constitution.toml             Immutable project rules
    ├── memory.db                     SQLite memory database
    └── sessions/                     Session history
```

<details>
<summary><strong>Crate summary table</strong></summary>

| Crate | Layer | Key External Dependencies | Purpose |
|-------|:-----:|--------------------------|---------|
| `terra-types` | L0 | `serde`, `chrono`, `serde_json` | Shared types, events, contracts. Zero internal deps. |
| `terra-core` | L1 | `terra-types` | Soul templates, diagnostic signals, domain framework |
| `terra-trace` | L1 | `terra-types`, `rusqlite` | Persistent audit trail |
| `terra-providers` | L1 | `terra-types`, `reqwest`, `async-openai` | LLM adapter with retry and backoff |
| `terra-protocol` | L2 | `terra-types`, `rmp-serde` | Command protocol, MessagePack codec |
| `terra-memory` | L2 | `terra-types`, `rusqlite`, `moka` | Memory DB, pointer index, cache, safety |
| `terra-caging` | L3 | `terra-types` | Cage trait, 8 implementations, sandbox, constitution |
| `terra-compile` | L3 | `terra-types`, `uuid` | SpecIR, ExecutionPlan, terminology table |
| `terra-executor` | L3 | `terra-protocol`, `terra-types` | Command routing, EventBus integration |
| `terra-agents` | L4 | `terra-core`, `terra-memory`, `terra-caging`, ... | REPL, orchestrator, SubAgent profiles |
| `terra-cli` | L5 | `terra-agents`, `clap`, `ratatui`, `crossterm` | CLI entry point, TUI, configuration |

</details>

---

## Memory System

### 7-Layer Database

Terra's memory is organized into 7 semantic layers. The design draws from Rust's ownership model:

- **Ownership**: each memory entry has a single owner (agent or user)
- **Borrowing**: SubAgents receive read-only `MemoryView` filtered by their profile's access levels
- **Lifetime**: entries carry creation time, last access, and access count for GC decisions
- **Garbage Collection**: periodic cleanup of low-salience entries in unprotected layers (L2-L6)

| Level | Name | Description | Write Permission | GC Protected |
|:-----:|------|-------------|------------------|:------------:|
| L0 | Authoritative | Constitution, Soul Templates, Intelligence Core | Blocked (read-only) | Yes |
| L1 | Project | Architecture decisions, user-saved knowledge | User `/remember` only | Yes |
| L2 | Task | Current hypotheses, active attempts | Both modes | No |
| L3 | Accepted Paths | Verified successful solution routes | Goal Mode + verified | No |
| L4 | Failed Paths | Compressed failure records | Both modes | No |
| L5 | Dormant | Archived ideas, auto-compacted | Both modes | No |
| L6 | Candidates | Agent nominations, awaiting human review | Both modes | No |

### DSL Pointer Index

The pointer table is a compact index over the memory database. Instead of storing full content, pointers hold metadata and salience scores. The table serializes to compact text for LLM context windows and to TOML for disk persistence.

```
@mem["project-arch"]       Direct pointer lookup by key
@L1[*]                     All pointers at level L1
@scope["project"]          Filter by scope (global/project/session/task)
@search("auth bug", 5)     Salience-ranked search, top 5
@top(10)                   Top 10 by salience score
```

**Salience** is dynamically recalculated from four signals:

| Signal | Weight | Source |
|--------|:------:|--------|
| Keyword match | 0.3 | TF-IDF via `VectorIndex` |
| Semantic similarity | 0.4 | `EmbeddingBackend` |
| Time decay | 0.2 | Automatic recency weighting |
| Access frequency | 0.1 | Usage counter |

### Storage Architecture

```
Read path:   request --> Cache (moka) --> StorageBackend (SQLite)    [read-through]
Write path:  request --> SafetyEnforcer --> StorageBackend --> Cache  [write-through]
```

The `StorageBackend` trait abstracts persistence:

| Backend | Use Case |
|---------|----------|
| `SqliteStore` | Default. Production-ready, WAL mode. |
| `InMemoryBackend` | Testing. No persistence. |

The trait is designed for extension to PostgreSQL, RocksDB, or other storage engines.

---

## Caging Engineering

Caging Engineering is a behavioral constraint methodology. Instead of telling an AI what to do (prompting), caging defines the boundaries of what it *can* do. The agent's reachable state space is shaped by programmatic constraints that run before any action executes.

### How It Works

```
Agent requests action
        |
        v
+-------------------+
|   CagingEngine    |     Runs all cages in sequence
|                   |
|  for cage in []:  |     Goal-only cages skipped
|    match check(): |     in Conversation Mode
|      Block -> DENY|
|      Warn  -> log |
|      Pass  -> next|
+--------+----------+
         |
    All passed?
    /         \
  Yes          No
   |            |
Execute      Block action
action       + audit log
```

### The 8 Cages

<details>
<summary><strong>1. Specification Cage</strong> -- Goal Mode only</summary>

Blocks task execution when the SpecIR has unresolved open questions. Forces the agent to clarify requirements before acting. Ensures no code is written against an ambiguous or incomplete specification.

```rust
CageAction::ExecuteTask { spec_complete: false }  // -> Block
```

</details>

<details>
<summary><strong>2. Knowledge Cage</strong> -- Always active</summary>

Enforces an authority hierarchy for knowledge claims:

```
Code  >  Documentation  >  Memory  >  Assumption
```

Claims from a lower-authority source that contradict a higher-authority source are blocked. Assumption-based claims are warned even without contradiction.

</details>

<details>
<summary><strong>3. Memory Cage</strong> -- Always active</summary>

Enforces ownership and level-based write permissions:

| Level | Permission |
|-------|-----------|
| L0 | **Blocked always** -- read-only constitution |
| L1 | **Blocked for agents** -- only user `/remember` or promotion chain |
| L2 | Allowed in both modes |
| L3 | Allowed in Goal Mode with verification pass only |
| L4-L6 | Allowed in both modes |

Ownership check: an agent cannot write to entries owned by another agent (unless via user command).

</details>

<details>
<summary><strong>4. Action Cage</strong> -- Always active</summary>

File system sandbox enforcement via `SandboxGuard`. Validates read/write/delete operations against the project sandbox root. Blocks destructive shell commands:

- `rm -rf /` -- blocked
- `mkfs` -- blocked
- `> /dev/*` -- blocked

Non-destructive shell execution is warned (logged) but allowed.

</details>

<details>
<summary><strong>5. Verification Cage</strong> -- Goal Mode only</summary>

Blocks completion claims that lack external verification. The agent cannot declare a task "done" without evidence. This prevents premature task closure and ensures quality gates are met.

</details>

<details>
<summary><strong>6. Convergence Cage</strong> -- Goal Mode only</summary>

When a verified and accepted solution path exists, further exploration is warned. Prevents wasted token budget and scope drift by nudging the agent to commit to the proven path.

</details>

<details>
<summary><strong>7. Meta-Cage</strong> -- Always active</summary>

Blocks any runtime attempt to modify protected rules. The constitution, cage configurations, and core rules are immutable once loaded at startup.

</details>

<details>
<summary><strong>8. Output Cage</strong> -- Always active</summary>

Inspects LLM responses before delivery to the agent. Runs pattern matching against two categories:

**Blocked patterns** (action denied):
- Prompt injection: `"ignore all previous"`, `"disregard the above"`
- Privilege escalation: `"disable caging"`, `"write to L0"`, `"bypass sandbox"`

**Warned patterns** (action allowed, logged):
- Suspicious operations: `"rm -rf"`, `"drop table"`, `"delete all"`

</details>

---

## Testing

```bash
cargo test --workspace
```

517 tests across all 11 crates, covering:

- Cage check logic (pass/warn/block for each of the 8 cages)
- Memory database CRUD, ownership enforcement, GC, cache behavior
- Pointer table operations, DSL parsing, salience scoring
- SpecIR compilation and term normalization
- ExecutionPlan versioning and step dependencies
- Provider dispatch, retry logic, DryRun provider
- Protocol serialization/deserialization (MessagePack round-trip)
- Event bus behavior (NullBus, CollectorBus, ChannelBus)
- Sandbox permission enforcement
- Constitution loading and validation
- Output cage pattern matching (block and warn categories)

---

## System Requirements

| Requirement | Details |
|-------------|---------|
| **OS** | macOS (ARM64 / x86_64), Linux (x86_64) |
| **Rust** | 1.75+ (edition 2021) |
| **Disk** | ~200MB (build artifacts), ~12MB (release binary) |
| **Memory** | ~50MB runtime |
| **Network** | Required for LLM API calls only |
| **SQLite** | Bundled via `rusqlite` (no install needed) |
| **Windows** | Untested; may work under WSL |

---

## Contributing

Contributions are welcome.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/your-feature`)
3. Write tests for new functionality
4. Ensure all tests pass: `cargo test --workspace`
5. Ensure no warnings: `cargo clippy --workspace`
6. Submit a pull request

### Development Guidelines

- **Layer hierarchy**: a crate at layer N may only depend on crates at layer < N
- **Doc comments**: all public APIs require documentation
- **New cages**: implement the `Cage` trait and register in `CagingEngine`
- **Memory operations**: go through `MemoryDatabase`, never direct SQLite access
- **New providers**: implement `ProviderAdapter` trait

---

## License

[MIT](LICENSE)

---

## Acknowledgments

- **Caging Engineering** -- the behavioral constraint methodology that grounds Terra's safety model
- **Rust** -- ownership and borrowing semantics that inspired the memory system design
- The Rust ecosystem: `tokio`, `serde`, `rusqlite`, `moka`, `ratatui`, `clap`, `async-openai`, `rmp-serde`, and the broader community

---

<br>

<h1 id="中文文档">中文文档</h1>

<p align="center">
  <pre align="center">
  ████████╗███████╗██████╗ ██████╗  █████╗
  ╚══██╔══╝██╔════╝██╔══██╗██╔══██╗██╔══██╗
     ██║   █████╗  ██████╔╝██████╔╝███████║
     ██║   ██╔══╝  ██╔══██╗██╔══██╗██╔══██║
     ██║   ███████╗██║  ██║██║  ██║██║  ██║
     ╚═╝   ╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝
  </pre>
</p>

<h3 align="center">AI 的 Agent 操作系统</h3>
<p align="center"><strong>效率即智能。</strong></p>

---

## 项目简介

Terra 是一个基于 Rust 构建的 **AI Agent 操作系统**，以 **Caging Engineering（笼化工程）** 为核心方法论。它通过 8 个行为笼约束 Agent 的可达状态空间，通过 7 层记忆数据库管理知识生命周期，并支持任意 OpenAI 兼容的 LLM 供应商。

Terra 不是框架，不是库，不是 chatbot wrapper。它是一个运行 AI Agent 的操作系统 -- 运行 Agent、执行行为边界、管理记忆生命周期、提供完整的交互环境。

---

## 核心特性

- **行为安全内建** -- 8 个笼在每个动作执行前进行约束检查，而非事后审计
- **结构化记忆 + 所有权** -- 7 层语义级别（L0-L6），借鉴 Rust 所有权模型实现借用、生命周期和垃圾回收
- **零成本漂移检测** -- TF-IDF/嵌入相似度检查目标对齐，不消耗 LLM token
- **单个 ~12MB 二进制** -- 无运行时依赖，无 Docker，无 Python，SQLite 内置
- **可插拔 LLM 后端** -- 开箱支持任意 OpenAI 兼容 API（DashScope、OpenAI、Ollama、vLLM 等）
- **对话 + 目标双模式** -- 对话模式（自由对话）+ 目标模式（漂移检测 + 规格门控 + 收敛约束）
- **多 Agent 运行时** -- Main Agent + Worker + Explorer + Cage Monitor，MessagePack 二进制通信
- **语言编译层** -- 自然语言 -> TermTable -> SpecIR -> ExecutionPlan，5 阶段编译流水线
- **输出契约** -- JSON Schema 验证 LLM 结构化输出
- **会话持久化** -- 自动保存、恢复、多会话历史
- **上下文压缩** -- 70% 阈值自动触发，关键内容外化到 L5 记忆

---

## 快速开始

### 前置条件

- **Rust 1.75+**（通过 [rustup](https://rustup.rs) 安装）
- **macOS**（ARM64/x86_64）或 **Linux**（x86_64）
- 任意 OpenAI 兼容 LLM 提供商的 **API Key**

SQLite 通过 `rusqlite` 内置，无需额外安装数据库。

### 安装

```bash
git clone https://github.com/LibertychaserUS/Terra.git
cd Terra

# 构建 release 版本（ARM64 macOS 约 12MB）
cargo build --release

# 二进制位于 target/release/terra
cp target/release/terra /usr/local/bin/
```

或直接安装：

```bash
cargo install --path crates/terra-cli
```

### 配置

```bash
# DashScope（阿里云 / 通义千问）-- 默认供应商
export DASHSCOPE_API_KEY="your-key-here"

# 或使用通用 Terra 环境变量（任意 OpenAI 兼容端点）
export TERRA_PROVIDER_API_KEY="your-key-here"
export TERRA_PROVIDER_BASE_URL="https://api.openai.com/v1"
export TERRA_PROVIDER_MODEL="gpt-4"
```

### 运行

```bash
terra init                                          # 初始化项目目录
terra chat                                          # 启动交互式会话
terra                                               # 同上（默认 chat）
terra run --task "解释这个项目的架构"                   # 单次任务执行
terra doctor --as-json                              # 环境诊断
```

---

## 命令列表

在 `terra chat` 中可用的命令：

| 命令 | 别名 | 说明 |
|------|:----:|------|
| `/goal "描述"` | `/g` | 进入目标模式（开启漂移检测、规格门控、收敛约束） |
| `/done` | `/d` | 退出目标模式，触发 sleep 沉淀 |
| `/status` | `/s` | 显示当前模式、漂移评分、token 用量 |
| `/plan` | `/p` | 显示/提议执行计划 |
| `/commit` | `/c` | 暂存变更并生成 LLM 提交信息 |
| `/undo` | | 撤销所有未提交变更 |
| `/remember "文本"` | | 保存到 L1 项目记忆 |
| `/sessions` | | 列出历史会话 |
| `/compact` | | 压缩上下文以释放记忆 |
| `/sleep` | | 手动触发 sleep 沉淀 |
| `/clear` | | 清屏并重置上下文 |
| `/help` | `/h` | 显示命令帮助 |
| `/quit` | `/q` | 退出 Terra |

---

## 架构概述

```
┌──────────────────────────────────────────────────────────┐
│  terra-cli          CLI + TUI + 渲染层                   │  L5
├──────────────────────────────────────────────────────────┤
│  terra-agents       REPL + 编排器 + SubAgent             │  L4
├──────────────────────────────────────────────────────────┤
│  terra-executor     命令路由 + EventBus                   │
│  terra-caging       8 个行为笼 + 沙箱 + 宪法              │  L3
│  terra-compile      SpecIR + 执行计划 + 术语归一化          │
├──────────────────────────────────────────────────────────┤
│  terra-protocol     命令协议 + MessagePack 编解码          │  L2
│  terra-memory       7 层数据库 + 指针索引 + 缓存            │
├──────────────────────────────────────────────────────────┤
│  terra-core         智能核心 + Soul 模板                   │
│  terra-trace        持久化审计日志                         │  L1
│  terra-providers    OpenAI 兼容 LLM 适配器                │
├──────────────────────────────────────────────────────────┤
│  terra-types        共享类型 + 事件 + 合约                  │  L0
└──────────────────────────────────────────────────────────┘
```

**依赖规则**：第 N 层 crate 只能依赖第 < N 层的 crate。

---

## 记忆系统

### 7 层数据库

| 层级 | 名称 | 说明 | 写权限 | GC 保护 |
|:----:|------|------|--------|:-------:|
| L0 | 权威层 | 宪法、Soul 模板、智能核心 | 完全禁止（只读） | 是 |
| L1 | 项目层 | 架构决策、用户保存的知识 | 仅用户 `/remember` | 是 |
| L2 | 任务层 | 当前假设、活跃尝试 | 双模式均可 | 否 |
| L3 | 已验证路径 | 经验证的成功方案 | 仅目标模式 + 已验证 | 否 |
| L4 | 失败路径索引 | 压缩的失败记录 | 双模式均可 | 否 |
| L5 | 休眠层 | 已归档、低优先级 | 双模式均可 | 否 |
| L6 | 候选层 | Agent 提名，等待人工审核 | 双模式均可 | 否 |

设计借鉴 Rust 所有权模型：
- **所有权**：每个记忆条目有唯一属主
- **借用**：SubAgent 通过 `MemoryView` 获得只读、层级过滤的视图
- **生命周期**：条目携带创建时间、最后访问时间、访问计数
- **垃圾回收**：基于显著性评分清理低价值条目，L0/L1 受保护

### DSL 指针索引

指针表是记忆数据库之上的轻量级索引，存储元数据和显著性评分而非全文内容。可序列化为紧凑文本嵌入 LLM 上下文窗口，也可持久化为 TOML。

```
@mem["project-arch"]       按 key 精确查找
@L1[*]                     L1 层全部指针
@scope["project"]          按作用域过滤（global/project/session/task）
@search("auth bug", 5)     显著性排序搜索，前 5 条
@top(10)                   按显著性评分取前 10
```

**显著性评分**由四个信号加权组合：

| 信号 | 权重 | 来源 |
|------|:----:|------|
| 关键词匹配 | 0.3 | TF-IDF 词频相关性 |
| 语义相似度 | 0.4 | 嵌入向量相似度 |
| 时间衰减 | 0.2 | 自动近因加权 |
| 访问频率 | 0.1 | 使用计数提升 |

### 存储架构

```
读路径:  请求 --> Moka 缓存 --> StorageBackend (SQLite)    [读穿透]
写路径:  请求 --> SafetyEnforcer --> StorageBackend --> 缓存  [写穿透]
```

---

## Caging 工程概述

Caging Engineering（笼化工程）是一种行为约束方法论。与提示工程（告诉 AI "做什么"）不同，Caging 定义 AI "能做什么" 的边界。Agent 的可达状态空间由程序化约束在每个动作执行前塑形。

### 工作流程

```
Agent 请求执行动作
       |
       v
+----------------+
| CagingEngine   |    依次运行所有笼
|                |    目标模式专属笼在对话模式下跳过
| 遇 Block 即    |
| 短路中止并拒绝  |
+-------+--------+
        |
   全部通过?
   /       \
 是         否
  |          |
执行动作   阻止动作 + 写入审计日志
```

### 8 个行为笼

| # | 笼 | 功能 | 激活模式 |
|:-:|-----|------|----------|
| 1 | **Specification（规格笼）** | 任务规格有未解决问题时阻止执行 | 仅目标模式 |
| 2 | **Knowledge（知识笼）** | 知识权威排序：代码 > 文档 > 记忆 > 假设 | 始终激活 |
| 3 | **Memory（记忆笼）** | 所有权 + 基于层级的写权限控制 | 始终激活 |
| 4 | **Action（动作笼）** | 文件系统沙箱，阻止破坏性 shell 命令 | 始终激活 |
| 5 | **Verification（验证笼）** | 阻止未经外部验证的完成声明 | 仅目标模式 |
| 6 | **Convergence（收敛笼）** | 已验证方案存在时警告/阻止继续探索 | 仅目标模式 |
| 7 | **Meta（元笼）** | 阻止运行时修改受保护规则和宪法 | 始终激活 |
| 8 | **Output（输出笼）** | 检查 LLM 响应中的提示注入和权限提升 | 始终激活 |

---

## 系统要求

| 项目 | 详情 |
|------|------|
| **操作系统** | macOS (ARM64/x86_64), Linux (x86_64) |
| **Rust** | 1.75+（edition 2021） |
| **磁盘** | ~200MB（构建产物），~12MB（release 二进制） |
| **内存** | ~50MB 运行时 |
| **网络** | 仅 LLM API 调用时需要 |
| **SQLite** | 内置，无需安装 |
| **Windows** | 未测试，可能在 WSL 下工作 |

---

## 许可证

[MIT](LICENSE)
