use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Spec IR — the standardized intermediate representation
/// output by the Language Compilation Layer.
///
/// All downstream operations (Plan, Code, Test) consume this
/// shared representation, ensuring terminology consistency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    /// Full 5-pass compilation.
    ///
    /// Passes 1-2: program-driven (lexical + terminology). Zero token cost.
    /// Passes 3-5: LLM-assisted when `llm_callback` is Some. Graceful fallback otherwise.
    ///
    /// `llm_callback`: takes a prompt string, returns LLM response string.
    /// Pass None to skip LLM passes (degrade to Pass 1-2 only).
    pub fn compile_full(
        description: &str,
        term_table: &TermTable,
        llm_callback: Option<&dyn Fn(&str) -> Option<String>>,
    ) -> Self {
        // Pass 1: Lexical — extract file paths and line numbers
        let (file, line) = lexical_extract(description);

        // Pass 2: Terminology normalization
        let resolved_terms = terminology_pass(description, term_table);

        let mut open_questions = Vec::new();
        let mut target = Target {
            file: file.clone(),
            line,
            function: None,
            component: None,
            scope: if line.is_some() { "line".into() } else { "unknown".into() },
        };

        if file.is_none() {
            open_questions.push("Target file not specified".into());
        }

        // Pass 3: Semantic Resolution (LLM-assisted)
        if let Some(callback) = llm_callback {
            let prompt = format!(
                "Given this task description, identify the specific project files, \
                 functions, or components being referenced. Return JSON: \
                 {{\"file\": \"...\", \"function\": \"...\", \"component\": \"...\", \"scope\": \"...\"}}\n\
                 Description: {}",
                description
            );
            if let Some(response) = callback(&prompt) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response) {
                    if let Some(f) = parsed.get("file").and_then(|v| v.as_str()) {
                        if !f.is_empty() {
                            target.file = Some(f.to_string());
                            // Remove the "not specified" question if we resolved it
                            open_questions.retain(|q: &String| !q.contains("Target file"));
                        }
                    }
                    if let Some(func) = parsed.get("function").and_then(|v| v.as_str()) {
                        if !func.is_empty() {
                            target.function = Some(func.to_string());
                        }
                    }
                    if let Some(comp) = parsed.get("component").and_then(|v| v.as_str()) {
                        if !comp.is_empty() {
                            target.component = Some(comp.to_string());
                        }
                    }
                    if let Some(scope) = parsed.get("scope").and_then(|v| v.as_str()) {
                        if !scope.is_empty() {
                            target.scope = scope.to_string();
                        }
                    }
                }
            }
        }

        // Pass 4: Completeness Check (LLM-assisted)
        if let Some(callback) = llm_callback {
            let prompt = format!(
                "Analyze this task for missing information. Return JSON: \
                 {{\"missing\": [\"question 1\", \"question 2\"]}}\n\
                 Task: {}\nTarget: {:?}\nTerms: {:?}",
                description, target.file, resolved_terms
            );
            if let Some(response) = callback(&prompt) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response) {
                    if let Some(missing) = parsed.get("missing").and_then(|v| v.as_array()) {
                        for q in missing {
                            if let Some(s) = q.as_str() {
                                if !s.is_empty() {
                                    open_questions.push(s.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Pass 5: Ambiguity Detection (LLM-assisted)
        if let Some(callback) = llm_callback {
            let prompt = format!(
                "Check for ambiguous references in this task. Flag words like \
                 'it', 'that', 'the thing', 'this'. Return JSON: \
                 {{\"ambiguous\": [\"term1\", \"term2\"]}}\n\
                 Task: {}",
                description
            );
            if let Some(response) = callback(&prompt) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response) {
                    if let Some(ambiguous) = parsed.get("ambiguous").and_then(|v| v.as_array()) {
                        for term in ambiguous {
                            if let Some(s) = term.as_str() {
                                if !s.is_empty() {
                                    open_questions.push(format!("Ambiguous reference: '{}'", s));
                                }
                            }
                        }
                    }
                }
            }
        }

        Self {
            spec_id: Uuid::new_v4().to_string(),
            intent: description.to_string(),
            target,
            constraints: Vec::new(),
            success_predicate: Vec::new(),
            open_questions,
            resolved_terms,
        }
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

    // --- 5-pass compilation tests ---

    // Pass 3: Semantic resolution with LLM callback
    #[test]
    fn pass3_semantic_resolution_with_llm() {
        let table = TermTable::default_coding();
        let mock_llm = |prompt: &str| -> Option<String> {
            if prompt.contains("identify the specific") {
                Some(r#"{"file": "src/auth.rs", "function": "validate_password", "component": "", "scope": "auth_module"}"#.into())
            } else {
                None
            }
        };

        let spec = SpecIR::compile_full(
            "fix the auth module validation",
            &table,
            Some(&mock_llm),
        );

        assert_eq!(spec.target.file.as_deref(), Some("src/auth.rs"));
        assert_eq!(spec.target.function.as_deref(), Some("validate_password"));
        assert_eq!(spec.target.scope, "auth_module");
    }

    // Pass 3: Falls back without LLM
    #[test]
    fn pass3_fallback_without_llm() {
        let table = TermTable::empty();
        let spec = SpecIR::compile_full("fix the auth module", &table, None);
        // No LLM → no semantic resolution → file still unknown
        assert!(spec.target.file.is_none());
        assert!(!spec.is_complete());
    }

    // Pass 4: Completeness check generates questions
    #[test]
    fn pass4_completeness_generates_questions() {
        let table = TermTable::empty();
        let mock_llm = |prompt: &str| -> Option<String> {
            if prompt.contains("missing information") {
                Some(r#"{"missing": ["Which branch should this fix target?", "Should tests be updated?"]}"#.into())
            } else if prompt.contains("identify the specific") {
                Some(r#"{"file": "src/main.rs", "function": "", "component": "", "scope": ""}"#.into())
            } else {
                None
            }
        };

        let spec = SpecIR::compile_full("fix the bug", &table, Some(&mock_llm));
        assert!(
            spec.open_questions.iter().any(|q| q.contains("branch")),
            "should have completeness question, got: {:?}", spec.open_questions
        );
    }

    // Pass 5: Ambiguity detection flags vague terms
    #[test]
    fn pass5_ambiguity_detection() {
        let table = TermTable::empty();
        let mock_llm = |prompt: &str| -> Option<String> {
            if prompt.contains("ambiguous") {
                Some(r#"{"ambiguous": ["it", "that thing"]}"#.into())
            } else if prompt.contains("identify") {
                Some(r#"{"file": "", "function": "", "component": "", "scope": ""}"#.into())
            } else if prompt.contains("missing") {
                Some(r#"{"missing": []}"#.into())
            } else {
                None
            }
        };

        let spec = SpecIR::compile_full(
            "fix it because that thing is broken",
            &table,
            Some(&mock_llm),
        );
        assert!(
            spec.open_questions.iter().any(|q| q.contains("Ambiguous")),
            "should flag ambiguous terms, got: {:?}", spec.open_questions
        );
    }

    // Full 5-pass: all passes together
    #[test]
    fn full_5pass_compilation() {
        let table = TermTable::default_coding();
        let call_count = std::cell::Cell::new(0u32);

        let mock_llm = |prompt: &str| -> Option<String> {
            call_count.set(call_count.get() + 1);
            if prompt.contains("identify") {
                Some(r#"{"file": "src/auth.rs", "function": "login", "component": "", "scope": "auth"}"#.into())
            } else if prompt.contains("missing") {
                Some(r#"{"missing": []}"#.into())
            } else if prompt.contains("ambiguous") {
                Some(r#"{"ambiguous": []}"#.into())
            } else {
                None
            }
        };

        let spec = SpecIR::compile_full(
            "fix the login bug in auth module",
            &table,
            Some(&mock_llm),
        );

        // All 3 LLM passes called
        assert_eq!(call_count.get(), 3, "should call LLM for passes 3, 4, 5");
        // Pass 1+2 (program): term resolved
        assert!(!spec.resolved_terms.is_empty() || spec.target.file.is_some());
        // Pass 3 (LLM): file resolved
        assert_eq!(spec.target.file.as_deref(), Some("src/auth.rs"));
        // Pass 4+5: no issues found → complete
        assert!(spec.is_complete() || spec.open_questions.is_empty());
    }

    // Graceful degradation: LLM callback returns None
    #[test]
    fn llm_returns_none_graceful() {
        let table = TermTable::empty();
        let broken_llm = |_prompt: &str| -> Option<String> { None };

        let spec = SpecIR::compile_full("fix bug", &table, Some(&broken_llm));
        // Should not panic, falls back to Pass 1-2 results
        assert_eq!(spec.intent, "fix bug");
    }

    // LLM returns invalid JSON — no crash
    #[test]
    fn llm_returns_invalid_json() {
        let table = TermTable::empty();
        let bad_llm = |_prompt: &str| -> Option<String> {
            Some("this is not json".into())
        };

        let spec = SpecIR::compile_full("fix bug", &table, Some(&bad_llm));
        // Invalid JSON silently ignored, falls back
        assert_eq!(spec.intent, "fix bug");
    }
}
