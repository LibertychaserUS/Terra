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
}

/// Result of executing a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub command_id: String,
    pub status: CommandStatus,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
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
        }
    }

    pub fn err(command_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            command_id: command_id.into(),
            status: CommandStatus::Error,
            data: None,
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_mem_write_command() {
        let cmd = Command::new(
            "main",
            CommandOp::MemWrite {
                key: "auth_pattern".into(),
                content: "Input boundary check at function entry".into(),
                scope: "project".into(),
                level: "L1".into(),
            },
        );
        assert_eq!(cmd.agent, "main");
    }

    #[test]
    fn command_roundtrip_msgpack() {
        let cmd = Command::new(
            "worker-1",
            CommandOp::TraceCheckpoint {
                title: "Bug fix complete".into(),
                summary: "Fixed empty password validation in auth.py".into(),
            },
        );

        let bytes = crate::codec::encode(&cmd).unwrap();
        let decoded: Command = crate::codec::decode(&bytes).unwrap();
        assert_eq!(decoded.agent, "worker-1");
    }
}
