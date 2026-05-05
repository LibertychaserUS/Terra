# Terra Technical Design Document

> Document ID: TERRA-TDD-002
> Version: 2.0.0
> Status: APPROVED
> Date: 2026-05-21
> Author: Oliver Zhang, Claude (Anthropic)
>
> POSIX-style module specs. RFC 2119 keywords.
> All tests verify BEHAVIOR, not just field existence.
> No MVP simplification — complete specification.

---

## 1. Architecture

### 1.1 Layer Model

```
L6 Shell:        terra-cli (CLI, config)
L5 Application:  terra-agents (orchestrator, worker, explorer, cage)
                 terra-compile (language compilation, Spec IR)
L4 Kernel:       terra-caging (8 cages, SandboxGuard)
                 terra-core (Intelligence Core, domain modules)
L3 Syscall:      terra-protocol (Command Protocol, MessagePack codec)
L2 Subsystems:   terra-memory (Memory Database, cache, vector index)
                 terra-trace (audit log, checkpoints)
L1 Drivers:      terra-providers (LLM adapters)
L0 Hardware:     SQLite, FAISS/usearch, moka, ONNX Runtime, fs
```

**Dependency Rule:** Higher MAY depend on lower. Lower MUST NOT depend on higher.

**Critical Constraint (from audit):** L5 (terra-agents) MUST NOT import L2 (terra-memory) or L1 (terra-providers) directly. All operations go through L3 (terra-protocol) Command Executor.

### 1.2 Technology Stack

| Component | Crate | Purpose |
|-----------|-------|---------|
| Async | tokio | Multi-agent concurrency |
| Serialization | serde + rmp-serde | MessagePack |
| Database | rusqlite | SQLite WAL |
| Cache | moka | Concurrent TTL/TTI |
| Vector | usearch or faiss-rs | Embedding retrieval |
| LLM | async-openai | Provider dispatch |
| CLI | clap | Arg parsing |
| Logging | tracing | Structured logging |
| Errors | anyhow + thiserror | Error handling |
| Embedding | ort | ONNX Runtime |
| HTTP | reqwest | Provider calls |

---

## 2. Module Specifications

### 2.1 Command Executor (NEW — was completely missing)

#### PURPOSE
Single gateway for all agent-system interactions. Enforces audit + caging + routing.

#### INTERFACE
```rust
pub struct CommandExecutor {
    memory: MemoryDatabase,
    artifacts: ArtifactStore,
    audit: PersistentAuditLog,
    caging: CagingEngine,
    sandbox: SandboxGuard,
}

impl CommandExecutor {
    pub async fn execute(&self, cmd: Command, agent: &str) -> CommandResult;
}
```

#### BEHAVIOR
```
execute(cmd, agent):
  1. seq = audit.record_command(cmd)
  2. cage_result = caging.check(cmd_to_cage_context(cmd, agent))
  3. IF cage_result.is_blocked():
       audit.record_result(seq, PermissionDenied, 0)
       RETURN CommandResult::blocked(cage_result.reason)
  4. result = route_and_execute(cmd)
  5. audit.record_result(seq, result, elapsed)
  6. RETURN result
```

#### ERROR CONDITIONS
| Error | Condition |
|-------|-----------|
| PermissionDenied | Cage blocked the action |
| NotFound | Memory key doesn't exist |
| ExecutionError | Backend operation failed |

---

### 2.2 Orchestrator (REWRITTEN — must use pipeline)

#### PURPOSE
Main Agent orchestration through the canonical pipeline.

#### INTERFACE
```rust
pub struct Orchestrator {
    executor: Arc<CommandExecutor>,
    core: Arc<IntelligenceCore>,
    provider: Arc<dyn ProviderAdapter>,
}

impl Orchestrator {
    pub async fn run(&self, task: &str) -> Result<RunResult>;
}
```

