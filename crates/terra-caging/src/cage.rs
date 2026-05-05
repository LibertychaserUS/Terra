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

/// Trait for all cage implementations.
///
/// Each cage checks a specific aspect of agent behavior.
/// The CagingEngine runs all cages in sequence; the first
/// Block short-circuits and prevents the action.
pub trait Cage: Send + Sync {
    /// Human-readable name of this cage.
    fn name(&self) -> &str;

    /// Check whether the given action is allowed.
    fn check(&self, context: &CageContext) -> CageCheckResult;
}

/// Context passed to cage checks.
///
/// Contains all information a cage might need to make a decision.
#[derive(Debug, Clone)]
pub struct CageContext {
    pub agent: String,
    pub action: CageAction,
}

/// The action being checked by the caging engine.
#[derive(Debug, Clone)]
pub enum CageAction {
    /// Agent wants to execute a task with this spec completeness.
    ExecuteTask { spec_complete: bool },
    /// Agent wants to write/delete a file at this path.
    FileWrite { path: std::path::PathBuf },
    /// Agent wants to read a file at this path.
    FileRead { path: std::path::PathBuf },
    /// Agent wants to delete a file at this path.
    FileDelete { path: std::path::PathBuf },
    /// Agent wants to write memory owned by another agent.
    MemoryWrite { key: String, owner: String },
    /// Agent claims task is complete.
    ClaimComplete { verified: bool },
    /// Agent attempts to modify a protected rule.
    ModifyRule { rule_name: String },
    /// Agent wants to continue exploring after path was accepted.
    ExploreAfterAccepted { path_id: String },
    /// Agent makes a knowledge claim. Authority ranking determines trust.
    KnowledgeClaim {
        /// The source of this knowledge
        source: KnowledgeSource,
        /// Whether a higher-authority source contradicts this claim
        contradicted_by_higher: bool,
    },
}

/// Authority ranking for knowledge claims (FR-073).
/// Higher rank = higher authority. Lower never overrides higher.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum KnowledgeSource {
    /// Rank 0 (lowest): Agent assumption / inference
    Assumption,
    /// Rank 1: From memory database (L1-L6)
    Memory,
    /// Rank 2: From project documentation
    Documentation,
    /// Rank 3 (highest): From current source code
    Code,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cage_check_result_pass() {
        let r = CageCheckResult::Pass;
        assert!(r.is_pass());
        assert!(!r.is_blocked());
    }

    #[test]
    fn cage_check_result_warn() {
        let r = CageCheckResult::Warn("careful".into());
        assert!(!r.is_pass());
        assert!(!r.is_blocked());
    }

    #[test]
    fn cage_check_result_block() {
        let r = CageCheckResult::Block("denied".into());
        assert!(r.is_blocked());
        assert!(!r.is_pass());
    }
}
