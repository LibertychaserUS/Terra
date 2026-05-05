# Terra Product Requirements Specification

> Document ID: TERRA-PRS-001
> Version: 1.0.0
> Status: DRAFT
> Date: 2026-05-20
> Author: Oliver Zhang, Claude (Anthropic)
> Classification: Public
>
> This document follows IEEE 830-1998 (Recommended Practice for
> Software Requirements Specifications) with RFC 2119 keyword
> conventions. The key words "MUST", "MUST NOT", "REQUIRED",
> "SHALL", "SHALL NOT", "SHOULD", "SHOULD NOT", "RECOMMENDED",
> "MAY", and "OPTIONAL" are to be interpreted as described in
> RFC 2119.

---

## Table of Contents

1. Introduction
2. Overall Description
3. Functional Requirements
4. Non-Functional Requirements
5. Interface Requirements
6. System Constraints
7. Acceptance Criteria
8. Appendices

---

## 1. Introduction

### 1.1 Purpose

This document specifies the product requirements for Terra, an
Agent Operating System (Agent OS) for reliable AI agent execution.
It is intended for use by developers, testers, and stakeholders
involved in the design, implementation, and validation of Terra.

### 1.2 Scope

Terra is a runtime environment that manages AI agent lifecycle,
memory, permissions, verification, and inter-agent communication.
It is analogous to a general-purpose operating system, where:

- Processes → Agents (Main, Worker, Explorer, Cage)
- Memory management → Memory Database (scoped, leveled, Rust-safe)
- Syscall interface → Command Protocol
- File system → Artifact Store + Sandbox
- IPC → MessagePack messages (Terra Internal Protocol)
- Kernel → Caging Engine (8 cages)
- Firmware → Intelligence Core (first principles)
- Device drivers → Provider Adapters (LLM endpoints)
- Syslog → Audit Trail

The initial release (MVP) targets the **software engineering** domain.
General-purpose agent workloads are out of scope for this version.

### 1.3 Definitions and Acronyms

| Term | Definition |
|------|-----------|
| Agent OS | Operating system for AI agents; manages runtime, memory, permissions |
| Cage | A constraint mechanism that shapes an agent's reachable state space (from Caging Engineering) |
| CE | Caging Engineering — reachability-oriented framework for reliable AI agent systems |
| Spec IR | Specification Intermediate Representation — normalized task description |
| TIP | Terra Internal Protocol — MessagePack-based inter-agent communication |
| Provider | An LLM endpoint (OpenAI-compatible) |
| Skill | A self-contained domain capability package registered with Terra |
| Scope | Memory isolation boundary: Global (S0), Project (S1), Session (S2), Task (S3) |
| Level | Memory semantic classification: L0 (Authoritative) through L6 (Model Candidates) |
| Promotion | The process of elevating a memory entry from a lower level to a higher level, subject to verification gates |
| Sleep Consolidation | Post-session process that abstracts concrete actions into general principles |
| SandboxGuard | File system permission enforcer: write inside project root, read-only outside |

### 1.4 References

| ID | Document |
|----|----------|
| REF-01 | IEEE 830-1998, Recommended Practice for Software Requirements Specifications |
| REF-02 | RFC 2119, Key words for use in RFCs to Indicate Requirement Levels |
| REF-03 | Zhang, O. "Caging Engineering v0.1.0", 2026. github.com/LibertychaserUS/caging-engineering |
| REF-04 | POSIX.1-2024, The Open Group Base Specifications |
| REF-05 | ARCHITECTURE.md — Terra technical architecture blueprint |
| REF-06 | VISION_OLIVER.md — Original design vision and rationale |

### 1.5 Overview

Section 2 provides a high-level description of the product.
Sections 3 and 4 enumerate functional and non-functional requirements
with unique identifiers, priorities, and acceptance criteria.
Section 5 defines external interfaces. Section 6 states constraints.
Section 7 provides cross-cutting acceptance criteria.

---

## 2. Overall Description

### 2.1 Product Perspective

Terra occupies a layer **below** agent frameworks (LangChain, CrewAI)
and **above** LLM providers (OpenAI, Anthropic, local models).

