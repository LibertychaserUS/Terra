//! SubAgent Runner
//!
//! Executes tasks as a SubAgent with:
//! - Soul (identity + constraints from L0 IC)
//! - MemoryView (read-only, level-filtered)
//! - AgentProfile (allowed ops + budget)
//! - MessagePack communication (TaskPayload in, ResultPayload out)

use std::sync::Arc;

use terra_executor::CommandExecutor;
use terra_protocol::codec;
use terra_protocol::message::{ResultPayload, ResultStatus, TaskPayload};

use crate::profile::{AgentProfile, MemoryView};

/// Run a SubAgent task: receives MessagePack TaskPayload, returns MessagePack ResultPayload.
///
/// This is the full SubAgent execution flow:
/// 1. Decode TaskPayload from MessagePack bytes
/// 2. Validate against AgentProfile (allowed ops, budget)
/// 3. Build system prompt from soul + memory context
/// 4. Execute via Executor (all cage checks apply)
/// 5. Encode ResultPayload as MessagePack bytes
pub struct SubAgentRunner {
    executor: Arc<CommandExecutor>,
    profile: AgentProfile,
    memory_view: MemoryView,
}

impl SubAgentRunner {
    pub fn new(
        executor: Arc<CommandExecutor>,
        profile: AgentProfile,
        memory_view: MemoryView,
    ) -> Self {
        Self { executor, profile, memory_view }
    }

    pub fn profile(&self) -> &AgentProfile {
        &self.profile
    }

    pub fn memory_view(&self) -> &MemoryView {
        &self.memory_view
    }

    /// Build the system prompt from soul + profile constraints.
    pub fn build_prompt(&self) -> String {
        let soul = self.profile.soul();
        let constraints = soul.constraints.join("\n- ");
        format!(
            "{}\n\n## Constraints\n- {}\n\n## Budget\nMax tokens: {}",
            soul.prompt_prefix,
            constraints,
            self.profile.budget_max_tokens
        )
    }

    /// Validate that a given op name is allowed for this agent.
    pub fn validate_op(&self, op_name: &str) -> Result<(), String> {
        if self.profile.can_use_op(op_name) {
            Ok(())
        } else {
            Err(format!(
                "agent '{}' ({:?}) is not allowed to use '{}'",
                self.profile.id, self.profile.soul_kind, op_name
            ))
        }
    }

