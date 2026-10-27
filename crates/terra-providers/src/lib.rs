//! LLM providers: the `ProviderAdapter` trait, an OpenAI-compatible
//! implementation on `async-openai`, and a keyless dry-run stub.

pub mod adapter;
pub mod openai;

pub use adapter::{CompletionRequest, CompletionResponse, DryRunAdapter, Message, ProviderAdapter, ProviderCapability, ToolCall, ToolDef};
pub use openai::OpenAICompatibleAdapter;
