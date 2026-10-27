/// Result of a cage check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CageCheckResult {
    /// Action is allowed.
    Pass,
    /// Action is allowed but logged as a warning.
    Warn(String),
    /// Action is blocked. The string describes why.
    Block(String),
}

impl CageCheckResult {
    pub fn is_blocked(&self) -> bool {
        matches!(self, CageCheckResult::Block(_))
    }

    pub fn is_pass(&self) -> bool {
        matches!(self, CageCheckResult::Pass)
    }
}

/// A cage: a predicate on the action an agent proposes.
///
/// Cages never change the action or choose another one. The engine runs
/// them in order; the first `Block` stops the action.
pub trait Cage: Send + Sync {
    /// Human-readable name of this cage.
    fn name(&self) -> &str;

    /// Check whether the given action is allowed.
    fn check(&self, context: &CageContext) -> CageCheckResult;

    /// Whether this cage only activates in Goal Mode.
    fn goal_only(&self) -> bool {
        false
    }
}

/// What the cages know when they judge an action.
#[derive(Debug, Clone)]
pub struct CageContext {
    pub agent: String,
    pub action: CageAction,
    /// Whether the session is in Goal Mode. Goal-only cages skip otherwise.
    pub goal_mode: bool,
    /// Whether the active goal's specification has no open questions.
    /// Meaningless outside Goal Mode.
    pub spec_complete: bool,
}

impl CageContext {
    /// A context outside Goal Mode.
    pub fn conversation(agent: impl Into<String>, action: CageAction) -> Self {
        Self { agent: agent.into(), action, goal_mode: false, spec_complete: true }
    }

    /// A context in Goal Mode with a complete specification.
    pub fn goal(agent: impl Into<String>, action: CageAction) -> Self {
        Self { agent: agent.into(), action, goal_mode: true, spec_complete: true }
    }
}

/// The action being checked.
#[derive(Debug, Clone)]
pub enum CageAction {
    /// The agent wants to start a task whose spec is this complete.
    ExecuteTask { spec_complete: bool },
    /// Write a file at this path.
    FileWrite { path: std::path::PathBuf },
    /// Read a file at this path.
    FileRead { path: std::path::PathBuf },
    /// Delete a file at this path.
    FileDelete { path: std::path::PathBuf },
    /// Write memory.
    MemoryWrite {
        key: String,
        owner: String,
        /// Target memory level (L0-L6)
        level: String,
        /// Whether this write originates from a user command (/remember, /accept)
        is_user_command: bool,
    },
    /// Claim the task is complete.
    ClaimComplete { verified: bool },
    /// Modify a protected rule.
    ModifyRule { rule_name: String },
    /// Execute a shell command.
    ShellExec { command: String },
    /// Anything else the executor routes (trace notes, provider calls).
    Internal,
}

impl CageAction {
    /// Whether the action can change the project (file system or shell).
    pub fn mutates(&self) -> bool {
        matches!(self, CageAction::FileWrite { .. } | CageAction::FileDelete { .. } | CageAction::ShellExec { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cage_check_result_predicates() {
        assert!(CageCheckResult::Pass.is_pass());
        assert!(!CageCheckResult::Warn("careful".into()).is_blocked());
        assert!(CageCheckResult::Block("denied".into()).is_blocked());
    }

    #[test]
    fn mutating_actions() {
        assert!(CageAction::ShellExec { command: "ls".into() }.mutates());
        assert!(!CageAction::FileRead { path: "x".into() }.mutates());
        assert!(!CageAction::Internal.mutates());
    }
}