```
┌────────────────────────┐
│ User / Application     │
├────────────────────────┤
│ Agent Frameworks       │  ← optional; agents can run directly on Terra
├────────────────────────┤
│ Terra (Agent OS)       │  ← THIS PRODUCT
├────────────────────────┤
│ LLM Providers          │
├────────────────────────┤
│ Hardware / OS          │
└────────────────────────┘
```

### 2.2 Product Functions (Summary)

| Function | Description |
|----------|-------------|
| F1. Intelligence Core | Built-in first principles for agent reasoning and behavior |
| F2. Language Compilation | User natural language → Spec IR normalization |
| F3. Agent Orchestration | Main/Sub/Cage agent lifecycle management |
| F4. Memory Management | Scoped, leveled, Rust-safe memory database |
| F5. Command Protocol | Fixed command set for all agent-system interactions |
| F6. Caging Engine | 8-cage constraint system from Caging Engineering |
| F7. Sandbox | File system isolation and permission enforcement |
| F8. Provider Dispatch | LLM adapter with capability detection and retry |
| F9. Audit Trail | Sequential command logging with replay capability |
| F10. Sleep Consolidation | Post-session abstraction and promotion |
| F11. Configuration | User settings, environment detection, project config |
| F12. Skill System | Domain capability registration and loading |

### 2.3 User Classes

| Class | Interaction Mode | Priority |
|-------|-----------------|----------|
| Software Engineer | CLI, direct task execution | Primary (MVP) |
| Skill Developer | MANIFEST.yml + MessagePack stdio | Secondary |
| Agent Operator | Config management, monitoring | Future |

### 2.4 Operating Environment

| Component | Requirement |
|-----------|-------------|
| OS | macOS 11+ (Apple Silicon), Linux (glibc 2.28+, x86_64/aarch64) |
| Runtime | Rust 1.75+ (compiled binary, no runtime dependency) |
| LLM Provider | OPTIONAL. Any OpenAI-compatible endpoint. System MUST operate in dry_run mode without a provider |
| Network | OPTIONAL. Required only for remote LLM calls and embedding models |
| Disk | SQLite databases + MessagePack artifacts. Estimated <100MB per project |

### 2.5 Design and Implementation Constraints

| ID | Constraint | Rationale |
|----|-----------|-----------|
| DC-01 | Implementation language SHALL be Rust (2021 edition) | Memory safety, performance, zero-cost abstractions |
| DC-02 | All inter-agent communication SHALL use MessagePack serialization | Token efficiency, compact binary format |
| DC-03 | All persistent storage SHALL use SQLite with WAL mode | Embedded, ACID, no external service dependency |
| DC-04 | The system MUST operate without network connectivity in dry_run mode | Offline development support |
| DC-05 | API credentials MUST be resolved from environment variables only | Security: never touch disk |
| DC-06 | The system SHALL NOT modify files outside the project sandbox root | Safety: bounded mutation |

### 2.6 Assumptions and Dependencies

| ID | Assumption |
|----|-----------|
| A-01 | Users have a Rust toolchain installed (for building from source) or use pre-built binaries |
| A-02 | At least one OpenAI-compatible provider is available for live mode |
| A-03 | Project directories follow standard software project layouts |
| A-04 | Users interact primarily through CLI in a terminal environment |

---

## 3. Functional Requirements

### 3.1 Intelligence Core

**FR-IC-001: Universal Reasoning Framework**
- Priority: MUST
- The system SHALL provide a built-in reasoning framework that
  defines how agents approach problems.
- The framework SHALL include rules for: depth-first exploration,
  evidence-based decisions, and complexity decomposition.
- The framework SHALL be read-only at runtime (L0 Authoritative).
- Acceptance: Agent behavior demonstrably follows framework rules
  in test scenarios.

**FR-IC-002: Universal Behavioral Framework**
- Priority: MUST
- The system SHALL provide behavioral rules including:
  elegant code over complex code, minimal change principle,
  safe defaults, and project pattern consistency.
- Acceptance: Agent-generated code adheres to behavioral rules
  in test scenarios.

