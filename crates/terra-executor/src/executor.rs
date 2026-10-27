use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use tracing::{info, warn};

use terra_caging::cage::{CageAction, CageContext};
use terra_caging::engine::CagingEngine;
use terra_memory::MemoryDatabase;
use terra_providers::adapter::ProviderAdapter;
use terra_trace::AuditLog;
use terra_types::command::{Command, CommandOp, CommandResult};

use crate::isolation::Isolation;
use crate::router;

/// What the cages need to know about the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionMode {
    pub goal: bool,
    /// Whether the active goal's spec has no open questions.
    pub spec_complete: bool,
}

impl SessionMode {
    pub const CONVERSATION: SessionMode = SessionMode { goal: false, spec_complete: true };
}

/// Generate a short status text for a CommandOp (for the event bus).
fn status_text(op: &CommandOp) -> String {
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    match op {
        CommandOp::FileRead { path } => format!("Reading {}", base(path)),
        CommandOp::FileWrite { path, .. } => format!("Writing {}", base(path)),
        CommandOp::FileDelete { path } => format!("Deleting {}", base(path)),
        CommandOp::ShellExec { command, .. } => format!("Running: {}", terra_types::text::truncate_ellipsis(command, 40)),
        CommandOp::ProviderCall { .. } => "Thinking...".into(),
        CommandOp::MemRead { key } => format!("Reading memory '{}'", key),
        CommandOp::MemWrite { key, .. } => format!("Writing memory '{}'", key),
        CommandOp::MemSearch { .. } => "Searching memory...".into(),
        CommandOp::TraceEvent { event_type, .. } => format!("Event: {}", event_type),
        CommandOp::ExecuteTask { .. } => "Checking the task spec".into(),
        CommandOp::ClaimComplete { .. } => "Checking the completion claim".into(),
    }
}

/// The single gateway between agents and the world.
///
/// Every command: audit (before) → cages → execute → audit (after).
/// A blocked command is audited and returns `PermissionDenied` without
/// running.
pub struct CommandExecutor {
    memory: Arc<MemoryDatabase>,
    audit: Arc<AuditLog>,
    caging: Arc<CagingEngine>,
    provider: Arc<dyn ProviderAdapter>,
    event_bus: Arc<dyn terra_types::EventBus>,
    mode: RwLock<SessionMode>,
    isolation: Isolation,
    project_root: PathBuf,
}

