use std::sync::Arc;

use anyhow::Result;
use terra_compile::SpecIR;
use terra_protocol::command::{Command, CommandOp};
use terra_protocol::message::{
    AgentMessage, AgentRole, ResultPayload, ResultStatus, TaskPayload,
};
use terra_providers::adapter::{CompletionRequest, ProviderAdapter};
use terra_providers::dispatch::{self, DispatchMode};
use terra_trace::AuditLog;

/// Result of a full orchestration run.
#[derive(Debug)]
pub struct RunResult {
    pub spec: SpecIR,
    pub worker_response: String,
    pub dispatch_mode: DispatchMode,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub audit_entry_count: usize,
}

/// The orchestrator implements the MVP1 end-to-end pipeline:
///
/// User task → Spec IR → Main Agent → Provider → Worker result → Trace
pub struct Orchestrator {
    provider: Arc<dyn ProviderAdapter>,
    audit_log: Arc<AuditLog>,
}

impl Orchestrator {
    pub fn new(provider: Arc<dyn ProviderAdapter>, audit_log: Arc<AuditLog>) -> Self {
        Self { provider, audit_log }
    }

    /// Run a task end-to-end.
    pub async fn run(&self, task_description: &str) -> Result<RunResult> {
        // 1. Language Compilation → Spec IR
        let spec = SpecIR::from_task_description(task_description);

        // 2. Record the task start
        let start_cmd = Command::new(
            "main",
            CommandOp::TraceEvent {
                event_type: "task_start".into(),
                data: serde_json::json!({
                    "spec_id": spec.spec_id,
                    "intent": spec.intent,
                }),
            },
        );
        self.audit_log.record_command(start_cmd);

        // 3. Main Agent creates a task assignment for Worker
        let task_msg = AgentMessage::task_assignment(
            AgentRole::Worker,
            TaskPayload {
                op: "execute_task".into(),
                target: serde_json::json!({
                    "intent": spec.intent,
                    "target": spec.target,
                }),
                constraints: spec.constraints.clone(),
                expected_outputs: vec!["response".into()],
                budget_max_tokens: Some(2000),
            },
        );

        // 4. Record the dispatch command
        let dispatch_cmd = Command::new(
            "main",
            CommandOp::TraceEvent {
                event_type: "worker_dispatch".into(),
                data: serde_json::json!({
                    "message_id": task_msg.id,
                    "to": "worker",
                }),
            },
        );
        self.audit_log.record_command(dispatch_cmd);

        // 5. Worker executes via Provider
        let request = CompletionRequest {
            system_prompt: "You are a precise coding assistant. Execute the task exactly as specified.".into(),
            user_prompt: format!(
                "Task: {}\nConstraints: {:?}",
                spec.intent, spec.constraints
            ),
            max_tokens: Some(2000),
            temperature: Some(0.0),
        };

        let dispatch_mode = if self.provider.capabilities().provider_id == "dry_run" {
            DispatchMode::DryRun
        } else {
            DispatchMode::Live
        };

        let dispatch_result = dispatch::dispatch(
            self.provider.as_ref(),
            request,
            dispatch_mode,
        )
        .await?;

        // 6. Worker returns result to Main
        let _result_msg = AgentMessage::task_result(
            AgentRole::Worker,
            ResultPayload {
                status: ResultStatus::Success,
                outputs: serde_json::json!({
                    "response": dispatch_result.response.content,
                }),
                error: None,
            },
        );

        // 7. Record completion
        let complete_cmd = Command::new(
            "main",
            CommandOp::TraceCheckpoint {
                title: "Task completed".into(),
                summary: format!(
                    "Spec: {} | Mode: {:?} | Tokens: {}+{}",
                    spec.spec_id,
                    dispatch_result.mode,
                    dispatch_result.response.usage_prompt_tokens,
                    dispatch_result.response.usage_completion_tokens,
                ),
            },
        );
        self.audit_log.record_command(complete_cmd);

        Ok(RunResult {
            spec,
            worker_response: dispatch_result.response.content.clone(),
            dispatch_mode: dispatch_result.mode,
            prompt_tokens: dispatch_result.response.usage_prompt_tokens,
            completion_tokens: dispatch_result.response.usage_completion_tokens,
            audit_entry_count: self.audit_log.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_providers::adapter::DryRunAdapter;

    #[tokio::test]
    async fn end_to_end_dry_run() {
        let provider = Arc::new(DryRunAdapter::new());
        let audit_log = Arc::new(AuditLog::new());
        let orchestrator = Orchestrator::new(provider, audit_log.clone());

        let result = orchestrator
            .run("fix the empty password bug in auth.py line 42")
            .await
            .unwrap();

        // Spec IR was created
        assert_eq!(result.spec.intent, "fix the empty password bug in auth.py line 42");

        // Provider was called (dry run)
        assert_eq!(result.dispatch_mode, DispatchMode::DryRun);
        assert!(result.worker_response.contains("DRY_RUN"));

        // Audit trail was recorded
        assert_eq!(result.audit_entry_count, 3); // start + dispatch + checkpoint
        let entries = audit_log.entries();
        assert_eq!(entries.len(), 3);

        // Can render audit to markdown
        let md = audit_log.render_markdown();
        assert!(md.contains("Task completed"));
    }
}