**FR-IC-003: Self-Diagnostic Rules**
- Priority: MUST
- The system SHALL detect and report:
  (a) goal drift — agent deviating from stated objective,
  (b) shallow reasoning — insufficient analysis depth,
  (c) shortcut-taking — bypassing verification or safety steps.
- Acceptance: Each diagnostic produces a detectable signal in
  test scenarios.

**FR-IC-004: Domain Module Registry**
- Priority: MUST
- The system SHALL support registration of domain-specific
  capability modules (e.g., `coding/`, `algorithm/`).
- Each module SHALL declare: perspectives, rules, and verifiers.
- Multiple modules SHALL be composable (active simultaneously).
- Acceptance: Two modules registered, both contribute perspectives
  to a single retrieval query.

**FR-IC-005: Perspective Registration**
- Priority: SHOULD
- Each domain module SHALL register one or more Perspectives.
- Perspectives generate specialized query vectors for
  Multi-Head Attention retrieval.
- Acceptance: Multi-head search returns results from multiple
  module perspectives.

### 3.2 Language Compilation Layer

**FR-LC-001: Spec IR Generation**
- Priority: MUST
- The system SHALL transform a user task description (natural language
  string) into a SpecIR structure containing: spec_id, intent,
  target, constraints, success_predicate, open_questions,
  and resolved_terms.
- Acceptance: `SpecIR::from_task_description("fix the login bug")`
  produces a valid SpecIR with non-empty intent.

**FR-LC-002: Completeness Detection**
- Priority: MUST
- The system SHALL identify when a SpecIR has unresolved
  questions (missing target, ambiguous scope, etc.).
- `SpecIR::is_complete()` SHALL return false when
  `open_questions` is non-empty.
- Acceptance: Incomplete specs detected in test scenarios.

**FR-LC-003: Terminology Normalization**
- Priority: SHOULD
- The system SHOULD map user terminology to project-specific
  normalized terms (e.g., "login button" → "LoginButton component").
- Mappings SHALL be stored in `resolved_terms`.
- Acceptance: Term mapping round-trips through serialization.

**FR-LC-004: LLM-Assisted Compilation**
- Priority: MAY (future)
- The system MAY use an LLM provider to perform semantic resolution,
  ambiguity detection, and completeness checking.
- This capability MUST be optional; the system SHALL operate
  with basic compilation when no provider is available.

### 3.3 Multi-Agent Runtime

**FR-AG-001: Main Agent**
- Priority: MUST
- The system SHALL instantiate a Main Agent that:
  (a) receives user input and full dialogue context,
  (b) queries the Intelligence Core for applicable rules,
  (c) creates task assignments for Sub-agents,
  (d) audits Sub-agent results before returning to user.
- Acceptance: End-to-end test with DryRunAdapter completes successfully.

**FR-AG-002: Worker Sub-agent**
- Priority: MUST
- The system SHALL support Worker Sub-agents that execute
  specific coding/modification tasks.
- Workers SHALL receive structured TaskPayload (MessagePack)
  and return structured ResultPayload.
- Acceptance: Worker receives task, calls provider, returns result.

**FR-AG-003: Explorer Sub-agent**
- Priority: SHOULD
- The system SHOULD support Explorer Sub-agents for
  search, research, and information gathering.
- Acceptance: Explorer receives search query, returns findings.

**FR-AG-004: Cage Sub-agent**
- Priority: SHOULD
- The system SHOULD support a Cage Sub-agent that monitors
  the Main Agent's execution without intervening in business logic.
- The Cage Sub-agent SHALL check for: goal drift, reasoning depth
  degradation, rule violations, and shortcut-taking.
- Acceptance: Cage agent detects a simulated goal drift scenario.

**FR-AG-005: Model Dispatch Policy**
- Priority: MUST
- The system SHALL auto-detect provider capabilities and assign
  models to agent roles:
  - Multi-model provider: strongest → Main, balanced → Worker,
    fast → Explorer
  - Single-model provider: same model, differentiated by parameters
    (max_tokens, temperature).
- Acceptance: Policy correctly assigns models in both scenarios.

### 3.4 Memory Database

