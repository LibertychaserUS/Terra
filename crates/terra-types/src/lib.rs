//! Terra Shared Types (L0)
//!
//! Command protocol types used across all layers.
//! This crate sits at L0 — any layer may depend on it.
//! It has no dependencies on other Terra crates.

pub mod command;

pub use command::{Command, CommandOp, CommandResult, CommandStatus};
