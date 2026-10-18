use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use tracing::{info, warn};

use tokio::sync::mpsc;

use crate::adapter::{
    CompletionRequest, CompletionResponse, LatencyClass, ProviderAdapter, ProviderCapability,
    StreamEvent,
};

/// Environment variable names for Terra provider configuration.
const ENV_API_KEY: &str = "TERRA_PROVIDER_API_KEY";
const ENV_BASE_URL: &str = "TERRA_PROVIDER_BASE_URL";
const ENV_MODEL: &str = "TERRA_PROVIDER_MODEL";

/// Fallback: also check DASHSCOPE_API_KEY for DashScope compatibility.
const ENV_DASHSCOPE_KEY: &str = "DASHSCOPE_API_KEY";

const DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
const DEFAULT_MODEL: &str = "qwen-plus";
/// HTTP timeout for provider requests.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_RETRIES: u32 = 3;

/// OpenAI-compatible provider adapter.
///
/// Works with any endpoint that implements the OpenAI chat completions API:
/// OpenAI, DashScope (Alibaba), local models, etc.
#[derive(Debug)]
pub struct OpenAICompatibleAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    capability: ProviderCapability,
}

/// OpenAI chat completions request body.
#[derive(Serialize, Clone)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ChatTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

/// OpenAI tool definition format.
#[derive(Serialize, Clone)]
struct ChatTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: ChatFunction,
}

#[derive(Serialize, Clone)]
struct ChatFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

/// OpenAI tool call in response.
#[derive(Serialize, Deserialize, Clone)]
struct ChatToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ChatFunctionCall,
}

#[derive(Serialize, Deserialize, Clone)]
struct ChatFunctionCall {
    name: String,
    arguments: String, // JSON string
}

/// OpenAI chat completions response body.
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    usage: ChatUsage,
    model: String,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

/// SSE chunk from a streaming chat completion response.
#[derive(Deserialize)]
struct SseChunk {
    choices: Vec<SseChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct SseChoice {
    delta: SseDelta,
}

#[derive(Deserialize)]
struct SseDelta {
    content: Option<String>,
}

/// OpenAI error response body.
#[derive(Deserialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

impl OpenAICompatibleAdapter {
    /// Create a new adapter, reading config from environment variables.
    ///
    /// Reads:
    /// - `TERRA_PROVIDER_API_KEY` (or fallback `DASHSCOPE_API_KEY`)
    /// - `TERRA_PROVIDER_BASE_URL` (default: DashScope endpoint)
    /// - `TERRA_PROVIDER_MODEL` (default: "qwen-plus")
    pub fn from_env() -> Result<Self> {
        let api_key = env::var(ENV_API_KEY)
            .or_else(|_| env::var(ENV_DASHSCOPE_KEY))
            .context(format!(
                "Neither {} nor {} is set",
                ENV_API_KEY, ENV_DASHSCOPE_KEY
            ))?;

        let base_url = env::var(ENV_BASE_URL).unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let model = env::var(ENV_MODEL).unwrap_or_else(|_| DEFAULT_MODEL.to_string());

        Self::new(api_key, base_url, model, DEFAULT_TIMEOUT_SECS)
    }

    /// Create with explicit parameters (for testing with mock servers).
    pub fn new(
        api_key: String,
        base_url: String,
        model: String,
        timeout_secs: u64,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .context("failed to build HTTP client")?;

        let capability = ProviderCapability {
            provider_id: "openai_compatible".into(),
            model_id: model.clone(),
            supports_structured_output: true,
            supports_streaming: true,
            supports_function_calling: true,
            max_context_tokens: 128_000,
            latency_class: LatencyClass::Medium,
        };

        info!(
            provider = "openai_compatible",
            model = %model,
            base_url = %base_url,
            "Provider adapter initialized"
        );

        Ok(Self {
            client,
            api_key,
            base_url,
            model,
            capability,
        })
    }

    /// Execute a request with retry and exponential backoff.
    async fn complete_with_retry(&self, request: &CompletionRequest) -> Result<CompletionResponse> {
        let chat_request = self.build_chat_request(request, false);

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(1 << attempt);
                warn!(attempt, delay_secs = delay.as_secs(), "retrying after backoff");
                tokio::time::sleep(delay).await;
            }

