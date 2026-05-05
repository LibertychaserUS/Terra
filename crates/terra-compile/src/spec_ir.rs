use std::collections::HashMap;

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

/// Terminology normalization table.
///
/// Maps informal user terms to canonical forms used in Spec IR.
/// This ensures all downstream systems use consistent vocabulary.
pub struct TermTable {
    mappings: HashMap<String, String>,
}

impl TermTable {
    /// Create a new term table with default coding-domain mappings.
    pub fn default_coding() -> Self {
        let mut mappings = HashMap::new();
        mappings.insert("login bug".into(), "authentication_failure".into());
        mappings.insert("login error".into(), "authentication_failure".into());
        mappings.insert("auth bug".into(), "authentication_failure".into());
        mappings.insert("crash".into(), "unhandled_exception".into());
        mappings.insert("slow".into(), "performance_degradation".into());
        mappings.insert("performance issue".into(), "performance_degradation".into());
        mappings.insert("broken".into(), "functional_regression".into());
        mappings.insert("doesn't work".into(), "functional_regression".into());
        mappings.insert("memory leak".into(), "resource_leak_memory".into());
        mappings.insert("race condition".into(), "concurrency_violation".into());
        mappings.insert("deadlock".into(), "concurrency_deadlock".into());
        mappings.insert("security hole".into(), "security_vulnerability".into());
        mappings.insert("xss".into(), "security_xss".into());
        mappings.insert("sql injection".into(), "security_sqli".into());
        Self { mappings }
    }

    /// Create an empty term table.
    pub fn empty() -> Self {
        Self { mappings: HashMap::new() }
    }

    /// Normalize a term. Returns the canonical form if found,
    /// otherwise returns the original term lowercase.
    pub fn normalize(&self, term: &str) -> String {
        let lower = term.to_lowercase();
        self.mappings.get(&lower).cloned().unwrap_or(lower)
    }

    /// Add a custom mapping.
    pub fn insert(&mut self, user_term: &str, normalized: &str) {
        self.mappings.insert(user_term.to_lowercase(), normalized.into());
    }
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

    /// Compile a task description through the lexical pass.
    ///
    /// This runs entity extraction (files, line numbers) and
    /// terminology normalization against the provided term table.
    pub fn compile(description: &str, term_table: &TermTable) -> Self {
        let spec_id = Uuid::new_v4().to_string();

        // Pass 1: Lexical — extract file paths and line numbers
        let (file, line) = lexical_extract(description);

        // Pass 2: Terminology normalization — map informal terms
        let resolved_terms = terminology_pass(description, term_table);

        // Build open questions based on what's missing
        let mut open_questions = Vec::new();
        if file.is_none() {
            open_questions.push("Target file not specified".into());
        }

        Self {
            spec_id,
            intent: description.to_string(),
            target: Target {
                file,
                line,
                function: None,
                component: None,
                scope: if line.is_some() { "line".into() } else { "unknown".into() },
            },
            constraints: Vec::new(),
            success_predicate: Vec::new(),
            open_questions,
            resolved_terms,
        }
    }

    /// Check if the spec has unresolved questions.
    pub fn is_complete(&self) -> bool {
        self.open_questions.is_empty()
    }
}

/// Lexical pass: extract file paths and line numbers from description.
///
/// Recognizes patterns like:
/// - "auth.py" (file with extension)
/// - "line 42" or "line:42" (line number)
/// - "src/auth.py:42" (file:line)
fn lexical_extract(description: &str) -> (Option<String>, Option<u32>) {
    let mut file = None;
    let mut line = None;

    let words: Vec<&str> = description.split_whitespace().collect();

    for (i, word) in words.iter().enumerate() {
        // Check for file:line pattern (e.g., "src/auth.py:42")
        if let Some(colon_pos) = word.rfind(':') {
            let (path_part, num_part) = word.split_at(colon_pos);
            let num_part = &num_part[1..]; // skip ':'
            if looks_like_file(path_part) {
                file = Some(path_part.to_string());
                if let Ok(n) = num_part.parse::<u32>() {
                    line = Some(n);
                }
                continue;
            }
        }

        // Check for standalone file patterns
        if file.is_none() && looks_like_file(word) {
            file = Some(word.to_string());
        }

        // Check for "line N" pattern
        if word.eq_ignore_ascii_case("line") {
            if let Some(next) = words.get(i + 1) {
                if let Ok(n) = next.parse::<u32>() {
                    line = Some(n);
                }
            }
        }
    }

    (file, line)
}

/// Check if a word looks like a file path (has a common extension).
fn looks_like_file(word: &str) -> bool {
    let extensions = [
        ".rs", ".py", ".js", ".ts", ".tsx", ".jsx", ".go", ".java",
        ".c", ".cpp", ".h", ".hpp", ".rb", ".ex", ".erl", ".zig",
        ".toml", ".yaml", ".yml", ".json", ".md", ".txt", ".sql",
        ".html", ".css", ".sh", ".bash", ".swift", ".kt",
    ];
    let lower = word.to_lowercase();
    extensions.iter().any(|ext| lower.ends_with(ext))
}