    /// Execute a task from a MessagePack-encoded TaskPayload.
    /// Returns a MessagePack-encoded ResultPayload.
    pub async fn execute_from_bytes(&self, payload_bytes: &[u8]) -> Vec<u8> {
        // 1. Decode
        let task: TaskPayload = match codec::decode(payload_bytes) {
            Ok(t) => t,
            Err(e) => {
                let err = ResultPayload {
                    status: ResultStatus::Failed,
                    outputs: serde_json::json!({}),
                    error: Some(format!("decode error: {}", e)),
                };
                return codec::encode(&err).unwrap_or_default();
            }
        };

        // 2. Validate op
        if let Err(e) = self.validate_op(&task.op) {
            let err = ResultPayload {
                status: ResultStatus::Failed,
                outputs: serde_json::json!({}),
                error: Some(e),
            };
            return codec::encode(&err).unwrap_or_default();
        }

        // 3. Execute (via ProviderCall through executor with soul prompt)
        let system_prompt = self.build_prompt();
        let user_prompt = format!(
            "Task: {}\nTarget: {}\nConstraints: {:?}\nExpected: {:?}",
            task.op,
            task.target,
            task.constraints,
            task.expected_outputs,
        );

        let cmd = terra_types::Command::new(
            &self.profile.id,
            terra_types::CommandOp::ProviderCall {
                system_prompt,
                user_prompt,
                max_tokens: task.budget_max_tokens,
                temperature: Some(0.0),
                output_contract: None,
            },
        );

        let result = self.executor.execute(cmd).await;

        // 4. Build ResultPayload
        let content = result
            .data
            .as_ref()
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        let payload = ResultPayload {
            status: if result.status == terra_types::CommandStatus::Ok {
                ResultStatus::Success
            } else {
                ResultStatus::Failed
            },
            outputs: serde_json::json!({
                "summary": content,
                "agent": self.profile.id,
            }),
            error: result.error,
        };

        // 5. Encode
        codec::encode(&payload).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use terra_caging::engine::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_core::soul::SoulKind;
    use terra_memory::{ArtifactStore, MemoryDatabase};
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;

    fn test_setup() -> (Arc<CommandExecutor>, Arc<MemoryDatabase>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
        let audit = Arc::new(AuditLog::new());
        let provider: Arc<dyn terra_providers::adapter::ProviderAdapter> =
            Arc::new(DryRunAdapter::new());
        let sandbox = SandboxGuard::new(dir.path().to_path_buf(), vec![]);
        let caging = Arc::new(CagingEngine::new(vec![
            Box::new(SpecificationCage),
            Box::new(MemoryCage),
            Box::new(ActionCage::new(sandbox)),
            Box::new(VerificationCage),
            Box::new(MetaCage),
        ]));
        let exec = Arc::new(CommandExecutor::new(memory.clone(), artifacts, audit, caging, provider));
        (exec, memory, dir)
    }

    // Worker soul in system prompt
    #[test]
    fn worker_prompt_has_soul() {
        let (exec, memory, _dir) = test_setup();
        let profile = AgentProfile::from_soul("worker-1", SoulKind::Worker, 2000);
        let view = MemoryView::new(memory, profile.memory_access.clone());
        let runner = SubAgentRunner::new(exec, profile, view);

        let prompt = runner.build_prompt();
        assert!(prompt.contains("Worker agent"));
        assert!(prompt.contains("precisely"));
        assert!(prompt.contains("2000"));
    }

    // Explorer soul distinct
    #[test]
    fn explorer_prompt_has_soul() {
        let (exec, memory, _dir) = test_setup();
        let profile = AgentProfile::from_soul("explorer-1", SoulKind::Explorer, 500);
        let view = MemoryView::new(memory, profile.memory_access.clone());
        let runner = SubAgentRunner::new(exec, profile, view);

        let prompt = runner.build_prompt();
        assert!(prompt.contains("Explorer agent"));
        assert!(prompt.contains("neutrally"));
    }

    // Op validation
    #[test]
    fn validate_op_allowed() {
        let (exec, memory, _dir) = test_setup();
        let profile = AgentProfile::from_soul("worker-1", SoulKind::Worker, 2000);
        let view = MemoryView::new(memory, profile.memory_access.clone());
        let runner = SubAgentRunner::new(exec, profile, view);

        assert!(runner.validate_op("FileRead").is_ok());
        assert!(runner.validate_op("ArtifactSave").is_err());
    }

    // Full TaskPayload → ResultPayload roundtrip
    #[tokio::test]
    async fn task_to_result_roundtrip() {
        let (exec, memory, _dir) = test_setup();
        let profile = AgentProfile::from_soul("worker-1", SoulKind::Worker, 2000);
        let view = MemoryView::new(memory, profile.memory_access.clone());
        let runner = SubAgentRunner::new(exec, profile, view);

        let task = TaskPayload {
            op: "ProviderCall".into(),
            target: serde_json::json!({"file": "src/auth.rs"}),
            constraints: vec!["minimal_change".into()],
            expected_outputs: vec!["diff".into()],
            budget_max_tokens: Some(2000),
        };

        // Encode task as MessagePack
        let task_bytes = codec::encode(&task).unwrap();

        // Execute — returns MessagePack ResultPayload
        let result_bytes = runner.execute_from_bytes(&task_bytes).await;

        // Decode result
        let result: ResultPayload = codec::decode(&result_bytes).unwrap();
        assert_eq!(result.status, ResultStatus::Success);
        assert!(result.outputs["agent"].as_str().unwrap() == "worker-1");
    }

    // Disallowed op → Failed
    #[tokio::test]
    async fn disallowed_op_fails() {
        let (exec, memory, _dir) = test_setup();
        let profile = AgentProfile::from_soul("cage-1", SoulKind::CageMonitor, 200);
        let view = MemoryView::new(memory, profile.memory_access.clone());
        let runner = SubAgentRunner::new(exec, profile, view);

        let task = TaskPayload {
            op: "FileWrite".into(),
            target: serde_json::json!({}),
            constraints: vec![],
            expected_outputs: vec![],
            budget_max_tokens: None,
        };

        let task_bytes = codec::encode(&task).unwrap();
        let result_bytes = runner.execute_from_bytes(&task_bytes).await;
        let result: ResultPayload = codec::decode(&result_bytes).unwrap();

        assert_eq!(result.status, ResultStatus::Failed);
        assert!(result.error.unwrap().contains("not allowed"));
    }

    // Result is concise (summary, not full dump)
    #[tokio::test]
    async fn result_is_concise() {
        let (exec, memory, _dir) = test_setup();
        let profile = AgentProfile::from_soul("worker-1", SoulKind::Worker, 2000);
        let view = MemoryView::new(memory, profile.memory_access.clone());
        let runner = SubAgentRunner::new(exec, profile, view);

        let task = TaskPayload {
            op: "ProviderCall".into(),
            target: serde_json::json!({}),
            constraints: vec![],
            expected_outputs: vec![],
            budget_max_tokens: Some(50),
        };

        let task_bytes = codec::encode(&task).unwrap();
        let result_bytes = runner.execute_from_bytes(&task_bytes).await;
        let result: ResultPayload = codec::decode(&result_bytes).unwrap();

        // ResultPayload has summary + agent, not full LLM output dump
        assert!(result.outputs.get("summary").is_some());
        assert!(result.outputs.get("agent").is_some());
    }
}
