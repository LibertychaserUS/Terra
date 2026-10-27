//! OpenAI-compatible provider built on the `async-openai` client.
//!
//! Works with any endpoint that speaks the chat completions API: OpenAI,
//! DashScope (the default), Ollama, vLLM, LiteLLM. The crate handles the
//! wire format, streaming and retries; this file only maps Terra's
//! `Message`/`ToolDef` types onto it.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessage,
    ChatCompletionRequestAssistantMessageContent, ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestToolMessage, ChatCompletionRequestUserMessage, ChatCompletionTool, ChatCompletionTools,
    CreateChatCompletionRequest, FunctionCall, FunctionObject, ResponseFormat,
};
use async_openai::Client;
use tracing::warn;

use crate::adapter::{CompletionRequest, CompletionResponse, Message, ProviderAdapter, ProviderCapability, ToolCall};

pub const DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
pub const DEFAULT_MODEL: &str = "qwen-plus";
/// HTTP timeout for one provider request.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Attempts per request on rate limits, server errors and timeouts.
const MAX_ATTEMPTS: u32 = 3;

pub struct OpenAICompatibleAdapter {
    client: Client<OpenAIConfig>,
    model: String,
    capability: ProviderCapability,
}

impl std::fmt::Debug for OpenAICompatibleAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAICompatibleAdapter").field("model", &self.model).finish()
    }
}

impl OpenAICompatibleAdapter {
    pub fn new(api_key: String, base_url: String, model: String, timeout_secs: u64) -> Result<Self> {
        if api_key.trim().is_empty() {
            bail!("provider API key is empty");
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .context("building HTTP client")?;
        let config = OpenAIConfig::new()
            .with_api_key(api_key)
            .with_api_base(base_url.trim_end_matches('/'));
        let client = Client::with_config(config).with_http_client(http);
        let provider_id = if base_url.contains("dashscope") {
            "dashscope"
        } else if base_url.contains("openai.com") {
            "openai"
        } else {
            "openai_compatible"
        };
        Ok(Self {
            client,
            capability: ProviderCapability {
                provider_id: provider_id.into(),
                model_id: model.clone(),
                supports_json_output: true,
                supports_function_calling: true,
                max_context_tokens: 128_000,
            },
            model,
        })
    }

    fn build_request(&self, request: &CompletionRequest) -> CreateChatCompletionRequest {
        let messages = request.messages.iter().map(to_openai_message).collect();
        let tools: Vec<ChatCompletionTools> = request
            .tools
            .iter()
            .map(|t| {
                ChatCompletionTools::Function(ChatCompletionTool {
                    function: FunctionObject {
                        name: t.name.clone(),
                        description: Some(t.description.clone()),
                        parameters: Some(t.parameters.clone()),
                        strict: None,
                    },
                })
            })
            .collect();
        #[allow(deprecated)] // DashScope's compatible mode reads max_tokens, not max_completion_tokens
        CreateChatCompletionRequest {
            model: self.model.clone(),
            messages,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            tools: if tools.is_empty() { None } else { Some(tools) },
            response_format: request.json.then_some(ResponseFormat::JsonObject),
            ..Default::default()
        }
    }
}

fn to_openai_message(message: &Message) -> ChatCompletionRequestMessage {
    match message {
        Message::System { content } => ChatCompletionRequestSystemMessage::from(content.as_str()).into(),
        Message::User { content } => ChatCompletionRequestUserMessage::from(content.as_str()).into(),
        Message::Assistant { content, tool_calls } => ChatCompletionRequestAssistantMessage {
            content: (!content.is_empty()).then(|| ChatCompletionRequestAssistantMessageContent::Text(content.clone())),
            tool_calls: (!tool_calls.is_empty()).then(|| {
                tool_calls
                    .iter()
                    .map(|c| {
                        ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                            id: c.id.clone(),
                            function: FunctionCall { name: c.name.clone(), arguments: c.arguments.to_string() },
                        })
                    })
                    .collect()
            }),
            ..Default::default()
        }
        .into(),
        Message::Tool { tool_call_id, content } => ChatCompletionRequestToolMessage {
            content: content.as_str().into(),
            tool_call_id: tool_call_id.clone(),
        }
        .into(),
    }
}

