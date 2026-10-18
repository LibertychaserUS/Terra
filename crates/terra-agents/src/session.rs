//! Session Manager + Sleep Consolidation
//!
//! Dual mode session lifecycle:
//! - Conversation Mode (default): free interaction, lightweight constraints
//! - Goal Mode (/goal): adds drift detection + spec gating on top of conversation
//!   - User can still talk freely — Goal Mode is additive, not restrictive
//!   - The difference: drift is monitored, `/execute` is gated on the goal spec,
//!     and `/done` is a completion claim the Verification cage checks against
//!     the audit log (changes made during the goal need a passing `/verify`)
//!
//! Sleep Consolidation:
//! - Post-session abstraction of concrete actions into general principles
//! - Generates L6 promotion candidates for user review
//! - User accept → promote to L1 (project knowledge)
//! - User reject → discard

use std::collections::HashMap;
use std::sync::Arc;

use terra_compile::{SpecIR, TermTable};
use terra_executor::CommandExecutor;
use terra_protocol::command::{Command, CommandOp, CommandStatus};
use terra_trace::AuditEntry;

use crate::drift::{DriftCheckResult, DriftDetector};

/// Runtime mode — determines which cages and checks are active.
///
/// Goal Mode is additive over Conversation: user can still talk freely,
/// but spec alignment and drift detection are active in the background.
// One SessionMode per session, so the size difference between variants is irrelevant.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum SessionMode {
    /// Free interaction. Only always-on cages (Meta, Memory, Action, Knowledge).
    Conversation,
    /// Goal-driven with conversation. Full pipeline + drift detection.
    /// User can still converse — the mode adds monitoring, not restrictions.
    Goal {
        spec: SpecIR,
        drift_score: f64,
        command_count: u32,
    },
}

impl SessionMode {
    pub fn is_goal(&self) -> bool {
        matches!(self, SessionMode::Goal { .. })
    }

    pub fn is_conversation(&self) -> bool {
        matches!(self, SessionMode::Conversation)
    }
}

/// Agent name of commands run by `/verify`. LLM tool calls run as "main", so a
/// passing command under this name is Terra's own check, not the agent vouching
/// for its work.
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
                (CommandOp::ShellExec { command, .. }, Some(status))
                    if entry.command.agent == VERIFIER_AGENT =>
                {
                    match status {
                        CommandStatus::Ok => Verification::Passed { command: command.clone() },
                        CommandStatus::PermissionDenied => state,
                        _ => Verification::Failed { command: command.clone() },
                    }
                }
                (CommandOp::FileWrite { .. } | CommandOp::FileDelete { .. }, Some(CommandStatus::Ok))
                | (CommandOp::ShellExec { .. }, Some(CommandStatus::Ok | CommandStatus::Error)) => {
                    match state {
                        Verification::Unverified { changes } => Verification::Unverified { changes: changes + 1 },
                        _ => Verification::Unverified { changes: 1 },
                    }
                }
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
            Verification::Unverified { changes: 1 } => {
                "1 write, delete or shell command is not followed by a passing /verify".into()
            }
            Verification::Unverified { changes } => {
                format!("{} writes, deletes or shell commands are not followed by a passing /verify", changes)
            }
            Verification::Passed { command } => format!("verified by `{}`", command),
            Verification::Failed { command } => format!("the last /verify (`{}`) failed", command),
        }
    }
}

/// Session state — tracks project and lifecycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Active { project: String },
}

/// The Session Manager.
pub struct SessionManager {
    executor: Arc<CommandExecutor>,
    state: SessionState,
    mode: SessionMode,
    session_actions: Vec<SessionAction>,
    goal_key: Option<String>,
    sleep_settings: SleepSettings,
    drift_detector: Option<DriftDetector>,
    /// Audit sequence number when Goal Mode was entered; verification is read
    /// from the entries after it.
    goal_audit_start: u64,
}

/// A recorded action during a session (for sleep consolidation).
#[derive(Debug, Clone)]
pub struct SessionAction {
    pub description: String,
    pub task_type: String,
    pub outcome: String,
}

impl SessionManager {
    pub fn new(executor: Arc<CommandExecutor>) -> Self {
        Self {
            executor,
            state: SessionState::Idle,
            mode: SessionMode::Conversation,
            session_actions: Vec::new(),
            goal_key: None,
            sleep_settings: SleepSettings::default(),
            drift_detector: None,
            goal_audit_start: 0,
        }
    }

