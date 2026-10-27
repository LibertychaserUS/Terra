//! The goal's specification: what the task is, what it touches, what counts
//! as done, and what is still unknown.
//!
//! The model writes it (one JSON-mode call through the executor, so it is
//! audited); when the provider cannot do that, a small heuristic stands in.
//! The Specification cage reads `open_questions`: while it is not empty,
//! nothing in the project may change.

use serde::{Deserialize, Serialize};
use terra_executor::CommandExecutor;
use terra_types::command::{Command, CommandOp, CommandStatus};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Spec {
    pub spec_id: String,
    /// The user's words.
    pub description: String,
    /// One sentence: what the task is.
    pub intent: String,
    /// Files, modules or commands the task touches.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Constraints the user stated or that follow from the project.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// How the result will be checked.
    #[serde(default)]
    pub success_criteria: Vec<String>,
    /// What must be answered before the task can start.
    #[serde(default)]
    pub open_questions: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ModelSpec {
    #[serde(default)]
    intent: String,
    #[serde(default)]
    targets: Vec<String>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    success_criteria: Vec<String>,
    #[serde(default)]
    open_questions: Vec<String>,
}

const SPEC_SYSTEM_PROMPT: &str = "You turn a user's task description for a coding agent into a specification. \
Reply with one JSON object and nothing else, with these keys: \
\"intent\" (one sentence), \"targets\" (files, modules or commands the task will touch), \
\"constraints\" (what must not change or must hold), \"success_criteria\" (how the result is checked, e.g. a test command), \
\"open_questions\" (things that must be answered before work can start). \
Leave open_questions empty when a competent engineer could start now; list a question only when the task cannot be done \
without the answer (for example the task names no file, module or behaviour at all). Keep every list short.";

impl Spec {
    /// Specify `description` with the model when it can emit JSON, otherwise
    /// with `Spec::heuristic`.
    pub async fn compile(executor: &CommandExecutor, description: &str) -> Spec {
        let description = description.trim();
        if !executor.provider().capabilities().supports_json_output {
            return Spec::heuristic(description);
        }
        let cmd = Command::new(
            "specifier",
            CommandOp::ProviderCall {
                system_prompt: SPEC_SYSTEM_PROMPT.into(),
                user_prompt: description.to_string(),
                max_tokens: Some(600),
                temperature: Some(0.0),
                json: true,
            },
        );
        let result = executor.execute(cmd).await;
        let parsed = (result.status == CommandStatus::Ok)
            .then(|| result.data.as_ref()?.get("content")?.as_str().map(parse_model_spec))
            .flatten()
            .flatten();
        match parsed {
            Some(model) => Spec {
                spec_id: new_id(),
                description: description.to_string(),
                intent: if model.intent.trim().is_empty() { description.to_string() } else { model.intent },
                targets: model.targets,
                constraints: model.constraints,
                success_criteria: model.success_criteria,
                open_questions: model.open_questions,
            },
            None => Spec::heuristic(description),
        }
    }

    /// No model: the task is specified when it names something concrete, a
    /// path-like token or a quoted name.
    pub fn heuristic(description: &str) -> Spec {
        let targets: Vec<String> = description
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| ",;:()[]{}\"'`".contains(c)))
            .filter(|w| looks_like_target(w))
            .map(String::from)
            .collect();
        let open_questions = if targets.is_empty() { vec!["Target file not specified".to_string()] } else { Vec::new() };
        Spec {
            spec_id: new_id(),
            description: description.to_string(),
            intent: description.to_string(),
            targets,
            constraints: Vec::new(),
            success_criteria: Vec::new(),
            open_questions,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.open_questions.is_empty()
    }

    /// The spec as the model should see it in the system prompt.
    pub fn render(&self) -> String {
        let mut out = format!("Goal: {}\n", self.intent);
        if !self.targets.is_empty() {
            out.push_str(&format!("Targets: {}\n", self.targets.join(", ")));
        }
        if !self.constraints.is_empty() {
            out.push_str(&format!("Constraints: {}\n", self.constraints.join("; ")));
        }
        if !self.success_criteria.is_empty() {
            out.push_str(&format!("Done when: {}\n", self.success_criteria.join("; ")));
        }
        if !self.open_questions.is_empty() {
            out.push_str("Open questions (no file, shell or delete action is allowed until the user answers them):\n");
            for q in &self.open_questions {
                out.push_str(&format!("  - {}\n", q));
            }
        }
        out
    }
}

fn new_id() -> String {
    format!("spec-{}", &uuid::Uuid::new_v4().to_string()[..8])
}

fn looks_like_target(word: &str) -> bool {
    if word.len() < 3 {
        return false;
    }
    let has_ext = word
        .rsplit_once('.')
        .map(|(stem, ext)| {
            !stem.is_empty()
                && ext.len() <= 5
                && ext.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or(false);
    has_ext || word.contains('/') || word.contains("::")
}

fn parse_model_spec(content: &str) -> Option<ModelSpec> {
    let trimmed = content.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(trimmed);
    serde_json::from_str(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_needs_a_concrete_target() {
        let spec = Spec::heuristic("test task");
        assert_eq!(spec.open_questions, ["Target file not specified"]);
        assert!(!spec.is_complete());

        for text in ["add a test to src/lib.rs", "fix main.rs", "document auth::login", "update README.md,"] {
            let spec = Spec::heuristic(text);
            assert!(spec.is_complete(), "{text}: {:?}", spec.open_questions);
            assert!(!spec.targets.is_empty());
        }
        assert!(!Spec::heuristic("version 1.2 please").is_complete(), "a version number is not a target");
    }

    #[test]
    fn model_spec_parses_with_or_without_fences() {
        let raw = r#"{"intent": "Add greeting", "targets": ["notes.txt"], "open_questions": []}"#;
        assert_eq!(parse_model_spec(raw).unwrap().targets, ["notes.txt"]);
        let fenced = format!("```json\n{raw}\n```");
        assert_eq!(parse_model_spec(&fenced).unwrap().intent, "Add greeting");
        assert!(parse_model_spec("not json").is_none());
    }

    #[test]
    fn render_lists_open_questions() {
        let mut spec = Spec::heuristic("explain the project");
        spec.success_criteria.push("cargo test passes".into());
        let text = spec.render();
        assert!(text.contains("Open questions"));
        assert!(text.contains("Target file not specified"));
        assert!(text.contains("Done when: cargo test passes"));
    }
}
