use crate::cage::{Cage, CageAction, CageCheckResult, CageContext};
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
    pub fn check(&self, context: &CageContext) -> CagingResult {
        let mut warnings = Vec::new();

        for cage in &self.cages {
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
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "action");
    }

    // Short-circuit: first block wins
    #[test]
    fn first_block_short_circuits() {
        let engine = test_engine();
        // Incomplete spec AND non-owner write — spec cage blocks first
        let ctx = CageContext {
            agent: "worker".into(),
            action: CageAction::ExecuteTask { spec_complete: false },
        };
        let result = engine.check(&ctx);
        assert!(!result.allowed);
        assert_eq!(result.block.as_ref().unwrap().0, "specification");
    }
}
