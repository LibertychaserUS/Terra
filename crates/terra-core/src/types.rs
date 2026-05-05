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
    /// For diagnostic rules: measurable threshold below which
    /// the diagnostic signal fires. None for non-diagnostic rules.
    pub threshold: Option<DiagnosticThreshold>,
}

/// Measurable threshold for self-diagnostic rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticThreshold {
    /// The metric name (e.g., "cosine_similarity", "evidence_ratio", "step_completion_ratio")
    pub metric: String,
    /// The threshold value below which the signal fires
    pub value: f64,
    /// Human-readable explanation
    pub description: String,
}

impl Rule {
    pub fn severity_str(&self) -> &str {
        match self.severity {
            Severity::Must => "MUST",
            Severity::Should => "SHOULD",
            Severity::May => "MAY",
        }
    }
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

impl Perspective {
    /// Expand the query template with the given context.
    ///
    /// Replaces `{context}` in the template with the actual context string.
    /// Returns None if the template is empty or context is empty.
    pub fn generate_query(&self, context: &str) -> Option<String> {
        if self.query_template.is_empty() || context.is_empty() {
            return None;
        }
        Some(self.query_template.replace("{context}", context))
    }
}

/// Task type for rule filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskType {
    BugFix,
    NewFeature,
    Refactor,
    Review,
    Research,
}
