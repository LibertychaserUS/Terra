# Terra TDD (Technical Design Document)

> Version: 0.1.0 — MVP
> Last updated: 2026-05-20
> Status: Draft

---

## 1. System Overview

Terra is an **Agent OS** — an operating system for running AI agents.

### 1.1 Architecture Layers (OS Analogy)

| OS Concept | Terra Implementation | Crate |
|-----------|---------------------|-------|
| BIOS / Firmware | Intelligence Core | `terra-core` |
| Kernel | Caging Engine | `terra-caging` |
| Memory Manager | Memory Database | `terra-memory` |
| Process Scheduler | Agent Orchestrator | `terra-agents` |
| Syscall Interface | Command Protocol | `terra-protocol` |
| File System | Artifact Store + Sandbox | `terra-memory` |
| IPC | MessagePack messages | `terra-protocol` |
| Shell | CLI | `terra-cli` |
| Compiler | Language Compilation Layer | `terra-compile` |
| Drivers | Provider Adapters | `terra-providers` |
| Syslog | Audit Trail | `terra-trace` |

### 1.2 Technology Stack

```
Language:         Rust 2021 edition (1.75+)
Async Runtime:    tokio
HTTP Client:      reqwest
LLM SDK:          async-openai (OpenAI-compatible)
Serialization:    serde + rmp-serde (MessagePack) + serde_json + serde_yaml
Database:         rusqlite (SQLite, WAL mode)
Cache:            moka (concurrent cache with TTL/TTI)
Vector Index:     usearch or faiss-rs
Bloom Filter:     bloom crate
CLI:              clap (derive)
Terminal:         ratatui or colored
Logging:          tracing + tracing-subscriber
Error Handling:   anyhow + thiserror
Embedding:        ort (ONNX Runtime, local sentence-transformers)
Testing:          built-in #[test] + tokio::test
```

### 1.3 Project Structure

Cargo workspace with 9 crates. See ARCHITECTURE.md Section 12 for full tree.

---

## 2. Module Specifications

### 2.1 terra-protocol

**Purpose:** Inter-agent communication protocol and command interface.

**Core Types:**
```rust
AgentMessage { id, from, to, kind, timestamp, payload }
AgentRole { Main, Worker, Explorer, Cage }
MessagePayload { Task(TaskPayload), Result(ResultPayload) }
Command { id, agent, timestamp, op: CommandOp }
CommandOp { MemWrite, MemRead, MemSearch, ArtifactSave, ArtifactLoad, ... }
CommandResult { command_id, status, data, error }
```

**MessagePack Codec:**
- `encode<T: Serialize>(value) -> Vec<u8>` using `rmp_serde::to_vec_named`
- `decode<T: DeserializeOwned>(bytes) -> T`
- Named fields for Option compatibility

**Markdown Renderer:**
- `render_message(msg) -> String` — human-readable view of agent messages
- `render_command(cmd) -> String` — human-readable view of commands

### 2.2 terra-compile

**Purpose:** Transform user natural language into standardized Spec IR.

**Core Types:**
```rust
SpecIR {
    spec_id, intent, target: Target,
    constraints, success_predicate, open_questions,
    resolved_terms: Vec<TermMapping>
}
Target { file, line, function, component, scope }
TermMapping { user_term, normalized }
```

**MVP Compilation:** `SpecIR::from_task_description(str)` — minimal extraction.
**Full Compilation (future):** LLM-assisted lexical, semantic, normalization passes.

### 2.3 terra-providers

**Purpose:** LLM provider abstraction with dispatch and retry.

**Core Trait:**
```rust
#[async_trait]
trait ProviderAdapter: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    fn capabilities(&self) -> &ProviderCapability;
}
```

**Implementations:**
- `DryRunAdapter` — deterministic responses, zero API calls
- `OpenAICompatibleAdapter` (MVP2) — real LLM calls with retry/backoff

