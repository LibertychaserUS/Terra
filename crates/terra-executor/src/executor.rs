use std::sync::Arc;
use std::time::Instant;

use tracing::{info, warn};

use terra_caging::cage::{CageAction, CageContext};
use terra_caging::engine::CagingEngine;
use terra_memory::{MemoryDatabase, ArtifactStore, VectorIndex};
use terra_protocol::command::{Command, CommandOp, CommandResult, CommandStatus};
use terra_trace::AuditLog;

use crate::router;

/// Generate a short status text for a CommandOp (for Event Bus).
fn status_text(op: &CommandOp) -> String {
    match op {
        CommandOp::FileRead { path } => format!("Reading {}", path.rsplit('/').next().unwrap_or(path)),
        CommandOp::FileWrite { path, .. } => format!("Writing {}", path.rsplit('/').next().unwrap_or(path)),
        CommandOp::FileDelete { path } => format!("Deleting {}", path.rsplit('/').next().unwrap_or(path)),
        CommandOp::ShellExec { command, .. } => {
            format!("Running: {}", terra_types::text::truncate_ellipsis(command, 30))
        }
        CommandOp::ProviderCall { .. } => "Thinking...".into(),
        CommandOp::MemRead { key } => format!("Reading memory '{}'", key),
        CommandOp::MemWrite { key, .. } => format!("Writing memory '{}'", key),
        CommandOp::MemSearch { .. } => "Searching memory...".into(),
        CommandOp::ArtifactSave { artifact_type, .. } => format!("Saving {}", artifact_type),
        CommandOp::ArtifactLoad { artifact_type, .. } => format!("Loading {}", artifact_type),
        CommandOp::TraceEvent { event_type, .. } => format!("Event: {}", event_type),
        CommandOp::TraceCheckpoint { title, .. } => format!("Checkpoint: {}", title),
    }
}

/// The Command Executor — single gateway for all agent-system interactions.
///
/// Pipeline for every command:
/// 1. Record command in audit log (before execution)
/// 2. Run CagingEngine.check() on the action
/// 3. If cage passes: execute via router
/// 4. Record result in audit log (after execution, with duration)
///
/// If cage blocks: return PermissionDenied without executing.
pub struct CommandExecutor {
    memory: Arc<MemoryDatabase>,
    artifacts: Arc<ArtifactStore>,
    search_index: std::sync::Mutex<VectorIndex>,
    audit: Arc<AuditLog>,
    caging: Arc<CagingEngine>,
    provider: Arc<dyn terra_providers::adapter::ProviderAdapter>,
    event_bus: Arc<dyn terra_types::EventBus>,
}

impl CommandExecutor {
    pub fn new(
        memory: Arc<MemoryDatabase>,
        artifacts: Arc<ArtifactStore>,
        audit: Arc<AuditLog>,
        caging: Arc<CagingEngine>,
        provider: Arc<dyn terra_providers::adapter::ProviderAdapter>,
    ) -> Self {
        Self {
            memory, artifacts,
            search_index: std::sync::Mutex::new(VectorIndex::new()),
            audit, caging, provider,
            event_bus: Arc::new(terra_types::NullBus),
        }
    }

    /// Attach an event bus for operation visibility.
    pub fn with_event_bus(mut self, bus: Arc<dyn terra_types::EventBus>) -> Self {
        self.event_bus = bus;
        self
    }

    /// Expose the provider adapter for direct calls (used by REPL agent loop).
    ///
    /// The REPL needs direct provider access for the tool-call loop because
    /// `CompletionRequest` carries tools/tool_results fields that `CommandOp::ProviderCall`
    /// does not support. Individual tool executions still go through `execute()` with
    /// full cage checking and audit.
    pub fn provider(&self) -> &Arc<dyn terra_providers::adapter::ProviderAdapter> {
        &self.provider
    }

    /// Clean up memory entries for a given scope (lifecycle boundary).
    /// Called by SessionManager when a goal/session ends.
    pub fn cleanup_memory_scope(&self, scope: terra_memory::types::Scope) -> usize {
        self.memory.cleanup_scope(scope).unwrap_or(0)
    }

    /// Increment ref_count for a memory entry (pointer registered).
    pub fn increment_memory_ref(&self, key: &str) {
        let _ = self.memory.increment_ref(key);
    }

    /// Decrement ref_count for a memory entry (pointer unregistered).
    pub fn decrement_memory_ref(&self, key: &str) {
        let _ = self.memory.decrement_ref(key);
    }

