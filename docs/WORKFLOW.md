# Terra Development Workflow

> Document ID: TERRA-WF-001
> Version: 1.0.0
> Date: 2026-05-21
> Author: Oliver Zhang, Claude (Anthropic)
>
> This document defines the complete development workflow for building
> Terra Agent OS. It is governed by Caging Engineering principles —
> the same constraints that Terra enforces on agents are applied to
> the development process itself.

---

## 1. Development Philosophy

### 1.1 Caging Engineering as Development Methodology

CE simultaneously governs development AND product:

| CE Cage | Development Process | Product Runtime |
|---------|-------------------|-----------------|
| Specification | PRD/TDD define what to build | TaskSpec defines what agent does |
| Knowledge | Current code > docs > memory > assumption | Authority ranking for agent knowledge |
| Memory | Conversation memory managed, decisions persisted | 7-level Memory Database |
| Action | Only change target code, no side modifications | Sandbox, tool permissions |
| Verification | Tests MUST pass before commit | External verifiers for agent output |
| Convergence | Feature done → stop, don't over-engineer | Accepted path → stop exploring |
| Meta | These principles are not negotiable | Constitution immutable |

### 1.2 Anti-Patterns to Actively Prevent

| Anti-Pattern | How Detected | Prevention |
|---|---|---|
| Lazy implementation | Tests pass trivially (field checks, not behavior) | TDD v2 requires behavioral tests |
| Shallow reasoning | Code works but architecture is bypassed | Pipeline integrity tests (T-PI-*) |
| Goal drift | Feature creep or solving different problem | Spec IR → CagingEngine gate |
| Skeleton without depth | All crates exist but internals are stubs | Cross-reference audit: every FR has working code + passing test |
| Direct subsystem access | Orchestrator calls provider/memory directly | Import graph audit (FR-002) |

---

## 2. Goal Structure

### 2.1 Multi-Dimensional Goal

The Goal is not a single statement. It is the intersection of three documents:

```
PRD (What)  ×  TDD (How)  ×  ARCHITECTURE (Structure)
     ↓              ↓               ↓
 132 FR        214 tests       9 crates + layers
     ↓              ↓               ↓
         → Together define "Done" ←
```

A feature is "done" when:
1. Its FR requirements are met (PRD)
2. Its behavioral tests pass (TDD)
3. Its code follows the layered architecture (ARCHITECTURE)

### 2.2 Completion Criteria

Terra is complete when:
- All 132 FR in PRD are implemented
- All 214 tests in TDD pass
- Import graph confirms no layer violations
- `terra doctor` reports all components healthy
- End-to-end pipeline integrity test passes (T-PI-001)

---

## 3. Development Order

### 3.1 Principle: Bottom-Up, Depth-First

Build the lowest layers first. Each layer is complete and tested before the next layer begins. No skeletal layers — every layer is functional before moving up.

### 3.2 Phase 1: Foundation (L0-L2)

**Goal:** Storage, protocol, and subsystems are complete and independently testable.

```
Step 1: terra-protocol (L3)
  - Command Executor struct + execute() method
  - All 15 CommandOp variants routed
  - Audit integration (pre/post recording)
  - Tests: T-PI-008 to T-PI-017, T-PT-001 to T-PT-022

Step 2: terra-memory (L2)
  - Complete Memory Database with all 5 layers:
    Bloom Filter → moka cache → SQLite → Vector Index
  - Ownership, borrowing, lifetime enforcement
  - Promotion chain with validation (no backwards)
  - Attention-based vector search (FAISS/usearch + ONNX embedding)
  - Artifact Store (versioned MessagePack files)
  - Tests: T-MD-001 to T-MD-030, T-AS-001 to T-AS-008

Step 3: terra-trace (L2)
  - Persistent audit log (SQLite)
  - Command replay executor
  - Tests: T-AT-001 to T-AT-012

Step 4: terra-providers (L1)
  - DryRun + OpenAI-compatible adapter
  - Retry with backoff (verified with mock server)
  - Capability matrix
  - Tests: T-PV-001 to T-PV-020
```

### 3.3 Phase 2: Kernel (L4)

**Goal:** Caging and Intelligence Core are active, not passive.

