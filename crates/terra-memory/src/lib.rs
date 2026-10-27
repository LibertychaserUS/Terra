//! Terra memory: one SQLite table of entries with an owner, a scope and a
//! level, plus an FTS5 index for search.
//!
//! The rules that make this a cage rather than a key-value store live in
//! `MemoryDatabase`: an entry is changed or deleted only by its owner, and
//! promotion to a more authoritative level needs a human for L6 candidates.
//! Level write permissions (L0 read-only, L1 user-only) are enforced by the
//! Memory cage in `terra-caging` before a write reaches this crate.

pub mod db;
pub mod error;
pub mod types;

pub use db::{GcReport, MemoryDatabase, SearchHit};
pub use error::MemoryError;
pub use types::{Level, MemoryEntry, Scope};