**FR-MD-001: Scoped Storage**
- Priority: MUST
- The system SHALL maintain four memory scopes:
  S0 (Global), S1 (Project), S2 (Session), S3 (Task).
- Each project SHALL have an independent SQLite database.
- Acceptance: Two projects have isolated memory; writes to one
  are invisible to the other.

**FR-MD-002: Leveled Memory**
- Priority: MUST
- Within each scope, memory entries SHALL be classified by level:
  L0 (Authoritative), L1 (Project), L2 (Task), L3 (Accepted Paths),
  L4 (Failed Path Index), L5 (Dormant Ideas), L6 (Model Candidates).
- Acceptance: Entries created at each level; level constraints enforced.

**FR-MD-003: Ownership**
- Priority: MUST
- Each memory entry SHALL have exactly one owner (the creating agent/task).
- Only the owner MAY modify or delete the entry.
- Non-owners MAY read the entry.
- Acceptance: Non-owner write attempt returns PermissionDenied.

**FR-MD-004: Reference Safety**
- Priority: MUST
- Memory pointers SHALL NOT be null.
  - Read of nonexistent key SHALL return `None` (explicit absence),
    not a null pointer or silent failure.
- Memory pointers SHALL NOT dangle.
  - Read through a pointer to a deleted entry SHALL return
    `MemoryError::Dangling`, not stale data.
- Version mismatch SHALL be detectable.
  - Read with expected version N when current version is N+1
    SHALL return `MemoryError::Stale`.
- Acceptance: Each error case tested explicitly.

**FR-MD-005: Garbage Collection**
- Priority: SHOULD
- The system SHOULD implement garbage collection via:
  (a) reference counting (real-time, decremented on pointer removal),
  (b) periodic integrity scan (detect orphaned entries, dangling pointers).
- L0 and L1 entries SHALL NOT be garbage collected regardless
  of reference count.
- Acceptance: GC removes zero-refcount L5 entry; preserves L0 entry.

**FR-MD-006: Promotion Chain**
- Priority: MUST
- Memory entries SHALL be promotable between levels:
  L2→L1, L3→L1, L4→L1 (project knowledge),
  L1→L0 (if verified repeatedly).
- L6 promotion SHALL REQUIRE explicit human review.
- Acceptance: Promotion gate blocks L6→L0 without review flag.

**FR-MD-007: Cache Layer**
- Priority: SHOULD
- The system SHOULD maintain an in-memory cache (moka)
  for the active project's working set.
- Cache hits SHALL bypass SQLite I/O.
- Writes SHALL update both cache and SQLite (write-through for
  critical writes, write-back for non-critical).
- Acceptance: Cache hit measured faster than SQLite read.

**FR-MD-008: Attention-Based Retrieval**
- Priority: SHOULD
- The system SHOULD support semantic search via embedding vectors.
- Query vectors SHALL be computed at retrieval time from the
  current context (not stored statically).
- Multi-Head retrieval: each Domain Module perspective generates
  a separate query; results are merged and deduplicated.
- Acceptance: Semantic search returns relevant entries ranked by score.

**FR-MD-009: Artifact Store**
- Priority: MUST
- Plan, Spec, Test, and Review artifacts SHALL be stored as
  versioned MessagePack files in `project/.terra/artifacts/`.
- The Memory Database SHALL store pointers to artifacts,
  not the artifact content itself.
- Acceptance: Artifact saved, loaded by version, pointer resolves.

**FR-MD-010: Compression Protection**
- Priority: SHOULD
- When context utilization reaches 70%, the system SHOULD trigger
  proactive externalization:
  (a) write accepted paths to Memory Database,
  (b) write current plan to Artifact Store,
  (c) update pointer table,
  (d) then allow context compression.
- Acceptance: Critical data survives a simulated compression event.

### 3.5 Command Protocol

**FR-CP-001: Fixed Command Set**
- Priority: MUST
- The system SHALL define a fixed set of commands (~15) covering
  all agent-system interactions.
- Agents SHALL NOT generate database code, SQL, or file I/O code.
  They SHALL only issue commands from this set.
