# Terra Product Requirements Specification

> Document ID: TERRA-PRS-002
> Version: 2.0.0
> Status: APPROVED
> Date: 2026-05-21
> Author: Oliver Zhang, Claude (Anthropic)
> Classification: Public
>
> This document follows IEEE 830-1998 with RFC 2119 keyword
> conventions. "MUST", "SHALL", "SHOULD", "MAY" are interpreted
> as described in RFC 2119.
>
> This version supersedes TERRA-PRS-001. No MVP simplification —
> this is the complete target specification.

---

## 1. Introduction

### 1.1 Purpose

This document specifies the complete product requirements for
Terra, an Agent Operating System for reliable AI agent execution
in software engineering tasks.

### 1.2 Scope

Terra is a runtime environment that manages AI agent lifecycle,
memory, permissions, verification, and inter-agent communication.

| OS Concept | Terra Implementation |
|-----------|---------------------|
| Firmware/BIOS | Intelligence Core (first principles) |
| Kernel | Caging Engine (8 cages) |
| Syscall interface | Command Executor + Command Protocol |
| Memory manager | Memory Database (scoped, leveled, Rust-safe) |
| Process scheduler | Agent Orchestrator (Main/Worker/Explorer/Cage) |
| IPC | MessagePack messages (Terra Internal Protocol) |
| File system | Artifact Store + Sandbox |
| Device drivers | Provider Adapters (LLM endpoints) |
| Shell | CLI |
| Compiler | Language Compilation Layer |
| Syslog | Audit Trail |

### 1.3 Definitions

| Term | Definition |
|------|-----------|
| Agent OS | Operating system for AI agents |
| Cage | Constraint mechanism shaping agent's reachable state space |
| CE | Caging Engineering — reachability-oriented framework |
| Spec IR | Specification Intermediate Representation |
| TIP | Terra Internal Protocol (MessagePack) |
| Command Executor | Single gateway for all agent-system interactions |
| Scope | Memory isolation: S0 Global, S1 Project, S2 Session, S3 Task |
| Level | Memory semantic class: L0-L6 |
| Promotion | Elevating memory entry between levels with verification gates |
| Sleep Consolidation | Post-session abstraction of actions into principles |
| Attention Retrieval | Dynamic context-dependent memory search via embeddings |

### 1.4 References

IEEE 830-1998, RFC 2119, Caging Engineering v0.1.0 (Zhang 2026),
ARCHITECTURE.md, VISION_OLIVER.md, REWRITE_CHECKLIST.md.

---

## 2. Overall Description

### 2.1 Product Position

Terra is an Agent OS — not a framework (LangChain), not a chatbot
(ChatGPT), not a grow-from-zero system (Hermes). It ships with
Intelligence Core: agents start from "one", not "zero".

### 2.2 Caging Engineering: Dual Role

CE governs both the development process and product behavior.

### 2.3 User Classes

| Class | Mode | Priority |
|-------|------|----------|
| Software Engineer | CLI, direct tasks | Primary |
| Skill Developer | MANIFEST.yml + MessagePack | Secondary |

### 2.4 Design Constraints

| ID | Constraint |
|----|-----------|
| DC-01 | Rust 2021 edition |
| DC-02 | Inter-agent: MessagePack only |
| DC-03 | Storage: SQLite WAL |
| DC-04 | Offline dry_run without network |
| DC-05 | Credentials from env vars only |
| DC-06 | No file modification outside sandbox |
| DC-07 | All agent-system interaction through Command Executor |
| DC-08 | All actions pass CagingEngine before execution |

---

## 3. Functional Requirements

### 3.1 Canonical Execution Pipeline

**FR-001: Mandatory Pipeline Path**
- Priority: MUST
- All task execution SHALL follow:
  User Input → Language Compilation → Spec IR → Intelligence Core consultation → CagingEngine gate → Command Executor → Provider/Memory/File operations → Verification Cage → Audit Trail → Result
- No component MAY bypass this pipeline.
- Rationale: Audit found orchestrator bypasses Caging, IC, and Command Protocol.
- Acceptance: Integration test traces task through all 7 stages via log inspection.

**FR-002: No Direct Subsystem Access**
- Priority: MUST
- Agents SHALL NOT directly call SQLite, provider adapters, or file system operations.
- All operations SHALL be Command objects to the Executor.
- Acceptance: Agent crate dependency audit shows no direct subsystem imports.

