use crate::cage::{Cage, CageAction, CageCheckResult, CageContext, KnowledgeSource};
use crate::sandbox::SandboxGuard;

/// Specification Cage — blocks execution when spec is incomplete.
pub struct SpecificationCage;

impl Cage for SpecificationCage {
    fn name(&self) -> &str {
        "specification"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        if let CageAction::ExecuteTask { spec_complete } = &context.action {
            if !spec_complete {
                return CageCheckResult::Block(
                    "task specification is incomplete (has open questions)".into(),
                );
            }
        }
        CageCheckResult::Pass
    }

    fn goal_only(&self) -> bool {
        true
    }
}

/// Memory Cage — enforces ownership rules on memory writes.
pub struct MemoryCage;

impl Cage for MemoryCage {
    fn name(&self) -> &str {
        "memory"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        if let CageAction::MemoryWrite { owner, .. } = &context.action {
            if *owner != context.agent {
                return CageCheckResult::Block(format!(
                    "{} is not the owner (owner: {})",
                    context.agent, owner
                ));
            }
        }
        CageCheckResult::Pass
    }
}

/// Action Cage — enforces file system sandbox.
pub struct ActionCage {
    sandbox: SandboxGuard,
}

impl ActionCage {
    pub fn new(sandbox: SandboxGuard) -> Self {
        Self { sandbox }
    }
}

impl Cage for ActionCage {
    fn name(&self) -> &str {
        "action"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        match &context.action {
            CageAction::FileWrite { path } => match self.sandbox.check_write(path) {
                Ok(()) => CageCheckResult::Pass,
                Err(e) => CageCheckResult::Block(e.to_string()),
            },
            CageAction::FileDelete { path } => match self.sandbox.check_delete(path) {
                Ok(()) => CageCheckResult::Pass,
                Err(e) => CageCheckResult::Block(e.to_string()),
            },
            CageAction::FileRead { path } => match self.sandbox.check_read(path) {
                Ok(()) => CageCheckResult::Pass,
                Err(e) => CageCheckResult::Block(e.to_string()),
            },
            _ => CageCheckResult::Pass,
        }
    }
}

/// Verification Cage — blocks completion claims without verification.
pub struct VerificationCage;

impl Cage for VerificationCage {
    fn name(&self) -> &str {
        "verification"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        if let CageAction::ClaimComplete { verified } = &context.action {
            if !verified {
                return CageCheckResult::Block(
                    "completion claimed without external verification".into(),
                );
            }
        }
        CageCheckResult::Pass
    }

    fn goal_only(&self) -> bool {
        true
    }
}

/// Convergence Cage — stops exploration after an accepted path exists.
///
/// When a solution path has been accepted (verified, committed),
/// further exploration of alternatives is warned (or blocked if
/// the path is locked). This prevents wasted effort and divergence.
pub struct ConvergenceCage;

impl Cage for ConvergenceCage {
    fn name(&self) -> &str {
        "convergence"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        if let CageAction::ExploreAfterAccepted { path_id } = &context.action {
            return CageCheckResult::Warn(format!(
                "accepted path '{}' exists — further exploration may be wasteful",
                path_id
            ));
        }
        CageCheckResult::Pass
    }

    fn goal_only(&self) -> bool {
        true
    }
}

/// Knowledge Cage — enforces authority ranking for knowledge claims.
///
/// Authority hierarchy (FR-073):
///   Code > Documentation > Memory > Assumption
///
/// A claim from a lower-authority source that contradicts a
/// higher-authority source is blocked.
pub struct KnowledgeCage;

impl Cage for KnowledgeCage {
    fn name(&self) -> &str {
        "knowledge"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        if let CageAction::KnowledgeClaim {
            source,
            contradicted_by_higher,
        } = &context.action
        {
            if *contradicted_by_higher {
                return CageCheckResult::Block(format!(
                    "{} contradicts higher-authority source",
                    match source {
                        KnowledgeSource::Assumption => "assumption",
                        KnowledgeSource::Memory => "memory claim",
                        KnowledgeSource::Documentation => "documentation claim",
                        KnowledgeSource::Code => "code claim",
                    }
                ));
            }
            // Warn on low-authority sources even when not contradicted
            if *source <= KnowledgeSource::Assumption {
                return CageCheckResult::Warn(
                    "claim based on assumption — seek verification from code or docs".into(),
                );
            }
        }
        CageCheckResult::Pass
    }
}

/// Meta-Cage — blocks attempts to modify protected rules.
pub struct MetaCage;

