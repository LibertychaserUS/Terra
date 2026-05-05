//! Terra Audit Trail
//!
//! Command audit logging with both in-memory and SQLite persistence.
//! Every command executed by agents is recorded.

pub mod audit;
pub mod persistent;

pub use audit::{AuditLog, AuditEntry};
pub use persistent::PersistentAuditLog;