**FR-003: Mandatory Intelligence Core Consultation**
- Priority: MUST
- Before executing, Orchestrator SHALL query IC for applicable rules and perspectives.
- Results SHALL be injected into the LLM system prompt.
- Acceptance: System prompt contains coding/ module rules for code tasks.

**FR-004: Mandatory Caging Gate**
- Priority: MUST
- Before every action, CagingEngine SHALL be invoked.
- Block → action prevented + audit log entry.
- Acceptance: Incomplete spec triggers Block, preventing execution.

### 3.2 Command Executor

**FR-010: Executor Existence**
- Priority: MUST
- CommandExecutor SHALL be the single gateway for all agent-system interactions.
- Acceptance: `CommandExecutor::execute(cmd) -> CommandResult` method exists.

**FR-011: Executor Pipeline**
- Priority: MUST
- For every command: (1) audit record, (2) cage check, (3) execute, (4) audit result with duration.
- If cage blocks: PermissionDenied without executing.
- Acceptance: MemWrite command → audit has pre/post entries, cage checked, memory written.

**FR-012: Executor Routes**
- Priority: MUST
- Routes: MemWrite/Read/Search/Promote/Archive → MemoryDatabase. ArtifactSave/Load/List → ArtifactStore. TraceEvent/Checkpoint → AuditLog. SessionStart/Switch/End → SessionManager.
- Acceptance: Each command type reaches correct backend.

**FR-013: Executor is Only Path**
- Priority: MUST
- No code outside executor directly calls MemoryDatabase.write(), Provider.complete(), or std::fs::write().
- Acceptance: Import graph audit confirms no direct access.

### 3.3 Intelligence Core

**FR-020: Universal Reasoning Framework**
- Priority: MUST
- Rules R-001 to R-005: understand before acting, decompose, seek evidence, acknowledge unknowns, depth-first.
- L0 read-only. Queryable by category.
- Acceptance: `core.rules_by_category(Reasoning)` returns 5 rules.

**FR-021: Universal Behavioral Framework**
- Priority: MUST
- Rules B-001 to B-005: elegant>complex, minimal change, safe defaults, verify before claiming, follow patterns.
- Acceptance: `core.rules_by_category(Behavioral)` returns 5 rules.

**FR-022: Self-Diagnostic with Measurable Signals**
- Priority: MUST
- D-001 Goal Drift: cosine similarity between action and spec intent. Signal below 0.6.
- D-002 Shallow Reasoning: ratio evidence/claims. Signal below 0.3.
- D-003 Shortcut Detection: verification steps skipped from plan.
- Acceptance: Simulated drift → D-001 produces score < 0.6.

**FR-023: Domain Module Registry**
- Priority: MUST
- Modules registered at startup. Each declares name, perspectives, rules, verifiers.
- Multiple modules compose orthogonally.
- Acceptance: 2 modules registered. all_perspectives() has both.

**FR-024: Multi-Head Retrieval Perspectives**
- Priority: MUST
- Each perspective provides query_template for memory search.
- System generates one query per perspective, merges results, deduplicates.
- Acceptance: Auth task retrieves from both security and optimization perspectives.

**FR-025: Rules Injected into Prompts**
- Priority: MUST
- Orchestrator includes IC rules in LLM system prompt, filtered by task type.
- Acceptance: Bug fix prompt contains "minimal change" rule.

### 3.4 Language Compilation Layer

**FR-030: Five-Pass Compilation**
- Priority: MUST
- Passes: Lexical, Semantic Resolution, Terminology Normalization, Completeness Check, Ambiguity Detection.
- Acceptance: "修复登录bug" → SpecIR with normalized terms and intent.

**FR-031: Spec IR as Universal Input**
- Priority: MUST
- All downstream operations consume SpecIR, not raw user input.
- Serializable to JSON and MessagePack.
- Acceptance: Orchestrator receives SpecIR, passes to all sub-agents.

**FR-032: Completeness Gating**
- Priority: MUST
- Non-empty open_questions → Specification Cage blocks execution.
- Acceptance: Spec with open_questions → Block.

**FR-033: LLM-Assisted Compilation**
- Priority: SHOULD
- With provider: LLM performs semantic resolution. Without: basic pattern matching + open_questions.
- Acceptance: With provider resolves to specific file. Without returns open questions.