    /// Create with custom sleep settings.
    pub fn with_sleep_settings(executor: Arc<CommandExecutor>, settings: SleepSettings) -> Self {
        Self {
            executor,
            state: SessionState::Idle,
            mode: SessionMode::Conversation,
            session_actions: Vec::new(),
            goal_key: None,
            sleep_settings: settings,
            drift_detector: None,
            goal_audit_start: 0,
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn mode(&self) -> &SessionMode {
        &self.mode
    }

    /// Start a new session in Conversation Mode.
    pub async fn start(&mut self, project: &str, goal: &str) -> Result<(), String> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let goal_key = format!("session:{}:goal", &session_id[..8]);
        let project_key = format!("session:{}:project", &session_id[..8]);

        let cmd = Command::new(
            "session",
            CommandOp::MemWrite {
                key: goal_key.clone(),
                content: goal.to_string(),
                scope: "task".into(),
                level: "L2".into(),
            },
        );
        let result = self.executor.execute(cmd).await;
        if result.status != CommandStatus::Ok {
            return Err(format!("failed to write goal: {:?}", result.error));
        }

        let cmd = Command::new(
            "session",
            CommandOp::MemWrite {
                key: project_key,
                content: project.to_string(),
                scope: "task".into(),
                level: "L2".into(),
            },
        );
        self.executor.execute(cmd).await;

        self.goal_key = Some(goal_key);
        self.state = SessionState::Active { project: project.into() };
        self.mode = SessionMode::Conversation;
        self.session_actions.clear();

        self.audit_event("session_start", &serde_json::json!({
            "project": project,
            "mode": "conversation",
        })).await;

        Ok(())
    }

    /// Enter Goal Mode. Compiles description into SpecIR.
    /// Returns error if already in Goal Mode (/done first).
    pub async fn enter_goal(&mut self, description: &str) -> Result<(), String> {
        if self.mode.is_goal() {
            return Err("already in Goal Mode — use /done first".into());
        }

        let term_table = TermTable::default_coding();
        let spec = SpecIR::compile_full(description, &term_table, None);

        self.audit_event("mode_switch", &serde_json::json!({
            "from": "conversation",
            "to": "goal",
            "spec_id": spec.spec_id,
            "intent": spec.intent,
        })).await;

        self.mode = SessionMode::Goal {
            spec,
            drift_score: 1.0,
            command_count: 0,
        };
        self.drift_detector = Some(DriftDetector::new());
        self.goal_audit_start = self.executor.audit().last_seq();

        Ok(())
    }

    /// Verification state of the active goal; None outside Goal Mode.
    pub fn verification(&self) -> Option<Verification> {
        if !self.mode.is_goal() {
            return None;
        }
        let entries = self.executor.audit().entries_after(self.goal_audit_start);
        Some(Verification::from_audit(&entries))
    }

    /// Claim the active goal is complete (`/done`).
    ///
    /// The claim is a command like any other: the Verification cage decides and
    /// the audit log records it. Allowed: leave Goal Mode and return the sleep
    /// consolidation report. Blocked: stay in Goal Mode; `Err` says why.
    pub async fn complete_goal(&mut self) -> Result<SleepConsolidationReport, String> {
        let (goal, verification) = match (&self.mode, self.verification()) {
            (SessionMode::Goal { spec, .. }, Some(v)) => (spec.intent.clone(), v),
            _ => return Err("not in Goal Mode".into()),
        };
        let cmd = Command::new("session", CommandOp::ClaimComplete {
            goal,
            verified: verification.is_verified(),
            evidence: verification.describe(),
        });
        let result = self.executor.execute(cmd).await;
        if result.status != CommandStatus::Ok {
            return Err(result.error.unwrap_or_else(|| "completion claim rejected".into()));
        }
        Ok(self.exit_goal().await)
    }

    /// Leave Goal Mode without claiming the goal is complete (`/abandon`).
    pub async fn abandon_goal(&mut self) -> Result<SleepConsolidationReport, String> {
        let verification = self.verification().ok_or("not in Goal Mode")?;
        self.audit_event("goal_abandoned", &serde_json::json!({
            "verification": verification.describe(),
        })).await;
        Ok(self.exit_goal().await)
    }

    /// Exit Goal Mode, trigger sleep consolidation, return to Conversation.
    ///
    /// Makes no completion claim: `/done` goes through `complete_goal` and
    /// `/abandon` through `abandon_goal`, which both end here.
    pub async fn exit_goal(&mut self) -> SleepConsolidationReport {
        self.audit_event("mode_switch", &serde_json::json!({
            "from": "goal",
            "to": "conversation",
        })).await;

        let report = consolidate(&self.session_actions, self.sleep_settings.max_candidates);

        // Lifecycle cleanup: soft-delete task-scoped memory entries
        let cleaned = self.executor.cleanup_memory_scope(terra_memory::types::Scope::Task);
        if cleaned > 0 {
            tracing::info!(cleaned, "task-scoped memory entries cleaned on goal exit");
        }

        self.mode = SessionMode::Conversation;
        self.session_actions.clear();
        self.drift_detector = None;

        report
    }

    /// Manual sleep consolidation (/sleep command).
    /// Runs consolidation without changing mode or clearing actions.
    pub async fn consolidate_manual(&self) -> SleepConsolidationReport {
        self.audit_event("sleep_manual", &serde_json::json!({
            "action_count": self.session_actions.len(),
        })).await;

        consolidate(&self.session_actions, self.sleep_settings.max_candidates)
    }

    /// Record a verdict on a promotion candidate and audit it.
    pub async fn record_verdict(&self, candidate_index: usize, accepted: bool) {
        self.audit_event("sleep_verdict", &serde_json::json!({
            "candidate_index": candidate_index,
            "accepted": accepted,
        })).await;
    }

    /// Record an action during the session.
    pub fn record_action(&mut self, action: SessionAction) {
        self.session_actions.push(action);
    }

    /// Check drift against the current goal spec.
    /// Returns None if not in Goal Mode or no detector is active.
    /// When a check fires, updates the drift_score and command_count in the mode.
    pub fn check_drift(&mut self, action: &str) -> Option<DriftCheckResult> {
        let intent = match &self.mode {
            SessionMode::Goal { spec, .. } => spec.intent.clone(),
            _ => return None,
        };

        let detector = self.drift_detector.as_mut()?;
        let result = detector.on_command(&intent, action);

        // Sync live score and count back into SessionMode
        if let SessionMode::Goal { drift_score, command_count, .. } = &mut self.mode {
            *drift_score = detector.last_score();
            *command_count = detector.command_count();
        }

        Some(result)
    }

    /// Get the current goal (if session is active).
    pub async fn get_goal(&self) -> Option<String> {
        let key = self.goal_key.as_ref()?;
        let cmd = Command::new(
            "session",
            CommandOp::MemRead { key: key.clone() },
        );
        let result = self.executor.execute(cmd).await;
        result
            .data
            .as_ref()
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
    }

    /// Get status summary for /status command.
    pub fn status(&self) -> SessionStatus {
        SessionStatus {
            state: self.state.clone(),
            mode_name: match &self.mode {
                SessionMode::Conversation => "conversation".into(),
                SessionMode::Goal { .. } => "goal".into(),
            },
            goal_intent: match &self.mode {
                SessionMode::Goal { spec, .. } => Some(spec.intent.clone()),
                _ => None,
            },
            drift_score: match &self.mode {
                SessionMode::Goal { drift_score, .. } => Some(*drift_score),
                _ => None,
            },
        }
    }

    /// Switch to a different project.
    pub async fn switch(&mut self, new_project: &str, new_goal: &str) -> Result<(), String> {
        let flush_cmd = Command::new(
            "session",
            CommandOp::TraceCheckpoint {
                title: "session_switch".into(),
                summary: format!("switching to {}", new_project),
            },
        );
        self.executor.execute(flush_cmd).await;
        self.start(new_project, new_goal).await
    }

    /// End the session entirely. Returns to Idle.
    pub async fn end(&mut self) -> SleepConsolidationReport {
        self.audit_event("session_end", &serde_json::json!({
            "mode": if self.mode.is_goal() { "goal" } else { "conversation" },
        })).await;

        let report = consolidate(&self.session_actions, self.sleep_settings.max_candidates);

        self.state = SessionState::Idle;
        self.mode = SessionMode::Conversation;
        self.session_actions.clear();

        report
    }

    async fn audit_event(&self, event_type: &str, data: &serde_json::Value) {
        let cmd = Command::new(
            "session",
            CommandOp::TraceEvent {
                event_type: event_type.to_string(),
                data: data.clone(),
            },
        );
        self.executor.execute(cmd).await;
    }
}

/// Status snapshot for /status command.
#[derive(Debug, Clone)]
pub struct SessionStatus {
    pub state: SessionState,
    pub mode_name: String,
    pub goal_intent: Option<String>,
    pub drift_score: Option<f64>,
}

/// Sleep Consolidation Report — generated at session end.
#[derive(Debug, Clone)]
pub struct SleepConsolidationReport {
    pub candidates: Vec<PromotionCandidate>,
    pub action_count: usize,
}

/// Sleep consolidation configuration (mirrors CLI SleepConfig).
#[derive(Debug, Clone)]
pub struct SleepSettings {
    pub trigger: String,
    pub interval_hours: u32,
    pub max_candidates: usize,
}

impl Default for SleepSettings {
    fn default() -> Self {
        Self {
            trigger: "session_end".into(),
            interval_hours: 24,
            max_candidates: 10,
        }
    }
}

/// A candidate for promotion from L6 to L1.
#[derive(Debug, Clone)]
pub struct PromotionCandidate {
    pub principle: String,
    pub derived_from: Vec<String>,
    /// IC rule ID this principle relates to (e.g., "B-002", "R-003")
    pub ic_rule_id: Option<String>,
    pub promoted: Option<bool>,
}

impl PromotionCandidate {
    pub fn accept(&mut self) {
        self.promoted = Some(true);
    }