- Commands: mem_write, mem_read, mem_update, mem_promote,
  mem_archive, mem_search, artifact_save, artifact_load,
  artifact_list, trace_event, trace_checkpoint,
  session_start, session_switch, session_end.
- Acceptance: Each command type created, serialized, and executed.

**FR-CP-002: Command Auditing**
- Priority: MUST
- Every command SHALL be recorded in the audit log before execution.
- Every command result SHALL be recorded after execution.
- Records SHALL include: sequence number, timestamp, agent,
  command, result, and duration.
- Acceptance: Audit log contains entry for every executed command.

**FR-CP-003: Command Replay**
- Priority: SHOULD
- The audit log SHOULD support replay: re-executing a sequence
  of recorded commands to reproduce system state.
- Acceptance: Replay of N commands produces same final state.

**FR-CP-004: Markdown Rendering**
- Priority: MUST
- Commands and results SHALL be renderable to human-readable
  Markdown via a deterministic program (no LLM involvement).
- Acceptance: Rendered markdown contains all key fields.

### 3.6 Caging Engine

**FR-CG-001: Cage Interface**
- Priority: MUST
- Each cage SHALL implement a common interface:
  `check(state) → Pass | Warn(reason) | Block(reason)`.
- Block results SHALL prevent the associated action.
- Warn results SHALL log but allow the action.
- Acceptance: Each result type handled correctly.

**FR-CG-002: Specification Cage**
- Priority: MUST
- The Specification Cage SHALL block task execution when
  the SpecIR is incomplete (has open questions).
- Acceptance: Incomplete spec → Block; complete spec → Pass.

**FR-CG-003: Memory Cage**
- Priority: MUST
- The Memory Cage SHALL enforce ownership rules (FR-MD-003)
  and promotion gates (FR-MD-006).
- Acceptance: Non-owner write blocked; ungated promotion blocked.

**FR-CG-004: Action Cage / Sandbox**
- Priority: MUST
- The Action Cage SHALL enforce file system boundaries:
  (a) writes MUST be within the project sandbox root,
  (b) reads MAY access any path,
  (c) writes to protected paths (.terra/constitution) MUST be blocked,
  (d) deletes outside sandbox root MUST be blocked.
- Acceptance: Each boundary tested with specific path scenarios.

**FR-CG-005: Verification Cage**
- Priority: MUST
- The Verification Cage SHALL block task completion claims
  when no external verifier has passed.
- Acceptance: Claim without verification → Block;
  claim with verification → Pass.

**FR-CG-006: Meta-Cage**
- Priority: MUST
- The Meta-Cage SHALL detect and block attempts to modify:
  constitution rules, cage definitions, verification rules,
  and permission boundaries.
- Acceptance: Modification attempt detected and blocked.

**FR-CG-007: Convergence Cage**
- Priority: SHOULD
- The Convergence Cage SHOULD enforce stop conditions after
  a valid path is found, preventing endless exploration.
- Acceptance: Agent stopped after accepted path recorded.

**FR-CG-008: Parametric Memory Cage**
- Priority: SHOULD
- The Parametric Memory Cage SHOULD gate L6 → model memory
  promotion, requiring explicit human review.
- Acceptance: Promotion without review flag → Block.

### 3.7 Provider Dispatch

**FR-PD-001: Provider Adapter Trait**
- Priority: MUST
- The system SHALL define a `ProviderAdapter` trait with:
  `complete(request) → CompletionResponse` and
  `capabilities() → ProviderCapability`.
- Acceptance: DryRunAdapter and OpenAICompatibleAdapter
  both implement the trait.

**FR-PD-002: DryRun Mode**
- Priority: MUST
- The system SHALL support a DryRun mode that returns
  deterministic responses without making API calls.
- DryRun SHALL work without network connectivity or API keys.
- Acceptance: DryRun produces consistent output; zero token usage.

**FR-PD-003: OpenAI-Compatible Adapter**
- Priority: MUST (MVP2)
- The system SHALL implement an adapter for OpenAI-compatible
  endpoints (OpenAI, DashScope, local models).
- The adapter SHALL support: API key from env var, base URL override,
  model selection, temperature, max_tokens.
