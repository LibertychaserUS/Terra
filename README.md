# Terra

**Agent OS for AI — Efficiency is Intelligence.**

Terra is a Rust-based AI Agent Operating System inspired by Caging Engineering. It provides structured, auditable, and safe agent behavior through 9 behavioral cages, 7-layer memory with DSL pointer indexing, and a pluggable provider architecture.

## Core Features

- **9 Behavioral Cages** — Specification, Knowledge, Memory, Action, Verification, Convergence, Parametric Memory, Meta-Cage, and Output Cage constrain agent behavior programmatically
- **7-Layer Memory Database** — L0 (Authoritative) through L6 (Candidates) with ownership, borrowing, lifetime management, and garbage collection
- **DSL Pointer Memory Index** — Lightweight pointer table with dynamic salience scoring (keyword + semantic + time decay + frequency). Compact serialization for LLM context inclusion
- **Pluggable Storage Backend** — `StorageBackend` trait abstracts persistence. Ships with SQLite (default) and InMemoryBackend (testing). Swap in PostgreSQL or RocksDB
- **Output Contract** — Structured validation of LLM outputs via JSON schema enforcement
- **Multi-Agent Runtime** — Main Agent + Worker + Explorer + Cage Monitor with MessagePack communication
- **Provider Dispatch** — OpenAI-compatible API adapter with retry, backoff, and capability detection
- **Event Bus** — NullBus (batch) / CollectorBus (test) / ChannelBus (TUI) for operation visibility
- **Smart Greeter** — Context-aware session startup with project detection
- **Conversation + Goal Dual Mode** — Conversation as base state; Goal adds drift detection and spec gating

## Quick Start

### Prerequisites

- **Rust** 1.75+ (install via [rustup](https://rustup.rs))
- **macOS** or **Linux** (Windows: untested, may work with WSL)
- **API Key** for an OpenAI-compatible LLM provider

SQLite is bundled — no external database installation required.

### Install

```bash
# Clone
git clone https://github.com/LibertychaserUS/terra.git
cd terra

# Build
cargo build --release

# The binary is at target/release/terra
# Optionally copy to PATH:
cp target/release/terra /usr/local/bin/
```

Or install directly:

```bash
cargo install --path crates/terra-cli
```

### Configure

Set your LLM provider API key:

```bash
# DashScope (Alibaba Cloud / Qwen)
export DASHSCOPE_API_KEY="your-key-here"

# Or use the generic Terra env vars:
export TERRA_PROVIDER_API_KEY="your-key-here"
export TERRA_PROVIDER_BASE_URL="https://api.openai.com/v1"  # optional
export TERRA_PROVIDER_MODEL="gpt-4"  # optional
```

Default provider: DashScope (`qwen-plus`). Any OpenAI-compatible endpoint works.

### Run

```bash
# Initialize a project
terra init

# Start interactive dialogue
terra chat

# Or just run terra (defaults to chat)
terra

# One-shot task
terra run --task "Explain the architecture of this project"

# Check environment
terra doctor --as-json
```

## Commands (in `terra chat`)

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
terra-cli          CLI + TUI + Renderer
terra-agents       REPL, Orchestrator, Greeter, SubAgent profiles
terra-executor     Command routing + EventBus + Caging enforcement
terra-caging       9 Cages + Sandbox + Constitution
terra-core         Intelligence Core (Coding + Algorithm modules)
terra-memory       7-layer DB + StorageBackend + Pointer Index + Artifacts
terra-compile      SpecIR + ExecutionPlan + 5-pass compilation
terra-providers    OpenAI-compatible adapter + DryRun + Dispatch
terra-protocol     Command types + MessagePack serialization
terra-types        Shared types (L0 crate, no internal deps)
terra-trace        Audit trail
```

### Layer Hierarchy

```
L0: terra-types       (shared types, no deps)
L1: terra-core        (Intelligence Core)
L1: terra-trace       (audit)
L1: terra-providers   (LLM adapters)
L2: terra-protocol    (command protocol)
L2: terra-memory      (storage + pointer index)
L3: terra-caging      (behavioral constraints)
L3: terra-compile     (language compilation)
L3: terra-executor    (command execution)
L4: terra-agents      (REPL + orchestrator)
L5: terra-cli         (user interface)
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `TERRA_PROVIDER_API_KEY` | — | LLM provider API key |
| `DASHSCOPE_API_KEY` | — | Fallback API key (DashScope) |
| `TERRA_PROVIDER_BASE_URL` | `https://dashscope.aliyuncs.com/compatible-mode/v1` | API endpoint |
| `TERRA_PROVIDER_MODEL` | `qwen-plus` | Model name |
| `RUST_LOG` | `terra=warn` (chat) / `terra=info` (other) | Log level |

## Testing

```bash
cargo test --workspace
```

Currently 517 tests across all crates.

## License

MIT