            let response = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&chat_request)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();

                    if status.is_success() {
                        let body: ChatResponse = resp
                            .json()
                            .await
                            .context("failed to parse provider response")?;

                        let content = body
                            .choices
                            .first()
                            .and_then(|c| c.message.content.clone())
                            .unwrap_or_default();

                        // Extract tool calls if present
                        let tool_calls: Vec<crate::adapter::ToolCall> = body
                            .choices
                            .first()
                            .and_then(|c| c.message.tool_calls.as_ref())
                            .map(|tcs| tcs.iter().map(|tc| {
                                let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                                crate::adapter::ToolCall {
                                    id: tc.id.clone(),
                                    name: tc.function.name.clone(),
                                    arguments: args,
                                }
                            }).collect())
                            .unwrap_or_default();

                        info!(
                            model = %body.model,
                            prompt_tokens = body.usage.prompt_tokens,
                            completion_tokens = body.usage.completion_tokens,
                            tool_calls = tool_calls.len(),
                            "provider response received"
                        );

                        return Ok(CompletionResponse {
                            content,
                            usage_prompt_tokens: body.usage.prompt_tokens,
                            usage_completion_tokens: body.usage.completion_tokens,
                            model: body.model,
                            tool_calls,
                        });
                    }

                    // Retryable status codes
                    if status.as_u16() == 429
                        || status.as_u16() == 500
                        || status.as_u16() == 502
                        || status.as_u16() == 503
                    {
                        let error_msg = resp
                            .json::<ErrorResponse>()
                            .await
                            .map(|e| e.error.message)
                            .unwrap_or_else(|_| format!("HTTP {}", status));
                        warn!(status = status.as_u16(), error = %error_msg, "retryable error");
                        last_error = Some(error_msg);
                        continue;
                    }

                    // Non-retryable error
                    let error_msg = resp
                        .json::<ErrorResponse>()
                        .await
                        .map(|e| e.error.message)
                        .unwrap_or_else(|_| format!("HTTP {}", status));
                    bail!("provider error (HTTP {}): {}", status, error_msg);
                }
                Err(e) => {
                    if e.is_timeout() {
                        warn!(attempt, "request timed out");
                        last_error = Some("request timed out".into());
                        continue;
                    }
                    warn!(attempt, error = %e, "network error");
                    last_error = Some(e.to_string());
                    continue;
                }
            }
        }

        bail!(
            "provider failed after {} retries: {}",
            MAX_RETRIES,
            last_error.unwrap_or_else(|| "unknown error".into())
        )
    }
}

impl OpenAICompatibleAdapter {
    /// Build a ChatRequest from a CompletionRequest (shared logic).
    fn build_chat_request(&self, request: &CompletionRequest, stream: bool) -> ChatRequest {
        let mut messages = vec![
            ChatMessage {
                role: "system".into(),
                content: Some(request.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: "user".into(),
                content: Some(request.user_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            },
        ];

        // Replay every earlier tool round in order (multi-step tool conversation)
        for round in &request.tool_rounds {
            let chat_tool_calls: Vec<ChatToolCall> = round
                .calls
                .iter()
                .map(|tc| ChatToolCall {
                    id: tc.id.clone(),
                    call_type: "function".into(),
                    function: ChatFunctionCall {
                        name: tc.name.clone(),
                        arguments: tc.arguments.to_string(),
                    },
                })
                .collect();
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                tool_calls: Some(chat_tool_calls),
                tool_call_id: None,
            });

            for result in &round.results {
                messages.push(ChatMessage {
                    role: "tool".into(),
                    content: Some(result.content.clone()),
                    tool_calls: None,
                    tool_call_id: Some(result.tool_call_id.clone()),
                });
            }
        }

        let tools: Option<Vec<ChatTool>> = request.tools.as_ref().map(|defs| {
            defs.iter()
                .map(|d| ChatTool {
                    tool_type: "function".into(),
                    function: ChatFunction {
                        name: d.name.clone(),
                        description: d.description.clone(),
                        parameters: d.parameters.clone(),
                    },
                })
                .collect()
        });

        ChatRequest {
            model: self.model.clone(),
            messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tools,
            stream: if stream { Some(true) } else { None },
        }
    }

