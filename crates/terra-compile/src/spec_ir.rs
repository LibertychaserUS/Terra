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

    // T-C-001: from_description sets intent
    #[test]
    fn from_description_sets_intent() {
        let spec = SpecIR::from_task_description("fix the login bug");
        assert_eq!(spec.intent, "fix the login bug");
    }

    // T-C-002: from_description is incomplete
    #[test]
    fn from_description_is_incomplete() {
        let spec = SpecIR::from_task_description("fix bug");
        assert!(!spec.is_complete());
        assert!(!spec.open_questions.is_empty());
    }

    // T-C-003: manually built spec is complete
    #[test]
    fn manual_spec_is_complete() {
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

    // T-C-004: JSON roundtrip
    #[test]
    fn json_roundtrip() {
        let spec = SpecIR::from_task_description("optimize the query");
        let json = serde_json::to_string(&spec).unwrap();
        let decoded: SpecIR = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.intent, "optimize the query");
        assert_eq!(decoded.spec_id, spec.spec_id);
    }

    // T-C-005: MessagePack roundtrip
    #[test]
    fn msgpack_roundtrip() {
        let spec = SpecIR::from_task_description("refactor module");
        let bytes = terra_protocol::codec::encode(&spec).unwrap();
        let decoded: SpecIR = terra_protocol::codec::decode(&bytes).unwrap();
        assert_eq!(decoded.intent, "refactor module");
        assert_eq!(decoded.spec_id, spec.spec_id);
    }

    // T-C-006: target with file and line
    #[test]
    fn target_file_and_line() {
        let target = Target {
            file: Some("src/auth.py".into()),
            line: Some(42),
            function: None,
            component: None,
            scope: "module".into(),
        };
        assert_eq!(target.file.as_deref(), Some("src/auth.py"));
        assert_eq!(target.line, Some(42));
    }

    // T-C-007: target with component
    #[test]
    fn target_component() {
        let target = Target {
            file: None,
            line: None,
            function: None,
            component: Some("LoginButton".into()),
            scope: "ui".into(),
        };
        assert_eq!(target.component.as_deref(), Some("LoginButton"));
    }

    // T-C-008: multiple constraints
    #[test]
    fn multiple_constraints() {
        let spec = SpecIR {
            spec_id: "t".into(),
            intent: "t".into(),
            target: Target { file: None, line: None, function: None, component: None, scope: "t".into() },
            constraints: vec!["a".into(), "b".into(), "c".into()],
            success_predicate: vec![],
            open_questions: vec![],
            resolved_terms: vec![],
        };
        assert_eq!(spec.constraints.len(), 3);
    }

    // T-C-009: success predicate
    #[test]
    fn success_predicate() {
        let spec = SpecIR {
            spec_id: "t".into(),
            intent: "t".into(),
            target: Target { file: None, line: None, function: None, component: None, scope: "t".into() },
            constraints: vec![],
            success_predicate: vec!["test passes".into(), "no regression".into()],
            open_questions: vec![],
            resolved_terms: vec![],
        };
        assert_eq!(spec.success_predicate.len(), 2);
    }

    // T-C-010: term mapping
    #[test]
    fn term_mapping() {
        let mapping = TermMapping {
            user_term: "login bug".into(),
            normalized: "empty_password".into(),
        };
        let json = serde_json::to_string(&mapping).unwrap();
        let decoded: TermMapping = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.user_term, "login bug");
        assert_eq!(decoded.normalized, "empty_password");
    }

    // T-C-011: empty description
    #[test]
    fn empty_description() {
        let spec = SpecIR::from_task_description("");
        assert_eq!(spec.intent, "");
        // Should not panic
    }

    // T-C-012: unicode description
    #[test]
    fn unicode_description() {
        let spec = SpecIR::from_task_description("修复登录问题 🐛");
        assert_eq!(spec.intent, "修复登录问题 🐛");
    }

    // T-C-013: long description
    #[test]
    fn long_description() {
        let long = "x".repeat(10_000);
        let spec = SpecIR::from_task_description(&long);
        assert_eq!(spec.intent.len(), 10_000);
    }

    // T-C-014: spec_id is UUID format
    #[test]
    fn spec_id_format() {
        let spec = SpecIR::from_task_description("test");
        // UUID v4 format: 8-4-4-4-12 hex chars
        assert_eq!(spec.spec_id.len(), 36);
        assert_eq!(spec.spec_id.chars().filter(|c| *c == '-').count(), 4);
    }

    // T-C-015: multiple open questions
    #[test]
    fn multiple_open_questions() {
        let spec = SpecIR {
            spec_id: "t".into(),
            intent: "t".into(),
            target: Target { file: None, line: None, function: None, component: None, scope: "t".into() },
            constraints: vec![],
            success_predicate: vec![],
            open_questions: vec!["q1".into(), "q2".into(), "q3".into()],
            resolved_terms: vec![],
        };
        assert_eq!(spec.open_questions.len(), 3);
        assert!(!spec.is_complete());
    }
}
