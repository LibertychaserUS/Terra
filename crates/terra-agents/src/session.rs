//! Goal Mode: a specified goal, the verification state read from the audit
//! log, and the completion claim that the Verification cage judges.

use std::sync::Arc;

use terra_executor::{CommandExecutor, SessionMode as ExecMode};
use terra_trace::AuditEntry;
use terra_types::command::{Command, CommandOp, CommandStatus};

use crate::spec::Spec;

/// Agent name under which `/verify` runs its command. Only a shell command
/// run by this agent counts as verification; the agent running the tests
/// itself is a change like any other.
pub const VERIFIER_AGENT: &str = "verifier";

/// Whether the active goal's work has been verified, read from the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// Nothing was written, deleted or run in the shell during the goal.
    NoChanges,
    /// Writes, deletes or shell commands not followed by a passing `/verify`.
    Unverified { changes: u32 },
    /// The last `/verify` passed and nothing changed after it.
    Passed { command: String },
    /// The last `/verify` failed.
    Failed { command: String },
}

impl Verification {
    /// Fold audit entries (oldest first) into a verification state.
    ///
    /// A change is a successful file write or delete, or any shell command the
    /// agent ran (it may have edited files even if it failed). A `/verify` run
    /// resets the state to its outcome; one blocked by a cage never ran.
    pub fn from_audit(entries: &[AuditEntry]) -> Self {
        entries.iter().fold(Verification::NoChanges, |state, entry| {
            let status = entry.result.as_ref().map(|r| &r.status);
            match (&entry.command.op, status) {
                (CommandOp::ShellExec { command, .. }, Some(status)) if entry.command.agent == VERIFIER_AGENT => match status {
                    CommandStatus::Ok => Verification::Passed { command: command.clone() },
                    CommandStatus::PermissionDenied => state,
                    _ => Verification::Failed { command: command.clone() },
                },
                (CommandOp::FileWrite { .. } | CommandOp::FileDelete { .. }, Some(CommandStatus::Ok))
                | (CommandOp::ShellExec { .. }, Some(CommandStatus::Ok | CommandStatus::Error)) => match state {
                    Verification::Unverified { changes } => Verification::Unverified { changes: changes + 1 },
                    _ => Verification::Unverified { changes: 1 },
                },
                _ => state,
            }
        })
    }

    /// Whether a completion claim may rest on this state.
    pub fn is_verified(&self) -> bool {
        matches!(self, Verification::NoChanges | Verification::Passed { .. })
    }

