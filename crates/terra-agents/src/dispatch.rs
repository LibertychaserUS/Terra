//! Model Dispatch Policy
//!
//! Maps agent roles to model selection based on provider capabilities.
//! Multi-model: Main→strongest, Worker→balanced, Explorer→fast
//! Single-model: all agents same model, different params

use crate::roles::AgentRole;

/// A model entry in the provider's capability list.
#[derive(Debug, Clone)]
pub struct ModelCapability {
    pub model_id: String,
    /// Tier: 0 = fast/cheap, 1 = balanced, 2 = strongest
    pub tier: u32,
    pub max_context: u32,
}

/// Dispatch parameters for a specific agent invocation.
#[derive(Debug, Clone)]
pub struct DispatchParams {
    pub model_id: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// Model dispatch policy.
pub struct ModelDispatcher {
    models: Vec<ModelCapability>,
}

impl ModelDispatcher {
    /// Create a dispatcher with the given model capabilities.
    pub fn new(models: Vec<ModelCapability>) -> Self {
        Self { models }
    }

    /// Create a single-model dispatcher.
    pub fn single(model_id: &str) -> Self {
        Self {
            models: vec![ModelCapability {
                model_id: model_id.into(),
                tier: 1,
                max_context: 8192,
            }],
        }
    }

    /// Get dispatch parameters for an agent role.
    ///
    /// Multi-model strategy:
    /// - Main → highest tier (strongest reasoning)
    /// - Worker → tier 1 (balanced cost/quality)
    /// - Explorer → tier 0 (fast, cheap)
    /// - CageMonitor → tier 0 (lightweight checks)
    ///
    /// Single-model: same model, role-specific params.
    pub fn dispatch(&self, role: &AgentRole) -> DispatchParams {
        let model = self.select_model(role);

        let (max_tokens, temperature) = match role {
            AgentRole::Main => (4000, 0.0_f32),
            AgentRole::Worker => (2000, 0.0),
            AgentRole::Explorer => (500, 0.0),
            AgentRole::CageMonitor => (200, 0.0),
        };

        DispatchParams {
            model_id: model.model_id.clone(),
            max_tokens,
            temperature,
        }
    }

    /// Select the best model for a role.
    fn select_model(&self, role: &AgentRole) -> &ModelCapability {
        if self.models.len() == 1 {
            return &self.models[0];
        }

        let target_tier = match role {
            AgentRole::Main => 2,       // strongest
            AgentRole::Worker => 1,     // balanced
            AgentRole::Explorer => 0,   // fast
            AgentRole::CageMonitor => 0, // lightweight
        };

        // Find closest match to target tier
        self.models
            .iter()
            .min_by_key(|m| (m.tier as i32 - target_tier).unsigned_abs())
            .unwrap_or(&self.models[0])
    }

    /// Number of available models.
    pub fn model_count(&self) -> usize {
        self.models.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-AG-010: multi-model dispatch
    #[test]
    fn model_dispatch_multi_model() {
        let dispatcher = ModelDispatcher::new(vec![
            ModelCapability { model_id: "fast-model".into(), tier: 0, max_context: 4096 },
            ModelCapability { model_id: "balanced-model".into(), tier: 1, max_context: 8192 },
            ModelCapability { model_id: "strongest-model".into(), tier: 2, max_context: 32768 },
        ]);

        let main_params = dispatcher.dispatch(&AgentRole::Main);
        let worker_params = dispatcher.dispatch(&AgentRole::Worker);
        let explorer_params = dispatcher.dispatch(&AgentRole::Explorer);

        assert_eq!(main_params.model_id, "strongest-model");
        assert_eq!(worker_params.model_id, "balanced-model");
        assert_eq!(explorer_params.model_id, "fast-model");

        // Different max_tokens per role
        assert!(main_params.max_tokens > worker_params.max_tokens);
        assert!(worker_params.max_tokens > explorer_params.max_tokens);
    }

    // T-AG-011: single-model dispatch
    #[test]
    fn model_dispatch_single_model() {
        let dispatcher = ModelDispatcher::single("qwen-plus");

        let main_params = dispatcher.dispatch(&AgentRole::Main);
        let worker_params = dispatcher.dispatch(&AgentRole::Worker);
        let explorer_params = dispatcher.dispatch(&AgentRole::Explorer);

        // All same model
        assert_eq!(main_params.model_id, "qwen-plus");
        assert_eq!(worker_params.model_id, "qwen-plus");
        assert_eq!(explorer_params.model_id, "qwen-plus");

        // But different params
        assert_ne!(main_params.max_tokens, explorer_params.max_tokens);
    }

    #[test]
    fn cage_monitor_gets_lightweight() {
        let dispatcher = ModelDispatcher::new(vec![
            ModelCapability { model_id: "fast".into(), tier: 0, max_context: 4096 },
            ModelCapability { model_id: "strong".into(), tier: 2, max_context: 32768 },
        ]);

        let params = dispatcher.dispatch(&AgentRole::CageMonitor);
        assert_eq!(params.model_id, "fast");
        assert_eq!(params.max_tokens, 200);
    }
}
