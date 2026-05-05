# Terra Technical Design Document

> Document ID: TERRA-TDD-001
> Version: 1.0.0
> Status: DRAFT
> Date: 2026-05-20
> Author: Oliver Zhang, Claude (Anthropic)
> Classification: Public
>
> This document follows the structure of POSIX interface documentation
> and Linux kernel module specifications. Each module is described with:
> PURPOSE, INTERFACE, BEHAVIOR, ERROR CONDITIONS, and TEST CASES.
>
> RFC 2119 keywords (MUST, SHALL, SHOULD, MAY) apply as defined
> in the companion PRD (TERRA-PRS-001).

---

## Table of Contents

1. System Architecture
2. Module Specifications
3. Test Specification
4. Data Schemas
5. Error Taxonomy
6. Performance Contracts
7. Migration and Versioning

---

## 1. System Architecture

### 1.1 OS Layer Mapping

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 6: Shell                                               │
│   terra-cli — user-facing CLI, config management             │
├─────────────────────────────────────────────────────────────┤
│ Layer 5: Application Services                                │
│   terra-agents — orchestrator, worker, explorer, cage agent  │
│   terra-compile — language compilation, Spec IR              │
├─────────────────────────────────────────────────────────────┤
│ Layer 4: Kernel Services                                     │
│   terra-caging — 8 cages, SandboxGuard                       │
│   terra-core — Intelligence Core, domain modules             │
├─────────────────────────────────────────────────────────────┤
│ Layer 3: Syscall Interface                                   │
│   terra-protocol — Command Protocol, MessagePack codec       │
├─────────────────────────────────────────────────────────────┤
│ Layer 2: Subsystems                                          │
│   terra-memory — Memory Database, cache, vector index        │
│   terra-trace — audit log, checkpoints                       │
├─────────────────────────────────────────────────────────────┤
│ Layer 1: Drivers                                             │
│   terra-providers — LLM adapters, capability detection       │
├─────────────────────────────────────────────────────────────┤
│ Layer 0: Hardware Abstraction                                │
│   SQLite, FAISS/usearch, moka, ONNX Runtime, file system     │
└─────────────────────────────────────────────────────────────┘
```

**Dependency Rule:** Higher layers MAY depend on lower layers.
Lower layers MUST NOT depend on higher layers. Circular
dependencies are forbidden.

### 1.2 Crate Dependency Graph

```
terra-cli
  └── terra-agents
        ├── terra-core
        ├── terra-compile
        ├── terra-providers
        ├── terra-caging
        │     ├── terra-core
        │     └── terra-protocol
        ├── terra-memory
        │     └── terra-protocol
        ├── terra-trace
        │     └── terra-protocol
        └── terra-protocol
```

### 1.3 Technology Stack

| Component | Crate | Version | Purpose |
|-----------|-------|---------|---------|
| Async runtime | tokio | 1.x | Multi-agent concurrency |
| Serialization | serde + rmp-serde | 1.x | MessagePack codec |
| Database | rusqlite | 0.32+ | SQLite with WAL |
| Cache | moka | 0.12+ | Concurrent TTL/TTI cache |
| Vector search | usearch or faiss-rs | latest | Embedding retrieval |
| LLM SDK | async-openai | 0.27+ | OpenAI-compatible dispatch |
| CLI | clap | 4.x | Derive-based arg parsing |
| Logging | tracing | 0.1.x | Structured logging |
| Errors | anyhow + thiserror | latest | Error handling |
| Embedding | ort | latest | ONNX Runtime for local models |
| HTTP | reqwest | 0.12+ | Provider communication |

---

## 2. Module Specifications

### 2.1 terra-protocol

#### 2.1.1 PURPOSE

Defines the communication protocol between all Terra components:
agent-to-agent messages, agent-to-system commands, and the
serialization/rendering layers.

#### 2.1.2 INTERFACE

```rust
// --- Messages ---

pub struct AgentMessage {
    pub id: String,           // UUID v4
    pub from: AgentRole,      // Main | Worker | Explorer | Cage
    pub to: AgentRole,
    pub kind: MessageKind,    // TaskAssignment | TaskResult | CageReport
    pub timestamp: DateTime<Utc>,
    pub payload: MessagePayload,
}

impl AgentMessage {
    pub fn task_assignment(to: AgentRole, payload: TaskPayload) -> Self;
    pub fn task_result(from: AgentRole, payload: ResultPayload) -> Self;
}

// --- Commands ---

pub struct Command {
    pub id: String,           // UUID v4
    pub agent: String,        // agent identifier
    pub timestamp: DateTime<Utc>,
    pub op: CommandOp,        // one of ~15 variants
}

pub struct CommandResult {
    pub command_id: String,
    pub status: CommandStatus, // Ok | NotFound | PermissionDenied | Error
    pub data: Option<Value>,
    pub error: Option<String>,
}

// --- Codec ---

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>>;
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T>;

// --- Render ---

