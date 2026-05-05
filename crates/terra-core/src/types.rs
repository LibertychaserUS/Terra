use serde::{Deserialize, Serialize};

/// Severity of a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Must,
    Should,
    May,
}

/// Category of a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleCategory {
    Reasoning,
    Behavioral,
    Diagnostic,
}

/// A single rule in the Intelligence Core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub category: RuleCategory,
    pub description: String,
    pub severity: Severity,
}

/// A perspective for Multi-Head Attention retrieval.
///
/// Each Domain Module registers perspectives that generate
/// specialized query vectors for memory search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Perspective {
    pub name: String,
    pub description: String,
    pub query_template: String,
}