fn is_retryable(err: &async_openai::error::OpenAIError) -> bool {
    use async_openai::error::OpenAIError;
    match err {
        OpenAIError::Reqwest(e) => e.is_timeout() || e.is_connect(),
        OpenAIError::ApiError(resp) => {
            resp.status_code.as_u16() == 429
                || resp.status_code.is_server_error()
                || resp.api_error.r#type.as_deref() == Some("rate_limit_error")
        }
        _ => false,
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for OpenAICompatibleAdapter {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        let body = self.build_request(&request);
        let mut delay = Duration::from_secs(1);
        let mut last_err = None;
        for attempt in 1..=MAX_ATTEMPTS {
            match self.client.chat().create(body.clone()).await {
                Ok(response) => {
                    let choice = response.choices.into_iter().next().context("provider returned no choices")?;
                    let tool_calls = choice
                        .message
                        .tool_calls
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|c| match c {
                            ChatCompletionMessageToolCalls::Function(f) => Some(ToolCall {
                                id: f.id,
                                name: f.function.name,
                                arguments: serde_json::from_str(&f.function.arguments)
                                    .unwrap_or_else(|_| serde_json::json!({ "_raw": f.function.arguments })),
                            }),
                            ChatCompletionMessageToolCalls::Custom(_) => None,
                        })
                        .collect();
                    let usage = response.usage.unwrap_or_default();
                    return Ok(CompletionResponse {
                        content: choice.message.content.unwrap_or_default(),
                        usage_prompt_tokens: usage.prompt_tokens,
                        usage_completion_tokens: usage.completion_tokens,
                        model: response.model,
                        tool_calls,
                    });
                }
                Err(e) if is_retryable(&e) && attempt < MAX_ATTEMPTS => {
                    warn!(attempt, error = %e, "provider call failed, retrying");
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                    last_err = Some(e);
                }
                Err(e) => return Err(anyhow::Error::new(e).context("provider call failed")),
            }
        }
        Err(anyhow::anyhow!(
            "provider call failed after {} retries: {}",
            MAX_ATTEMPTS,
            last_err.map(|e| e.to_string()).unwrap_or_default()
        ))
    }

    fn capabilities(&self) -> &ProviderCapability {
        &self.capability
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::ToolDef;

    fn adapter() -> OpenAICompatibleAdapter {
        OpenAICompatibleAdapter::new("k".into(), "http://localhost:1".into(), "m".into(), 1).unwrap()
    }

    #[test]
    fn request_keeps_message_roles_and_tool_rounds() {
        let req = CompletionRequest {
            messages: vec![
                Message::system("sys"),
                Message::user("read a"),
                Message::Assistant {
                    content: String::new(),
                    tool_calls: vec![ToolCall { id: "c1".into(), name: "FileRead".into(), arguments: serde_json::json!({"path": "a"}) }],
                },
                Message::tool("c1", "alpha"),
            ],
            tools: vec![ToolDef { name: "FileRead".into(), description: "read".into(), parameters: serde_json::json!({"type": "object"}) }],
            max_tokens: Some(10),
            temperature: None,
            json: false,
        };
        let body = adapter().build_request(&req);
        let json = serde_json::to_value(&body).unwrap();
        let roles: Vec<&str> = json["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool"]);
        assert_eq!(json["messages"][2]["tool_calls"][0]["function"]["name"], "FileRead");
        assert_eq!(json["messages"][3]["tool_call_id"], "c1");
        assert_eq!(json["tools"][0]["function"]["name"], "FileRead");
        assert!(json.get("response_format").is_none());
    }

    #[test]
    fn json_mode_sets_response_format() {
        let mut req = CompletionRequest::simple("s", "u");
        req.json = true;
        let json = serde_json::to_value(adapter().build_request(&req)).unwrap();
        assert_eq!(json["response_format"]["type"], "json_object");
    }

    #[test]
    fn empty_key_is_rejected() {
        assert!(OpenAICompatibleAdapter::new(" ".into(), "http://x".into(), "m".into(), 1).is_err());
    }
}
