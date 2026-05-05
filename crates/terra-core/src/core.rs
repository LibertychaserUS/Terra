use crate::domain::DomainModule;
use crate::framework::UniversalFramework;
use crate::types::{Perspective, Rule};

/// The Intelligence Core — agent's first principles.
///
/// Composes the universal framework (L0, read-only) with
/// registered domain modules (pluggable, composable).
pub struct IntelligenceCore {
    universal: UniversalFramework,
    modules: Vec<Box<dyn DomainModule>>,
}

impl IntelligenceCore {
    /// Create a new core with only universal rules.
    pub fn new() -> Self {
        Self {
            universal: UniversalFramework::new(),
            modules: Vec::new(),
        }
    }

    /// Register a domain module.
    pub fn register(&mut self, module: Box<dyn DomainModule>) {
        self.modules.push(module);
    }

    /// Get all rules: universal + all domain modules.
    pub fn all_rules(&self) -> Vec<Rule> {
        let mut rules: Vec<Rule> = self.universal.rules().to_vec();
        for module in &self.modules {
            rules.extend(module.rules());
        }
        rules
    }

    /// Get all perspectives from all domain modules.
    pub fn all_perspectives(&self) -> Vec<Perspective> {
        let mut perspectives = Vec::new();
        for module in &self.modules {
            perspectives.extend(module.perspectives());
        }
        perspectives
    }

    /// Get registered module names.
    pub fn module_names(&self) -> Vec<&str> {
        self.modules.iter().map(|m| m.name()).collect()
    }

    /// Get universal rules only.
    pub fn universal_rules(&self) -> &[Rule] {
        self.universal.rules()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AlgorithmModule, CodingModule};
    use crate::types::RuleCategory;

    // T-IC-001
    #[test]
    fn core_loads_universal() {
        let core = IntelligenceCore::new();
        assert!(!core.universal_rules().is_empty());
    }

    // T-IC-002
    #[test]
    fn core_registers_module() {
        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        assert_eq!(core.module_names(), vec!["coding"]);
    }

    // T-IC-003
    #[test]
    fn core_lists_all_perspectives() {
        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        core.register(Box::new(AlgorithmModule));
        let perspectives = core.all_perspectives();
        assert!(perspectives.len() >= 5); // 3 coding + 2 algorithm
    }

    // T-IC-004
    #[test]
    fn core_lists_all_rules() {
        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        let rules = core.all_rules();
        let universal_count = core.universal_rules().len();
        assert!(rules.len() > universal_count); // universal + coding rules
    }

    // T-IC-005
    #[test]
    fn coding_module_has_perspectives() {
        let module = CodingModule;
        assert!(module.perspectives().len() >= 2);
    }

    // T-IC-006
    #[test]
    fn coding_module_has_rules() {
        let module = CodingModule;
        assert!(!module.rules().is_empty());
    }

    // T-IC-007
    #[test]
    fn algorithm_module_has_perspectives() {
        let module = AlgorithmModule;
        assert!(module.perspectives().len() >= 1);
    }

    // T-IC-008
    #[test]
    fn modules_composable() {
        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        core.register(Box::new(AlgorithmModule));
        let names = core.module_names();
        assert!(names.contains(&"coding"));
        assert!(names.contains(&"algorithm"));
        // Both modules' perspectives present
        let perspectives = core.all_perspectives();
        let coding_p = perspectives.iter().any(|p| p.name == "code_quality");
        let algo_p = perspectives.iter().any(|p| p.name == "complexity");
        assert!(coding_p && algo_p);
    }

    // T-IC-009 (universal rules immutability — enforced by type system)
    #[test]
    fn universal_rules_are_readonly() {
        let core = IntelligenceCore::new();
        let rules = core.universal_rules();
        // rules is &[Rule], not &mut — cannot modify.
        // This test verifies the API design enforces immutability.
        assert!(!rules.is_empty());
    }

    // T-IC-010
    #[test]
    fn self_diagnostic_goal_drift() {
        let core = IntelligenceCore::new();
        let diagnostics: Vec<_> = core
            .universal_rules()
            .iter()
            .filter(|r| r.category == RuleCategory::Diagnostic)
            .collect();
        assert!(diagnostics.iter().any(|r| r.id == "D-001"));
    }

    // T-IC-011
    #[test]
    fn self_diagnostic_shallow_reasoning() {
        let core = IntelligenceCore::new();
        let diagnostics: Vec<_> = core
            .universal_rules()
            .iter()
            .filter(|r| r.category == RuleCategory::Diagnostic)
            .collect();
        assert!(diagnostics.iter().any(|r| r.id == "D-002"));
    }

    // T-IC-012
    #[test]
    fn self_diagnostic_shortcut() {
        let core = IntelligenceCore::new();
        let diagnostics: Vec<_> = core
            .universal_rules()
            .iter()
            .filter(|r| r.category == RuleCategory::Diagnostic)
            .collect();
        assert!(diagnostics.iter().any(|r| r.id == "D-003"));
    }
}
