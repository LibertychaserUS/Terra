use crate::error::MemoryError;
use crate::types::{Level, MemoryEntry};

/// Trait for pluggable memory storage backends.
///
/// Abstracts the persistence layer so that the upper layers
/// (cache, safety enforcer, MemoryDatabase) are decoupled from
/// any concrete storage engine.
///
/// Default implementation: `SqliteStore`.
/// Testing: `InMemoryBackend` (HashMap-based, no persistence).
/// Future: PostgreSQL, RocksDB.
pub trait StorageBackend: Send + Sync {
    /// Insert a new memory entry.
    fn insert(&self, entry: &MemoryEntry) -> Result<(), MemoryError>;

    /// Read a memory entry by key (including dead entries).
    fn read(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError>;

    /// Update content and increment version. Returns new version.
    /// Fails on version mismatch (optimistic concurrency).
    fn update_content(
        &self,
        key: &str,
        content: &[u8],
        expected_version: u64,
    ) -> Result<u64, MemoryError>;

    /// Soft-delete: mark entry as not alive.
    fn soft_delete(&self, key: &str) -> Result<(), MemoryError>;

    /// Update the level of a memory entry (promotion/demotion).
    fn update_level(&self, key: &str, new_level: &Level) -> Result<(), MemoryError>;

    /// Get the owner of a memory entry (None if key not found or dead).
    fn get_owner(&self, key: &str) -> Result<Option<String>, MemoryError>;

    /// Garbage-collect: remove dead entries with ref_count=0 and non-protected level.
    /// Returns number of entries removed.
    fn gc(&self) -> Result<usize, MemoryError>;

    /// Count all alive entries.
    fn count_alive(&self) -> Result<usize, MemoryError>;

    /// Increment reference count for a memory entry.
    fn increment_ref(&self, key: &str) -> Result<(), MemoryError>;

    /// Decrement reference count for a memory entry.
    fn decrement_ref(&self, key: &str) -> Result<(), MemoryError>;

    /// Soft-delete all entries matching the given scope.
    /// Used for lifecycle cleanup (task ends -> delete task entries, etc.)
    fn soft_delete_by_scope(&self, scope: &str) -> Result<usize, MemoryError>;
}

/// HashMap-based in-memory backend for testing.
///
/// No persistence, no SQL constraints — just a simple concurrent map.
/// Useful for unit tests that don't need SQLite overhead.
pub struct InMemoryBackend {
    entries: std::sync::RwLock<std::collections::HashMap<String, MemoryEntry>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self {
            entries: std::sync::RwLock::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageBackend for InMemoryBackend {
    fn insert(&self, entry: &MemoryEntry) -> Result<(), MemoryError> {
        let mut map = self.entries.write().unwrap();
        if map.contains_key(&entry.key) {
            return Err(MemoryError::DuplicateKey(entry.key.clone()));
        }
        map.insert(entry.key.clone(), entry.clone());
        Ok(())
    }

    fn read(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        let map = self.entries.read().unwrap();
        Ok(map.get(key).cloned())
    }

    fn update_content(
        &self,
        key: &str,
        content: &[u8],
        expected_version: u64,
    ) -> Result<u64, MemoryError> {
        let mut map = self.entries.write().unwrap();
        match map.get_mut(key) {
            Some(entry) => {
                if !entry.alive {
                    return Err(MemoryError::Dangling(key.to_string()));
                }
                if entry.version != expected_version {
                    return Err(MemoryError::Stale {
                        expected: expected_version,
                        found: entry.version,
                    });
                }
                entry.content = content.to_vec();
                entry.version += 1;
                entry.updated_at = chrono::Utc::now();
                Ok(entry.version)
            }
            None => Err(MemoryError::NotFound(key.to_string())),
        }
    }

    fn soft_delete(&self, key: &str) -> Result<(), MemoryError> {
        let mut map = self.entries.write().unwrap();
        match map.get_mut(key) {
            Some(entry) if entry.alive => {
                entry.alive = false;
                entry.updated_at = chrono::Utc::now();
                Ok(())
            }
            Some(_) => Err(MemoryError::NotFound(key.to_string())),
            None => Err(MemoryError::NotFound(key.to_string())),
        }
    }

    fn update_level(&self, key: &str, new_level: &Level) -> Result<(), MemoryError> {
        let mut map = self.entries.write().unwrap();
        match map.get_mut(key) {
            Some(entry) if entry.alive => {
                entry.level = new_level.clone();
                entry.updated_at = chrono::Utc::now();
                Ok(())
            }
            Some(_) => Err(MemoryError::NotFound(key.to_string())),
            None => Err(MemoryError::NotFound(key.to_string())),
        }
    }

    fn get_owner(&self, key: &str) -> Result<Option<String>, MemoryError> {
        let map = self.entries.read().unwrap();
        Ok(map.get(key).filter(|e| e.alive).map(|e| e.owner.clone()))
    }

    fn gc(&self) -> Result<usize, MemoryError> {
        let mut map = self.entries.write().unwrap();
        let before = map.len();
        map.retain(|_, entry| {
            // Keep if: alive, or has refs, or gc-protected level
            entry.alive || entry.ref_count > 0 || entry.level.is_gc_protected()
        });
        Ok(before - map.len())
    }

    fn count_alive(&self) -> Result<usize, MemoryError> {
        let map = self.entries.read().unwrap();
        Ok(map.values().filter(|e| e.alive).count())
    }

    fn increment_ref(&self, key: &str) -> Result<(), MemoryError> {
        let mut map = self.entries.write().unwrap();
        match map.get_mut(key) {
            Some(entry) if entry.alive => {
                entry.ref_count += 1;
                Ok(())
            }
            Some(_) => Err(MemoryError::Dangling(key.to_string())),
            None => Err(MemoryError::NotFound(key.to_string())),
        }
    }

    fn decrement_ref(&self, key: &str) -> Result<(), MemoryError> {
        let mut map = self.entries.write().unwrap();
        match map.get_mut(key) {
            Some(entry) if entry.ref_count > 0 => {
                entry.ref_count -= 1;
                Ok(())
            }
            Some(entry) if !entry.alive => Err(MemoryError::Dangling(key.to_string())),
            Some(_) => Ok(()), // already 0, don't underflow
            None => Err(MemoryError::NotFound(key.to_string())),
        }
    }

    fn soft_delete_by_scope(&self, scope: &str) -> Result<usize, MemoryError> {
        let mut map = self.entries.write().unwrap();
        let mut count = 0;
        for entry in map.values_mut() {
            if entry.alive && entry.scope.as_str() == scope {
                entry.alive = false;
                entry.updated_at = chrono::Utc::now();
                count += 1;
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::new_entry;
    use crate::types::{Level, Scope};

    fn backend() -> InMemoryBackend {
        InMemoryBackend::new()
    }

    fn test_entry(key: &str) -> MemoryEntry {
        new_entry(key, Scope::Project, Level::L1, "main", b"test content".to_vec())
    }

    #[test]
    fn insert_and_read() {
        let b = backend();
        let entry = test_entry("key1");
        b.insert(&entry).unwrap();
        let read = b.read("key1").unwrap().unwrap();
        assert_eq!(read.key, "key1");
        assert_eq!(read.content, b"test content");
    }

    #[test]
    fn duplicate_key() {
        let b = backend();
        b.insert(&test_entry("dup")).unwrap();
        let result = b.insert(&test_entry("dup"));
        assert!(matches!(result, Err(MemoryError::DuplicateKey(_))));
    }

    #[test]
    fn read_missing() {
        let b = backend();
        assert!(b.read("nope").unwrap().is_none());
    }

    #[test]
    fn update_content_increments_version() {
        let b = backend();
        b.insert(&test_entry("v")).unwrap();
        let new_ver = b.update_content("v", b"updated", 1).unwrap();
        assert_eq!(new_ver, 2);
        let read = b.read("v").unwrap().unwrap();
        assert_eq!(read.content, b"updated");
        assert_eq!(read.version, 2);
    }

    #[test]
    fn update_version_mismatch() {
        let b = backend();
        b.insert(&test_entry("s")).unwrap();
        b.update_content("s", b"v2", 1).unwrap();
        let result = b.update_content("s", b"v3", 1);
        assert!(matches!(result, Err(MemoryError::Stale { expected: 1, found: 2 })));
    }

    #[test]
    fn soft_delete_and_dangling() {
        let b = backend();
        b.insert(&test_entry("del")).unwrap();
        b.soft_delete("del").unwrap();
        let read = b.read("del").unwrap().unwrap();
        assert!(!read.alive);
        let result = b.update_content("del", b"new", 1);
        assert!(matches!(result, Err(MemoryError::Dangling(_))));
    }

    #[test]
    fn update_level() {
        let b = backend();
        b.insert(&new_entry("promo", Scope::Task, Level::L2, "main", b"x".to_vec())).unwrap();
        b.update_level("promo", &Level::L1).unwrap();
        let read = b.read("promo").unwrap().unwrap();
        assert_eq!(read.level, Level::L1);
    }

    #[test]
    fn get_owner() {
        let b = backend();
        b.insert(&new_entry("owned", Scope::Task, Level::L2, "worker-1", b"x".to_vec())).unwrap();
        assert_eq!(b.get_owner("owned").unwrap(), Some("worker-1".into()));
    }

    #[test]
    fn gc_removes_dead() {
        let b = backend();
        b.insert(&new_entry("gc_target", Scope::Task, Level::L5, "main", b"x".to_vec())).unwrap();
        b.soft_delete("gc_target").unwrap();
        let removed = b.gc().unwrap();
        assert_eq!(removed, 1);
    }

    #[test]
    fn gc_preserves_l0() {
        let b = backend();
        b.insert(&new_entry("constitution", Scope::Global, Level::L0, "system", b"law".to_vec())).unwrap();
        b.soft_delete("constitution").unwrap();
        let removed = b.gc().unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn count_alive() {
        let b = backend();
        b.insert(&test_entry("a")).unwrap();
        b.insert(&new_entry("b", Scope::Task, Level::L2, "main", b"x".to_vec())).unwrap();
        assert_eq!(b.count_alive().unwrap(), 2);
        b.soft_delete("a").unwrap();
        assert_eq!(b.count_alive().unwrap(), 1);
    }

    #[test]
    fn increment_and_decrement_ref() {
        let b = backend();
        b.insert(&test_entry("ref")).unwrap();
        b.increment_ref("ref").unwrap();
        let entry = b.read("ref").unwrap().unwrap();
        assert_eq!(entry.ref_count, 1);
        b.decrement_ref("ref").unwrap();
        let entry = b.read("ref").unwrap().unwrap();
        assert_eq!(entry.ref_count, 0);
    }

    #[test]
    fn gc_preserves_with_refs() {
        let b = backend();
        b.insert(&new_entry("ref_gc", Scope::Task, Level::L5, "main", b"x".to_vec())).unwrap();
        b.increment_ref("ref_gc").unwrap();
        b.soft_delete("ref_gc").unwrap();
        let removed = b.gc().unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn soft_delete_by_scope_test() {
        let b = backend();
        b.insert(&new_entry("t1", Scope::Task, Level::L2, "main", b"a".to_vec())).unwrap();
        b.insert(&new_entry("t2", Scope::Task, Level::L2, "main", b"b".to_vec())).unwrap();
        b.insert(&new_entry("p1", Scope::Project, Level::L1, "main", b"c".to_vec())).unwrap();
        let removed = b.soft_delete_by_scope("task").unwrap();
        assert_eq!(removed, 2);
        assert_eq!(b.count_alive().unwrap(), 1);
    }
}