### 3.5 Multi-Agent Runtime

**FR-040: Main Agent**
- Priority: MUST
- Receives user input + context. Queries IC. Creates MessagePack task assignments. Audits sub-agent results. Uses strongest model.
- Acceptance: E2E: Main → IC query → dispatch Worker → audit result → return.

**FR-041: Worker Sub-agent**
- Priority: MUST
- Distinct execution context (not provider.complete() wrapper). Own IC rules subset. Receives/returns MessagePack.
- Acceptance: Worker system prompt differs from Main's. Worker has filtered IC rules.

**FR-042: Explorer Sub-agent**
- Priority: MUST
- Handles search/research. Searches memory (Attention), codebase, optionally web. Returns structured findings.
- Acceptance: Explorer finds auth-related files from memory and codebase.

**FR-043: Cage Sub-agent**
- Priority: MUST
- Continuously monitors Main Agent. Does not intervene in business logic. Checks D-001/D-002/D-003 scores. Reports via CageReport. May use lighter model.
- Acceptance: During long task, Cage detects simulated goal drift, generates warning.

**FR-044: Model Dispatch Policy**
- Priority: MUST
- Auto-detect capabilities. Multi-model: strongest→Main, balanced→Worker, fast→Explorer, lightest→Cage. Single-model: same model, different params.
- Acceptance: 3-model provider → each role gets different model.

**FR-045: Agent Lifecycle**
- Priority: MUST
- Spawn, collect results/timeouts, terminate after completion, handle failures.
- Acceptance: Worker timeout → error result, no hang.

### 3.6 Memory Database

**FR-050: Scoped Storage**
- Priority: MUST
- S0 Global, S1 Project, S2 Session, S3 Task. Each project independent SQLite.
- Acceptance: Cross-project isolation verified.

**FR-051: Leveled Memory (L0-L6)**
- Priority: MUST
- 7 levels with ordering. L0 survives GC. L5 collectable.
- Acceptance: Level ordering enforced. GC respects levels.

**FR-052: Ownership**
- Priority: MUST
- One owner per entry. Owner-only write/update/delete. Any-agent read. Explicit transfer.
- Acceptance: Non-owner write → PermissionDenied.

**FR-053: Reference Safety**
- Priority: MUST
- No null (None for missing). No dangling (error for deleted). Version consistency (stale detection).
- Acceptance: Each error case tested.

**FR-054: Promotion Chain Validation**
- Priority: MUST
- Valid: L2→L1, L3→L1, L4→L1, L5→L1. L6 requires reviewed=true. L0 only from L1. Backwards blocked.
- Acceptance: L1→L2 → PromotionBlocked. L6 without review → blocked.

**FR-055: Garbage Collection**
- Priority: MUST
- Remove: alive=false AND ref_count=0 AND level NOT IN (L0,L1). Never remove L0/L1. Integrity scan for orphans/dangling.
- Acceptance: Dead L5 collected. Dead L0 preserved.

**FR-056: Cache Layer**
- Priority: MUST
- Read: Bloom→moka→SQLite. Write: SQLite→cache. Update: SQLite→invalidate cache. Stats tracked.
- Acceptance: Second read is cache hit. Write updates both.

**FR-057: Bloom Filter**
- Priority: SHOULD
- In-memory for alive keys. Definite non-existence skips cache and SQLite.
- Acceptance: Nonexistent key → None without touching SQLite.

**FR-058: Attention-Based Vector Search**
- Priority: MUST
- Vector index (FAISS/usearch). Local embedding model (sentence-transformers via ONNX). Cosine similarity. Top-K ranked.
- Acceptance: 10 entries stored. Related query → top result is semantically closest.

**FR-059: Multi-Head Attention**
- Priority: MUST
- One query per IC perspective. Merge and deduplicate.
- Acceptance: Two perspectives → results from both queries.

**FR-060: Artifact Store**
- Priority: MUST
- Versioned MessagePack files in .terra/artifacts/{type}/. Immutable append. Memory stores pointers.
- Acceptance: Save v1, save v2. Load latest → v2. Load v1 → v1.

**FR-061: Compression Protection**
- Priority: MUST
- 70% threshold: externalize accepted paths, plan, key decisions, update pointers, then compress.
- Acceptance: Critical data persisted before compression, retrievable after.

### 3.7 Caging Engine

