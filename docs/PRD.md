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

---

## Patch 1 — Dual Mode, Memory Permissions, Sleep Cycle, Output Cage

> Patch ID: TERRA-PRS-002-P1
> Date: 2026-05-22
> Scope: New functional requirements extending v2 baseline.
> Rule: This patch only adds. No existing FR is modified.

---

### P1.1 Conversation/Goal Dual Mode

Terra operates in two modes. Conversation Mode is the default.
Goal Mode is explicitly activated by the user.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-143 | MUST | Terra SHALL support two runtime modes: Conversation Mode (default) and Goal Mode (user-activated). |
| FR-144 | MUST | Goal Mode SHALL be activated by explicit user command (`/goal "<description>"`). |
| FR-145 | MUST | Goal Mode SHALL activate: Specification Cage, Verification Cage, Convergence Cage, D-001/D-002/D-003 drift detection. |
| FR-146 | MUST | Conversation Mode SHALL keep active: Meta-Cage, Memory Cage, Action Cage (sandbox), Audit (lightweight). |
| FR-147 | MUST | Conversation Mode SHALL NOT require SpecIR compilation or Specification Cage gating. |
| FR-148 | MUST | Mode transitions SHALL be recorded in the Audit log. |
| FR-149 | SHOULD | Agent MAY suggest Goal Mode when task intent is detected in Conversation Mode. User confirms before switch. |

### P1.2 Memory Write Permissions by Mode and Level

Agent write access is constrained by memory level. Persistent knowledge
requires promotion + user verdict. Cage enforcement is programmatic.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-150 | MUST | L0 (Authoritative) SHALL be read-only in all modes. Enforced by Memory Cage. |
| FR-151 | MUST | L1 (Project Knowledge) SHALL only be written via promotion chain + user verdict. No direct agent write. |
| FR-152 | MUST | L2 (Task Scope) SHALL be writable by agent in both modes. Cleaned on session end. |
| FR-153 | MUST | L3 (Accepted Paths) SHALL be writable in Goal Mode only, gated by Verification Cage pass. |
| FR-154 | MUST | L4 (Failed Paths) SHALL be writable by agent in both modes (auto-recorded on failure). |
| FR-155 | MUST | L5 (Dormant Ideas) SHALL be writable by agent in both modes. This is the buffer layer for observations. |
| FR-156 | MUST | L6 (Model Candidates) SHALL be writable by agent in both modes. Agent nominates candidates for user review. |
| FR-157 | MUST | When agent writes to L5 or L6, a fixed-format notification SHALL be emitted: `[Terra] Recorded → L{n}: "{summary}"`. This notification is program-generated, not LLM-generated. |

### P1.3 Sleep Consolidation Cycle

Sleep consolidation is a configurable periodic process that abstracts
concrete actions into reusable principles via the L0-L6 promotion chain.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-158 | MUST | Sleep consolidation SHALL be triggered by one of: session end, configurable interval, or manual `/sleep` command. |
| FR-159 | MUST | Consolidation trigger mode and interval SHALL be configurable in config.toml under `[sleep]` section. |
| FR-160 | MUST | Consolidation SHALL scan L2 + L5 entries, match against IC rules, group by pattern, and produce L6 candidates. |
| FR-161 | MUST | Consolidation results SHALL be presented to user as a candidate list. User accepts (→ promote to L1/L3) or rejects (→ discard or remain L5). |
| FR-162 | MUST | User verdicts on promotion candidates SHALL be recorded in Audit log. |
| FR-163 | SHOULD | Consolidation SHALL respect `max_candidates` config to limit single-batch output. |

### P1.4 Output Cage

Current caging only audits input commands. Output Cage audits model responses.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-164 | MUST | An Output Cage SHALL inspect ProviderCall responses before they are returned to the agent. |
| FR-165 | MUST | Output Cage SHALL block responses that contain instructions to bypass caging, modify constitution, or escalate permissions. |
| FR-166 | SHOULD | Output Cage SHALL flag responses that reference operations outside the current agent's permitted CommandOp set. |

