use std::path::Path;

use crate::cache::MemoryCache;
use crate::error::MemoryError;
use crate::safety::SafetyEnforcer;
use crate::store::{self, SqliteStore};
use crate::types::{Level, MemoryEntry, Scope};

/// Default cache capacity (number of entries).
const DEFAULT_CACHE_CAPACITY: u64 = 1000;
/// Default time-to-idle in seconds (5 minutes).
const DEFAULT_CACHE_TTL: u64 = 300;

/// Public API for the Memory Database.
///
/// Composes three layers:
/// 1. Cache (moka) — fast in-memory access
/// 2. Safety (SafetyEnforcer) — ownership and permission checks
/// 3. Store (SqliteStore) — persistent SQLite storage
///
/// Read path: cache → SQLite (read-through)
/// Write path: safety check → SQLite → cache (write-through)
pub struct MemoryDatabase {
    store: SqliteStore,
    cache: MemoryCache,
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
            cache: MemoryCache::new(DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_TTL),
        })
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self, MemoryError> {
        Ok(Self {
            store: SqliteStore::open_in_memory()?,
            cache: MemoryCache::new(DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_TTL),
        })
    }

    /// Open with custom cache configuration.
    pub fn open_with_cache(
        path: &Path,
        cache_capacity: u64,
        cache_ttl_secs: u64,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            store: SqliteStore::open(path)?,
            cache: MemoryCache::new(cache_capacity, cache_ttl_secs),
        })
    }

    /// Write a new memory entry.
    /// Write-through: SQLite first, then cache.
    pub fn write(
        &self,
        key: impl Into<String>,
        scope: Scope,
        level: Level,
        owner: impl Into<String>,
        content: Vec<u8>,
    ) -> Result<(), MemoryError> {
        let entry = store::new_entry(key, scope, level, owner, content);
        // Write to SQLite first (source of truth)
        self.store.insert(&entry)?;
        // Then populate cache
        let cache_key = entry.key.clone();
        self.cache.put(&cache_key, entry);
        Ok(())
    }

    /// Read a memory entry by key.
    /// Read-through: check cache first, fall through to SQLite on miss.
    pub fn read(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        // Check cache first
        if let Some(cached) = self.cache.get(key) {
            if !cached.alive {
                return Err(MemoryError::Dangling(key.to_string()));
            }
            // Return owned clone from Arc
            return Ok(Some((*cached).clone()));
        }

        // Cache miss — fall through to SQLite
        let entry = self.store.read(key)?;
        if let Some(ref e) = entry {
            if !e.alive {
                return Err(MemoryError::Dangling(key.to_string()));
            }
            // Populate cache for next access
            self.cache.put(key, e.clone());
        }
        Ok(entry)
    }

    /// Update content of an existing entry. Enforces ownership.
    /// Write-through: safety check → SQLite → invalidate cache.
    pub fn update(
        &self,
        key: &str,
        content: Vec<u8>,
        agent: &str,
    ) -> Result<u64, MemoryError> {
        SafetyEnforcer::check_write_permission(&self.store, key, agent)?;
        let entry = self.store.read(key)?
            .ok_or_else(|| MemoryError::NotFound(key.to_string()))?;
        let new_version = self.store.update_content(key, &content, entry.version)?;
        // Invalidate cache — next read will repopulate from SQLite
        self.cache.invalidate(key);
        Ok(new_version)
    }

    /// Soft-delete a memory entry. Enforces ownership.
    pub fn delete(&self, key: &str, agent: &str) -> Result<(), MemoryError> {
        SafetyEnforcer::check_write_permission(&self.store, key, agent)?;
        self.store.soft_delete(key)?;
        self.cache.invalidate(key);
        Ok(())
    }

    /// Promote a memory entry to a higher (more authoritative) level.
    pub fn promote(
        &self,
        key: &str,
        target_level: Level,
        reviewed: bool,
    ) -> Result<(), MemoryError> {
        SafetyEnforcer::check_promotion(&self.store, key, &target_level, reviewed)?;
        self.store.update_level(key, &target_level)?;
        self.cache.invalidate(key);
        Ok(())
    }

    /// Run garbage collection.
    pub fn gc(&self) -> Result<GcReport, MemoryError> {
        let removed = self.store.gc()?;
        // Clear cache to avoid serving stale entries
        if removed > 0 {
            self.cache.clear();
        }
        Ok(GcReport {
            entries_removed: removed,
        })
    }

    /// Get cache statistics.
    pub fn cache_hits(&self) -> u64 {
        self.cache.stats().hits()
    }

    /// Get cache misses.
    pub fn cache_misses(&self) -> u64 {
        self.cache.stats().misses()
    }

    /// Get number of entries currently in cache.
    pub fn cache_entry_count(&self) -> u64 {
        self.cache.entry_count()
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

    // --- Cache-specific tests ---

    // T-M-019: cache hit on second read
    #[test]
    fn cache_hit_on_second_read() {
        let db = db();
        db.write("cached", Scope::Project, Level::L1, "main", b"data".to_vec()).unwrap();

        // First read: cache miss (populated by write-through), or miss then SQLite
        let _ = db.read("cached").unwrap();
        let initial_hits = db.cache_hits();

        // Second read: should be cache hit
        let entry = db.read("cached").unwrap().unwrap();
        assert_eq!(entry.content, b"data");
        assert!(db.cache_hits() > initial_hits, "second read should be cache hit");
    }

    // T-M-020: cache miss falls through to SQLite
    #[test]
    fn cache_miss_falls_through() {
        let db = db();
        db.write("fallthrough", Scope::Project, Level::L1, "main", b"data".to_vec()).unwrap();

        // Invalidate cache manually to force miss
        db.cache.invalidate("fallthrough");

        // Read should still work (from SQLite)
        let entry = db.read("fallthrough").unwrap().unwrap();
        assert_eq!(entry.content, b"data");
        // Misses should have increased
        assert!(db.cache_misses() > 0);
    }

    // T-M-021: write updates both cache and SQLite
    #[test]
    fn write_updates_both_layers() {
        let db = db();
        db.write("both", Scope::Project, Level::L1, "main", b"data".to_vec()).unwrap();

        // Cache should have it
        assert!(db.cache.get("both").is_some());

        // SQLite should have it (bypass cache)
        db.cache.invalidate("both");
        let from_sqlite = db.read("both").unwrap();
        assert!(from_sqlite.is_some());
    }

    // T-M-022: update invalidates cache
    #[test]
    fn update_invalidates_cache() {
        let db = db();
        db.write("inv", Scope::Project, Level::L1, "main", b"v1".to_vec()).unwrap();

        // Verify in cache
        assert!(db.cache.get("inv").is_some());

        // Update
        db.update("inv", b"v2".to_vec(), "main").unwrap();

        // Cache should be invalidated; next read repopulates
        let entry = db.read("inv").unwrap().unwrap();
        assert_eq!(entry.content, b"v2");
        assert_eq!(entry.version, 2);
    }

    #[test]
    fn cache_stats_tracked() {
        let db = db();
        db.write("s1", Scope::Task, Level::L2, "main", b"x".to_vec()).unwrap();

        // Force a miss by invalidating
        db.cache.invalidate("s1");
        let _ = db.read("s1").unwrap(); // miss → SQLite → repopulate cache
        let _ = db.read("s1").unwrap(); // hit

        assert!(db.cache_hits() >= 1);
        assert!(db.cache_misses() >= 1);
    }
}