pub fn render_message(msg: &AgentMessage) -> String;  // → Markdown
pub fn render_command(cmd: &Command) -> String;        // → Markdown
```

#### 2.1.3 BEHAVIOR

- `encode` uses `rmp_serde::to_vec_named` (named fields for Option compatibility).
- `decode` uses `rmp_serde::from_slice`.
- All IDs are UUID v4, generated at creation time.
- Timestamps are UTC, generated at creation time.
- `render_*` functions produce deterministic Markdown output.
  No LLM involvement. No side effects.

#### 2.1.4 ERROR CONDITIONS

| Error | Condition | Response |
|-------|-----------|----------|
| `SerializationError` | Value cannot be serialized to MessagePack | Return `Err` |
| `DeserializationError` | Bytes are not valid MessagePack or wrong schema | Return `Err` |

No panics. No `.unwrap()`.

---

### 2.2 terra-compile

#### 2.2.1 PURPOSE

Transforms user natural language into a standardized Spec IR
(Specification Intermediate Representation).

#### 2.2.2 INTERFACE

```rust
pub struct SpecIR {
    pub spec_id: String,              // UUID v4
    pub intent: String,               // normalized task intent
    pub target: Target,               // what to operate on
    pub constraints: Vec<String>,     // rules the agent must follow
    pub success_predicate: Vec<String>, // how to verify success
    pub open_questions: Vec<String>,  // unresolved ambiguities
    pub resolved_terms: Vec<TermMapping>, // user term → normalized
}

pub struct Target {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub function: Option<String>,
    pub component: Option<String>,
    pub scope: String,
}

impl SpecIR {
    pub fn from_task_description(description: &str) -> Self;
    pub fn is_complete(&self) -> bool;
}
```

#### 2.2.3 BEHAVIOR

- `from_task_description`: MVP creates a minimal SpecIR with the
  description as intent, no target resolution, one open question.
- `is_complete`: returns `true` iff `open_questions.is_empty()`.
- All string fields accept UTF-8 (including CJK, emoji).
- Empty description is valid (produces SpecIR with empty intent).

#### 2.2.4 PRECONDITIONS

None. This function is infallible.

#### 2.2.5 POSTCONDITIONS

- `spec_id` is a valid UUID v4 string.
- `intent == description` (in MVP; future: normalized).
- `open_questions.len() >= 1` (in MVP, always incomplete).

---

### 2.3 terra-providers

#### 2.3.1 PURPOSE

Abstracts LLM providers behind a common trait, supporting
dispatch with retry and capability detection.

#### 2.3.2 INTERFACE

```rust
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse>;
    fn capabilities(&self) -> &ProviderCapability;
}

pub struct CompletionRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

pub struct CompletionResponse {
    pub content: String,
    pub usage_prompt_tokens: u32,
    pub usage_completion_tokens: u32,
    pub model: String,
}

pub async fn dispatch(
    adapter: &dyn ProviderAdapter,
    request: CompletionRequest,
    mode: DispatchMode,  // DryRun | Live
) -> Result<DispatchResult>;
```

#### 2.3.3 IMPLEMENTATIONS

**DryRunAdapter:**
- Returns `"DRY_RUN response to: {first 100 chars of user_prompt}"`.
- Usage is always 0/0.
- No network calls. No API key required.
- Capabilities: structured_output=true, streaming=false, latency=Fast.

**OpenAICompatibleAdapter (MVP2):**
- Reads `TERRA_PROVIDER_API_KEY` from env.
- Reads `TERRA_PROVIDER_BASE_URL` from env (default: `https://api.openai.com/v1`).
- Calls `/v1/chat/completions` via `async-openai` crate.
- Retry: 3 attempts, exponential backoff (1s, 2s, 4s), on HTTP 429/500/502/503.
- Timeout: 60 seconds per request.

#### 2.3.4 ERROR CONDITIONS

| Error | Condition | Response |
|-------|-----------|----------|
| `ProviderError::MissingApiKey` | TERRA_PROVIDER_API_KEY not set in Live mode | Return `Err` |
| `ProviderError::NetworkError` | Connection failed after retries | Return `Err` |
| `ProviderError::RateLimit` | 429 after all retries exhausted | Return `Err` |
| `ProviderError::Timeout` | Request exceeded 60s | Return `Err` |
| `ProviderError::InvalidResponse` | Response body unparseable | Return `Err` |

---

### 2.4 terra-memory

#### 2.4.1 PURPOSE

Rust-safe Memory Database. Manages scoped, leveled memory
with ownership, borrowing, lifetime, cache, and vector retrieval.

#### 2.4.2 INTERFACE

```rust
pub struct MemoryDatabase {
    // Internal: SQLite + moka + optional vector index
}

impl MemoryDatabase {
    pub async fn new(db_path: &Path) -> Result<Self>;
    pub async fn write(&self, entry: MemoryEntry) -> Result<()>;
    pub async fn read(&self, key: &str) -> Result<Option<MemoryEntry>>;
    pub async fn update(&self, key: &str, content: Vec<u8>, agent: &str) -> Result<()>;
    pub async fn delete(&self, key: &str, agent: &str) -> Result<()>;
    pub async fn promote(&self, key: &str, target_level: Level) -> Result<()>;
    pub async fn search(&self, query: &str, top_k: usize) -> Result<Vec<MemoryEntry>>;
    pub async fn gc(&self) -> Result<GcReport>;
}

pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub scope: Scope,       // Global | Project | Session | Task
    pub level: Level,       // L0..L6
    pub owner: String,
    pub content: Vec<u8>,   // MessagePack blob
    pub version: u64,
    pub ref_count: u64,
    pub alive: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

#### 2.4.3 STORAGE ARCHITECTURE

```
Query path (read):
  Bloom Filter (ns) → moka cache (ns) → Project SQLite (ms) → Global SQLite (ms)

Write path:
  Validate ownership → moka cache (sync) → SQLite (write-through for L0-L1, write-back for L2+)

Startup:
  Open/create SQLite → create tables if needed → PRAGMA journal_mode=WAL
  → build Bloom Filter from all keys → warm moka cache with recent entries