#### BEHAVIOR
```
run(task):
  1. spec = compile(task)                    // Language Compilation
  2. rules = core.applicable_rules(&spec)    // IC consultation
  3. perspectives = core.all_perspectives()
  4. cage_result = executor.execute(CageCheck(spec))  // Caging gate
  5. IF blocked: RETURN error
  6. memories = executor.execute(MemSearch(perspectives))  // Attention retrieval
  7. prompt = build_prompt(spec, rules, memories)
  8. response = executor.execute(ProviderCall(prompt))  // Through executor
  9. executor.execute(TraceCheckpoint("completed"))
  10. RETURN RunResult
```

**Critical difference from current code:** Steps 2, 3, 4, 6 are new. Step 8 goes through executor, not direct provider call.

---

### 2.3 Memory Database

#### STORAGE ARCHITECTURE
```
Query:  Bloom Filter (ns) → moka cache (ns) → Project SQLite (ms) → Global SQLite (ms)
Write:  Validate ownership → SQLite (truth) → cache update
Startup: Open SQLite → WAL mode → build Bloom from keys → warm cache
```

#### SCHEMA
```sql
PRAGMA journal_mode = WAL;
PRAGMA user_version = 2;

CREATE TABLE memories (
    id TEXT PRIMARY KEY, key TEXT NOT NULL UNIQUE,
    scope TEXT NOT NULL CHECK(scope IN ('global','project','session','task')),
    level TEXT NOT NULL CHECK(level IN ('L0','L1','L2','L3','L4','L5','L6')),
    owner TEXT NOT NULL, content BLOB NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    ref_count INTEGER NOT NULL DEFAULT 0,
    alive INTEGER NOT NULL DEFAULT 1,
    embedding BLOB,  -- 384-dim float32 vector (NEW)
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL CHECK(type IN ('plan','spec','test','review')),
    version INTEGER NOT NULL, content BLOB NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(type, version)
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project_root TEXT NOT NULL,
    goal TEXT, status TEXT NOT NULL DEFAULT 'active',
    started_at TEXT NOT NULL, ended_at TEXT
);
```

#### VECTOR INDEX
- Embedding model: all-MiniLM-L6-v2 (384-dim, ONNX)
- Index: FAISS IndexFlatIP or usearch
- Built from `memories.embedding` column on startup
- Incremental: add/remove on write/delete
- Re-index: full rebuild when embedding model changes

#### ATTENTION RETRIEVAL
```
search(context, perspectives):
  queries = [perspective.generate_query(context) for p in perspectives]
  vectors = [embed(q) for q in queries]  // Multi-Head
  results_per_head = [index.search(v, top_k) for v in vectors]
  merged = deduplicate(flatten(results_per_head))
  RETURN sorted_by_max_score(merged)
```

---

### 2.4 Caging Engine

#### 8 CAGES
| Cage | Checks | Block Condition |
|------|--------|-----------------|
| Specification | SpecIR completeness | open_questions non-empty |
| Knowledge | Authority ranking | Lower-authority source overriding higher |
| Memory | Ownership + promotion | Non-owner write, backwards promotion |
| Action | SandboxGuard | Write outside root or to protected |
| Verification | External verifier | Completion without verification pass |
| Convergence | Stop condition | Exploration after accepted path |
| Parametric | L6 gate | Promote without review |
| Meta | Constitution | Any cage/constitution rule modification |

#### SANDBOX
```rust
impl SandboxGuard {
    fn check_mutation(&self, path: &Path) -> Result<(), SandboxViolation> {
        let canonical = path.canonicalize()?;  // Symlink escape prevention
        if !canonical.starts_with(&self.root) { return Err(OutsideSandbox) }
        for p in &self.protected { if canonical.starts_with(p) { return Err(ProtectedPath) } }
        Ok(())
    }
}
```

---

### 2.5 Session Manager (NEW)

#### PURPOSE
Manages session lifecycle: S2 scope creation, project switching, Sleep Consolidation.

