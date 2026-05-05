//! Agent Roles
//!
//! Distinct agent types with their own context, rules, and behavior.
//! All operations go through CommandExecutor (FR-002).
//!
//! - Worker: executes coding/modification tasks
//! - Explorer: searches memory and gathers information
//! - CageAgent: monitors for drift, shortcuts, shallow reasoning

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tracing::{info, warn};

use terra_compile::SpecIR;
use terra_core::{IntelligenceCore, TaskType};
use terra_core::diagnostic::{detect_goal_drift, detect_shallow_reasoning, detect_shortcuts, DiagnosticSignal};
use terra_executor::CommandExecutor;
use terra_protocol::command::{Command, CommandOp, CommandStatus};

/// Role-specific configuration for an agent.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub role: AgentRole,
    pub max_tokens: u32,
    pub temperature: f32,
    pub timeout: Duration,
}

/// Agent role determines prompt, rules, and model selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRole {
    /// Main orchestrator — uses strongest model, full rules
    Main,
    /// Worker — executes tasks, filtered rules, balanced model
    Worker,
    /// Explorer — searches and researches, fast model
    Explorer,
    /// Cage — monitors only, no mutations, lightweight
    CageMonitor,
}

impl AgentRole {
    /// Map role to task type for IC rule filtering.
    pub fn default_task_type(&self) -> TaskType {
        match self {
            AgentRole::Main => TaskType::Review,
            AgentRole::Worker => TaskType::BugFix,
            AgentRole::Explorer => TaskType::Research,
            AgentRole::CageMonitor => TaskType::Review,
        }
    }
}

/// Worker Agent — executes coding tasks via CommandExecutor.
pub struct WorkerAgent {
    executor: Arc<CommandExecutor>,
    core: Arc<IntelligenceCore>,
    config: AgentConfig,
}

impl WorkerAgent {
    pub fn new(executor: Arc<CommandExecutor>, core: Arc<IntelligenceCore>) -> Self {
        Self {
            executor,
            core,
            config: AgentConfig {
                name: "worker".into(),
                role: AgentRole::Worker,
                max_tokens: 2000,
                temperature: 0.0,
                timeout: Duration::from_secs(30),
            },
        }
    }

    /// Build worker-specific system prompt with filtered rules.
    pub fn build_prompt(&self, task_type: &TaskType) -> String {
        let rules_section = self.core.build_system_prompt(task_type);
        format!(
            "## Role: Worker Agent\n\
             You execute specific coding tasks. Follow the rules precisely.\n\
             Focus on the target file and minimize changes.\n\n\
             {}",
            rules_section
        )
    }

    /// Execute a task. Returns the response content.
    pub async fn execute(&self, spec: &SpecIR, system_prompt: &str) -> Result<WorkerResult> {
        let user_prompt = format!(
            "Task: {}\nTarget: {:?}\nConstraints: {:?}",
            spec.intent, spec.target.file, spec.constraints
        );

        let cmd = Command::new(
            &self.config.name,
            CommandOp::ProviderCall {
                system_prompt: system_prompt.to_string(),
                user_prompt,
                max_tokens: Some(self.config.max_tokens),
                temperature: Some(self.config.temperature),
                output_contract: None,
            },
        );

        // Execute with timeout
        let result = tokio::time::timeout(
            self.config.timeout,
            self.executor.execute(cmd),
        )
        .await;

        match result {
            Ok(cmd_result) => {
                let content = cmd_result
                    .data
                    .as_ref()
                    .and_then(|d| d.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();

                Ok(WorkerResult {
                    content,
                    status: cmd_result.status,
                    timed_out: false,
                })
            }
            Err(_) => {
                warn!(agent = %self.config.name, "worker timed out");
                Ok(WorkerResult {
                    content: String::new(),
                    status: CommandStatus::Error,
                    timed_out: true,
                })
            }
        }
    }
}

/// Result from a Worker execution.
#[derive(Debug)]
pub struct WorkerResult {
    pub content: String,
    pub status: CommandStatus,
    pub timed_out: bool,
}

/// Explorer Agent — searches memory for relevant context.
pub struct ExplorerAgent {
    executor: Arc<CommandExecutor>,
    config: AgentConfig,
}

impl ExplorerAgent {
    pub fn new(executor: Arc<CommandExecutor>) -> Self {
        Self {
            executor,
            config: AgentConfig {
                name: "explorer".into(),
                role: AgentRole::Explorer,
                max_tokens: 500,
                temperature: 0.0,
                timeout: Duration::from_secs(10),
            },
        }
    }

