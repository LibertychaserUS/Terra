use std::path::Path;

use crate::error::MemoryError;
use crate::safety::SafetyEnforcer;
use crate::store::{self, SqliteStore};
use crate::types::{Level, MemoryEntry, Scope};

/// Public API for the Memory Database.
///
/// Composes the storage layer (SqliteStore) with the safety layer
/// (SafetyEnforcer). All mutations go through ownership and
/// permission checks before reaching storage.
pub struct MemoryDatabase {
    store: SqliteStore,
}

/// Report from garbage collection.
#[derive(Debug)]
pub struct GcReport {
    pub entries_removed: usize,
}

impl MemoryDatabase {
    /// Open a database at the given path.
    pub fn open(path: &Path) -> Result<Self, MemoryError> {
        Ok(Self {
            store: SqliteStore::open(path)?,
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self, MemoryError> {
        Ok(Self {
            store: SqliteStore::open_in_memory()?,
        })
    }

    /// Write a new memory entry.
    pub fn write(
        &self,
        key: impl Into<String>,
        scope: Scope,
        level: Level,
        owner: impl Into<String>,
        content: Vec<u8>,
    ) -> Result<(), MemoryError> {
        let entry = store::new_entry(key, scope, level, owner, content);
        self.store.insert(&entry)
    }

    /// Read a memory entry by key. Returns None if not found.
    pub fn read(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        let entry = self.store.read(key)?;
        if let Some(ref e) = entry {
            if !e.alive {
                return Err(MemoryError::Dangling(key.to_string()));
            }
        }
        Ok(entry)
    }

    /// Update content of an existing entry. Enforces ownership.
    pub fn update(
        &self,
        key: &str,
        content: Vec<u8>,
        agent: &str,
    ) -> Result<u64, MemoryError> {
        SafetyEnforcer::check_write_permission(&self.store, key, agent)?;
        let entry = self.store.read(key)?
            .ok_or_else(|| MemoryError::NotFound(key.to_string()))?;
        self.store.update_content(key, &content, entry.version)
    }

    /// Soft-delete a memory entry. Enforces ownership.
    pub fn delete(&self, key: &str, agent: &str) -> Result<(), MemoryError> {
        SafetyEnforcer::check_write_permission(&self.store, key, agent)?;
        self.store.soft_delete(key)
    }

    /// Promote a memory entry to a higher (more authoritative) level.
    pub fn promote(
        &self,
        key: &str,
        target_level: Level,
        reviewed: bool,
    ) -> Result<(), MemoryError> {
        SafetyEnforcer::check_promotion(&self.store, key, &target_level, reviewed)?;
        self.store.update_level(key, &target_level)
    }

    /// Run garbage collection. Removes dead entries with no references
    /// that are not at protected levels (L0, L1).
    pub fn gc(&self) -> Result<GcReport, MemoryError> {
        let removed = self.store.gc()?;
        Ok(GcReport {
            entries_removed: removed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> MemoryDatabase {
        MemoryDatabase::open_in_memory().unwrap()
    }

    #[test]
    fn write_read_roundtrip() {
        let db = db();
        db.write("key1", Scope::Project, Level::L1, "main", b"hello".to_vec()).unwrap();
        let entry = db.read("key1").unwrap().unwrap();
        assert_eq!(entry.content, b"hello");
        assert_eq!(entry.owner, "main");
    }

    #[test]
    fn read_nonexistent_returns_none() {
        let db = db();
        assert!(db.read("nope").unwrap().is_none());
    }

    #[test]
    fn read_deleted_returns_dangling() {
        let db = db();
        db.write("del", Scope::Task, Level::L2, "main", b"x".to_vec()).unwrap();
        db.delete("del", "main").unwrap();
        let result = db.read("del");
        assert!(matches!(result, Err(MemoryError::Dangling(_))));
    }

    #[test]
    fn update_by_owner_succeeds() {
        let db = db();
        db.write("upd", Scope::Project, Level::L1, "main", b"v1".to_vec()).unwrap();
        let v = db.update("upd", b"v2".to_vec(), "main").unwrap();
        assert_eq!(v, 2);
    }

    #[test]
    fn update_by_non_owner_fails() {
        let db = db();
        db.write("owned", Scope::Project, Level::L1, "main", b"x".to_vec()).unwrap();
        let result = db.update("owned", b"hack".to_vec(), "attacker");
        assert!(matches!(result, Err(MemoryError::PermissionDenied { .. })));
    }

    #[test]
    fn delete_by_non_owner_fails() {
        let db = db();
        db.write("protected", Scope::Project, Level::L1, "main", b"x".to_vec()).unwrap();
        let result = db.delete("protected", "attacker");
        assert!(matches!(result, Err(MemoryError::PermissionDenied { .. })));
    }

    #[test]
    fn promote_l2_to_l1() {
        let db = db();
        db.write("task_knowledge", Scope::Task, Level::L2, "main", b"insight".to_vec()).unwrap();
        db.promote("task_knowledge", Level::L1, false).unwrap();
        let entry = db.read("task_knowledge").unwrap().unwrap();
        assert_eq!(entry.level, Level::L1);
    }

    #[test]
    fn promote_l6_blocked_without_review() {
        let db = db();
        db.write("candidate", Scope::Project, Level::L6, "main", b"pattern".to_vec()).unwrap();
        let result = db.promote("candidate", Level::L1, false);
        assert!(matches!(result, Err(MemoryError::PromotionBlocked { .. })));
    }

    #[test]
    fn promote_l6_allowed_with_review() {
        let db = db();
        db.write("reviewed", Scope::Project, Level::L6, "main", b"pattern".to_vec()).unwrap();
        db.promote("reviewed", Level::L1, true).unwrap();
        let entry = db.read("reviewed").unwrap().unwrap();
        assert_eq!(entry.level, Level::L1);
    }

    #[test]
    fn gc_cleans_dead_entries() {
        let db = db();
        db.write("garbage", Scope::Task, Level::L5, "main", b"x".to_vec()).unwrap();
        db.delete("garbage", "main").unwrap();
        let report = db.gc().unwrap();
        assert_eq!(report.entries_removed, 1);
    }

    #[test]
    fn gc_preserves_l0() {
        let db = db();
        db.write("law", Scope::Global, Level::L0, "system", b"constitution".to_vec()).unwrap();
        db.delete("law", "system").unwrap();
        let report = db.gc().unwrap();
        assert_eq!(report.entries_removed, 0);
    }

    #[test]
    fn duplicate_key_error() {
        let db = db();
        db.write("dup", Scope::Task, Level::L2, "main", b"a".to_vec()).unwrap();
        let result = db.write("dup", Scope::Task, Level::L2, "main", b"b".to_vec());
        assert!(matches!(result, Err(MemoryError::DuplicateKey(_))));
    }
}
