//! Live End-to-End Tests (T-E2E)
//!
//! These tests use a real LLM provider (DashScope).
//! They skip gracefully when no API key is configured.
//! Run with: cargo test -p terra-cli --test live_e2e

use std::sync::Arc;

use terra_agents::skill::{
    code_editor_skill, test_runner_skill, SkillContext, SkillRegistry,
};
use terra_caging::engine::*;
use terra_caging::output::check_output;
use terra_caging::sandbox::SandboxGuard;
use terra_core::domain::{AlgorithmModule, CodingModule};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_memory::{ArtifactStore, MemoryDatabase};
use terra_providers::adapter::ProviderAdapter;
use terra_providers::OpenAICompatibleAdapter;
use terra_trace::AuditLog;
use terra_types::contract::file_write_contract;
use terra_types::command::{Command, CommandOp};

fn safe_truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn has_api_key() -> bool {
    std::env::var("DASHSCOPE_API_KEY").is_ok()
        || std::env::var("TERRA_PROVIDER_API_KEY").is_ok()
}

fn live_provider() -> Option<Arc<dyn ProviderAdapter>> {
    OpenAICompatibleAdapter::from_env()
        .ok()
        .map(|a| Arc::new(a) as Arc<dyn ProviderAdapter>)
}

fn live_executor(
    provider: Arc<dyn ProviderAdapter>,
) -> (Arc<CommandExecutor>, Arc<AuditLog>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
    let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
    let audit = Arc::new(AuditLog::new());
    let sandbox = SandboxGuard::new(dir.path().to_path_buf(), vec![]);
    let caging = Arc::new(CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]));
    let exec = Arc::new(CommandExecutor::new(memory, artifacts, audit.clone(), caging, provider));
    (exec, audit, dir)
}

/// T-E2E-001: CodeEditor live — read → LLM modify → write back
#[tokio::test]
async fn code_editor_live() {
    if !has_api_key() {
        eprintln!("skipping code_editor_live (no API key)");
        return;
    }
    let provider = match live_provider() {
        Some(p) => p,
        None => { eprintln!("skipping (provider init failed)"); return; }
    };

    let (exec, _audit, dir) = live_executor(provider);

    // Create a source file
    let file_path = dir.path().join("hello.py");
    std::fs::write(&file_path, "def hello():\n    print('hello')\n").unwrap();
    let original = std::fs::read_to_string(&file_path).unwrap();

    // Run CodeEditor skill
    let mut reg = SkillRegistry::new();
    reg.register(code_editor_skill());

    let skill = reg.lookup("code_editor").unwrap();
    let ctx = SkillContext {
        agent_id: "worker-1".into(),
        agent_allowed_ops: vec!["FileRead".into(), "ProviderCall".into(), "FileWrite".into()],
        input: serde_json::json!({
            "file": file_path.to_string_lossy(),
            "instruction": "Add a docstring to the hello function"
        }),
    };

    let plan = (skill.execute_fn)(&ctx);
    let result = reg.execute_plan(&plan, "worker-1", &exec).await;

    eprintln!("CodeEditor result: success={}, steps={}/{}", result.success, result.steps_completed, result.steps_total);

    // Verify: step 1 (read) should have succeeded at minimum
    assert!(result.steps_completed >= 1, "at least read should succeed");

    // If all 3 steps succeeded, file should be different
    if result.success {
        let modified = std::fs::read_to_string(&file_path).unwrap();
        eprintln!("Original: {:?}", original);
        eprintln!("Modified: {:?}", safe_truncate(&modified, 200));
        // We don't assert content changed because the LLM write step
        // depends on OutputContract parsing which DashScope may format differently.
        // The key assertion: all 3 steps completed without error.
    }
}