### P1.5 Active Drift Detection (Goal Mode Only)

D-001 drift detection transitions from passive (on-demand) to active
(automatic) when in Goal Mode.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-167 | MUST | In Goal Mode, D-001 (goal drift) SHALL be evaluated automatically at configurable intervals (default: every 5 commands). |
| FR-168 | MUST | When D-001 score falls below threshold, execution SHALL pause and agent SHALL re-align with spec before continuing. |
| FR-169 | MUST | D-002 (shallow reasoning) and D-003 (shortcut detection) SHALL be evaluated at checkpoints in Goal Mode. |
| FR-170 | MUST | Drift detection results SHALL be recorded in Audit log. Drift events SHALL NOT consume LLM tokens for justification unless cage requests it. |

### P1.6 User Commands

Fixed set of user-facing commands. Program-processed, no LLM token cost.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-171 | MUST | `/goal "<description>"` SHALL activate Goal Mode with the given description as spec input. |
| FR-172 | MUST | `/done` SHALL end current Goal Mode, trigger sleep consolidation, return to Conversation Mode. |
| FR-173 | MUST | `/status` SHALL display: current mode, goal (if any), drift score (if Goal Mode), session duration. |
| FR-174 | MUST | `/remember "<content>"` SHALL write content to Memory L1 (Project) directly, bypassing promotion chain. This is explicit user intent. |
| FR-175 | MUST | `/forget <key>` SHALL soft-delete a memory entry. Requires ownership or user authority. |
| FR-176 | MUST | `/sleep` SHALL manually trigger sleep consolidation. |
| FR-177 | MUST | `/trace` SHALL display current session audit log summary. |
| FR-178 | SHOULD | All user commands SHALL be processed without LLM invocation. They are program-handled at CLI layer. |

### P1.7 Known Issues (from Phase 1-5 Review)

Documented for tracking. These are defects in the current implementation,
not new features.

| ID | Category | Description |
|----|----------|-------------|
| ISSUE-001 | Architecture | Layer violation: terra-memory (L2) depends on terra-protocol (L3). Command types should be in shared L0 crate. |
| ISSUE-002 | Architecture | Layer violation: terra-trace (L2) depends on terra-protocol (L3). Same root cause as ISSUE-001. |
| ISSUE-003 | Architecture | Layer violation: terra-providers (L1) depends on terra-protocol (L3). Same root cause as ISSUE-001. |
| ISSUE-004 | Testing | Test ID prefix inconsistency: T-PT, T-MD, T-AT tests exist but use non-standard prefixes (T-C-, T-TR-, etc). |
| ISSUE-005 | Feature | Embedding not integrated. Vector search uses TF-IDF fallback. ONNX Runtime (ort) + usearch not added. |
| ISSUE-006 | Feature | Audit log persistence is in-memory only. Persistent SQLite storage exists but not connected. |
| ISSUE-007 | Feature | Agent inter-communication MessagePack pipeline not fully connected end-to-end. |
| ISSUE-008 | Feature | Streaming provider response not implemented (capability advertised but not functional). |

---

## Patch 2 — Skill Data Flow, Streaming Interaction, Live E2E

> Patch ID: TERRA-PRS-002-P2
> Date: 2026-05-06
> Scope: Three critical capabilities that make Terra usable.
> Rule: This patch only adds. No existing FR is modified.

---

### P2.1 Skill Execution Data Flow

SkillPlan steps execute sequentially. Each step's output is available
to subsequent steps. Without this, multi-step Skills like CodeEditor
cannot pass LLM output to FileWrite.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-179 | MUST | SkillPlan execution SHALL pass each step's CommandResult.data to the next step as `prev_result`. |
| FR-180 | MUST | A SkillStep MAY declare `uses_prev: true`. When set, the step's CommandOp is constructed at runtime from prev_result, not statically. |
| FR-181 | MUST | CodeEditor Skill step 3 (FileWrite) SHALL use the LLM response content from step 2 (ProviderCall) as its file content. |
| FR-182 | MUST | If a step fails, subsequent steps SHALL NOT execute. SkillResult reports which step failed. |
| FR-183 | SHOULD | SkillPlan MAY define a `rollback` step that executes on failure (e.g., restore original file). |

