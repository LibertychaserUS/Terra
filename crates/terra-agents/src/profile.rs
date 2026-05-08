//! Agent Profile + Memory View
//!
//! AgentProfile: created by Main, validated by Cage, immutable for SubAgent.
//! MemoryView: read-only, level-filtered wrapper over MemoryDatabase.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use terra_core::soul::{SoulKind, SoulTemplate, soul_template};
use terra_memory::MemoryDatabase;
use terra_memory::types::MemoryEntry;

/// Agent profile — identity + permissions + budget.
/// Created by Main Agent, immutable once assigned to SubAgent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub soul_kind: SoulKind,
    /// Memory levels this agent can read (no writes).
    pub memory_access: Vec<String>,
    /// CommandOp names this agent can use.
    pub allowed_ops: Vec<String>,
    /// Max tokens this agent may consume.
    pub budget_max_tokens: u32,
}

impl AgentProfile {
    /// Create a profile from a SoulTemplate with default memory access.
    pub fn from_soul(id: &str, kind: SoulKind, budget: u32) -> Self {
        let template = soul_template(&kind);

        let memory_access = match kind {
            SoulKind::Worker => vec!["L2".into(), "L3".into(), "L4".into()],
            SoulKind::Explorer => vec!["L1".into(), "L2".into(), "L5".into()],
            SoulKind::CageMonitor => vec!["L0".into(), "L2".into()],
        };

        Self {
            id: id.into(),
            soul_kind: kind,
            memory_access,
            allowed_ops: template.allowed_ops,
            budget_max_tokens: budget,
        }
    }

    /// Get the soul template for this profile.
    pub fn soul(&self) -> SoulTemplate {
        soul_template(&self.soul_kind)
    }

    /// Check if this agent is allowed to use a given CommandOp.
    pub fn can_use_op(&self, op_name: &str) -> bool {
        self.allowed_ops.iter().any(|o| o == op_name)
    }

    /// Check if this agent can read a given memory level.
    pub fn can_read_level(&self, level: &str) -> bool {
        self.memory_access.iter().any(|l| l == level)
    }
}

/// Read-only, level-filtered view of MemoryDatabase.
///
/// This is what SubAgents receive — a shared borrow (&) of memory,
/// filtered to only the levels they're authorized to see.
/// No write methods exist on this type (compile-time enforced).
pub struct MemoryView {
    db: Arc<MemoryDatabase>,
    allowed_levels: Vec<String>,
}

impl MemoryView {
    /// Create a view from a database and authorized levels.
    pub fn new(db: Arc<MemoryDatabase>, allowed_levels: Vec<String>) -> Self {
        Self { db, allowed_levels }
    }

    /// Read a memory entry. Returns None if key doesn't exist or level not authorized.
    pub fn read(&self, key: &str) -> Option<MemoryEntry> {
        match self.db.read(key) {
            Ok(Some(entry)) => {
                if self.allowed_levels.iter().any(|l| l == entry.level.as_str()) {
                    Some(entry)
                } else {
                    None // level not authorized — invisible to this agent
                }
            }
            _ => None,
        }
    }

    /// Check if a key exists and is visible to this agent.
    pub fn exists(&self, key: &str) -> bool {
        self.read(key).is_some()
    }

    /// Get authorized levels.
    pub fn authorized_levels(&self) -> &[String] {
        &self.allowed_levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_memory::types::{Level, Scope};

    #[test]
    fn worker_profile_has_file_ops() {
        let profile = AgentProfile::from_soul("worker-1", SoulKind::Worker, 2000);
        assert!(profile.can_use_op("FileRead"));
        assert!(profile.can_use_op("FileWrite"));
        assert!(profile.can_use_op("ShellExec"));
        assert!(!profile.can_use_op("ArtifactSave"));
    }

    #[test]
    fn explorer_profile_no_write() {
        let profile = AgentProfile::from_soul("explorer-1", SoulKind::Explorer, 500);
        assert!(profile.can_use_op("FileRead"));
        assert!(!profile.can_use_op("FileWrite"));
        assert!(!profile.can_use_op("ShellExec"));
    }

    #[test]
    fn cage_monitor_minimal_ops() {
        let profile = AgentProfile::from_soul("cage-1", SoulKind::CageMonitor, 200);
        assert!(profile.can_use_op("MemRead"));
        assert!(!profile.can_use_op("FileRead"));
        assert!(!profile.can_use_op("ProviderCall"));
    }

    #[test]
    fn memory_view_level_filter() {
        let db = Arc::new(MemoryDatabase::open_in_memory().unwrap());

        // Write entries at different levels
        db.write(String::from("l1_key"), Scope::Project, Level::L1, "main", b"project data".to_vec()).unwrap();
        db.write(String::from("l2_key"), Scope::Task, Level::L2, "main", b"task data".to_vec()).unwrap();

        // Worker view: can see L2, L3, L4 — not L1
        let view = MemoryView::new(db.clone(), vec!["L2".into(), "L3".into(), "L4".into()]);
        assert!(view.read("l2_key").is_some(), "worker should see L2");
        assert!(view.read("l1_key").is_none(), "worker should NOT see L1");
    }

    #[test]
    fn memory_view_no_write_methods() {
        // This test verifies by compilation: MemoryView has no
        // write(), update(), delete(), promote() methods.
        // If someone adds a write method, this comment reminds
        // them that SubAgents MUST NOT have write access.
        let db = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        let view = MemoryView::new(db, vec!["L2".into()]);
        let _ = view.authorized_levels();
        // No write operations possible — type system enforces this.
    }

    #[test]
    fn explorer_sees_l1_l2_l5() {
        let db = Arc::new(MemoryDatabase::open_in_memory().unwrap());
        db.write(String::from("l1"), Scope::Project, Level::L1, "main", b"project".to_vec()).unwrap();
        db.write(String::from("l2"), Scope::Task, Level::L2, "main", b"task".to_vec()).unwrap();
        db.write(String::from("l5"), Scope::Project, Level::L5, "main", b"dormant".to_vec()).unwrap();
        db.write(String::from("l0"), Scope::Global, Level::L0, "main", b"const".to_vec()).unwrap();

        let profile = AgentProfile::from_soul("explorer-1", SoulKind::Explorer, 500);
        let view = MemoryView::new(db, profile.memory_access.clone());

        assert!(view.read("l1").is_some(), "explorer should see L1");
        assert!(view.read("l2").is_some(), "explorer should see L2");
        assert!(view.read("l5").is_some(), "explorer should see L5");
        assert!(view.read("l0").is_none(), "explorer should NOT see L0");
    }
}
