/// Domain-specific errors for the Memory Database.
///
/// Each variant maps to a specific error condition from TDD Section 2.4.5.
#[derive(thiserror::Error, Debug)]
pub enum MemoryError {
    #[error("key not found: {0}")]
    NotFound(String),

    #[error("duplicate key: {0}")]
    DuplicateKey(String),

    #[error("permission denied: {agent} is not owner of {key}")]
    PermissionDenied { agent: String, key: String },

    #[error("dangling pointer: {0} is deleted")]
    Dangling(String),

    #[error("stale version: expected {expected}, found {found}")]
    Stale { expected: u64, found: u64 },

    #[error("promotion blocked: {reason}")]
    PromotionBlocked { reason: String },

    #[error("invalid scope: {0}")]
    InvalidScope(String),

    #[error("invalid level: {0}")]
    InvalidLevel(String),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}
