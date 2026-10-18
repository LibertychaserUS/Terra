use std::sync::{Arc, Mutex};

use anyhow::Result;
use tracing::info;

use terra_compile::{SpecIR, ExecutionPlan, PlanStep, StepStatus, TermTable};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_memory::PointerTable;
use terra_memory::types::{Level, Scope};
use terra_protocol::command::{Command, CommandOp, CommandStatus};

/// Result of a full orchestration run.
#[derive(Debug)]
pub struct RunResult {
    pub spec: SpecIR,
    pub worker_response: String,
    pub provider_id: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Commands this run recorded in the audit log.
    pub audit_entry_count: usize,
    pub ic_rules_applied: usize,
    /// Commands of this run that every cage let through.
    pub cage_checks_passed: usize,
    /// Why the task did not run (a cage block), if it was stopped.
    pub blocked: Option<String>,
}

/// Result of a single goal-mode turn through the pipeline.
#[derive(Debug)]
pub struct GoalTurnResult {
    pub response: String,
    pub ic_rules_applied: usize,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

/// The Orchestrator — follows the canonical pipeline (FR-001).
///
/// Two modes:
/// - run(): single-shot task execution (terra run --task)
/// - run_goal_turn(): interactive turn within Goal Mode (terra chat /goal)
///
/// Both go through: IC → system prompt → Executor → audit.
/// CRITICAL: ALL operations through CommandExecutor (FR-002).
pub struct Orchestrator {
    executor: Arc<CommandExecutor>,
    core: Arc<IntelligenceCore>,
    pointer_table: Arc<Mutex<PointerTable>>,
}

impl Orchestrator {
    pub fn new(executor: Arc<CommandExecutor>, core: Arc<IntelligenceCore>) -> Self {
        Self {
            executor,
            core,
            pointer_table: Arc::new(Mutex::new(PointerTable::new())),
        }
    }

    /// Get a reference to the executor (for REPL to share).
    pub fn executor(&self) -> &Arc<CommandExecutor> {
        &self.executor
    }

    /// Get a reference to the IC (for REPL to share).
    pub fn core(&self) -> &Arc<IntelligenceCore> {
        &self.core
    }

    /// Get a reference to the pointer table (for REPL to share).
    pub fn pointer_table(&self) -> &Arc<Mutex<PointerTable>> {
        &self.pointer_table
    }

    /// Register a memory pointer in the pointer table.
    ///
    /// Called after successful MemWrite to keep the pointer index in sync.
    pub fn register_pointer(&self, key: &str, level: Level, scope: Scope, annotation: &str) {
        if let Ok(mut pt) = self.pointer_table.lock() {
            pt.register(key.to_string(), level, scope, annotation.to_string());
        }
        // Increment ref_count in memory DB so GC won't collect this entry
        self.executor.increment_memory_ref(key);
    }

    /// Unregister a pointer and decrement the memory ref_count.
    pub fn unregister_pointer(&self, key: &str) {
        if let Ok(mut pt) = self.pointer_table.lock() {
            pt.unregister(key);
        }
        self.executor.decrement_memory_ref(key);
    }

    /// Specification gate: ask the cages whether a task with this spec may run.
    ///
    /// Goes through the executor like any command, so the decision is audited.
    /// `Err` carries the block reason (the spec has open questions).
    pub async fn check_spec(&self, spec: &SpecIR) -> Result<(), String> {
        let cmd = Command::new("main", CommandOp::ExecuteTask {
            spec_id: spec.spec_id.clone(),
            intent: spec.intent.clone(),
            open_questions: spec.open_questions.clone(),
        });
        let result = self.executor.execute(cmd).await;
        match result.status {
            CommandStatus::Ok => Ok(()),
            _ => Err(result.error.unwrap_or_else(|| "blocked".into())),
        }
    }

    /// Build IC-enriched system prompt for Goal Mode.
    fn build_goal_prompt(&self, goal: &str) -> String {
        let rules = self.core.all_rules();
        let rules_text: String = rules
            .iter()
            .map(|r| format!("- [{}] {}: {}", r.severity_str(), r.id, r.description))
            .collect::<Vec<_>>()
            .join("\n");

        // Pointer memory context
        let pointer_ctx = {
            let pt = self.pointer_table.lock().unwrap();
            let ws = pt.working_set(10);
            if !ws.is_empty() {
                pt.serialize_compact()
            } else {
                String::new()
            }
        };

        let mut prompt = format!(
            "You are a precise coding assistant operating under Terra Agent OS.\n\
             \n\
             ## Active Rules\n\
             {}\n\
             \n\
             ## Current Goal\n\
             {}\n\
             \n\
             Execute precisely. Report results concisely.",
            rules_text, goal
        );

        if !pointer_ctx.is_empty() {
            prompt.push_str("\n\n## Memory Pointers\n");
            prompt.push_str(&pointer_ctx);
        }

        prompt
    }