### P2.2 Streaming Interaction

Terra SHALL support streaming output and interactive conversation.
This is the core of usability. Without it, Terra is a batch processor.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-184 | MUST | ProviderCall SHALL support streaming mode. Tokens are emitted as they arrive, not buffered to completion. |
| FR-185 | MUST | Streaming output SHALL be displayable in terminal as tokens arrive. |
| FR-186 | MUST | User SHALL be able to interrupt a running ProviderCall (Ctrl+C or equivalent). Partial result is preserved in audit. |
| FR-187 | MUST | In Conversation Mode, Terra SHALL accept user input as a continuous dialogue, not one-shot commands. |
| FR-188 | MUST | In Goal Mode, user SHALL be able to converse with the agent mid-task to provide guidance, clarification, or corrections. Goal Mode is additive over conversation, not a replacement. |
| FR-189 | MUST | Progress feedback SHALL be displayed during multi-step operations: which step, what it's doing, elapsed time. |
| FR-190 | SHOULD | Token count and cost SHALL be displayed in real-time during interaction (footer or status line). |
| FR-191 | SHOULD | Terra SHALL support an event-based output model: each operation emits structured events that a UI layer can consume (terminal, TUI, or web). |

### P2.3 End-to-End Live Pipeline

The full pipeline SHALL be verified with a real LLM provider,
not just DryRunAdapter.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-192 | MUST | CodeEditor Skill SHALL be verified end-to-end: read a real file → LLM modifies it → write back → file content changed. |
| FR-193 | MUST | TestRunner Skill SHALL be verified: run `cargo test` → parse result → report pass/fail. |
| FR-194 | MUST | OutputContract parsing SHALL be verified against real LLM output (not mock JSON). |
| FR-195 | MUST | Output Cage SHALL be verified against real LLM responses (no false positives on normal coding output). |
| FR-196 | MUST | Full pipeline: user task → compile → IC → cage → worker (CodeEditor) → result → audit. Verified with real Provider. |

### P2.4 Known Issues (from V2 comparison review)

| ID | Category | Description |
|----|----------|-------------|
| ISSUE-009 | Architecture | SkillPlan steps have no data flow — step 2 output cannot feed step 3 input. |
| ISSUE-010 | Architecture | No streaming support — all ProviderCall responses are buffered. |
| ISSUE-011 | Architecture | No interactive dialogue — CLI is one-shot `terra run --task`. |
| ISSUE-012 | Architecture | Constitution loaded but not used to drive cage behavior (hardcoded). |
| ISSUE-013 | Architecture | No context window management / compression protection. |
| ISSUE-014 | Testing | 395 tests all use DryRunAdapter — zero tests against real LLM. |

---

## Patch 3 — Session Persistence, Dynamic Planning, Interactive Quality, Product Packaging

> Patch ID: TERRA-PRS-002-P3
> Date: 2026-05-06
> Scope: Session management, dynamic self-evolution, TUI, compaction, distribution.
> Rule: This patch only adds. No existing FR is modified.

---

### P3.1 Session Persistence + History

Sessions SHALL be persisted to disk and recoverable across restarts.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-197 | MUST | Each session SHALL be assigned a unique ID and persisted to `.terra/sessions/{id}.json`. |
| FR-198 | MUST | Session file SHALL contain: all conversation turns (role + content + timestamp), mode history, token consumption, goal (if any). |
| FR-199 | MUST | `terra chat` SHALL resume the last active session by default. `terra chat --new` starts fresh. |
| FR-200 | MUST | `/sessions` command SHALL list past sessions with date, turn count, and token summary. |
| FR-201 | MUST | `terra chat --resume {id}` SHALL restore a specific historical session, including full context. |
| FR-202 | MUST | Session context SHALL be loaded into LLM prompt on resume (subject to compaction if too long). |
| FR-203 | SHOULD | Session files SHALL be human-readable (JSON with indentation). |