```

#### 2.4.4 SCHEMA (SQLite)

```sql
PRAGMA journal_mode = WAL;
PRAGMA user_version = 1;  -- schema version for migration

CREATE TABLE IF NOT EXISTS memories (
    id          TEXT PRIMARY KEY,
    key         TEXT NOT NULL UNIQUE,
    scope       TEXT NOT NULL CHECK (scope IN ('global','project','session','task')),
    level       TEXT NOT NULL CHECK (level IN ('L0','L1','L2','L3','L4','L5','L6')),
    owner       TEXT NOT NULL,
    content     BLOB NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1,
    ref_count   INTEGER NOT NULL DEFAULT 0,
    alive       INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS artifacts (
    id          TEXT PRIMARY KEY,
    type        TEXT NOT NULL CHECK (type IN ('plan','spec','test','review')),
    version     INTEGER NOT NULL,
    content     BLOB NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_log (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp   TEXT NOT NULL,
    agent       TEXT NOT NULL,
    command     BLOB NOT NULL,
    result      BLOB,
    duration_us INTEGER
);

CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope);
CREATE INDEX IF NOT EXISTS idx_memories_level ON memories(level);
CREATE INDEX IF NOT EXISTS idx_memories_owner ON memories(owner);
CREATE INDEX IF NOT EXISTS idx_memories_alive ON memories(alive);
```

#### 2.4.5 ERROR CONDITIONS

| Error | Condition | Response |
|-------|-----------|----------|
| `MemoryError::NotFound` | Key does not exist | Return `None` (read) or `Err` (update) |
| `MemoryError::DuplicateKey` | Key already exists on write | Return `Err` |
| `MemoryError::PermissionDenied` | Non-owner attempting write/delete | Return `Err` |
| `MemoryError::Dangling` | Pointer to deleted (alive=false) entry | Return `Err` |
| `MemoryError::Stale` | Version mismatch (optimistic lock) | Return `Err` |
| `MemoryError::PromotionBlocked` | L6 promotion without review flag | Return `Err` |
| `MemoryError::DatabaseError` | SQLite internal error | Return `Err` |

---

### 2.5 terra-core

#### 2.5.1 PURPOSE

Intelligence Core — agent's built-in first principles.
Provides universal reasoning/behavioral frameworks and
domain module registry.

#### 2.5.2 INTERFACE

```rust
pub struct IntelligenceCore {
    universal: UniversalFramework,
    modules: Vec<Box<dyn DomainModule>>,
}

pub trait DomainModule: Send + Sync {
    fn name(&self) -> &str;
    fn perspectives(&self) -> Vec<Perspective>;
    fn rules(&self) -> Vec<Rule>;
    fn verifiers(&self) -> Vec<Box<dyn Verifier>>;
}

pub struct Perspective {
    pub name: String,
    pub description: String,
    pub query_template: String,  // template for generating search query
}

pub struct Rule {
    pub id: String,
    pub category: RuleCategory,  // Reasoning | Behavioral | Diagnostic
    pub description: String,
    pub severity: Severity,      // Must | Should | May
}
```

#### 2.5.3 BEHAVIOR

- Universal framework rules are loaded once at startup, immutable.
- Domain modules are registered at startup from Skill manifests.
- `perspectives()` collects perspectives from all registered modules.
- `rules()` collects rules from all registered modules + universal.
- Rules are checked by the Caging Engine, not by the Core itself.

---

### 2.6 terra-caging

#### 2.6.1 PURPOSE

Implements Caging Engineering's 8 cages as runtime constraint checkers.

#### 2.6.2 INTERFACE

```rust
pub trait Cage: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self, state: &AgentState) -> CageCheckResult;
}

pub enum CageCheckResult {
    Pass,
    Warn(String),
    Block(String),
}

pub struct SandboxGuard {
    root: PathBuf,
    protected: Vec<PathBuf>,
}

impl SandboxGuard {
    pub fn new(root: PathBuf, protected: Vec<PathBuf>) -> Self;
    pub fn check_write(&self, path: &Path) -> Result<(), SandboxViolation>;
    pub fn check_read(&self, path: &Path) -> Result<(), SandboxViolation>;
    pub fn check_delete(&self, path: &Path) -> Result<(), SandboxViolation>;
}
```

#### 2.6.3 BEHAVIOR

- Cages are checked in order: Spec → Knowledge → Memory → Action → Verify → Meta.
- First `Block` result short-circuits: action is prevented.
- `Warn` results are logged but do not prevent action.
- SandboxGuard canonicalizes paths before comparison (resolves symlinks).

#### 2.6.4 ERROR CONDITIONS

| Error | Condition | Response |
|-------|-----------|----------|
| `SandboxViolation::OutsideSandbox` | Write/delete path outside root | Block + log |
| `SandboxViolation::ProtectedPath` | Write to constitution/cage config | Block + log |
| `CageViolation::IncompleteSpec` | Task execution with open questions | Block |
| `CageViolation::NoVerification` | Completion claim without verification | Block |
| `CageViolation::ConstitutionModified` | Attempt to modify constitution | Block |

---

### 2.7 terra-agents

#### 2.7.1 PURPOSE

Multi-agent orchestrator: Main → Sub-agent dispatch → result audit.

#### 2.7.2 INTERFACE

```rust
pub struct Orchestrator {
    provider: Arc<dyn ProviderAdapter>,
    audit_log: Arc<AuditLog>,
}