impl CommandExecutor {
    pub fn new(
        memory: Arc<MemoryDatabase>,
        audit: Arc<AuditLog>,
        caging: Arc<CagingEngine>,
        provider: Arc<dyn ProviderAdapter>,
    ) -> Self {
        Self {
            memory,
            audit,
            caging,
            provider,
            event_bus: Arc::new(terra_types::NullBus),
            mode: RwLock::new(SessionMode::CONVERSATION),
            isolation: Isolation::None,
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    /// Attach an event bus for operation visibility.
    pub fn with_event_bus(mut self, bus: Arc<dyn terra_types::EventBus>) -> Self {
        self.event_bus = bus;
        self
    }

    /// Confine shell commands with this isolation.
    pub fn with_isolation(mut self, isolation: Isolation) -> Self {
        self.isolation = isolation;
        self
    }

    /// Working directory for shell commands and relative paths.
    pub fn with_project_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.project_root = root.into();
        self
    }

    pub fn provider(&self) -> &Arc<dyn ProviderAdapter> {
        &self.provider
    }

    pub fn audit(&self) -> &Arc<AuditLog> {
        &self.audit
    }

    pub fn memory(&self) -> &Arc<MemoryDatabase> {
        &self.memory
    }

    pub fn isolation(&self) -> &Isolation {
        &self.isolation
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Tell the cages which mode the session is in.
    pub fn set_mode(&self, mode: SessionMode) {
        *self.mode.write().unwrap_or_else(|e| e.into_inner()) = mode;
    }

    pub fn mode(&self) -> SessionMode {
        *self.mode.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Execute a command through the full pipeline.
    pub async fn execute(&self, cmd: Command) -> CommandResult {
        let start = Instant::now();
        let agent = cmd.agent.clone();
        let cmd_id = cmd.id.clone();

        // 1. Audit before execution
        let seq = self.audit.record_command(cmd.clone());

        // 2. Cages
        let context = self.cage_context(&cmd.op, &agent);
        let verdict = self.caging.check(&context);
        if !verdict.allowed {
            let (cage, reason) = verdict.block.unwrap_or(("unknown".into(), "blocked".into()));
            if matches!(cmd.op, CommandOp::ExecuteTask { .. } | CommandOp::ClaimComplete { .. }) {
                info!(agent = %agent, cage = %cage, reason = %reason, "gate blocked by cage");
            } else {
                warn!(agent = %agent, cage = %cage, reason = %reason, "command blocked by cage");
            }
            let result = CommandResult::denied(cmd_id, format!("cage '{}' blocked: {}", cage, reason));
            self.audit.record_result(seq, result.clone(), start.elapsed().as_micros() as u64);
            return result;
        }
        for (cage, warning) in &verdict.warnings {
            info!(cage = %cage, warning = %warning, "cage warning");
        }

        // 3. Execute
        self.event_bus.emit(terra_types::StreamEvent::step_start(cmd.op.name(), &status_text(&cmd.op)));
        let mut result = router::route(&cmd.op, &agent, self).await;
        result.command_id = cmd_id.clone();

        // 4. Output cage on model answers
        if matches!(cmd.op, CommandOp::ProviderCall { .. }) {
            if let Some(content) = result.data.as_ref().and_then(|d| d.get("content")).and_then(|c| c.as_str()) {
                match terra_caging::check_output(content) {
                    terra_caging::OutputCheckResult::Pass => {}
                    terra_caging::OutputCheckResult::Warn(reason) => {
                        warn!(agent = %agent, reason = %reason, "output cage warning");
                    }
                    terra_caging::OutputCheckResult::Block(reason) => {
                        warn!(agent = %agent, reason = %reason, "output cage blocked response");
                        result = CommandResult::denied(cmd_id, format!("output cage blocked: {}", reason));
                    }
                }
            }
        }

        // 5. Audit after execution
        let elapsed = start.elapsed().as_micros() as u64;
        self.audit.record_result(seq, result.clone(), elapsed);
        self.event_bus.emit(terra_types::StreamEvent::step_complete(cmd.op.name()));
        info!(agent = %agent, seq, status = ?result.status, duration_us = elapsed, "command executed");
        result
    }

    /// Record a command the user declined at the confirmation prompt. It
    /// never ran, but the attempt is part of the record.
    pub fn record_declined(&self, cmd: Command) -> CommandResult {
        let seq = self.audit.record_command(cmd.clone());
        let result = CommandResult::denied(cmd.id, "declined by user at the confirmation prompt");
        self.audit.record_result(seq, result.clone(), 0);
        result
    }

    /// Record a model call the agent loop made directly (it needs tools, which
    /// `ProviderCall` does not carry), so every call is in the audit log.
    pub fn audit_llm_call(&self, agent: &str, prompt_tokens: u32, completion_tokens: u32, tool_calls: usize, iteration: u32) {
        let cmd = Command::new(
            agent,
            CommandOp::ProviderCall {
                system_prompt: format!("[agent_loop iter={}]", iteration),
                user_prompt: format!("[{} tool_calls]", tool_calls),
                max_tokens: None,
                temperature: None,
                json: false,
            },
        );
        let seq = self.audit.record_command(cmd);
        let result = CommandResult::ok(
            "",
            Some(serde_json::json!({
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "tool_calls": tool_calls,
                "iteration": iteration,
                "source": "agent_loop",
            })),
        );
        self.audit.record_result(seq, result, 0);
    }

    fn cage_context(&self, op: &CommandOp, agent: &str) -> CageContext {
        let action = match op {
            CommandOp::MemWrite { key, level, .. } => {
                let owner = self.memory.owner_of(key).ok().flatten().unwrap_or_else(|| agent.to_string());
                CageAction::MemoryWrite {
                    key: key.clone(),
                    owner,
                    level: level.clone(),
                    is_user_command: agent == "user",
                }
            }
            CommandOp::MemRead { .. } | CommandOp::MemSearch { .. } | CommandOp::TraceEvent { .. } | CommandOp::ProviderCall { .. } => {
                CageAction::Internal
            }
            CommandOp::FileRead { path } => CageAction::FileRead { path: self.absolute(path) },
            CommandOp::FileWrite { path, .. } => CageAction::FileWrite { path: self.absolute(path) },
            CommandOp::FileDelete { path } => CageAction::FileDelete { path: self.absolute(path) },
            CommandOp::ShellExec { command, .. } => CageAction::ShellExec { command: command.clone() },
            CommandOp::ExecuteTask { open_questions, .. } => CageAction::ExecuteTask { spec_complete: open_questions.is_empty() },
            CommandOp::ClaimComplete { verified, .. } => CageAction::ClaimComplete { verified: *verified },
        };
        let mode = self.mode();
        CageContext { agent: agent.to_string(), action, goal_mode: mode.goal, spec_complete: mode.spec_complete }
    }

    /// Resolve a path the way the router will open it.
    pub(crate) fn absolute(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.project_root.join(p)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_types::command::CommandStatus;
    use terra_providers::adapter::DryRunAdapter;

    fn test_executor() -> (CommandExecutor, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".terra")).unwrap();
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let audit = Arc::new(AuditLog::new());
        let provider: Arc<dyn ProviderAdapter> = Arc::new(DryRunAdapter::new());
        let sandbox = SandboxGuard::new(root.clone(), vec![root.join(".terra/constitution.toml")]);
        let caging = Arc::new(CagingEngine::standard(sandbox, true, true));
        let exec = CommandExecutor::new(memory, audit, caging, provider).with_project_root(root);
        (exec, dir)
    }

    fn write_cmd(key: &str, agent: &str) -> Command {
        Command::new(
            agent,
            CommandOp::MemWrite { key: key.into(), content: "test data".into(), scope: "project".into(), level: "L2".into() },
        )
    }

    fn read_cmd(key: &str) -> Command {
        Command::new("main", CommandOp::MemRead { key: key.into() })
    }

    #[tokio::test]
    async fn audit_before_and_after_execute() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("key1", "main")).await;
        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].duration_us.is_some());
        assert_eq!(entries[0].result.as_ref().unwrap().status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn memory_ownership_enforced_and_blocked_commands_audited() {
        let (exec, _dir) = test_executor();
        exec.execute(write_cmd("owned_key", "main")).await;
        let result = exec.execute(write_cmd("owned_key", "attacker")).await;
        assert_eq!(result.status, CommandStatus::PermissionDenied);
        let read = exec.execute(read_cmd("owned_key")).await;
        assert_eq!(read.data.unwrap()["owner"], "main");
        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].result.as_ref().unwrap().status, CommandStatus::PermissionDenied);
    }

    #[tokio::test]
    async fn file_write_read_delete_inside_project() {
        let (exec, dir) = test_executor();
        let result = exec
            .execute(Command::new("main", CommandOp::FileWrite { path: "notes/test_file.txt".into(), content: "hello terra".into() }))
            .await;
        assert_eq!(result.status, CommandStatus::Ok, "{:?}", result.error);
        assert_eq!(result.data.as_ref().unwrap()["bytes_written"], 11);
        assert!(dir.path().join("notes/test_file.txt").exists(), "relative paths resolve against the project root");

        let read = exec.execute(Command::new("main", CommandOp::FileRead { path: "notes/test_file.txt".into() })).await;
        assert_eq!(read.data.unwrap()["content"], "hello terra");

        let del = exec.execute(Command::new("main", CommandOp::FileDelete { path: "notes/test_file.txt".into() })).await;
        assert_eq!(del.status, CommandStatus::Ok);
        assert!(!dir.path().join("notes/test_file.txt").exists());
    }

    #[tokio::test]
    async fn writes_outside_and_to_the_constitution_are_blocked() {
        let (exec, dir) = test_executor();
        let outside = dir.path().parent().unwrap().join("terra_exec_escape.txt");
        let result = exec
            .execute(Command::new("main", CommandOp::FileWrite { path: outside.to_string_lossy().into(), content: "x".into() }))
            .await;
        assert_eq!(result.status, CommandStatus::PermissionDenied);
        assert!(result.error.unwrap().contains("cage 'action'"));
        assert!(!outside.exists());

        let result = exec
            .execute(Command::new("main", CommandOp::FileWrite { path: ".terra/constitution.toml".into(), content: "x".into() }))
            .await;
        assert!(result.error.unwrap().contains("cage 'meta'"));
    }

    #[tokio::test]
    async fn file_read_nonexistent_is_an_error() {
        let (exec, _dir) = test_executor();
        let result = exec.execute(Command::new("main", CommandOp::FileRead { path: "/nonexistent/path/file.txt".into() })).await;
        assert_eq!(result.status, CommandStatus::Error);
    }

    #[tokio::test]
    async fn shell_exec_runs_in_project_root_and_reports_exit() {
        let (exec, dir) = test_executor();
        let result = exec
            .execute(Command::new("main", CommandOp::ShellExec { command: "pwd && echo hello".into(), workdir: None, timeout_ms: Some(5000) }))
            .await;
        assert_eq!(result.status, CommandStatus::Ok, "{:?}", result.error);
        let data = result.data.unwrap();
        let stdout = data["stdout"].as_str().unwrap();
        assert!(stdout.contains("hello"));
        let cwd = std::fs::canonicalize(dir.path()).unwrap();
        assert!(stdout.contains(cwd.to_str().unwrap()), "stdout: {stdout}");
        assert_eq!(data["isolation"], "none");

        let fail = exec.execute(Command::new("main", CommandOp::ShellExec { command: "exit 1".into(), workdir: None, timeout_ms: None })).await;
        assert_eq!(fail.status, CommandStatus::Error);

        let slow = exec
            .execute(Command::new("main", CommandOp::ShellExec { command: "sleep 10".into(), workdir: None, timeout_ms: Some(200) }))
            .await;
        assert_eq!(slow.status, CommandStatus::Error);
        assert!(slow.error.unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn goal_mode_without_spec_blocks_changes_until_complete() {
        let (exec, _dir) = test_executor();
        exec.set_mode(SessionMode { goal: true, spec_complete: false });
        let write = || Command::new("main", CommandOp::FileWrite { path: "a.txt".into(), content: "x".into() });
        let blocked = exec.execute(write()).await;
        assert!(blocked.error.unwrap().contains("cage 'specification'"));
        let read = exec.execute(Command::new("main", CommandOp::FileRead { path: "Cargo.toml".into() })).await;
        assert_ne!(read.status, CommandStatus::PermissionDenied, "reads are allowed while specifying");

        exec.set_mode(SessionMode { goal: true, spec_complete: true });
        assert_eq!(exec.execute(write()).await.status, CommandStatus::Ok);
    }

    #[tokio::test]
    async fn completion_claims_go_through_the_verification_cage() {
        let (exec, _dir) = test_executor();
        let claim = |verified: bool| Command::new("session", CommandOp::ClaimComplete { goal: "g".into(), verified, evidence: "e".into() });
        exec.set_mode(SessionMode { goal: true, spec_complete: true });
        assert_eq!(exec.execute(claim(false)).await.status, CommandStatus::PermissionDenied);
        assert_eq!(exec.execute(claim(true)).await.status, CommandStatus::Ok);
        exec.set_mode(SessionMode::CONVERSATION);
        assert_eq!(exec.execute(claim(false)).await.status, CommandStatus::Ok, "goal-only cage is off in conversation");
    }

    #[tokio::test]
    async fn provider_call_and_declined_commands_are_audited() {
        let (exec, _dir) = test_executor();
        let cmd = Command::new(
            "main",
            CommandOp::ProviderCall { system_prompt: "You are helpful.".into(), user_prompt: "What is 2+2?".into(), max_tokens: Some(50), temperature: None, json: false },
        );
        let result = exec.execute(cmd).await;
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.data.unwrap()["content"].as_str().unwrap().contains("DRY_RUN"));

        exec.audit_llm_call("main", 100, 50, 2, 1);
        let declined = exec.record_declined(Command::new("main", CommandOp::ShellExec { command: "rm x".into(), workdir: None, timeout_ms: None }));
        assert_eq!(declined.status, CommandStatus::PermissionDenied);
        let entries = exec.audit.entries();
        assert_eq!(entries.len(), 3);
        assert!(entries[2].result.as_ref().unwrap().error.as_ref().unwrap().contains("declined"));
    }

    #[tokio::test]
    async fn mem_search_goes_through_the_database() {
        let (exec, _dir) = test_executor();
        exec.execute(Command::new("main", CommandOp::MemWrite { key: "jwt".into(), content: "auth uses JWT tokens".into(), scope: "project".into(), level: "L2".into() })).await;
        let found = exec.execute(Command::new("main", CommandOp::MemSearch { query: "JWT".into(), top_k: 5 })).await;
        assert_eq!(found.data.unwrap()["results"][0]["key"], "jwt");
    }
}
