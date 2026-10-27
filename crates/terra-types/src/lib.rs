//! Terra shared types: the command protocol between agents and the executor,
//! streaming events, and UTF-8 safe text helpers. No other Terra crate is a
//! dependency of this one.

pub mod command;
pub mod event;
pub mod text;

pub use command::{Command, CommandOp, CommandResult, CommandStatus};
pub use event::{ChannelBus, CollectorBus, EventBus, EventType, NullBus, StreamEvent, TokenTracker};