    /// Run a single conversation turn through the full Goal Mode pipeline.
    ///
    /// This is what REPL calls in Goal Mode instead of direct ProviderCall.
    /// Pipeline: IC rules → system prompt → memory search → ProviderCall → audit.
    pub async fn run_goal_turn(
        &self,
        user_input: &str,
        goal: &str,
        context_history: &str,
    ) -> Result<GoalTurnResult> {
        let ic_rules_count = self.core.all_rules().len();

        // 1. Record turn start
        let start_cmd = Command::new("main", CommandOp::TraceEvent {
            event_type: "goal_turn".into(),
            data: serde_json::json!({"input_len": user_input.len()}),
        });
        self.executor.execute(start_cmd).await;

        // 2. Memory search for context
        let search_cmd = Command::new("main", CommandOp::MemSearch {
            query: user_input.to_string(),
            top_k: 3,
        });
        let search_result = self.executor.execute(search_cmd).await;
        let memory_context = search_result.data
            .as_ref()
            .and_then(|d| d.get("results"))
            .map(|r| format!("{}", r))
            .unwrap_or_default();

        // 3. Build prompt with IC rules + goal + history + memory
        let system_prompt = self.build_goal_prompt(goal);
        let user_prompt = if context_history.is_empty() {
            format!("{}\n\nMemory context: {}", user_input, memory_context)
        } else {
            format!("{}\n\nLatest: {}\n\nMemory context: {}", context_history, user_input, memory_context)
        };

        // 4. ProviderCall through Executor (cage-checked, audited)
        let provider_cmd = Command::new("main", CommandOp::ProviderCall {
            system_prompt,
            user_prompt,
            max_tokens: Some(2000),
            temperature: Some(0.0),
            output_contract: None,
        });
        let result = self.executor.execute(provider_cmd).await;

        let response = result.data.as_ref()
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("(no response)")
            .to_string();

        let prompt_tokens = result.data.as_ref()
            .and_then(|d| d.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        let completion_tokens = result.data.as_ref()
            .and_then(|d| d.get("completion_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;

        Ok(GoalTurnResult {
            response,
            ic_rules_applied: ic_rules_count,
            prompt_tokens,
            completion_tokens,
        })
    }

    /// Propose an ExecutionPlan for a goal spec by asking LLM.
    ///
    /// Returns a plan with steps the LLM suggests. User should confirm/modify
    /// before execution. Falls back to a default plan if LLM output can't be parsed.
    /// The plan keeps `spec`, so `/execute` is gated on the goal's own open questions.
    pub async fn propose_plan(&self, spec: &SpecIR) -> Result<ExecutionPlan> {
        let goal = spec.intent.as_str();
        let spec = spec.clone();

        let prompt = format!(
            "Given this coding task, propose an execution plan as JSON.\n\
             Task: {}\n\n\
             Return ONLY a JSON array of steps:\n\
             [{{\"name\": \"step_name\", \"description\": \"what to do\", \"ops\": [\"FileRead\", \"ProviderCall\"]}}]\n\n\
             Keep it concise. 2-5 steps max.",
            goal
        );

        let cmd = Command::new("main", CommandOp::ProviderCall {
            system_prompt: "You are a planning assistant. Output ONLY valid JSON.".into(),
            user_prompt: prompt,
            max_tokens: Some(500),
            temperature: Some(0.0),
            output_contract: None,
        });

        let result = self.executor.execute(cmd).await;
        let content = result.data.as_ref()
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("[]");

        let mut plan = ExecutionPlan::from_spec(spec);

        // Try to parse LLM steps
        if let Ok(steps) = serde_json::from_str::<Vec<serde_json::Value>>(
            &extract_json_array(content)
        ) {
            let mut prev_name: Option<String> = None;
            for step_val in &steps {
                let name = step_val.get("name").and_then(|n| n.as_str()).unwrap_or("step").to_string();
                let desc = step_val.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
                let ops: Vec<String> = step_val.get("ops")
                    .and_then(|o| o.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_default();

                let depends = prev_name.clone().map(|n| vec![n]).unwrap_or_default();

                plan.add_step(PlanStep {
                    name: name.clone(),
                    description: desc,
                    required_ops: ops,
                    input_ref: None,
                    depends_on: depends,
                    status: StepStatus::Pending,
                    output: None,
                });
                prev_name = Some(name);
            }
        }

        // Fallback: if no steps parsed, add a generic single step
        if plan.steps.is_empty() {
            plan.add_step(PlanStep {
                name: "execute".into(),
                description: format!("Execute: {}", goal),
                required_ops: vec!["ProviderCall".into()],
                input_ref: None,
                depends_on: vec![],
                status: StepStatus::Pending,
                output: None,
            });
        }

        info!(steps = plan.steps.len(), version = plan.version, "plan proposed");
        Ok(plan)
    }

    /// Run a task through the complete canonical pipeline (single-shot).
    pub async fn run(&self, task_description: &str) -> Result<RunResult> {
        let audit_start = self.executor.audit().last_seq();

        // 1. Language Compilation → Spec IR (lexical + terminology passes)
        let spec = SpecIR::compile_full(task_description, &TermTable::default_coding(), None);
        info!(spec_id = %spec.spec_id, intent = %spec.intent, "spec compiled");

        // 2. Intelligence Core consultation
        let rules = self.core.all_rules();
        let perspectives = self.core.all_perspectives();
        let ic_rules_count = rules.len();
        info!(rules = ic_rules_count, perspectives = perspectives.len(), "IC consulted");

        // Build system prompt with IC rules injected (FR-003, FR-025)
        let rules_text: String = rules
            .iter()
            .map(|r| format!("- [{}] {}: {}", r.severity_str(), r.id, r.description))
            .collect::<Vec<_>>()
            .join("\n");

        let system_prompt = format!(
            "You are a precise coding assistant operating under Terra Agent OS.\n\
             \n\
             ## Active Rules\n\
             {}\n\
             \n\
             ## Task\n\
             Execute the following task precisely.",
            rules_text
        );

        // 3. Record task start (via Executor — FR-002)
        let start_cmd = Command::new(
            "main",
            CommandOp::TraceEvent {
                event_type: "task_start".into(),
                data: serde_json::json!({
                    "spec_id": spec.spec_id,
                    "intent": spec.intent,
                    "ic_rules": ic_rules_count,
                }),
            },
        );
        self.executor.execute(start_cmd).await;

        // 4. Caging gate — the Specification cage blocks a spec with open
        //    questions (FR-004). A blocked task never reaches the worker.
        if let Err(reason) = self.check_spec(&spec).await {
            let blocked_cmd = Command::new(
                "main",
                CommandOp::TraceCheckpoint {
                    title: "task_blocked".into(),
                    summary: reason.clone(),
                },
            );
            self.executor.execute(blocked_cmd).await;
            let (audit_entry_count, cage_checks_passed) = self.audit_counts_since(audit_start);
            return Ok(RunResult {
                spec,
                worker_response: String::new(),
                provider_id: String::new(),
                prompt_tokens: 0,
                completion_tokens: 0,
                audit_entry_count,
                ic_rules_applied: ic_rules_count,
                cage_checks_passed,
                blocked: Some(reason),
            });
        }

        // 5. Memory search for relevant context (via Executor)
        let search_cmd = Command::new(
            "main",
            CommandOp::MemSearch {
                query: spec.intent.clone(),
                top_k: 5,
            },
        );
        let search_result = self.executor.execute(search_cmd).await;

        // 6. Prepare worker task with IC rules and context
        let memory_keys: Vec<String> = search_result.data.as_ref()
            .and_then(|d| d.get("results"))
            .and_then(|r| r.as_array())
            .map(|hits| hits.iter()
                .filter_map(|h| h.get("key").and_then(|k| k.as_str()).map(String::from))
                .collect())
            .unwrap_or_default();
        let user_prompt = format!(
            "Task: {}\nConstraints: {}\nRelated memory keys: {}",
            spec.intent,
            list_or_none(&spec.constraints),
            list_or_none(&memory_keys),
        );

        // 7. Record worker dispatch (via Executor)
        let dispatch_cmd = Command::new(
            "main",
            CommandOp::TraceEvent {
                event_type: "worker_dispatch".into(),
                data: serde_json::json!({
                    "system_prompt_length": system_prompt.len(),
                    "user_prompt_length": user_prompt.len(),
                }),
            },
        );
        self.executor.execute(dispatch_cmd).await;

        // 8. Provider call through executor (FR-002: no direct provider access)
        let provider_cmd = Command::new(
            "main",
            CommandOp::ProviderCall {
                system_prompt,
                user_prompt,
                max_tokens: Some(2000),
                temperature: Some(0.0),
                output_contract: None,
            },
        );
        let provider_result = self.executor.execute(provider_cmd).await;

        let worker_response = provider_result
            .data
            .as_ref()
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("no response")
            .to_string();

        let prompt_tokens = provider_result
            .data
            .as_ref()
            .and_then(|d| d.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;

        let completion_tokens = provider_result
            .data
            .as_ref()
            .and_then(|d| d.get("completion_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;

        let provider_id = provider_result
            .data
            .as_ref()
            .and_then(|d| d.get("model"))
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        // 9. Record completion checkpoint (via Executor)
        let complete_cmd = Command::new(
            "main",
            CommandOp::TraceCheckpoint {
                title: "task_completed".into(),
                summary: format!(
                    "Spec: {} | IC rules: {} | Pipeline: full",
                    spec.spec_id, ic_rules_count
                ),
            },
        );
        self.executor.execute(complete_cmd).await;

        let (audit_entry_count, cage_checks_passed) = self.audit_counts_since(audit_start);
        Ok(RunResult {
            spec,
            worker_response,
            provider_id,
            prompt_tokens,
            completion_tokens,
            audit_entry_count,
            ic_rules_applied: ic_rules_count,
            cage_checks_passed,
            blocked: None,
        })
    }

    /// Audit entries recorded after `seq`, and how many of them the cages allowed.
    fn audit_counts_since(&self, seq: u64) -> (usize, usize) {
        let entries = self.executor.audit().entries_after(seq);
        let passed = entries.iter()
            .filter(|e| e.result.as_ref().is_some_and(|r| r.status != CommandStatus::PermissionDenied))
            .count();
        (entries.len(), passed)
    }
}

/// Extract a JSON array from LLM response (handles code blocks).
fn extract_json_array(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with('[') {
        return trimmed.to_string();
    }
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if start < end {
            return trimmed[start..=end].to_string();
        }
    }
    "[]".to_string()
}

/// Comma-separated list for prompts, or "(none)".
pub(crate) fn list_or_none(items: &[String]) -> String {
    if items.is_empty() { "(none)".into() } else { items.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use terra_caging::engine::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_core::domain::{AlgorithmModule, CodingModule};
    use terra_memory::{MemoryDatabase, ArtifactStore};
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;

    fn test_orchestrator() -> (Orchestrator, tempfile::TempDir) {
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
        let executor = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));

        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        core.register(Box::new(AlgorithmModule));
        let core = Arc::new(core);

        (Orchestrator::new(executor, core), dir)
    }

    // T-AG-001: main_agent_queries_ic
    #[tokio::test]
    async fn main_agent_queries_ic() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("refactor the module").await.unwrap();
        assert!(result.ic_rules_applied >= 13, "IC rules should be consulted");
    }

    // T-AG-003: main_agent_audits_result
    #[tokio::test]
    async fn main_agent_audits_result() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("fix the auth bug in src/auth.rs").await.unwrap();
        // Orchestrator records audit entries at each step
        assert!(result.audit_entry_count >= 5, "audit entries should be recorded");
        // Worker response is inspected (not just passed through)
        assert!(!result.worker_response.is_empty(), "response should be present");
    }

    // T-AG-014: end_to_end_dry_run
    #[tokio::test]
    async fn end_to_end_dry_run() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("fix the empty password bug in auth.py line 42").await.unwrap();

        assert_eq!(result.spec.intent, "fix the empty password bug in auth.py line 42");
        assert!(result.ic_rules_applied >= 13);
        assert!(result.worker_response.contains("DRY_RUN"));
        assert_eq!(result.provider_id, "dry_run");
        assert!(result.audit_entry_count >= 5);
    }

    fn ops_in_audit(orch: &Orchestrator) -> Vec<(String, CommandStatus)> {
        orch.executor().audit().entries().iter()
            .map(|e| {
                let name = match &e.command.op {
                    CommandOp::ExecuteTask { .. } => "ExecuteTask",
                    CommandOp::ProviderCall { .. } => "ProviderCall",
                    _ => "other",
                };
                (name.to_string(), e.result.as_ref().unwrap().status.clone())
            })
            .collect()
    }

    // The specification cage stops a task whose spec has open questions
    // before the worker is called.
    #[tokio::test]
    async fn run_with_open_questions_is_blocked_before_the_provider_call() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("refactor the module").await.unwrap();

        assert!(!result.spec.is_complete());
        let ops = ops_in_audit(&orch);
        assert!(
            ops.contains(&("ExecuteTask".into(), CommandStatus::PermissionDenied)),
            "spec gate should be denied: {:?}", ops
        );
        assert!(!ops.iter().any(|(op, _)| op == "ProviderCall"), "no worker call: {:?}", ops);
        assert!(result.blocked.as_deref().unwrap_or("").contains("specification"));
        assert_eq!(result.audit_entry_count, ops.len());
    }

    #[tokio::test]
    async fn run_with_a_named_file_passes_the_spec_gate() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("fix the empty password bug in auth.py line 42").await.unwrap();

        assert!(result.blocked.is_none());
        let ops = ops_in_audit(&orch);
        assert!(ops.contains(&("ExecuteTask".into(), CommandStatus::Ok)), "{:?}", ops);
        assert!(ops.contains(&("ProviderCall".into(), CommandStatus::Ok)), "{:?}", ops);
        assert_eq!(result.audit_entry_count, ops.len());
        assert_eq!(result.cage_checks_passed, ops.len());
    }

