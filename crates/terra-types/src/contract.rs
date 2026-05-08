//! Output Contracts
//!
//! Every ProviderCall can specify an OutputContract that defines
//! what the LLM must return. The contract is:
//! 1. Injected into the system prompt as format specification
//! 2. Used to parse the LLM response
//! 3. Violations → error, never guessing
//!
//! This ensures the LLM→program boundary is typed and validated.

use serde::{Deserialize, Serialize};

/// An output contract defines expected LLM response structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputContract {
    /// Contract name (e.g., "file_write", "task_result", "analysis")
    pub name: String,
    /// Human-readable format instruction injected into prompt
    pub format_instruction: String,
    /// Required top-level JSON keys
    pub required_fields: Vec<String>,
}

/// Result of parsing LLM output against a contract.
#[derive(Debug, Clone)]
pub enum ContractParseResult {
    /// Output matches contract. Parsed value returned.
    Ok(serde_json::Value),
    /// Output is not valid JSON.
    JsonError(String),
    /// Output is valid JSON but missing required fields.
    MissingFields(Vec<String>),
}

impl OutputContract {
    /// Parse LLM text output against this contract.
    ///
    /// Extracts JSON from the response (handles markdown code blocks),
    /// then validates required fields are present.
    pub fn parse(&self, response: &str) -> ContractParseResult {
        // Extract JSON — handle bare JSON or ```json blocks
        let json_text = extract_json(response);

        let value: serde_json::Value = match serde_json::from_str(&json_text) {
            Ok(v) => v,
            Err(e) => return ContractParseResult::JsonError(format!(
                "contract '{}': invalid JSON: {}", self.name, e
            )),
        };

        // Validate required fields
        let missing: Vec<String> = self.required_fields
            .iter()
            .filter(|f| value.get(f.as_str()).is_none())
            .cloned()
            .collect();

        if !missing.is_empty() {
            return ContractParseResult::MissingFields(missing);
        }

        ContractParseResult::Ok(value)
    }

    /// Generate the format instruction to inject into system prompt.
    pub fn prompt_section(&self) -> String {
        format!(
            "## Output Format (STRICT)\n\
             You MUST respond with ONLY a JSON object. No explanation, no markdown.\n\
             Required fields: {}\n\
             \n\
             {}",
            self.required_fields.join(", "),
            self.format_instruction,
        )
    }
}

/// Extract JSON from LLM response.
/// Handles: bare JSON, ```json blocks, text with embedded JSON.
fn extract_json(text: &str) -> String {
    let trimmed = text.trim();

    // Try bare JSON first
    if trimmed.starts_with('{') {
        return trimmed.to_string();
    }

    // Try ```json block
    if let Some(start) = trimmed.find("```json") {
        let after_marker = &trimmed[start + 7..];
        if let Some(end) = after_marker.find("```") {
            return after_marker[..end].trim().to_string();
        }
    }

    // Try ``` block (no language tag)
    if let Some(start) = trimmed.find("```") {
        let after_marker = &trimmed[start + 3..];
        if let Some(end) = after_marker.find("```") {
            let inner = after_marker[..end].trim();
            if inner.starts_with('{') {
                return inner.to_string();
            }
        }
    }

    // Last resort: find first { and last }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            return trimmed[start..=end].to_string();
        }
    }

    trimmed.to_string()
}

// --- Built-in contracts ---

/// Contract for file write operations.
pub fn file_write_contract() -> OutputContract {
    OutputContract {
        name: "file_write".into(),
        format_instruction: r#"Example:
{"action": "file_write", "path": "src/auth.rs", "content": "// file content here"}"#.into(),
        required_fields: vec!["action".into(), "path".into(), "content".into()],
    }
}

/// Contract for task result (SubAgent report).
pub fn task_result_contract() -> OutputContract {
    OutputContract {
        name: "task_result".into(),
        format_instruction: r#"Example:
{"status": "completed", "summary": "Fixed auth.rs line 42", "files_modified": ["src/auth.rs"], "tests_passed": true}"#.into(),
        required_fields: vec!["status".into(), "summary".into()],
    }
}

/// Contract for analysis/research results.
pub fn analysis_contract() -> OutputContract {
    OutputContract {
        name: "analysis".into(),
        format_instruction: r#"Example:
{"findings": ["finding 1", "finding 2"], "recommendations": ["rec 1"]}"#.into(),
        required_fields: vec!["findings".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Valid JSON matching contract → parsed
    #[test]
    fn valid_json_passes_contract() {
        let contract = file_write_contract();
        let response = r#"{"action": "file_write", "path": "src/main.rs", "content": "fn main() {}"}"#;
        let result = contract.parse(response);
        assert!(matches!(result, ContractParseResult::Ok(_)));
        if let ContractParseResult::Ok(v) = result {
            assert_eq!(v["path"], "src/main.rs");
        }
    }

    // Invalid JSON → error
    #[test]
    fn invalid_json_fails() {
        let contract = file_write_contract();
        let result = contract.parse("this is not json at all");
        assert!(matches!(result, ContractParseResult::JsonError(_)));
    }

    // Missing required fields → error
    #[test]
    fn missing_fields_detected() {
        let contract = file_write_contract();
        let response = r#"{"action": "file_write"}"#;
        let result = contract.parse(response);
        assert!(matches!(result, ContractParseResult::MissingFields(_)));
        if let ContractParseResult::MissingFields(missing) = result {
            assert!(missing.contains(&"path".to_string()));
            assert!(missing.contains(&"content".to_string()));
        }
    }

    // JSON in markdown code block extracted
    #[test]
    fn extracts_json_from_code_block() {
        let contract = task_result_contract();
        let response = "Here's the result:\n```json\n{\"status\": \"completed\", \"summary\": \"done\"}\n```\n";
        let result = contract.parse(response);
        assert!(matches!(result, ContractParseResult::Ok(_)));
    }

    // Contract injected into prompt
    #[test]
    fn contract_prompt_section() {
        let contract = file_write_contract();
        let section = contract.prompt_section();
        assert!(section.contains("Output Format"));
        assert!(section.contains("action"));
        assert!(section.contains("path"));
        assert!(section.contains("content"));
        assert!(section.contains("STRICT"));
    }

    // Task result contract
    #[test]
    fn task_result_contract_works() {
        let contract = task_result_contract();
        let response = r#"{"status": "completed", "summary": "Fixed the bug", "files_modified": ["a.rs"]}"#;
        let result = contract.parse(response);
        assert!(matches!(result, ContractParseResult::Ok(_)));
    }

    // Analysis contract with only required field
    #[test]
    fn analysis_contract_minimal() {
        let contract = analysis_contract();
        let response = r#"{"findings": ["found issue in auth"]}"#;
        let result = contract.parse(response);
        assert!(matches!(result, ContractParseResult::Ok(_)));
    }

    // Binary roundtrip: contract → msgpack → back
    #[test]
    fn contract_msgpack_roundtrip() {
        let contract = file_write_contract();
        let bytes = rmp_serde::to_vec_named(&contract).unwrap();
        let decoded: OutputContract = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.name, "file_write");
        assert_eq!(decoded.required_fields.len(), 3);
    }
}