```
Step 5: terra-core (L4)
  - Universal Framework with 13 rules
  - Self-diagnostic rules with measurable thresholds (D-001/D-002/D-003)
  - Domain Module trait + CodingModule + AlgorithmModule
  - Multi-Head perspective generation
  - Rule injection into prompt building
  - Tests: T-IC-001 to T-IC-015

Step 6: terra-caging (L4)
  - All 8 cages implemented
  - SandboxGuard with symlink escape prevention
  - CagingEngine sequential check with short-circuit
  - Constitution loading and immutability
  - Tests: T-CG-001 to T-CG-020
```

### 3.4 Phase 3: Application (L5)

**Goal:** Multi-agent runtime uses the full pipeline.

```
Step 7: terra-compile (L5)
  - Five-pass compilation pipeline
  - LLM-assisted semantic resolution (when provider available)
  - Tests: T-LC-001 to T-LC-018

Step 8: terra-agents (L5)
  - Command Executor integration (NO direct subsystem access)
  - Orchestrator follows canonical pipeline (FR-001)
  - Main Agent with IC consultation
  - Worker as distinct agent with own context
  - Explorer for search tasks
  - Cage Sub-agent for monitoring
  - Model dispatch policy
  - Session Manager + Sleep Consolidation
  - Tests: T-AG-001 to T-AG-016, T-SL-001 to T-SL-010
```

### 3.5 Phase 4: Shell (L6)

**Goal:** CLI complete, user-facing.

```
Step 9: terra-cli (L6)
  - All CLI commands
  - Config system (layered precedence)
  - Environment detection
  - Tests: T-CLI-001 to T-CLI-010, T-CF-001 to T-CF-008
```

### 3.6 Phase 5: Integration

**Goal:** Everything works end-to-end.

```
Step 10: Integration tests
  - Pipeline integrity (T-PI-001 to T-PI-007)
  - Cross-crate integration (T-INT-001 to T-INT-008)
  - Full system validation
```

---

## 4. Per-Feature Workflow

For each feature within a phase:

```
1. IDENTIFY
   - Which FR does this implement? (from PRD)
   - Which tests does it need? (from TDD)
   - Which crate does it belong to? (from ARCHITECTURE)

2. DESIGN
   - Claude Code Plan mode
   - Read existing code in the target crate
   - Design approach that follows layered architecture
   - Verify: does this go through Command Executor? Through CagingEngine?

3. IMPLEMENT
   - Write code (elegant, precise, only target changes)
   - No .unwrap() in production code
   - Follow existing patterns in the crate
   - Public APIs get doc comments

4. TEST
   - Write behavioral tests (not field checks)
   - Each test verifies a specific TDD test case (T-XX-NNN)
   - Run: cargo test -p <crate>

5. VERIFY
   - cargo test (full workspace)
   - Import graph check: no layer violations
   - Cross-reference: FR has code + passing test

6. COMMIT
   - Descriptive commit message referencing FR and T- IDs
   - Only commit when tests pass
```

---

## 5. Quality Gates

### 5.1 Before Every Commit

- [ ] `cargo test` passes (all crates)
- [ ] `cargo build` has no warnings
- [ ] New code has no `.unwrap()` in production paths
- [ ] Only target files modified (no side changes)
- [ ] Commit message references FR-NNN and T-XX-NNN

### 5.2 Before Phase Completion

- [ ] All FR for this phase implemented
- [ ] All TDD tests for this phase pass
- [ ] Import graph confirms no layer violations
- [ ] No dead code (all structs used by something above)

### 5.3 Before Release

- [ ] All 132 FR implemented
- [ ] All 214 tests pass
- [ ] Pipeline integrity test (T-PI-001) passes
- [ ] `terra doctor --as-json` reports all components healthy
- [ ] Live provider test passes
- [ ] Documentation up to date

---

## 6. Branch Strategy

```
main     — stable, all tests passing, no known issues
dev      — active development, tests should pass but may have WIP
feature/ — short-lived branches for specific features (merged to dev)
```

Current state: main and dev at same commit. Future development on dev, merge to main at phase boundaries.

---

## 7. Audit Protocol

### 7.1 After Each Phase

Run a gap analysis:
1. List all FR for the phase
2. For each FR: does code exist? Does test exist? Does test pass?
3. Report any FR without working code or passing test
4. Report any code that bypasses the pipeline

### 7.2 Cross-Reference Matrix

Maintain a matrix showing: FR-NNN → test T-XX-NNN → crate:file:line

This ensures nothing falls through the cracks.