    #[tokio::test]
    async fn plan_keeps_the_goal_spec() {
        let (orch, _dir) = test_orchestrator();
        let spec = SpecIR::compile("add logging to auth.rs", &terra_compile::TermTable::default_coding());
        let plan = orch.propose_plan(&spec).await.unwrap();
        assert_eq!(plan.spec.spec_id, spec.spec_id);
        assert!(plan.spec.is_complete());
    }

    // T-AG-016: empty_task_handled
    #[tokio::test]
    async fn empty_task_handled() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("").await.unwrap();
        assert_eq!(result.spec.intent, "");
        // Should not panic
    }

    // Pipeline integrity: all stages invoked
    #[tokio::test]
    async fn pipeline_all_stages_invoked() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("fix the auth bug in src/auth.rs").await.unwrap();
        assert!(result.ic_rules_applied > 10);
        assert!(result.audit_entry_count >= 5);
        assert!(result.worker_response.contains("DRY_RUN"));
    }

    // Pipeline: provider called through executor
    #[tokio::test]
    async fn pipeline_provider_through_executor() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("test main.rs").await.unwrap();
        assert!(result.worker_response.contains("DRY_RUN"));
        assert_eq!(result.provider_id, "dry_run");
    }

    // T-DP-001: goal generates plan
    #[tokio::test]
    async fn goal_generates_plan() {
        let (orch, _dir) = test_orchestrator();
        let spec = SpecIR::compile("add logging to auth.rs", &terra_compile::TermTable::default_coding());
        let plan = orch.propose_plan(&spec).await.unwrap();
        assert!(!plan.steps.is_empty(), "plan should have steps");
        assert_eq!(plan.spec.intent, "add logging to auth.rs");
        // DryRun won't produce real JSON, so fallback step expected
        assert!(!plan.steps.is_empty());
    }

    // T-DP-005: plan versions trackable
    #[tokio::test]
    async fn plan_versions_trackable() {
        let (orch, _dir) = test_orchestrator();
        let mut plan = orch.propose_plan(&SpecIR::from_task_description("fix bug")).await.unwrap();
        let v1 = plan.version;

        plan.add_step(PlanStep {
            name: "extra".into(),
            description: "extra step".into(),
            required_ops: vec![],
            input_ref: None,
            depends_on: vec![],
            status: StepStatus::Pending,
            output: None,
        });
        assert!(plan.version > v1, "version should increment");
    }

    // T-DP-006: spec mutable after compile
    #[tokio::test]
    async fn spec_mutable_after_compile() {
        let (orch, _dir) = test_orchestrator();
        let mut plan = orch.propose_plan(&SpecIR::from_task_description("fix bug")).await.unwrap();
        let v1 = plan.version;

        let mut new_spec = plan.spec.clone();
        new_spec.constraints.push("minimal_change".into());
        plan.update_spec(new_spec);

        assert!(plan.version > v1);
        assert!(plan.spec.constraints.contains(&"minimal_change".into()));
    }

    // Test extract_json_array
    #[test]
    fn extract_json_array_bare() {
        let result = extract_json_array(r#"[{"name": "test"}]"#);
        assert!(result.starts_with('['));
    }

    #[test]
    fn extract_json_array_code_block() {
        let result = extract_json_array("Here's the plan:\n```json\n[{\"name\": \"step1\"}]\n```");
        assert!(result.starts_with('['));
    }

    #[test]
    fn extract_json_array_fallback() {
        let result = extract_json_array("no json here");
        assert_eq!(result, "[]");
    }
}
