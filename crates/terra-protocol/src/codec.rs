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
    use crate::message::*;

    #[test]
    fn roundtrip_msgpack() {
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

        let bytes = encode(&msg).unwrap();
        let decoded: AgentMessage = decode(&bytes).unwrap();

        assert_eq!(decoded.from, msg.from);
        assert_eq!(decoded.to, msg.to);
        assert_eq!(decoded.kind, msg.kind);

        // MessagePack should be smaller than JSON
        let json_bytes = serde_json::to_vec(&msg).unwrap();
        assert!(
            bytes.len() < json_bytes.len(),
            "msgpack {} bytes vs json {} bytes",
            bytes.len(),
            json_bytes.len()
        );
    }
}
