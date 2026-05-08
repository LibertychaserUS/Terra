//! Smart Greeter
//!
//! Personalized startup suggestions based on:
//! - Last session (unfinished goals, recent topics)
//! - Memory system (L6 candidates pending review)
//! - Project state (git changes since last session)
//! - User custom entries (config.toml [greeter])
//!
//! GreeterProvider trait allows pluggable sources.

use std::path::Path;

/// A single suggestion displayed at startup.
#[derive(Debug, Clone)]
pub struct Suggestion {
    pub icon: &'static str,
    pub label: String,
    pub hint: String,
}

/// Trait for pluggable greeter sources.
pub trait GreeterProvider {
    fn suggestions(&self) -> Vec<Suggestion>;
}

/// Session-based greeter: unfinished goals from last session.
pub struct SessionGreeter {
    last_goal: Option<String>,
    last_turn_count: usize,
}

impl SessionGreeter {
    pub fn from_session_dir(terra_dir: &Path) -> Self {
        let sessions_dir = terra_dir.join("sessions");
        if !sessions_dir.exists() {
            return Self { last_goal: None, last_turn_count: 0 };
        }

        // Find newest session file
        let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(meta) = std::fs::metadata(&path) {
                        if let Ok(modified) = meta.modified() {
                            if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
                                newest = Some((modified, path));
                            }
                        }
                    }
                }
            }
        }

        if let Some((_, path)) = newest {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(record) = serde_json::from_str::<serde_json::Value>(&content) {
                    let goal = record.get("goal").and_then(|g| g.as_str()).map(String::from);
                    let turns = record.get("turns").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0);
                    return Self { last_goal: goal, last_turn_count: turns };
                }
            }
        }

        Self { last_goal: None, last_turn_count: 0 }
    }
}

impl GreeterProvider for SessionGreeter {
    fn suggestions(&self) -> Vec<Suggestion> {
        let mut out = Vec::new();
        if let Some(ref goal) = self.last_goal {
            out.push(Suggestion {
                icon: "★",
                label: format!("Continue: {}", goal),
                hint: format!("{} turns in last session", self.last_turn_count),
            });
        } else if self.last_turn_count > 0 {
            out.push(Suggestion {
                icon: "↩",
                label: "Resume last conversation".into(),
                hint: format!("{} turns", self.last_turn_count),
            });
        }
        out
    }
}

/// Git-based greeter: files changed since last session.
pub struct GitGreeter {
    changed_files: usize,
    branch: String,
}

impl GitGreeter {
    pub fn from_git_info(info: &crate::codebase::GitInfo) -> Self {
        Self {
            changed_files: info.changed_files,
            branch: info.branch.clone(),
        }
    }
}

impl GreeterProvider for GitGreeter {
    fn suggestions(&self) -> Vec<Suggestion> {
        if self.changed_files > 0 {
            vec![Suggestion {
                icon: "◆",
                label: format!("Review: {} files changed on {}", self.changed_files, self.branch),
                hint: "Use /commit when ready".into(),
            }]
        } else {
            vec![]
        }
    }
}

/// Default greeter: helpful starting points for new users.
pub struct DefaultGreeter;

impl GreeterProvider for DefaultGreeter {
    fn suggestions(&self) -> Vec<Suggestion> {
        vec![
            Suggestion {
                icon: "→",
                label: "Start coding: describe what you need".into(),
                hint: "e.g., \"add error handling to main.rs\"".into(),
            },
        ]
    }
}

/// Collect suggestions from all providers.
pub fn collect_suggestions(providers: &[&dyn GreeterProvider], max: usize) -> Vec<Suggestion> {
    let mut all: Vec<Suggestion> = providers.iter()
        .flat_map(|p| p.suggestions())
        .collect();
    all.truncate(max);

    // Always have at least the default if empty
    if all.is_empty() {
        all = DefaultGreeter.suggestions();
    }
    all
}

/// Render suggestions as formatted terminal output.
pub fn render_greeter(suggestions: &[Suggestion]) -> String {
    if suggestions.is_empty() { return String::new(); }

    let mut lines = vec![
        "  \x1b[2m┌─ What would you like to do? ─────────────────────┐\x1b[0m".into(),
        "  \x1b[2m│\x1b[0m".into(),
    ];

    for s in suggestions {
        lines.push(format!("  \x1b[2m│\x1b[0m  \x1b[36m{}\x1b[0m {}  \x1b[2m{}\x1b[0m", s.icon, s.label, s.hint));
    }

    lines.push("  \x1b[2m│\x1b[0m".into());
    lines.push("  \x1b[2m│\x1b[0m  Or just start typing to chat freely.".into());
    lines.push("  \x1b[2m└──────────────────────────────────────────────────┘\x1b[0m".into());

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-SG-001: greeter shows on startup
    #[test]
    fn greeter_renders() {
        let suggestions = vec![
            Suggestion { icon: "★", label: "Continue: fix auth".into(), hint: "5 turns".into() },
            Suggestion { icon: "◆", label: "Review: 3 files changed".into(), hint: "".into() },
        ];
        let output = render_greeter(&suggestions);
        assert!(output.contains("What would you like to do?"));
        assert!(output.contains("fix auth"));
        assert!(output.contains("3 files changed"));
    }

    // T-SG-002: greeter from last session
    #[test]
    fn session_greeter_with_goal() {
        let greeter = SessionGreeter {
            last_goal: Some("fix authentication".into()),
            last_turn_count: 12,
        };
        let suggestions = greeter.suggestions();
        assert_eq!(suggestions.len(), 1);
        assert!(suggestions[0].label.contains("Continue"));
        assert!(suggestions[0].label.contains("fix authentication"));
    }

    // T-SG-003: greeter from git
    #[test]
    fn git_greeter_with_changes() {
        let info = crate::codebase::GitInfo {
            is_repo: true,
            branch: "main".into(),
            has_changes: true,
            changed_files: 5,
        };
        let greeter = GitGreeter::from_git_info(&info);
        let suggestions = greeter.suggestions();
        assert_eq!(suggestions.len(), 1);
        assert!(suggestions[0].label.contains("5 files"));
    }

    #[test]
    fn default_greeter_when_empty() {
        let suggestions = collect_suggestions(&[], 5);
        assert!(!suggestions.is_empty(), "should have default suggestion");
    }

    #[test]
    fn collect_respects_max() {
        let s1 = SessionGreeter { last_goal: Some("a".into()), last_turn_count: 1 };
        let d = DefaultGreeter;
        let suggestions = collect_suggestions(&[&s1, &d], 1);
        assert_eq!(suggestions.len(), 1);
    }
}
