//! The provider abstraction: a chat completion over a standard message list,
//! with native tool calling and optional JSON output.

use serde::{Deserialize, Serialize};

/// What a provider can do; the loop adapts to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapability {
    pub provider_id: String,
    pub model_id: String,
    pub supports_json_output: bool,
    pub supports_function_calling: bool,
    pub max_context_tokens: u32,
}

/// A tool the model may call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments.
    pub parameters: serde_json::Value,
}

/// A tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// One message in the conversation, in the shape every chat API uses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    System { content: String },
    User { content: String },
    Assistant {
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    Tool { tool_call_id: String, content: String },
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Message::System { content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Message::User { content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Message::Assistant { content: content.into(), tool_calls: Vec::new() }
    }
    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Message::Tool { tool_call_id: tool_call_id.into(), content: content.into() }
    }

    /// The text of the message (empty for pure tool-call messages).
    pub fn text(&self) -> &str {
        match self {
            Message::System { content } | Message::User { content } | Message::Tool { content, .. } => content,
            Message::Assistant { content, .. } => content,
        }
    }
}

/// A completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDef>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Ask for a JSON object instead of prose (structured output).
    #[serde(default)]
    pub json: bool,
}

impl CompletionRequest {
    /// A plain system + user call with no tools.
    pub fn simple(system: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            messages: vec![Message::system(system), Message::user(user)],
            tools: Vec::new(),
            max_tokens: None,
            temperature: None,
            json: false,
        }
    }

    /// The last user message, if any (used by test doubles and dry-run).
    pub fn last_user_text(&self) -> &str {
        self.messages
            .iter()
            .rev()
            .find_map(|m| match m {
                Message::User { content } => Some(content.as_str()),
                _ => None,
            })
            .unwrap_or("")
    }
}

/// A completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub usage_prompt_tokens: u32,
    pub usage_completion_tokens: u32,
    pub model: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

impl CompletionResponse {
    /// The assistant message to append to the conversation.
    pub fn as_message(&self) -> Message {
        Message::Assistant { content: self.content.clone(), tool_calls: self.tool_calls.clone() }
    }
}

/// An LLM provider. Any OpenAI-compatible endpoint implements this.
#[async_trait::async_trait]
pub trait ProviderAdapter: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse>;
    fn capabilities(&self) -> &ProviderCapability;
}

/// A provider that answers without any network: echoes the last user message.
/// Terra runs on it when no API key is configured.
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
                supports_json_output: false,
                supports_function_calling: false,
                max_context_tokens: 128_000,
            },
        }
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for DryRunAdapter {
    async fn complete(&self, request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        let echo: String = request.last_user_text().chars().take(100).collect();
        Ok(CompletionResponse {
            content: format!("DRY_RUN response to: {}", echo),
            usage_prompt_tokens: 0,
            usage_completion_tokens: 0,
            model: "dry_run".into(),
            tool_calls: vec![],
        })
    }

    fn capabilities(&self) -> &ProviderCapability {
        &self.capability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dry_run_echoes_last_user_message() {
        let mut req = CompletionRequest::simple("sys", "first");
        req.messages.push(Message::assistant("ok"));
        req.messages.push(Message::user("second question"));
        let resp = DryRunAdapter::new().complete(req).await.unwrap();
        assert_eq!(resp.content, "DRY_RUN response to: second question");
        assert!(resp.tool_calls.is_empty());
    }

    #[test]
    fn message_json_uses_role_tag() {
        let json = serde_json::to_string(&Message::tool("c1", "done")).unwrap();
        assert!(json.contains("\"role\":\"tool\""));
        let json = serde_json::to_string(&Message::assistant("hi")).unwrap();
        assert!(!json.contains("tool_calls"), "empty tool_calls are omitted: {json}");
    }
}
