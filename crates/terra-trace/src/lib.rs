//! Terra Audit Trail
//!
//! Command audit logging and replay capability.
//! Every command executed by agents is recorded here.

pub mod audit;

pub use audit::{AuditLog, AuditEntry};
