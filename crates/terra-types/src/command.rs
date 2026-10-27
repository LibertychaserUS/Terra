use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A command from an agent to the Command Executor.
///
/// Agents never touch the file system, the memory store or the shell
/// directly. They issue commands from this fixed set, and the executor
/// audits them, runs the cages and performs the I/O.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub agent: String,
    pub timestamp: DateTime<Utc>,
    pub op: CommandOp,
}

/// The fixed set of operations an agent can request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CommandOp {
    MemWrite {
        key: String,
        content: String,
        scope: String,
        level: String,
    },
    MemRead {
        key: String,
    },
    MemSearch {
        query: String,
        top_k: u32,
    },
    /// A structured note in the audit log (mode switches, verdicts, ...).
    TraceEvent {
        event_type: String,
        data: serde_json::Value,
    },
    /// A one-shot model call made through the executor, so it is audited.
    /// `json` asks the model for a JSON object (structured output).
    ProviderCall {
        system_prompt: String,
        user_prompt: String,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
        #[serde(default)]
        json: bool,
    },
    /// Read a file from the filesystem.
    FileRead {
        path: String,
    },
    /// Write content to a file. Creates or overwrites.
    FileWrite {
        path: String,
        content: String,
    },
    /// Delete a file.
    FileDelete {
        path: String,
    },
    /// Execute a shell command.
    ShellExec {
        command: String,
        workdir: Option<String>,
        timeout_ms: Option<u64>,
    },
    /// Gate before a task runs: the Specification cage blocks it while the
    /// spec has open questions.
    ExecuteTask {
        spec_id: String,
        intent: String,
        open_questions: Vec<String>,
    },
    /// Claim that the active goal is complete: the Verification cage blocks
    /// the claim unless `verified`. `evidence` says what the claim rests on.
    ClaimComplete {
        goal: String,
        verified: bool,
        evidence: String,
    },
}

impl CommandOp {
    /// The operation's name as it appears in tool lists and the audit log.
    pub fn name(&self) -> &'static str {
        match self {
            CommandOp::MemWrite { .. } => "MemWrite",
            CommandOp::MemRead { .. } => "MemRead",
            CommandOp::MemSearch { .. } => "MemSearch",
            CommandOp::TraceEvent { .. } => "TraceEvent",
            CommandOp::ProviderCall { .. } => "ProviderCall",
            CommandOp::FileRead { .. } => "FileRead",
            CommandOp::FileWrite { .. } => "FileWrite",
            CommandOp::FileDelete { .. } => "FileDelete",
            CommandOp::ShellExec { .. } => "ShellExec",
            CommandOp::ExecuteTask { .. } => "ExecuteTask",
            CommandOp::ClaimComplete { .. } => "ClaimComplete",
        }
    }

    /// Whether the operation can change the project (file system or shell).
    pub fn mutates(&self) -> bool {
        matches!(
            self,
            CommandOp::FileWrite { .. } | CommandOp::FileDelete { .. } | CommandOp::ShellExec { .. }
        )
    }
}

/// Result of executing a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command_id: String,
    pub status: CommandStatus,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
    /// Program-generated note for the user (not model output).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Ok,
    NotFound,
    PermissionDenied,
    Error,
}

impl Command {
    pub fn new(agent: impl Into<String>, op: CommandOp) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            agent: agent.into(),
            timestamp: Utc::now(),
            op,
        }
    }
}

impl CommandResult {
    pub fn ok(command_id: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        Self {
            command_id: command_id.into(),
            status: CommandStatus::Ok,
            data,
            error: None,
            notification: None,
        }
    }

    pub fn err(command_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            command_id: command_id.into(),
            status: CommandStatus::Error,
            data: None,
            error: Some(error.into()),
            notification: None,
        }
    }

    pub fn denied(command_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            command_id: command_id.into(),
            status: CommandStatus::PermissionDenied,
            data: None,
            error: Some(reason.into()),
            notification: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_mem_write() {
        let cmd = Command::new(
            "main",
            CommandOp::MemWrite {
                key: "auth_pattern".into(),
                content: "Input boundary check".into(),
                scope: "project".into(),
                level: "L1".into(),
            },
        );
        assert_eq!(cmd.agent, "main");
        assert_eq!(cmd.op.name(), "MemWrite");
        if let CommandOp::MemWrite { ref key, ref scope, ref level, .. } = cmd.op {
            assert_eq!(key, "auth_pattern");
            assert_eq!(scope, "project");
            assert_eq!(level, "L1");
        } else {
            panic!("expected MemWrite");
        }
    }

    #[test]
    fn mutating_ops() {
        assert!(CommandOp::FileWrite { path: "a".into(), content: String::new() }.mutates());
        assert!(CommandOp::ShellExec { command: "ls".into(), workdir: None, timeout_ms: None }.mutates());
        assert!(!CommandOp::FileRead { path: "a".into() }.mutates());
        assert!(!CommandOp::MemRead { key: "k".into() }.mutates());
    }

    #[test]
    fn json_roundtrip_keeps_tag() {
        let cmd = Command::new("main", CommandOp::MemSearch { query: "auth bug".into(), top_k: 5 });
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"cmd\":\"mem_search\""));
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, cmd.id);
    }

    #[test]
    fn command_result_constructors() {
        assert_eq!(CommandResult::ok("c", None).status, CommandStatus::Ok);
        assert_eq!(CommandResult::err("c", "boom").error.as_deref(), Some("boom"));
        assert_eq!(CommandResult::denied("c", "no").status, CommandStatus::PermissionDenied);
    }
}
