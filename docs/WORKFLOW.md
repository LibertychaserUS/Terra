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
