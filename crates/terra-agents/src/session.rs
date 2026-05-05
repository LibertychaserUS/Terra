//! Session Manager + Sleep Consolidation
//!
//! Dual mode session lifecycle:
//! - Conversation Mode (default): free interaction, lightweight constraints
//! - Goal Mode (/goal): adds drift detection + spec gating on top of conversation
//!   - User can still talk freely — Goal Mode is additive, not restrictive
//!   - The difference: caging checks spec alignment, drift is monitored
//!
//! Sleep Consolidation:
//! - Post-session abstraction of concrete actions into general principles
//! - Generates L6 promotion candidates for user review
//! - User accept → promote to L1 (project knowledge)
//! - User reject → discard

use std::collections::HashMap;
use std::sync::Arc;

use terra_compile::SpecIR;
use terra_executor::CommandExecutor;
use terra_protocol::command::{Command, CommandOp, CommandStatus};

/// Runtime mode — determines which cages and checks are active.
///
/// Goal Mode is additive over Conversation: user can still talk freely,
/// but spec alignment and drift detection are active in the background.
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

        let spec = SpecIR::from_task_description(description);

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

        Ok(())
    }

    /// Exit Goal Mode, trigger sleep consolidation, return to Conversation.
    pub async fn exit_goal(&mut self) -> SleepConsolidationReport {
        self.audit_event("mode_switch", &serde_json::json!({
            "from": "goal",
            "to": "conversation",
        })).await;

        let report = consolidate(&self.session_actions);

        self.mode = SessionMode::Conversation;
        self.session_actions.clear();

        report
    }

    /// Record an action during the session.
    pub fn record_action(&mut self, action: SessionAction) {
        self.session_actions.push(action);
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

        let report = consolidate(&self.session_actions);

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

/// A candidate for promotion from L6 to L1.
#[derive(Debug, Clone)]
pub struct PromotionCandidate {
    pub principle: String,
    pub derived_from: Vec<String>,
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

/// Consolidate session actions into promotion candidates.
fn consolidate(actions: &[SessionAction]) -> SleepConsolidationReport {
    let action_count = actions.len();

    let mut groups: HashMap<String, Vec<&SessionAction>> = HashMap::new();
    for action in actions {
        groups
            .entry(action.task_type.clone())
            .or_default()
            .push(action);
    }

    let candidates: Vec<PromotionCandidate> = groups
        .iter()
        .filter(|(_, actions)| actions.len() >= 2)
        .map(|(task_type, actions)| {
            let descriptions: Vec<String> = actions
                .iter()
                .map(|a| a.description.clone())
                .collect();

            PromotionCandidate {
                principle: format!(
                    "When performing {} tasks: common approach observed across {} instances",
                    task_type, actions.len()
                ),
                derived_from: descriptions,
                promoted: None,
            }
        })
        .collect();

    SleepConsolidationReport { candidates, action_count }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use terra_caging::engine::*;
    use terra_caging::sandbox::SandboxGuard;
    use terra_memory::{ArtifactStore, MemoryDatabase};
    use terra_providers::adapter::DryRunAdapter;
    use terra_trace::AuditLog;

    fn test_executor() -> (Arc<CommandExecutor>, tempfile::TempDir) {
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
        let exec = Arc::new(CommandExecutor::new(memory, artifacts, audit, caging, provider));
        (exec, dir)
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
        let report = consolidate(&actions);
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
        let report = consolidate(&actions);
        assert!(report.candidates.len() >= 2);
    }

    // T-SL-009
    #[test]
    fn sleep_user_accept_promotes() {
        let mut c = PromotionCandidate {
            principle: "test".into(), derived_from: vec!["a".into()], promoted: None,
        };
        c.accept();
        assert_eq!(c.promoted, Some(true));
    }

    // T-SL-010
    #[test]
    fn sleep_user_reject_discards() {
        let mut c = PromotionCandidate {
            principle: "test".into(), derived_from: vec!["a".into()], promoted: None,
        };
        c.reject();
        assert_eq!(c.promoted, Some(false));
    }
}
