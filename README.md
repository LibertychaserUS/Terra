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
  <a href="#project-status">Status</a> &nbsp;&bull;&nbsp;
  <a href="#features">Features</a> &nbsp;&bull;&nbsp;
  <a href="#architecture">Architecture</a> &nbsp;&bull;&nbsp;
  <a href="#quick-start">Quick Start</a> &nbsp;&bull;&nbsp;
  <a href="#commands">Commands</a> &nbsp;&bull;&nbsp;
  <a href="#caging-engineering">Caging</a> &nbsp;&bull;&nbsp;
  <a href="#中文文档">中文文档</a>
</p>

<p align="center">
  <a href="https://github.com/LibertychaserUS/Terra/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/LibertychaserUS/Terra/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <a href="https://www.rust-lang.org/"><img alt="Rust 1.89+" src="https://img.shields.io/badge/rust-1.89%2B-orange.svg"></a>
  <img alt="Tests: 620 passing" src="https://img.shields.io/badge/tests-620%20passing-brightgreen.svg">
  <img alt="Crates: 12" src="https://img.shields.io/badge/crates-12-informational.svg">
  <img alt="Binary: ~10 MB" src="https://img.shields.io/badge/binary-~10%20MB-lightgrey.svg">
  <img alt="Platform: macOS | Linux" src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg">
</p>

---

Terra is an AI agent runtime written in Rust and built on **Caging Engineering**: instead of only telling a model what to do, it limits what the agent *can* do. Every file, memory and shell operation the agent requests passes through a caging pipeline and an audit log before it runs. Knowledge lives in a 7-level memory database with a compact pointer index, and any OpenAI-compatible LLM can drive it.

