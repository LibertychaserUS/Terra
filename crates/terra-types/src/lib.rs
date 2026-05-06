//! Terra Shared Types (L0)
//!
//! Command protocol types used across all layers.
//! This crate sits at L0 — any layer may depend on it.
//! It has no dependencies on other Terra crates.

pub mod command;
pub mod contract;
pub mod event;

pub use command::{Command, CommandOp, CommandResult, CommandStatus};
pub use contract::{OutputContract, ContractParseResult};
pub use event::{StreamEvent, EventType, TokenTracker, EventBus, NullBus, CollectorBus};
