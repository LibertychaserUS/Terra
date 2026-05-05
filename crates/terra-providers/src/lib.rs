//! Terra Provider Dispatch
//!
//! LLM provider adapter with capability detection,
//! dispatch policy, retry/backoff, and dry_run mode.

pub mod adapter;
pub mod dispatch;

pub use adapter::{ProviderAdapter, ProviderCapability};
pub use dispatch::{DispatchResult, DispatchMode, dispatch};