/// Terminology pass: find informal terms in description and normalize them.
fn terminology_pass(description: &str, table: &TermTable) -> Vec<TermMapping> {
    let lower = description.to_lowercase();
    let mut found = Vec::new();

    // Check each known term against the description
    for (user_term, normalized) in &table.mappings {
        if lower.contains(user_term.as_str()) {
            found.push(TermMapping {
                user_term: user_term.clone(),
                normalized: normalized.clone(),
            });
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-LC-001: from_description sets intent
    #[test]
    fn from_description_sets_intent() {
        let spec = SpecIR::from_task_description("fix the login bug");
        assert_eq!(spec.intent, "fix the login bug");
    }

    // T-LC-002: from_description is incomplete
    #[test]
    fn from_description_is_incomplete() {
        let spec = SpecIR::from_task_description("fix bug");
        assert!(!spec.is_complete());
        assert!(!spec.open_questions.is_empty());
    }

    // T-LC-003: manually built spec is complete
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

    // T-LC-004: JSON roundtrip
    #[test]
    fn json_roundtrip() {
        let spec = SpecIR::from_task_description("optimize the query");
        let json = serde_json::to_string(&spec).unwrap();
        let decoded: SpecIR = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.intent, "optimize the query");
        assert_eq!(decoded.spec_id, spec.spec_id);
    }

    // T-LC-005: MessagePack roundtrip
    #[test]
    fn msgpack_roundtrip() {
        let spec = SpecIR::from_task_description("refactor module");
        let bytes = terra_protocol::codec::encode(&spec).unwrap();
        let decoded: SpecIR = terra_protocol::codec::decode(&bytes).unwrap();
        assert_eq!(decoded.intent, "refactor module");
        assert_eq!(decoded.spec_id, spec.spec_id);
    }

    // T-LC-006: target with file and line
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

    // T-LC-007: target with component
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

    // T-LC-008: multiple constraints
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

    // T-LC-009: success predicate
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

    // T-LC-010: term mapping
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

    // T-LC-011: empty description
    #[test]
    fn empty_description() {
        let spec = SpecIR::from_task_description("");
        assert_eq!(spec.intent, "");
        // Should not panic
    }

    // T-LC-012: unicode description
    #[test]
    fn unicode_description() {
        let spec = SpecIR::from_task_description("修复登录问题 🐛");
        assert_eq!(spec.intent, "修复登录问题 🐛");
    }

    // T-LC-013: long description
    #[test]
    fn long_description() {
        let long = "x".repeat(10_000);
        let spec = SpecIR::from_task_description(&long);
        assert_eq!(spec.intent.len(), 10_000);
    }

    // T-LC-014: spec_id is UUID format
    #[test]
    fn spec_id_format() {
        let spec = SpecIR::from_task_description("test");
        // UUID v4 format: 8-4-4-4-12 hex chars
        assert_eq!(spec.spec_id.len(), 36);
        assert_eq!(spec.spec_id.chars().filter(|c| *c == '-').count(), 4);
    }

    // T-LC-015: multiple open questions
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

    // T-LC-016: lexical_pass_extracts_entities
    #[test]
    fn lexical_pass_extracts_entities() {
        let table = TermTable::empty();
        let spec = SpecIR::compile("fix auth.py line 42", &table);
        assert_eq!(spec.target.file.as_deref(), Some("auth.py"));
        assert_eq!(spec.target.line, Some(42));
    }

    #[test]
    fn lexical_pass_file_colon_line() {
        let table = TermTable::empty();
        let spec = SpecIR::compile("fix src/auth.py:42 immediately", &table);
        assert_eq!(spec.target.file.as_deref(), Some("src/auth.py"));
        assert_eq!(spec.target.line, Some(42));
    }

    #[test]
    fn lexical_pass_no_file() {
        let table = TermTable::empty();
        let spec = SpecIR::compile("fix the login system", &table);
        assert!(spec.target.file.is_none());
        assert!(!spec.is_complete()); // open question about target
    }

    // T-LC-017: terminology_normalization
    #[test]
    fn terminology_normalization() {
        let table = TermTable::default_coding();
        let spec = SpecIR::compile("there is a login bug in auth.py", &table);
        assert!(
            spec.resolved_terms.iter().any(|t| t.normalized == "authentication_failure"),
            "should normalize 'login bug' to 'authentication_failure', got: {:?}",
            spec.resolved_terms
        );
    }

    #[test]
    fn terminology_custom_mapping() {
        let mut table = TermTable::empty();
        table.insert("flaky test", "test_nondeterminism");
        assert_eq!(table.normalize("flaky test"), "test_nondeterminism");
        assert_eq!(table.normalize("unknown term"), "unknown term");
    }

    // T-LC-018: completeness_blocks_execution
    #[test]
    fn completeness_blocks_execution() {
        // A spec with open questions should not be complete
        let spec = SpecIR::compile("fix the bug", &TermTable::empty());
        assert!(!spec.is_complete(), "spec without file target should be incomplete");

        // Verify this would trigger SpecificationCage block
        // (Integration: incomplete spec → CageAction::ExecuteTask{spec_complete: false} → Block)
        use terra_protocol::command::{Command, CommandOp};
        let _ = Command::new(
            "main",
            CommandOp::TraceEvent {
                event_type: "spec_check".into(),
                data: serde_json::json!({"complete": spec.is_complete()}),
            },
        );
        // The actual cage enforcement is in terra-executor;
        // here we verify the spec correctly reports incompleteness.
    }

    #[test]
    fn compile_with_file_is_complete() {
        let table = TermTable::empty();
        let spec = SpecIR::compile("fix auth.py line 10", &table);
        assert!(spec.is_complete(), "spec with file target should be complete");
    }
}
