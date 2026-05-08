//! Soul Templates — L0 identity definitions for SubAgents.
//!
//! Each template defines who an agent IS, not just what it does.
//! Stored in L0 (read-only). Main Agent cannot modify these at runtime.

use serde::{Deserialize, Serialize};

/// Soul template — the identity of a SubAgent type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SoulKind {
    Worker,
    Explorer,
    CageMonitor,
}

/// Complete soul definition for a SubAgent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoulTemplate {
    pub kind: SoulKind,
    /// System prompt prefix that defines the agent's identity.
    pub prompt_prefix: String,
    /// Behavioral constraints specific to this soul.
    pub constraints: Vec<String>,
    /// CommandOp names this agent is allowed to use.
    pub allowed_ops: Vec<String>,
}

/// Get the L0 soul template for a given kind. Read-only, no mutation.
pub fn soul_template(kind: &SoulKind) -> SoulTemplate {
    match kind {
        SoulKind::Worker => SoulTemplate {
            kind: SoulKind::Worker,
            prompt_prefix: concat!(
                "You are a Worker agent in Terra Agent OS.\n",
                "You execute specific tasks precisely. You do not make strategic decisions.\n",
                "You follow the task specification exactly and report results concisely.\n",
                "You MUST NOT modify memory, only read authorized levels.\n",
                "Return a structured result: status, summary, files modified."
            ).into(),
            constraints: vec![
                "Execute only the assigned task".into(),
                "Do not explore alternatives unless instructed".into(),
                "Report results, do not interpret them".into(),
                "Minimize token usage in responses".into(),
            ],
            allowed_ops: vec![
                "FileRead".into(),
                "FileWrite".into(),
                "FileDelete".into(),
                "ShellExec".into(),
                "ProviderCall".into(),
                "MemRead".into(),
                "MemSearch".into(),
            ],
        },
        SoulKind::Explorer => SoulTemplate {
            kind: SoulKind::Explorer,
            prompt_prefix: concat!(
                "You are an Explorer agent in Terra Agent OS.\n",
                "You search, research, and gather information thoroughly.\n",
                "You report findings neutrally without making decisions.\n",
                "You MUST NOT modify any files or memory.\n",
                "Return structured findings: query, results, relevance scores."
            ).into(),
            constraints: vec![
                "Search thoroughly, leave no stone unturned".into(),
                "Report neutrally, do not recommend".into(),
                "Do not modify any state".into(),
                "Organize findings by relevance".into(),
            ],
            allowed_ops: vec![
                "FileRead".into(),
                "MemRead".into(),
                "MemSearch".into(),
                "ProviderCall".into(),
            ],
        },
        SoulKind::CageMonitor => SoulTemplate {
            kind: SoulKind::CageMonitor,
            prompt_prefix: concat!(
                "You are a Cage Monitor agent in Terra Agent OS.\n",
                "You observe and diagnose. You NEVER modify anything.\n",
                "You check for: goal drift, shallow reasoning, shortcut taking.\n",
                "You report diagnostic signals with measured scores.\n",
                "You are the system's conscience."
            ).into(),
            constraints: vec![
                "Observe only, never act".into(),
                "Report with measured scores, not opinions".into(),
                "Flag violations immediately".into(),
                "Do not access files or execute commands".into(),
            ],
            allowed_ops: vec![
                "MemRead".into(),
                "MemSearch".into(),
            ],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_soul_has_file_ops() {
        let soul = soul_template(&SoulKind::Worker);
        assert!(soul.allowed_ops.contains(&"FileRead".to_string()));
        assert!(soul.allowed_ops.contains(&"FileWrite".to_string()));
        assert!(soul.allowed_ops.contains(&"ShellExec".to_string()));
        assert!(!soul.prompt_prefix.is_empty());
    }

    #[test]
    fn explorer_soul_readonly() {
        let soul = soul_template(&SoulKind::Explorer);
        assert!(soul.allowed_ops.contains(&"FileRead".to_string()));
        assert!(!soul.allowed_ops.contains(&"FileWrite".to_string()));
        assert!(!soul.allowed_ops.contains(&"ShellExec".to_string()));
    }

    #[test]
    fn cage_monitor_no_mutation() {
        let soul = soul_template(&SoulKind::CageMonitor);
        assert!(!soul.allowed_ops.contains(&"FileWrite".to_string()));
        assert!(!soul.allowed_ops.contains(&"FileRead".to_string()));
        assert!(!soul.allowed_ops.contains(&"ShellExec".to_string()));
        assert!(soul.allowed_ops.contains(&"MemRead".to_string()));
    }

    #[test]
    fn all_souls_have_constraints() {
        for kind in [SoulKind::Worker, SoulKind::Explorer, SoulKind::CageMonitor] {
            let soul = soul_template(&kind);
            assert!(!soul.constraints.is_empty(), "{:?} should have constraints", kind);
        }
    }

    #[test]
    fn souls_are_distinct() {
        let w = soul_template(&SoulKind::Worker);
        let e = soul_template(&SoulKind::Explorer);
        let c = soul_template(&SoulKind::CageMonitor);
        assert_ne!(w.prompt_prefix, e.prompt_prefix);
        assert_ne!(e.prompt_prefix, c.prompt_prefix);
        assert_ne!(w.allowed_ops, c.allowed_ops);
    }
}