    /// Execute a command through the full pipeline.
    pub async fn execute(&self, cmd: Command) -> CommandResult {
        let start = Instant::now();
        let agent = cmd.agent.clone();
        let cmd_id = cmd.id.clone();

        // 1. Audit: record before execution
        let seq = self.audit.record_command(cmd.clone());

        // 2. Cage check
        let cage_context = self.command_to_cage_context(&cmd.op, &agent);
        let cage_result = self.caging.check(&cage_context);

        if !cage_result.allowed {
            let (cage_name, reason) = cage_result.block.unwrap_or(("unknown".into(), "blocked".into()));
            warn!(
                agent = %agent,
                cage = %cage_name,
                reason = %reason,
                "command blocked by cage"
            );

            let result = CommandResult {
                command_id: cmd_id,
                status: CommandStatus::PermissionDenied,
                data: None,
                error: Some(format!("cage '{}' blocked: {}", cage_name, reason)),
                notification: None,
            };

            let elapsed = start.elapsed().as_micros() as u64;
            self.audit.record_result(seq, result.clone(), elapsed);
            return result;
        }

        // Log any warnings
        for (cage_name, warning) in &cage_result.warnings {
            warn!(cage = %cage_name, warning = %warning, "cage warning");
        }

        // 3. Emit start event
        self.event_bus.emit(terra_types::StreamEvent::step_start(
            &format!("{:?}", std::mem::discriminant(&cmd.op)),
            &status_text(&cmd.op),
        ));

        // 4. Execute via router
        let mut result = router::route(
            &cmd.op, &agent, &self.memory, &self.artifacts,
            &self.search_index, &self.provider,
        ).await;
        result.command_id = cmd_id.clone();

        // 3.5. Output Cage: inspect ProviderCall responses
        if matches!(cmd.op, CommandOp::ProviderCall { .. }) {
            if let Some(ref data) = result.data {
                if let Some(content) = data.get("content").and_then(|c| c.as_str()) {
                    match terra_caging::check_output(content) {
                        terra_caging::OutputCheckResult::Pass => {}
                        terra_caging::OutputCheckResult::Warn(reason) => {
                            warn!(agent = %agent, reason = %reason, "output cage warning");
                        }
                        terra_caging::OutputCheckResult::Block(reason) => {
                            warn!(agent = %agent, reason = %reason, "output cage BLOCKED response");
                            result = CommandResult {
                                command_id: cmd_id,
                                status: CommandStatus::PermissionDenied,
                                data: None,
                                error: Some(format!("output cage blocked: {}", reason)),
                                notification: None,
                            };
                        }
                    }
                }
            }
        }

        // 5. Audit: record result with duration
        let elapsed = start.elapsed().as_micros() as u64;
        self.audit.record_result(seq, result.clone(), elapsed);

        // 6. Emit complete event
        self.event_bus.emit(terra_types::StreamEvent::step_complete(
            &format!("{:?}", std::mem::discriminant(&cmd.op)),
        ));

        info!(
            agent = %agent,
            seq = seq,
            status = ?result.status,
            duration_us = elapsed,
            "command executed"
        );

        result
    }

