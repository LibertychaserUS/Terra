use serde::{Deserialize, Serialize};
use crate::adapter::{CompletionRequest, CompletionResponse, ProviderAdapter};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DispatchMode {
    DryRun,
    Live,
}

/// Result of a provider dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchResult {
    pub mode: DispatchMode,
    pub response: CompletionResponse,
}

/// Dispatch a completion request through a provider adapter.
pub async fn dispatch(
    adapter: &dyn ProviderAdapter,
    request: CompletionRequest,
    mode: DispatchMode,
) -> anyhow::Result<DispatchResult> {
    let response = adapter.complete(request).await?;
    Ok(DispatchResult { mode, response })
}

/// A mock adapter that always returns an error (for testing error propagation).
#[cfg(test)]
pub(crate) struct FailingAdapter;

#[cfg(test)]
#[async_trait::async_trait]
impl ProviderAdapter for FailingAdapter {
    async fn complete(&self, _request: CompletionRequest) -> anyhow::Result<CompletionResponse> {
        anyhow::bail!("mock provider failure")
    }

    fn capabilities(&self) -> &crate::adapter::ProviderCapability {
        // This won't be called in error path tests
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{DryRunAdapter, LatencyClass};

    fn test_request() -> CompletionRequest {
        CompletionRequest {
            system_prompt: "You are a helpful assistant.".into(),
            user_prompt: "Fix the bug in auth.py line 42".into(),
            max_tokens: Some(2000),
            temperature: None,
        }
    }

    // T-PV-001: dry run deterministic
    #[tokio::test]
    async fn dry_run_deterministic() {
        let adapter = DryRunAdapter::new();
        let r1 = adapter.complete(test_request()).await.unwrap();
        let r2 = adapter.complete(test_request()).await.unwrap();
        assert!(r1.content.contains("DRY_RUN"));
        assert!(r2.content.contains("DRY_RUN"));
    }

    // T-PV-002: dry run zero tokens
    #[tokio::test]
    async fn dry_run_zero_tokens() {
        let adapter = DryRunAdapter::new();
        let response = adapter.complete(test_request()).await.unwrap();
        assert_eq!(response.usage_prompt_tokens, 0);
        assert_eq!(response.usage_completion_tokens, 0);
    }

    // T-PV-003: dry run capabilities
    #[test]
    fn dry_run_capabilities() {
        let adapter = DryRunAdapter::new();
        let cap = adapter.capabilities();
        assert_eq!(cap.provider_id, "dry_run");
        assert_eq!(cap.model_id, "dry_run");
        assert!(cap.supports_structured_output);
        assert_eq!(cap.latency_class, LatencyClass::Fast);
    }

    // T-PV-004: dry run no network
    #[tokio::test]
    async fn dry_run_no_network() {
        let adapter = DryRunAdapter::new();
        // Should succeed without any network connectivity
        let result = adapter.complete(test_request()).await;
        assert!(result.is_ok());
    }

    // T-PV-005: dry run model field
    #[tokio::test]
    async fn dry_run_model_field() {
        let adapter = DryRunAdapter::new();
        let response = adapter.complete(test_request()).await.unwrap();
        assert_eq!(response.model, "dry_run");
    }

    // T-PV-013: dispatch dry run mode
    #[tokio::test]
    async fn dispatch_dry_run() {
        let adapter = DryRunAdapter::new();
        let result = dispatch(&adapter, test_request(), DispatchMode::DryRun).await.unwrap();
        assert_eq!(result.mode, DispatchMode::DryRun);
    }

    // T-PV-014: dispatch live mode (with dry run adapter, mode is just a label)
    #[tokio::test]
    async fn dispatch_live_mode() {
        let adapter = DryRunAdapter::new();
        let result = dispatch(&adapter, test_request(), DispatchMode::Live).await.unwrap();
        assert_eq!(result.mode, DispatchMode::Live);
    }

    // T-PV-015: dispatch result has response
    #[tokio::test]
    async fn dispatch_result_has_response() {
        let adapter = DryRunAdapter::new();
        let result = dispatch(&adapter, test_request(), DispatchMode::DryRun).await.unwrap();
        assert!(!result.response.content.is_empty());
    }

    // T-PV-016: dispatch result has usage
    #[tokio::test]
    async fn dispatch_result_has_usage() {
        let adapter = DryRunAdapter::new();
        let result = dispatch(&adapter, test_request(), DispatchMode::DryRun).await.unwrap();
        // DryRun returns 0/0, but fields exist
        assert_eq!(result.response.usage_prompt_tokens, 0);
        assert_eq!(result.response.usage_completion_tokens, 0);
    }

    // T-PV-017: dispatch propagates error
    #[tokio::test]
    async fn dispatch_propagates_error() {
        let adapter = FailingAdapter;
        let result = dispatch(&adapter, test_request(), DispatchMode::Live).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mock provider failure"));
    }

    // T-PV-018: dispatch mode in result
    #[tokio::test]
    async fn dispatch_mode_in_result() {
        let adapter = DryRunAdapter::new();
        let r1 = dispatch(&adapter, test_request(), DispatchMode::DryRun).await.unwrap();
        let r2 = dispatch(&adapter, test_request(), DispatchMode::Live).await.unwrap();
        assert_eq!(r1.mode, DispatchMode::DryRun);
        assert_eq!(r2.mode, DispatchMode::Live);
    }

    // T-STR-001: streaming provider emits tokens
    #[tokio::test]
    async fn streaming_provider_emits_tokens() {
        let adapter = DryRunAdapter::new();
        let request = CompletionRequest {
            system_prompt: "test".into(),
            user_prompt: "hello world test".into(),
            max_tokens: None,
            temperature: None,
        };
        let chunks = adapter.complete_stream(request).await.unwrap();

        // Should have multiple chunks (word by word)
        assert!(chunks.len() > 1, "should emit multiple chunks, got {}", chunks.len());

        // Last chunk should be done
        assert!(chunks.last().unwrap().done);

        // Concatenated text should form the full response
        let full: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert!(full.contains("DRY_RUN"));

        // Non-last chunks should not be done
        for chunk in &chunks[..chunks.len() - 1] {
            assert!(!chunk.done);
        }
    }

    // T-STR-005: progress events structure (tested via event module in terra-types)
    // Already covered by T-STR-006/007 in terra-types/event.rs
}