**Dispatch:**
```rust
async fn dispatch(adapter, request, mode: DryRun|Live) -> DispatchResult
```

**Capability Matrix:**
```rust
ProviderCapability {
    provider_id, model_id,
    supports_structured_output, supports_streaming,
    max_context_tokens, latency_class: Fast|Medium|Slow
}
```

### 2.4 terra-memory

**Purpose:** Rust-safe Memory Database with SQLite + moka + vector index.

**Storage Architecture:**
```
Bloom Filter (ns) → moka Cache (ns) → Project SQLite (ms) → Global SQLite (ms)
```

**Scopes:** S0 Global, S1 Project, S2 Session, S3 Task
**Levels:** L0-L6 (Authoritative → Model Candidates)

**Safety Rules (enforced at compile time and runtime):**
- Ownership: one owner per entry (SQL CHECK constraint + Rust type)
- Borrowing: multiple readers, exclusive writer (SQLite WAL + app-level lock)
- Lifetime: references cannot outlive referents (SQL FOREIGN KEY + app check)
- No null: NOT NULL constraints + Rust Option<T>
- GC: reference counting + periodic integrity scan

**Schema (SQLite):**
```sql
CREATE TABLE memories (
    id          TEXT PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    scope       TEXT NOT NULL CHECK (scope IN ('global','project','session','task')),
    level       TEXT NOT NULL CHECK (level IN ('L0','L1','L2','L3','L4','L5','L6')),
    owner       TEXT NOT NULL,
    content     BLOB NOT NULL,  -- MessagePack
    version     INTEGER NOT NULL DEFAULT 1,
    ref_count   INTEGER NOT NULL DEFAULT 0,
    alive       INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE pointers (
    ptr_id      TEXT PRIMARY KEY,
    source_id   TEXT NOT NULL REFERENCES memories(id),
    target_id   TEXT NOT NULL REFERENCES memories(id),
    ref_type    TEXT NOT NULL CHECK (ref_type IN ('owned','borrowed','weak')),
    CHECK (source_id != target_id)
);

CREATE TABLE artifacts (
    id          TEXT PRIMARY KEY,
    type        TEXT NOT NULL CHECK (type IN ('plan','spec','test','review')),
    version     INTEGER NOT NULL,
    content     BLOB NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE audit_log (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp   TEXT NOT NULL,
    agent       TEXT NOT NULL,
    command     BLOB NOT NULL,  -- MessagePack serialized Command
    result      BLOB,           -- MessagePack serialized CommandResult
    duration_us INTEGER
);
```

**Attention-based Retrieval:**
- Embedding: local sentence-transformers via ONNX Runtime (384-dim)
- Index: FAISS or usearch, rebuilt from SQLite on startup
- Multi-Head: each Domain Module contributes a perspective (query vector)
- Dynamic salience: computed at retrieval time, not stored

### 2.5 terra-core

**Purpose:** Intelligence Core — agent's first principles.

**Structure:**
```rust
struct IntelligenceCore {
    universal: UniversalFramework,
    modules: Vec<Box<dyn DomainModule>>,
}

trait DomainModule: Send + Sync {
    fn name(&self) -> &str;
    fn perspectives(&self) -> Vec<Perspective>;
    fn rules(&self) -> Vec<Rule>;
    fn verifiers(&self) -> Vec<Verifier>;
}
```

**Universal Framework Rules (L0, read-only):**
- Reasoning: depth-first, evidence-based, decompose complexity
- Behavioral: elegant code, minimal change, safe defaults
- Self-Diagnostic: detect goal drift, shallow reasoning, shortcuts

### 2.6 terra-caging

**Purpose:** Implementation of Caging Engineering's 8 cages.

**Core Trait:**
```rust
trait Cage: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, state: &AgentState) -> CageCheckResult;
}

enum CageCheckResult {
    Pass,
    Warn(String),
    Block(String),  // prevents action
}
```