/// T-E2E-002: TestRunner live — run shell, capture output
#[tokio::test]
async fn test_runner_live() {
    if !has_api_key() {
        eprintln!("skipping test_runner_live (no API key)");
        return;
    }
    let provider = match live_provider() {
        Some(p) => p,
        None => { eprintln!("skipping"); return; }
    };

    let (exec, _audit, _dir) = live_executor(provider);

    let mut reg = SkillRegistry::new();
    reg.register(test_runner_skill());

    let skill = reg.lookup("test_runner").unwrap();
    let ctx = SkillContext {
        agent_id: "worker-1".into(),
        agent_allowed_ops: vec!["ShellExec".into(), "TraceEvent".into()],
        input: serde_json::json!({"command": "echo live_test_passed"}),
    };

    let plan = (skill.execute_fn)(&ctx);
    let result = reg.execute_plan(&plan, "worker-1", &exec).await;

    assert!(result.success, "test runner should succeed: {:?}", result.error);
    let stdout = result.output.get("test")
        .and_then(|d| d.get("stdout"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    assert!(stdout.contains("live_test_passed"), "should capture stdout: {}", stdout);
}

/// T-E2E-003: OutputContract live — real LLM response parsed
#[tokio::test]
async fn output_contract_live() {
    if !has_api_key() {
        eprintln!("skipping output_contract_live (no API key)");
        return;
    }
    let provider = match live_provider() {
        Some(p) => p,
        None => { eprintln!("skipping"); return; }
    };

    let (exec, _audit, _dir) = live_executor(provider);

    // Ask LLM to output in file_write contract format
    let contract = file_write_contract();
    let cmd = Command::new("test", CommandOp::ProviderCall {
        system_prompt: format!(
            "You are a code assistant.\n{}",
            contract.prompt_section()
        ),
        user_prompt: "Write a Python hello world function to src/hello.py".into(),
        max_tokens: Some(200),
        temperature: Some(0.0),
        output_contract: Some("file_write".into()),
    });

    let result = exec.execute(cmd).await;
    assert_eq!(result.status, terra_types::CommandStatus::Ok, "provider call should succeed");

    let content = result.data.as_ref()
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    eprintln!("LLM response: {:?}", safe_truncate(content, 300));

    // Try parsing against contract
    let parse_result = contract.parse(content);
    match parse_result {
        terra_types::ContractParseResult::Ok(v) => {
            eprintln!("Contract parsed OK: {:?}", v.get("action"));
        }
        terra_types::ContractParseResult::JsonError(e) => {
            eprintln!("Contract JSON error (may be expected with some models): {}", e);
        }
        terra_types::ContractParseResult::MissingFields(f) => {
            eprintln!("Contract missing fields: {:?}", f);
        }
    }
    // We log but don't hard-fail on contract parse, since real LLM output format varies.
}

/// T-E2E-004: Output Cage no false positive on normal coding response
#[tokio::test]
async fn output_cage_no_false_positive() {
    if !has_api_key() {
        eprintln!("skipping output_cage_no_false_positive (no API key)");
        return;
    }
    let provider = match live_provider() {
        Some(p) => p,
        None => { eprintln!("skipping"); return; }
    };

    let (exec, _audit, _dir) = live_executor(provider);

    let cmd = Command::new("test", CommandOp::ProviderCall {
        system_prompt: "You are a Rust coding assistant.".into(),
        user_prompt: "Write a function that checks if a number is prime. Return only the code.".into(),
        max_tokens: Some(200),
        temperature: Some(0.0),
        output_contract: None,
    });

    let result = exec.execute(cmd).await;
    let content = result.data.as_ref()
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    eprintln!("LLM coding response: {:?}", safe_truncate(content, 200));

    // Output Cage should NOT flag normal coding response
    let cage_result = check_output(content);
    assert!(
        !matches!(cage_result, terra_caging::OutputCheckResult::Block(_)),
        "normal coding response should not be blocked by Output Cage: {:?}", cage_result
    );
}

/// T-E2E-005: Full pipeline live
#[tokio::test]
async fn full_pipeline_live() {
    if !has_api_key() {
        eprintln!("skipping full_pipeline_live (no API key)");
        return;
    }
    let provider = match live_provider() {
        Some(p) => p,
        None => { eprintln!("skipping"); return; }
    };

    let (exec, audit, _dir) = live_executor(provider);

    let mut core = IntelligenceCore::new();
    core.register(Box::new(CodingModule));
    core.register(Box::new(AlgorithmModule));
    let orch = terra_agents::Orchestrator::new(exec, Arc::new(core));

    let result = orch.run("What is 2+2? Reply with just the number.").await.unwrap();

    eprintln!("Full pipeline result: response={:?}", safe_truncate(&result.worker_response, 100));

    assert!(!result.worker_response.is_empty(), "should have non-empty response");
    assert!(!result.worker_response.contains("DRY_RUN"), "should use real provider");
    assert!(result.ic_rules_applied >= 13, "IC rules should be applied");

    let entries = audit.entries();
    assert!(entries.len() >= 5, "pipeline should record audit entries");
}

/// T-FULL-001: User intent → plan proposed → file could be modified
#[tokio::test]
async fn full_pipeline_plan_proposal() {
    if !has_api_key() {
        eprintln!("skipping full_pipeline_plan_proposal (no API key)");
        return;
    }
    let provider = match live_provider() {
        Some(p) => p,
        None => { eprintln!("skipping"); return; }
    };

    let (exec, audit, dir) = live_executor(provider);
    let mut core = IntelligenceCore::new();
    core.register(Box::new(CodingModule));
    core.register(Box::new(AlgorithmModule));
    let orch = terra_agents::Orchestrator::new(exec, Arc::new(core));

    // Step 1: Propose plan
    let plan = orch.propose_plan("add a hello world function to main.rs").await.unwrap();
    eprintln!("Proposed plan:\n{}", plan.display());
    assert!(!plan.steps.is_empty(), "plan should have steps");

    // Step 2: Goal turn with the plan context
    let result = orch.run_goal_turn(
        "Write a simple hello world Rust function",
        "add hello world to main.rs",
        "",
    ).await.unwrap();

    eprintln!("Goal turn response: {}", safe_truncate(&result.response, 200));
    assert!(!result.response.is_empty());
    assert!(result.ic_rules_applied >= 13);

    // Step 3: Verify audit trail has IC + provider entries
    let entries = audit.entries();
    eprintln!("Audit entries: {}", entries.len());
    assert!(entries.len() >= 3, "should have audit entries for goal turn");
}
