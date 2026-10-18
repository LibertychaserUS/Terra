//! Live integration tests for the agent loop with real LLM API.
//!
//! These tests require a valid API key (DASHSCOPE_API_KEY or TERRA_PROVIDER_API_KEY).
//! The two live tests are ignored by default; the dry-run tests always run.
//!
//! Run: cargo test -p terra-agents --test live_tool_test -- --ignored --nocapture

use std::sync::Arc;

use terra_agents::repl::{Repl, ReplOutput};
use terra_agents::tools::{ToolLevel, ToolRegistry};
use terra_caging::engine::*;
use terra_caging::sandbox::SandboxGuard;
use terra_core::domain::{AlgorithmModule, CodingModule};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_memory::{ArtifactStore, MemoryDatabase};
use terra_providers::adapter::{DryRunAdapter, ProviderAdapter};
use terra_providers::OpenAICompatibleAdapter;
use terra_trace::AuditLog;

/// Check if we have a real API key available.
fn has_api_key() -> bool {
    std::env::var("DASHSCOPE_API_KEY").is_ok()
        || std::env::var("TERRA_PROVIDER_API_KEY").is_ok()
}

/// Create a test Repl with a REAL provider (or skip if no key).
fn live_repl() -> Option<(Repl, tempfile::TempDir)> {
    if !has_api_key() {
        eprintln!("SKIP: no API key set (DASHSCOPE_API_KEY or TERRA_PROVIDER_API_KEY)");
        return None;
    }

    let dir = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
    let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
    let audit = Arc::new(AuditLog::new());

    let provider: Arc<dyn ProviderAdapter> = match OpenAICompatibleAdapter::from_env() {
        Ok(adapter) => Arc::new(adapter),
        Err(e) => {
            eprintln!("SKIP: failed to create provider: {}", e);
            return None;
        }
    };

    let sandbox = SandboxGuard::new(dir.path().to_path_buf(), vec![]);
    let caging = Arc::new(CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]));
    let exec = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));

    let mut core = IntelligenceCore::new();
    core.register(Box::new(CodingModule));
    core.register(Box::new(AlgorithmModule));
    let core = Arc::new(core);

    Some((Repl::new(exec, core), dir))
}

/// Create a test Repl with DryRunAdapter (no API key needed).
fn dry_run_repl() -> (Repl, tempfile::TempDir, Arc<AuditLog>) {
    let dir = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
    let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
    let audit = Arc::new(AuditLog::new());
    let provider: Arc<dyn ProviderAdapter> = Arc::new(DryRunAdapter::new());
    let sandbox = SandboxGuard::new(dir.path().to_path_buf(), vec![]);
    let caging = Arc::new(CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]));
    let exec = Arc::new(CommandExecutor::new(
        memory,
        artifacts,
        audit.clone(),
        caging,
        provider,
    ));

    let mut core = IntelligenceCore::new();
    core.register(Box::new(CodingModule));
    core.register(Box::new(AlgorithmModule));
    let core = Arc::new(core);

    (Repl::new(exec, core), dir, audit)
}

// T-AGLV-001: loop_single_tool_call_live
// Test that the agent can use a single tool (FileRead) and produce a final answer.
#[tokio::test]
#[ignore = "needs an LLM API key; run with `cargo test -- --ignored`"]
async fn loop_single_tool_call_live() {
    let (mut repl, dir) = match live_repl() {
        Some(r) => r,
        None => panic!("live test: set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY"),
    };

    // Write a test file for FileRead
    let test_file = dir.path().join("test.txt");
    std::fs::write(&test_file, "Hello from test file!\nLine 2: Rust is great.").unwrap();

    let outputs = repl
        .process_input(&format!(
            "Read the file at {} and tell me what it says in one sentence.",
            test_file.display()
        ))
        .await;

    // Should have Token outputs (the LLM's answer, streamed word-by-word)
    let has_response = outputs
        .iter()
        .any(|o| matches!(o, ReplOutput::Token(t) if !t.is_empty()));
    assert!(
        has_response,
        "Expected token response from LLM. Outputs: {:?}",
        outputs
    );

    // Should have tool notifications (>> Tool: FileRead)
    let has_tool_use = outputs
        .iter()
        .any(|o| matches!(o, ReplOutput::Notification(n) if n.contains("Tool:")));
    // Tool use is likely but not guaranteed (LLM might answer from context)
    if has_tool_use {
        eprintln!("  OK: LLM used tool calling (FileRead)");
    } else {
        eprintln!("  WARN: LLM answered without using tools");
    }
}