**FR-070: Cage Trait**
- Priority: MUST
- check(context) → Pass|Warn|Block. First Block short-circuits. Warn logged but allowed.
- Acceptance: Third cage blocks → fourth not invoked.

**FR-071: Specification Cage** — MUST. Blocks incomplete specs.

**FR-072: Knowledge Cage** — SHOULD. Authority ranking enforcement.

**FR-073: Memory Cage** — MUST. Ownership + promotion gates.

**FR-074: Action Cage (Sandbox)** — MUST. SandboxGuard invoked by Executor in production path.

**FR-075: Verification Cage** — MUST. Blocks unverified completion claims.

**FR-076: Convergence Cage** — SHOULD. Stops exploration after accepted path.

**FR-077: Meta-Cage** — MUST. Constitution immutable. Cage rules immutable.

**FR-078: Parametric Memory Cage** — SHOULD. L6 requires human review.

### 3.8 Provider Dispatch

**FR-080: Provider Adapter Trait** — MUST.
**FR-081: DryRun Mode** — MUST.
**FR-082: OpenAI-Compatible Adapter** — MUST.
**FR-083: Retry with Backoff** — MUST. 3 attempts, 1s/2s/4s.
**FR-084: Streaming** — SHOULD. Mid-stream interruption.
**FR-085: Capability Matrix** — MUST. Used for model dispatch.

### 3.9 Audit Trail

**FR-090: Sequential Logging** — MUST. No gaps.
**FR-091: Persistent Storage** — MUST. SQLite.
**FR-092: Markdown Rendering** — MUST.
**FR-093: Command Replay** — MUST. Reproduce state.
**FR-094: Concurrent Safety** — MUST. 10 threads safe.

### 3.10 Session Lifecycle

**FR-100: Session Start** — MUST. Set project, create S2, record goal.
**FR-101: Session Switch** — MUST. Flush→clear→load.
**FR-102: Session End** — MUST. Trigger Sleep, clean S3, flush cache.
**FR-103: Sleep Consolidation** — MUST. Clean→abstract→promote candidates→user confirm.

### 3.11 Sandbox

**FR-110: File System Boundaries** — MUST. Inside write, outside read-only, protected read-only.
**FR-111: Enforced by Executor** — MUST. Not just unit-tested.
**FR-112: Symlink Escape Prevention** — SHOULD. Canonicalize paths.

### 3.12 Configuration

**FR-120: Layered Config** — MUST. env > project > global > defaults.
**FR-121: Config Show/Set** — MUST. API keys redacted.
**FR-122: Environment Detection** — MUST. Full component check.

### 3.13 Skill System

**FR-130: Skill Registration** — SHOULD. MANIFEST.yml.
**FR-131: Skill Communication** — SHOULD. MessagePack stdio.
**FR-132: Superpower Skill** — SHOULD. Built-in engineering standards.

### 3.14 Communication Protocol

**FR-140: Agent-Agent MessagePack** — MUST.
**FR-141: Human-Agent Natural Language** — MUST.
**FR-142: MessagePack-to-Markdown Rendering** — MUST.

---

## 4. Non-Functional Requirements

| ID | Priority | Requirement |
|----|----------|-------------|
| NFR-001 | MUST | Token efficiency: MessagePack, ~15 tokens/command |
| NFR-002 | MUST | CLI-first, all commands --as-json |
| NFR-003 | MUST | Offline dry_run, local embedding |
| NFR-004 | MUST | Provider agnostic |
| NFR-005 | MUST | Elegant code, no .unwrap(), all APIs documented |
| NFR-006 | MUST | Every action audited, logs replayable |
| NFR-007 | MUST | Credentials env-only, sandbox enforced, constitution immutable |
| NFR-008 | SHOULD | Cache <1ms, SQLite <5ms, msgpack <100μs, vector <10ms |
| NFR-009 | MUST | All ops return Result, cage violations logged not panicked |
| NFR-010 | SHOULD | Extensible via Skills and Provider trait |
| NFR-011 | MUST | Layered architecture enforced, no circular deps |

---

## 5. Interface Requirements

### CLI
```
terra, terra doctor, terra run, terra memory read/search,
terra trace show/replay, terra config show/set,
terra skill list, terra session start/end, terra --help/--version
```

### Skill: MessagePack stdin/stdout, JSON stderr
### Provider: POST /v1/chat/completions (OpenAI-compatible)