---

## 8. Known Technical Challenges

| Challenge | Mitigation |
|-----------|-----------|
| Embedding model load time (~3s) | Consider daemon mode for long sessions |
| FAISS/SQLite sync on crash | Re-index from SQLite on startup |
| Rust compile time with 9 crates | cargo cache, sccache |
| Concurrent SQLite writes | WAL mode + app-level version check |
| Embedding model migration | Full re-index from content column |
| Skill version compatibility | manifest_version field, validated on load |
| Streaming LLM responses | tokio stream processing, mid-stream interruption |

---

## Iteration Workflow (Post-Phase 5)

> Appended: 2026-05-22
> Scope: Phase 1-5 baseline complete (296 tests). This section defines
> the iterative workflow for implementing PRD/TDD Patch 1 (FR-143~178,
> 48 new test specs) and resolving 8 known issues.

---

## 9. Iteration Principles

### 9.1 Fix Before Build

Architectural defects are resolved before new features.
A new feature on a broken foundation inherits the break.

### 9.2 Horizontal Before Vertical

Cross-cutting concerns (layer fix, test standardization) are done in
dedicated passes, not interleaved with feature work. This prevents
partial fixes that create inconsistent states.

### 9.3 Each Iteration is Shippable

Every iteration ends with: all tests pass, zero warnings, clean commit.
No "WIP" iterations. If a feature is too large for one iteration,
split it into independently testable sub-features.

### 9.4 Commit Per Iteration

One commit per completed iteration. Commit message references
FR-NNN, T-XX-NNN, and ISSUE-NNN as applicable.

---

## 10. Iteration Plan

### Iteration 6: Architecture Fix — Layer Violations + Test IDs

**Goal:** Zero architectural debt. Clean foundation for all subsequent work.

**Priority:** P0 — blocks everything else.

```
Step 6.1: Extract shared types to terra-types (L0)
  - New crate: terra-types
  - Move from terra-protocol: CommandOp, CommandResult, CommandStatus, Command
  - terra-protocol keeps: MessagePack codec, render, AgentMessage
  - Update all crate imports: protocol → types for command types
  - ISSUE-001, ISSUE-002, ISSUE-003 resolved
  - Test: T-FIX-001 (import graph audit: no L2/L1 → L3+ dependency)

Step 6.2: Standardize test IDs
  - Rename test function comments to match TDD v2 prefixes:
    T-C-NNN → T-LC-NNN (terra-compile)
    T-TR-NNN → T-AT-NNN (terra-trace)
    T-PR-NNN → T-PV-NNN (terra-providers)
    Add T-PT-NNN tags to terra-protocol tests
    Add T-MD-NNN tags to terra-memory tests
  - Do NOT change test logic, only comments/names
  - ISSUE-004 resolved
  - Test: T-FIX-002 (all test functions tagged with correct prefix)
```

**Quality Gate:**
- `cargo test --workspace` passes
- `cargo build --workspace` zero warnings
- Grep confirms: no T-C-, T-TR-, T-PR- prefixes remain in test comments
- No crate in L2/L1 has [dependencies] pointing to L3+

---

### Iteration 7: Audit Persistence + Embedding Backend

**Goal:** Two foundational gaps closed — audit survives restart,
vector search has real semantic capability.

**Priority:** P1 — enables meaningful memory retrieval and traceable history.

```
Step 7.1: Audit log SQLite persistence
  - Connect terra-trace persistent.rs to AuditLog
  - Write-through: in-memory + SQLite
  - Load on startup: restore from SQLite
  - ISSUE-006 resolved
  - Tests: T-FIX-003 (audit persists to disk), T-AT-013+ as needed

Step 7.2: Embedding backend trait + ONNX integration
  - Add EmbeddingBackend trait to terra-memory/search.rs
  - TfIdfBackend (existing, fallback)
  - OnnxBackend (new, uses ort crate with sentence-transformers model)
  - VectorIndex dispatches to whichever backend is available
  - ISSUE-005 resolved
  - Tests: T-FIX-004 (backend swappable), T-RS-001 (index rebuild)
```

**Quality Gate:**
- Audit entries survive simulated process restart in test
- VectorIndex works with both TfIdf and (if ort available) Onnx backend
- Chinese text retrieval works with Onnx backend

---

### Iteration 8: Conversation/Goal Dual Mode

