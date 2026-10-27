//! The audit trail: append-only, hash-chained, persisted in SQLite.

pub mod audit;
pub mod persistent;

pub use audit::{op_summary, verify_events, AuditEntry, AuditEvent, AuditLog, ChainVerdict, EventKind, GENESIS_HASH};
