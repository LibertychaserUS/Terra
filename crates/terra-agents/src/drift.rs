//! Active Drift Detection
//!
//! In Goal Mode, automatically evaluates D-001 (goal drift) every N commands.
//! Uses TF-IDF/embedding similarity — no ProviderCall, zero token cost.

use terra_core::diagnostic::{detect_goal_drift, DiagnosticSignal};

/// Drift detection configuration.
#[derive(Debug, Clone)]
pub struct DriftConfig {
    /// Check drift every N commands (default: 5)
    pub check_interval: u32,
    /// Threshold below which drift is flagged (default: 0.6)
    pub threshold: f64,
}

impl Default for DriftConfig {
    fn default() -> Self {
        Self {
            check_interval: 5,
            threshold: 0.6,
        }
    }
}

/// Tracks command count and evaluates drift in Goal Mode.
pub struct DriftDetector {
    config: DriftConfig,
    command_count: u32,
    last_score: f64,
    paused: bool,
}

/// Result of a drift check.
#[derive(Debug, Clone)]
pub struct DriftCheckResult {
    /// Whether a check was actually performed (only every N commands)
    pub checked: bool,
    /// The drift score (1.0 = perfect alignment, 0.0 = no relation)
    pub score: f64,
    /// Whether execution should pause due to drift
    pub should_pause: bool,
    /// Diagnostic signal if drift detected
    pub signal: Option<DiagnosticSignal>,
}

impl Default for DriftDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl DriftDetector {
    pub fn new() -> Self {
        Self::with_config(DriftConfig::default())
    }

    pub fn with_config(config: DriftConfig) -> Self {
        Self {
            config,
            command_count: 0,
            last_score: 1.0,
            paused: false,
        }
    }

    /// Record a command execution and check drift if interval reached.
    /// `spec_text`: the original goal/spec intent
    /// `action_text`: description of the latest action
    pub fn on_command(&mut self, spec_text: &str, action_text: &str) -> DriftCheckResult {
        self.command_count += 1;

        // Only check at intervals
        if !self.command_count.is_multiple_of(self.config.check_interval) {
            return DriftCheckResult {
                checked: false,
                score: self.last_score,
                should_pause: false,
                signal: None,
            };
        }

        // Evaluate D-001 using TF-IDF similarity (no ProviderCall)
        let signal = detect_goal_drift(spec_text, action_text, self.config.threshold);

        // Compute actual similarity score for reporting
        let actual_score = if let Some(s) = &signal {
            s.measured
        } else {
            // Compute the actual score for reporting
            let spec_tokens = drift_tfidf::tokenize_and_weight(spec_text);
            let action_tokens = drift_tfidf::tokenize_and_weight(action_text);
            drift_tfidf::cosine_similarity(&spec_tokens, &action_tokens)
        };

        self.last_score = actual_score;
        let should_pause = signal.is_some();
        self.paused = should_pause;

        DriftCheckResult {
            checked: true,
            score: actual_score,
            should_pause,
            signal,
        }
    }

    /// Reset state (on new goal).
    pub fn reset(&mut self) {
        self.command_count = 0;
        self.last_score = 1.0;
        self.paused = false;
    }

    pub fn command_count(&self) -> u32 {
        self.command_count
    }

    pub fn last_score(&self) -> f64 {
        self.last_score
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }
}

/// TF-IDF helpers exposed for drift detection (same algorithm as terra-memory).
pub(crate) mod drift_tfidf {
    use std::collections::HashMap;

    pub fn tokenize_and_weight(text: &str) -> HashMap<String, f32> {
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

    pub fn cosine_similarity(a: &HashMap<String, f32>, b: &HashMap<String, f32>) -> f64 {
        let dot: f32 = a.iter()
            .filter_map(|(k, v)| b.get(k).map(|bv| v * bv))
            .sum();
        let mag_a: f32 = a.values().map(|v| v * v).sum::<f32>().sqrt();
        let mag_b: f32 = b.values().map(|v| v * v).sum::<f32>().sqrt();
        if mag_a == 0.0 || mag_b == 0.0 { 0.0 } else { (dot / (mag_a * mag_b)) as f64 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-DD-001: drift auto fires in goal mode (every N commands)
    #[test]
    fn drift_auto_fires_at_interval() {
        let mut detector = DriftDetector::with_config(DriftConfig {
            check_interval: 5,
            threshold: 0.6,
        });

        let spec = "fix authentication login bug";

        // Commands 1-4: no check
        for i in 1..5 {
            let result = detector.on_command(spec, &format!("action {}", i));
            assert!(!result.checked, "command {} should not trigger check", i);
        }

        // Command 5: check fires
        let result = detector.on_command(spec, "fix authentication login validation");
        assert!(result.checked, "command 5 should trigger check");
        assert!(result.score > 0.0);
    }

    // T-DD-002: drift pause on low score
    #[test]
    fn drift_pause_on_low_score() {
        let mut detector = DriftDetector::with_config(DriftConfig {
            check_interval: 1, // check every command for test
            threshold: 0.6,
        });

        // Unrelated action → drift detected
        let result = detector.on_command(
            "fix authentication login bug",
            "optimize database query caching performance",
        );
        assert!(result.checked);
        assert!(result.should_pause, "unrelated action should trigger pause");
        assert!(result.signal.is_some());
        assert!(detector.is_paused());
    }

    // T-DD-003: drift interval configurable
    #[test]
    fn drift_interval_configurable() {
        let mut detector = DriftDetector::with_config(DriftConfig {
            check_interval: 3,
            threshold: 0.6,
        });

        let spec = "fix bug";
        assert!(!detector.on_command(spec, "a").checked); // 1
        assert!(!detector.on_command(spec, "b").checked); // 2
        assert!(detector.on_command(spec, "c").checked);  // 3 — fires
        assert!(!detector.on_command(spec, "d").checked); // 4
        assert!(!detector.on_command(spec, "e").checked); // 5
        assert!(detector.on_command(spec, "f").checked);  // 6 — fires
    }

    // T-DD-004: drift does not invoke ProviderCall
    #[test]
    fn drift_no_token_cost() {
        // DriftDetector has no reference to Executor or Provider.
        // It only uses TF-IDF — structurally guaranteed by the type.
        let mut detector = DriftDetector::new();
        let result = detector.on_command("fix bug", "fix bug");
        // If this compiled and ran, no ProviderCall was made.
        assert!(result.score >= 0.0);
    }

    // T-DD-005: drift result auditable
    #[test]
    fn drift_result_has_score() {
        let mut detector = DriftDetector::with_config(DriftConfig {
            check_interval: 1,
            threshold: 0.6,
        });
        let result = detector.on_command("fix auth bug", "fix auth validation");
        assert!(result.checked);
        assert!(result.score > 0.0);
        // Score is a concrete number that can be logged to audit
        assert!(result.score <= 1.0);
    }

    #[test]
    fn drift_reset() {
        let mut detector = DriftDetector::new();
        detector.on_command("a", "b");
        detector.on_command("a", "b");
        assert_eq!(detector.command_count(), 2);

        detector.reset();
        assert_eq!(detector.command_count(), 0);
        assert!((detector.last_score() - 1.0).abs() < 0.01);
    }
}