- Acceptance: Successful completion against a live endpoint.

**FR-PD-004: Retry with Backoff**
- Priority: SHOULD
- The adapter SHOULD retry failed requests with exponential backoff.
- Maximum 3 retry attempts. Rate limit (HTTP 429) MUST trigger retry.
- Acceptance: Mock 429 response → retry → eventual success.

**FR-PD-005: Capability Matrix**
- Priority: SHOULD
- Each adapter SHALL report its capabilities:
  structured_output, streaming, max_context_tokens, latency_class.
- The orchestrator SHALL use capabilities for model dispatch policy.
- Acceptance: Capability query returns expected fields.

### 3.8 Audit Trail

**FR-AT-001: Sequential Logging**
- Priority: MUST
- The audit log SHALL assign monotonically increasing sequence
  numbers to every recorded command.
- Acceptance: 100 commands → seq 1..100, no gaps.

**FR-AT-002: Duration Tracking**
- Priority: SHOULD
- The audit log SHOULD record execution duration (microseconds)
  for each command.
- Acceptance: Duration field populated after execution.

**FR-AT-003: Markdown Report**
- Priority: MUST
- The audit log SHALL be renderable to a complete Markdown
  report listing all entries with timestamps and results.
- Acceptance: Rendered report parseable and contains all entries.

**FR-AT-004: Concurrent Safety**
- Priority: MUST
- The audit log MUST be safe for concurrent writes from
  multiple agents (threads).
- Acceptance: 10 threads writing simultaneously; no data loss.

### 3.9 Configuration

**FR-CF-001: Global Config**
- Priority: MUST
- The system SHALL read global configuration from
  `~/.terra/config.toml`.
- Acceptance: Config file read; settings applied.

**FR-CF-002: Project Config Override**
- Priority: MUST
- Project-level `project/.terra/config.toml` SHALL override
  global settings.
- Acceptance: Project setting overrides global for same key.

**FR-CF-003: Environment Variable Override**
- Priority: MUST
- Environment variables (`TERRA_PROVIDER_API_KEY`,
  `TERRA_PROVIDER_BASE_URL`) SHALL override config file settings.
- Precedence: env var > project config > global config > defaults.
- Acceptance: Env var takes precedence in all test scenarios.

**FR-CF-004: Environment Detection**
- Priority: SHOULD
- `terra doctor` SHALL detect and report:
  OS type and version, Rust version, available providers,
  SQLite availability, embedding model status.
- Acceptance: Doctor output includes all detected components.

### 3.10 Sleep Consolidation

**FR-SC-001: Post-Session Cleanup**
- Priority: SHOULD
- On session end, the system SHOULD remove S3 (Task) entries
  that are not promoted.
- Acceptance: Task entries absent after session end.

**FR-SC-002: Abstraction**
- Priority: SHOULD
- The system SHOULD abstract concrete actions into general
  principles (e.g., "added null check to auth.py:42" →
  "validate input boundaries at function entry").
- Acceptance: Abstracted principle is more general than
  original action.

**FR-SC-003: Promotion Candidates**
- Priority: SHOULD
- The system SHOULD generate a list of promotion candidates
  for user review after sleep consolidation.
- Acceptance: Candidate list presented; user can accept/reject.

### 3.11 Skill System

**FR-SK-001: Skill Registration**
- Priority: SHOULD
- Skills SHALL be registered via a MANIFEST.yml file declaring:
  name, version, capabilities, inputs, outputs.
- Acceptance: Skill manifest loaded and capabilities listed.

**FR-SK-002: Skill Communication**
- Priority: SHOULD
- Skills SHALL communicate with Terra via MessagePack over stdio.
- Acceptance: Skill receives MessagePack input, returns output.

---

## 4. Non-Functional Requirements

**NFR-001: Token Efficiency**
- Priority: MUST
- Inter-agent communication SHALL use MessagePack (not natural language).
- Command Protocol SHALL use ~15 tokens per command (not generated code).
- Rationale: Token waste is resource waste; efficiency is a first-class concern.

**NFR-002: CLI-First**
- Priority: MUST
- All functionality SHALL be accessible via CLI.
- All commands SHALL support `--as-json` for machine-readable output.

