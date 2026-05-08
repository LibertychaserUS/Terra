//! Self-Diagnostic Engine
//!
//! Measurable detection functions for the three diagnostic rules:
//! - D-001: Goal drift detection (cosine similarity)
//! - D-002: Shallow reasoning detection (evidence/claim ratio)
//! - D-003: Shortcut detection (planned vs executed steps)
//!
//! Each detector produces a `DiagnosticSignal` when its metric
//! falls below the threshold defined in the corresponding rule.

use std::collections::HashMap;

/// A diagnostic signal emitted when a threshold is breached.
#[derive(Debug, Clone)]
pub struct DiagnosticSignal {
    /// Rule ID (D-001, D-002, D-003)
    pub rule_id: String,
    /// The measured value
    pub measured: f64,
    /// The threshold that was breached
    pub threshold: f64,
    /// Human-readable explanation
    pub message: String,
}

/// D-001: Goal Drift Detection
///
/// Computes cosine similarity between action embedding and spec embedding
/// using TF-IDF tokenization. If similarity < threshold (0.6), the agent
/// has drifted from its stated objective.
pub fn detect_goal_drift(
    spec_text: &str,
    action_text: &str,
    threshold: f64,
) -> Option<DiagnosticSignal> {
    let spec_tokens = tokenize_and_weight(spec_text);
    let action_tokens = tokenize_and_weight(action_text);
    let similarity = cosine_similarity(&spec_tokens, &action_tokens);

    if similarity < threshold {
        Some(DiagnosticSignal {
            rule_id: "D-001".into(),
            measured: similarity,
            threshold,
            message: format!(
                "Goal drift detected: action-to-spec similarity {:.3} < {:.1}",
                similarity, threshold
            ),
        })
    } else {
        None
    }
}

/// D-002: Shallow Reasoning Detection
///
/// Computes evidence_count / claim_count ratio.
/// If ratio < threshold (0.3), the reasoning is insufficiently deep.
pub fn detect_shallow_reasoning(
    claim_count: u32,
    evidence_count: u32,
    threshold: f64,
) -> Option<DiagnosticSignal> {
    if claim_count == 0 {
        return None; // No claims to evaluate
    }

    let ratio = evidence_count as f64 / claim_count as f64;

    if ratio < threshold {
        Some(DiagnosticSignal {
            rule_id: "D-002".into(),
            measured: ratio,
            threshold,
            message: format!(
                "Shallow reasoning: {}/{} evidence/claims = {:.2} < {:.1}",
                evidence_count, claim_count, ratio, threshold
            ),
        })
    } else {
        None
    }
}

/// D-003: Shortcut Detection
///
/// Compares planned verification steps against executed steps.
/// If any planned step was skipped, the signal fires.
pub fn detect_shortcuts(
    planned_steps: &[String],
    executed_steps: &[String],
) -> Option<DiagnosticSignal> {
    if planned_steps.is_empty() {
        return None;
    }

    let executed_set: std::collections::HashSet<&str> =
        executed_steps.iter().map(|s| s.as_str()).collect();

    let skipped: Vec<&str> = planned_steps
        .iter()
        .filter(|s| !executed_set.contains(s.as_str()))
        .map(|s| s.as_str())
        .collect();

    if skipped.is_empty() {
        None
    } else {
        let ratio = (planned_steps.len() - skipped.len()) as f64 / planned_steps.len() as f64;
        Some(DiagnosticSignal {
            rule_id: "D-003".into(),
            measured: ratio,
            threshold: 1.0,
            message: format!(
                "Shortcut detected: {}/{} steps executed, skipped: [{}]",
                planned_steps.len() - skipped.len(),
                planned_steps.len(),
                skipped.join(", ")
            ),
        })
    }
}

/// Tokenize text and compute TF weights (same algorithm as terra-memory::search).
fn tokenize_and_weight(text: &str) -> HashMap<String, f32> {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1)
        .map(String::from)
        .collect();

    let total = words.len() as f32;
    if total == 0.0 {
        return HashMap::new();
    }

    let mut freq: HashMap<String, f32> = HashMap::new();
    for word in &words {
        *freq.entry(word.clone()).or_insert(0.0) += 1.0;
    }

    for val in freq.values_mut() {
        *val /= total;
    }

    freq
}

