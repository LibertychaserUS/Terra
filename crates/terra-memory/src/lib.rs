//! Terra Memory Database
//!
//! Rust-safe memory system with ownership, borrowing, lifetime management,
//! SQLite persistence, moka cache, and layered access.
//!
//! Architecture (bottom-up):
//! - `types`: core data types (Scope, Level, MemoryEntry)
//! - `error`: domain error taxonomy
//! - `store`: SQLite persistence layer
//! - `safety`: ownership and lifetime enforcement
//! - `cache`: moka in-memory cache layer
//! - `db`: public API combining all layers

pub mod types;
pub mod error;
pub mod store;
pub mod safety;
pub mod cache;
pub mod artifact;
pub mod db;

pub use db::MemoryDatabase;
pub use error::MemoryError;
pub use types::{Level, MemoryEntry, Scope};
pub use artifact::{ArtifactStore, ArtifactType, ArtifactMeta};
