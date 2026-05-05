use serde::{Deserialize, Serialize};

/// Capabilities a provider/model supports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapability {
    pub provider_id: String,
    pub model_id: String,
    pub supports_structured_output: bool,
    pub supports_streaming: bool,
    pub max_context_tokens: u32,
    pub latency_class: LatencyClass,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LatencyClass {
    Fast,
    Medium,
    Slow,
}

/// A streaming chunk from the provider.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// Incremental text content.
    pub text: String,
    /// True if this is the final chunk.
    pub done: bool,
}

/// Trait for LLM provider adapters.
///
/// Terra is provider-agnostic. Any OpenAI-compatible endpoint
/// implements this trait.
#[async_trait::async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Send a prompt and get a complete response (buffered).
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse>;

    /// Send a prompt and get a streaming response.
    /// Returns a Vec of chunks (simplified stream — real impl would use async channel).
    /// Default: falls back to complete() and returns single chunk.
    async fn complete_stream(&self, request: CompletionRequest) -> anyhow::Result<Vec<StreamChunk>> {
        let response = self.complete(request).await?;
        Ok(vec![StreamChunk {
            text: response.content,
            done: true,
        }])
    }

    /// Get the capabilities of this provider.
    fn capabilities(&self) -> &ProviderCapability;
}

/// A completion request to an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

/// A completion response from an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub usage_prompt_tokens: u32,
    pub usage_completion_tokens: u32,
    pub model: String,
}

/// A dry-run adapter that returns deterministic responses without calling any API.
pub struct DryRunAdapter {
    capability: ProviderCapability,
}

impl DryRunAdapter {
    pub fn new() -> Self {
        Self {
            capability: ProviderCapability {
                provider_id: "dry_run".into(),
                model_id: "dry_run".into(),
                supports_structured_output: true,
                supports_streaming: false,
                max_context_tokens: 128_000,
                latency_class: LatencyClass::Fast,
            },
        }
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for DryRunAdapter {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        Ok(CompletionResponse {
            content: format!(
                "DRY_RUN response to: {}",
                &request.user_prompt[..request.user_prompt.len().min(100)]
            ),
            usage_prompt_tokens: 0,
            usage_completion_tokens: 0,
            model: "dry_run".into(),
        })
    }

    async fn complete_stream(&self, request: CompletionRequest) -> anyhow::Result<Vec<StreamChunk>> {
        let full = format!(
            "DRY_RUN response to: {}",
            &request.user_prompt[..request.user_prompt.len().min(100)]
        );
        // Emit word by word to simulate streaming
        let words: Vec<&str> = full.split_whitespace().collect();
        let mut chunks = Vec::new();
        for (i, word) in words.iter().enumerate() {
            let is_last = i == words.len() - 1;
            chunks.push(StreamChunk {
                text: if i == 0 { word.to_string() } else { format!(" {}", word) },
                done: is_last,
            });
        }
        Ok(chunks)
    }

    fn capabilities(&self) -> &ProviderCapability {
        &self.capability
    }
}
