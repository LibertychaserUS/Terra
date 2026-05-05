//! Terra Command Executor
//!
//! The single gateway through which all agents interact with system resources.
//! Enforces: audit → caging → execute → record.
//!
//! No agent may directly access MemoryDatabase, ProviderAdapter, or filesystem.
//! All operations are Command objects routed through this executor.

pub mod executor;
pub mod router;
pub mod error;

pub use executor::CommandExecutor;
pub use error::ExecutorError;