**Goal:** Terra has two runtime modes. This is the largest architectural
change — it touches session, caging, CLI, and orchestrator.

**Priority:** P1 — prerequisite for all subsequent feature work.

```
Step 8.1: Mode state machine in SessionManager
  - Add SessionMode enum { Conversation, Goal { spec, drift_score } }
  - SessionManager tracks current mode
  - Mode transitions recorded in Audit
  - Tests: T-DM-001, T-DM-002, T-DM-003, T-DM-007, T-DM-010

Step 8.2: CagingEngine mode-aware checking
  - CagingEngine.check() receives current mode
  - Goal-only cages (Spec, Verify, Convergence) skip in Conversation
  - Always-on cages (Meta, Memory, Action) check in both
  - Tests: T-DM-004, T-DM-005, T-DM-006

Step 8.3: User commands (/goal, /done, /status)
  - CLI parses /goal, /done, /status as program-handled commands
  - /goal → compile SpecIR + switch to Goal Mode
  - /done → trigger sleep + switch to Conversation Mode
  - /status → display mode, goal, drift score
  - No LLM invocation for any of these
  - Tests: T-CMD-001, T-CMD-002, T-CMD-003, T-CMD-004, T-CMD-008
```

**Quality Gate:**
- `/goal "fix bug"` → mode switches, spec compiled, cages activated
- `/done` → sleep triggers, mode returns, audit records transition
- Conversation Mode: ProviderCall works without SpecIR
- Goal Mode: incomplete spec still blocks via SpecificationCage

---

### Iteration 9: Memory Write Permissions + Agent Auto-Record

**Goal:** L0-L6 write permission matrix enforced by MemoryCage.
Agent can buffer observations to L5/L6 with fixed-format notification.

```
Step 9.1: Level-aware MemoryCage
  - MemoryCage checks (level, mode) against permission matrix:
    L0: blocked always
    L1: blocked for agent (only /remember or promotion)
    L2: allowed both modes
    L3: allowed Goal Mode + Verification pass only
    L4: allowed both modes
    L5: allowed both modes
    L6: allowed both modes
  - Tests: T-MW-001 through T-MW-005

Step 9.2: Auto-record notification
  - When agent writes L5/L6 via Executor, Executor emits
    fixed-format notification to output stream
  - Format: [Terra] Recorded → L{n}: "{summary}"
  - Program-generated, not LLM-generated
  - Tests: T-MW-006, T-MW-007, T-MW-008

Step 9.3: /remember and /forget commands
  - /remember writes L1 with owner=user (bypasses promotion)
  - /forget soft-deletes with owner check
  - Tests: T-CMD-005, T-CMD-006
```

**Quality Gate:**
- Agent MemWrite to L0 → PermissionDenied
- Agent MemWrite to L1 → PermissionDenied
- Agent MemWrite to L5 → Ok + notification emitted
- `/remember "content"` → L1 written, no LLM cost

---

### Iteration 10: Sleep Consolidation Cycle (Full)

**Goal:** Sleep consolidation connected to IC rules, Memory promotion
chain, and user verdict loop. Configurable trigger cycle.

```
Step 10.1: Sleep config in config.toml
  - [sleep] section: trigger, interval_hours, max_candidates
  - Parsed by config system, passed to SessionManager
  - Tests: T-SL-011, T-SL-016

Step 10.2: IC-connected consolidation
  - Consolidate scans L2+L5 (not L0/L1/L3)
  - Groups by pattern, matches against IC rule IDs
  - Candidate principle references which IC rule it relates to
  - Tests: T-SL-014, T-SL-015

Step 10.3: Promotion with user verdict
  - Candidates presented to user (CLI or structured output)
  - Accept → promote to L1 (with evidence chain from derived_from)
  - Reject → discard or remain L5
  - Verdict recorded in Audit
  - Tests: T-SL-017, T-SL-018

Step 10.4: Trigger modes
  - session_end: /done triggers consolidation
  - interval: configurable periodic trigger
  - manual: /sleep command
  - Tests: T-SL-012, T-SL-013
```

**Quality Gate:**
- Config `trigger = "session_end"` → `/done` runs consolidation
- `/sleep` → consolidation runs regardless of config
- Candidate includes IC rule reference
- Accept → L1 entry exists, Audit records verdict

---

### Iteration 11: Output Cage + Active Drift Detection