**NFR-003: Offline Operation**
- Priority: MUST
- The system MUST operate without network connectivity in dry_run mode.
- Embedding models SHOULD be loadable locally (ONNX Runtime).

**NFR-004: Provider Agnosticism**
- Priority: MUST
- The system SHALL work with any OpenAI-compatible LLM endpoint.
- No vendor-specific features SHALL be required.

**NFR-005: Code Quality**
- Priority: MUST
- Code SHALL be elegant and precise, not written merely to complete features.
- No `.unwrap()` in production code.
- All public APIs SHALL have documentation comments.
- Naming SHALL follow Rust conventions (snake_case, CamelCase).

**NFR-006: Auditability**
- Priority: MUST
- Every agent action SHALL be logged in the audit trail.
- Audit logs SHALL be replayable.

**NFR-007: Security**
- Priority: MUST
- Credentials SHALL be read from environment variables only.
- Sandbox SHALL prevent writes outside project root.
- Constitution and cage rules SHALL be immutable at runtime.

**NFR-008: Performance**
- Priority: SHOULD
- Cache hit latency: < 1ms
- SQLite read: < 5ms
- MessagePack encode: < 100μs
- Vector search (1K entries): < 10ms
- CLI startup: < 500ms (excluding embedding model load)

**NFR-009: Reliability**
- Priority: MUST
- All fallible operations SHALL return `Result<T, E>`.
- Cage violations SHALL be logged and blocked, not panicked.
- Provider errors SHALL be retried (max 3 attempts).
- SQLite write failures SHALL not corrupt existing data (ACID).

**NFR-010: Extensibility**
- Priority: SHOULD
- New domain capabilities SHALL be addable via Skill packages
  without modifying core Terra code.
- New Provider implementations SHALL require only implementing
  the `ProviderAdapter` trait.

---

## 5. Interface Requirements

### 5.1 User Interface (CLI)

```
terra                              -- banner + environment scan
terra doctor [--as-json]           -- environment readiness
terra run --task "..." [--as-json] -- execute task
terra memory read <key>            -- read memory entry
terra memory search <query>        -- semantic search
terra trace show                   -- display audit trail
terra config show                  -- show configuration
terra config set <key> <value>     -- update setting
terra --help                       -- command listing
terra --version                    -- version display
```

### 5.2 Skill Interface (MessagePack over stdio)

```
Input:  MessagePack-encoded TaskPayload on stdin
Output: MessagePack-encoded ResultPayload on stdout
Errors: JSON error object on stderr
```

### 5.3 Provider Interface (HTTP)

```
Endpoint: OpenAI-compatible /v1/chat/completions
Auth:     Bearer token from TERRA_PROVIDER_API_KEY
Format:   JSON request/response (standard OpenAI format)
```

---

## 6. System Constraints

| ID | Constraint |
|----|-----------|
| SC-01 | Single-user system (no multi-tenancy in MVP) |
| SC-02 | Single-machine deployment (no distributed agents in MVP) |
| SC-03 | English and Chinese natural language input |
| SC-04 | Software engineering domain focus (MVP) |

---

## 7. Acceptance Criteria

### 7.1 MVP1 (Complete)

- [x] End-to-end path: User task → Spec IR → Main Agent → Provider → Worker → Trace
- [x] CLI: `terra run --task "..." [--as-json]`
- [x] DryRun Provider functional
- [x] 16 tests passing

### 7.2 MVP2 (Current Target)

- [ ] Real LLM Provider connected and tested
- [ ] Memory Database: write, read, search functional
- [ ] Intelligence Core: basic rules loaded
- [ ] Sandbox: write protection enforced
- [ ] Config: global + project + env var precedence
- [ ] 100+ tests passing
- [ ] `terra doctor` detects full environment

### 7.3 MVP3 (Future)

- [ ] Full Language Compilation (LLM-assisted)
- [ ] Cage Sub-agent active monitoring
- [ ] Sleep Consolidation
- [ ] Multi-Head Attention retrieval
- [ ] Streaming LLM support
- [ ] Skill registration and loading