**Cages:**
1. SpecificationCage — validates TaskSpec completeness
2. KnowledgeCage — authority ranking enforcement
3. MemoryCage — write permission, promotion gate
4. ActionCage — tool/file permissions, SandboxGuard
5. VerificationCage — requires external verifier pass
6. ConvergenceCage — stops exploration after success
7. ParametricMemoryCage — L6 promotion requires human review
8. MetaCage — constitution integrity, cage modification detection

**SandboxGuard:**
```rust
struct SandboxGuard {
    root: PathBuf,
    protected: Vec<PathBuf>,
}

fn check_write(&self, path: &Path) -> Result<(), SandboxViolation>
fn check_read(&self, path: &Path) -> Result<(), SandboxViolation>
fn check_delete(&self, path: &Path) -> Result<(), SandboxViolation>
```

### 2.7 terra-agents

**Purpose:** Multi-agent orchestration.

**Orchestrator Pipeline:**
```
User Input → SpecIR → Plan → Dispatch Sub-agents → Audit Results → Output
```

**Agent Dispatch Policy:**
```rust
struct ModelDispatchPolicy {
    main_model: ModelConfig,
    worker_model: ModelConfig,
    explorer_model: ModelConfig,
    cage_model: Option<ModelConfig>,
}
```

Auto-detected from Provider capability matrix.

### 2.8 terra-trace

**Purpose:** Auditable operation trail.

**AuditLog:** Sequential command recording with replay capability.
**Checkpoint:** Named milestones with summary.
**Render:** Markdown output for human review.

### 2.9 terra-cli

**Purpose:** User-facing CLI.

**Commands:**
```
terra                              -- banner + env scan
terra doctor [--as-json]           -- environment readiness
terra run --task "..." [--as-json] -- execute task
terra memory read <key>            -- inspect memory
terra memory search <query>        -- semantic search
terra trace show                   -- display audit trail
terra config show                  -- show configuration
terra config set <key> <value>     -- update setting
```

**Config System:**
- `~/.terra/config.toml` — global settings
- `project/.terra/config.toml` — project overrides
- Environment variables: `TERRA_PROVIDER_API_KEY`, `TERRA_PROVIDER_BASE_URL`
- `terra doctor` detects: OS, Rust version, Provider availability, SQLite, available models

---

## 3. Test Specification (100+ test cases)

### 3.1 terra-protocol (20 tests)

#### Message Tests
| # | Test | Description |
|---|------|-------------|
| T-P01 | `task_assignment_creation` | Create a task assignment message with all fields |
| T-P02 | `task_result_creation` | Create a result message with success status |
| T-P03 | `task_result_failure` | Create a result message with failure and error |
| T-P04 | `message_id_uniqueness` | Two messages have different UUIDs |
| T-P05 | `message_timestamp_is_utc` | Timestamp is in UTC |
| T-P06 | `agent_role_serialization` | All AgentRole variants serialize/deserialize |

#### Codec Tests
| # | Test | Description |
|---|------|-------------|
| T-P07 | `msgpack_roundtrip_task` | TaskPayload survives encode→decode |
| T-P08 | `msgpack_roundtrip_result` | ResultPayload survives encode→decode |
| T-P09 | `msgpack_roundtrip_command` | Command survives encode→decode |
| T-P10 | `msgpack_smaller_than_json` | MessagePack output is smaller than JSON for same data |
| T-P11 | `msgpack_with_optional_fields` | Optional fields (None) don't break codec |
| T-P12 | `decode_invalid_bytes_returns_error` | Random bytes return Err, not panic |

#### Command Tests
| # | Test | Description |
|---|------|-------------|
| T-P13 | `mem_write_command` | Create MemWrite with all required fields |
| T-P14 | `mem_read_command` | Create MemRead command |
| T-P15 | `mem_search_command` | Create MemSearch with query and top_k |
| T-P16 | `artifact_save_command` | Create ArtifactSave with type and content |
| T-P17 | `trace_checkpoint_command` | Create TraceCheckpoint |
| T-P18 | `command_result_ok` | CommandResult::ok with data |
| T-P19 | `command_result_error` | CommandResult::err with message |