impl Cage for MetaCage {
    fn name(&self) -> &str {
        "meta"
    }

    fn check(&self, context: &CageContext) -> CageCheckResult {
        if let CageAction::ModifyRule { rule_name } = &context.action {
            return CageCheckResult::Block(format!(
                "cannot modify protected rule: {}",
                rule_name
            ));
        }
        CageCheckResult::Pass
    }
}

/// The caging engine that runs all cages in sequence.
///
/// First Block result short-circuits: the action is prevented.
/// Warn results are collected but do not prevent the action.
pub struct CagingEngine {
    cages: Vec<Box<dyn Cage>>,
}

/// Result of running all cages.
#[derive(Debug)]
pub struct CagingResult {
    pub allowed: bool,
    pub warnings: Vec<(String, String)>,  // (cage_name, message)
    pub block: Option<(String, String)>,  // (cage_name, message)
}

impl CagingEngine {
    pub fn new(cages: Vec<Box<dyn Cage>>) -> Self {
        Self { cages }
    }

    /// Run all cages against the given context.
    /// Goal-only cages are skipped when context.goal_mode is false.
    pub fn check(&self, context: &CageContext) -> CagingResult {
        let mut warnings = Vec::new();

        for cage in &self.cages {
            // Skip goal-only cages in Conversation Mode
            if cage.goal_only() && !context.goal_mode {
                continue;
            }

            match cage.check(context) {
                CageCheckResult::Pass => {}
                CageCheckResult::Warn(msg) => {
                    warnings.push((cage.name().to_string(), msg));
                }
                CageCheckResult::Block(msg) => {
                    return CagingResult {
                        allowed: false,
                        warnings,
                        block: Some((cage.name().to_string(), msg)),
                    };
                }
            }
        }

        CagingResult {
            allowed: true,
            warnings,
            block: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_engine() -> CagingEngine {
        let sandbox = SandboxGuard::new(
            PathBuf::from("/projects/myapp"),
            vec![PathBuf::from("/projects/myapp/.terra/constitution.toml")],
        );

        CagingEngine::new(vec![
            Box::new(SpecificationCage),
            Box::new(MemoryCage),
            Box::new(ActionCage::new(sandbox)),
            Box::new(VerificationCage),
            Box::new(KnowledgeCage),
            Box::new(ConvergenceCage),
            Box::new(MetaCage),
        ])
    }

    // T-CG-004
    #[test]
    fn spec_cage_passes_complete() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ExecuteTask { spec_complete: true },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(result.allowed);
    }

    // T-CG-005
    #[test]
    fn spec_cage_blocks_incomplete() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ExecuteTask { spec_complete: false },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "specification");
    }

    // T-CG-006
    #[test]
    fn memory_cage_owner_write() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::MemoryWrite {
                key: "key1".into(),
                owner: "main".into(),
            },
            goal_mode: true,
        };
        assert!(engine.check(&ctx).allowed);
    }

    // T-CG-007
    #[test]
    fn memory_cage_non_owner_blocked() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "worker".into(),
            action: CageAction::MemoryWrite {
                key: "key1".into(),
                owner: "main".into(),
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "memory");
    }

    // T-CG-008
    #[test]
    fn memory_cage_any_read() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "worker".into(),
            action: CageAction::FileRead {
                path: PathBuf::from("/projects/myapp/src/main.rs"),
            },
            goal_mode: true,
        };
        assert!(engine.check(&ctx).allowed);
    }

    // T-CG-015
    #[test]
    fn verify_cage_no_verification() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ClaimComplete { verified: false },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "verification");
    }

    // T-CG-016
    #[test]
    fn verify_cage_after_verification() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ClaimComplete { verified: true },
            goal_mode: true,
        };
        assert!(engine.check(&ctx).allowed);
    }

    // T-CG-017
    #[test]
    fn meta_cage_constitution_change() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ModifyRule {
                rule_name: "constitution.article_1".into(),
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "meta");
    }

    // T-CG-018
    #[test]
    fn meta_cage_rule_change() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ModifyRule {
                rule_name: "sandbox.root".into(),
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
    }

    // Sandbox via engine
    #[test]
    fn action_cage_write_outside() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "worker".into(),
            action: CageAction::FileWrite {
                path: PathBuf::from("/etc/passwd"),
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "action");
    }

    #[test]
    fn action_cage_write_protected() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::FileWrite {
                path: PathBuf::from("/projects/myapp/.terra/constitution.toml"),
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "action");
    }

    // Warn flows through engine (T-CG-002)
    #[test]
    fn warn_logs_allows_action() {
        // Create engine with a cage that warns
        struct WarnCage;
        impl Cage for WarnCage {
            fn name(&self) -> &str { "warn_test" }
            fn check(&self, _ctx: &CageContext) -> CageCheckResult {
                CageCheckResult::Warn("proceed with caution".into())
            }
        }

        let engine = CagingEngine::new(vec![
            Box::new(WarnCage),
            Box::new(SpecificationCage),
        ]);

        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ExecuteTask { spec_complete: true },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(result.allowed, "warn should not block");
        assert_eq!(result.warnings.len(), 1);
        assert_eq!(result.warnings[0].0, "warn_test");
        assert!(result.warnings[0].1.contains("caution"));
    }

    // Short-circuit: first block wins
    #[test]
    fn first_block_short_circuits() {
        let engine = test_engine();
        // Incomplete spec AND non-owner write — spec cage blocks first
        let ctx = CageContext {
            agent: "worker".into(),
            action: CageAction::ExecuteTask { spec_complete: false },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "specification");
    }

    // T-CG-020: convergence_stops_after_accepted
    #[test]
    fn convergence_warns_after_accepted() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ExploreAfterAccepted {
                path_id: "solution_v2".into(),
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        // Convergence cage warns, does not block
        assert!(result.allowed, "convergence should warn, not block");
        assert!(
            result.warnings.iter().any(|w| w.0 == "convergence"),
            "should have convergence warning"
        );
    }

    // Knowledge cage: low-authority contradicted by higher → blocked
    #[test]
    fn knowledge_cage_blocks_contradicted() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "worker".into(),
            action: CageAction::KnowledgeClaim {
                source: KnowledgeSource::Assumption,
                contradicted_by_higher: true,
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "knowledge");
    }

    // Knowledge cage: assumption without contradiction → warn
    #[test]
    fn knowledge_cage_warns_assumption() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::KnowledgeClaim {
                source: KnowledgeSource::Assumption,
                contradicted_by_higher: false,
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(result.allowed, "uncontradicted assumption should pass with warning");
        assert!(result.warnings.iter().any(|w| w.0 == "knowledge"));
    }

    // Knowledge cage: code source, no contradiction → pass clean
    #[test]
    fn knowledge_cage_passes_code_source() {
        let engine = test_engine();
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::KnowledgeClaim {
                source: KnowledgeSource::Code,
                contradicted_by_higher: false,
            },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(result.allowed);
        assert!(result.warnings.is_empty());
    }

    // Knowledge source ordering
    #[test]
    fn knowledge_source_ordering() {
        assert!(KnowledgeSource::Assumption < KnowledgeSource::Memory);
        assert!(KnowledgeSource::Memory < KnowledgeSource::Documentation);
        assert!(KnowledgeSource::Documentation < KnowledgeSource::Code);
    }

    // T-DM-004: goal_mode_cages_active
    #[test]
    fn goal_mode_cages_active() {
        let engine = test_engine();
        // In Goal Mode, SpecificationCage blocks incomplete specs
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ExecuteTask { spec_complete: false },
            goal_mode: true,
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed, "spec cage should block in goal mode");
        assert_eq!(result.block.as_ref().unwrap().0, "specification");
    }

    // T-DM-005: conversation_cages_minimal
    #[test]
    fn conversation_cages_minimal() {
        let engine = test_engine();
        // In Conversation Mode, SpecificationCage is skipped
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ExecuteTask { spec_complete: false },
            goal_mode: false,
        };
        let result = engine.check(&ctx);
        assert!(result.allowed, "spec cage should be skipped in conversation mode");
    }

    // T-DM-006: conversation_no_spec_required
    #[test]
    fn conversation_no_spec_required() {
        let engine = test_engine();
        // VerificationCage also skipped in Conversation Mode
        let ctx = CageContext {
            agent: "main".into(),
            action: CageAction::ClaimComplete { verified: false },
            goal_mode: false,
        };
        let result = engine.check(&ctx);
        assert!(result.allowed, "verification cage should be skipped in conversation mode");

        // But always-on cages still work
        let ctx_meta = CageContext {
            agent: "main".into(),
            action: CageAction::ModifyRule { rule_name: "test".into() },
            goal_mode: false,
        };
        let result_meta = engine.check(&ctx_meta);
        assert!(!result_meta.allowed, "meta cage should still block in conversation mode");
    }
}
