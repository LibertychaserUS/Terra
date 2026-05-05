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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::DryRunAdapter;

    #[tokio::test]
    async fn dry_run_dispatch() {
        let adapter = DryRunAdapter::new();
        let request = CompletionRequest {
            system_prompt: "You are a helpful assistant.".into(),
            user_prompt: "Fix the bug in auth.py line 42".into(),
            max_tokens: Some(2000),
            temperature: None,
        };

        let result = dispatch(&adapter, request, DispatchMode::DryRun).await.unwrap();
        assert_eq!(result.mode, DispatchMode::DryRun);
        assert!(result.response.content.contains("DRY_RUN"));
        assert_eq!(result.response.usage_prompt_tokens, 0);
    }

    #[test]
    fn dry_run_adapter_capabilities() {
        let adapter = DryRunAdapter::new();
        let cap = adapter.capabilities();
        assert_eq!(cap.provider_id, "dry_run");
        assert!(cap.supports_structured_output);
    }
}
