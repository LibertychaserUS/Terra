use std::sync::Arc;
use std::time::Duration;

use moka::sync::Cache;
use tracing::{debug, trace};

use crate::types::MemoryEntry;

/// In-memory cache layer sitting between the public API and SQLite.
///
/// Implements a read-through / write-through pattern:
/// - Reads: check cache first, fall through to SQLite on miss
/// - Writes: update cache and SQLite together
///
/// Uses moka for concurrent access, TTL-based expiration, and
/// capacity-bounded eviction (LRU-like, frequency-aware).
pub struct MemoryCache {
    inner: Cache<String, Arc<MemoryEntry>>,
    stats: CacheStats,
}

/// Cache hit/miss statistics for monitoring.
#[derive(Debug, Default)]
pub struct CacheStats {
    hits: std::sync::atomic::AtomicU64,
    misses: std::sync::atomic::AtomicU64,
}

impl CacheStats {
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn record_miss(&self) {
        self.misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn misses(&self) -> u64 {
        self.misses.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl MemoryCache {
    /// Create a new cache with the given capacity and TTL.
    pub fn new(capacity: u64, ttl_secs: u64) -> Self {
        let inner = Cache::builder()
            .max_capacity(capacity)
            .time_to_idle(Duration::from_secs(ttl_secs))
            .build();

        Self {
            inner,
            stats: CacheStats::default(),
        }
    }

    /// Try to get an entry from cache.
    pub fn get(&self, key: &str) -> Option<Arc<MemoryEntry>> {
        match self.inner.get(&key.to_string()) {
            Some(entry) => {
                self.stats.record_hit();
                trace!(key = key, "cache hit");
                Some(entry)
            }
            None => {
                self.stats.record_miss();
                trace!(key = key, "cache miss");
                None
            }
        }
    }

    /// Insert or update an entry in cache.
    pub fn put(&self, key: &str, entry: MemoryEntry) {
        debug!(key = key, "cache put");
        self.inner.insert(key.to_string(), Arc::new(entry));
    }

    /// Remove an entry from cache.
    pub fn invalidate(&self, key: &str) {
        debug!(key = key, "cache invalidate");
        self.inner.invalidate(&key.to_string());
    }

    /// Clear the entire cache (e.g., on project switch).
    pub fn clear(&self) {
        debug!("cache cleared");
        self.inner.invalidate_all();
    }

    /// Get current number of entries in cache.
    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::new_entry;
    use crate::types::{Level, Scope};

    fn make_entry(key: &str) -> MemoryEntry {
        new_entry(key, Scope::Project, Level::L1, "main", b"content".to_vec())
    }

    // T-M-019: cache hit returns fast
    #[test]
    fn cache_hit() {
        let cache = MemoryCache::new(100, 300);
        let entry = make_entry("key1");
        cache.put("key1", entry.clone());

        let result = cache.get("key1");
        assert!(result.is_some());
        assert_eq!(result.unwrap().key, "key1");
        assert_eq!(cache.stats().hits(), 1);
        assert_eq!(cache.stats().misses(), 0);
    }

    // T-M-020: cache miss
    #[test]
    fn cache_miss() {
        let cache = MemoryCache::new(100, 300);

        let result = cache.get("nonexistent");
        assert!(result.is_none());
        assert_eq!(cache.stats().hits(), 0);
        assert_eq!(cache.stats().misses(), 1);
    }

    // T-M-022: eviction on capacity
    #[test]
    fn cache_eviction() {
        // Small cache
        let cache = MemoryCache::new(10, 300);

        // Insert more than capacity
        for i in 0..20 {
            cache.put(&format!("key-{}", i), make_entry(&format!("key-{}", i)));
        }

        // Force pending evictions
        cache.inner.run_pending_tasks();

        // Should have fewer entries than inserted (eviction occurred)
        let count = cache.entry_count();
        assert!(
            count <= 10,
            "cache should evict down to capacity, got {} entries",
            count
        );
    }

    #[test]
    fn cache_invalidate() {
        let cache = MemoryCache::new(100, 300);
        cache.put("key1", make_entry("key1"));
        assert!(cache.get("key1").is_some());

        cache.invalidate("key1");
        // After invalidation, should miss
        let _ = cache.get("key1"); // reset stats from hit above
        assert!(cache.get("key1").is_none());
    }

    #[test]
    fn cache_clear() {
        let cache = MemoryCache::new(100, 300);
        cache.put("a", make_entry("a"));
        cache.put("b", make_entry("b"));
        cache.clear();
        cache.inner.run_pending_tasks();

        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn cache_overwrite() {
        let cache = MemoryCache::new(100, 300);
        cache.put("key1", make_entry("key1"));

        let mut updated = make_entry("key1");
        updated.content = b"updated_content".to_vec();
        cache.put("key1", updated);

        let result = cache.get("key1").unwrap();
        assert_eq!(result.content, b"updated_content");
    }

    // Concurrent reads and writes on cache
    #[test]
    fn cache_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let cache = Arc::new(MemoryCache::new(1000, 300));
        let mut handles = Vec::new();

        // 5 writer threads
        for t in 0..5 {
            let cache = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    let key = format!("t{}-k{}", t, i);
                    cache.put(&key, make_entry(&key));
                }
            }));
        }

        // 5 reader threads
        for t in 0..5 {
            let cache = Arc::clone(&cache);
            handles.push(thread::spawn(move || {
                for i in 0..100 {
                    let key = format!("t{}-k{}", t, i);
                    let _ = cache.get(&key); // may or may not exist yet
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // No panic, no data corruption
        assert!(cache.entry_count() > 0);
    }
}
