use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Spec IR — the standardized intermediate representation
/// output by the Language Compilation Layer.
///
/// All downstream operations (Plan, Code, Test) consume this
/// shared representation, ensuring terminology consistency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecIR {
    pub spec_id: String,
    pub intent: String,
    pub target: Target,
    pub constraints: Vec<String>,
    pub success_predicate: Vec<String>,
    pub open_questions: Vec<String>,
    pub resolved_terms: Vec<TermMapping>,
}

/// The target of an operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    pub scope: String,
}

/// A mapping from user terminology to normalized terminology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermMapping {
    pub user_term: String,
    pub normalized: String,
}

impl SpecIR {
    /// Create a SpecIR from a simple task description.
    ///
    /// MVP1: minimal compilation — extracts basic structure
    /// without full lexical/semantic analysis (that requires LLM).
    pub fn from_task_description(description: &str) -> Self {
        Self {
            spec_id: Uuid::new_v4().to_string(),
            intent: description.to_string(),
            target: Target {
                file: None,
                line: None,
                function: None,
                component: None,
                scope: "unknown".into(),
            },
            constraints: Vec::new(),
            success_predicate: Vec::new(),
            open_questions: vec!["Target file and scope not specified".into()],
            resolved_terms: Vec::new(),
        }
    }

    /// Check if the spec has unresolved questions.
    pub fn is_complete(&self) -> bool {
        self.open_questions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_simple_description() {
        let spec = SpecIR::from_task_description("fix the login bug");
        assert_eq!(spec.intent, "fix the login bug");
        assert!(!spec.is_complete());
    }

    #[test]
    fn manually_built_spec_is_complete() {
        let spec = SpecIR {
            spec_id: "test-001".into(),
            intent: "fix_bug".into(),
            target: Target {
                file: Some("src/auth.py".into()),
                line: Some(42),
                function: Some("login".into()),
                component: None,
                scope: "auth_module".into(),
            },
            constraints: vec!["pass_existing_tests".into(), "minimal_change".into()],
            success_predicate: vec!["empty password returns error".into()],
            open_questions: vec![],
            resolved_terms: vec![TermMapping {
                user_term: "login bug".into(),
                normalized: "empty_password_not_rejected".into(),
            }],
        };
        assert!(spec.is_complete());
        assert_eq!(spec.constraints.len(), 2);
    }

    #[test]
    fn spec_ir_roundtrip_json() {
        let spec = SpecIR::from_task_description("optimize the query");
        let json = serde_json::to_string(&spec).unwrap();
        let decoded: SpecIR = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.intent, "optimize the query");
    }
}
