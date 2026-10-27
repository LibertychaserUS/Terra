//! The adapter against a mock HTTP server: retries, timeouts, tool calls.

use std::sync::atomic::{AtomicU32, Ordering};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use terra_providers::adapter::{CompletionRequest, ProviderAdapter};
use terra_providers::openai::OpenAICompatibleAdapter;

fn test_request() -> CompletionRequest {
    CompletionRequest::simple("test", "test")
}

fn adapter(uri: String, timeout: u64) -> OpenAICompatibleAdapter {
    OpenAICompatibleAdapter::new("test-key".into(), uri, "test-model".into(), timeout).unwrap()
}

/// Returns 429 for the first N calls, then 200.
struct RetryResponder {
    fail_count: AtomicU32,
    fail_until: u32,
}

impl wiremock::Respond for RetryResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let current = self.fail_count.fetch_add(1, Ordering::SeqCst);
        if current < self.fail_until {
            ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "error": {"message": "rate limited", "type": "rate_limit_error"}
            }))
        } else {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "x", "object": "chat.completion", "created": 0,
                "choices": [{"index": 0, "message": {"content": "ok", "role": "assistant"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6},
                "model": "test-model"
            }))
        }
    }
}

#[tokio::test]
async fn retries_on_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(RetryResponder { fail_count: AtomicU32::new(0), fail_until: 2 })
        .expect(3)
        .mount(&server)
        .await;

    let result = adapter(server.uri(), 30).complete(test_request()).await;
    assert!(result.is_ok(), "should succeed after retries: {:?}", result.err());
    let resp = result.unwrap();
    assert_eq!(resp.content, "ok");
    assert_eq!(resp.usage_prompt_tokens, 5);
}

#[tokio::test]
async fn times_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(5)))
        .mount(&server)
        .await;

    let result = adapter(server.uri(), 1).complete(test_request()).await;
    assert!(result.is_err(), "should time out");
}

#[tokio::test]
async fn parses_tool_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "x", "object": "chat.completion", "created": 0, "model": "test-model",
            "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": null,
                "tool_calls": [{"id": "call_1", "type": "function",
                    "function": {"name": "FileRead", "arguments": "{\"path\": \"src/lib.rs\"}"}}]
            }}],
            "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}
        })))
        .mount(&server)
        .await;

    let resp = adapter(server.uri(), 30).complete(test_request()).await.unwrap();
    assert_eq!(resp.content, "");
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "FileRead");
    assert_eq!(resp.tool_calls[0].arguments["path"], "src/lib.rs");
}

#[tokio::test]
async fn non_retryable_error_is_reported_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": {"message": "bad key", "type": "invalid_request_error"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let err = adapter(server.uri(), 30).complete(test_request()).await.unwrap_err();
    assert!(err.to_string().contains("provider call failed"), "{err}");
}
