//! Operation Status Text
//!
//! Generates human-readable status lines from CommandOp.
//! Zero LLM token cost — purely programmatic.
//! Used by REPL to show thinking chain.

use terra_types::CommandOp;

/// Generate a status line for a CommandOp (shown before execution).
pub fn op_status_text(op: &CommandOp) -> String {
    match op {
        CommandOp::FileRead { path } => format!("Reading {}...", short_path(path)),
        CommandOp::FileWrite { path, content } => {
            format!("Writing {} ({} bytes)...", short_path(path), content.len())
        }
        CommandOp::FileDelete { path } => format!("Deleting {}...", short_path(path)),
        CommandOp::ShellExec { command, .. } => {
            let short = if command.len() > 50 { format!("{}...", &command[..47]) } else { command.clone() };
            format!("Running: {}...", short)
        }
        CommandOp::ProviderCall { .. } => "Thinking...".into(),
        CommandOp::MemRead { key } => format!("Reading memory '{}'...", key),
        CommandOp::MemWrite { key, level, .. } => format!("Writing memory '{}' ({})", key, level),
        CommandOp::MemSearch { query, top_k } => format!("Searching memory for '{}' (top {})...",
            if query.len() > 30 { format!("{}...", &query[..27]) } else { query.clone() }, top_k),
        CommandOp::ArtifactSave { artifact_type, .. } => format!("Saving {} artifact...", artifact_type),
        CommandOp::ArtifactLoad { artifact_type, .. } => format!("Loading {} artifact...", artifact_type),
        CommandOp::TraceEvent { event_type, .. } => format!("Recording event: {}", event_type),
        CommandOp::TraceCheckpoint { title, .. } => format!("Checkpoint: {}", title),
    }
}

/// Generate a result summary after FileWrite (diff-like).
pub fn file_write_summary(path: &str, content: &str) -> String {
    let lines = content.lines().count();
    format!("Wrote {}: {} lines", short_path(path), lines)
}

/// Shorten a file path for display.
fn short_path(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-TC-001: file read shows status
    #[test]
    fn file_read_status() {
        let op = CommandOp::FileRead { path: "/project/src/auth.rs".into() };
        let status = op_status_text(&op);
        assert!(status.contains("Reading"));
        assert!(status.contains("auth.rs"));
    }

    // T-TC-002: provider call shows thinking
    #[test]
    fn provider_call_status() {
        let op = CommandOp::ProviderCall {
            system_prompt: "test".into(),
            user_prompt: "test".into(),
            max_tokens: None,
            temperature: None,
            output_contract: None,
        };
        assert_eq!(op_status_text(&op), "Thinking...");
    }

    // T-TC-003: status from commandop programmatic
    #[test]
    fn status_is_programmatic() {
        // All CommandOp variants produce a non-empty status
        let ops = vec![
            CommandOp::FileRead { path: "a.rs".into() },
            CommandOp::FileWrite { path: "b.rs".into(), content: "x".into() },
            CommandOp::FileDelete { path: "c.rs".into() },
            CommandOp::ShellExec { command: "echo hi".into(), workdir: None, timeout_ms: None },
            CommandOp::MemRead { key: "k".into() },
            CommandOp::MemWrite { key: "k".into(), content: "v".into(), scope: "project".into(), level: "L2".into() },
            CommandOp::MemSearch { query: "q".into(), top_k: 5 },
            CommandOp::TraceEvent { event_type: "test".into(), data: serde_json::json!({}) },
        ];
        for op in &ops {
            let status = op_status_text(op);
            assert!(!status.is_empty(), "status should not be empty for {:?}", op);
        }
    }

    // T-TC-004: file write summary
    #[test]
    fn file_write_diff_summary() {
        let summary = file_write_summary("/src/auth.rs", "line1\nline2\nline3\n");
        assert!(summary.contains("auth.rs"));
        assert!(summary.contains("3 lines"));
    }
}
