//! Mock server tests for OpenAI-compatible adapter.
//! T-PV-010: retry on 429
//! T-PV-011: timeout
//! T-PV-012: streaming (placeholder)

use std::sync::atomic::{AtomicU32, Ordering};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use terra_providers::adapter::{CompletionRequest, ProviderAdapter};
use terra_providers::openai::OpenAICompatibleAdapter;

fn test_request() -> CompletionRequest {
    CompletionRequest {
        system_prompt: "test".into(),
        user_prompt: "test".into(),
        max_tokens: Some(10),
        temperature: None,
    }
}

/// Custom responder that returns 429 for the first N calls, then 200.
struct RetryResponder {
    fail_count: AtomicU32,
    fail_until: u32,
}

impl RetryResponder {
    fn new(fail_until: u32) -> Self {
        Self {
            fail_count: AtomicU32::new(0),
            fail_until,
        }
    }
}

impl wiremock::Respond for RetryResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let current = self.fail_count.fetch_add(1, Ordering::SeqCst);
        if current < self.fail_until {
            ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "error": {"message": "rate limited", "type": "rate_limit"}
            }))
        } else {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": "ok", "role": "assistant"}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 1},
                "model": "test-model"
            }))
        }
    }
}

// T-PV-010: retry on 429
#[tokio::test]
async fn openai_retry_on_429() {
    let mock_server = MockServer::start().await;

    // First 2 calls return 429, third returns 200
    let responder = RetryResponder::new(2);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(responder)
        .expect(3) // exactly 3 calls: 2 retries + 1 success
        .mount(&mock_server)
        .await;

    let adapter = OpenAICompatibleAdapter::new(
        "test-key".into(),
        mock_server.uri(),
        "test-model".into(),
        30,
    )
    .unwrap();

    let result = adapter.complete(test_request()).await;
    assert!(result.is_ok(), "should succeed after retries: {:?}", result.err());
    assert_eq!(result.unwrap().content, "ok");
}

// T-PV-011: timeout
#[tokio::test]
async fn openai_timeout() {
    let mock_server = MockServer::start().await;

    // Server delays 5 seconds, adapter timeout is 2 seconds
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(5)))
        .mount(&mock_server)
        .await;

    let adapter = OpenAICompatibleAdapter::new(
        "test-key".into(),
        mock_server.uri(),
        "test-model".into(),
        2, // 2 second timeout
    )
    .unwrap();

    let result = adapter.complete(test_request()).await;
    assert!(result.is_err(), "should timeout");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("timeout") || err.contains("retries"),
        "error should mention timeout: {}",
        err
    );
}

// T-PV-012: streaming placeholder
#[tokio::test]
async fn openai_streaming_placeholder() {
    // Streaming is a future feature. This test documents the intent
    // and verifies the capability flag is set correctly.
    let adapter = match OpenAICompatibleAdapter::from_env() {
        Ok(a) => a,
        Err(_) => return, // skip if no env
    };
    assert!(
        adapter.capabilities().supports_streaming,
        "adapter should report streaming support"
    );
}
