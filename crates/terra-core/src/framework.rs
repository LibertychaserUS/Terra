use crate::types::{DiagnosticThreshold, Rule, RuleCategory, Severity};

/// The Universal Framework — L0 read-only rules.
///
/// These are the agent's first principles. They cannot be
/// modified at runtime (enforced by returning owned Vec, not &mut).
pub struct UniversalFramework {
    rules: Vec<Rule>,
}

impl UniversalFramework {
    /// Create the universal framework with all built-in rules.
    pub fn new() -> Self {
        Self {
            rules: vec![
                // --- Reasoning Framework ---
                Rule {
                    id: "R-001".into(),
                    category: RuleCategory::Reasoning,
                    description: "Understand before acting: read existing code before modifying".into(),
                    severity: Severity::Must,
                    threshold: None,
                },
                Rule {
                    id: "R-002".into(),
                    category: RuleCategory::Reasoning,
                    description: "Decompose complex problems: never attempt to solve in one step".into(),
                    severity: Severity::Must,
                    threshold: None,
                },
                Rule {
                    id: "R-003".into(),
                    category: RuleCategory::Reasoning,
                    description: "Seek evidence, not confidence: verify, don't believe".into(),
                    severity: Severity::Must,
                    threshold: None,
                },
                Rule {
                    id: "R-004".into(),
                    category: RuleCategory::Reasoning,
                    description: "Acknowledge unknowns: never fabricate information".into(),
                    severity: Severity::Must,
                    threshold: None,
                },
                Rule {
                    id: "R-005".into(),
                    category: RuleCategory::Reasoning,
                    description: "Depth-first: push one direction to completion before branching".into(),
                    severity: Severity::Should,
                    threshold: None,
                },

                // --- Behavioral Framework ---
                Rule {
                    id: "B-001".into(),
                    category: RuleCategory::Behavioral,
                    description: "Elegant over complex: if 3 lines suffice, don't write 30".into(),
                    severity: Severity::Must,
                    threshold: None,
                },
                Rule {
                    id: "B-002".into(),
                    category: RuleCategory::Behavioral,
                    description: "Minimal change: modify only what the task requires".into(),
                    severity: Severity::Must,
                    threshold: None,
                },
                Rule {
                    id: "B-003".into(),
                    category: RuleCategory::Behavioral,
                    description: "Safe defaults: never introduce security vulnerabilities".into(),
                    severity: Severity::Must,
                    threshold: None,
                },
                Rule {
                    id: "B-004".into(),
                    category: RuleCategory::Behavioral,
                    description: "Verify before claiming: external verification must pass".into(),
                    severity: Severity::Must,
                    threshold: None,
                },
                Rule {
                    id: "B-005".into(),
                    category: RuleCategory::Behavioral,
                    description: "Consistency: follow the project's existing patterns".into(),
                    severity: Severity::Should,
                    threshold: None,
                },

                // --- Self-Diagnostic Rules (with measurable thresholds) ---
                Rule {
                    id: "D-001".into(),
                    category: RuleCategory::Diagnostic,
                    description: "Detect goal drift: am I still moving toward the stated objective?".into(),
                    severity: Severity::Must,
                    threshold: Some(DiagnosticThreshold {
                        metric: "cosine_similarity".into(),
                        value: 0.6,
                        description: "Action-to-spec embedding similarity must be >= 0.6".into(),
                    }),
                },
                Rule {
                    id: "D-002".into(),
                    category: RuleCategory::Diagnostic,
                    description: "Detect shallow reasoning: is my analysis sufficiently deep?".into(),
                    severity: Severity::Must,
                    threshold: Some(DiagnosticThreshold {
                        metric: "evidence_ratio".into(),
                        value: 0.3,
                        description: "Evidence-to-claim ratio must be >= 0.3".into(),
                    }),
                },
                Rule {
                    id: "D-003".into(),
                    category: RuleCategory::Diagnostic,
                    description: "Detect shortcuts: am I bypassing verification or safety steps?".into(),
                    severity: Severity::Must,
                    threshold: Some(DiagnosticThreshold {
                        metric: "step_completion_ratio".into(),
                        value: 1.0,
                        description: "All planned verification steps must be executed".into(),
                    }),
                },
            ],
        }
    }

    /// Get all rules. Returns owned copy (immutable from caller's perspective).
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn universal_has_rules() {
        let fw = UniversalFramework::new();
        assert!(fw.rules().len() >= 10);
    }

    #[test]
    fn universal_has_reasoning_rules() {
        let fw = UniversalFramework::new();
        let reasoning: Vec<_> = fw.rules().iter()
            .filter(|r| r.category == RuleCategory::Reasoning)
            .collect();
        assert!(reasoning.len() >= 3);
    }

    #[test]
    fn universal_has_behavioral_rules() {
        let fw = UniversalFramework::new();
        let behavioral: Vec<_> = fw.rules().iter()
            .filter(|r| r.category == RuleCategory::Behavioral)
            .collect();
        assert!(behavioral.len() >= 3);
    }

    #[test]
    fn universal_has_diagnostic_rules() {
        let fw = UniversalFramework::new();
        let diagnostic: Vec<_> = fw.rules().iter()
            .filter(|r| r.category == RuleCategory::Diagnostic)
            .collect();
        assert_eq!(diagnostic.len(), 3);
    }
}
