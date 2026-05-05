use std::sync::Arc;
use std::time::Instant;

use tracing::{info, warn};

use terra_caging::cage::{CageAction, CageContext};
use terra_caging::engine::CagingEngine;
use terra_memory::MemoryDatabase;
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
    audit: Arc<AuditLog>,
    caging: Arc<CagingEngine>,
}

impl CommandExecutor {
    pub fn new(
        memory: Arc<MemoryDatabase>,
        audit: Arc<AuditLog>,
        caging: Arc<CagingEngine>,
    ) -> Self {
        Self { memory, audit, caging }
    }

    /// Execute a command through the full pipeline.
    pub fn execute(&self, cmd: Command) -> CommandResult {
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
        let mut result = router::route(&cmd.op, &agent, &self.memory);
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

    fn test_executor() -> CommandExecutor {
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let audit = Arc::new(AuditLog::new());

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

        CommandExecutor::new(memory, audit, caging)
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

    // T-PI-008: audit entry timestamp before execution
    #[test]
    fn audit_before_execute() {
        let exec = test_executor();
        let cmd = write_cmd("key1", "main");
        exec.execute(cmd);

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].duration_us.is_some()); // duration recorded after
    }

    // T-PI-009: cage check before execute
    #[test]
    fn cage_check_before_execute() {
        let exec = test_executor();
        // Write by "main", then try write by "attacker" (non-owner)
        exec.execute(write_cmd("owned_key", "main"));
        let result = exec.execute(write_cmd("owned_key", "attacker"));

        // Memory cage should block non-owner write
        assert_eq!(result.status, CommandStatus::PermissionDenied);
        assert!(result.error.as_ref().unwrap().contains("memory"));
    }

    // T-PI-010: cage block prevents write
    #[test]
    fn cage_block_prevents_write() {
        let exec = test_executor();
        exec.execute(write_cmd("protected", "main"));

        // Non-owner attempt
        let result = exec.execute(write_cmd("protected", "hacker"));
        assert_eq!(result.status, CommandStatus::PermissionDenied);

        // Original data unchanged (hacker's write didn't go through)
        let read_result = exec.execute(read_cmd("protected"));
        assert_eq!(read_result.status, CommandStatus::Ok);
        let data = read_result.data.unwrap();
        assert_eq!(data["owner"], "main"); // still owned by main
    }

    // T-PI-011: mem commands route to memory
    #[test]
    fn routes_mem_to_memory() {
        let exec = test_executor();
        let result = exec.execute(write_cmd("routed", "main"));
        assert_eq!(result.status, CommandStatus::Ok);

        let read = exec.execute(read_cmd("routed"));
        assert_eq!(read.status, CommandStatus::Ok);
        assert_eq!(read.data.unwrap()["key"], "routed");
    }

    // T-PI-013: trace commands route to audit
    #[test]
    fn routes_trace_to_audit() {
        let exec = test_executor();
        let cmd = Command::new(
            "main",
            CommandOp::TraceCheckpoint {
                title: "test checkpoint".into(),
                summary: "testing routing".into(),
            },
        );
        let result = exec.execute(cmd);
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(exec.audit.len() > 0);
    }

    // T-PI-015: result has duration
    #[test]
    fn result_has_duration() {
        let exec = test_executor();
        exec.execute(write_cmd("timed", "main"));

        let entries = exec.audit.entries();
        assert!(entries[0].duration_us.is_some());
        assert!(entries[0].duration_us.unwrap() > 0);
    }

    // T-PI-016: permission denied on cage block
    #[test]
    fn permission_denied_on_block() {
        let exec = test_executor();
        exec.execute(write_cmd("owned", "alice"));

        let result = exec.execute(write_cmd("owned", "bob"));
        assert_eq!(result.status, CommandStatus::PermissionDenied);
        assert!(result.error.is_some());
    }

    // Audit records both command and result
    #[test]
    fn audit_records_command_and_result() {
        let exec = test_executor();
        exec.execute(write_cmd("audited", "main"));

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].result.is_some());
        assert_eq!(entries[0].result.as_ref().unwrap().status, CommandStatus::Ok);
    }

    // Multiple commands sequential
    #[test]
    fn multiple_commands_sequential() {
        let exec = test_executor();
        exec.execute(write_cmd("a", "main"));
        exec.execute(write_cmd("b", "main"));
        exec.execute(write_cmd("c", "main"));

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].seq, 1);
        assert_eq!(entries[1].seq, 2);
        assert_eq!(entries[2].seq, 3);
    }

    // Blocked command still gets audit entry
    #[test]
    fn blocked_command_audited() {
        let exec = test_executor();
        exec.execute(write_cmd("x", "owner"));
        exec.execute(write_cmd("x", "attacker")); // blocked

        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 2);
        // Second entry should have PermissionDenied result
        let blocked = &entries[1];
        assert_eq!(
            blocked.result.as_ref().unwrap().status,
            CommandStatus::PermissionDenied
        );
    }
}
