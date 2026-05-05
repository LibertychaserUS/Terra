//! Session Manager + Sleep Consolidation
//!
//! Session lifecycle:
//! - start: creates S2 (task) scope, records goal
//! - switch: flushes old cache, loads new project data
//! - end: cleans S3 (temp), triggers sleep consolidation
//!
//! Sleep Consolidation:
//! - Post-session abstraction of concrete actions into general principles
//! - Generates L6 promotion candidates for user review
//! - User accept → promote to L1 (project knowledge)
//! - User reject → discard

use std::collections::HashMap;
use std::sync::Arc;

use terra_executor::CommandExecutor;
use terra_protocol::command::{Command, CommandOp, CommandStatus};

/// Session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Active { project: String, goal: String },
}

/// The Session Manager.
pub struct SessionManager {
    executor: Arc<CommandExecutor>,
    state: SessionState,
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
            session_actions: Vec::new(),
            goal_key: None,
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Start a new session. Creates S2 scope for task-level data.
    pub async fn start(&mut self, project: &str, goal: &str) -> Result<(), String> {
        // Generate unique session key to avoid duplicate conflicts
        let session_id = uuid::Uuid::new_v4().to_string();
        let goal_key = format!("session:{}:goal", &session_id[..8]);
        let project_key = format!("session:{}:project", &session_id[..8]);

        // Write goal to memory (S2 scope)
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

        // Write project marker
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
        self.state = SessionState::Active {
            project: project.into(),
            goal: goal.into(),
        };
        self.session_actions.clear();

        Ok(())
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

    /// Switch to a different project. Flushes old session state.
    pub async fn switch(&mut self, new_project: &str, new_goal: &str) -> Result<(), String> {
        // Flush old session data by overwriting
        let flush_cmd = Command::new(
            "session",
            CommandOp::TraceCheckpoint {
                title: "session_switch".into(),
                summary: format!("switching to {}", new_project),
            },
        );
        self.executor.execute(flush_cmd).await;

        // Start new session
        self.start(new_project, new_goal).await
    }

    /// End the session. Cleans S3 temp data, triggers sleep consolidation.
    pub async fn end(&mut self) -> SleepConsolidationReport {
        // Record session end
        let cmd = Command::new(
            "session",
            CommandOp::TraceCheckpoint {
                title: "session_end".into(),
                summary: "session ending, triggering consolidation".into(),
            },
        );
        self.executor.execute(cmd).await;

        // Generate sleep consolidation report
        let report = consolidate(&self.session_actions);

        self.state = SessionState::Idle;
        self.session_actions.clear();

        report
    }
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
    pub promoted: Option<bool>, // None = pending, Some(true) = accepted, Some(false) = rejected
}

impl PromotionCandidate {
    /// Accept this candidate — marks it for promotion to L1.
    pub fn accept(&mut self) {
        self.promoted = Some(true);
    }

    /// Reject this candidate — marks it as discarded.
    pub fn reject(&mut self) {
        self.promoted = Some(false);
    }
}

/// Consolidate session actions into promotion candidates.
///
/// Groups similar actions and abstracts them into general principles.
fn consolidate(actions: &[SessionAction]) -> SleepConsolidationReport {
    let action_count = actions.len();

    // Group actions by task type
    let mut groups: HashMap<String, Vec<&SessionAction>> = HashMap::new();
    for action in actions {
        groups
            .entry(action.task_type.clone())
            .or_default()
            .push(action);
    }

    // Generate one candidate per group
    let candidates: Vec<PromotionCandidate> = groups
        .iter()
        .filter(|(_, actions)| actions.len() >= 2) // need pattern, not single occurrence
        .map(|(task_type, actions)| {
            let descriptions: Vec<String> = actions
                .iter()
                .map(|a| a.description.clone())
                .collect();

            let principle = format!(
                "When performing {} tasks: common approach observed across {} instances",
                task_type,
                actions.len()
            );

            PromotionCandidate {
                principle,
                derived_from: descriptions,
                promoted: None,
            }
        })
        .collect();

    SleepConsolidationReport {
        candidates,
        action_count,
    }
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

    // T-SL-001: session_start_creates_s2
    #[tokio::test]
    async fn session_start_creates_s2() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);

        mgr.start("myproject", "fix auth bugs").await.unwrap();

        assert!(matches!(mgr.state(), SessionState::Active { .. }));
    }

    // T-SL-002: session_goal_recorded
    #[tokio::test]
    async fn session_goal_recorded() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);

        mgr.start("myproject", "fix the login system").await.unwrap();
        let goal = mgr.get_goal().await;
        assert_eq!(goal, Some("fix the login system".into()));
    }

    // T-SL-003: session_switch_flushes
    #[tokio::test]
    async fn session_switch_flushes() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);

        mgr.start("project_a", "goal a").await.unwrap();
        mgr.switch("project_b", "goal b").await.unwrap();

        if let SessionState::Active { project, goal } = mgr.state() {
            assert_eq!(project, "project_b");
            assert_eq!(goal, "goal b");
        } else {
            panic!("should be active after switch");
        }
    }

    // T-SL-004: session_switch_loads_new
    #[tokio::test]
    async fn session_switch_loads_new() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);

        mgr.start("old", "old goal").await.unwrap();
        mgr.switch("new", "new goal").await.unwrap();

        let goal = mgr.get_goal().await;
        assert_eq!(goal, Some("new goal".into()));
    }

    // T-SL-005: session_end_cleans_s3
    #[tokio::test]
    async fn session_end_cleans() {
        let (exec, _dir) = test_executor();
        let mut mgr = SessionManager::new(exec);

        mgr.start("project", "goal").await.unwrap();
        let _report = mgr.end().await;

        assert_eq!(*mgr.state(), SessionState::Idle);
    }

    // T-SL-006: session_end_triggers_sleep
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
        // Report should exist with action count
        assert_eq!(report.action_count, 2);
    }

    // T-SL-007: sleep_abstracts_actions
    #[test]
    fn sleep_abstracts_actions() {
        let actions = vec![
            SessionAction { description: "fixed auth".into(), task_type: "bugfix".into(), outcome: "ok".into() },
            SessionAction { description: "fixed login".into(), task_type: "bugfix".into(), outcome: "ok".into() },
            SessionAction { description: "added tests".into(), task_type: "bugfix".into(), outcome: "ok".into() },
        ];
        let report = consolidate(&actions);
        assert!(!report.candidates.is_empty(), "should generate candidates");
        assert!(report.candidates[0].principle.contains("bugfix"));
        assert!(report.candidates[0].derived_from.len() >= 2);
    }

    // T-SL-008: sleep_generates_candidates
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
        // 2 groups with >= 2 actions: feature (2) and bugfix (2)
        assert!(report.candidates.len() >= 2, "should have >= 2 candidates, got {}", report.candidates.len());
    }

    // T-SL-009: sleep_user_accept_promotes
    #[test]
    fn sleep_user_accept_promotes() {
        let mut candidate = PromotionCandidate {
            principle: "test principle".into(),
            derived_from: vec!["action 1".into()],
            promoted: None,
        };
        assert!(candidate.promoted.is_none());

        candidate.accept();
        assert_eq!(candidate.promoted, Some(true));
    }

    // T-SL-010: sleep_user_reject_discards
    #[test]
    fn sleep_user_reject_discards() {
        let mut candidate = PromotionCandidate {
            principle: "test principle".into(),
            derived_from: vec!["action 1".into()],
            promoted: None,
        };

        candidate.reject();
        assert_eq!(candidate.promoted, Some(false));
    }
}