#### INTERFACE
```rust
pub struct SessionManager {
    current_session: Option<Session>,
    memory: Arc<MemoryDatabase>,
}

impl SessionManager {
    pub fn start(&mut self, project_root: &Path, goal: &str) -> Result<Session>;
    pub fn switch(&mut self, project_root: &Path) -> Result<Session>;
    pub fn end(&mut self) -> Result<SleepConsolidationReport>;
}
```

#### SLEEP CONSOLIDATION
```
end():
  1. Collect all S3 (Task) entries from current session
  2. For each accepted path (L3):
     a. abstract: "added null check to auth.py:42" → "validate input boundaries at function entry"
     b. create promotion candidate
  3. Clean S3 entries (not promoted)
  4. Generate SleepConsolidationReport with candidates
  5. Present to user for accept/reject/defer
  6. Promote accepted candidates to S1
```

---

## 3. Test Specification

**Philosophy:** Every test verifies BEHAVIOR under realistic conditions, not just field existence. Tests are organized by the problem they catch.

### 3.1 Pipeline Integrity Tests (17 tests)

Tests that verify the canonical pipeline is followed.

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-PI-001 | `pipeline_all_stages_invoked` | Task traverses: Compilation → IC → Caging → Executor → Provider → Audit. Each stage produces a trace event. |
| T-PI-002 | `pipeline_ic_rules_in_prompt` | System prompt sent to provider CONTAINS rules from Intelligence Core. |
| T-PI-003 | `pipeline_cage_blocks_incomplete` | Incomplete spec → CagingEngine invoked → Block → task NOT executed → audit log has block entry. |
| T-PI-004 | `pipeline_cage_blocks_unverified` | Agent claims complete without verification → Block. |
| T-PI-005 | `pipeline_audit_records_every_step` | After task run, audit log has entries for: task_start, ic_query, cage_check, worker_dispatch, provider_call, task_complete. |
| T-PI-006 | `pipeline_no_direct_provider_call` | Orchestrator calls executor.execute(ProviderCall), NOT provider.complete() directly. |
| T-PI-007 | `pipeline_no_direct_memory_access` | Orchestrator calls executor.execute(MemWrite), NOT memory.write() directly. |
| T-PI-008 | `executor_audit_before_execute` | Audit entry timestamp is BEFORE execution timestamp. |
| T-PI-009 | `executor_cage_check_before_execute` | Caging check happens BEFORE backend operation. |
| T-PI-010 | `executor_cage_block_prevents_write` | Cage blocks MemWrite → memory entry does NOT exist. |
| T-PI-011 | `executor_routes_mem_to_memory` | MemWrite command → MemoryDatabase.write() called. |
| T-PI-012 | `executor_routes_artifact_to_store` | ArtifactSave → ArtifactStore.save() called. |
| T-PI-013 | `executor_routes_trace_to_audit` | TraceCheckpoint → AuditLog.record() called. |
| T-PI-014 | `executor_routes_session_to_manager` | SessionStart → SessionManager.start() called. |
| T-PI-015 | `executor_returns_duration` | CommandResult has non-zero duration_us. |
| T-PI-016 | `executor_permission_denied_on_block` | Cage Block → CommandResult.status == PermissionDenied. |
| T-PI-017 | `executor_sandbox_in_production_path` | File write command goes through SandboxGuard in executor, not just unit test. |

