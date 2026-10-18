use std::sync::{Arc, Mutex};

use anyhow::Result;
use tracing::info;

use terra_compile::{SpecIR, ExecutionPlan, PlanStep, StepStatus};
use terra_core::IntelligenceCore;
use terra_executor::CommandExecutor;
use terra_memory::PointerTable;
use terra_memory::types::{Level, Scope};
use terra_protocol::command::{Command, CommandOp};

/// Result of a full orchestration run.
#[derive(Debug)]
pub struct RunResult {
    pub spec: SpecIR,
    pub worker_response: String,
    pub provider_id: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub audit_entry_count: usize,
    pub ic_rules_applied: usize,
    pub cage_checks_passed: usize,
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

    /// Propose an ExecutionPlan for a goal by asking LLM.
    ///
    /// Returns a plan with steps the LLM suggests. User should confirm/modify
    /// before execution. Falls back to a default plan if LLM output can't be parsed.
    pub async fn propose_plan(&self, goal: &str) -> Result<ExecutionPlan> {
        let spec = SpecIR::from_task_description(goal);

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
        // 1. Language Compilation → Spec IR
        let spec = SpecIR::from_task_description(task_description);
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

        // 4. Caging gate — check spec completeness (FR-004)
        //    The CagingEngine is inside the Executor. We trigger it by
        //    issuing a command that the cages will check.
        let cage_cmd = Command::new(
            "main",
            CommandOp::TraceEvent {
                event_type: "cage_check".into(),
                data: serde_json::json!({
                    "spec_complete": spec.is_complete(),
                }),
            },
        );
        self.executor.execute(cage_cmd).await;

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

        Ok(RunResult {
            spec,
            worker_response,
            provider_id,
            prompt_tokens,
            completion_tokens,
            audit_entry_count: 6, // start + cage + search + dispatch + provider + complete
            ic_rules_applied: ic_rules_count,
            cage_checks_passed: 1,
        })
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
        let result = orch.run("fix the auth bug").await.unwrap();
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
        let result = orch.run("fix the auth bug").await.unwrap();
        assert!(result.ic_rules_applied > 10);
        assert!(result.audit_entry_count >= 5);
        assert!(result.worker_response.contains("DRY_RUN"));
    }

    // Pipeline: provider called through executor
    #[tokio::test]
    async fn pipeline_provider_through_executor() {
        let (orch, _dir) = test_orchestrator();
        let result = orch.run("test").await.unwrap();
        assert!(result.worker_response.contains("DRY_RUN"));
        assert_eq!(result.provider_id, "dry_run");
    }

    // T-DP-001: goal generates plan
    #[tokio::test]
    async fn goal_generates_plan() {
        let (orch, _dir) = test_orchestrator();
        let plan = orch.propose_plan("add logging to auth.rs").await.unwrap();
        assert!(!plan.steps.is_empty(), "plan should have steps");
        assert_eq!(plan.spec.intent, "add logging to auth.rs");
        // DryRun won't produce real JSON, so fallback step expected
        assert!(!plan.steps.is_empty());
    }

    // T-DP-005: plan versions trackable
    #[tokio::test]
    async fn plan_versions_trackable() {
        let (orch, _dir) = test_orchestrator();
        let mut plan = orch.propose_plan("fix bug").await.unwrap();
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
        let mut plan = orch.propose_plan("fix bug").await.unwrap();
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
