use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Memory isolation scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Global,
    Project,
    Session,
    Task,
}

impl Scope {
    pub fn as_str(&self) -> &str {
        match self {
            Scope::Global => "global",
            Scope::Project => "project",
            Scope::Session => "session",
            Scope::Task => "task",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "global" => Some(Scope::Global),
            "project" => Some(Scope::Project),
            "session" => Some(Scope::Session),
            "task" => Some(Scope::Task),
            _ => None,
        }
    }

    /// Return all scopes visible from this scope level.
    /// Task can see: task + session + project + global
    /// Session can see: session + project + global
    /// Project can see: project + global
    /// Global can see: global only
    pub fn visible_scopes(&self) -> Vec<&'static str> {
        match self {
            Scope::Task => vec!["task", "session", "project", "global"],
            Scope::Session => vec!["session", "project", "global"],
            Scope::Project => vec!["project", "global"],
            Scope::Global => vec!["global"],
        }
    }
}

/// Memory semantic level (L0 = most authoritative, L6 = least).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Level {
    L0, // Authoritative: Constitution, Intelligence Core
    L1, // Project memory: architecture, decisions
    L2, // Task memory: current hypotheses, attempts
    L3, // Accepted paths: verified successful routes
    L4, // Failed path index: compressed failure records
    L5, // Dormant ideas: archived, low priority
    L6, // Model memory candidates: needs human review
}

impl Level {
    pub fn as_str(&self) -> &str {
        match self {
            Level::L0 => "L0",
            Level::L1 => "L1",
            Level::L2 => "L2",
            Level::L3 => "L3",
            Level::L4 => "L4",
            Level::L5 => "L5",
            Level::L6 => "L6",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "L0" => Some(Level::L0),
            "L1" => Some(Level::L1),
            "L2" => Some(Level::L2),
            "L3" => Some(Level::L3),
            "L4" => Some(Level::L4),
            "L5" => Some(Level::L5),
            "L6" => Some(Level::L6),
            _ => None,
        }
    }

    /// Whether this level is protected from garbage collection.
    pub fn is_gc_protected(&self) -> bool {
        matches!(self, Level::L0 | Level::L1)
    }
}

/// A single memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub scope: Scope,
    pub level: Level,
    pub owner: String,
    pub content: String,
    pub version: u64,
    pub alive: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_roundtrip() {
        for scope in [Scope::Global, Scope::Project, Scope::Session, Scope::Task] {
            assert_eq!(Scope::parse(scope.as_str()), Some(scope));
        }
    }

    #[test]
    fn level_roundtrip() {
        for level in [
            Level::L0, Level::L1, Level::L2, Level::L3,
            Level::L4, Level::L5, Level::L6,
        ] {
            assert_eq!(Level::parse(level.as_str()), Some(level));
        }
    }

    #[test]
    fn level_ordering() {
        assert!(Level::L0 < Level::L1);
        assert!(Level::L1 < Level::L6);
    }

    #[test]
    fn gc_protection() {
        assert!(Level::L0.is_gc_protected());
        assert!(Level::L1.is_gc_protected());
        assert!(!Level::L2.is_gc_protected());
        assert!(!Level::L5.is_gc_protected());
    }

    #[test]
    fn invalid_scope() {
        assert_eq!(Scope::parse("invalid"), None);
    }

    #[test]
    fn invalid_level() {
        assert_eq!(Level::parse("L9"), None);
    }

    #[test]
    fn scope_visibility() {
        assert_eq!(Scope::Task.visible_scopes(), vec!["task", "session", "project", "global"]);
        assert_eq!(Scope::Session.visible_scopes(), vec!["session", "project", "global"]);
        assert_eq!(Scope::Project.visible_scopes(), vec!["project", "global"]);
        assert_eq!(Scope::Global.visible_scopes(), vec!["global"]);
    }
}
