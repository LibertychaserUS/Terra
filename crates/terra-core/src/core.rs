use crate::domain::DomainModule;
use crate::framework::UniversalFramework;
use crate::types::{Perspective, Rule, RuleCategory, Severity, TaskType};

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

    /// Filter rules applicable to a specific task type.
    ///
    /// All MUST-severity rules always apply. SHOULD/MAY rules
    /// are filtered by task-type relevance:
    /// - BugFix: B-002 (minimal change), R-003 (evidence), C-001 (no unwrap)
    /// - NewFeature: R-002 (decompose), B-001 (elegant), A-001 (complexity)
    /// - Refactor: B-001 (elegant), B-005 (consistency), C-003 (naming)
    /// - Review: R-003 (evidence), B-004 (verify), D-001/D-002/D-003 (diagnostics)
    /// - Research: R-001 (understand first), R-004 (acknowledge unknowns)
    pub fn rules_for_task(&self, task_type: &TaskType) -> Vec<Rule> {
        let all = self.all_rules();

        all.into_iter()
            .filter(|rule| {
                // MUST rules always apply
                if rule.severity == Severity::Must {
                    return true;
                }
                // Diagnostic rules always apply
                if rule.category == RuleCategory::Diagnostic {
                    return true;
                }
                // SHOULD/MAY filtered by task relevance
                match task_type {
                    TaskType::BugFix => matches!(
                        rule.id.as_str(),
                        "R-003" | "R-005" | "B-002" | "B-005" | "C-001"
                    ),
                    TaskType::NewFeature => matches!(
                        rule.id.as_str(),
                        "R-002" | "R-005" | "B-001" | "B-005" | "A-001" | "C-002"
                    ),
                    TaskType::Refactor => matches!(
                        rule.id.as_str(),
                        "B-001" | "B-005" | "C-002" | "C-003"
                    ),
                    TaskType::Review => matches!(
                        rule.id.as_str(),
                        "R-003" | "R-005" | "B-005"
                    ),
                    TaskType::Research => matches!(
                        rule.id.as_str(),
                        "R-001" | "R-004" | "R-005"
                    ),
                }
            })
            .collect()
    }

    /// Build a system prompt section containing all applicable IC rules.
    ///
    /// This is injected into the LLM system prompt so rules are active,
    /// not just stored data. Format:
    /// ```text
    /// ## Intelligence Core Rules
    ///
    /// [MUST] R-001: Understand before acting...
    /// [SHOULD] R-005: Depth-first...
    /// ```
    pub fn build_system_prompt(&self, task_type: &TaskType) -> String {
        let rules = self.rules_for_task(task_type);
        let mut lines = Vec::with_capacity(rules.len() + 2);

        lines.push("## Intelligence Core Rules\n".to_string());

        for rule in &rules {
            lines.push(format!(
                "[{}] {}: {}",
                rule.severity_str(),
                rule.id,
                rule.description
            ));
        }

        lines.join("\n")
    }

    /// Generate queries from all perspectives for Multi-Head search.
    ///
    /// Expands each perspective's query_template with the given context,
    /// returning the list of expanded queries for VectorIndex.multi_head_search().
    pub fn generate_queries(&self, context: &str) -> Vec<String> {
        self.all_perspectives()
            .iter()
            .filter_map(|p| p.generate_query(context))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AlgorithmModule, CodingModule};
    use crate::types::RuleCategory;

    fn full_core() -> IntelligenceCore {
        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        core.register(Box::new(AlgorithmModule));
        core
    }

    // T-IC-001
    #[test]
    fn core_loads_universal() {
        let core = IntelligenceCore::new();
        assert!(!core.universal_rules().is_empty());
    }

    // T-IC-002: universal readonly (enforced by type system — &[Rule] not &mut)
    #[test]
    fn universal_rules_are_readonly() {
        let core = IntelligenceCore::new();
        let rules = core.universal_rules();
        assert!(!rules.is_empty());
    }

    // T-IC-003: module registration
    #[test]
    fn core_registers_module() {
        let mut core = IntelligenceCore::new();
        core.register(Box::new(CodingModule));
        assert_eq!(core.module_names(), vec!["coding"]);
    }

    // T-IC-004: modules composable
    #[test]
    fn modules_composable() {
        let core = full_core();
        let names = core.module_names();
        assert!(names.contains(&"coding"));
        assert!(names.contains(&"algorithm"));
        let perspectives = core.all_perspectives();
        let coding_p = perspectives.iter().any(|p| p.name == "code_quality");
        let algo_p = perspectives.iter().any(|p| p.name == "complexity");
        assert!(coding_p && algo_p);
    }

    // T-IC-005: perspectives for multihead
    #[test]
    fn coding_module_has_perspectives() {
        let module = CodingModule;
        let perspectives = module.perspectives();
        assert!(perspectives.len() >= 3);
        // Each has a query_template
        for p in &perspectives {
            assert!(!p.query_template.is_empty());
        }
    }

    // T-IC-006: rules_filtered_by_task
    #[test]
    fn rules_filtered_by_task() {
        let core = full_core();

        // Bug fix task → must include B-002 (minimal change)
        let bugfix_rules = core.rules_for_task(&TaskType::BugFix);
        assert!(
            bugfix_rules.iter().any(|r| r.id == "B-002"),
            "bug fix should include B-002 (minimal change)"
        );
        // All MUST rules present
        assert!(bugfix_rules.iter().all(|r| {
            r.severity == Severity::Must
                || r.category == RuleCategory::Diagnostic
                || matches!(
                    r.id.as_str(),
                    "R-003" | "R-005" | "B-002" | "B-005" | "C-001"
                )
        }));

        // New feature → different set
        let feature_rules = core.rules_for_task(&TaskType::NewFeature);
        assert!(
            feature_rules.iter().any(|r| r.id == "B-001"),
            "new feature should include B-001 (elegant)"
        );
    }

    // T-IC-007: rules_injected_into_prompt
    #[test]
    fn rules_injected_into_prompt() {
        let core = full_core();
        let prompt = core.build_system_prompt(&TaskType::BugFix);

        assert!(prompt.contains("Intelligence Core Rules"));
        assert!(prompt.contains("[MUST]"));
        assert!(prompt.contains("B-002"));
        assert!(prompt.contains("Minimal change"));
        // Diagnostic rules also present
        assert!(prompt.contains("D-001"));
    }

    // T-IC-008 (already tested in diagnostic.rs)

    // T-IC-009 (already tested in diagnostic.rs)

    // T-IC-010 (already tested in diagnostic.rs)

    // T-IC-011: coding_module_has_rules
    #[test]
    fn coding_module_has_rules() {
        let module = CodingModule;
        assert!(module.rules().len() >= 3);
    }

    // T-IC-012: algorithm_module_has_perspectives
    #[test]
    fn algorithm_module_has_perspectives() {
        let module = AlgorithmModule;
        assert!(module.perspectives().len() >= 2);
    }

    // T-IC-013: perspective generates query
    #[test]
    fn perspective_generates_query() {
        let core = full_core();
        let perspectives = core.all_perspectives();
        let p = &perspectives[0];
        let query = p.generate_query("fix authentication bug");
        assert!(query.is_some());
        let q = query.unwrap();
        assert!(!q.is_empty());
        assert!(q.contains("fix authentication bug"));
        assert!(!q.contains("{context}")); // template expanded
    }

    #[test]
    fn perspective_empty_context_returns_none() {
        use crate::types::Perspective;
        let p = Perspective {
            name: "test".into(),
            description: "test".into(),
            query_template: "Query about: {context}".into(),
        };
        assert!(p.generate_query("").is_none());
    }

    // T-IC-014: all_rules includes universal and modules
    #[test]
    fn all_rules_includes_universal_and_modules() {
        let core = full_core();
        let all = core.all_rules();
        let universal_count = core.universal_rules().len();
        assert!(
            all.len() > universal_count,
            "all_rules ({}) must exceed universal_rules ({})",
            all.len(),
            universal_count
        );
    }

    // T-IC-015 (tested in diagnostic.rs)

    // Additional: generate_queries for multi-head search
    #[test]
    fn generate_queries_produces_expanded_queries() {
        let core = full_core();
        let queries = core.generate_queries("implement login feature");
        assert!(queries.len() >= 5); // 3 coding + 2 algorithm perspectives
        for q in &queries {
            assert!(q.contains("implement login feature"));
        }
    }

    // Self-diagnostic rule presence (from Phase 1 tests)
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