#### Render Tests
| # | Test | Description |
|---|------|-------------|
| T-P20 | `render_task_message_contains_op` | Markdown contains operation name |
| T-P21 | `render_command_contains_agent` | Markdown contains agent name |
| T-P22 | `render_result_contains_status` | Markdown contains status |

### 3.2 terra-compile (15 tests)

#### SpecIR Tests
| # | Test | Description |
|---|------|-------------|
| T-C01 | `from_description_sets_intent` | Intent matches input description |
| T-C02 | `from_description_incomplete` | Has open questions (not complete) |
| T-C03 | `manually_built_spec_complete` | Spec with all fields, no open questions = complete |
| T-C04 | `spec_ir_json_roundtrip` | Serialize to JSON and back preserves all fields |
| T-C05 | `spec_ir_msgpack_roundtrip` | Serialize to MessagePack and back |
| T-C06 | `target_with_file_and_line` | Target correctly stores file:line |
| T-C07 | `target_with_component` | Target correctly stores component name |
| T-C08 | `constraints_preserved` | Multiple constraints are all preserved |
| T-C09 | `success_predicate_preserved` | Success predicates are all preserved |
| T-C10 | `term_mapping_user_to_normalized` | Term mappings store both forms |
| T-C11 | `empty_description_is_valid` | Empty string doesn't panic |
| T-C12 | `unicode_description_handled` | Chinese/emoji in description works |
| T-C13 | `long_description_handled` | 10K character description doesn't panic |
| T-C14 | `spec_id_is_uuid` | spec_id is a valid UUID format |
| T-C15 | `multiple_open_questions` | Multiple open questions tracked |

### 3.3 terra-providers (18 tests)

#### DryRunAdapter Tests
| # | Test | Description |
|---|------|-------------|
| T-PR01 | `dry_run_returns_deterministic` | Same input → same output pattern |
| T-PR02 | `dry_run_zero_tokens` | Usage is 0 prompt + 0 completion |
| T-PR03 | `dry_run_capabilities` | Reports correct capability matrix |
| T-PR04 | `dry_run_no_network` | Works without network (no external calls) |
| T-PR05 | `dry_run_model_is_dry_run` | Response model field is "dry_run" |

#### OpenAI-Compatible Adapter Tests
| # | Test | Description |
|---|------|-------------|
| T-PR06 | `openai_adapter_creation` | Create with API key and base URL |
| T-PR07 | `openai_adapter_capabilities` | Reports capabilities from config |
| T-PR08 | `openai_missing_api_key_error` | Error when TERRA_PROVIDER_API_KEY not set |
| T-PR09 | `openai_completion_success` | Successful completion (live test, skippable) |
| T-PR10 | `openai_completion_retry_on_429` | Retries on rate limit (mock) |
| T-PR11 | `openai_completion_timeout` | Timeout after configured duration |
| T-PR12 | `openai_streaming_basic` | Stream returns tokens incrementally (future) |

#### Dispatch Tests
| # | Test | Description |
|---|------|-------------|
| T-PR13 | `dispatch_dry_run_mode` | DryRun mode uses DryRunAdapter |
| T-PR14 | `dispatch_live_mode` | Live mode calls real adapter |
| T-PR15 | `dispatch_result_contains_mode` | Result reflects dispatch mode |
| T-PR16 | `dispatch_result_contains_response` | Result has response content |
| T-PR17 | `dispatch_result_contains_usage` | Result has token usage |
| T-PR18 | `dispatch_propagates_adapter_error` | Adapter error propagated to caller |

### 3.4 terra-memory (25 tests)