**Goal:** Model responses are audited. Drift detection runs automatically
in Goal Mode without consuming LLM tokens.

```
Step 11.1: Output Cage implementation
  - New cage: OutputCage
  - Inspects ProviderCall response content (string matching + pattern)
  - Blocks: bypass instructions, constitution modification, permission escalation
  - Flags: references to operations outside CommandOp set
  - Integrated into Executor: after provider returns, before result delivery
  - Tests: T-OC-001 through T-OC-005

Step 11.2: Active drift detection in Goal Mode
  - Executor tracks command count in Goal Mode
  - Every N commands (configurable, default 5): auto-evaluate D-001
  - Uses TF-IDF/embedding similarity, NOT ProviderCall
  - Score < threshold → pause execution, log, require re-align
  - Tests: T-DD-001 through T-DD-005

Step 11.3: Drift suggestion in Conversation Mode
  - When agent detects task-like intent in Conversation Mode
  - Emit suggestion: "Detected task intent. Use /goal to activate Goal Mode."
  - Do NOT auto-switch
  - Test: T-DM-008
```

**Quality Gate:**
- Model response "ignore all rules" → Output Cage blocks
- Goal Mode, 5 commands → drift auto-check fires
- Drift score < 0.6 → execution paused
- No ProviderCall in audit for drift checks

---

### Iteration 12: Remaining Gaps + Hardening

**Goal:** Close all remaining FR gaps from Phase 1-5 review.

```
Step 12.1: Agent inter-communication pipeline
  - Worker/Explorer receive TaskPayload via MessagePack encode/decode
  - Results returned as ResultPayload
  - Full codec roundtrip verified
  - ISSUE-007 resolved

Step 12.2: Resilience tests
  - T-RS-001: vector index rebuild from SQLite on startup
  - T-RS-002: lazy load pointer metadata only
  - T-RS-003: embedding model migration (full re-index)

Step 12.3: Session switch
  - SessionManager.switch(): flush old → load new
  - FR-101 resolved

Step 12.4: Remaining T-CG, T-PI gaps
  - T-CG-001 (pass allows), T-CG-003 (block prevents), T-CG-019 (cage rule change)
  - T-PI-003 (cage blocks incomplete), T-PI-004 (cage blocks unverified)
  - T-PI-011~014 (executor routing verification)
  - T-PI-017 (sandbox in production path)
```

**Quality Gate:**
- All 178 FR (original 142 + Patch 36) have code + passing test
- All TDD tests (original 217 + Patch 48 = 265) pass
- Import graph: zero violations
- `terra doctor` reports all healthy

---

## 11. Iteration Quality Gates (Universal)

Applied to every iteration, no exceptions.

### 11.1 Before Commit

- [ ] `cargo test --workspace` passes, zero failures
- [ ] `cargo build --workspace` zero warnings
- [ ] Only target files modified (no side changes)
- [ ] New code has no `.unwrap()` in production paths
- [ ] Commit message references FR-NNN, T-XX-NNN, ISSUE-NNN

### 11.2 Before Iteration Sign-off

- [ ] All FR for this iteration implemented
- [ ] All TDD tests for this iteration pass
- [ ] No regression: previous iteration tests still pass
- [ ] Import graph confirms no layer violations

### 11.3 After Iteration 12 (Merge Gate)

- [ ] All 265 TDD tests pass
- [ ] `cargo build` zero warnings
- [ ] Import graph: zero violations (T-FIX-001)
- [ ] Test IDs standardized (T-FIX-002)
- [ ] Audit persists (T-FIX-003)
- [ ] Embedding swappable (T-FIX-004)
- [ ] `terra doctor --as-json` all healthy
- [ ] Live provider E2E passes
- [ ] dev → main merge

---

## 12. Dependency Graph (Iteration Order)

```
Iteration 6 (layer fix)
    │
    ├── Iteration 7 (audit + embedding)
    │       │
    │       ├── Iteration 8 (dual mode)
    │       │       │
    │       │       ├── Iteration 9 (memory permissions)
    │       │       │       │
    │       │       │       └── Iteration 10 (sleep cycle)
    │       │       │
    │       │       └── Iteration 11 (output cage + drift)
    │       │
    │       └── Iteration 12 (gaps + hardening)
    │
    └── dev → main merge (after Iteration 12)
```

Iterations 9/10 and 11 can run in parallel after 8.
Iteration 12 waits for all others.