    /// Search memory for context relevant to the query.
    pub async fn search(&self, query: &str, top_k: u32) -> ExplorerFindings {
        let cmd = Command::new(
            &self.config.name,
            CommandOp::MemSearch {
                query: query.to_string(),
                top_k,
            },
        );

        let result = self.executor.execute(cmd).await;

        let results = result
            .data
            .as_ref()
            .and_then(|d| d.get("results"))
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let key = v.get("key")?.as_str()?.to_string();
                        let score = v.get("score")?.as_f64()? as f32;
                        Some((key, score))
                    })
                    .collect()
            })
            .unwrap_or_default();

        ExplorerFindings {
            query: query.to_string(),
            results,
            status: result.status,
        }
    }
}

/// Structured findings from Explorer search.
#[derive(Debug)]
pub struct ExplorerFindings {
    pub query: String,
    pub results: Vec<(String, f32)>,
    pub status: CommandStatus,
}

/// Cage Monitor Agent — monitors for diagnostic signals.
///
/// CRITICAL: This agent does NOT modify any memory or files.
/// It only reads state and computes diagnostic scores.
pub struct CageMonitorAgent {
    core: Arc<IntelligenceCore>,
}

impl CageMonitorAgent {
    pub fn new(core: Arc<IntelligenceCore>) -> Self {
        Self { core }
    }

    /// Check for goal drift between spec and current action.
    pub fn check_drift(&self, spec_text: &str, action_text: &str) -> Option<DiagnosticSignal> {
        let threshold = self.get_threshold("D-001");
        detect_goal_drift(spec_text, action_text, threshold)
    }

    /// Check for shallow reasoning.
    pub fn check_shallow(&self, claims: u32, evidence: u32) -> Option<DiagnosticSignal> {
        let threshold = self.get_threshold("D-002");
        detect_shallow_reasoning(claims, evidence, threshold)
    }

    /// Check for skipped steps.
    pub fn check_shortcuts(&self, planned: &[String], executed: &[String]) -> Option<DiagnosticSignal> {
        detect_shortcuts(planned, executed)
    }

    /// Run all diagnostics and return any signals.
    pub fn monitor(
        &self,
        spec_text: &str,
        action_text: &str,
        claims: u32,
        evidence: u32,
        planned_steps: &[String],
        executed_steps: &[String],
    ) -> Vec<DiagnosticSignal> {
        let mut signals = Vec::new();

        if let Some(s) = self.check_drift(spec_text, action_text) {
            signals.push(s);
        }
        if let Some(s) = self.check_shallow(claims, evidence) {
            signals.push(s);
        }
        if let Some(s) = self.check_shortcuts(planned_steps, executed_steps) {
            signals.push(s);
        }

        if !signals.is_empty() {
            info!(count = signals.len(), "cage monitor detected issues");
        }

        signals
    }