#### SQLite Store Tests
| # | Test | Description |
|---|------|-------------|
| T-M01 | `create_database_and_tables` | Database and all tables created on init |
| T-M02 | `write_and_read_memory` | Write a memory entry, read it back |
| T-M03 | `write_duplicate_key_returns_error` | Duplicate key insert fails |
| T-M04 | `read_nonexistent_key_returns_none` | Missing key returns None, not error |
| T-M05 | `update_increments_version` | Update bumps version number |
| T-M06 | `delete_sets_alive_false` | Soft delete, not physical delete |
| T-M07 | `content_stored_as_msgpack` | Content BLOB is valid MessagePack |
| T-M08 | `scope_constraint_enforced` | Invalid scope string rejected |
| T-M09 | `level_constraint_enforced` | Invalid level string rejected |

#### Ownership Tests
| # | Test | Description |
|---|------|-------------|
| T-M10 | `owner_set_on_creation` | Owner field set when memory created |
| T-M11 | `non_owner_cannot_update` | Update by non-owner agent fails |
| T-M12 | `non_owner_can_read` | Read by non-owner succeeds |
| T-M13 | `ownership_transfer` | Explicit ownership transfer works |

#### Lifetime Tests
| # | Test | Description |
|---|------|-------------|
| T-M14 | `pointer_alive_check` | Deref checks alive status |
| T-M15 | `dangling_pointer_detected` | Pointer to deleted entry returns error |
| T-M16 | `version_mismatch_detected` | Stale version returns MemoryStale error |
| T-M17 | `gc_removes_zero_refcount` | GC collects entries with ref_count 0 and weak lifetime |
| T-M18 | `gc_preserves_authoritative` | GC never removes L0 entries |

#### Cache Tests
| # | Test | Description |
|---|------|-------------|
| T-M19 | `cache_hit_returns_fast` | Cached entry returned without SQLite query |
| T-M20 | `cache_miss_falls_through_to_sqlite` | Cache miss queries SQLite |
| T-M21 | `write_updates_cache_and_sqlite` | Write goes to both layers |
| T-M22 | `cache_eviction_on_capacity` | Entries evicted when cache is full |

#### Promotion Tests
| # | Test | Description |
|---|------|-------------|
| T-M23 | `promote_l2_to_l1` | Task memory promoted to project memory |
| T-M24 | `promote_requires_verification` | L3 promotion requires verification evidence |
| T-M25 | `promote_l6_requires_review` | Model candidate promotion blocked without review |

### 3.5 terra-core (12 tests)

#### Intelligence Core Tests
| # | Test | Description |
|---|------|-------------|
| T-IC01 | `core_loads_universal_framework` | Universal layer loaded on init |
| T-IC02 | `core_registers_domain_module` | Domain module registered successfully |
| T-IC03 | `core_lists_all_perspectives` | All perspectives from all modules listed |
| T-IC04 | `core_lists_all_rules` | All rules from all modules listed |
| T-IC05 | `coding_module_has_perspectives` | Coding module provides perspectives |
| T-IC06 | `coding_module_has_rules` | Coding module provides rules |
| T-IC07 | `algorithm_module_has_perspectives` | Algorithm module provides perspectives |
| T-IC08 | `modules_are_composable` | Multiple modules active simultaneously |
| T-IC09 | `universal_rules_are_readonly` | Cannot modify universal framework at runtime |
| T-IC10 | `self_diagnostic_goal_drift` | Detect goal drift scenario |
| T-IC11 | `self_diagnostic_shallow_reasoning` | Detect shallow reasoning scenario |
| T-IC12 | `self_diagnostic_shortcut` | Detect shortcut-taking scenario |

### 3.6 terra-caging (18 tests)

#### Cage Trait Tests
| # | Test | Description |
|---|------|-------------|
| T-CG01 | `cage_pass_result` | CageCheckResult::Pass allows action |
| T-CG02 | `cage_warn_result` | CageCheckResult::Warn logs but allows |
| T-CG03 | `cage_block_result` | CageCheckResult::Block prevents action |

