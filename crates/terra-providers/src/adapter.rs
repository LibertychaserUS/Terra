use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Capabilities a provider/model supports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapability {
    pub provider_id: String,
    pub model_id: String,
    pub supports_structured_output: bool,
    pub supports_streaming: bool,
    pub supports_function_calling: bool,
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

/// Stream events emitted during streaming completion via mpsc channel.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Stream has started.
    Start,
    /// A token delta arrived.
    Delta(String),
    /// Stream completed with usage stats.
    End {
        prompt_tokens: u32,
        completion_tokens: u32,
        total_tokens: u32,
    },
}

/// A tool definition for function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool name (e.g., "FileRead", "MemSearch", "PointerQuery").
    pub name: String,
    /// Short description for the LLM.
    pub description: String,
    /// JSON Schema for the tool's parameters.
    pub parameters: serde_json::Value,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this call (provider-assigned).
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Arguments as JSON.
    pub arguments: serde_json::Value,
}

/// A tool result to send back to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Matches the tool_call ID.
    pub tool_call_id: String,
    /// Result content (stringified).
    pub content: String,
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

    /// Start streaming completion, sending events to the channel.
    /// Default falls back to complete() and emits Start + Delta(full_content) + End.
    async fn complete_streaming(
        &self,
        request: CompletionRequest,
        tx: mpsc::UnboundedSender<StreamEvent>,
    ) -> anyhow::Result<CompletionResponse> {
        let response = self.complete(request).await?;
        let _ = tx.send(StreamEvent::Start);
        let _ = tx.send(StreamEvent::Delta(response.content.clone()));
        let _ = tx.send(StreamEvent::End {
            prompt_tokens: response.usage_prompt_tokens,
            completion_tokens: response.usage_completion_tokens,
            total_tokens: response.usage_prompt_tokens + response.usage_completion_tokens,
        });
        Ok(response)
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
    /// Tool definitions for function calling (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    /// Earlier tool calls and their results in this turn, oldest first.
    /// Each round is replayed after the user message so the model sees the
    /// whole chain, not just the latest result.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_rounds: Vec<ToolRound>,
}

/// One assistant tool-call message and the results sent back for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRound {
    pub calls: Vec<ToolCall>,
    pub results: Vec<ToolResult>,
}

/// A completion response from an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub usage_prompt_tokens: u32,
    pub usage_completion_tokens: u32,
    pub model: String,
    /// Tool calls requested by the LLM (if any).
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

/// A dry-run adapter that returns deterministic responses without calling any API.
pub struct DryRunAdapter {
    capability: ProviderCapability,
}

impl Default for DryRunAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl DryRunAdapter {
    pub fn new() -> Self {
        Self {
            capability: ProviderCapability {
                provider_id: "dry_run".into(),
                model_id: "dry_run".into(),
                supports_structured_output: true,
                supports_streaming: false,
                supports_function_calling: false,
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
                request.user_prompt.chars().take(100).collect::<String>()
            ),
            usage_prompt_tokens: 0,
            usage_completion_tokens: 0,
            model: "dry_run".into(),
            tool_calls: vec![],
        })
    }

    async fn complete_stream(&self, request: CompletionRequest) -> anyhow::Result<Vec<StreamChunk>> {
        let full = format!(
            "DRY_RUN response to: {}",
            request.user_prompt.chars().take(100).collect::<String>()
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