    pub fn reject(&mut self) {
        self.promoted = Some(false);
    }
}

/// IC rule keywords for matching actions to rules during consolidation.
const IC_RULE_KEYWORDS: &[(&str, &str)] = &[
    ("B-002", "minimal"),
    ("B-001", "elegant"),
    ("R-001", "understand"),
    ("R-003", "evidence"),
    ("R-002", "decompose"),
    ("B-003", "safe"),
    ("B-004", "verify"),
    ("C-001", "unwrap"),
    ("B-005", "pattern"),
    ("A-002", "edge case"),
];

/// Consolidate session actions into promotion candidates.
///
/// Scans actions (from L2+L5 scope), groups by task type,
/// matches against IC rule keywords, and generates candidates.
fn consolidate(actions: &[SessionAction], max_candidates: usize) -> SleepConsolidationReport {
    let action_count = actions.len();

    let mut groups: HashMap<String, Vec<&SessionAction>> = HashMap::new();
    for action in actions {
        groups
            .entry(action.task_type.clone())
            .or_default()
            .push(action);
    }

    let mut candidates: Vec<PromotionCandidate> = groups
        .iter()
        .filter(|(_, actions)| actions.len() >= 2)
        .map(|(task_type, actions)| {
            let descriptions: Vec<String> = actions
                .iter()
                .map(|a| a.description.clone())
                .collect();

            // Match against IC rules by scanning action descriptions
            let combined_text = descriptions.join(" ").to_lowercase();
            let ic_rule_id = IC_RULE_KEYWORDS
                .iter()
                .find(|(_, keyword)| combined_text.contains(keyword))
                .map(|(id, _)| id.to_string());

            PromotionCandidate {
                principle: format!(
                    "When performing {} tasks: common approach observed across {} instances",
                    task_type, actions.len()
                ),
                derived_from: descriptions,
                ic_rule_id,
                promoted: None,
            }
        })
        .collect();

    candidates.truncate(max_candidates);

    SleepConsolidationReport { candidates, action_count }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_caging::engine::*;
    use terra_protocol::command::CommandResult;
    use terra_caging::sandbox::SandboxGuard;
    use terra_memory::{ArtifactStore, MemoryDatabase};
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::{AuditEntry, AuditLog};

    fn test_executor() -> (Arc<CommandExecutor>, tempfile::TempDir) {
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
        let exec = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));
        (exec, dir)
    }

    fn entry(seq: u64, agent: &str, op: CommandOp, status: CommandStatus) -> AuditEntry {
        let result = CommandResult { status, ..CommandResult::ok("", None) };
        AuditEntry {
            seq,
            timestamp: chrono::Utc::now(),
            command: Command::new(agent, op),
            result: Some(result),
            duration_us: Some(1),
        }
    }

    fn write_op() -> CommandOp {
        CommandOp::FileWrite { path: "src/lib.rs".into(), content: "x".into() }
    }

    fn shell_op(command: &str) -> CommandOp {
        CommandOp::ShellExec { command: command.into(), workdir: None, timeout_ms: None }
    }

    #[test]
    fn verification_follows_the_audit_log() {
        use CommandStatus::*;
        let state = |entries: &[AuditEntry]| Verification::from_audit(entries);

        assert_eq!(state(&[]), Verification::NoChanges);
        // Reads and failed or blocked writes change nothing.
        assert_eq!(state(&[
            entry(1, "main", CommandOp::FileRead { path: "a".into() }, Ok),
            entry(2, "main", write_op(), Error),
            entry(3, "main", write_op(), PermissionDenied),
        ]), Verification::NoChanges);
        // Every write, delete or shell command by the agent counts, even a failing one.
        assert_eq!(state(&[
            entry(1, "main", write_op(), Ok),
            entry(2, "main", CommandOp::FileDelete { path: "b".into() }, Ok),
            entry(3, "main", shell_op("sed -i s/a/b/ c"), Error),
        ]), Verification::Unverified { changes: 3 });
        // A passing /verify clears them; a later change makes it stale.
        let verified = [entry(1, "main", write_op(), Ok), entry(2, VERIFIER_AGENT, shell_op("cargo test"), Ok)];
        assert_eq!(state(&verified), Verification::Passed { command: "cargo test".into() });
        let stale = [verified[0].clone(), verified[1].clone(), entry(3, "main", write_op(), Ok)];
        assert_eq!(state(&stale), Verification::Unverified { changes: 1 });
        // The agent running tests itself is a change, not a verification.
        assert_eq!(state(&[entry(1, "main", shell_op("cargo test"), Ok)]), Verification::Unverified { changes: 1 });
        // A failing /verify is remembered until the next one passes.
        let failed = [entry(1, VERIFIER_AGENT, shell_op("cargo test"), Error)];
        assert_eq!(state(&failed), Verification::Failed { command: "cargo test".into() });
        // A /verify blocked by a cage never ran.
        assert_eq!(state(&[entry(1, VERIFIER_AGENT, shell_op("mkfs"), PermissionDenied)]), Verification::NoChanges);

        assert!(Verification::NoChanges.is_verified());
        assert!(Verification::Passed { command: "t".into() }.is_verified());
        assert!(!Verification::Unverified { changes: 1 }.is_verified());
        assert!(!Verification::Failed { command: "t".into() }.is_verified());
    }

    // The verification cage stops /done while the goal's changes are unverified.
    #[tokio::test]
    async fn completion_claim_needs_verification_after_changes() {
        let (exec, dir) = test_executor();
        let mut mgr = SessionManager::new(exec.clone());
        mgr.start("project", "general").await.unwrap();
        mgr.enter_goal("add a greeting to notes.txt").await.unwrap();

        let path = dir.path().join("notes.txt").to_string_lossy().to_string();
        let write = exec.execute(Command::new("main", CommandOp::FileWrite { path, content: "hi".into() })).await;
        assert_eq!(write.status, CommandStatus::Ok);

        let err = mgr.complete_goal().await.unwrap_err();
        assert!(err.contains("verification"), "{}", err);
        assert!(mgr.mode().is_goal(), "a rejected claim keeps Goal Mode");

        let failing = Command::new(VERIFIER_AGENT, shell_op("exit 3"));
        exec.execute(failing).await;
        assert!(mgr.complete_goal().await.is_err(), "a failing verification does not count");

        exec.execute(Command::new(VERIFIER_AGENT, shell_op("true"))).await;
        assert!(mgr.complete_goal().await.is_ok());
        assert!(mgr.mode().is_conversation());

        let claims: Vec<CommandStatus> = exec.audit().entries().into_iter()
            .filter(|e| matches!(e.command.op, CommandOp::ClaimComplete { .. }))
            .map(|e| e.result.unwrap().status)
            .collect();
        assert_eq!(claims, vec![
            CommandStatus::PermissionDenied, CommandStatus::PermissionDenied, CommandStatus::Ok,
        ]);
    }

    #[tokio::test]
    async fn completion_claim_without_changes_is_allowed() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "general").await.unwrap();
        mgr.enter_goal("explain the auth flow").await.unwrap();
        assert_eq!(mgr.verification(), Some(Verification::NoChanges));
        assert!(mgr.complete_goal().await.is_ok());
        assert!(mgr.complete_goal().await.is_err(), "no goal left to claim");
    }

    #[tokio::test]
    async fn abandon_leaves_goal_mode_without_a_claim() {
        let (exec, dir) = test_executor();
        let mut mgr = SessionManager::new(exec.clone());
        mgr.start("project", "general").await.unwrap();
        mgr.enter_goal("rewrite notes.txt").await.unwrap();
        let path = dir.path().join("notes.txt").to_string_lossy().to_string();
        exec.execute(Command::new("main", CommandOp::FileWrite { path, content: "x".into() })).await;

        assert!(mgr.abandon_goal().await.is_ok());
        assert!(mgr.mode().is_conversation());
        assert!(!exec.audit().entries().iter().any(|e| matches!(e.command.op, CommandOp::ClaimComplete { .. })));
        assert!(mgr.abandon_goal().await.is_err(), "nothing to abandon outside Goal Mode");
    }

    // T-DM-001: default_is_conversation
    #[tokio::test]
    async fn default_is_conversation() {
        let (exec, _dir) = test_executor();
        let mgr = SessionManager::new(exec);
        assert!(mgr.mode().is_conversation());
    }

    // T-DM-002: goal_command_activates
    #[tokio::test]
    async fn goal_command_activates() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "general").await.unwrap();

        mgr.enter_goal("fix the auth bug").await.unwrap();

        assert!(mgr.mode().is_goal());
        if let SessionMode::Goal { spec, drift_score, .. } = mgr.mode() {
            assert_eq!(spec.intent, "fix the auth bug");
            assert!((drift_score - 1.0).abs() < 0.01);
        }
    }

    // T-DM-003: done_returns_conversation
    #[tokio::test]
    async fn done_returns_conversation() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "general").await.unwrap();

        mgr.enter_goal("fix bug").await.unwrap();
        assert!(mgr.mode().is_goal());

        let report = mgr.exit_goal().await;
        assert!(mgr.mode().is_conversation());
        assert_eq!(report.action_count, 0);
    }

    // T-DM-007: mode_switch_audited
    #[tokio::test]
    async fn mode_switch_audited() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "general").await.unwrap();

        // Conversation → Goal
        mgr.enter_goal("fix bug").await.unwrap();
        // Goal → Conversation
        mgr.exit_goal().await;

        // Both transitions should have been audited (TraceEvent commands)
        // We verify the mode transitions happened correctly
        assert!(mgr.mode().is_conversation());
    }

    // T-DM-010: nested_goal_rejected
    #[tokio::test]
    async fn nested_goal_rejected() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "general").await.unwrap();

        mgr.enter_goal("task 1").await.unwrap();
        let result = mgr.enter_goal("task 2").await;
        assert!(result.is_err(), "nested goal should be rejected");
        assert!(result.unwrap_err().contains("/done"));
    }

    // T-DM-009: drift only tracked in goal mode
    #[test]
    fn drift_only_in_goal() {
        let mode_conv = SessionMode::Conversation;
        assert!(mode_conv.is_conversation());

        let mode_goal = SessionMode::Goal {
            spec: SpecIR::from_task_description("test"),
            drift_score: 0.8,
            command_count: 5,
        };
        assert!(mode_goal.is_goal());
        if let SessionMode::Goal { drift_score, .. } = &mode_goal {
            assert!(*drift_score > 0.0);
        }
    }

    // Status reports mode correctly
    #[tokio::test]
    async fn status_shows_mode() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "general").await.unwrap();

        let status = mgr.status();
        assert_eq!(status.mode_name, "conversation");
        assert!(status.drift_score.is_none());

        mgr.enter_goal("fix bug").await.unwrap();
        let status = mgr.status();
        assert_eq!(status.mode_name, "goal");
        assert!(status.drift_score.is_some());
        assert_eq!(status.goal_intent, Some("fix bug".into()));
    }

    // Existing T-SL tests (preserved)
    // T-SL-001
    #[tokio::test]
    async fn session_start_creates_s2() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("myproject", "fix auth bugs").await.unwrap();
        assert!(matches!(mgr.state(), SessionState::Active { .. }));
    }

    // T-SL-002
    #[tokio::test]
    async fn session_goal_recorded() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("myproject", "fix the login system").await.unwrap();
        let goal = mgr.get_goal().await;
        assert_eq!(goal, Some("fix the login system".into()));
    }

    // T-SL-003
    #[tokio::test]
    async fn session_switch_flushes() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project_a", "goal a").await.unwrap();
        mgr.switch("project_b", "goal b").await.unwrap();
        if let SessionState::Active { project } = mgr.state() {
            assert_eq!(project, "project_b");
        } else {
            panic!("should be active after switch");
        }
    }

    // T-SL-004
    #[tokio::test]
    async fn session_switch_loads_new() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("old", "old goal").await.unwrap();
        mgr.switch("new", "new goal").await.unwrap();
        let goal = mgr.get_goal().await;
        assert_eq!(goal, Some("new goal".into()));
    }

    // T-SL-005
    #[tokio::test]
    async fn session_end_cleans() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "goal").await.unwrap();
        let _report = mgr.end().await;
        assert_eq!(*mgr.state(), SessionState::Idle);
        assert!(mgr.mode().is_conversation());
    }

    // T-SL-006
    #[tokio::test]
    async fn session_end_triggers_sleep() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "goal").await.unwrap();
        mgr.record_action(SessionAction {
            description: "fixed auth bug".into(),
            task_type: "bugfix".into(),
            outcome: "success".into(),
        });
        mgr.record_action(SessionAction {
            description: "fixed validation bug".into(),
            task_type: "bugfix".into(),
            outcome: "success".into(),
        });
        let report = mgr.end().await;
        assert_eq!(report.action_count, 2);
    }

    // T-SL-007
    #[test]
    fn sleep_abstracts_actions() {
        let actions = vec![
            SessionAction { description: "fixed auth".into(), task_type: "bugfix".into(), outcome: "ok".into() },
            SessionAction { description: "fixed login".into(), task_type: "bugfix".into(), outcome: "ok".into() },
            SessionAction { description: "added tests".into(), task_type: "bugfix".into(), outcome: "ok".into() },
        ];
        let report = consolidate(&actions, 10);
        assert!(!report.candidates.is_empty());
        assert!(report.candidates[0].principle.contains("bugfix"));
    }

    // T-SL-008
    #[test]
    fn sleep_generates_candidates() {
        let actions = vec![
            SessionAction { description: "task 1".into(), task_type: "feature".into(), outcome: "ok".into() },
            SessionAction { description: "task 2".into(), task_type: "feature".into(), outcome: "ok".into() },
            SessionAction { description: "task 3".into(), task_type: "bugfix".into(), outcome: "ok".into() },
            SessionAction { description: "task 4".into(), task_type: "bugfix".into(), outcome: "ok".into() },
            SessionAction { description: "task 5".into(), task_type: "refactor".into(), outcome: "ok".into() },
        ];
        let report = consolidate(&actions, 10);
        assert!(report.candidates.len() >= 2);
    }

    // T-SL-009
    #[test]
    fn sleep_user_accept_promotes() {
        let mut c = PromotionCandidate {
            principle: "test".into(), derived_from: vec!["a".into()],
            ic_rule_id: None, promoted: None,
        };
        c.accept();
        assert_eq!(c.promoted, Some(true));
    }

    // T-SL-010
    #[test]
    fn sleep_user_reject_discards() {
        let mut c = PromotionCandidate {
            principle: "test".into(), derived_from: vec!["a".into()],
            ic_rule_id: None, promoted: None,
        };
        c.reject();
        assert_eq!(c.promoted, Some(false));
    }

    // T-SL-011: sleep_config_from_toml
    #[test]
    fn sleep_config_defaults() {
        let settings = SleepSettings::default();
        assert_eq!(settings.trigger, "session_end");
        assert_eq!(settings.interval_hours, 24);
        assert_eq!(settings.max_candidates, 10);
    }

    // T-SL-012: sleep_trigger_session_end
    #[tokio::test]
    async fn sleep_trigger_session_end() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "goal").await.unwrap();
        mgr.enter_goal("fix bug").await.unwrap();
        mgr.record_action(SessionAction {
            description: "fixed auth".into(), task_type: "bugfix".into(), outcome: "ok".into(),
        });
        mgr.record_action(SessionAction {
            description: "fixed login".into(), task_type: "bugfix".into(), outcome: "ok".into(),
        });
        // exit_goal triggers consolidation
        let report = mgr.exit_goal().await;
        assert_eq!(report.action_count, 2);
        assert!(!report.candidates.is_empty());
    }

    // T-SL-013: sleep_trigger_manual
    #[tokio::test]
    async fn sleep_trigger_manual() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);
        mgr.start("project", "goal").await.unwrap();
        mgr.record_action(SessionAction {
            description: "task a".into(), task_type: "feature".into(), outcome: "ok".into(),
        });
        mgr.record_action(SessionAction {
            description: "task b".into(), task_type: "feature".into(), outcome: "ok".into(),
        });
        // Manual consolidation without mode change
        let report = mgr.consolidate_manual().await;
        assert_eq!(report.action_count, 2);
        // Actions NOT cleared (manual doesn't change state)
        assert!(mgr.mode().is_conversation());
    }

    // T-SL-014: sleep scans by grouping (L2+L5 scope represented by actions)
    #[test]
    fn sleep_scans_actions_by_type() {
        let actions = vec![
            SessionAction { description: "a".into(), task_type: "bugfix".into(), outcome: "ok".into() },
            SessionAction { description: "b".into(), task_type: "bugfix".into(), outcome: "ok".into() },
            SessionAction { description: "c".into(), task_type: "feature".into(), outcome: "ok".into() },
        ];
        let report = consolidate(&actions, 10);
        // Only bugfix group has >= 2, feature has 1 (filtered out)
        assert_eq!(report.candidates.len(), 1);
        assert!(report.candidates[0].principle.contains("bugfix"));
    }

    // T-SL-015: sleep matches IC rules
    #[test]
    fn sleep_matches_ic_rules() {
        let actions = vec![
            SessionAction {
                description: "applied minimal change to fix auth".into(),
                task_type: "bugfix".into(), outcome: "ok".into(),
            },
            SessionAction {
                description: "kept change minimal, only target file".into(),
                task_type: "bugfix".into(), outcome: "ok".into(),
            },
        ];
        let report = consolidate(&actions, 10);
        assert!(!report.candidates.is_empty());
        // "minimal" keyword should match B-002
        assert_eq!(
            report.candidates[0].ic_rule_id.as_deref(),
            Some("B-002"),
            "should match B-002 (minimal change)"
        );
    }

    // T-SL-016: max_candidates respected
    #[test]
    fn sleep_max_candidates_respected() {
        let mut actions = Vec::new();
        for i in 0..20 {
            let task_type = format!("type_{}", i / 2);
            actions.push(SessionAction {
                description: format!("action {}", i), task_type, outcome: "ok".into(),
            });
        }
        let report = consolidate(&actions, 5);
        assert!(report.candidates.len() <= 5, "should respect max_candidates=5, got {}", report.candidates.len());
    }

    // T-SL-017: accept promotes (verdict recorded)
    #[tokio::test]
    async fn sleep_verdict_accept() {
        let (exec, _dir) = test_executor();
        let mgr = SessionManager::new(exec);

        let mut candidate = PromotionCandidate {
            principle: "always verify before commit".into(),
            derived_from: vec!["action 1".into(), "action 2".into()],
            ic_rule_id: Some("B-004".into()),
            promoted: None,
        };
        candidate.accept();
        assert_eq!(candidate.promoted, Some(true));

        // Verdict audited
        mgr.record_verdict(0, true).await;
    }

    // T-SL-018: verdict recorded in audit
    #[tokio::test]
    async fn sleep_verdict_reject_audited() {
        let (exec, _dir) = test_executor();
        let mgr = SessionManager::new(exec);

        let mut candidate = PromotionCandidate {
            principle: "test".into(),
            derived_from: vec!["a".into()],
            ic_rule_id: None,
            promoted: None,
        };
        candidate.reject();
        assert_eq!(candidate.promoted, Some(false));

        // Verdict audited
        mgr.record_verdict(0, false).await;
    }
}
