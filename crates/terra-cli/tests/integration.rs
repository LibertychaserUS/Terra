//! Integration tests for Terra Agent OS.
//!
//! Verify cross-crate behavior through the canonical pipeline.

use std::path::PathBuf;
use std::sync::Arc;

use terra_agents::Orchestrator;
use terra_caging::cage::{CageAction, CageContext};
use terra_caging::engine::*;
use terra_caging::sandbox::SandboxGuard;
use terra_core::domain::{AlgorithmModule, CodingModule};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_memory::MemoryDatabase;
use terra_memory::types::{Level, Scope};
use terra_trace::AuditLog;

use terra_memory::ArtifactStore;
use terra_providers::adapter::{DryRunAdapter, ProviderAdapter};

fn test_orchestrator() -> (Orchestrator, Arc<AuditLog>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
    let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
    let audit = Arc::new(AuditLog::new());
    let provider: Arc<dyn ProviderAdapter> = Arc::new(DryRunAdapter::new());
    let sandbox = SandboxGuard::new(PathBuf::from("/projects/test"), vec![]);
    let caging = Arc::new(CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]));
    let executor = Arc::new(CommandExecutor::new(memory, artifacts, audit.clone(), caging, provider));

    let mut core = IntelligenceCore::new();
    core.register(Box::new(CodingModule));
    core.register(Box::new(AlgorithmModule));
    let core = Arc::new(core);

    (Orchestrator::new(executor, core), audit, dir)
}

/// T-INT-001: Run task → result available.
#[tokio::test]
async fn run_produces_result() {
    let (orch, _audit, _dir) = test_orchestrator();
    let result = orch.run("fix the auth bug").await.unwrap();
    assert!(!result.worker_response.is_empty());
    assert!(result.ic_rules_applied > 0);
}

/// T-INT-002: Sandbox enforced in executor.
#[test]
fn sandbox_enforced() {
    let sandbox = SandboxGuard::new(
        PathBuf::from("/projects/myapp"),
        vec![PathBuf::from("/projects/myapp/.terra/constitution.toml")],
    );
    assert!(sandbox.check_write(&PathBuf::from("/projects/myapp/src/main.rs")).is_ok());
    assert!(sandbox.check_write(&PathBuf::from("/etc/hosts")).is_err());
    assert!(sandbox.check_write(&PathBuf::from("/projects/myapp/.terra/constitution.toml")).is_err());
    assert!(sandbox.check_read(&PathBuf::from("/projects/myapp/.terra/constitution.toml")).is_ok());
}

/// T-INT-003: Full caging chain checked.
#[test]
fn full_caging_chain() {
    let sandbox = SandboxGuard::new(PathBuf::from("/projects/myapp"), vec![]);
    let engine = CagingEngine::new(vec![
        Box::new(SpecificationCage),
        Box::new(MemoryCage),
        Box::new(ActionCage::new(sandbox)),
        Box::new(VerificationCage),
        Box::new(MetaCage),
    ]);

    let ctx = CageContext {
        agent: "main".into(),
        action: CageAction::ExecuteTask { spec_complete: true },
    };
    assert!(engine.check(&ctx).allowed);

    let bad_ctx = CageContext {
        agent: "main".into(),
        action: CageAction::ExecuteTask { spec_complete: false },
    };
    assert!(!engine.check(&bad_ctx).allowed);
}

/// T-INT-004: Audit trail records pipeline steps.
#[tokio::test]
async fn audit_trail_pipeline() {
    let (orch, audit, _dir) = test_orchestrator();
    orch.run("test task").await.unwrap();

    let entries = audit.entries();
    assert!(entries.len() >= 5, "pipeline should record at least 5 audit entries");

    // Verify sequential numbering
    for (i, entry) in entries.iter().enumerate() {
        assert_eq!(entry.seq, (i + 1) as u64);
    }
}

/// T-INT-006: Project isolation.
#[test]
fn project_isolation() {
    let db_a = MemoryDatabase::open_in_memory().unwrap();
    let db_b = MemoryDatabase::open_in_memory().unwrap();

    db_a.write("shared_key", Scope::Project, Level::L1, "main", b"project_a".to_vec()).unwrap();
    assert!(db_b.read("shared_key").unwrap().is_none());

    db_b.write("shared_key", Scope::Project, Level::L1, "main", b"project_b".to_vec()).unwrap();
    let a = db_a.read("shared_key").unwrap().unwrap();
    let b = db_b.read("shared_key").unwrap().unwrap();
    assert_eq!(a.content, b"project_a");
    assert_eq!(b.content, b"project_b");
}

/// T-PI-001: Pipeline IC rules applied.
#[tokio::test]
async fn pipeline_ic_rules_applied() {
    let (orch, _, _dir) = test_orchestrator();
    let result = orch.run("refactor module").await.unwrap();
    // Universal (13) + CodingModule (3) + AlgorithmModule (2) = 18
    assert!(result.ic_rules_applied >= 15, "should have 15+ rules, got {}", result.ic_rules_applied);
}