    /// One line for the user and the audit log.
    pub fn describe(&self) -> String {
        match self {
            Verification::NoChanges => "nothing was written, deleted or run in the shell during the goal".into(),
            Verification::Unverified { changes: 1 } => "1 write, delete or shell command is not followed by a passing /verify".into(),
            Verification::Unverified { changes } => {
                format!("{} writes, deletes or shell commands are not followed by a passing /verify", changes)
            }
            Verification::Passed { command } => format!("verified by `{}`", command),
            Verification::Failed { command } => format!("the last /verify (`{}`) failed", command),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionMode {
    Conversation,
    Goal {
        spec: Spec,
        /// Audit sequence number when the goal started.
        audit_start: u64,
    },
}

impl SessionMode {
    pub fn is_goal(&self) -> bool {
        matches!(self, SessionMode::Goal { .. })
    }

    pub fn spec(&self) -> Option<&Spec> {
        match self {
            SessionMode::Goal { spec, .. } => Some(spec),
            SessionMode::Conversation => None,
        }
    }
}

/// Tracks the mode and keeps the executor's cages informed of it.
pub struct Session {
    executor: Arc<CommandExecutor>,
    mode: SessionMode,
}

impl Session {
    pub fn new(executor: Arc<CommandExecutor>) -> Self {
        executor.set_mode(ExecMode::CONVERSATION);
        Self { executor, mode: SessionMode::Conversation }
    }

    pub fn mode(&self) -> &SessionMode {
        &self.mode
    }

    pub fn spec(&self) -> Option<&Spec> {
        self.mode.spec()
    }

    fn sync_executor(&self) {
        let mode = match &self.mode {
            SessionMode::Conversation => ExecMode::CONVERSATION,
            SessionMode::Goal { spec, .. } => ExecMode { goal: true, spec_complete: spec.is_complete() },
        };
        self.executor.set_mode(mode);
    }

    /// Enter Goal Mode with a specified goal. Fails when a goal is active.
    pub async fn enter_goal(&mut self, description: &str) -> Result<&Spec, String> {
        if self.mode.is_goal() {
            return Err("already in Goal Mode; /done or /abandon first".into());
        }
        let spec = Spec::compile(&self.executor, description).await;
        self.audit_event("mode_switch", serde_json::json!({
            "from": "conversation", "to": "goal",
            "spec_id": spec.spec_id, "intent": spec.intent, "open_questions": spec.open_questions,
        }))
        .await;
        self.mode = SessionMode::Goal { spec, audit_start: self.executor.audit().last_seq() };
        self.sync_executor();
        Ok(self.mode.spec().expect("goal mode"))
    }

    /// Answer the goal's open questions: the description grows and the spec
    /// is compiled again. The goal's audit window is unchanged.
    pub async fn refine_goal(&mut self, answer: &str) -> Result<&Spec, String> {
        let (description, audit_start) = match &self.mode {
            SessionMode::Goal { spec, audit_start } => (format!("{}\n{}", spec.description, answer), *audit_start),
            SessionMode::Conversation => return Err("not in Goal Mode".into()),
        };
        let spec = Spec::compile(&self.executor, &description).await;
        self.audit_event("spec_refined", serde_json::json!({
            "spec_id": spec.spec_id, "intent": spec.intent, "open_questions": spec.open_questions,
        }))
        .await;
        self.mode = SessionMode::Goal { spec, audit_start };
        self.sync_executor();
        Ok(self.mode.spec().expect("goal mode"))
    }

    /// Verification state of the active goal; None outside Goal Mode.
    pub fn verification(&self) -> Option<Verification> {
        match &self.mode {
            SessionMode::Goal { audit_start, .. } => {
                Some(Verification::from_audit(&self.executor.audit().entries_after(*audit_start)))
            }
            SessionMode::Conversation => None,
        }
    }

    /// Audit entries recorded since the goal started.
    pub fn goal_entries(&self) -> Vec<AuditEntry> {
        match &self.mode {
            SessionMode::Goal { audit_start, .. } => self.executor.audit().entries_after(*audit_start),
            SessionMode::Conversation => Vec::new(),
        }
    }

    /// Claim the goal is complete (`/done`). The claim is a command: the
    /// Verification cage decides and the audit log records it. On success
    /// the session is back in Conversation Mode.
    pub async fn complete_goal(&mut self) -> Result<Verification, String> {
        let (goal, verification) = match (&self.mode, self.verification()) {
            (SessionMode::Goal { spec, .. }, Some(v)) => (spec.intent.clone(), v),
            _ => return Err("not in Goal Mode".into()),
        };
        let cmd = Command::new(
            "session",
            CommandOp::ClaimComplete { goal, verified: verification.is_verified(), evidence: verification.describe() },
        );
        let result = self.executor.execute(cmd).await;
        if result.status != CommandStatus::Ok {
            return Err(result.error.unwrap_or_else(|| "completion claim rejected".into()));
        }
        self.exit_goal().await;
        Ok(verification)
    }

    /// Leave Goal Mode without claiming completion (`/abandon`).
    pub async fn abandon_goal(&mut self) -> Result<(), String> {
        let verification = self.verification().ok_or("not in Goal Mode")?;
        self.audit_event("goal_abandoned", serde_json::json!({ "verification": verification.describe() })).await;
        self.exit_goal().await;
        Ok(())
    }

    async fn exit_goal(&mut self) {
        self.audit_event("mode_switch", serde_json::json!({ "from": "goal", "to": "conversation" })).await;
        let cleaned = self.executor.memory().cleanup_scope(terra_memory::Scope::Task).unwrap_or(0);
        if cleaned > 0 {
            tracing::info!(cleaned, "task-scoped memory entries cleaned on goal exit");
        }
        self.mode = SessionMode::Conversation;
        self.sync_executor();
    }

    pub async fn audit_event(&self, event_type: &str, data: serde_json::Value) {
        let cmd = Command::new("session", CommandOp::TraceEvent { event_type: event_type.to_string(), data });
        self.executor.execute(cmd).await;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use terra_caging::engine::CagingEngine;
    use terra_caging::sandbox::SandboxGuard;
    use terra_memory::MemoryDatabase;
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;
    use terra_types::command::CommandResult;

    pub(crate) fn test_executor() -> (Arc<CommandExecutor>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let memory = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let caging = Arc::new(CagingEngine::standard(SandboxGuard::new(root.clone(), vec![]), true, true));
        let exec = CommandExecutor::new(memory, Arc::new(AuditLog::new()), caging, Arc::new(DryRunAdapter::new()))
            .with_project_root(root);
        (Arc::new(exec), dir)
    }

    fn entry(seq: u64, agent: &str, op: CommandOp, status: CommandStatus) -> AuditEntry {
        let result = CommandResult { status, ..CommandResult::ok("", None) };
        AuditEntry { seq, timestamp: chrono::Utc::now(), command: Command::new(agent, op), result: Some(result), duration_us: Some(1) }
    }

    fn write_op() -> CommandOp {
        CommandOp::FileWrite { path: "a.txt".into(), content: "x".into() }
    }

    fn shell_op(cmd: &str) -> CommandOp {
        CommandOp::ShellExec { command: cmd.into(), workdir: None, timeout_ms: None }
    }

    #[test]
    fn verification_folds_the_audit_log() {
        use CommandStatus::*;
        let state = |entries: &[AuditEntry]| Verification::from_audit(entries);
        assert_eq!(state(&[]), Verification::NoChanges);
        assert_eq!(state(&[entry(1, "main", write_op(), Ok)]), Verification::Unverified { changes: 1 });
        assert_eq!(
            state(&[entry(1, "main", write_op(), Ok), entry(2, "main", shell_op("cargo test"), Ok)]),
            Verification::Unverified { changes: 2 },
            "the agent running tests is a change, not verification"
        );
        assert_eq!(
            state(&[entry(1, "main", write_op(), Ok), entry(2, VERIFIER_AGENT, shell_op("cargo test"), Ok)]),
            Verification::Passed { command: "cargo test".into() }
        );
        assert_eq!(
            state(&[entry(1, VERIFIER_AGENT, shell_op("cargo test"), Error)]),
            Verification::Failed { command: "cargo test".into() }
        );
        assert_eq!(state(&[entry(1, VERIFIER_AGENT, shell_op("mkfs"), PermissionDenied)]), Verification::NoChanges);
        assert_eq!(
            state(&[entry(1, VERIFIER_AGENT, shell_op("cargo test"), Ok), entry(2, "main", write_op(), Ok)]),
            Verification::Unverified { changes: 1 },
            "changes after a passing verify need a new one"
        );
    }

    #[tokio::test]
    async fn goal_lifecycle_through_the_cages() {
        let (exec, _dir) = test_executor();
        let mut session = Session::new(exec.clone());
        assert!(!exec.mode().goal);

        let spec = session.enter_goal("say hi").await.unwrap();
        assert!(!spec.is_complete());
        assert!(exec.mode().goal && !exec.mode().spec_complete);
        assert!(session.enter_goal("again").await.is_err());

        let spec = session.refine_goal("write it to notes.txt").await.unwrap();
        assert!(spec.is_complete(), "{:?}", spec.open_questions);
        assert!(exec.mode().spec_complete);

        // The agent writes; the claim is blocked until a verifier run passes.
        exec.execute(Command::new("main", write_op())).await;
        assert_eq!(session.verification(), Some(Verification::Unverified { changes: 1 }));
        let err = session.complete_goal().await.unwrap_err();
        assert!(err.contains("cage 'verification'"), "{err}");
        assert!(session.mode().is_goal());

        exec.execute(Command::new(VERIFIER_AGENT, shell_op("true"))).await;
        let verification = session.complete_goal().await.unwrap();
        assert!(matches!(verification, Verification::Passed { .. }));
        assert!(!session.mode().is_goal());
        assert!(!exec.mode().goal);
    }

    #[tokio::test]
    async fn abandon_leaves_goal_mode_without_a_claim() {
        let (exec, _dir) = test_executor();
        let mut session = Session::new(exec.clone());
        session.enter_goal("fix src/lib.rs").await.unwrap();
        exec.execute(Command::new("main", write_op())).await;
        session.abandon_goal().await.unwrap();
        assert!(!session.mode().is_goal());
        let ops: Vec<&str> = exec.audit().entries().iter().map(|e| e.command.op.name()).collect();
        assert!(!ops.contains(&"ClaimComplete"));
    }
}