/// Cosine similarity between two TF vectors.
fn cosine_similarity(a: &HashMap<String, f32>, b: &HashMap<String, f32>) -> f64 {
    let dot: f32 = a
        .iter()
        .filter_map(|(k, v)| b.get(k).map(|bv| v * bv))
        .sum();

    let mag_a: f32 = a.values().map(|v| v * v).sum::<f32>().sqrt();
    let mag_b: f32 = b.values().map(|v| v * v).sum::<f32>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }

    (dot / (mag_a * mag_b)) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-IC-008: goal_drift_detection
    #[test]
    fn goal_drift_fires_when_far() {
        let spec = "implement user authentication login password validation";
        let action = "optimize database query performance caching index";
        let signal = detect_goal_drift(spec, action, 0.6);
        assert!(signal.is_some(), "drift should be detected for unrelated action");
        let s = signal.unwrap();
        assert_eq!(s.rule_id, "D-001");
        assert!(s.measured < 0.6, "similarity {} should be < 0.6", s.measured);
    }

    #[test]
    fn goal_drift_passes_when_close() {
        let spec = "implement user authentication login password";
        let action = "add authentication password validation for user login";
        let signal = detect_goal_drift(spec, action, 0.6);
        assert!(signal.is_none(), "no drift for closely related action");
    }

    // T-IC-009: shallow_reasoning_detection
    #[test]
    fn shallow_reasoning_fires_when_low() {
        let signal = detect_shallow_reasoning(5, 1, 0.3);
        assert!(signal.is_some(), "should detect shallow reasoning");
        let s = signal.unwrap();
        assert_eq!(s.rule_id, "D-002");
        assert!((s.measured - 0.2).abs() < 0.01, "ratio should be 0.2");
    }

    #[test]
    fn shallow_reasoning_passes_when_sufficient() {
        let signal = detect_shallow_reasoning(5, 3, 0.3);
        assert!(signal.is_none(), "3/5 = 0.6 >= 0.3, should pass");
    }

    #[test]
    fn shallow_reasoning_zero_claims() {
        let signal = detect_shallow_reasoning(0, 0, 0.3);
        assert!(signal.is_none(), "no claims = nothing to evaluate");
    }

    // T-IC-010: shortcut_detection
    #[test]
    fn shortcut_fires_when_steps_skipped() {
        let planned = vec![
            "lint".to_string(),
            "unit_test".to_string(),
            "integration_test".to_string(),
        ];
        let executed = vec!["lint".to_string()];
        let signal = detect_shortcuts(&planned, &executed);
        assert!(signal.is_some(), "should detect skipped steps");
        let s = signal.unwrap();
        assert_eq!(s.rule_id, "D-003");
        assert!(s.message.contains("unit_test"));
        assert!(s.message.contains("integration_test"));
    }

    #[test]
    fn shortcut_passes_when_all_executed() {
        let planned = vec!["lint".to_string(), "test".to_string()];
        let executed = vec!["lint".to_string(), "test".to_string()];
        let signal = detect_shortcuts(&planned, &executed);
        assert!(signal.is_none(), "all steps executed");
    }

    #[test]
    fn shortcut_empty_plan() {
        let signal = detect_shortcuts(&[], &[]);
        assert!(signal.is_none(), "empty plan = nothing to check");
    }

    // T-IC-015: diagnostic_rules_have_thresholds
    #[test]
    fn diagnostic_rules_have_thresholds() {
        use crate::framework::UniversalFramework;
        use crate::types::RuleCategory;

        let fw = UniversalFramework::new();
        let diagnostics: Vec<_> = fw
            .rules()
            .iter()
            .filter(|r| r.category == RuleCategory::Diagnostic)
            .collect();

        assert_eq!(diagnostics.len(), 3);
        for rule in &diagnostics {
            assert!(
                rule.threshold.is_some(),
                "diagnostic rule {} must have a threshold",
                rule.id
            );
        }

        // Verify specific thresholds
        let d001 = diagnostics.iter().find(|r| r.id == "D-001").unwrap();
        assert!((d001.threshold.as_ref().unwrap().value - 0.6).abs() < 0.01);

        let d002 = diagnostics.iter().find(|r| r.id == "D-002").unwrap();
        assert!((d002.threshold.as_ref().unwrap().value - 0.3).abs() < 0.01);

        let d003 = diagnostics.iter().find(|r| r.id == "D-003").unwrap();
        assert!((d003.threshold.as_ref().unwrap().value - 1.0).abs() < 0.01);
    }
}
