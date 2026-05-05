//! Terra Internal Protocol (TIP)
//!
//! MessagePack-based communication protocol for agent-to-agent
//! and agent-to-tool interaction.

pub mod message;
pub mod codec;
pub mod command;
pub mod render;

pub use message::{AgentMessage, AgentRole, MessageKind};
pub use command::{Command, CommandResult};
pub use codec::{encode, decode};
