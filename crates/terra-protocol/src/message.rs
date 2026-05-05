use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Role of an agent in the multi-agent runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Main,
    Worker,
    Explorer,
    Cage,
}

/// The kind of message being sent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// A task assignment from main to sub-agent.
    TaskAssignment,
    /// A result returned from sub-agent to main.
    TaskResult,
    /// A cage observation report.
    CageReport,
}

/// Core message exchanged between agents.
///
/// All inter-agent communication uses this structure,
/// serialized as MessagePack for transport and storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub from: AgentRole,
    pub to: AgentRole,
    pub kind: MessageKind,
    pub timestamp: DateTime<Utc>,
    pub payload: MessagePayload,
}

/// The payload carried by an agent message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePayload {
    /// Task to be executed by a sub-agent.
    Task(TaskPayload),
    /// Result of a completed task.
    Result(ResultPayload),
}

/// A task assignment payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPayload {
    pub op: String,
    pub target: serde_json::Value,
    pub constraints: Vec<String>,
    pub expected_outputs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_max_tokens: Option<u32>,
}

/// A task result payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultPayload {
    pub status: ResultStatus,
    pub outputs: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Success,
    Failed,
    Partial,
}

impl AgentMessage {
    /// Create a new task assignment from main to a sub-agent.
    pub fn task_assignment(to: AgentRole, payload: TaskPayload) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from: AgentRole::Main,
            to,
            kind: MessageKind::TaskAssignment,
            timestamp: Utc::now(),
            payload: MessagePayload::Task(payload),
        }
    }

    /// Create a result message from a sub-agent back to main.
    pub fn task_result(from: AgentRole, payload: ResultPayload) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from,
            to: AgentRole::Main,
            kind: MessageKind::TaskResult,
            timestamp: Utc::now(),
            payload: MessagePayload::Result(payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_task_assignment() {
        let msg = AgentMessage::task_assignment(
            AgentRole::Worker,
            TaskPayload {
                op: "fix_bug".into(),
                target: serde_json::json!({"file": "src/auth.py", "line": 42}),
                constraints: vec!["pass_tests".into(), "minimal_change".into()],
                expected_outputs: vec!["diff".into(), "test_result".into()],
                budget_max_tokens: Some(2000),
            },
        );
        assert_eq!(msg.from, AgentRole::Main);
        assert_eq!(msg.to, AgentRole::Worker);
        assert_eq!(msg.kind, MessageKind::TaskAssignment);
    }

    #[test]
    fn create_task_result() {
        let msg = AgentMessage::task_result(
            AgentRole::Worker,
            ResultPayload {
                status: ResultStatus::Success,
                outputs: serde_json::json!({"diff": "+    if password.is_empty() { return Err(...) }"}),
                error: None,
            },
        );
        assert_eq!(msg.from, AgentRole::Worker);
        assert_eq!(msg.to, AgentRole::Main);
    }
}