### 3.2 Intelligence Core Behavior Tests (15 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-IC-001 | `universal_rules_loaded` | 13 rules across 3 categories at startup. |
| T-IC-002 | `universal_readonly` | Attempt to modify rules → rejected (type system or runtime error). |
| T-IC-003 | `module_registration` | coding/ module registered → appears in module list. |
| T-IC-004 | `modules_composable` | coding/ + algorithm/ both active → perspectives from both returned. |
| T-IC-005 | `perspectives_for_multihead` | coding/ module provides 3 perspectives with query_templates. |
| T-IC-006 | `rules_filtered_by_task` | Bug fix task → applicable rules include B-002 (minimal change). |
| T-IC-007 | `rules_injected_into_prompt` | Build prompt with IC → output string contains rule descriptions. |
| T-IC-008 | `goal_drift_detection` | Simulated drift: action embedding far from spec embedding → D-001 score < 0.6. |
| T-IC-009 | `shallow_reasoning_detection` | 5 claims, 1 evidence → D-002 ratio = 0.2 < 0.3 → signal fired. |
| T-IC-010 | `shortcut_detection` | Plan has 3 verification steps, execution skips 2 → D-003 signal fired. |
| T-IC-011 | `coding_module_has_rules` | CodingModule.rules() returns at least 3 rules. |
| T-IC-012 | `algorithm_module_has_perspectives` | AlgorithmModule.perspectives() returns at least 2. |
| T-IC-013 | `perspective_generates_query` | Perspective.query_template with context → produces non-empty query string. |
| T-IC-014 | `all_rules_includes_universal_and_modules` | core.all_rules().len() > universal_only.len(). |
| T-IC-015 | `diagnostic_rules_have_thresholds` | D-001/D-002/D-003 each have defined threshold values. |

### 3.3 Language Compilation Tests (18 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-LC-001 | `from_description_produces_spec` | "fix login bug" → SpecIR with non-empty intent. |
| T-LC-002 | `incomplete_spec_has_questions` | Basic compilation → open_questions non-empty. |
| T-LC-003 | `complete_spec_no_questions` | Manually built spec → is_complete() == true. |
| T-LC-004 | `json_roundtrip` | SpecIR survives JSON serialize → deserialize. |
| T-LC-005 | `msgpack_roundtrip` | SpecIR survives MessagePack roundtrip. |
| T-LC-006 | `target_file_line` | Target with file + line preserves both. |
| T-LC-007 | `target_component` | Target with component preserves it. |
| T-LC-008 | `constraints_preserved` | 3 constraints → all preserved. |
| T-LC-009 | `success_predicate_preserved` | 2 predicates → all preserved. |
| T-LC-010 | `term_mapping_roundtrip` | TermMapping survives serialization. |
| T-LC-011 | `empty_description_safe` | "" → no panic, empty intent. |
| T-LC-012 | `unicode_description` | "修复登录问题 🐛" → preserved. |
| T-LC-013 | `long_description` | 10K chars → no panic. |
| T-LC-014 | `spec_id_is_uuid` | spec_id has UUID v4 format (36 chars, 4 dashes). |
| T-LC-015 | `multiple_open_questions` | 3 questions → is_complete() false. |
| T-LC-016 | `lexical_pass_extracts_entities` | "fix auth.py line 42" → target.file="auth.py", target.line=42. |
| T-LC-017 | `terminology_normalization` | "login bug" mapped to "authentication_failure". |
| T-LC-018 | `completeness_blocks_execution` | Spec with open_questions → SpecificationCage blocks. |

### 3.4 Multi-Agent Tests (16 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-AG-001 | `main_agent_queries_ic` | Main Agent calls core.applicable_rules() before dispatch. |
| T-AG-002 | `main_agent_dispatches_worker` | Worker receives MessagePack TaskPayload. |
| T-AG-003 | `main_agent_audits_result` | Main Agent inspects Worker result before returning. |
| T-AG-004 | `worker_has_own_context` | Worker system prompt differs from Main's. |
| T-AG-005 | `worker_has_filtered_rules` | Worker gets coding/ rules for code task, not all rules. |
| T-AG-006 | `explorer_searches_memory` | Explorer issues MemSearch via executor. |
| T-AG-007 | `explorer_returns_findings` | Explorer returns structured ResultPayload with findings. |
| T-AG-008 | `cage_agent_monitors_drift` | Cage agent receives state → computes D-001 score → reports if low. |
| T-AG-009 | `cage_agent_no_business_logic` | Cage agent does not modify any memory or file. |
| T-AG-010 | `model_dispatch_multi_model` | 3 models available → Main gets strongest, Worker balanced, Explorer fast. |
| T-AG-011 | `model_dispatch_single_model` | 1 model → all agents same model, different params. |
| T-AG-012 | `worker_timeout_handled` | Worker exceeds timeout → error result returned (no hang). |
| T-AG-013 | `concurrent_workers_safe` | 2 workers running in parallel → no data corruption. |
| T-AG-014 | `end_to_end_dry_run` | Full pipeline with DryRunAdapter completes. |
| T-AG-015 | `end_to_end_live` | Full pipeline with real provider (skip if no key). |
| T-AG-016 | `empty_task_handled` | Empty string task → no panic. |

