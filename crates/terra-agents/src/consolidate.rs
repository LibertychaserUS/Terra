//! Sleep consolidation: when a goal ends, the model reads what happened and
//! proposes a few reusable principles. Nothing is kept until the user
//! accepts a candidate; accepted ones become L1 project memory.

use serde::Deserialize;
use terra_executor::CommandExecutor;
use terra_trace::{op_summary, AuditEntry};
use terra_types::command::{Command, CommandOp, CommandStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub principle: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Default)]
pub struct ConsolidationReport {
    pub candidates: Vec<Candidate>,
    pub action_count: usize,
}

#[derive(Debug, Deserialize)]
struct ModelCandidate {
    principle: String,
    #[serde(default)]
    evidence: String,
}

#[derive(Debug, Deserialize)]
struct ModelReport {
    #[serde(default)]
    candidates: Vec<ModelCandidate>,
}

const SYSTEM_PROMPT: &str = "You review the audit log of a coding agent's finished goal and extract at most three \
principles worth remembering for this project: facts about the codebase, conventions, or lessons from failures. \
Each principle is one sentence that would help on a future task; skip anything generic or already obvious. \
Reply with one JSON object: {\"candidates\": [{\"principle\": \"...\", \"evidence\": \"which actions support it\"}]}. \
Return an empty list when nothing is worth keeping.";

/// Ask the model for candidates. Goals with nothing done yield none.
pub async fn consolidate(executor: &CommandExecutor, goal: &str, entries: &[AuditEntry], max_candidates: usize) -> ConsolidationReport {
    let actions: Vec<String> = entries
        .iter()
        .filter(|e| !matches!(e.command.op, CommandOp::TraceEvent { .. } | CommandOp::ProviderCall { .. }))
        .map(|e| {
            let status = e.result.as_ref().map(|r| format!("{:?}", r.status)).unwrap_or_else(|| "pending".into());
            format!("- [{}] {} -> {}", e.command.agent, op_summary(&e.command.op), status)
        })
        .collect();
    let action_count = actions.len();
    if action_count == 0 || !executor.provider().capabilities().supports_json_output {
        return ConsolidationReport { candidates: Vec::new(), action_count };
    }
    let cmd = Command::new(
        "consolidator",
        CommandOp::ProviderCall {
            system_prompt: SYSTEM_PROMPT.into(),
            user_prompt: format!("Goal: {}\n\nActions:\n{}", goal, actions.join("\n")),
            max_tokens: Some(600),
            temperature: Some(0.2),
            json: true,
        },
    );
    let result = executor.execute(cmd).await;
    let candidates = (result.status == CommandStatus::Ok)
        .then(|| result.data.as_ref()?.get("content")?.as_str().map(parse))
        .flatten()
        .flatten()
        .unwrap_or_default();
    ConsolidationReport { candidates: candidates.into_iter().take(max_candidates).collect(), action_count }
}

fn parse(content: &str) -> Option<Vec<Candidate>> {
    let trimmed = content.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(trimmed);
    let report: ModelReport = serde_json::from_str(json).ok()?;
    Some(
        report
            .candidates
            .into_iter()
            .filter(|c| !c.principle.trim().is_empty())
            .map(|c| Candidate { principle: c.principle.trim().to_string(), evidence: c.evidence.trim().to_string() })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_output() {
        let out = parse(r#"{"candidates": [{"principle": "Tests live in tests/", "evidence": "FileRead tests/x.rs"}, {"principle": " "}]}"#).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].principle, "Tests live in tests/");
        assert!(parse("nope").is_none());
    }

    #[tokio::test]
    async fn dry_run_yields_no_candidates_but_counts_actions() {
        let (exec, _dir) = crate::session::tests::test_executor();
        exec.execute(Command::new("main", CommandOp::FileWrite { path: "a.txt".into(), content: "x".into() })).await;
        let entries = exec.audit().entries();
        let report = consolidate(&exec, "goal", &entries, 3).await;
        assert_eq!(report.action_count, 1);
        assert!(report.candidates.is_empty());
    }
}
