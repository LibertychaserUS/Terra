use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};

/// Encode a value to MessagePack bytes (named fields for Option compatibility).
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(rmp_serde::to_vec_named(value)?)
}

/// Decode MessagePack bytes to a value.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(rmp_serde::from_slice(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, CommandOp};
    use crate::message::*;

    // T-PT-007: task assignment roundtrip
    #[test]
    fn msgpack_roundtrip_task_assignment() {
        let msg = AgentMessage::task_assignment(
            AgentRole::Worker,
            TaskPayload {
                op: "fix_bug".into(),
                target: serde_json::json!({"file": "auth.py", "line": 42}),
                constraints: vec!["minimal_change".into()],
                expected_outputs: vec!["diff".into()],
                budget_max_tokens: Some(2000),
            },
        );

        let bytes = encode(&msg).unwrap();
        let decoded: AgentMessage = decode(&bytes).unwrap();

        assert_eq!(decoded.from, msg.from);
        assert_eq!(decoded.to, msg.to);
        assert_eq!(decoded.kind, msg.kind);
        assert_eq!(decoded.id, msg.id);
    }

    // T-PT-008: result roundtrip
    #[test]
    fn msgpack_roundtrip_task_result() {
        let msg = AgentMessage::task_result(
            AgentRole::Worker,
            ResultPayload {
                status: ResultStatus::Success,
                outputs: serde_json::json!({"diff": "+fix"}),
                error: None,
            },
        );

        let bytes = encode(&msg).unwrap();
        let decoded: AgentMessage = decode(&bytes).unwrap();
        assert_eq!(decoded.from, msg.from);
        assert_eq!(decoded.kind, MessageKind::TaskResult);
    }

    // T-PT-009: command roundtrip
    #[test]
    fn msgpack_roundtrip_command() {
        let cmd = Command::new("main", CommandOp::MemRead { key: "test".into() });
        let bytes = encode(&cmd).unwrap();
        let decoded: Command = decode(&bytes).unwrap();
        assert_eq!(decoded.agent, "main");
    }

    // T-PT-010: msgpack smaller than json
    #[test]
    fn msgpack_smaller_than_json() {
        let msg = AgentMessage::task_assignment(
            AgentRole::Worker,
            TaskPayload {
                op: "fix_bug".into(),
                target: serde_json::json!({"file": "auth.py"}),
                constraints: vec!["minimal_change".into()],
                expected_outputs: vec!["diff".into()],
                budget_max_tokens: None,
            },
        );

        let msgpack_bytes = encode(&msg).unwrap();
        let json_bytes = serde_json::to_vec(&msg).unwrap();
        assert!(
            msgpack_bytes.len() < json_bytes.len(),
            "msgpack {} bytes vs json {} bytes",
            msgpack_bytes.len(),
            json_bytes.len()
        );
    }

    // T-PT-011: optional fields as None
    #[test]
    fn msgpack_optional_none() {
        let msg = AgentMessage::task_assignment(
            AgentRole::Explorer,
            TaskPayload {
                op: "search".into(),
                target: serde_json::json!(null),
                constraints: vec![],
                expected_outputs: vec![],
                budget_max_tokens: None,
            },
        );

        let bytes = encode(&msg).unwrap();
        let decoded: AgentMessage = decode(&bytes).unwrap();
        if let MessagePayload::Task(ref t) = decoded.payload {
            assert!(t.budget_max_tokens.is_none());
            assert!(t.constraints.is_empty());
        } else {
            panic!("expected Task payload");
        }
    }

    // T-PT-012: invalid bytes return Err
    #[test]
    fn decode_invalid_bytes() {
        let result = decode::<AgentMessage>(&[0xFF, 0xFE, 0x00, 0x01]);
        assert!(result.is_err());
    }
}