### 3.5 Memory Database Tests (30 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-MD-001 | `create_and_tables_exist` | Database init creates all tables. |
| T-MD-002 | `write_and_read` | Write entry → read returns matching content. |
| T-MD-003 | `duplicate_key_rejected` | Second write same key → DuplicateKey error. |
| T-MD-004 | `read_missing_returns_none` | Read nonexistent → Ok(None). |
| T-MD-005 | `update_increments_version` | Update → version N+1. |
| T-MD-006 | `delete_is_soft` | Delete → alive=false, not physically removed. |
| T-MD-007 | `content_is_msgpack` | Stored content deserializable from MessagePack. |
| T-MD-008 | `scope_constraint_enforced` | Invalid scope string → SQL error. |
| T-MD-009 | `level_constraint_enforced` | Invalid level → SQL error. |
| T-MD-010 | `owner_set_on_creation` | Write → owner field matches creator. |
| T-MD-011 | `non_owner_write_rejected` | Non-owner update → PermissionDenied. |
| T-MD-012 | `non_owner_read_allowed` | Non-owner read → Ok. |
| T-MD-013 | `ownership_transfer` | Explicit transfer → new owner can write. |
| T-MD-014 | `dangling_read_error` | Read deleted entry → Dangling error. |
| T-MD-015 | `version_mismatch_error` | Update with wrong version → Stale error. |
| T-MD-016 | `gc_removes_dead_weak` | Dead L5 ref_count=0 → removed by GC. |
| T-MD-017 | `gc_preserves_authoritative` | Dead L0 → NOT removed by GC. |
| T-MD-018 | `cache_hit_on_second_read` | Write → read → read again → second is cache hit (stats prove). |
| T-MD-019 | `cache_miss_falls_to_sqlite` | Invalidate cache → read → data from SQLite (miss stat increases). |
| T-MD-020 | `write_updates_cache_and_sqlite` | Write → both cache and SQLite have entry. |
| T-MD-021 | `update_invalidates_cache` | Update → cache invalidated → next read from SQLite. |
| T-MD-022 | `cache_eviction_on_capacity` | Insert beyond capacity → entries evicted. |
| T-MD-023 | `promote_l2_to_l1` | L2 → L1 → level changed. |
| T-MD-024 | `promote_l6_blocked_without_review` | L6 → L1 no review → PromotionBlocked. |
| T-MD-025 | `promote_l6_allowed_with_review` | L6 → L1 with review → allowed. |
| T-MD-026 | `promote_backwards_blocked` | L1 → L2 → PromotionBlocked. |
| T-MD-027 | `vector_search_returns_relevant` | 10 entries with embeddings. Query → top result semantically closest. |
| T-MD-028 | `multihead_search_merges` | 2 perspectives → results from both queries merged. |
| T-MD-029 | `concurrent_writes_safe` | 10 threads writing different keys → all succeed, no corruption. |
| T-MD-030 | `project_isolation` | Write to project A → not visible in project B. |