    /// Record an LLM call in the audit trail without going through cage/router.
    ///
    /// The REPL agent loop calls provider.complete() directly (needs tools/tool_results
    /// fields that CommandOp::ProviderCall doesn't support). This method ensures those
    /// calls are still audited.
    pub fn audit_llm_call(
        &self,
        agent: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        tool_calls_count: usize,
        iteration: u32,
    ) {
        let cmd = Command::new(agent, CommandOp::ProviderCall {
            system_prompt: format!("[agent_loop iter={}]", iteration),
            user_prompt: format!("[{} tool_calls]", tool_calls_count),
            max_tokens: None,
            temperature: None,
            output_contract: None,
        });
        let seq = self.audit.record_command(cmd);
        let result = CommandResult::ok("", Some(serde_json::json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "tool_calls": tool_calls_count,
            "iteration": iteration,
            "source": "agent_loop",
        })));
        self.audit.record_result(seq, result, 0);
    }

    /// Map a CommandOp to a CageContext for cage checking.
    fn command_to_cage_context(&self, op: &CommandOp, agent: &str) -> CageContext {
        let action = match op {
            CommandOp::MemWrite { key, level, .. } => {
                // Check if agent is owner of existing key
                let owner = self.memory.read(key)
                    .ok()
                    .flatten()
                    .map(|e| e.owner.clone())
                    .unwrap_or_else(|| agent.to_string());

                CageAction::MemoryWrite {
                    key: key.clone(),
                    owner,
                    level: level.clone(),
                    is_user_command: agent == "user",
                }
            }
            CommandOp::MemRead { .. } => {
                CageAction::FileRead {
                    path: std::path::PathBuf::from("memory://read"),
                }
            }
            CommandOp::MemSearch { .. } => {
                CageAction::FileRead {
                    path: std::path::PathBuf::from("memory://search"),
                }
            }
            CommandOp::ArtifactSave { .. } => {
                CageAction::FileWrite {
                    path: std::path::PathBuf::from("artifact://save"),
                }
            }
            CommandOp::ArtifactLoad { .. } => {
                CageAction::FileRead {
                    path: std::path::PathBuf::from("artifact://load"),
                }
            }
            CommandOp::TraceEvent { .. } | CommandOp::TraceCheckpoint { .. } => {
                // Trace operations are always allowed
                CageAction::ExecuteTask { spec_complete: true }
            }
            CommandOp::ProviderCall { .. } => {
                // Provider calls are allowed (they go through the pipeline)
                CageAction::ExecuteTask { spec_complete: true }
            }
            CommandOp::FileRead { path } => {
                CageAction::FileRead {
                    path: std::path::PathBuf::from(path),
                }
            }
            CommandOp::FileWrite { path, .. } => {
                CageAction::FileWrite {
                    path: std::path::PathBuf::from(path),
                }
            }
            CommandOp::FileDelete { path } => {
                CageAction::FileDelete {
                    path: std::path::PathBuf::from(path),
                }
            }
            CommandOp::ShellExec { command, .. } => {
                CageAction::ShellExec {
                    command: command.clone(),
                }
            }
        };

        CageContext {
            agent: agent.to_string(),
            action,
            goal_mode: true, // TODO: receive mode from session manager
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use terra_caging::engine::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_providers::adapter::DryRunAdapter;

    fn test_executor() -> (CommandExecutor, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let artifacts = Arc::new(ArtifactStore::new(dir.path().to_path_buf()));
        let audit = Arc::new(AuditLog::new());
        let provider: Arc<dyn terra_providers::adapter::ProviderAdapter> =
            Arc::new(DryRunAdapter::new());

        let sandbox = SandboxGuard::new(
            dir.path().to_path_buf(),
            vec![dir.path().join(".terra/constitution.toml")],
        );

        let caging = Arc::new(CagingEngine::new(vec![
            Box::new(SpecificationCage),
            Box::new(MemoryCage),
            Box::new(ActionCage::new(sandbox)),
            Box::new(VerificationCage),
            Box::new(MetaCage),
        ]));

        let exec = CommandExecutor::new(memory, artifacts, audit, caging, provider);
        (exec, dir)
    }

    fn write_cmd(key: &str, agent: &str) -> Command {
        Command::new(
            agent,
            CommandOp::MemWrite {
                key: key.into(),
                content: "test data".into(),
                scope: "project".into(),
                level: "L2".into(),
            },
        )
    }

    fn read_cmd(key: &str) -> Command {
        Command::new("main", CommandOp::MemRead { key: key.into() })
    }

    #[tokio::test]
    async fn audit_before_execute() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("key1", "main")).await;

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].duration_us.is_some());
    }

    #[tokio::test]
    async fn cage_check_before_execute() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("owned_key", "main")).await;
        let result = exec.execute(write_cmd("owned_key", "attacker")).await;
        assert_eq!(result.status, CommandStatus::PermissionDenied);
    }

    #[tokio::test]
    async fn cage_block_prevents_write() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("protected", "main")).await;
        let result = exec.execute(write_cmd("protected", "hacker")).await;
        assert_eq!(result.status, CommandStatus::PermissionDenied);

        let read_result = exec.execute(read_cmd("protected")).await;
        assert_eq!(read_result.status, CommandStatus::Ok);
        assert_eq!(read_result.data.unwrap()["owner"], "main");
    }

    #[tokio::test]
    async fn routes_mem_to_memory() {
        let (exec, _dir) = test_executor();
        let result = exec.execute(write_cmd("routed", "main")).await;
        assert_eq!(result.status, CommandStatus::Ok);

        let read = exec.execute(read_cmd("routed")).await;
        assert_eq!(read.status, CommandStatus::Ok);
        assert_eq!(read.data.unwrap()["key"], "routed");
    }

    #[tokio::test]
    async fn routes_trace_to_audit() {
        let (exec, _dir) = test_executor();
        let cmd = Command::new(
            "main",
            CommandOp::TraceCheckpoint {
                title: "test checkpoint".into(),
                summary: "testing routing".into(),
            },
        );
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(exec.audit.len() > 0);
    }

    #[tokio::test]
    async fn result_has_duration() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("timed", "main")).await;

        let entries = exec.audit.entries();
        assert!(entries[0].duration_us.is_some());
    }

    #[tokio::test]
    async fn permission_denied_on_block() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("owned", "alice")).await;

        let result = exec.execute(write_cmd("owned", "bob")).await;
        assert_eq!(result.status, CommandStatus::PermissionDenied);
    }

    #[tokio::test]
    async fn audit_records_command_and_result() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("audited", "main")).await;

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].result.is_some());
        assert_eq!(entries[0].result.as_ref().unwrap().status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn multiple_commands_sequential() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("a", "main")).await;
        exec.execute(write_cmd("b", "main")).await;
        exec.execute(write_cmd("c", "main")).await;

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 3);
    }

    #[tokio::test]
    async fn blocked_command_audited() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("x", "owner")).await;
        exec.execute(write_cmd("x", "attacker")).await;

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[1].result.as_ref().unwrap().status,
            CommandStatus::PermissionDenied
        );
    }

    // NEW: ProviderCall through executor
    #[tokio::test]
    async fn provider_call_through_executor() {
        let (exec, _dir) = test_executor();
        let cmd = Command::new(
            "main",
            CommandOp::ProviderCall {
                system_prompt: "You are helpful.".into(),
                user_prompt: "What is 2+2?".into(),
                max_tokens: Some(50),
                temperature: None,
                output_contract: None,
            },
        );
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Ok);
        let data = result.data.unwrap();
        assert!(data["content"].as_str().unwrap().contains("DRY_RUN"));

        // Audit recorded the provider call
        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].result.is_some());
    }

    // --- FileOps syscall tests ---

    #[tokio::test]
    async fn file_write_and_read() {
        let (exec, dir) = test_executor();
        let file_path = dir.path().join("test_file.txt");

        // Write
        let cmd = Command::new("main", CommandOp::FileWrite {
            path: file_path.to_string_lossy().to_string(),
            content: "hello terra".into(),
        });
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.data.as_ref().unwrap()["bytes_written"], 11);

        // Read back
        let cmd = Command::new("main", CommandOp::FileRead {
            path: file_path.to_string_lossy().to_string(),
        });
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert_eq!(result.data.as_ref().unwrap()["content"], "hello terra");
    }

    #[tokio::test]
    async fn file_delete() {
        let (exec, dir) = test_executor();
        let file_path = dir.path().join("to_delete.txt");
        std::fs::write(&file_path, "temp").unwrap();

        let cmd = Command::new("main", CommandOp::FileDelete {
            path: file_path.to_string_lossy().to_string(),
        });
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn file_read_nonexistent() {
        let (exec, _dir) = test_executor();
        let cmd = Command::new("main", CommandOp::FileRead {
            path: "/nonexistent/path/file.txt".into(),
        });
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Error);
    }

    #[tokio::test]
    async fn shell_exec_simple() {
        let (exec, _dir) = test_executor();
        let cmd = Command::new("main", CommandOp::ShellExec {
            command: "echo hello".into(),
            workdir: None,
            timeout_ms: Some(5000),
        });
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Ok);
        let stdout = result.data.as_ref().unwrap()["stdout"].as_str().unwrap();
        assert!(stdout.contains("hello"));
    }

    #[tokio::test]
    async fn shell_exec_failure() {
        let (exec, _dir) = test_executor();
        let cmd = Command::new("main", CommandOp::ShellExec {
            command: "exit 1".into(),
            workdir: None,
            timeout_ms: None,
        });
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Error);
    }

    #[tokio::test]
    async fn file_ops_audited() {
        let (exec, dir) = test_executor();
        let file_path = dir.path().join("audit_test.txt");

        exec.execute(Command::new("main", CommandOp::FileWrite {
            path: file_path.to_string_lossy().to_string(),
            content: "audit me".into(),
        })).await;

        exec.execute(Command::new("main", CommandOp::FileRead {
            path: file_path.to_string_lossy().to_string(),
        })).await;

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 2, "both file ops should be audited");
    }

    #[tokio::test]
    async fn audit_llm_call_records() {
        let (exec, _dir) = test_executor();
        assert_eq!(exec.audit.len(), 0);

        exec.audit_llm_call("main", 100, 50, 2, 1);

        assert_eq!(exec.audit.len(), 1, "audit_llm_call should create one entry");
    }

    #[tokio::test]
    async fn shell_exec_timeout_via_executor() {
        let (exec, _dir) = test_executor();
        let cmd = Command::new("main", CommandOp::ShellExec {
            command: "sleep 10".into(),
            workdir: None,
            timeout_ms: Some(200),
        });
        let result = exec.execute(cmd).await;
        // Should be blocked by cage (ShellExec is warned but allowed) or timeout
        // The cage may block or allow depending on configuration
        // Either way it should not hang
        assert!(result.status == CommandStatus::Error || result.status == CommandStatus::PermissionDenied);
    }
}
