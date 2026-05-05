/// Executor-specific errors.
#[derive(thiserror::Error, Debug)]
pub enum ExecutorError {
    #[error("cage blocked: {cage} — {reason}")]
    CageBlocked { cage: String, reason: String },

    #[error("backend error: {0}")]
    Backend(#[from] anyhow::Error),

    #[error("command not routable: unknown op")]
    NotRoutable,
}