### P3.2 Dynamic Self-Evolution (Conversational Planning)

Terra SHALL support dynamic, conversation-driven planning.
Goals and plans evolve through dialogue, not static specification.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-204 | MUST | In Goal Mode, Terra (LLM) SHALL be able to propose a plan (manifest/criteria) based on the goal description and project context. |
| FR-205 | MUST | User SHALL be able to confirm, modify, or reject the proposed plan within the conversation. No /done + /goal restart needed. |
| FR-206 | MUST | When user provides mid-task correction ("change direction", "skip this", "try a different approach"), Terra SHALL dynamically update the active SpecIR and execution plan. |
| FR-207 | MUST | Plan versions SHALL be recorded in audit log. Previous plans recoverable. |
| FR-208 | MUST | SpecIR SHALL support mutation after initial compilation: add/remove constraints, update target, add open_questions. |
| FR-209 | SHOULD | Terra MAY proactively suggest plan adjustments when drift is detected or new information is discovered during execution. |
| FR-210 | SHOULD | Dynamic plan changes SHALL NOT require exiting Goal Mode. The mode is additive — conversation continues while plan evolves. |

### P3.3 Interactive Quality

Basic interaction quality that every CLI product must have.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-211 | MUST | Input history SHALL be available via up/down arrow keys. Persisted across sessions. |
| FR-212 | MUST | Ctrl+C SHALL cancel the current LLM call (not exit the process). Partial result preserved. |
| FR-213 | MUST | `/commands` SHALL support tab completion. |
| FR-214 | MUST | Cost estimation SHALL be displayed: token count + estimated cost per model. |
| FR-215 | SHOULD | Spinner/progress indicator while waiting for LLM response. |

### P3.4 Context Compaction

Long conversations SHALL not crash or degrade.

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-216 | MUST | When conversation context exceeds 70% of model's max_context_tokens, compaction SHALL be triggered automatically. |
| FR-217 | MUST | Compaction SHALL: externalize key decisions/facts to Memory (L2/L5), summarize conversation history, reduce token count below 50%. |
| FR-218 | MUST | Compaction SHALL preserve: current goal, active plan, recent turns (last 5). |
| FR-219 | MUST | Compaction events SHALL be logged in audit. User notified: "[Terra] Context compacted: N turns summarized." |
| FR-220 | SHOULD | User MAY trigger compaction manually via `/compact`. |

### P3.5 TUI + Logo + Visual Identity

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-221 | SHOULD | Terra SHALL have an ASCII art logo rendered at startup. |
| FR-222 | SHOULD | TUI layout SHALL include: input area (bottom), output area (scrollable), status bar (mode + tokens + cost). |
| FR-223 | SHOULD | Code blocks in LLM responses SHALL be syntax-highlighted. |
| FR-224 | MAY | Pet status indicator as optional decorative element (decoupled, plug-in). |

### P3.6 Product Packaging

| FR | Priority | Requirement |
|----|----------|-------------|
| FR-225 | SHOULD | `cargo install terra-cli` SHALL work from crates.io. |
| FR-226 | MAY | Pre-compiled binaries available via GitHub Releases for macOS/Linux/Windows. |
| FR-227 | MAY | npm/bun wrapper package for `npx terra` convenience. |

### P3.7 Known Issues

| ID | Category | Description |
|----|----------|-------------|
| ISSUE-015 | UX | No session persistence — conversation lost on exit. |
| ISSUE-016 | Architecture | Static pipeline — SpecIR compiled once, never evolves mid-conversation. |
| ISSUE-017 | UX | No input history (no up arrow). |
| ISSUE-018 | UX | Ctrl+C kills process instead of cancelling current call. |
| ISSUE-019 | Architecture | Context grows unbounded — no compaction. 100 turns will exceed model limit. |
| ISSUE-020 | UX | No cost estimation display. |
