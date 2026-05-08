//! Terra Memory Database
//!
//! Rust-safe memory system with ownership, borrowing, lifetime management,
//! pluggable storage backends, moka cache, and layered access.
//!
//! Architecture (bottom-up):
//! - `types`: core data types (Scope, Level, MemoryEntry)
//! - `error`: domain error taxonomy
//! - `backend`: StorageBackend trait + InMemoryBackend
//! - `store`: SQLite persistence layer (implements StorageBackend)
//! - `safety`: ownership and lifetime enforcement
//! - `cache`: moka in-memory cache layer
//! - `db`: public API combining all layers

pub mod types;
pub mod error;
pub mod backend;
pub mod store;
pub mod safety;
pub mod cache;
pub mod artifact;
pub mod search;
pub mod pointer;
pub mod db;

pub use db::MemoryDatabase;
pub use error::MemoryError;
pub use types::{Level, MemoryEntry, Scope};
pub use backend::{StorageBackend, InMemoryBackend};
pub use artifact::{ArtifactStore, ArtifactType, ArtifactMeta};
pub use search::{VectorIndex, EmbeddingBackend, TfIdfBackend};
pub use pointer::{Pointer, PointerTable, SalienceSignal};
