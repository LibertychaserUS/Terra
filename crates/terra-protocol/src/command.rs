//! Command types — re-exported from terra-types for backward compatibility.
//!
//! All command type definitions live in terra-types (L0).
//! This module re-exports them so existing `use terra_protocol::command::*`
//! continues to work without changes to all consumers.

pub use terra_types::command::{Command, CommandOp, CommandResult, CommandStatus};

#[cfg(test)]
mod tests {
    use super::*;

    // Command roundtrip through MessagePack (tests codec, not types)
    #[test]
    fn command_roundtrip_msgpack() {
        let cmd = Command::new(
            "worker-1",
            CommandOp::TraceCheckpoint {
                title: "Bug fix complete".into(),
                summary: "Fixed empty password".into(),
            },
        );
        let bytes = crate::codec::encode(&cmd).unwrap();
        let decoded: Command = crate::codec::decode(&bytes).unwrap();
        assert_eq!(decoded.agent, "worker-1");
    }
}