#### Specification Cage
| # | Test | Description |
|---|------|-------------|
| T-CG04 | `spec_cage_passes_complete_spec` | Complete SpecIR passes |
| T-CG05 | `spec_cage_blocks_incomplete_spec` | Incomplete SpecIR blocked |

#### Memory Cage
| # | Test | Description |
|---|------|-------------|
| T-CG06 | `memory_cage_allows_owner_write` | Owner can write |
| T-CG07 | `memory_cage_blocks_non_owner_write` | Non-owner blocked |
| T-CG08 | `memory_cage_allows_any_read` | Any agent can read |

#### Action Cage + Sandbox
| # | Test | Description |
|---|------|-------------|
| T-CG09 | `sandbox_allows_write_inside_root` | Write inside project dir: ok |
| T-CG10 | `sandbox_blocks_write_outside_root` | Write outside project dir: blocked |
| T-CG11 | `sandbox_allows_read_outside_root` | Read outside project dir: ok |
| T-CG12 | `sandbox_blocks_write_to_protected` | Write to .terra/constitution: blocked |
| T-CG13 | `sandbox_allows_read_of_protected` | Read .terra/constitution: ok |
| T-CG14 | `sandbox_blocks_delete_outside_root` | Delete outside project dir: blocked |

#### Verification Cage
| # | Test | Description |
|---|------|-------------|
| T-CG15 | `verify_cage_requires_external_check` | Completion without verification: blocked |
| T-CG16 | `verify_cage_passes_after_verification` | Completion after verification: pass |

#### Meta Cage
| # | Test | Description |
|---|------|-------------|
| T-CG17 | `meta_cage_detects_constitution_change` | Modification attempt detected |
| T-CG18 | `meta_cage_detects_cage_rule_change` | Rule modification attempt detected |

### 3.7 terra-agents (12 tests)

#### Orchestrator Tests
| # | Test | Description |
|---|------|-------------|
| T-A01 | `end_to_end_dry_run` | Full pipeline with DryRunAdapter |
| T-A02 | `spec_ir_created_from_task` | Task description → SpecIR |
| T-A03 | `worker_dispatched_via_provider` | Provider called with correct prompt |
| T-A04 | `audit_trail_recorded` | 3+ audit entries after run |
| T-A05 | `result_contains_worker_response` | RunResult has response content |
| T-A06 | `result_contains_token_usage` | RunResult has token counts |
| T-A07 | `result_contains_spec_id` | RunResult has spec ID |
| T-A08 | `empty_task_handled` | Empty task string doesn't panic |
| T-A09 | `long_task_handled` | Very long task string works |
| T-A10 | `provider_error_propagated` | Provider failure → Run failure |
| T-A11 | `multiple_sequential_runs` | Two runs in sequence, independent audit |
| T-A12 | `concurrent_runs_safe` | Two concurrent runs don't interfere (future) |

### 3.8 terra-trace (10 tests)

#### Audit Log Tests
| # | Test | Description |
|---|------|-------------|
| T-TR01 | `record_command` | Command recorded with seq number |
| T-TR02 | `record_result` | Result attached to existing seq |
| T-TR03 | `sequential_numbering` | seq numbers increment: 1, 2, 3... |
| T-TR04 | `entries_retrievable` | All entries returned in order |
| T-TR05 | `render_markdown` | Markdown output contains key fields |
| T-TR06 | `empty_log_renders` | Empty log renders valid markdown |
| T-TR07 | `result_without_command_ignored` | Result for unknown seq doesn't panic |
| T-TR08 | `concurrent_recording_safe` | Multiple threads recording simultaneously |
| T-TR09 | `large_log_performance` | 10K entries don't degrade performance |
| T-TR10 | `audit_log_msgpack_roundtrip` | Entries survive MessagePack serialization |

### 3.9 terra-cli (8 tests)