impl Orchestrator {
    pub fn new(provider: Arc<dyn ProviderAdapter>, audit_log: Arc<AuditLog>) -> Self;
    pub async fn run(&self, task_description: &str) -> Result<RunResult>;
}

pub struct RunResult {
    pub spec: SpecIR,
    pub worker_response: String,
    pub dispatch_mode: DispatchMode,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub audit_entry_count: usize,
}
```

#### 2.7.3 PIPELINE

```
1. task_description → SpecIR::from_task_description()
2. Record trace_event("task_start")
3. Create TaskPayload from SpecIR
4. Record trace_event("worker_dispatch")
5. Call provider.complete() via dispatch()
6. Create ResultPayload from provider response
7. Record trace_checkpoint("task_completed")
8. Return RunResult
```

---

### 2.8 terra-trace

#### 2.8.1 PURPOSE

Audit trail: sequential command logging with replay capability.

#### 2.8.2 INTERFACE

```rust
pub struct AuditLog {
    // Internal: Mutex<Vec<AuditEntry>> + atomic seq counter
}

impl AuditLog {
    pub fn new() -> Self;
    pub fn record_command(&self, command: Command) -> u64;  // returns seq
    pub fn record_result(&self, seq: u64, result: CommandResult, duration_us: u64);
    pub fn entries(&self) -> Vec<AuditEntry>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn render_markdown(&self) -> String;
}
```

#### 2.8.3 BEHAVIOR

- Sequence numbers start at 1 and increment monotonically.
- `record_result` attaches to existing entry by seq number.
- Thread-safe via `Mutex`. Future: lock-free with crossbeam.
- `render_markdown` produces a complete report of all entries.

---

## 3. Test Specification

### 3.1 Test Organization

Tests are organized by crate and category. Each test has:
- **ID**: unique identifier (T-XX-NNN)
- **Precondition**: required state before execution
- **Steps**: exact operations to perform
- **Expected**: verifiable outcome
- **Priority**: MUST (blocks release) | SHOULD (important) | MAY (nice to have)

### 3.2 terra-protocol Tests (22 tests)

**T-P-001: task_assignment_fields** [MUST]
- Pre: None
- Steps: Create AgentMessage::task_assignment(Worker, payload)
- Expected: from=Main, to=Worker, kind=TaskAssignment, id is UUID, timestamp is UTC

**T-P-002: task_result_success** [MUST]
- Pre: None
- Steps: Create AgentMessage::task_result(Worker, ResultPayload{status: Success})
- Expected: from=Worker, to=Main, kind=TaskResult, status=Success

**T-P-003: task_result_failure** [MUST]
- Pre: None
- Steps: Create result with status=Failed and error message
- Expected: error field is Some(message)

**T-P-004: message_id_unique** [MUST]
- Pre: None
- Steps: Create two messages
- Expected: msg1.id != msg2.id

**T-P-005: message_timestamp_utc** [MUST]
- Pre: None
- Steps: Create message, check timestamp
- Expected: Timestamp within 1 second of Utc::now()

**T-P-006: agent_role_serialization** [MUST]
- Pre: None
- Steps: Serialize/deserialize each AgentRole variant
- Expected: All 4 variants round-trip correctly

**T-P-007: msgpack_roundtrip_task_assignment** [MUST]
- Pre: None
- Steps: encode(task_msg) → bytes → decode(bytes)
- Expected: All fields preserved

**T-P-008: msgpack_roundtrip_task_result** [MUST]
- Pre: None
- Steps: encode(result_msg) → bytes → decode(bytes)
- Expected: All fields preserved

**T-P-009: msgpack_roundtrip_command** [MUST]
- Pre: None
- Steps: encode(command) → bytes → decode(bytes)
- Expected: All fields preserved

**T-P-010: msgpack_smaller_than_json** [MUST]
- Pre: None
- Steps: Serialize same message as MessagePack and JSON
- Expected: msgpack_bytes.len() < json_bytes.len()

**T-P-011: msgpack_optional_none** [MUST]
- Pre: None
- Steps: Encode message with all Optional fields set to None
- Expected: Decode succeeds, Optional fields are None

**T-P-012: decode_invalid_bytes** [MUST]
- Pre: None
- Steps: decode::<AgentMessage>(&[0xFF, 0xFE])
- Expected: Returns Err (not panic)

**T-P-013: command_mem_write** [MUST]
- Pre: None
- Steps: Create Command with MemWrite op
- Expected: All MemWrite fields accessible

**T-P-014: command_mem_read** [MUST]
- Pre: None
- Steps: Create Command with MemRead op
- Expected: key field accessible

**T-P-015: command_mem_search** [MUST]
- Pre: None
- Steps: Create Command with MemSearch op
- Expected: query and top_k fields accessible

**T-P-016: command_artifact_save** [MUST]
- Pre: None
- Steps: Create Command with ArtifactSave op
- Expected: type and content fields accessible

**T-P-017: command_trace_checkpoint** [MUST]
- Pre: None
- Steps: Create Command with TraceCheckpoint op
- Expected: title and summary fields accessible

**T-P-018: command_result_ok** [MUST]
- Pre: None
- Steps: CommandResult::ok("id", Some(data))
- Expected: status=Ok, data=Some, error=None

**T-P-019: command_result_error** [MUST]
- Pre: None
- Steps: CommandResult::err("id", "message")
- Expected: status=Error, data=None, error=Some

**T-P-020: render_message_contains_op** [MUST]
- Pre: Task assignment message with op="fix_bug"
- Steps: render_message(&msg)
- Expected: Output contains "fix_bug"

**T-P-021: render_command_contains_agent** [MUST]
- Pre: Command from agent "main"
- Steps: render_command(&cmd)
- Expected: Output contains "main"

**T-P-022: render_message_contains_status** [MUST]
- Pre: Result message with status=Success
- Steps: render_message(&msg)
- Expected: Output contains "Success"

### 3.3 terra-compile Tests (15 tests)

**T-C-001: from_description_sets_intent** [MUST]
- Steps: SpecIR::from_task_description("fix the login bug")
- Expected: spec.intent == "fix the login bug"

**T-C-002: from_description_is_incomplete** [MUST]
- Steps: SpecIR::from_task_description("fix bug")
- Expected: spec.is_complete() == false

**T-C-003: manual_spec_is_complete** [MUST]
- Steps: Build SpecIR with all fields, empty open_questions
- Expected: spec.is_complete() == true

**T-C-004: json_roundtrip** [MUST]
- Steps: serde_json::to_string → serde_json::from_str
- Expected: All fields preserved

**T-C-005: msgpack_roundtrip** [MUST]
- Steps: encode → decode
- Expected: All fields preserved

**T-C-006: target_file_and_line** [MUST]
- Steps: Create Target with file="src/auth.py", line=42
- Expected: Fields accessible

**T-C-007: target_component** [SHOULD]
- Steps: Create Target with component="LoginButton"
- Expected: component is Some("LoginButton")

**T-C-008: multiple_constraints** [MUST]
- Steps: Build SpecIR with 3 constraints
- Expected: constraints.len() == 3

**T-C-009: success_predicate** [MUST]
- Steps: Build SpecIR with 2 success predicates
- Expected: success_predicate.len() == 2

**T-C-010: term_mapping** [SHOULD]
- Steps: Build TermMapping { user: "login bug", normalized: "empty_password" }
- Expected: Both fields preserved after serialization

**T-C-011: empty_description** [MUST]
- Steps: SpecIR::from_task_description("")
- Expected: No panic, intent == ""

**T-C-012: unicode_description** [MUST]
- Steps: SpecIR::from_task_description("修复登录问题 🐛")
- Expected: No panic, intent preserved

**T-C-013: long_description** [SHOULD]
- Steps: SpecIR::from_task_description(&"x".repeat(10_000))
- Expected: No panic, intent length == 10_000

**T-C-014: spec_id_format** [MUST]
- Steps: Create SpecIR, parse spec_id as UUID
- Expected: Valid UUID v4

**T-C-015: multiple_open_questions** [SHOULD]
- Steps: Build SpecIR with 3 open questions
- Expected: open_questions.len() == 3, is_complete() == false

### 3.4 terra-providers Tests (18 tests)

**T-PR-001: dry_run_deterministic** [MUST]
- Steps: Two calls with same input
- Expected: Both responses contain "DRY_RUN" and input substring

**T-PR-002: dry_run_zero_tokens** [MUST]
- Steps: DryRunAdapter.complete(request)
- Expected: usage_prompt_tokens == 0, usage_completion_tokens == 0

**T-PR-003: dry_run_capabilities** [MUST]
- Steps: DryRunAdapter.capabilities()
- Expected: provider_id="dry_run", structured_output=true

**T-PR-004: dry_run_no_network** [MUST]
- Steps: Complete with network disabled (dry_run doesn't need network)
- Expected: Succeeds

**T-PR-005: dry_run_model_field** [MUST]
- Steps: DryRunAdapter.complete(request)
- Expected: response.model == "dry_run"

**T-PR-006: openai_adapter_creation** [MUST]
- Pre: TERRA_PROVIDER_API_KEY set
- Steps: Create OpenAICompatibleAdapter
- Expected: No error

**T-PR-007: openai_adapter_capabilities** [MUST]
- Steps: adapter.capabilities()
- Expected: All capability fields populated

**T-PR-008: openai_missing_key_error** [MUST]
- Pre: TERRA_PROVIDER_API_KEY not set
- Steps: Attempt live dispatch
- Expected: ProviderError::MissingApiKey

**T-PR-009: openai_completion_live** [MAY]
- Pre: TERRA_PROVIDER_API_KEY set, network available
- Steps: Complete a simple prompt
- Expected: Non-empty response, positive token counts
- Note: Skipped in CI if no key

**T-PR-010: openai_retry_on_429** [SHOULD]
- Pre: Mock server returning 429 then 200
- Steps: dispatch with Live mode
- Expected: Succeeds after retry

**T-PR-011: openai_timeout** [SHOULD]
- Pre: Mock server with 61s delay
- Steps: dispatch with Live mode
- Expected: ProviderError::Timeout

**T-PR-012: openai_streaming** [MAY]
- Pre: Provider supports streaming
- Steps: Stream completion
- Expected: Tokens arrive incrementally (future test)

**T-PR-013: dispatch_dry_run** [MUST]
- Steps: dispatch(adapter, request, DryRun)
- Expected: result.mode == DryRun

**T-PR-014: dispatch_live** [MUST]
- Steps: dispatch(adapter, request, Live)
- Expected: result.mode == Live

**T-PR-015: dispatch_result_has_response** [MUST]
- Steps: dispatch → result
- Expected: result.response.content is non-empty

**T-PR-016: dispatch_result_has_usage** [MUST]
- Steps: dispatch → result
- Expected: usage fields are present (even if 0 for dry_run)

**T-PR-017: dispatch_propagates_error** [MUST]
- Pre: Adapter that returns Err
- Steps: dispatch
- Expected: Err propagated

**T-PR-018: dispatch_mode_in_result** [MUST]
- Steps: dispatch with DryRun mode
- Expected: result.mode matches input mode

### 3.5 terra-memory Tests (25 tests)

**T-M-001: create_database** [MUST]
- Steps: MemoryDatabase::new(path)
- Expected: SQLite file created, tables exist

**T-M-002: write_and_read** [MUST]
- Steps: write(entry), read(key)
- Expected: Entry returned with matching content

**T-M-003: write_duplicate_key** [MUST]
- Steps: write(key="a"), write(key="a")
- Expected: Second write returns MemoryError::DuplicateKey

**T-M-004: read_missing_key** [MUST]
- Steps: read("nonexistent")
- Expected: Returns Ok(None)

**T-M-005: update_version** [MUST]
- Steps: write(key), update(key), read(key)
- Expected: version incremented

**T-M-006: delete_soft** [MUST]
- Steps: write(key), delete(key), read(key)
- Expected: Entry has alive=false

**T-M-007: content_is_msgpack** [MUST]
- Steps: write(entry with msgpack content), read, decode
- Expected: Content deserializes from MessagePack

**T-M-008: scope_constraint** [MUST]
- Steps: write(scope="invalid")
- Expected: Error (SQL CHECK violation)

**T-M-009: level_constraint** [MUST]
- Steps: write(level="invalid")
- Expected: Error (SQL CHECK violation)

**T-M-010: owner_set** [MUST]
- Steps: write(entry with owner="main")
- Expected: read().owner == "main"

**T-M-011: non_owner_cannot_update** [MUST]
- Steps: write(owner="main"), update(agent="worker")
- Expected: MemoryError::PermissionDenied

**T-M-012: non_owner_can_read** [MUST]
- Steps: write(owner="main"), read(by any agent)
- Expected: Ok(Some(entry))

**T-M-013: ownership_transfer** [SHOULD]
- Steps: write(owner="main"), transfer(new_owner="worker")
- Expected: owner changed

**T-M-014: pointer_alive_check** [MUST]
- Steps: write(key), read(key)
- Expected: entry.alive == true

**T-M-015: dangling_pointer** [MUST]
- Steps: write(key), delete(key), attempt deref
- Expected: MemoryError::Dangling

**T-M-016: version_mismatch** [MUST]
- Steps: write(key, v1), update(key, v2), read(expected_version=1)
- Expected: MemoryError::Stale

**T-M-017: gc_removes_weak** [SHOULD]
- Steps: write(level=L5, ref_count=0), gc()
- Expected: Entry removed

**T-M-018: gc_preserves_authoritative** [MUST]
- Steps: write(level=L0, ref_count=0), gc()
- Expected: Entry preserved

**T-M-019: cache_hit** [SHOULD]
- Steps: write(key), read(key) twice
- Expected: Second read is faster (cache hit)

**T-M-020: cache_miss_to_sqlite** [SHOULD]
- Steps: Evict from cache, read(key)
- Expected: Entry loaded from SQLite

**T-M-021: write_updates_both** [MUST]
- Steps: write(key), verify in cache and SQLite
- Expected: Present in both

**T-M-022: cache_eviction** [SHOULD]
- Steps: Fill cache beyond capacity, check oldest entry
- Expected: Oldest entry evicted from cache (still in SQLite)

**T-M-023: promote_l2_to_l1** [MUST]
- Steps: write(level=L2), promote(target=L1)
- Expected: level changed to L1

**T-M-024: promote_requires_verification** [SHOULD]
- Steps: promote to L3 without verification flag
- Expected: MemoryError::PromotionBlocked

**T-M-025: promote_l6_requires_review** [MUST]
- Steps: promote(level=L6, reviewed=false)
- Expected: MemoryError::PromotionBlocked

### 3.6 terra-core Tests (12 tests)

**T-IC-001: load_universal** [MUST]
- Steps: IntelligenceCore::new()
- Expected: universal framework has rules

**T-IC-002: register_module** [MUST]
- Steps: core.register(coding_module)
- Expected: module listed in core.modules

**T-IC-003: all_perspectives** [MUST]
- Steps: Register 2 modules, core.all_perspectives()
- Expected: Perspectives from both modules

**T-IC-004: all_rules** [MUST]
- Steps: Register module, core.all_rules()
- Expected: Universal rules + module rules

**T-IC-005: coding_perspectives** [MUST]
- Steps: coding_module.perspectives()
- Expected: At least 1 perspective

**T-IC-006: coding_rules** [MUST]
- Steps: coding_module.rules()
- Expected: At least 1 rule

**T-IC-007: algorithm_perspectives** [SHOULD]
- Steps: algorithm_module.perspectives()
- Expected: At least 1 perspective

**T-IC-008: modules_composable** [MUST]
- Steps: Register coding + algorithm, query perspectives
- Expected: Both modules' perspectives present

**T-IC-009: universal_readonly** [MUST]
- Steps: Attempt to modify universal rules at runtime
- Expected: Compilation error or runtime error

**T-IC-010: detect_goal_drift** [SHOULD]
- Steps: Simulate goal drift scenario
- Expected: Diagnostic signal produced

**T-IC-011: detect_shallow_reasoning** [SHOULD]
- Steps: Simulate shallow reasoning scenario
- Expected: Diagnostic signal produced

**T-IC-012: detect_shortcut** [SHOULD]
- Steps: Simulate shortcut scenario
- Expected: Diagnostic signal produced

### 3.7 terra-caging Tests (18 tests)

**T-CG-001: cage_pass** [MUST]
- Expected: CageCheckResult::Pass allows action

**T-CG-002: cage_warn** [MUST]
- Expected: CageCheckResult::Warn logs but allows

**T-CG-003: cage_block** [MUST]
- Expected: CageCheckResult::Block prevents action

**T-CG-004: spec_cage_pass** [MUST]
- Pre: Complete SpecIR
- Expected: Pass

**T-CG-005: spec_cage_block** [MUST]
- Pre: Incomplete SpecIR
- Expected: Block

**T-CG-006: memory_cage_owner_write** [MUST]
- Expected: Owner write allowed

**T-CG-007: memory_cage_non_owner_block** [MUST]
- Expected: Non-owner write blocked

**T-CG-008: memory_cage_any_read** [MUST]
- Expected: Any agent can read

**T-CG-009: sandbox_write_inside** [MUST]
- Pre: Path inside project root
- Expected: check_write → Ok

**T-CG-010: sandbox_write_outside** [MUST]
- Pre: Path outside project root
- Expected: check_write → SandboxViolation::OutsideSandbox

**T-CG-011: sandbox_read_outside** [MUST]
- Pre: Path outside project root
- Expected: check_read → Ok

**T-CG-012: sandbox_write_protected** [MUST]
- Pre: Path to .terra/constitution.toml
- Expected: check_write → SandboxViolation::ProtectedPath

**T-CG-013: sandbox_read_protected** [MUST]
- Pre: Path to .terra/constitution.toml
- Expected: check_read → Ok

**T-CG-014: sandbox_delete_outside** [MUST]
- Pre: Path outside project root
- Expected: check_delete → SandboxViolation::OutsideSandbox

**T-CG-015: verify_cage_no_verification** [MUST]
- Pre: No external verifier passed
- Expected: Block

**T-CG-016: verify_cage_after_verification** [MUST]
- Pre: External verifier passed
- Expected: Pass

**T-CG-017: meta_cage_constitution** [MUST]
- Pre: Attempt to modify constitution
- Expected: Block + logged

**T-CG-018: meta_cage_rule_change** [MUST]
- Pre: Attempt to modify cage rule
- Expected: Block + logged

### 3.8 terra-agents Tests (12 tests)

**T-A-001: end_to_end_dry_run** [MUST]
- Steps: orchestrator.run("fix auth bug")
- Expected: RunResult with all fields populated

**T-A-002: spec_ir_from_task** [MUST]
- Expected: result.spec.intent == task description

**T-A-003: provider_called** [MUST]
- Expected: result.worker_response contains DRY_RUN output

**T-A-004: audit_recorded** [MUST]
- Expected: audit_log.len() >= 3

**T-A-005: result_has_response** [MUST]
- Expected: result.worker_response is non-empty

**T-A-006: result_has_tokens** [MUST]
- Expected: prompt_tokens and completion_tokens present

**T-A-007: result_has_spec_id** [MUST]
- Expected: result.spec.spec_id is valid UUID

**T-A-008: empty_task** [MUST]
- Steps: run("")
- Expected: Completes without panic

**T-A-009: long_task** [SHOULD]
- Steps: run(&"x".repeat(10_000))
- Expected: Completes without panic

**T-A-010: provider_error** [MUST]
- Pre: Adapter that always returns Err
- Expected: run() returns Err

**T-A-011: sequential_runs** [MUST]
- Steps: run() twice
- Expected: Both complete, independent audit trails

**T-A-012: concurrent_runs** [MAY]
- Steps: Two concurrent run() calls
- Expected: Both complete without data corruption

### 3.9 terra-trace Tests (10 tests)

**T-TR-001: record_command** [MUST]
- Expected: seq=1, entry in log

**T-TR-002: record_result** [MUST]
- Expected: result attached to entry

**T-TR-003: sequential_numbering** [MUST]
- Expected: seq 1,2,3...N

**T-TR-004: entries_ordered** [MUST]
- Expected: entries() returns in seq order

**T-TR-005: render_markdown** [MUST]
- Expected: Valid markdown with all fields

**T-TR-006: empty_log_renders** [MUST]
- Expected: Empty log produces valid header-only markdown

**T-TR-007: unknown_seq_ignored** [MUST]
- Steps: record_result(seq=999)
- Expected: No panic, no effect

**T-TR-008: concurrent_writes** [MUST]
- Steps: 10 threads, each recording 100 commands
- Expected: 1000 entries, no gaps, no duplicates

**T-TR-009: large_log** [SHOULD]
- Steps: Record 10,000 entries
- Expected: len()==10000, no performance degradation

**T-TR-010: msgpack_roundtrip** [MUST]
- Steps: Serialize entries to msgpack, deserialize
- Expected: All entries preserved

### 3.10 terra-cli Tests (8 tests)

**T-CLI-001: no_args_banner** [MUST]
- Steps: Run `terra`
- Expected: Version and "Agent OS" in output

**T-CLI-002: doctor_status** [MUST]
- Steps: Run `terra doctor`
- Expected: "OK" in output

**T-CLI-003: doctor_json** [MUST]
- Steps: Run `terra doctor --as-json`
- Expected: Valid JSON output

**T-CLI-004: run_task** [MUST]
- Steps: Run `terra run --task "test"`
- Expected: Exit code 0, output contains "complete"

**T-CLI-005: run_json** [MUST]
- Steps: Run `terra run --task "test" --as-json`
- Expected: Valid JSON with spec_id, intent, response fields

**T-CLI-006: run_json_fields** [MUST]
- Expected: JSON has required keys: spec_id, intent, worker_response

**T-CLI-007: help_flag** [MUST]
- Steps: Run `terra --help`
- Expected: Lists all subcommands

**T-CLI-008: unknown_command** [MUST]
- Steps: Run `terra nonexistent`
- Expected: Non-zero exit code, error message

### 3.11 Config Tests (6 tests)

**T-CF-001: default_config** [MUST]
- Steps: Run without config file
- Expected: Default settings applied

**T-CF-002: project_overrides_global** [MUST]
- Pre: Global config has key=A, project config has key=B
- Expected: key resolves to B

**T-CF-003: env_overrides_config** [MUST]
- Pre: Config has api_key=X, env has TERRA_PROVIDER_API_KEY=Y
- Expected: api_key resolves to Y

**T-CF-004: config_show** [SHOULD]
- Steps: `terra config show`
- Expected: All settings listed

**T-CF-005: config_set** [SHOULD]
- Steps: `terra config set provider.model gpt-4o`
- Expected: Setting persisted to file

**T-CF-006: invalid_config** [MUST]
- Pre: Malformed config.toml
- Expected: Warning message, fallback to defaults, no crash

### 3.12 Integration Tests (6 tests)

**T-INT-001: run_with_memory** [MUST]
- Steps: Run task → result stored in memory → read back
- Expected: Result retrievable

**T-INT-002: run_with_sandbox** [MUST]
- Steps: Run task that attempts file write inside/outside sandbox
- Expected: Inside succeeds, outside blocked

**T-INT-003: run_with_caging** [SHOULD]
- Steps: Run task → all 8 cages checked
- Expected: Pass for valid task

**T-INT-004: audit_replay** [SHOULD]
- Steps: Run task → get audit log → replay
- Expected: Same final state

**T-INT-005: live_provider** [MAY]
- Pre: TERRA_PROVIDER_API_KEY set
- Steps: Run with live provider
- Expected: Real LLM response

**T-INT-006: project_isolation** [MUST]
- Steps: Write to project A memory, read from project B
- Expected: Not found in project B

---

**Total: 152 test cases.**

| Category | Count | MUST | SHOULD | MAY |
|----------|-------|------|--------|-----|
| terra-protocol | 22 | 22 | 0 | 0 |
| terra-compile | 15 | 11 | 4 | 0 |
| terra-providers | 18 | 13 | 3 | 2 |
| terra-memory | 25 | 18 | 7 | 0 |
| terra-core | 12 | 7 | 5 | 0 |
| terra-caging | 18 | 16 | 0 | 2 |
| terra-agents | 12 | 10 | 1 | 1 |
| terra-trace | 10 | 8 | 1 | 1 |
| terra-cli | 8 | 8 | 0 | 0 |
| config | 6 | 4 | 2 | 0 |
| integration | 6 | 3 | 2 | 1 |
| **Total** | **152** | **120** | **25** | **7** |

---

## 4. Data Schemas

See Section 2.4.4 for SQLite schema.
See Section 2.1.2 for MessagePack message schema.
See Section 2.2.2 for Spec IR schema.

## 5. Error Taxonomy

All errors derive from `thiserror`. Domain errors are specific
and matchable. Unexpected errors use `anyhow`.

```rust
// terra-memory
#[derive(thiserror::Error, Debug)]
pub enum MemoryError {
    #[error("key not found: {0}")]
    NotFound(String),
    #[error("duplicate key: {0}")]
    DuplicateKey(String),
    #[error("permission denied: {agent} is not owner of {key}")]
    PermissionDenied { agent: String, key: String },
    #[error("dangling pointer: {0} is deleted")]
    Dangling(String),
    #[error("stale version: expected {expected}, found {found}")]
    Stale { expected: u64, found: u64 },
    #[error("promotion blocked: {reason}")]
    PromotionBlocked { reason: String },
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

// terra-providers
#[derive(thiserror::Error, Debug)]
pub enum ProviderError {
    #[error("TERRA_PROVIDER_API_KEY not set")]
    MissingApiKey,
    #[error("network error: {0}")]
    Network(String),
    #[error("rate limited after retries")]
    RateLimit,
    #[error("request timeout")]
    Timeout,
    #[error("invalid response: {0}")]
    InvalidResponse(String),
}

// terra-caging
#[derive(thiserror::Error, Debug)]
pub enum SandboxViolation {
    #[error("write outside sandbox: {0}")]
    OutsideSandbox(PathBuf),
    #[error("write to protected path: {0}")]
    ProtectedPath(PathBuf),
}
```

## 6. Performance Contracts

| Operation | Target | Measurement |
|-----------|--------|-------------|
| moka cache read | < 1ms p99 | `std::time::Instant` in test |
| SQLite read | < 5ms p99 | `std::time::Instant` in test |
| MessagePack encode (1KB payload) | < 100μs p99 | Benchmark |
| Vector search (1K entries, 384-dim) | < 10ms p99 | Benchmark |
| CLI startup (no embedding) | < 500ms | Wall clock |
| Embedding model load | < 3s | Wall clock |
| Audit log write | < 50μs p99 | `std::time::Instant` in test |

## 7. Migration and Versioning

- SQLite schema version tracked via `PRAGMA user_version`.
- Schema changes: forward-only migrations, auto-applied on startup.
- Embedding model change: full re-index from `memories.content` column.
- Config format change: auto-migrate with `.bak` backup of old file.
- Skill manifest version: `manifest_version` field, validated on load.