    /// Real SSE streaming implementation.
    ///
    /// Sends a request with `stream: true`, reads the response body,
    /// parses SSE `data:` lines, and emits `StreamEvent` deltas through the channel.
    async fn complete_streaming_sse(
        &self,
        request: &CompletionRequest,
        tx: mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<CompletionResponse> {
        let chat_request = self.build_chat_request(request, true);
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&chat_request)
            .send()
            .await
            .context("failed to send streaming request")?;

        let status = response.status();
        if !status.is_success() {
            let error_msg = response
                .json::<ErrorResponse>()
                .await
                .map(|e| e.error.message)
                .unwrap_or_else(|_| format!("HTTP {}", status));
            bail!("streaming request failed (HTTP {}): {}", status, error_msg);
        }

        let body = response
            .text()
            .await
            .context("failed to read streaming response body")?;

        let _ = tx.send(StreamEvent::Start);

        let mut full_content = String::new();
        let mut total_prompt = 0u32;
        let mut total_completion = 0u32;
        let mut parse_errors = 0u32;

        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() || line == "data: [DONE]" {
                continue;
            }
            if let Some(json_str) = line.strip_prefix("data: ") {
                match serde_json::from_str::<SseChunk>(json_str) {
                    Ok(chunk) => {
                        if let Some(delta) = chunk
                            .choices
                            .first()
                            .and_then(|c| c.delta.content.as_ref())
                        {
                            if !delta.is_empty() {
                                full_content.push_str(delta);
                                let _ = tx.send(StreamEvent::Delta(delta.clone()));
                            }
                        }
                        if let Some(usage) = chunk.usage {
                            total_prompt = usage.prompt_tokens;
                            total_completion = usage.completion_tokens;
                        }
                    }
                    Err(e) => {
                        parse_errors += 1;
                        warn!(error = %e, "failed to parse SSE chunk, skipping");
                    }
                }
            }
        }

        // If we got parse errors but no content, the stream was likely corrupted
        if full_content.is_empty() && parse_errors > 0 {
            bail!("SSE stream failed: {} chunks could not be parsed, no content received", parse_errors);
        }

        let _ = tx.send(StreamEvent::End {
            prompt_tokens: total_prompt,
            completion_tokens: total_completion,
            total_tokens: total_prompt + total_completion,
        });

        info!(
            model = %self.model,
            prompt_tokens = total_prompt,
            completion_tokens = total_completion,
            content_len = full_content.len(),
            "streaming response complete"
        );

        Ok(CompletionResponse {
            content: full_content,
            usage_prompt_tokens: total_prompt,
            usage_completion_tokens: total_completion,
            model: self.model.clone(),
            tool_calls: vec![],
        })
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for OpenAICompatibleAdapter {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        self.complete_with_retry(&request).await
    }

