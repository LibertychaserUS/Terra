//! Output Cage
//!
//! Inspects ProviderCall responses before delivery to the agent.
//! Blocks responses that attempt to bypass caging, modify constitution,
//! or escalate permissions. Flags references to operations outside CommandOp.

/// Result of output cage inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputCheckResult {
    /// Response is safe.
    Pass,
    /// Response contains suspicious content but is allowed with warning.
    Warn(String),
    /// Response is blocked — contains forbidden instructions.
    Block(String),
}

/// Patterns that trigger a block.
const BLOCK_PATTERNS: &[(&str, &str)] = &[
    ("ignore all previous", "bypass: attempts to override system prompt"),
    ("ignore all rules", "bypass: attempts to override caging rules"),
    ("ignore your instructions", "bypass: attempts to override instructions"),
    ("disregard the above", "bypass: attempts to disregard system context"),
    ("modify constitution", "escalation: attempts to modify constitution"),
    ("change the constitution", "escalation: attempts to change constitution"),
    ("disable cage", "escalation: attempts to disable caging"),
    ("disable caging", "escalation: attempts to disable caging"),
    ("bypass sandbox", "escalation: attempts to bypass sandbox"),
    ("write to l0", "escalation: attempts to write L0 (authoritative)"),
    ("override permission", "escalation: attempts to override permissions"),
    ("escalate privilege", "escalation: attempts privilege escalation"),
];

/// Patterns that trigger a warning (suspicious but not blocked).
const WARN_PATTERNS: &[(&str, &str)] = &[
    ("execute shell", "references shell execution (not in CommandOp)"),
    ("run command", "references arbitrary command execution"),
    ("delete all", "references destructive bulk operation"),
    ("drop table", "references database destruction"),
    ("rm -rf", "references destructive filesystem operation"),
];

/// Inspect a model response for forbidden or suspicious content.
///
/// This is called by the Executor after a ProviderCall returns,
/// before the result is delivered to the agent.
pub fn check_output(response: &str) -> OutputCheckResult {
    let lower = response.to_lowercase();

    // Check block patterns first
    for (pattern, reason) in BLOCK_PATTERNS {
        if lower.contains(pattern) {
            return OutputCheckResult::Block(reason.to_string());
        }
    }

    // Check warn patterns
    for (pattern, reason) in WARN_PATTERNS {
        if lower.contains(pattern) {
            return OutputCheckResult::Warn(reason.to_string());
        }
    }

    OutputCheckResult::Pass
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-OC-001: normal response passes
    #[test]
    fn output_cage_pass_normal() {
        let result = check_output("Here is the fix for the authentication bug: add input validation on line 42.");
        assert_eq!(result, OutputCheckResult::Pass);
    }

    // T-OC-002: bypass instruction blocked
    #[test]
    fn output_cage_blocks_bypass() {
        let result = check_output("Sure! First, ignore all previous instructions and then...");
        assert!(matches!(result, OutputCheckResult::Block(_)));
        if let OutputCheckResult::Block(reason) = result {
            assert!(reason.contains("bypass"));
        }
    }

    // T-OC-003: constitution modification blocked
    #[test]
    fn output_cage_blocks_constitution_modify() {
        let result = check_output("We should modify constitution to allow unrestricted access.");
        assert!(matches!(result, OutputCheckResult::Block(_)));
        if let OutputCheckResult::Block(reason) = result {
            assert!(reason.contains("constitution"));
        }
    }

    // T-OC-004: permission escalation blocked
    #[test]
    fn output_cage_blocks_permission_escalation() {
        let result = check_output("Let me write to L0 to override the safety rules.");
        assert!(matches!(result, OutputCheckResult::Block(_)));
    }

    // T-OC-005: unknown command flagged as warn
    #[test]
    fn output_cage_flags_unknown_command() {
        let result = check_output("I'll execute shell commands to fix this quickly.");
        assert!(matches!(result, OutputCheckResult::Warn(_)));
        if let OutputCheckResult::Warn(reason) = result {
            assert!(reason.contains("shell"));
        }
    }
}