Terra is an early-stage project (v0.2). [Project status](#project-status) lists what works end to end today and what exists only as tested library code.

### Why Terra?

- **Checks before execution, not after** -- file writes and deletes must resolve inside the project (`..` and symlinks are resolved first), the constitution is write-protected, and shell commands and deletes run only after you answer `y`
- **Structured memory with ownership** -- 7 semantic levels (L0-L6) with owners and level-based write permissions, inspired by Rust's ownership model
- **Zero-cost drift detection** -- in Goal Mode, TF-IDF similarity between the goal and each answer flags drift without spending LLM tokens
- **Auditable** -- every executed operation is stored in `.terra/audit.db`; `terra trace` prints it
- **Single ~10 MB binary** -- no runtime dependencies, no Docker, no Python; SQLite is bundled
- **Any OpenAI-compatible LLM** -- DashScope (default), OpenAI, Ollama, vLLM, LiteLLM, ...
- **MCP server** -- `terra mcp` exposes the same caged tools to any Model Context Protocol client

---

## Table of Contents

- [Project Status](#project-status)
- [Features](#features)
- [Architecture](#architecture)
- [Quick Start](#quick-start)
- [Commands](#commands)
- [Configuration](#configuration)
- [Memory System](#memory-system)
- [Caging Engineering](#caging-engineering)
- [Testing](#testing)
- [System Requirements](#system-requirements)
- [Known Limitations](#known-limitations)
- [Contributing](#contributing)
- [License](#license)
- [中文文档](#中文文档)

---

## Project Status

| Area | State today |
|------|-------------|
| Interactive chat (`terra chat`) | Works end to end: LLM tool calling (files, memory, shell, artifacts, pointer queries), y/N confirmation for shell and delete, session auto-save, context compaction |
| Full-screen TUI (`terra chat --tui`) | Works, with the same confirmations; simpler than the readline UI (Ctrl+C cannot cancel a running request) |
| One-shot tasks (`terra run`) | Works: spec compilation, Intelligence Core rules, specification gate, caged provider call, audit. The task has to name what it changes (a file, for example), or the Specification cage stops it |
| MCP server (`terra mcp`) | Works over stdio: 13 tools routed through the same executor and cages; approving shell and delete calls is left to the MCP client |
| Audit trail (`terra trace`) | Works: persistent SQLite log of every executed command |
| Cages enforced at runtime | **Action** (sandboxed writes/deletes, destructive shell patterns), **Memory** (L0 read-only, L1 user-only), **Output** (prompt-injection patterns in LLM answers), **Specification** (`terra run` and `/execute` do not start while the spec has open questions), **Verification** (`/done` needs a passing `/verify` after the goal's changes), **Meta** (no writes or deletes of `.terra/constitution.toml`) |
| Cages not registered | **Knowledge** and **Convergence** are implemented and unit-tested, but nothing in the runtime produces the knowledge claims or accepted-path events they judge |
| Multi-agent roles | Worker, Explorer, Cage Monitor and SubAgent profiles exist as tested library types; the CLI runs a single main agent |
| Embeddings | `EmbeddingBackend` trait with a TF-IDF implementation; no neural embedding backend ships yet |

---

## Features

### Behavioral Caging

Cages run in sequence on every operation the executor receives; the first `Block` stops it, and the block is written to the audit log. The agent cannot skip them: every tool call goes through the executor.

| Cage | Purpose | Mode | Enforced at runtime |
|------|---------|:----:|:-------------------:|
| **Action** | Writes/deletes must resolve inside the sandbox root; protected paths are read-only; `rm -rf /`, `mkfs`, `> /dev/` are blocked | Always | Yes |
| **Memory** | L0 is read-only; L1 accepts only user commands (`/remember`, `/accept`); L3 is meant for Goal Mode + verification | Always | L0, L1: yes. L3: not yet |
| **Output** | Withholds LLM answers containing prompt-injection or privilege-escalation phrases | Always | Yes (chat answers and provider calls) |
| **Specification** | Stops a task while its spec has open questions (no target file named, for example) | `terra run`, Goal | Yes (`terra run`, `/execute`) |
| **Verification** | Stops `/done` while the goal's writes, deletes or shell commands are not followed by a passing `/verify` | Goal | Yes (`/done`) |
| **Meta** | Blocks rule changes: any write or delete that resolves to `.terra/constitution.toml` | Always | Yes |
| **Knowledge** | Authority ranking: Code > Docs > Memory > Assumption | Always | Library only |
| **Convergence** | Warns about exploring after an accepted path exists | Goal | Library only |

The constitution's `require_spec_complete` and `require_verification` settings (both `true` by default) turn the Specification and Verification cages off for a project.

Shell commands are not sandboxed: once you approve one, it runs with your user's permissions. The confirmation prompt is the control for shell access.

### 7-Level Memory Database

Semantic memory organized from most authoritative to least, with write permissions per level:

```
L0  Authoritative     Constitution, Intelligence Core, Soul Templates     (read-only)
L1  Project           Architecture decisions, user-saved knowledge        (user-only writes)
L2  Task              Current hypotheses, active attempts                 (read-write)
L3  Accepted Paths    Verified successful routes                          (meant: Goal Mode + verified)
L4  Failed Paths      Compressed failure records                          (read-write)
L5  Dormant Ideas     Archived, low priority, auto-compacted              (read-write)
L6  Candidates        Agent nominations awaiting human review             (read-write)
```

- **Ownership**: each entry has an owner; writes check ownership and level
- **Pluggable backend**: `StorageBackend` trait with SQLite (WAL) and in-memory implementations
- **Cache**: moka read-through/write-through cache in front of the store
- **Garbage collection**: salience-aware GC that protects L0/L1 is implemented in `terra-memory`; the CLI does not schedule it yet

### DSL Pointer Memory Index

A lightweight pointer table over the memory database, compact enough to put in an LLM context window. The agent queries it through the `PointerQuery` tool:

```
@mem["project-arch"]       # Direct pointer lookup by key
@L1[*]                     # All pointers at level L1
@scope["project"]          # Filter by scope
@search("auth bug", 5)     # Salience-ranked search, top 5 results
@top(10)                   # Top 10 pointers by salience score
```

Salience combines four signals:

| Signal | Weight | Source |
|--------|:------:|--------|
| Keyword match | 0.3 | TF-IDF (`VectorIndex`) |
| Semantic similarity | 0.4 | `EmbeddingBackend` (TF-IDF today) |
| Time decay | 0.2 | Recency weighting |
| Access frequency | 0.1 | Usage counter |

### Dual Mode Operation

| Mode | Behavior |
|------|----------|
| **Conversation** | Open-ended dialogue with tools. Base state. |
| **Goal** | Entered with `/goal`. Tracks the goal, scores each answer for drift, and runs sleep consolidation on `/done`. |

### Language Compilation

`terra run` compiles natural language into a structured spec before calling the model:

```
User Input --> TermTable --> SpecIR --> ExecutionPlan --> Steps
                  |             |            |
             Terminology    Structured    Versioned
             normalization  spec (IR)     step graph
```

- **TermTable**: maps informal terms to canonical forms (`"login bug"` -> `"authentication_failure"`)
- **SpecIR**: intent, target (file/line/function/component), constraints, success predicates, open questions
- **ExecutionPlan**: versioned step graph with dependencies and per-step status (`/plan`, `/execute`)

### Additional Features

- **Output contracts** -- JSON schema validation of structured LLM outputs
- **Provider dispatch** -- OpenAI-compatible adapter with retry and exponential backoff; dry-run adapter for keyless use
- **Event bus** -- `NullBus`, `CollectorBus` (tests), `ChannelBus` (live operation lines in the REPL)
- **Constitution** -- `.terra/constitution.toml` sets the sandbox root and protected paths
- **Smart greeter** -- session start suggestions from git status, past sessions and project type
- **Sessions** -- auto-saved to `.terra/sessions/`, listed with `/sessions`
- **Context compaction** -- at 70% of the context budget older turns are summarized and the summary is stored in L5 memory
- **Intelligence Core** -- pluggable domain modules (Coding, Algorithm) with ranked diagnostic rules

---

## Architecture

```
┌────────────────────────────────────────────────────────────────┐
│ 6  terra-cli        CLI (clap), readline REPL, ratatui TUI     │
├────────────────────────────────────────────────────────────────┤
│ 5  terra-mcp        MCP server (JSON-RPC over stdio)           │
├────────────────────────────────────────────────────────────────┤
│ 4  terra-agents     REPL loop + tool calling, orchestrator,    │
│                     sessions, drift detector, agent roles      │
├────────────────────────────────────────────────────────────────┤
│ 3  terra-executor   Command pipeline: audit → cages → router   │
├────────────────────────────────────────────────────────────────┤
│ 2  terra-caging     8 cages, sandbox guard, constitution       │
│    terra-compile    SpecIR, ExecutionPlan, term table          │
│    terra-trace      Persistent audit trail                     │
├────────────────────────────────────────────────────────────────┤
│ 1  terra-protocol   Message types, MessagePack codec, render   │
├────────────────────────────────────────────────────────────────┤
│ 0  terra-types      Commands, events, contracts, text helpers  │
│    terra-core       Intelligence Core, soul templates          │
│    terra-memory     7-level memory DB, pointer index, cache    │
│    terra-providers  OpenAI-compatible adapter, dry-run         │
└────────────────────────────────────────────────────────────────┘
```

**Dependency rule**: a crate only depends on crates in lower layers. `crates/terra-cli/tests/layering.rs` checks every `Cargo.toml` against this table.

**How a tool call flows**: the REPL sends the conversation and the tool schemas to the LLM → the LLM asks for a tool → shell and delete calls ask you `y/N` → the executor records the command in the audit log, runs the cages, routes the operation, and records the result → the result goes back to the LLM with all earlier tool results of the turn → the final answer passes the output cage before you see it.

<details>
<summary><strong>Crate summary</strong></summary>

| Crate | Layer | Terra dependencies | Purpose |
|-------|:-----:|--------------------|---------|
| `terra-types` | 0 | -- | Commands, events, output contracts, UTF-8 safe text helpers |
| `terra-core` | 0 | -- | Intelligence Core, soul templates, diagnostic signals |
| `terra-memory` | 0 | -- | Memory DB, pointer index, cache, ownership checks |
| `terra-providers` | 0 | -- | OpenAI-compatible adapter (retry/backoff, SSE), dry-run adapter |
| `terra-protocol` | 1 | types | Message format, MessagePack codec, Markdown rendering |
| `terra-trace` | 2 | types, protocol | Audit log with SQLite persistence |
| `terra-compile` | 2 | protocol | SpecIR, ExecutionPlan, terminology table |
| `terra-caging` | 2 | core, protocol | Cage trait, 8 cages, sandbox guard, constitution |
| `terra-executor` | 3 | types, protocol, memory, trace, caging, providers | Command pipeline and router, event bus |
| `terra-agents` | 4 | core, types, protocol, compile, executor, memory, providers, caging, trace | REPL, tools, orchestrator, sessions, roles |
| `terra-mcp` | 5 | types, executor, memory, caging, core, providers, trace, agents | MCP server |
| `terra-cli` | 6 | agents, types, core, executor, memory, caging, trace, providers, mcp | `terra` binary |

</details>

---

## Quick Start

### Prerequisites

- **Rust 1.89+** (install via [rustup](https://rustup.rs))
- **macOS** (ARM64/x86_64) or **Linux** (x86_64)
- An **API key** for any OpenAI-compatible LLM provider (optional: without one Terra runs in dry-run mode)

SQLite is bundled via `rusqlite` -- no external database required.

### Install

```bash
git clone https://github.com/LibertychaserUS/Terra.git
cd Terra
cargo build --release          # binary at target/release/terra (~10 MB on arm64 macOS)
```

Or install it on your `PATH`:

```bash
cargo install --path crates/terra-cli --locked
```

### Configure

```bash
# DashScope (Alibaba Cloud / Qwen) -- default provider
export DASHSCOPE_API_KEY="your-key-here"

# Any OpenAI-compatible endpoint
export TERRA_PROVIDER_API_KEY="your-key-here"
export TERRA_PROVIDER_BASE_URL="https://api.openai.com/v1"
export TERRA_PROVIDER_MODEL="gpt-4o"

# Or store settings in a config file instead of the environment
terra config set provider.model gpt-4o            # ~/.terra/config.toml
terra config set provider.api_key "..." --project # .terra/config.toml (git-ignored)
```

**No API key?** Everything still runs: the dry-run provider echoes your prompt, which is enough to try commands, memory, confirmations and the audit trail.

### Run

```bash
terra init                    # Create .terra/ (constitution, config, prompt, .gitignore)
terra chat                    # Interactive session (also: just `terra`)
terra chat --tui              # Full-screen terminal UI
terra run --task "Add input validation to src/main.rs"   # One-shot task
terra doctor                  # Environment and provider check
terra trace -n 20             # Last 20 audit entries for this project
terra mcp                     # MCP server on stdio
```

A `terra run` task has to name its target, such as a file. Open-ended tasks ("Explain this project's architecture") are blocked by the Specification cage unless `require_spec_complete = false` is set in `.terra/constitution.toml`.

---

## Commands

Commands available inside `terra chat`:

| Command | Alias | Description |
|---------|:-----:|-------------|
| `/goal "description"` | `/g` | Enter Goal Mode: the goal is tracked and answers are checked for drift |
| `/done` | `/d` | Claim the goal is done; blocked until the goal's changes pass `/verify`. Then runs sleep consolidation |
| `/verify [command]` | | Run a check (default: the project's test command, such as `cargo test`) that verifies the goal's changes |
| `/abandon` | | Leave Goal Mode without claiming the goal is done |
| `/accept N` / `/reject N` | | Keep or drop a consolidation candidate (kept ones go to L1 memory) |
| `/status` | `/s` | Mode, goal alignment, token usage |
| `/plan` | `/p` | Ask the LLM for an execution plan for the current goal |
| `/execute` | | Run the proposed plan step by step (each step is an LLM call without tools) |
| `/skill [name]` | | List or run a built-in skill (`code_editor`, `test_runner`, `memory_inspector`) |
| `/commit` | `/c` | Stage all changes and commit with an LLM-written message (asks first) |
| `/undo` | | Discard unstaged edits to tracked files (lists them, asks first) |
| `/remember "text"` | | Save text to L1 project memory |
| `/trace` | | Show the last 10 audit entries |
| `/sessions` | | List saved sessions |
| `/compact` | | Summarize older turns to free context |
| `/sleep` | | Run sleep consolidation manually |
| `/clear` | | Clear screen and conversation context |
| `/help` | `/h` | Show command list |
| `/quit` | `/q` | Exit (also `/exit` or Ctrl+D) |

---

## Configuration

Settings are resolved in this order: environment variables > `.terra/config.toml` > `~/.terra/config.toml` > defaults. `terra config show` prints the result; `terra doctor` shows where the API key comes from.

| Variable | Config key | Default | Description |
|----------|------------|---------|-------------|
| `TERRA_PROVIDER_API_KEY` | `provider.api_key` | -- | LLM provider API key |
| `DASHSCOPE_API_KEY` | -- | -- | Fallback API key (DashScope) |
| `TERRA_PROVIDER_BASE_URL` | `provider.base_url` | `https://dashscope.aliyuncs.com/compatible-mode/v1` | API endpoint |
| `TERRA_PROVIDER_MODEL` | `provider.model` | `qwen-plus` | Model name |
| `RUST_LOG` | -- | `terra=warn` | Log level (logs go to stderr) |
| `NO_COLOR` | -- | -- | Disable colors (they are also off when output is not a terminal) |

`.terra/` layout after `terra init` and a first session:

```
.terra/
├── .gitignore          Keeps everything below out of git except the shareable files
├── constitution.toml   Sandbox root and protected paths            (shareable)
├── prompt.md           System prompt, editable                     (shareable)
├── config.toml         Provider settings                           (git-ignored)
├── memory.db           Memory database                             (git-ignored)
├── audit.db            Audit trail                                 (git-ignored)
├── history.txt         Input history                               (git-ignored)
└── sessions/           Saved sessions                              (git-ignored)
```

---

## Memory System

### 7-Level Database

The design borrows from Rust's ownership model:

- **Ownership**: each memory entry has a single owner (agent or user)
- **Borrowing**: SubAgents receive a read-only `MemoryView` filtered by their profile's levels
- **Lifetime**: entries carry creation time, last access, access count and a reference count
- **Garbage collection**: removes low-salience entries in L2-L6 (library; not scheduled by the CLI yet)

| Level | Name | Description | Write Permission | GC Protected |
|:-----:|------|-------------|------------------|:------------:|
| L0 | Authoritative | Constitution, Soul Templates, Intelligence Core | Blocked (read-only) | Yes |
| L1 | Project | Architecture decisions, user-saved knowledge | User `/remember` only | Yes |
| L2 | Task | Current hypotheses, active attempts | Both modes | No |
| L3 | Accepted Paths | Verified successful solution routes | Meant for Goal Mode + verified; not enforced yet | No |
| L4 | Failed Paths | Compressed failure records | Both modes | No |
| L5 | Dormant | Archived ideas, auto-compacted | Both modes | No |
| L6 | Candidates | Agent nominations, awaiting human review | Both modes | No |

### Storage Architecture

```
Read path:   request --> Cache (moka) --> StorageBackend (SQLite)    [read-through]
Write path:  request --> SafetyEnforcer --> StorageBackend --> Cache  [write-through]
```

| Backend | Use Case |
|---------|----------|
| `SqliteStore` | Default, WAL mode |
| `InMemoryBackend` | Tests, no persistence |

---

## Caging Engineering

Caging Engineering is a behavioral constraint methodology. Instead of telling an AI what to do (prompting), caging defines the boundaries of what it *can* do, with programmatic checks that run before an action executes.

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

### Cage Details

<details>
<summary><strong>Action Cage</strong> -- enforced</summary>

Validates file writes and deletes with `SandboxGuard`. Paths are resolved the way the OS would open them (`.`/`..` applied, symlinks in existing ancestors followed) before checking that they are inside the sandbox root and not under a protected path. Regression tests cover `../` escapes, `.terra/../.terra/constitution.toml`, symlinked directories and symlink cycles. File reads are not restricted.

Shell commands matching `rm -rf /`, `mkfs` or `> /dev/` are blocked; other shell commands are logged and, in chat, need your confirmation.

</details>

<details>
<summary><strong>Memory Cage</strong> -- enforced</summary>

| Level | Permission |
|-------|-----------|
| L0 | **Blocked always** -- read-only |
| L1 | **Blocked for agents** -- only user `/remember` or `/accept` |
| L2 | Allowed in both modes |
| L3 | Meant for Goal Mode with verification; not enforced yet (see [Known limitations](#known-limitations)) |
| L4-L6 | Allowed in both modes |

</details>

<details>
<summary><strong>Output Cage</strong> -- enforced</summary>

Inspects LLM answers before you or the agent see them.

**Blocked** (answer withheld): prompt-injection phrases such as `"ignore all previous"` or `"disregard the above"`, and escalation phrases such as `"disable caging"`, `"write to L0"`, `"bypass sandbox"`.

**Warned** (logged): `"rm -rf"`, `"drop table"`, `"delete all"`.

This is phrase matching, a cheap first filter rather than a complete defense.

</details>

<details>
<summary><strong>Specification Cage</strong> -- enforced on tasks</summary>

`terra run` compiles the task into a SpecIR (lexical and terminology passes) and asks the cages before the worker is called; in Goal Mode, `/execute` does the same with the goal's spec. A spec with open questions is blocked: the task does not run, `terra run` exits with code 1, and the audit log records a denied `ExecuteTask`. Without an LLM in the compilation step the open question is usually "Target file not specified", so a task has to name what it changes:

```bash
terra run --task "explain the project"                 # blocked: Target file not specified
terra run --task "add input validation to src/api.rs"  # runs
```

Chat is not gated, in Conversation or Goal Mode. `/goal` lists the goal's open questions when it starts.

</details>

<details>
<summary><strong>Verification Cage</strong> -- enforced on /done</summary>

In Goal Mode, `/done` is a completion claim, recorded in the audit log as `ClaimComplete`. Terra reads the audit log since `/goal`: each successful file write or delete, and each shell command the agent ran, is a change. Only a command started by `/verify` that exits with 0 verifies the changes before it; the agent running `cargo test` itself counts as a change. While changes are unverified, or the last `/verify` failed, the claim is blocked and Goal Mode stays on. `/abandon` leaves Goal Mode without a claim.

```
/goal add a greeting to notes.txt
  ... the agent writes notes.txt ...
/done     -> Goal not closed: 1 write, delete or shell command is not followed by a passing /verify
/verify   -> runs the project's test command (cargo test, npm test, pytest, go test ./...)
/done     -> Goal complete (verified by `cargo test`)
```

</details>

<details>
<summary><strong>Meta Cage</strong> -- enforced</summary>

Blocks writes and deletes that resolve to `.terra/constitution.toml` or a path beneath it (`..` and symlinks are resolved, case is ignored). It does not rely on the constitution listing itself as a protected path, so the agent cannot create a constitution with a wider sandbox for the next session. It runs before the Action cage, whose protected paths still apply. Shell commands are not covered.

</details>

<details>
<summary><strong>Knowledge, Convergence</strong> -- library only</summary>

- **Knowledge** enforces Code > Documentation > Memory > Assumption: lower-authority claims that contradict higher ones are blocked.
- **Convergence** warns about further exploration once a verified path was accepted.

Both are implemented and unit-tested but not registered in the runtime engine: nothing in the runtime produces knowledge claims or accepted-path events yet, so they would never fire.

</details>

---

## Testing

```bash
cargo test --workspace                                  # 620 tests, no network or API key needed
cargo test --workspace -- --ignored                     # 17 live tests against a real LLM (needs an API key)
cargo clippy --workspace --all-targets -- -D warnings   # clean
```

The default suite covers:

- Cage checks (pass/warn/block) and sandbox path resolution (`..`, symlinks, cycles)
- The Specification, Verification and Meta gates in use: `terra run`, `/execute`, `/done` with `/verify`, and MCP writes to the constitution
- Memory database CRUD, ownership, GC, cache; pointer table DSL and salience scoring
- SpecIR compilation, term normalization, ExecutionPlan versioning
- Provider retry against a mock HTTP server; OpenAI request building with multi-round tool calls
- The REPL agent loop with a scripted provider (what each request contains), confirmations, `/undo`, `/commit`
- Non-ASCII input (CJK, emoji) in previews and the TUI line editor
- The `terra` binary in an isolated project and HOME: init, run, trace, config, doctor, piped output
- MCP JSON-RPC handling, including sandbox escapes through `FileWrite`
- Crate layering (`tests/layering.rs`)

CI (`.github/workflows/ci.yml`) runs build, tests and clippy on Linux and macOS, plus `cargo check` on Rust 1.89.

---

## System Requirements

| Requirement | Details |
|-------------|---------|
| **OS** | macOS (tested on ARM64), Linux x86_64 (CI) |
| **Rust** | 1.89+ (floor set by locked dependencies; edition 2021) |
| **Disk** | ~475 MB of build artifacts for a release build; ~10 MB binary |
| **Memory** | ~12 MB resident for an idle chat session |
| **Network** | Only for LLM API calls |
| **SQLite** | Bundled via `rusqlite` |
| **Windows** | Untested; may work under WSL |

---

## Known Limitations

- Approved shell commands run with your user's permissions; the sandbox covers Terra's own file tools, not what a shell command does.
- File reads are not limited to the project directory.
- `terra mcp` has no y/N prompt of its own: the MCP client decides whether a shell or delete call runs.
- Chat prints an answer after it has fully arrived (word by word); the provider's SSE streaming is not used by the chat loop yet.
- From the second turn on, the system prompt is reduced to Terra's identity to save tokens; `prompt.md` guidance and the memory index are sent on the first turn only.
- Knowledge and Convergence are not registered (see [Project status](#project-status)).
- The executor does not pass the session mode to the cages yet; it reports Goal Mode for every command. Goal-only cages are therefore scoped by where their actions come from (`terra run`, `/execute`, `/done`), and the Memory cage's L3 rule is not enforced.
- Spec compilation does not use the LLM yet, so `terra run` blocks questions and open-ended tasks that name no file, unless `require_spec_complete = false`.
- Shell commands can still change `.terra/constitution.toml`: the Meta cage, like the sandbox, covers Terra's file tools.
- Multi-agent roles, memory GC and session resume exist in the libraries but are not exposed by the CLI.
- `/execute` runs plan steps as LLM calls without tools.
- Similarity scoring is TF-IDF; there is no neural embedding backend.
- The TUI cannot cancel a request in flight.

---

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/your-feature`)
3. Write tests for new functionality
4. Make sure `cargo test --workspace` passes
5. Make sure `cargo clippy --workspace --all-targets -- -D warnings` passes
6. Submit a pull request

### Development Guidelines

- **Layering**: a crate only depends on lower layers; add new crates to `crates/terra-cli/tests/layering.rs`
- **Doc comments** on public APIs
- **New cages**: implement the `Cage` trait and register it in `CagingEngine`
- **Memory operations** go through `MemoryDatabase`, never direct SQLite access
- **New providers** implement `ProviderAdapter`
- **Text previews**: use `terra_types::text` helpers instead of byte slicing

---

## License

[MIT](LICENSE)

---

## Acknowledgments

- **Caging Engineering** -- the behavioral constraint methodology that grounds Terra's safety model
- **Rust** -- ownership and borrowing semantics that inspired the memory system design
- The Rust ecosystem: `tokio`, `serde`, `rusqlite`, `moka`, `ratatui`, `clap`, `reqwest`, `rmp-serde`, and the broader community

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

Terra 是一个用 Rust 编写的 **AI Agent 运行时**，以 **Caging Engineering（笼化工程）** 为核心方法论：不只是告诉模型"做什么"，而是限定 Agent "能做什么"。Agent 请求的每一个文件、记忆和 shell 操作，都要先经过笼化检查和审计日志才会执行。知识存放在带指针索引的 7 层记忆数据库中，可接入任意 OpenAI 兼容的 LLM。

Terra 目前处于早期阶段（v0.2）。[项目状态](#项目状态) 列出了哪些功能已端到端可用，哪些目前只是经过测试的库代码。

---

## 项目状态

| 模块 | 当前状态 |
|------|----------|
| 交互式对话（`terra chat`） | 端到端可用：LLM 工具调用（文件、记忆、shell、制品、指针查询），shell 与删除操作需 y/N 确认，会话自动保存，上下文压缩 |
| 全屏 TUI（`terra chat --tui`） | 可用，确认机制相同；比 readline 界面简单（Ctrl+C 无法取消进行中的请求） |
| 单次任务（`terra run`） | 可用：规格编译、智能核心规则、规格闸门、经笼检查的模型调用、审计。任务需写明要改动的对象（例如文件），否则会被规格笼拦下 |
| MCP 服务（`terra mcp`） | 通过 stdio 可用：13 个工具，经过同一套执行器和笼；shell 与删除操作由 MCP 客户端负责确认 |
| 审计日志（`terra trace`） | 可用：每条已执行命令都持久化在 SQLite 中 |
| 运行时生效的笼 | **Action**（沙箱化写入/删除、破坏性 shell 模式）、**Memory**（L0 只读、L1 仅用户可写）、**Output**（LLM 回答中的提示注入短语）、**Specification**（规格有未解决问题时 `terra run` 与 `/execute` 不会开始）、**Verification**（目标期间的改动通过 `/verify` 后 `/done` 才能完成）、**Meta**（禁止写入或删除 `.terra/constitution.toml`） |
| 未注册的笼 | **Knowledge**、**Convergence** 已实现并有单元测试，但运行时尚无组件产生它们所判断的知识声明或"已接受路径"事件 |
| 多 Agent 角色 | Worker、Explorer、Cage Monitor 与 SubAgent 配置是经过测试的库类型；CLI 目前只运行一个主 Agent |
| 向量嵌入 | 提供 `EmbeddingBackend` trait 与 TF-IDF 实现，暂无神经网络嵌入后端 |

---

## 核心特性

- **执行前检查，而非事后审计** -- 文件写入和删除必须解析到项目目录内（先解析 `..` 与符号链接），宪法文件写保护，shell 命令和删除需要你回答 `y` 才会执行
- **结构化记忆 + 所有权** -- 7 层语义级别（L0-L6），借鉴 Rust 所有权模型的属主与分级写权限
- **零成本漂移检测** -- 目标模式下用 TF-IDF 相似度比较目标与回答，不消耗 LLM token
- **可审计** -- 每个已执行操作都写入 `.terra/audit.db`，`terra trace` 可查看
- **单个约 10 MB 二进制** -- 无运行时依赖，无 Docker，无 Python，SQLite 内置
- **任意 OpenAI 兼容 LLM** -- DashScope（默认）、OpenAI、Ollama、vLLM、LiteLLM 等
- **MCP 服务** -- `terra mcp` 把同一套受笼约束的工具提供给任意 MCP 客户端

---

## 快速开始

### 前置条件

- **Rust 1.89+**（通过 [rustup](https://rustup.rs) 安装）
- **macOS**（ARM64/x86_64）或 **Linux**（x86_64）
- 任意 OpenAI 兼容 LLM 提供商的 **API Key**（可选：没有 key 时以 dry-run 模式运行）

### 安装

```bash
git clone https://github.com/LibertychaserUS/Terra.git
cd Terra
cargo build --release          # 二进制位于 target/release/terra（arm64 macOS 约 10 MB）
```

或安装到 `PATH`：

```bash
cargo install --path crates/terra-cli --locked
```

### 配置

```bash
# DashScope（阿里云 / 通义千问）-- 默认供应商
export DASHSCOPE_API_KEY="your-key-here"

# 任意 OpenAI 兼容端点
export TERRA_PROVIDER_API_KEY="your-key-here"
export TERRA_PROVIDER_BASE_URL="https://api.openai.com/v1"
export TERRA_PROVIDER_MODEL="gpt-4o"

# 也可以写入配置文件
terra config set provider.model gpt-4o            # ~/.terra/config.toml
terra config set provider.api_key "..." --project # .terra/config.toml（已被 git 忽略）
```

优先级：环境变量 > `.terra/config.toml` > `~/.terra/config.toml` > 默认值。**没有 API key** 也能运行：dry-run 供应商会回显你的输入，足以体验命令、记忆、确认机制和审计日志。

### 运行

```bash
terra init                    # 创建 .terra/（宪法、配置、提示词、.gitignore）
terra chat                    # 交互式会话（直接运行 `terra` 亦可）
terra chat --tui              # 全屏终端界面
terra run --task "为 src/main.rs 添加输入校验"   # 单次任务
terra doctor                  # 环境与供应商检查
terra trace -n 20             # 本项目最近 20 条审计记录
terra mcp                     # stdio 上的 MCP 服务
```

`terra run` 的任务需写明目标，例如文件；文件名前后要留空格。开放式任务（如"解释这个项目的架构"）会被规格笼拦下，除非在 `.terra/constitution.toml` 中设置 `require_spec_complete = false`。

---

## 命令列表

在 `terra chat` 中可用的命令：

| 命令 | 别名 | 说明 |
|------|:----:|------|
| `/goal "描述"` | `/g` | 进入目标模式：跟踪目标并检查回答是否偏离 |
| `/done` | `/d` | 声明目标完成；目标期间的改动需先通过 `/verify`，随后触发 sleep 沉淀 |
| `/verify [命令]` | | 运行检查命令（默认为项目的测试命令，例如 `cargo test`），验证目标期间的改动 |
| `/abandon` | | 退出目标模式，不声明完成 |
| `/accept N` / `/reject N` | | 保留或丢弃沉淀候选（保留的写入 L1 记忆） |
| `/status` | `/s` | 显示模式、目标对齐度、token 用量 |
| `/plan` | `/p` | 让 LLM 为当前目标生成执行计划 |
| `/execute` | | 逐步执行计划（每一步是一次不带工具的 LLM 调用） |
| `/skill [名称]` | | 列出或运行内置技能（`code_editor`、`test_runner`、`memory_inspector`） |
| `/commit` | `/c` | 暂存全部变更并用 LLM 生成的信息提交（先询问） |
| `/undo` | | 丢弃已跟踪文件的未暂存修改（先列出文件并询问） |
| `/remember "文本"` | | 保存到 L1 项目记忆 |
| `/trace` | | 显示最近 10 条审计记录 |
| `/sessions` | | 列出已保存会话 |
| `/compact` | | 压缩较早的对话以释放上下文 |
| `/sleep` | | 手动触发 sleep 沉淀 |
| `/clear` | | 清屏并重置对话上下文 |
| `/help` | `/h` | 显示命令帮助 |
| `/quit` | `/q` | 退出（也可 `/exit` 或 Ctrl+D） |

---

## 架构概述

```
┌────────────────────────────────────────────────────────────────┐
│ 6  terra-cli        命令行（clap）、readline REPL、ratatui TUI │
├────────────────────────────────────────────────────────────────┤
│ 5  terra-mcp        MCP 服务（stdio 上的 JSON-RPC）            │
├────────────────────────────────────────────────────────────────┤
│ 4  terra-agents     REPL 循环 + 工具调用、编排器、会话、       │
│                     漂移检测、Agent 角色                       │
├────────────────────────────────────────────────────────────────┤
│ 3  terra-executor   命令管线：审计 → 笼检查 → 路由             │
├────────────────────────────────────────────────────────────────┤
│ 2  terra-caging     8 个笼、沙箱守卫、宪法                     │
│    terra-compile    SpecIR、执行计划、术语表                   │
│    terra-trace      持久化审计日志                             │
├────────────────────────────────────────────────────────────────┤
│ 1  terra-protocol   消息类型、MessagePack 编解码、渲染         │
├────────────────────────────────────────────────────────────────┤
│ 0  terra-types      命令、事件、契约、文本工具                 │
│    terra-core       智能核心、Soul 模板                        │
│    terra-memory     7 层记忆库、指针索引、缓存                 │
│    terra-providers  OpenAI 兼容适配器、dry-run                 │
└────────────────────────────────────────────────────────────────┘
```

**依赖规则**：crate 只依赖更低层的 crate，由 `crates/terra-cli/tests/layering.rs` 对每个 `Cargo.toml` 自动检查。

---

## 记忆系统

| 层级 | 名称 | 说明 | 写权限 | GC 保护 |
|:----:|------|------|--------|:-------:|
| L0 | 权威层 | 宪法、Soul 模板、智能核心 | 完全禁止（只读） | 是 |
| L1 | 项目层 | 架构决策、用户保存的知识 | 仅用户 `/remember`、`/accept` | 是 |
| L2 | 任务层 | 当前假设、活跃尝试 | 双模式均可 | 否 |
| L3 | 已验证路径 | 经验证的成功方案 | 设计为仅目标模式 + 已验证，尚未生效 | 否 |
| L4 | 失败路径 | 压缩的失败记录 | 双模式均可 | 否 |
| L5 | 休眠层 | 已归档、低优先级 | 双模式均可 | 否 |
| L6 | 候选层 | Agent 提名，等待人工审核 | 双模式均可 | 否 |

设计借鉴 Rust 所有权模型：每个条目有唯一属主；SubAgent 通过只读、按层过滤的 `MemoryView` 借用；条目记录创建时间、访问次数和引用计数；基于显著性的垃圾回收已在 `terra-memory` 中实现（CLI 尚未调度）。

指针表是记忆库之上的轻量索引，Agent 通过 `PointerQuery` 工具查询：

```
@mem["project-arch"]       按 key 精确查找
@L1[*]                     L1 层全部指针
@scope["project"]          按作用域过滤（global/project/session/task）
@search("auth bug", 5)     显著性排序搜索，前 5 条
@top(10)                   按显著性评分取前 10
```

显著性 = 关键词匹配 0.3（TF-IDF）+ 语义相似度 0.4（`EmbeddingBackend`，目前为 TF-IDF）+ 时间衰减 0.2 + 访问频率 0.1。

---

## Caging 工程概述

Caging Engineering（笼化工程）是一种行为约束方法论。与提示工程（告诉 AI "做什么"）不同，Caging 用在动作执行前运行的程序化检查来定义 AI "能做什么" 的边界。

| 笼 | 功能 | 模式 | 运行时生效 |
|----|------|:----:|:----------:|
| **Action（动作笼）** | 写入/删除必须解析到沙箱根目录内；受保护路径只读；拦截 `rm -rf /`、`mkfs`、`> /dev/` | 始终 | 是 |
| **Memory（记忆笼）** | L0 只读；L1 仅接受用户命令；L3 设计为需目标模式 + 验证 | 始终 | L0、L1 是；L3 尚未生效 |
| **Output（输出笼）** | 扣留含提示注入或权限提升短语的 LLM 回答 | 始终 | 是（对话回答与模型调用） |
| **Specification（规格笼）** | 规格有未解决问题（例如未写明目标文件）时阻止任务执行 | `terra run`、目标 | 是（`terra run`、`/execute`） |
| **Verification（验证笼）** | 目标期间的写入、删除或 shell 命令之后没有通过的 `/verify` 时，阻止 `/done` | 目标 | 是（`/done`） |
| **Meta（元笼）** | 阻止修改规则：解析后指向 `.terra/constitution.toml` 的写入或删除 | 始终 | 是 |
| **Knowledge（知识笼）** | 权威排序：代码 > 文档 > 记忆 > 假设 | 始终 | 仅库代码 |
| **Convergence（收敛笼）** | 已接受方案后继续探索时发出警告 | 目标 | 仅库代码 |

宪法中的 `require_spec_complete` 与 `require_verification`（默认均为 `true`）可以为单个项目关闭规格笼与验证笼。

shell 命令不在沙箱内：一旦你批准，它以你的用户权限运行，确认提示就是 shell 访问的控制点。文件读取不受项目目录限制。

---

## 测试

```bash
cargo test --workspace                                  # 620 个测试，无需网络和 API key
cargo test --workspace -- --ignored                     # 17 个连接真实 LLM 的测试（需要 API key）
cargo clippy --workspace --all-targets -- -D warnings   # 无警告
```

CI（`.github/workflows/ci.yml`）在 Linux 和 macOS 上运行构建、测试和 clippy，并用 Rust 1.89 执行 `cargo check`。

---

## 系统要求

| 项目 | 详情 |
|------|------|
| **操作系统** | macOS（已在 ARM64 上测试）、Linux x86_64（CI） |
| **Rust** | 1.89+（由锁定的依赖决定；edition 2021） |
| **磁盘** | release 构建产物约 475 MB，二进制约 10 MB |
| **内存** | 空闲对话会话常驻内存约 12 MB |
| **网络** | 仅 LLM API 调用时需要 |
| **SQLite** | 通过 `rusqlite` 内置 |
| **Windows** | 未测试，可能在 WSL 下工作 |

---

## 已知限制

- 已批准的 shell 命令以你的用户权限运行；沙箱只约束 Terra 自己的文件工具。
- 文件读取不限于项目目录。
- `terra mcp` 没有自己的 y/N 确认，shell 与删除调用是否执行由 MCP 客户端决定。
- 对话回答在完整返回后才逐词打印；对话循环尚未使用供应商的 SSE 流式输出。
- 从第二轮起系统提示词精简为身份声明以节省 token；`prompt.md` 与记忆索引只在第一轮发送。
- Knowledge 与 Convergence 尚未注册（见 [项目状态](#项目状态)）。
- 执行器尚未把会话模式传给笼，对每条命令都报告为目标模式。因此仅目标模式生效的笼由其动作的来源（`terra run`、`/execute`、`/done`）限定范围，记忆笼的 L3 规则尚未生效。
- 规格编译尚未使用 LLM，所以没有写明文件的提问式或开放式任务会被 `terra run` 拦下，除非设置 `require_spec_complete = false`。
- shell 命令仍可修改 `.terra/constitution.toml`：元笼与沙箱一样，只约束 Terra 自己的文件工具。
- 多 Agent 角色、记忆 GC、会话恢复已在库中实现，但 CLI 尚未开放。
- `/execute` 以不带工具的 LLM 调用执行计划步骤。
- 相似度计算基于 TF-IDF，暂无神经网络嵌入。
- TUI 无法取消进行中的请求。

---

## 许可证

[MIT](LICENSE)
