use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A command from an agent to the Command Executor.
///
/// Agents never write database code directly. They issue commands
/// from this fixed set, and the executor handles all I/O.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub agent: String,
    pub timestamp: DateTime<Utc>,
    pub op: CommandOp,
}

/// The fixed set of operations (~15 commands covering all interactions).
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
    ArtifactSave {
        artifact_type: String,
        content: serde_json::Value,
    },
    ArtifactLoad {
        artifact_type: String,
        version: Option<u32>,
    },
    TraceEvent {
        event_type: String,
        data: serde_json::Value,
    },
    TraceCheckpoint {
        title: String,
        summary: String,
    },
    ProviderCall {
        system_prompt: String,
        user_prompt: String,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
    },
}

/// Result of executing a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command_id: String,
    pub status: CommandStatus,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
    /// Program-generated notification for user display (e.g., auto-record alerts).
    /// Not LLM-generated. Format: `[Terra] Recorded → L{n}: "{summary}"`
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
        if let CommandOp::MemWrite { ref key, ref scope, ref level, .. } = cmd.op {
            assert_eq!(key, "auth_pattern");
            assert_eq!(scope, "project");
            assert_eq!(level, "L1");
        } else {
            panic!("expected MemWrite");
        }
    }

    #[test]
    fn command_mem_read() {
        let cmd = Command::new("worker", CommandOp::MemRead { key: "config".into() });
        if let CommandOp::MemRead { ref key } = cmd.op {
            assert_eq!(key, "config");
        } else {
            panic!("expected MemRead");
        }
    }

    #[test]
    fn command_mem_search() {
        let cmd = Command::new(
            "main",
            CommandOp::MemSearch { query: "auth bug".into(), top_k: 5 },
        );
        if let CommandOp::MemSearch { ref query, top_k } = cmd.op {
            assert_eq!(query, "auth bug");
            assert_eq!(top_k, 5);
        } else {
            panic!("expected MemSearch");
        }
    }

    #[test]
    fn command_artifact_save() {
        let cmd = Command::new(
            "main",
            CommandOp::ArtifactSave {
                artifact_type: "plan".into(),
                content: serde_json::json!({"steps": ["step1", "step2"]}),
            },
        );
        if let CommandOp::ArtifactSave { ref artifact_type, ref content } = cmd.op {
            assert_eq!(artifact_type, "plan");
            assert!(content.get("steps").is_some());
        } else {
            panic!("expected ArtifactSave");
        }
    }

    #[test]
    fn command_trace_checkpoint() {
        let cmd = Command::new(
            "main",
            CommandOp::TraceCheckpoint {
                title: "MVP done".into(),
                summary: "All tests pass".into(),
            },
        );
        if let CommandOp::TraceCheckpoint { ref title, ref summary } = cmd.op {
            assert_eq!(title, "MVP done");
            assert_eq!(summary, "All tests pass");
        } else {
            panic!("expected TraceCheckpoint");
        }
    }

    #[test]
    fn command_result_ok() {
        let result = CommandResult::ok("cmd-1", Some(serde_json::json!({"count": 3})));
        assert_eq!(result.status, CommandStatus::Ok);
        assert!(result.data.is_some());
        assert!(result.error.is_none());
    }

    #[test]
    fn command_result_error() {
        let result = CommandResult::err("cmd-2", "not found");
        assert_eq!(result.status, CommandStatus::Error);
        assert!(result.data.is_none());
        assert_eq!(result.error.as_deref(), Some("not found"));
    }
}
