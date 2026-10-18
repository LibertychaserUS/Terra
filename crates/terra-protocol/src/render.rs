use crate::command::{Command, CommandOp};
use crate::message::{AgentMessage, MessagePayload};

/// Render an AgentMessage to human-readable Markdown.
pub fn render_message(msg: &AgentMessage) -> String {
    let mut lines = Vec::new();
    lines.push(format!("## Message {}", &msg.id[..8]));
    lines.push(format!(
        "- **From**: {:?} → **To**: {:?}",
        msg.from, msg.to
    ));
    lines.push(format!("- **Kind**: {:?}", msg.kind));
    lines.push(format!("- **Time**: {}", msg.timestamp.format("%Y-%m-%d %H:%M:%S UTC")));
    lines.push(String::new());

    match &msg.payload {
        MessagePayload::Task(t) => {
            lines.push(format!("**Operation**: `{}`", t.op));
            lines.push(format!("**Target**: `{}`", t.target));
            if !t.constraints.is_empty() {
                lines.push(format!("**Constraints**: {}", t.constraints.join(", ")));
            }
            if !t.expected_outputs.is_empty() {
                lines.push(format!("**Expected**: {}", t.expected_outputs.join(", ")));
            }
            if let Some(budget) = t.budget_max_tokens {
                lines.push(format!("**Budget**: {} tokens", budget));
            }
        }
        MessagePayload::Result(r) => {
            lines.push(format!("**Status**: {:?}", r.status));
            lines.push(format!("**Outputs**: ```json\n{}\n```", serde_json::to_string_pretty(&r.outputs).unwrap_or_default()));
            if let Some(err) = &r.error {
                lines.push(format!("**Error**: {}", err));
            }
        }
    }

    lines.join("\n")
}

/// Render a Command to human-readable Markdown.
pub fn render_command(cmd: &Command) -> String {
    let mut lines = Vec::new();
    lines.push(format!("## Command {}", &cmd.id[..8]));
    lines.push(format!("- **Agent**: {}", cmd.agent));
    lines.push(format!("- **Time**: {}", cmd.timestamp.format("%Y-%m-%d %H:%M:%S UTC")));

    match &cmd.op {
        CommandOp::MemWrite { key, content, scope, level } => {
            lines.push("- **Op**: mem_write".to_string());
            lines.push(format!("- **Key**: `{}`", key));
            lines.push(format!("- **Scope**: {} / **Level**: {}", scope, level));
            lines.push(format!("- **Content**: {}", content));
        }
        CommandOp::MemRead { key } => {
            lines.push("- **Op**: mem_read".to_string());
            lines.push(format!("- **Key**: `{}`", key));
        }
        CommandOp::MemSearch { query, top_k } => {
            lines.push("- **Op**: mem_search".to_string());
            lines.push(format!("- **Query**: {}", query));
            lines.push(format!("- **Top-K**: {}", top_k));
        }
        CommandOp::ArtifactSave { artifact_type, .. } => {
            lines.push("- **Op**: artifact_save".to_string());
            lines.push(format!("- **Type**: {}", artifact_type));
        }
        CommandOp::ArtifactLoad { artifact_type, version } => {
            lines.push("- **Op**: artifact_load".to_string());
            lines.push(format!("- **Type**: {}", artifact_type));
            if let Some(v) = version {
                lines.push(format!("- **Version**: {}", v));
            }
        }
        CommandOp::TraceEvent { event_type, data: _ } => {
            lines.push("- **Op**: trace_event".to_string());
            lines.push(format!("- **Event**: {}", event_type));
        }
        CommandOp::TraceCheckpoint { title, summary } => {
            lines.push("- **Op**: trace_checkpoint".to_string());
            lines.push(format!("- **Title**: {}", title));
            lines.push(format!("- **Summary**: {}", summary));
        }
        CommandOp::ProviderCall { system_prompt: _, user_prompt, max_tokens, temperature, output_contract } => {
            lines.push("- **Op**: provider_call".to_string());
            let prompt_end = user_prompt.char_indices().nth(100).map(|(i, _)| i).unwrap_or(user_prompt.len());
            lines.push(format!("- **Prompt**: {}", &user_prompt[..prompt_end]));
            if let Some(mt) = max_tokens {
                lines.push(format!("- **Max tokens**: {}", mt));
            }
            if let Some(t) = temperature {
                lines.push(format!("- **Temperature**: {}", t));
            }
            if let Some(c) = output_contract {
                lines.push(format!("- **Contract**: {}", c));
            }
        }
        CommandOp::FileRead { path } => {
            lines.push("- **Op**: file_read".to_string());
            lines.push(format!("- **Path**: `{}`", path));
        }
        CommandOp::FileWrite { path, content } => {
            lines.push("- **Op**: file_write".to_string());
            lines.push(format!("- **Path**: `{}`", path));
            lines.push(format!("- **Bytes**: {}", content.len()));
        }
        CommandOp::FileDelete { path } => {
            lines.push("- **Op**: file_delete".to_string());
            lines.push(format!("- **Path**: `{}`", path));
        }
        CommandOp::ShellExec { command, workdir, timeout_ms } => {
            lines.push("- **Op**: shell_exec".to_string());
            lines.push(format!("- **Command**: `{}`", command));
            if let Some(dir) = workdir {
                lines.push(format!("- **Workdir**: `{}`", dir));
            }
            if let Some(t) = timeout_ms {
                lines.push(format!("- **Timeout**: {}ms", t));
            }
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AgentRole, ResultPayload, ResultStatus, TaskPayload};

    // T-PT-020: render message contains operation
    #[test]
    fn render_message_contains_op() {
        let msg = AgentMessage::task_assignment(
            AgentRole::Worker,
            TaskPayload {
                op: "fix_bug".into(),
                target: serde_json::json!({"file": "auth.py", "line": 42}),
                constraints: vec!["pass_tests".into()],
                expected_outputs: vec!["diff".into()],
                budget_max_tokens: Some(2000),
            },
        );
        let md = render_message(&msg);
        assert!(md.contains("fix_bug"));
        assert!(md.contains("Worker"));
        assert!(md.contains("2000 tokens"));
    }

    // T-PT-021: render command contains agent
    #[test]
    fn render_command_contains_agent() {
        let cmd = Command::new(
            "main",
            CommandOp::MemWrite {
                key: "auth_pattern".into(),
                content: "Check input boundaries".into(),
                scope: "project".into(),
                level: "L1".into(),
            },
        );
        let md = render_command(&cmd);
        assert!(md.contains("main"));
        assert!(md.contains("mem_write"));
        assert!(md.contains("auth_pattern"));
    }

    // T-PT-022: render result contains status
    #[test]
    fn render_message_contains_status() {
        let msg = AgentMessage::task_result(
            AgentRole::Worker,
            ResultPayload {
                status: ResultStatus::Success,
                outputs: serde_json::json!({"diff": "+fix"}),
                error: None,
            },
        );
        let md = render_message(&msg);
        assert!(md.contains("Success"));
    }
}
