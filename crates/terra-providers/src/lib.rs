//! Terra Provider Dispatch
//!
//! LLM provider adapter with capability detection,
//! dispatch policy, retry/backoff, and dry_run mode.

pub mod adapter;
pub mod dispatch;
pub mod openai;

pub use adapter::{ProviderAdapter, ProviderCapability, StreamChunk, StreamEvent};
pub use dispatch::{DispatchResult, DispatchMode, dispatch};
pub use openai::OpenAICompatibleAdapter;
