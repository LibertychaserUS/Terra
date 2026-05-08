use crate::types::{Perspective, Rule, RuleCategory, Severity};

/// Trait for pluggable domain modules.
///
/// Each module provides domain-specific perspectives, rules,
/// and (future) verifiers. Modules are composable — multiple
/// can be active simultaneously.
pub trait DomainModule: Send + Sync {
    fn name(&self) -> &str;
    fn perspectives(&self) -> Vec<Perspective>;
    fn rules(&self) -> Vec<Rule>;
}

/// Coding domain module — software engineering principles.
pub struct CodingModule;

impl DomainModule for CodingModule {
    fn name(&self) -> &str {
        "coding"
    }

    fn perspectives(&self) -> Vec<Perspective> {
        vec![
            Perspective {
                name: "code_quality".into(),
                description: "Code readability, maintainability, and elegance".into(),
                query_template: "Code quality aspects of: {context}".into(),
            },
            Perspective {
                name: "architecture".into(),
                description: "System design, module boundaries, dependencies".into(),
                query_template: "Architectural implications of: {context}".into(),
            },
            Perspective {
                name: "security".into(),
                description: "Vulnerability patterns, input validation, auth".into(),
                query_template: "Security concerns for: {context}".into(),
            },
        ]
    }

    fn rules(&self) -> Vec<Rule> {
        vec![
            Rule {
                id: "C-001".into(),
                category: RuleCategory::Behavioral,
                description: "No .unwrap() in production code".into(),
                severity: Severity::Must,
                threshold: None,
            },
            Rule {
                id: "C-002".into(),
                category: RuleCategory::Behavioral,
                description: "All public APIs must have documentation".into(),
                severity: Severity::Should,
                threshold: None,
            },
            Rule {
                id: "C-003".into(),
                category: RuleCategory::Behavioral,
                description: "Follow existing project naming conventions".into(),
                severity: Severity::Must,
                threshold: None,
            },
        ]
    }
}

/// Algorithm domain module — complexity and optimization.
pub struct AlgorithmModule;

impl DomainModule for AlgorithmModule {
    fn name(&self) -> &str {
        "algorithm"
    }

    fn perspectives(&self) -> Vec<Perspective> {
        vec![
            Perspective {
                name: "complexity".into(),
                description: "Time and space complexity analysis".into(),
                query_template: "Complexity characteristics of: {context}".into(),
            },
            Perspective {
                name: "optimization".into(),
                description: "Performance bottlenecks and optimization strategies".into(),
                query_template: "Optimization opportunities for: {context}".into(),
            },
        ]
    }

    fn rules(&self) -> Vec<Rule> {
        vec![
            Rule {
                id: "A-001".into(),
                category: RuleCategory::Reasoning,
                description: "State complexity class before implementing".into(),
                severity: Severity::Should,
                threshold: None,
            },
            Rule {
                id: "A-002".into(),
                category: RuleCategory::Reasoning,
                description: "Consider edge cases: empty input, single element, maximum size".into(),
                severity: Severity::Must,
                threshold: None,
            },
        ]
    }
}