### 3.6 Caging Engine Tests (20 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-CG-001 | `pass_allows_action` | Pass → action proceeds. |
| T-CG-002 | `warn_logs_allows` | Warn → logged + action proceeds. |
| T-CG-003 | `block_prevents_action` | Block → action prevented. |
| T-CG-004 | `first_block_short_circuits` | Third cage blocks → fourth never checked. |
| T-CG-005 | `spec_complete_passes` | Complete spec → Pass. |
| T-CG-006 | `spec_incomplete_blocks` | Incomplete spec → Block. |
| T-CG-007 | `memory_owner_write_passes` | Owner MemWrite → Pass. |
| T-CG-008 | `memory_non_owner_blocks` | Non-owner MemWrite → Block. |
| T-CG-009 | `sandbox_write_inside_passes` | Write inside root → Pass. |
| T-CG-010 | `sandbox_write_outside_blocks` | Write outside root → Block. |
| T-CG-011 | `sandbox_read_outside_passes` | Read outside root → Pass. |
| T-CG-012 | `sandbox_write_protected_blocks` | Write constitution → Block. |
| T-CG-013 | `sandbox_read_protected_passes` | Read constitution → Pass. |
| T-CG-014 | `sandbox_delete_outside_blocks` | Delete outside → Block. |
| T-CG-015 | `sandbox_symlink_escape_blocks` | Symlink inside pointing outside → write blocked. |
| T-CG-016 | `verify_unverified_blocks` | Claim complete without verification → Block. |
| T-CG-017 | `verify_verified_passes` | Claim after verification → Pass. |
| T-CG-018 | `meta_constitution_change_blocks` | Modify constitution → Block + logged. |
| T-CG-019 | `meta_cage_rule_change_blocks` | Modify cage rule → Block. |
| T-CG-020 | `convergence_stops_after_accepted` | Accepted path → further exploration warned/blocked. |

### 3.7 Provider Tests (20 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-PV-001 | `dry_run_deterministic` | Same input → same output pattern. |
| T-PV-002 | `dry_run_zero_tokens` | Usage 0/0. |
| T-PV-003 | `dry_run_capabilities` | provider_id="dry_run", structured_output=true. |
| T-PV-004 | `dry_run_no_network` | Works offline. |
| T-PV-005 | `dry_run_model_field` | model="dry_run". |
| T-PV-006 | `openai_creation_from_env` | Reads API key from env. |
| T-PV-007 | `openai_capabilities` | Reports all capability fields. |
| T-PV-008 | `openai_missing_key_error` | No key → clear error. |
| T-PV-009 | `openai_live_completion` | Real API call succeeds (skip if no key). |
| T-PV-010 | `openai_retry_on_429` | Mock returns 429 twice → 200 → succeeds. |
| T-PV-011 | `openai_timeout` | Mock delays → timeout error. |
| T-PV-012 | `openai_streaming_flag` | supports_streaming=true in capabilities. |
| T-PV-013 | `dispatch_dry_run_mode` | Dispatch with DryRun → mode in result. |
| T-PV-014 | `dispatch_live_mode` | Dispatch with Live → mode in result. |
| T-PV-015 | `dispatch_has_response` | Result has non-empty content. |
| T-PV-016 | `dispatch_has_usage` | Result has token counts. |
| T-PV-017 | `dispatch_propagates_error` | Failing adapter → error propagated. |
| T-PV-018 | `dispatch_mode_matches` | DryRun → DryRun, Live → Live. |
| T-PV-019 | `openai_retry_exhausted` | Mock always 429 → error after 3 retries. |
| T-PV-020 | `openai_non_retryable_error` | Mock 400 → immediate error, no retry. |