    async fn complete_streaming(
        &self,
        request: CompletionRequest,
        tx: mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<CompletionResponse> {
        self.complete_streaming_sse(&request, tx).await
    }

    fn capabilities(&self) -> &ProviderCapability {
        &self.capability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-PV-006 (partial): env var names
    #[test]
    fn env_var_names_correct() {
        assert_eq!(ENV_API_KEY, "TERRA_PROVIDER_API_KEY");
        assert_eq!(ENV_BASE_URL, "TERRA_PROVIDER_BASE_URL");
        assert_eq!(ENV_MODEL, "TERRA_PROVIDER_MODEL");
    }

    // T-PV-006: adapter creation
    #[test]
    fn openai_adapter_creation() {
        let adapter = OpenAICompatibleAdapter::from_env();
        // If env is set, should succeed; if not, should fail with clear error
        match adapter {
            Ok(a) => {
                assert_eq!(a.capability.provider_id, "openai_compatible");
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains(ENV_API_KEY) || msg.contains(ENV_DASHSCOPE_KEY));
            }
        }
    }

    // T-PV-007: adapter capabilities
    #[test]
    fn openai_adapter_capabilities() {
        let adapter = match OpenAICompatibleAdapter::from_env() {
            Ok(a) => a,
            Err(_) => return, // skip if no key
        };
        let cap = adapter.capabilities();
        assert_eq!(cap.provider_id, "openai_compatible");
        assert!(!cap.model_id.is_empty());
        assert!(cap.max_context_tokens > 0);
    }

    // T-PV-008: missing key error
    #[test]
    fn openai_missing_key_error() {
        let terra_key = env::var(ENV_API_KEY).ok();
        let dash_key = env::var(ENV_DASHSCOPE_KEY).ok();

        if terra_key.is_some() || dash_key.is_some() {
            return; // can't test missing key when keys are set
        }

        let result = OpenAICompatibleAdapter::from_env();
        assert!(result.is_err());
    }

    // T-PV-009: live completion (skippable)
    #[tokio::test]
    async fn openai_completion_live() {
        let adapter = match OpenAICompatibleAdapter::from_env() {
            Ok(a) => a,
            Err(_) => {
                eprintln!("SKIP: no provider API key set");
                return;
            }
        };

        let request = CompletionRequest {
            system_prompt: "You are a helpful assistant. Respond in one sentence.".into(),
            user_prompt: "What is 2 + 2?".into(),
            max_tokens: Some(50),
            temperature: Some(0.0),
            tools: None,
            tool_rounds: vec![],
        };

        let response = adapter.complete(request).await.unwrap();
        assert!(!response.content.is_empty());
        assert!(response.usage_prompt_tokens > 0);
        assert!(response.usage_completion_tokens > 0);
        assert!(!response.model.is_empty());
    }

    // --- SSE parsing unit tests ---

    // T-SSE-001: parse a single SSE chunk with delta content
    #[test]
    fn sse_chunk_parse_delta() {
        let json = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"delta":{"content":"Hello"},"index":0}]}"#;
        let chunk: SseChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(
            chunk.choices[0].delta.content.as_deref(),
            Some("Hello")
        );
        assert!(chunk.usage.is_none());
    }

    // T-SSE-002: parse SSE chunk with empty delta (role-only chunk)
    #[test]
    fn sse_chunk_parse_empty_delta() {
        let json = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"delta":{},"index":0}]}"#;
        let chunk: SseChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices[0].delta.content.is_none());
    }

    // T-SSE-003: parse SSE chunk with usage stats (final chunk from some providers)
    #[test]
    fn sse_chunk_parse_with_usage() {
        let json = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#;
        let chunk: SseChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices.is_empty());
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
    }

    // T-SSE-004: simulate full SSE body parsing
    #[test]
    fn sse_body_parse_full_stream() {
        let body = "\
data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"index\":0}]}\n\
\n\
data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"index\":0}]}\n\
\n\
data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"content\":\" world\"},\"index\":0}]}\n\
\n\
data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{},\"index\":0}],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":2}}\n\
\n\
data: [DONE]\n";

        let mut full_content = String::new();
        let mut total_prompt = 0u32;
        let mut total_completion = 0u32;
        let mut delta_count = 0usize;

        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() || line == "data: [DONE]" {
                continue;
            }
            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(chunk) = serde_json::from_str::<SseChunk>(json_str) {
                    if let Some(delta) = chunk
                        .choices
                        .first()
                        .and_then(|c| c.delta.content.as_ref())
                    {
                        if !delta.is_empty() {
                            full_content.push_str(delta);
                            delta_count += 1;
                        }
                    }
                    if let Some(usage) = chunk.usage {
                        total_prompt = usage.prompt_tokens;
                        total_completion = usage.completion_tokens;
                    }
                }
            }
        }

        assert_eq!(full_content, "Hello world");
        assert_eq!(delta_count, 2);
        assert_eq!(total_prompt, 8);
        assert_eq!(total_completion, 2);
    }

    // T-SSE-005: ChatRequest serializes stream field correctly
    #[test]
    fn chat_request_stream_field() {
        let req = ChatRequest {
            model: "test".into(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            tools: None,
            stream: Some(true),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["stream"], true);

        // When stream is None, field should be omitted
        let req_no_stream = ChatRequest {
            model: "test".into(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            tools: None,
            stream: None,
        };
        let json2 = serde_json::to_value(&req_no_stream).unwrap();
        assert!(json2.get("stream").is_none());
    }

    // Regression: every earlier tool round must be replayed after the user message.
    #[test]
    fn build_chat_request_replays_all_tool_rounds() {
        use crate::adapter::{ToolCall, ToolResult, ToolRound};
        let adapter = OpenAICompatibleAdapter::new(
            "test-key".into(),
            "http://localhost".into(),
            "test-model".into(),
            10,
        )
        .unwrap();
        let round = |id: &str| ToolRound {
            calls: vec![ToolCall { id: id.into(), name: "FileRead".into(), arguments: serde_json::json!({"path": id}) }],
            results: vec![ToolResult { tool_call_id: id.into(), content: format!("contents of {}", id) }],
        };
        let request = CompletionRequest {
            system_prompt: "system".into(),
            user_prompt: "user: summarize a and b".into(),
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_rounds: vec![round("a"), round("b")],
        };

        let messages = adapter.build_chat_request(&request, false).messages;
        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool", "assistant", "tool"]);
        assert_eq!(messages[1].content.as_deref(), Some("user: summarize a and b"));
        assert_eq!(messages[3].tool_call_id.as_deref(), Some("a"));
        assert_eq!(messages[5].tool_call_id.as_deref(), Some("b"));
    }

    // T-SSE-006: build_chat_request sets stream field
    #[test]
    fn build_chat_request_stream() {
        let adapter = OpenAICompatibleAdapter::new(
            "test-key".into(),
            "http://localhost".into(),
            "test-model".into(),
            10,
        )
        .unwrap();

        let request = CompletionRequest {
            system_prompt: "system".into(),
            user_prompt: "hello".into(),
            max_tokens: None,
            temperature: None,
            tools: None,
            tool_rounds: vec![],
        };

        let chat_req_stream = adapter.build_chat_request(&request, true);
        assert_eq!(chat_req_stream.stream, Some(true));

        let chat_req_no_stream = adapter.build_chat_request(&request, false);
        assert_eq!(chat_req_no_stream.stream, None);
    }

    // T-SSE-007: StreamEvent enum construction
    #[test]
    fn stream_event_variants() {
        let start = StreamEvent::Start;
        assert!(matches!(start, StreamEvent::Start));

        let delta = StreamEvent::Delta("token".into());
        match delta {
            StreamEvent::Delta(s) => assert_eq!(s, "token"),
            _ => panic!("expected Delta"),
        }

        let end = StreamEvent::End {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        };
        match end {
            StreamEvent::End {
                prompt_tokens,
                completion_tokens,
                total_tokens,
            } => {
                assert_eq!(prompt_tokens, 10);
                assert_eq!(completion_tokens, 5);
                assert_eq!(total_tokens, 15);
            }
            _ => panic!("expected End"),
        }
    }

    // T-SSE-008: mpsc channel streaming simulation
    #[tokio::test]
    async fn stream_event_channel_flow() {
        let (tx, mut rx) = mpsc::unbounded_channel::<StreamEvent>();

        // Simulate what complete_streaming does
        tx.send(StreamEvent::Start).unwrap();
        tx.send(StreamEvent::Delta("Hello".into())).unwrap();
        tx.send(StreamEvent::Delta(" world".into())).unwrap();
        tx.send(StreamEvent::End {
            prompt_tokens: 10,
            completion_tokens: 2,
            total_tokens: 12,
        })
        .unwrap();
        drop(tx);

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }

        assert_eq!(events.len(), 4);
        assert!(matches!(events[0], StreamEvent::Start));
        match &events[1] {
            StreamEvent::Delta(s) => assert_eq!(s, "Hello"),
            _ => panic!("expected Delta"),
        }
        match &events[2] {
            StreamEvent::Delta(s) => assert_eq!(s, " world"),
            _ => panic!("expected Delta"),
        }
        assert!(matches!(events[3], StreamEvent::End { .. }));
    }
}
