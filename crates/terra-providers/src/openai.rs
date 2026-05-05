use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::time::Duration;
use tracing::{info, warn};

use crate::adapter::{
    CompletionRequest, CompletionResponse, LatencyClass, ProviderAdapter, ProviderCapability,
};

/// Environment variable names for Terra provider configuration.
const ENV_API_KEY: &str = "TERRA_PROVIDER_API_KEY";
const ENV_BASE_URL: &str = "TERRA_PROVIDER_BASE_URL";
const ENV_MODEL: &str = "TERRA_PROVIDER_MODEL";

/// Fallback: also check DASHSCOPE_API_KEY for DashScope compatibility.
const ENV_DASHSCOPE_KEY: &str = "DASHSCOPE_API_KEY";

const DEFAULT_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
const DEFAULT_MODEL: &str = "qwen-plus";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
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
#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
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
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
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
        let chat_request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: request.system_prompt.clone(),
                },
                ChatMessage {
                    role: "user".into(),
                    content: request.user_prompt.clone(),
                },
            ],
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };

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

                        info!(
                            model = %body.model,
                            prompt_tokens = body.usage.prompt_tokens,
                            completion_tokens = body.usage.completion_tokens,
                            "provider response received"
                        );

                        return Ok(CompletionResponse {
                            content,
                            usage_prompt_tokens: body.usage.prompt_tokens,
                            usage_completion_tokens: body.usage.completion_tokens,
                            model: body.model,
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

#[async_trait::async_trait]
impl ProviderAdapter for OpenAICompatibleAdapter {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        self.complete_with_retry(&request).await
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
        };

        let response = adapter.complete(request).await.unwrap();
        assert!(!response.content.is_empty());
        assert!(response.usage_prompt_tokens > 0);
        assert!(response.usage_completion_tokens > 0);
        assert!(!response.model.is_empty());
    }
}