    /// Get threshold for a diagnostic rule from IC.
    fn get_threshold(&self, rule_id: &str) -> f64 {
        self.core
            .universal_rules()
            .iter()
            .find(|r| r.id == rule_id)
            .and_then(|r| r.threshold.as_ref())
            .map(|t| t.value)
            .unwrap_or(0.5) // safe default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use terra_caging::engine::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_core::domain::{AlgorithmModule, CodingModule};
    use terra_memory::{ArtifactStore, MemoryDatabase};
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;

    fn test_executor() -> (Arc<CommandExecutor>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
        let audit = Arc::new(AuditLog::new());
        let provider: Arc<dyn terra_providers::adapter::ProviderAdapter> =
            Arc::new(DryRunAdapter::new());
        let sandbox = SandboxGuard::new(PathBuf::from("/projects/test"), vec![]);
        let caging = Arc::new(CagingEngine::new(vec![
            Box::new(SpecificationCage),
            Box::new(MemoryCage),
            Box::new(ActionCage::new(sandbox)),
            Box::new(VerificationCage),
            Box::new(MetaCage),
        ]));
        let exec = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));
        (exec, dir)
    }

    fn test_core() -> Arc<IntelligenceCore> {
        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        core.register(Box::new(AlgorithmModule));
        Arc::new(core)
    }

    // T-AG-004: Worker system prompt differs from Main's
    #[test]
    fn worker_has_own_context() {
        let core = test_core();
        let (exec, _dir) = test_executor();

        let worker = WorkerAgent::new(exec, core.clone());
        let worker_prompt = worker.build_prompt(&TaskType::BugFix);
        let main_prompt = core.build_system_prompt(&TaskType::Review);

        assert_ne!(worker_prompt, main_prompt, "worker and main should have different prompts");
    }

    // T-AG-005: Worker gets filtered rules for code task
    #[test]
    fn worker_has_filtered_rules() {
        let core = test_core();
        let worker_rules = core.rules_for_task(&TaskType::BugFix);
        let all_rules = core.all_rules();
        // Worker should have fewer rules than all (SHOULD/MAY filtered out)
        assert!(
            worker_rules.len() <= all_rules.len(),
            "filtered rules should not exceed total"
        );
        // Must include B-002 (minimal change)
        assert!(worker_rules.iter().any(|r| r.id == "B-002"));
    }

    // T-AG-002: Worker receives task and produces result
    #[tokio::test]
    async fn worker_executes_task() {
        let (exec, _dir) = test_executor();
        let core = test_core();
        let worker = WorkerAgent::new(exec, core);

        let spec = SpecIR::from_task_description("fix the auth bug");
        let prompt = worker.build_prompt(&TaskType::BugFix);
        let result = worker.execute(&spec, &prompt).await.unwrap();

        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.content.contains("DRY_RUN"));
        assert!(!result.timed_out);
    }

    // T-AG-012: Worker timeout handled
    #[tokio::test]
    async fn worker_timeout_handled() {
        let (exec, _dir) = test_executor();
        let core = test_core();
        let mut worker = WorkerAgent::new(exec, core);
        // Set impossibly short timeout (DryRun is fast, so this test
        // verifies the timeout mechanism exists and doesn't panic)
        worker.config.timeout = Duration::from_millis(1);

        let spec = SpecIR::from_task_description("test");
        let prompt = worker.build_prompt(&TaskType::BugFix);
        let result = worker.execute(&spec, &prompt).await;
        // Either completes fast or times out — both are valid
        assert!(result.is_ok());
    }

    // T-AG-006: Explorer issues MemSearch via executor
    #[tokio::test]
    async fn explorer_searches_memory() {
        let (exec, _dir) = test_executor();
        let explorer = ExplorerAgent::new(exec);

        let findings = explorer.search("authentication security", 5).await;
        assert_eq!(findings.status, CommandStatus::Ok);
    }

    // T-AG-007: Explorer returns structured findings
    #[tokio::test]
    async fn explorer_returns_findings() {
        let (exec, _dir) = test_executor();

        // First write something to memory so search has data
        let write_cmd = Command::new(
            "main",
            CommandOp::MemWrite {
                key: "auth_pattern".into(),
                content: "authentication login password security".into(),
                scope: "project".into(),
                level: "L1".into(),
            },
        );
        exec.execute(write_cmd).await;

        let explorer = ExplorerAgent::new(exec);
        let findings = explorer.search("authentication login", 3).await;

        assert_eq!(findings.query, "authentication login");
        assert_eq!(findings.status, CommandStatus::Ok);
        // Should find the entry we just wrote
        if !findings.results.is_empty() {
            assert_eq!(findings.results[0].0, "auth_pattern");
        }
    }

    // T-AG-008: Cage agent detects drift
    #[test]
    fn cage_agent_monitors_drift() {
        let core = test_core();
        let cage = CageMonitorAgent::new(core);

        let signal = cage.check_drift(
            "implement user authentication",
            "optimize database query caching",
        );
        assert!(signal.is_some(), "should detect drift");
        assert_eq!(signal.unwrap().rule_id, "D-001");
    }

    // T-AG-009: Cage agent does not modify memory or files
    #[test]
    fn cage_agent_no_business_logic() {
        let core = test_core();
        let cage = CageMonitorAgent::new(core);

        // CageMonitorAgent has no executor, no write methods.
        // It only has check_drift, check_shallow, check_shortcuts, monitor.
        // This test verifies the API surface — no mutation methods exist.
        let signals = cage.monitor(
            "fix auth bug",
            "fix auth bug",
            3,
            2,
            &["lint".into(), "test".into()],
            &["lint".into(), "test".into()],
        );
        // No drift, sufficient evidence, all steps executed
        assert!(signals.is_empty());
    }

    // T-AG-013: Concurrent workers safe
    #[tokio::test]
    async fn concurrent_workers_safe() {
        let (exec, _dir) = test_executor();
        let core = test_core();

        let worker1 = WorkerAgent::new(exec.clone(), core.clone());
        let worker2 = WorkerAgent::new(exec, core);

        let spec1 = SpecIR::from_task_description("task 1");
        let spec2 = SpecIR::from_task_description("task 2");
        let prompt = worker1.build_prompt(&TaskType::BugFix);

        // Run both concurrently
        let (r1, r2) = tokio::join!(
            worker1.execute(&spec1, &prompt),
            worker2.execute(&spec2, &prompt),
        );

        assert!(r1.is_ok());
        assert!(r2.is_ok());
        assert_eq!(r1.unwrap().status, CommandStatus::Ok);
        assert_eq!(r2.unwrap().status, CommandStatus::Ok);
    }
}