### 3.8 Audit Trail Tests (12 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-AT-001 | `record_command_returns_seq` | First command → seq=1. |
| T-AT-002 | `record_result_attaches` | Result attached to correct seq. |
| T-AT-003 | `sequential_no_gaps` | 100 commands → seqs 1..100. |
| T-AT-004 | `entries_ordered` | Entries returned in seq order. |
| T-AT-005 | `render_markdown` | Markdown contains all entry fields. |
| T-AT-006 | `empty_log_renders` | Empty log → valid markdown header. |
| T-AT-007 | `unknown_seq_ignored` | Result for seq=999 → no panic. |
| T-AT-008 | `concurrent_10_threads` | 10×100 commands → 1000, no gaps. |
| T-AT-009 | `large_log_10k` | 10K entries < 2 seconds. |
| T-AT-010 | `msgpack_roundtrip` | Entries survive MessagePack serialization. |
| T-AT-011 | `persistent_survives_reopen` | Write → close → reopen → entries present. |
| T-AT-012 | `replay_reproduces_state` | Record commands → replay → same final state. |

### 3.9 Session Lifecycle Tests (10 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-SL-001 | `session_start_creates_s2` | After start, S2 scope writable. |
| T-SL-002 | `session_goal_recorded` | Goal retrievable after start. |
| T-SL-003 | `session_switch_flushes` | Switch → old cache flushed. |
| T-SL-004 | `session_switch_loads_new` | Switch → new project data loaded. |
| T-SL-005 | `session_end_cleans_s3` | End → S3 entries removed. |
| T-SL-006 | `session_end_triggers_sleep` | End → SleepConsolidationReport generated. |
| T-SL-007 | `sleep_abstracts_actions` | Concrete action → general principle. |
| T-SL-008 | `sleep_generates_candidates` | 5 tasks → N promotion candidates. |
| T-SL-009 | `sleep_user_accept_promotes` | User accepts → entry promoted to S1. |
| T-SL-010 | `sleep_user_reject_discards` | User rejects → entry not promoted. |

### 3.10 Artifact Store Tests (8 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-AS-001 | `save_creates_version` | Save plan → version 1 created. |
| T-AS-002 | `save_increments_version` | Save twice → version 1 and 2. |
| T-AS-003 | `load_latest` | Two versions → load without version → latest. |
| T-AS-004 | `load_specific_version` | Load version 1 → returns v1 content. |
| T-AS-005 | `list_artifacts` | Save 3 → list returns 3. |
| T-AS-006 | `content_is_msgpack` | Stored content is valid MessagePack. |
| T-AS-007 | `memory_stores_pointer` | After save, memory has pointer to artifact. |
| T-AS-008 | `artifact_types` | Plan, spec, test, review all supported. |

### 3.11 Protocol Tests (22 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-PT-001 to T-PT-006 | Message creation | Fields, uniqueness, timestamp, role serialization. |
| T-PT-007 to T-PT-012 | Codec | Roundtrips (task/result/command), size comparison, optionals, invalid bytes. |
| T-PT-013 to T-PT-019 | Commands | All CommandOp variants, result ok/err. |
| T-PT-020 to T-PT-022 | Render | Message op, command agent, result status in markdown. |

### 3.12 Config Tests (8 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-CF-001 | `default_values` | Default model, no key, cache=1000. |
| T-CF-002 | `project_overrides_global` | Same key in both → project wins. |
| T-CF-003 | `env_overrides_all` | Env var takes highest precedence. |
| T-CF-004 | `show_redacts_keys` | API key shown as ***REDACTED***. |
| T-CF-005 | `set_persists` | Set value → read file → value present. |
| T-CF-006 | `invalid_config_handled` | Malformed TOML → fallback to defaults, no crash. |
| T-CF-007 | `set_integer` | Set numeric value → stored as integer. |
| T-CF-008 | `set_creates_file` | Config file doesn't exist → created. |

### 3.13 CLI Binary Tests (10 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-CLI-001 | `no_args_banner` | Shows "Terra" and "Agent OS". |
| T-CLI-002 | `doctor_ok` | Shows "OK". |
| T-CLI-003 | `doctor_json` | Valid JSON with os, version fields. |
| T-CLI-004 | `run_completes` | Exit 0, output contains "complete". |
| T-CLI-005 | `run_json` | Valid JSON output. |
| T-CLI-006 | `run_json_fields` | spec_id, intent, worker_response, tokens present. |
| T-CLI-007 | `help_lists_commands` | Lists doctor, run, config. |
| T-CLI-008 | `unknown_command_fails` | Non-zero exit. |
| T-CLI-009 | `config_show` | Displays config. |
| T-CLI-010 | `config_set_and_show` | Set → show → value visible. |

