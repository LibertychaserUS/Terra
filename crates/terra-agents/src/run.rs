//! `terra run`: one task, non-interactive, through the same gates as chat.
//!
//! The task enters Goal Mode (specified first), the loop works on it, and
//! the completion claim goes through the Verification cage. With no one to
//! answer confirmation prompts, shell and delete calls are declined unless
//! the caller passed `--yes`.

use serde::Serialize;

use terra_types::command::{Command, CommandOp, CommandStatus};

use crate::agent::{Agent, Output};
use crate::session::Verification;

#[derive(Debug, Serialize)]
pub struct TaskReport {
    /// "blocked", "complete", "unverified" or "error" (the provider failed).
    pub status: String,
    pub spec_id: String,
    pub intent: String,
    pub open_questions: Vec<String>,
    pub response: String,
    pub provider: String,
    pub verification: String,
    pub audit_entries: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl TaskReport {
    pub fn succeeded(&self) -> bool {
        self.status == "complete"
    }
}

pub async fn run_task(agent: &mut Agent, task: &str, auto_approve: bool) -> TaskReport {
    agent.auto_approve_shell = auto_approve;
    agent.auto_approve_delete = auto_approve;
    let provider = agent.executor().provider().capabilities().model_id.clone();
    let audit_start = agent.executor().audit().len();

    let spec = match agent.session.enter_goal(task).await {
        Ok(spec) => spec.clone(),
        Err(e) => {
            return TaskReport {
                status: "blocked".into(),
                spec_id: String::new(),
                intent: task.into(),
                open_questions: vec![e],
                response: String::new(),
                provider,
                verification: String::new(),
                audit_entries: 0,
                prompt_tokens: 0,
                completion_tokens: 0,
            }
        }
    };
    // The gate: an ExecuteTask command the Specification cage judges (when
    // the constitution registers it). The verdict is in the audit log.
    let gate = Command::new(
        "main",
        CommandOp::ExecuteTask { spec_id: spec.spec_id.clone(), intent: spec.intent.clone(), open_questions: spec.open_questions.clone() },
    );
    let verdict = agent.executor().execute(gate).await;
    if verdict.status == CommandStatus::PermissionDenied {
        let _ = agent.session.abandon_goal().await;
        return TaskReport {
            status: "blocked".into(),
            spec_id: spec.spec_id,
            intent: spec.intent,
            open_questions: spec.open_questions,
            response: verdict.error.unwrap_or_default(),
            provider,
            verification: String::new(),
            audit_entries: agent.executor().audit().len() - audit_start,
            prompt_tokens: 0,
            completion_tokens: 0,
        };
    }

    let out = agent.turn(task).await;
    let response = out.iter().find_map(|o| match o { Output::Answer(a) => Some(a.clone()), _ => None }).unwrap_or_default();
    let verification = agent.session.verification().unwrap_or(Verification::NoChanges);
    let status = if agent.last_error().is_some() {
        let _ = agent.session.abandon_goal().await;
        "error"
    } else {
        match agent.session.complete_goal().await {
            Ok(_) => "complete",
            Err(_) => {
                let _ = agent.session.abandon_goal().await;
                "unverified"
            }
        }
    };
    TaskReport {
        status: status.into(),
        spec_id: spec.spec_id,
        intent: spec.intent,
        open_questions: Vec::new(),
        response,
        provider,
        verification: verification.describe(),
        audit_entries: agent.executor().audit().len() - audit_start,
        prompt_tokens: agent.tokens().prompt_tokens,
        completion_tokens: agent.tokens().completion_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tests::{agent_with, call, reply, ScriptedProvider};

    #[tokio::test]
    async fn blocked_task_never_calls_the_model() {
        let provider = ScriptedProvider::new(vec![]);
        let (mut agent, _dir) = agent_with(provider.clone());
        let report = run_task(&mut agent, "test task", false).await;
        assert_eq!(report.status, "blocked");
        assert_eq!(report.open_questions, ["Target file not specified"]);
        assert!(report.response.contains("cage 'specification'"));
        assert!(provider.requests.lock().unwrap().is_empty());
        assert!(!agent.session.mode().is_goal());
    }

    #[tokio::test]
    async fn task_without_changes_completes() {
        let provider = ScriptedProvider::new(vec![reply("done reading", vec![])]);
        let (mut agent, _dir) = agent_with(provider);
        let report = run_task(&mut agent, "explain src/lib.rs", false).await;
        assert_eq!(report.status, "complete");
        assert_eq!(report.response, "done reading");
        assert!(report.verification.contains("nothing was written"));
    }

    #[tokio::test]
    async fn unverified_changes_are_reported() {
        let provider = ScriptedProvider::new(vec![
            reply("", vec![call("c1", "FileWrite", serde_json::json!({"path": "notes.txt", "content": "x"}))]),
            reply("wrote it", vec![]),
        ]);
        let (mut agent, dir) = agent_with(provider);
        let report = run_task(&mut agent, "write notes.txt", false).await;
        assert_eq!(report.status, "unverified");
        assert!(dir.path().join("notes.txt").exists());
        assert!(report.verification.contains("not followed by a passing /verify"));
    }
}