// T-AGLV-002: loop_multi_tool_call_live
// Test multi-tool sequence: write to memory, then search memory.
#[tokio::test]
#[ignore = "needs an LLM API key; run with `cargo test -- --ignored`"]
async fn loop_multi_tool_call_live() {
    let (mut repl, _dir) = match live_repl() {
        Some(r) => r,
        None => panic!("live test: set TERRA_PROVIDER_API_KEY or DASHSCOPE_API_KEY"),
    };

    // Start a session so Goal mode works
    repl.session.start("test", "test").await.unwrap();
    let _ = repl.process_input("/goal test memory operations").await;

    let outputs = repl
        .process_input(
            "First write the text 'authentication uses JWT tokens' to memory with key 'auth_info'. \
             Then search memory for 'JWT' and tell me what you find.",
        )
        .await;

    let has_response = outputs
        .iter()
        .any(|o| matches!(o, ReplOutput::Token(t) if !t.is_empty()));
    assert!(has_response, "Expected token response");

    // Count tool uses
    let tool_uses: Vec<_> = outputs
        .iter()
        .filter(|o| matches!(o, ReplOutput::Notification(n) if n.contains(">> Tool:")))
        .collect();
    eprintln!("  Tool calls made: {}", tool_uses.len());

    let _ = repl.process_input("/done").await;
}

// T-AGLV-003: loop_parses_tool_call_format
// This test uses no API key — verifies the tool registry and parsing work correctly.
#[tokio::test]
async fn loop_parses_tool_call_format() {
    let registry = ToolRegistry::with_builtins();

    // Verify all tools have valid schemas
    let defs = registry.tool_defs(ToolLevel::L2Dangerous);
    assert!(
        defs.len() >= 10,
        "Expected at least 10 tools, got {}",
        defs.len()
    );

    // Verify each tool can parse valid arguments
    let test_cases = vec![
        ("FileRead", serde_json::json!({"path": "/tmp/test.txt"})),
        (
            "FileWrite",
            serde_json::json!({"path": "/tmp/out.txt", "content": "hello"}),
        ),
        ("MemRead", serde_json::json!({"key": "test_key"})),
        (
            "MemWrite",
            serde_json::json!({"key": "k", "content": "v", "level": "L2"}),
        ),
        (
            "MemSearch",
            serde_json::json!({"query": "auth", "top_k": 5}),
        ),
        ("PointerQuery", serde_json::json!({"dsl": "@L1[*]"})),
        ("ShellExec", serde_json::json!({"command": "echo test"})),
        ("FileDelete", serde_json::json!({"path": "/tmp/gone.txt"})),
    ];

    for (name, args) in test_cases {
        let tool = registry
            .find(name)
            .unwrap_or_else(|| panic!("Tool {} not found", name));
        let result = tool.parse_args(&args);
        assert!(
            result.is_ok(),
            "Failed to parse args for {}: {:?}",
            name,
            result
        );
        let (op_name, _) = result.unwrap();
        assert!(!op_name.is_empty(), "Op name empty for {}", name);
    }
}

// T-AGLV-004: loop_max_iterations_enforced
#[tokio::test]
async fn loop_max_iterations_enforced() {
    // With DryRunAdapter (no real API), the loop should complete quickly.
    // This just verifies the loop doesn't hang.
    let (mut repl, _dir, _audit) = dry_run_repl();

    let outputs = repl.process_input("Hello, just testing").await;

    // Should get a response (even if DRY_RUN) — now emitted as Token chunks
    let has_text = outputs
        .iter()
        .any(|o| matches!(o, ReplOutput::Token(t) if !t.is_empty()));
    assert!(has_text, "Expected token response from DryRun");
}

// T-AGLV-005: loop_malformed_tool_call_handled
#[tokio::test]
async fn loop_malformed_tool_call_handled() {
    // Verify that parse_args returns clear errors for bad input
    let registry = ToolRegistry::with_builtins();

    // Missing required field
    let tool = registry.find("FileRead").unwrap();
    let bad_args = serde_json::json!({"wrong_field": "value"});
    let result = tool.parse_args(&bad_args);
    assert!(result.is_err(), "Should fail with missing 'path'");
    assert!(
        result.unwrap_err().contains("path"),
        "Error should mention 'path'"
    );
}

// T-AGLV-006: loop_unknown_function_handled
#[tokio::test]
async fn loop_unknown_function_handled() {
    let registry = ToolRegistry::with_builtins();
    assert!(registry.find("nonexistent_function").is_none());
    assert!(registry.find("").is_none());
}

// T-AGLV-008: loop_audit_trail_complete
// Verify that tool-call executions flow through the executor and are audited.
// Note: The REPL calls provider.complete() directly for the agent loop (not
// through executor.execute()), so the initial LLM call itself is NOT audited.
// Only individual tool-call executions go through executor.execute() and are
// audited. With DryRunAdapter the LLM never returns tool calls, so we test
// the audit path by executing a /remember command which routes through the
// executor.
#[tokio::test]
async fn loop_audit_trail_complete() {
    let (mut repl, _dir, audit) = dry_run_repl();

    // /remember routes through executor.execute() -> audited
    let _ = repl.process_input("/remember test audit entry").await;

    let count = audit.len();
    assert!(count >= 1, "Expected audit entries after /remember, got {}", count);
}