### 3.14 Integration Tests (8 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-INT-001 | `run_result_stored_in_memory` | Task → result in memory → retrievable. |
| T-INT-002 | `sandbox_enforced_in_executor` | File command → executor checks sandbox. |
| T-INT-003 | `full_caging_chain` | All 8 cages checked for valid task. |
| T-INT-004 | `audit_replay_state` | Run → audit → replay → same state. |
| T-INT-005 | `live_provider_e2e` | Real API through full pipeline (skip if no key). |
| T-INT-006 | `project_isolation` | Two projects completely separate. |
| T-INT-007 | `session_lifecycle_e2e` | Start → work → end → sleep → promotions. |
| T-INT-008 | `compression_protection` | 70% trigger → data externalized → survives compression. |

---

## Test Summary

| Category | Count | Behavior Tests |
|----------|-------|----------------|
| Pipeline Integrity | 17 | 17 (all verify execution flow) |
| Intelligence Core | 15 | 15 (rules active, diagnostics measured) |
| Language Compilation | 18 | 18 |
| Multi-Agent | 16 | 16 (real agents, not wrappers) |
| Memory Database | 30 | 30 (including vector search, concurrency) |
| Caging Engine | 20 | 20 (including symlink, convergence) |
| Provider | 20 | 20 (including retry, timeout, exhaustion) |
| Audit Trail | 12 | 12 (including replay, persistence) |
| Session Lifecycle | 10 | 10 (including sleep consolidation) |
| Artifact Store | 8 | 8 |
| Protocol | 22 | 22 |
| Config | 8 | 8 |
| CLI | 10 | 10 |
| Integration | 8 | 8 |
| Resilience | 3 | 3 (crash recovery, lazy load, model migration) |
| **Total** | **217** | **217 (100% behavioral)** |

---

### 3.15 Resilience Tests (3 tests)

| ID | Test | Behavior Verified |
|----|------|-------------------|
| T-RS-001 | `vector_index_rebuild_from_sqlite` | Delete FAISS index file → restart → index rebuilt from SQLite embedding column → search works. |
| T-RS-002 | `lazy_load_pointer_metadata_only` | Read 1000 entries → only metadata in memory → content loaded only on explicit deref → memory usage stays low. |
| T-RS-003 | `embedding_model_migration` | Change embedding model config → re-index triggered → all entries get new embeddings → search quality maintained. |

---

## 4. Error Taxonomy

```rust
// Memory
MemoryError { NotFound, DuplicateKey, PermissionDenied, Dangling, Stale, PromotionBlocked, InvalidScope, InvalidLevel, Database }

// Provider
ProviderError { MissingApiKey, Network, RateLimit, Timeout, InvalidResponse }

// Caging
SandboxViolation { OutsideSandbox, ProtectedPath, SymlinkEscape }
CageViolation { IncompleteSpec, NoVerification, ConstitutionModified, UnauthorizedWrite, BackwardsPromotion }

// Executor
ExecutorError { CageBlocked, BackendError, CommandNotRouted }
```

## 5. Performance Contracts

| Operation | Target |
|-----------|--------|
| Bloom filter check | < 100ns |
| moka cache read | < 1ms p99 |
| SQLite read | < 5ms p99 |
| MessagePack encode 1KB | < 100μs p99 |
| Vector search 1K entries | < 10ms p99 |
| CLI startup (no embedding) | < 500ms |
| Embedding model load | < 3s |
| Audit log write | < 50μs p99 |

## 6. Migration

- Schema version: `PRAGMA user_version`
- Embedding model change: full re-index
- Config format: auto-migrate with .bak backup
- Skill manifest: `manifest_version` field validated
