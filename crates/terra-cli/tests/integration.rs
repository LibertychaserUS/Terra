//! Integration tests for Terra Agent OS.
//!
//! These tests verify cross-crate behavior: the full pipeline
//! from task input through orchestration, memory, caging, and audit.

use std::path::PathBuf;
use std::sync::Arc;

use terra_agents::Orchestrator;
use terra_caging::cage::{CageAction, CageContext};
use terra_caging::engine::CagingEngine;
use terra_caging::engine::{
    ActionCage, MemoryCage, MetaCage, SpecificationCage, VerificationCage,
};
use terra_caging::sandbox::SandboxGuard;
use terra_memory::MemoryDatabase;
use terra_memory::types::{Level, Scope};
use terra_providers::adapter::DryRunAdapter;
use terra_providers::dispatch::DispatchMode;
use terra_trace::AuditLog;

/// T-INT-001: Run task → result stored in memory → retrievable.
#[tokio::test]
async fn run_with_memory() {
    let provider = Arc::new(DryRunAdapter::new());
    let audit_log = Arc::new(AuditLog::new());
    let orchestrator = Orchestrator::new(provider, audit_log.clone());

    // Execute task
    let result = orchestrator.run("fix the auth bug").await.unwrap();
    assert!(!result.worker_response.is_empty());

    // Store result in memory
    let db = MemoryDatabase::open_in_memory().unwrap();
    let content = terra_protocol::codec::encode(&result.worker_response).unwrap();
    db.write(
        result.spec.spec_id.clone(),
        Scope::Task,
        Level::L3,
        "main",
        content,
    )
    .unwrap();

    // Retrieve from memory
    let entry = db.read(&result.spec.spec_id).unwrap().unwrap();
    let decoded: String = terra_protocol::codec::decode(&entry.content).unwrap();
    assert_eq!(decoded, result.worker_response);
}

/// T-INT-002: Sandbox allows inside, blocks outside.
#[test]
fn run_with_sandbox() {
    let sandbox = SandboxGuard::new(
        PathBuf::from("/projects/myapp"),
        vec![PathBuf::from("/projects/myapp/.terra/constitution.toml")],
    );

    // Inside sandbox: allowed
    assert!(sandbox
        .check_write(&PathBuf::from("/projects/myapp/src/main.rs"))
        .is_ok());

    // Outside sandbox: blocked
    assert!(sandbox
        .check_write(&PathBuf::from("/etc/hosts"))
        .is_err());

    // Protected path: blocked for write, allowed for read
    assert!(sandbox
        .check_write(&PathBuf::from("/projects/myapp/.terra/constitution.toml"))
        .is_err());
    assert!(sandbox
        .check_read(&PathBuf::from("/projects/myapp/.terra/constitution.toml"))
        .is_ok());
}

/// T-INT-003: All cages checked for a valid task.
#[test]
fn run_with_caging() {
    let sandbox = SandboxGuard::new(
        PathBuf::from("/projects/myapp"),
        vec![],
    );

    let engine = CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]);

    // Valid task: complete spec, verified
    let ctx = CageContext {
        agent: "main".into(),
        action: CageAction::ExecuteTask { spec_complete: true },
    };
    let result = engine.check(&ctx);
    assert!(result.allowed);
    assert!(result.block.is_none());

    // Invalid: incomplete spec
    let ctx_bad = CageContext {
        agent: "main".into(),
        action: CageAction::ExecuteTask { spec_complete: false },
    };
    let result_bad = engine.check(&ctx_bad);
    assert!(!result_bad.allowed);
}

/// T-INT-004: Audit log replay produces same entries.
#[tokio::test]
async fn audit_replay() {
    let provider = Arc::new(DryRunAdapter::new());
    let audit_log = Arc::new(AuditLog::new());
    let orchestrator = Orchestrator::new(provider, audit_log.clone());

    // Run a task
    orchestrator.run("test task").await.unwrap();

    // Get audit entries
    let entries = audit_log.entries();
    assert!(entries.len() >= 3);

    // Verify entries are sequential
    for (i, entry) in entries.iter().enumerate() {
        assert_eq!(entry.seq, (i + 1) as u64);
    }

    // Verify markdown rendering contains all entries
    let md = audit_log.render_markdown();
    for entry in &entries {
        assert!(md.contains(&format!("#{}", entry.seq)));
    }
}

/// T-INT-005: Live provider (skipped if no key).
#[tokio::test]
async fn live_provider_smoke() {
    let adapter = match terra_providers::OpenAICompatibleAdapter::from_env() {
        Ok(a) => a,
        Err(_) => {
            eprintln!("SKIP: no provider API key set");
            return;
        }
    };

    let provider: Arc<dyn terra_providers::ProviderAdapter> = Arc::new(adapter);
    let audit_log = Arc::new(AuditLog::new());
    let orchestrator = Orchestrator::new(provider, audit_log.clone());

    let result = orchestrator.run("What is 1+1? One word.").await.unwrap();

    assert_eq!(result.dispatch_mode, DispatchMode::Live);
    assert!(!result.worker_response.is_empty());
    assert!(result.prompt_tokens > 0);
}

/// T-INT-006: Two projects have separate memory databases.
#[test]
fn project_isolation() {
    let db_a = MemoryDatabase::open_in_memory().unwrap();
    let db_b = MemoryDatabase::open_in_memory().unwrap();

    // Write to project A
    db_a.write("shared_key", Scope::Project, Level::L1, "main", b"project_a_data".to_vec())
        .unwrap();

    // Project B should not see project A's data
    let result = db_b.read("shared_key").unwrap();
    assert!(result.is_none());

    // Project B writes its own data
    db_b.write("shared_key", Scope::Project, Level::L1, "main", b"project_b_data".to_vec())
        .unwrap();

    // Both have independent entries
    let a = db_a.read("shared_key").unwrap().unwrap();
    let b = db_b.read("shared_key").unwrap().unwrap();
    assert_eq!(a.content, b"project_a_data");
    assert_eq!(b.content, b"project_b_data");
}