#### CLI Tests
| # | Test | Description |
|---|------|-------------|
| T-CLI01 | `no_args_shows_banner` | Running `terra` shows version info |
| T-CLI02 | `doctor_shows_status` | `terra doctor` shows OK |
| T-CLI03 | `doctor_as_json` | `terra doctor --as-json` outputs valid JSON |
| T-CLI04 | `run_with_task` | `terra run --task "..."` completes |
| T-CLI05 | `run_as_json` | `terra run --as-json` outputs valid JSON |
| T-CLI06 | `run_json_has_required_fields` | JSON has spec_id, intent, response |
| T-CLI07 | `help_flag` | `terra --help` shows all commands |
| T-CLI08 | `unknown_command_error` | Invalid subcommand returns error |

### 3.10 Config System (6 tests)

| # | Test | Description |
|---|------|-------------|
| T-CF01 | `default_config_created` | First run creates ~/.terra/config.toml |
| T-CF02 | `project_config_overrides_global` | Project config takes precedence |
| T-CF03 | `env_var_overrides_config` | TERRA_PROVIDER_API_KEY overrides config |
| T-CF04 | `config_show_displays_all` | `terra config show` lists all settings |
| T-CF05 | `config_set_persists` | `terra config set` writes to file |
| T-CF06 | `invalid_config_handled` | Malformed config.toml doesn't crash |

### 3.11 Integration Tests (6 tests)

| # | Test | Description |
|---|------|-------------|
| T-INT01 | `end_to_end_with_memory` | Run → result stored in memory → retrievable |
| T-INT02 | `end_to_end_with_sandbox` | Run → file operations checked by sandbox |
| T-INT03 | `end_to_end_with_caging` | Run → all 8 cages checked |
| T-INT04 | `end_to_end_with_audit_replay` | Run → audit log → replay produces same state |
| T-INT05 | `live_provider_smoke_test` | Real API call (skipped if no key) |
| T-INT06 | `project_isolation` | Two projects have separate memory databases |

---

**Total: 150 test cases across 11 categories.**

---

## 4. Data Flow

```
User Input (natural language)
    │
    ▼
Language Compilation Layer ──→ Spec IR
    │
    ▼
Orchestrator (Main Agent)
    │
    ├──→ Intelligence Core (query perspectives + rules)
    │
    ├──→ Command Executor ──→ Memory Database
    │    (~15 tokens/cmd)     (Bloom → moka → SQLite)
    │
    ├──→ Sub-agent Worker (MessagePack) ◄──┐
    │                                       │ Cage Sub-agent
    ├──→ Sub-agent Explorer (MessagePack) ◄─┘ (monitoring)
    │
    ├──→ Verification Cage (external check)
    │
    └──→ Audit Trail (every operation logged)
    │
    ▼
Verified Output ──→ render_to_markdown ──→ User
```

## 5. Security Model

- **Sandbox:** Project directory write, outside read-only
- **Credentials:** Environment variables only, never disk
- **Audit:** Every command logged, replayable
- **Cage Integrity:** Constitution and cage rules immutable at runtime
- **Memory Safety:** Rust ownership model + SQLite constraints

## 6. Performance Targets

| Metric | Target |
|--------|--------|
| Cache hit latency | < 1ms |
| SQLite read latency | < 5ms |
| MessagePack encode | < 100μs per message |
| Vector search (1K entries) | < 10ms |
| CLI startup | < 500ms (without embedding model) |
| Embedding model load | < 3s (first load, then daemon) |

## 7. Error Handling Strategy

- All fallible operations return `Result<T, E>`
- Domain errors via `thiserror` (specific, matchable)
- Unexpected errors via `anyhow` (context chain)
- No `.unwrap()` in production code
- Cage violations logged + blocked (not panicked)
- Provider errors: retry with exponential backoff (3 attempts)

## 8. Migration Strategy

- Schema version stored in SQLite: `PRAGMA user_version`
- Forward-only migrations
- Embedding model change: full re-index from SQLite content
- Config format change: auto-migrate with backup
