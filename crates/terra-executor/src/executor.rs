use std::sync::Arc;
use std::time::Instant;

use tracing::{info, warn};

use terra_caging::cage::{CageAction, CageContext};
use terra_caging::engine::CagingEngine;
use terra_memory::{MemoryDatabase, ArtifactStore, VectorIndex};
use terra_protocol::command::{Command, CommandOp, CommandResult, CommandStatus};
use terra_trace::AuditLog;

use crate::router;

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
        }
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
            };

            let elapsed = start.elapsed().as_micros() as u64;
            self.audit.record_result(seq, result.clone(), elapsed);
            return result;
        }

        // Log any warnings
        for (cage_name, warning) in &cage_result.warnings {
            warn!(cage = %cage_name, warning = %warning, "cage warning");
        }

        // 3. Execute via router
        let mut result = router::route(
            &cmd.op, &agent, &self.memory, &self.artifacts,
            &self.search_index, &self.provider,
        ).await;
        result.command_id = cmd_id;

        // 4. Audit: record result with duration
        let elapsed = start.elapsed().as_micros() as u64;
        self.audit.record_result(seq, result.clone(), elapsed);

        info!(
            agent = %agent,
            seq = seq,
            status = ?result.status,
            duration_us = elapsed,
            "command executed"
        );

        result
    }

    /// Map a CommandOp to a CageContext for cage checking.
    fn command_to_cage_context(&self, op: &CommandOp, agent: &str) -> CageContext {
        let action = match op {
            CommandOp::MemWrite { key, .. } => {
                // Check if agent is owner of existing key
                let owner = self.memory.read(key)
                    .ok()
                    .flatten()
                    .map(|e| e.owner.clone())
                    .unwrap_or_else(|| agent.to_string());

                CageAction::MemoryWrite {
                    key: key.clone(),
                    owner,
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
        };

        CageContext {
            agent: agent.to_string(),
            action,
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
            PathBuf::from("/projects/test"),
            vec![PathBuf::from("/projects/test/.terra/constitution.toml")],
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
                level: "L1".into(),
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
}
